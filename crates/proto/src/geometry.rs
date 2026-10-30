use std::fmt;
use std::str::FromStr;

/// A point in logical (scale-independent) desktop pixels, with sub-pixel precision.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pos {
    pub x: f64,
    pub y: f64,
}

impl Pos {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// One monitor, in logical desktop pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Pos) -> bool {
        p.x >= f64::from(self.x)
            && p.x < f64::from(self.right())
            && p.y >= f64::from(self.y)
            && p.y < f64::from(self.bottom())
    }
    pub fn center(&self) -> Pos {
        Pos::new(f64::from(self.x) + f64::from(self.w) / 2.0, f64::from(self.y) + f64::from(self.h) / 2.0)
    }
    /// Nearest point inside the rect (upper bounds are exclusive).
    pub fn clamp(&self, p: Pos) -> Pos {
        const IN: f64 = 1.0 / 256.0;
        Pos {
            x: p.x.clamp(f64::from(self.x), f64::from(self.right()) - IN),
            y: p.y.clamp(f64::from(self.y), f64::from(self.bottom()) - IN),
        }
    }
}

/// Which side of a desktop another machine sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
    /// Whether positions along this edge run along x (top and bottom
    /// edges) rather than y (left and right).
    pub fn along_x(self) -> bool {
        matches!(self, Side::Top | Side::Bottom)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Top => "top",
            Side::Bottom => "bottom",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Side {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Side::ALL
            .into_iter()
            .find(|side| side.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown side {s:?} (expected left, right, top or bottom)"))
    }
}

/// All monitors of one machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desktop {
    pub screens: Vec<Rect>,
}

impl Desktop {
    pub fn new(screens: Vec<Rect>) -> Self {
        assert!(!screens.is_empty(), "a desktop needs at least one screen");
        Self { screens }
    }

    pub fn bounds(&self) -> Rect {
        let x = self.screens.iter().map(|r| r.x).min().unwrap_or(0);
        let y = self.screens.iter().map(|r| r.y).min().unwrap_or(0);
        let r = self.screens.iter().map(Rect::right).max().unwrap_or(0);
        let b = self.screens.iter().map(Rect::bottom).max().unwrap_or(0);
        Rect::new(x, y, r - x, b - y)
    }

    /// The main monitor. It isn't on the wire, so this is the one at the
    /// origin, which is where Windows and macOS always put it (and where
    /// Linux desktops put it unless arranged otherwise), else the first.
    pub fn primary(&self) -> &Rect {
        self.screens.iter().find(|r| r.contains(Pos::new(0.0, 0.0))).unwrap_or(&self.screens[0])
    }

    pub fn screen_at(&self, p: Pos) -> Option<&Rect> {
        self.screens.iter().find(|r| r.contains(p))
    }

    /// Nearest point on any screen, so the pointer never ends up in a gap
    /// between monitors of different sizes.
    pub fn clamp(&self, p: Pos) -> Pos {
        if self.screen_at(p).is_some() {
            return p;
        }
        self.screens
            .iter()
            .map(|r| r.clamp(p))
            .min_by(|a, b| dist2(*a, p).total_cmp(&dist2(*b, p)))
            .unwrap_or(p)
    }

    /// Where the screens on `side`'s outer edge begin and how far they run
    /// along it. This, not the whole bounding box, is what crossing happens
    /// on, so it is what fractions are measured against.
    fn edge_span(&self, side: Side) -> (f64, f64) {
        let (lo, hi) = self.outer_edges(side).fold((i32::MAX, i32::MIN), |(lo, hi), r| match side {
            Side::Left | Side::Right => (lo.min(r.y), hi.max(r.bottom())),
            Side::Top | Side::Bottom => (lo.min(r.x), hi.max(r.right())),
        });
        (f64::from(lo), f64::from((hi - lo).max(1)))
    }

    /// Position along `side` as a fraction in [0, 1], used to carry the
    /// pointer across machines whose desktops have different sizes.
    pub fn edge_fraction(&self, side: Side, p: Pos) -> f64 {
        let (start, len) = self.edge_span(side);
        let along = match side {
            Side::Left | Side::Right => p.y,
            Side::Top | Side::Bottom => p.x,
        };
        ((along - start) / len).clamp(0.0, 1.0)
    }

    /// A point just inside `side` at fraction `t`, `inset` pixels from the edge.
    pub fn point_on_edge(&self, side: Side, t: f64, inset: f64) -> Pos {
        let b = self.bounds();
        let (start, len) = self.edge_span(side);
        let along = start + t.clamp(0.0, 1.0) * len;
        let p = match side {
            Side::Left => Pos::new(f64::from(b.x) + inset, along),
            Side::Right => Pos::new(f64::from(b.right()) - 1.0 - inset, along),
            Side::Top => Pos::new(along, f64::from(b.y) + inset),
            Side::Bottom => Pos::new(along, f64::from(b.bottom()) - 1.0 - inset),
        };
        self.clamp(p)
    }

    /// Screens whose `side` edge is on the outer boundary of the desktop;
    /// these are where pointer barriers go.
    pub fn outer_edges(&self, side: Side) -> impl Iterator<Item = &Rect> {
        let b = self.bounds();
        self.screens.iter().filter(move |r| match side {
            Side::Left => r.x == b.x,
            Side::Right => r.right() == b.right(),
            Side::Top => r.y == b.y,
            Side::Bottom => r.bottom() == b.bottom(),
        })
    }

    /// The parts of the screens' `side` edges with no screen of this
    /// desktop just beyond them, wherever they are (not only on the
    /// bounding box).
    pub fn exposed_edges(&self, side: Side) -> Vec<Edge> {
        let mut out = Vec::new();
        for r in &self.screens {
            let e = Edge::of(side, r);
            let mut spans = vec![(e.lo, e.hi)];
            // Screens covering the pixels just outside `e` hide that part.
            for s in self.screens.iter().filter(|s| match side {
                Side::Left => s.x < e.at && e.at <= s.right(),
                Side::Right => s.x <= e.at && e.at < s.right(),
                Side::Top => s.y < e.at && e.at <= s.bottom(),
                Side::Bottom => s.y <= e.at && e.at < s.bottom(),
            }) {
                let b = Edge::of(side, s);
                spans = spans
                    .into_iter()
                    .flat_map(|(lo, hi)| [(lo, hi.min(b.lo)), (lo.max(b.hi), hi)])
                    .filter(|(lo, hi)| lo < hi)
                    .collect();
            }
            out.extend(spans.into_iter().map(|(lo, hi)| Edge { side, at: e.at, lo, hi }));
        }
        out
    }

    /// Nearest point on a screen to `p`, then `inset` pixels in from `side`.
    pub fn place(&self, p: Pos, side: Side, inset: f64) -> Pos {
        let p = self.clamp(p);
        self.clamp(match side {
            Side::Left => Pos::new(p.x + inset, p.y),
            Side::Right => Pos::new(p.x - inset, p.y),
            Side::Top => Pos::new(p.x, p.y + inset),
            Side::Bottom => Pos::new(p.x, p.y - inset),
        })
    }
}

/// Edges within this many pixels of each other count as touching.
const TOUCH: i32 = 2;

/// A straight piece of a desktop's `side` boundary: the line `at` (x for
/// left/right, y for top/bottom; right/bottom are one past the last
/// pixel, like [`Rect::right`]), from `lo` up to (not including) `hi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub side: Side,
    pub at: i32,
    pub lo: i32,
    pub hi: i32,
}

impl Edge {
    /// The whole `side` edge of `r`.
    pub fn of(side: Side, r: &Rect) -> Self {
        match side {
            Side::Left => Edge { side, at: r.x, lo: r.y, hi: r.bottom() },
            Side::Right => Edge { side, at: r.right(), lo: r.y, hi: r.bottom() },
            Side::Top => Edge { side, at: r.y, lo: r.x, hi: r.right() },
            Side::Bottom => Edge { side, at: r.bottom(), lo: r.x, hi: r.right() },
        }
    }

    /// `p` split into (across, along) this edge.
    fn axes(&self, p: Pos) -> (f64, f64) {
        match self.side {
            Side::Left | Side::Right => (p.x, p.y),
            Side::Top | Side::Bottom => (p.y, p.x),
        }
    }

    /// How far `p` is from this segment (0 on it).
    pub fn dist(&self, p: Pos) -> f64 {
        let (across, along) = self.axes(p);
        let off = along.clamp(f64::from(self.lo), f64::from(self.hi)) - along;
        (across - f64::from(self.at)).hypot(off)
    }

    /// Whether moving from `from` to `to` goes out through this segment.
    pub fn crossed(&self, from: Pos, to: Pos) -> bool {
        let ((a, _), (b, along)) = (self.axes(from), self.axes(to));
        let at = f64::from(self.at);
        let out = match self.side {
            Side::Left | Side::Top => a >= at && b < at,
            Side::Right | Side::Bottom => a < at && b >= at,
        };
        out && along >= f64::from(self.lo) && along < f64::from(self.hi)
    }
}

/// Where `mine` (my desktop) and `peer`, placed with its origin at
/// `offset` in my coordinates, touch across my `side`: my exposed edges
/// that face an exposed edge of the peer, as segments in my coordinates.
pub fn touching(mine: &Desktop, peer: &Desktop, side: Side, offset: (i32, i32)) -> Vec<Edge> {
    let (across, along) = match side {
        Side::Left | Side::Right => offset,
        Side::Top | Side::Bottom => (offset.1, offset.0),
    };
    let theirs = peer.exposed_edges(side.opposite());
    let mut out = Vec::new();
    for e in mine.exposed_edges(side) {
        for t in &theirs {
            let (lo, hi) = (e.lo.max(t.lo + along), e.hi.min(t.hi + along));
            if (t.at + across - e.at).abs() <= TOUCH && lo < hi {
                out.push(Edge { lo, hi, ..e });
            }
        }
    }
    out
}

fn dist2(a: Pos, b: Pos) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn dual() -> Desktop {
        // 1920x1080 on the left, taller 1440x2560 portrait monitor on the right.
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1440, 2560)])
    }

    #[test]
    fn clamp_avoids_gaps_between_monitors() {
        let d = dual();
        let p = d.clamp(Pos::new(1800.0, 2000.0)); // below the short monitor, near the tall one
        assert!(d.screen_at(p).is_some());
        assert_eq!(p.y, 2000.0);
        assert_eq!(p.x, 1920.0);
    }

    #[test]
    fn edge_round_trip() {
        let d = dual();
        let p = d.point_on_edge(Side::Right, 0.5, 0.0);
        assert!((d.edge_fraction(Side::Right, p) - 0.5).abs() < 1e-9);
        assert_eq!(p.x, 3359.0);
    }

    #[test]
    fn edge_fraction_follows_the_screens_on_that_edge() {
        // A real 4-display Mac: the right-most monitor (y 0..1080) is far
        // smaller than the bounding box (y -840..1080). Crossing happens on
        // that monitor, so its edge must map to the full 0..1 range.
        let d = Desktop::new(vec![
            Rect::new(0, 0, 1920, 1080),
            Rect::new(-2592, 98, 1512, 982),
            Rect::new(-1080, -840, 1080, 1920),
            Rect::new(1920, 0, 1920, 1080),
        ]);
        let right = |y: f64| d.edge_fraction(Side::Right, Pos::new(3839.0, y));
        assert!(right(0.0).abs() < 1e-9);
        assert!((right(540.0) - 0.5).abs() < 1e-9);
        assert!((right(1080.0) - 1.0).abs() < 1e-9);
        let p = d.point_on_edge(Side::Right, 0.5, 2.0);
        assert_eq!((p.x, p.y), (3837.0, 540.0));
        // The left edge belongs to the 1512x982 monitor at x=-2592 and the
        // portrait one at -1080 does not touch it: only the former counts.
        assert_eq!(d.point_on_edge(Side::Left, 0.0, 0.0).y, 98.0);
    }

    #[test]
    fn outer_edges() {
        let d = dual();
        assert_eq!(d.outer_edges(Side::Right).count(), 1);
        assert_eq!(d.outer_edges(Side::Top).count(), 2);
        assert_eq!(d.outer_edges(Side::Bottom).count(), 1);
    }

    fn mac4() -> Desktop {
        Desktop::new(vec![
            Rect::new(0, 0, 1920, 1080),
            Rect::new(-2592, 98, 1512, 982),
            Rect::new(-1080, -840, 1080, 1920),
            Rect::new(1920, 0, 1920, 1080),
        ])
    }

    #[test]
    fn exposed_edges_include_inner_ones() {
        let d = mac4();
        // The portrait monitor's left edge is exposed only above the
        // 1512x982 monitor beside it; the main display's is hidden.
        assert_eq!(
            d.exposed_edges(Side::Left),
            vec![Edge { side: Side::Left, at: -2592, lo: 98, hi: 1080 }, Edge { side: Side::Left, at: -1080, lo: -840, hi: 98 }]
        );
        // Right: the portrait monitor's above the main display, and the outer one.
        assert_eq!(
            d.exposed_edges(Side::Right),
            vec![Edge { side: Side::Right, at: 0, lo: -840, hi: 0 }, Edge { side: Side::Right, at: 3840, lo: 0, hi: 1080 }]
        );
        // Top: every screen's, including the main display's below the box top.
        assert_eq!(d.exposed_edges(Side::Top).len(), 4);
    }

    #[test]
    fn touching_spans() {
        let d = mac4();
        let peer = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
        // To the left of the portrait monitor's top part: touches only above y=98.
        let left = Edge { side: Side::Left, at: -1080, lo: -840, hi: 98 };
        assert_eq!(touching(&d, &peer, Side::Left, (-3000, -900)), vec![left]);
        assert_eq!(touching(&d, &peer, Side::Left, (-3001, -900)), vec![left], "within the tolerance");
        assert_eq!(touching(&d, &peer, Side::Left, (-3010, -900)), vec![], "a gap");
        assert_eq!(touching(&d, &peer, Side::Left, (-3000, 0)), vec![Edge { lo: 0, ..left }], "partial");
        assert_eq!(
            touching(&d, &peer, Side::Right, (3840, 500)),
            vec![Edge { side: Side::Right, at: 3840, lo: 500, hi: 1080 }]
        );
        // The same contact seen from the peer, in its coordinates.
        assert_eq!(
            touching(&peer, &d, Side::Right, (3000, 900)),
            vec![Edge { side: Side::Right, at: 1920, lo: 60, hi: 998 }]
        );
    }

    #[test]
    fn translation_round_trip() {
        let (d, peer) = (mac4(), Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]));
        let (ox, oy) = (-3000.0, -900.0);
        let there = peer.place(Pos::new(-1080.0 - ox, -500.0 - oy), Side::Right, 0.0);
        assert_eq!(there, Pos::new(1920.0 - 1.0 / 256.0, 400.0));
        let back = d.place(Pos::new(there.x + ox, there.y + oy), Side::Left, 2.0);
        assert_eq!(back, Pos::new(-1078.0, -500.0));
        let e = Edge { side: Side::Left, at: -1080, lo: -840, hi: 98 };
        assert!(e.crossed(Pos::new(-1080.0, -500.0), Pos::new(-1081.0, -500.0)));
        assert!(!e.crossed(Pos::new(-1080.0, 500.0), Pos::new(-1081.0, 500.0)), "outside the span");
        assert_eq!(e.dist(Pos::new(-1081.0, 100.0)), 2f64.hypot(1.0));
    }

    /// A tiny deterministic generator, so failures reproduce.
    pub(crate) struct Rng(pub(crate) u64);
    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub(crate) fn below(&mut self, n: u64) -> i32 {
            (self.next() % n.max(1)) as i32
        }
        pub(crate) fn pick<T: Copy>(&mut self, of: &[T]) -> T {
            of[self.next() as usize % of.len()]
        }
    }

    /// A connected desktop of 1 to 4 monitors of assorted sizes (logical
    /// pixels, as a scaled monitor reports), each placed against a side of
    /// one already there at a random offset, never overlapping.
    pub(crate) fn random_desktop(rng: &mut Rng) -> Desktop {
        const SIZES: [(i32, i32); 7] = [(1920, 1080), (2560, 1440), (1280, 800), (1080, 1920), (1440, 900), (3840, 1600), (1707, 1067)];
        let (w, h) = rng.pick(&SIZES);
        let mut screens = vec![Rect::new(rng.below(400) - 200, rng.below(400) - 200, w, h)];
        let n = 1 + rng.below(4);
        let mut tries = 0;
        while (screens.len() as i32) < n && tries < 100 {
            tries += 1;
            let base = screens[rng.below(screens.len() as u64) as usize];
            let (w, h) = rng.pick(&SIZES);
            let r = match rng.pick(&Side::ALL) {
                Side::Left => Rect::new(base.x - w, base.y + rng.below((base.h + h) as u64) - h + 1, w, h),
                Side::Right => Rect::new(base.right(), base.y + rng.below((base.h + h) as u64) - h + 1, w, h),
                Side::Top => Rect::new(base.x + rng.below((base.w + w) as u64) - w + 1, base.y - h, w, h),
                Side::Bottom => Rect::new(base.x + rng.below((base.w + w) as u64) - w + 1, base.bottom(), w, h),
            };
            let overlaps = |a: &Rect, b: &Rect| a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
            if !screens.iter().any(|s| overlaps(s, &r)) {
                screens.push(r);
            }
        }
        Desktop::new(screens)
    }

    /// For many random pairs of multi-monitor desktops, placed so that one
    /// exposed edge of each meets (or, sometimes, misses by a gap): both
    /// computers agree on where they touch; every crossing lands on a
    /// monitor right across the seam; and pushing straight back through
    /// the exits brings the pointer home next to where it left.
    #[test]
    fn random_layouts_cross_and_return() {
        use crate::{HOME_INSET, RemoteCursor, Step};
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut crossed, mut apart) = (0, 0);
        for _ in 0..3000 {
            let (us, them) = (random_desktop(&mut rng), random_desktop(&mut rng));
            let side = rng.pick(&Side::ALL);
            let (mine, theirs) = (us.exposed_edges(side), them.exposed_edges(side.opposite()));
            let (e, t) = (mine[rng.below(mine.len() as u64) as usize], theirs[rng.below(theirs.len() as u64) as usize]);
            let gap = if rng.below(5) == 0 { 3 + rng.below(50) } else { 0 };
            let across = e.at - t.at + if matches!(side, Side::Right | Side::Bottom) { gap } else { -gap };
            let along = e.lo - t.lo + rng.below((e.hi - e.lo + t.hi - t.lo) as u64) - (t.hi - t.lo) + 1;
            let offset = if side.along_x() { (along, across) } else { (across, along) };
            let back = (-offset.0, -offset.1);
            let (here, there) = (touching(&us, &them, side, offset), touching(&them, &us, side.opposite(), back));
            let len = |v: &[Edge]| v.iter().map(|e| i64::from(e.hi - e.lo)).sum::<i64>();
            assert_eq!(len(&here), len(&there), "both computers see the same contact: {us:?} {them:?} {side} {offset:?}");
            if gap > 0 {
                // Other edges may still meet; mostly nothing does.
                apart += usize::from(here.is_empty());
                continue;
            }
            for h in &here {
                if h.hi - h.lo < 3 {
                    continue;
                }
                let v = f64::from(h.lo + 1 + rng.below((h.hi - h.lo - 2) as u64));
                // Just inside our screen at that edge.
                let at = |line: i32| match side {
                    Side::Left | Side::Top => f64::from(line),
                    Side::Right | Side::Bottom => f64::from(line - 1),
                };
                let p = if side.along_x() { Pos::new(v, at(h.at)) } else { Pos::new(at(h.at), v) };
                assert!(us.screen_at(p).is_some(), "{p:?} on {us:?}");
                let there_p = Pos::new(p.x - f64::from(offset.0), p.y - f64::from(offset.1));
                let mut c = RemoteCursor::enter_at(them.clone(), side.opposite(), there_p, there.clone(), 1);
                crossed += 1;
                assert!(them.screen_at(c.pos()).is_some(), "lands on a monitor: {:?} {them:?}", c.pos());
                // Edges up to TOUCH apart count as touching, so another pair
                // of edges may be what touches here.
                assert!(dist2(c.pos(), there_p).sqrt() <= f64::from(TOUCH) + 1.0 + 1e-9, "right across the seam: {there_p:?} -> {:?}", c.pos());
                let push = match side.opposite() {
                    Side::Left => (-3.0, 0.0),
                    Side::Right => (3.0, 0.0),
                    Side::Top => (0.0, -3.0),
                    Side::Bottom => (0.0, 3.0),
                };
                assert!(matches!(c.motion(push.0, push.1), Step::Leave { .. }), "the way back is open at {:?}", c.pos());
                let q = c.pos();
                let home = us.place(Pos::new(q.x + f64::from(offset.0), q.y + f64::from(offset.1)), side, HOME_INSET);
                assert!(us.screen_at(home).is_some());
                assert!(dist2(home, p).sqrt() <= HOME_INSET + f64::from(TOUCH) + 1.0, "{p:?} -> {q:?} -> {home:?}");
            }
        }
        assert!(crossed > 2000 && apart > 300, "only {crossed} crossings, {apart} apart");
    }

    #[test]
    fn primary_is_the_screen_at_the_origin() {
        let d = Desktop::new(vec![Rect::new(-1440, 0, 1440, 900), Rect::new(0, 0, 1920, 1080)]);
        assert_eq!(d.primary(), &Rect::new(0, 0, 1920, 1080));
        assert_eq!(d.primary().center(), Pos::new(960.0, 540.0));
        // Nothing at the origin: the first one.
        let d = Desktop::new(vec![Rect::new(100, 100, 800, 600), Rect::new(900, 100, 800, 600)]);
        assert_eq!(d.primary().center(), Pos::new(500.0, 400.0));
    }

    #[test]
    fn side_parse() {
        assert_eq!("Right".parse::<Side>().unwrap(), Side::Right);
        assert!("up".parse::<Side>().is_err());
    }
}
