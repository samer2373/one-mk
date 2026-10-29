//! Where a neighbour's desktop sits against ours, and how the pointer maps
//! across the edge the two share.
//!
//! A neighbour sits on one [`Side`] of this computer. How its facing edge
//! lines up with ours is its [`Align`]:
//!
//! - [`Align::Stretch`] (the default, and all there was before alignment
//!   existed): the two facing edges map onto each other end to end,
//!   scaled, so all of each edge leads to the other computer.
//! - [`Align::Offset`] and [`Align::Center`]: the edges sit against each
//!   other in logical pixels, 1:1, like two monitors of one computer. The
//!   pointer only crosses where they really overlap; anywhere else the
//!   edge is a wall, as beside a real monitor.
//!
//! Either way, only the parts of an edge on the desktop's outer boundary
//! lead anywhere ([`Desktop::edge_spans`]): with several monitors, one set
//! back from the boundary (an L-shaped desktop) faces a gap.
//!
//! A [`Seam`] is one shared edge between this desktop and a neighbour's,
//! with both desktops' monitors; it maps positions both ways.

use crate::{Desktop, Pos, Side};
use std::fmt;
use std::str::FromStr;

/// How a neighbour's facing edge lines up with ours. Positions along an
/// edge are y for left and right, x for top and bottom, so "start" means
/// the top of a left or right edge and the left end of a top or bottom one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    /// The whole facing edges map onto each other, scaled to fit.
    #[default]
    Stretch,
    /// 1:1 in logical pixels: the neighbour's edge starts this many pixels
    /// after ours (further right, or further down); negative, before it.
    /// A laptop below whose left edge is 300 px right of ours is
    /// `Offset(300)`.
    Offset(i32),
    /// 1:1 in logical pixels, the two edges centred on each other.
    Center,
}

impl Align {
    /// The same alignment seen from the neighbour: its edge starting `o`
    /// pixels after ours means ours starts `o` pixels before its.
    pub fn mirror(self) -> Align {
        match self {
            Align::Offset(o) => Align::Offset(o.saturating_neg()),
            a => a,
        }
    }

    /// Whether positions map 1:1 (and so only overlapping parts cross).
    pub fn is_exact(self) -> bool {
        self != Align::Stretch
    }
}

impl fmt::Display for Align {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Align::Stretch => f.write_str("stretch"),
            Align::Center => f.write_str("center"),
            Align::Offset(o) => write!(f, "{o}"),
        }
    }
}

impl FromStr for Align {
    type Err = String;
    /// `stretch`, `center` (or `centre`), or a whole number of pixels,
    /// optionally ending in `px`. Empty means stretch, so a missing value
    /// in a settings file reads as the default.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "" | "stretch" | "none" => Ok(Align::Stretch),
            "center" | "centre" => Ok(Align::Center),
            n => n
                .strip_suffix("px")
                .unwrap_or(n)
                .trim()
                .parse::<i32>()
                .map(Align::Offset)
                .map_err(|_| format!("expected a number of pixels, center or stretch, not {s:?}")),
        }
    }
}

/// The edge this desktop shares with a neighbour on its `side`: maps
/// pointer positions from one to the other.
#[derive(Clone, Debug, PartialEq)]
pub struct Seam {
    side: Side,
    align: Align,
    local: Desktop,
    remote: Desktop,
    /// [`Desktop::edge_spans`] of our `side` edge and of the neighbour's
    /// facing (`side.opposite()`) edge.
    local_spans: Vec<(i32, i32)>,
    remote_spans: Vec<(i32, i32)>,
}

impl Seam {
    /// `remote` sits on our `side`, lined up as `align` says.
    pub fn new(local: Desktop, remote: Desktop, side: Side, align: Align) -> Seam {
        let local_spans = local.edge_spans(side);
        let remote_spans = remote.edge_spans(side.opposite());
        Seam { side, align, local, remote, local_spans, remote_spans }
    }

    /// Our edge that leads to the neighbour.
    pub fn side(&self) -> Side {
        self.side
    }
    /// The neighbour's edge that leads back here.
    pub fn home(&self) -> Side {
        self.side.opposite()
    }
    pub fn align(&self) -> Align {
        self.align
    }
    pub fn local(&self) -> &Desktop {
        &self.local
    }
    pub fn remote(&self) -> &Desktop {
        &self.remote
    }

    /// Our monitors changed.
    pub fn set_local(&mut self, local: Desktop) {
        *self = Seam::new(local, self.remote.clone(), self.side, self.align);
    }

    /// The neighbour's monitors changed.
    pub fn set_remote(&mut self, remote: Desktop) {
        *self = Seam::new(self.local.clone(), remote, self.side, self.align);
    }

    /// For 1:1 alignments, how far the neighbour's facing edge starts after
    /// ours, in pixels, measured between the two desktops' bounds.
    fn shift(&self) -> Option<f64> {
        let (l, r) = (self.local.bounds(), self.remote.bounds());
        let len = |b: crate::Rect| if self.side.along_x() { f64::from(b.w) } else { f64::from(b.h) };
        match self.align {
            Align::Stretch => None,
            Align::Offset(o) => Some(f64::from(o)),
            Align::Center => Some((len(l) - len(r)) / 2.0),
        }
    }

    /// Where the bounds of each desktop start along the edge.
    fn starts(&self) -> (f64, f64) {
        let (l, r) = (self.local.bounds(), self.remote.bounds());
        match self.side.along_x() {
            true => (f64::from(l.x), f64::from(r.x)),
            false => (f64::from(l.y), f64::from(r.y)),
        }
    }

    /// Our position `u` along the edge, as the neighbour's.
    fn to_remote(&self, u: f64) -> f64 {
        let (ls, rs) = self.starts();
        match self.shift() {
            Some(shift) => u - ls - shift + rs,
            None => uncover(&self.remote_spans, cover(&self.local_spans, u) * ratio(&self.remote_spans, &self.local_spans)),
        }
    }

    /// The neighbour's position `v` along the edge, as ours.
    fn to_local(&self, v: f64) -> f64 {
        let (ls, rs) = self.starts();
        match self.shift() {
            Some(shift) => v - rs + shift + ls,
            None => uncover(&self.local_spans, cover(&self.remote_spans, v) * ratio(&self.local_spans, &self.remote_spans)),
        }
    }

    /// The pointer leaves our desktop through the shared edge at `p`:
    /// where it enters the neighbour's, on its facing edge. `None` if
    /// nothing is there (that part of our edge doesn't face the
    /// neighbour): the pointer stays here.
    pub fn enter(&self, p: Pos) -> Option<Pos> {
        let u = Desktop::along(self.side, p);
        let v = match self.align.is_exact() {
            true => within(&self.local_spans, u).then(|| self.to_remote(u)),
            // All of our edge leads there; a point beside it (which the
            // edge barriers never report) goes to the nearest end.
            false => Some(self.to_remote(snap(&self.local_spans, u))),
        };
        v.filter(|v| within(&self.remote_spans, *v)).map(|v| self.remote.edge_point(self.home(), v, 0.0))
    }

    /// The pointer leaves the neighbour's desktop through its facing edge
    /// at `p`: where it arrives here, `inset` pixels inside our edge.
    /// `None` if that part of its edge doesn't face us: it stays there.
    pub fn exit(&self, p: Pos, inset: f64) -> Option<Pos> {
        let v = Desktop::along(self.home(), p);
        if !within(&self.remote_spans, v) {
            return None;
        }
        let u = self.to_local(v);
        within(&self.local_spans, u).then(|| self.local.edge_point(self.side, u, inset))
    }

    /// Where the pointer comes home when it is brought back without
    /// crossing (a shortcut, a timeout): the point of our edge facing `p`
    /// on the neighbour, or the nearest one that faces it at all.
    pub fn home_near(&self, p: Pos, inset: f64) -> Pos {
        let v = snap(&self.remote_spans, Desktop::along(self.home(), p));
        let u = self.to_local(v);
        let shared = self.shared();
        let u = match shared.is_empty() {
            true => snap(&self.local_spans, u),
            false => snap_f(&shared, u),
        };
        self.local.edge_point(self.side, u, inset)
    }

    /// The parts of our edge that lead to the neighbour, as `[start, end)`
    /// ranges along it (y for left and right, x for top and bottom).
    /// Empty if the two edges don't overlap anywhere.
    pub fn shared(&self) -> Vec<(f64, f64)> {
        let local = self.local_spans.iter().map(|&(a, b)| (f64::from(a), f64::from(b)));
        if !self.align.is_exact() {
            return local.collect();
        }
        let remote: Vec<(f64, f64)> =
            self.remote_spans.iter().map(|&(a, b)| (self.to_local(f64::from(a)), self.to_local(f64::from(b)))).collect();
        let mut out = Vec::new();
        for (a, b) in local {
            for &(c, d) in &remote {
                let (lo, hi) = (a.max(c), b.min(d));
                if lo < hi {
                    out.push((lo, hi));
                }
            }
        }
        out
    }
}

/// Whether two sets of `[start, end)` ranges overlap anywhere: two
/// neighbours on the same side whose [`Seam::shared`] parts overlap claim
/// the same stretch of edge.
pub fn overlaps(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
    a.iter().any(|&(p, q)| b.iter().any(|&(r, s)| p.max(r) < q.min(s)))
}

/// Just below the end of a half-open range, the way [`crate::Rect::clamp`]
/// stays inside a monitor.
const IN: f64 = 1.0 / 256.0;

fn within(spans: &[(i32, i32)], v: f64) -> bool {
    spans.iter().any(|&(a, b)| v >= f64::from(a) && v < f64::from(b))
}

/// The nearest point to `v` inside `spans`.
fn snap(spans: &[(i32, i32)], v: f64) -> f64 {
    let f: Vec<(f64, f64)> = spans.iter().map(|&(a, b)| (f64::from(a), f64::from(b))).collect();
    snap_f(&f, v)
}

fn snap_f(spans: &[(f64, f64)], v: f64) -> f64 {
    spans
        .iter()
        .map(|&(a, b)| v.clamp(a, (b - IN).max(a)))
        .min_by(|x, y| (x - v).abs().total_cmp(&(y - v).abs()))
        .unwrap_or(v)
}

/// Total length of `spans`.
fn total(spans: &[(i32, i32)]) -> f64 {
    spans.iter().map(|&(a, b)| f64::from(b - a)).sum()
}

fn ratio(to: &[(i32, i32)], from: &[(i32, i32)]) -> f64 {
    let from = total(from);
    if from > 0.0 { total(to) / from } else { 0.0 }
}

/// How far along `spans` (counting only the spans, skipping the gaps
/// between them) `v` is; a point in a gap counts as the nearest span end.
fn cover(spans: &[(i32, i32)], v: f64) -> f64 {
    let v = snap(spans, v);
    let mut done = 0.0;
    for &(a, b) in spans {
        let (a, b) = (f64::from(a), f64::from(b));
        if v < b {
            return done + (v - a).max(0.0);
        }
        done += b - a;
    }
    done
}

/// The inverse of [`cover`]: the point `c` along `spans`, kept inside them.
fn uncover(spans: &[(i32, i32)], c: f64) -> f64 {
    let mut left = c.max(0.0);
    for &(a, b) in spans {
        let len = f64::from(b - a);
        if left < len {
            return f64::from(a) + left;
        }
        left -= len;
    }
    spans.last().map_or(c, |&(_, b)| f64::from(b) - IN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    fn one(w: i32, h: i32) -> Desktop {
        Desktop::new(vec![Rect::new(0, 0, w, h)])
    }

    #[test]
    fn align_parses_prints_and_mirrors() {
        for (text, a) in [("300", Align::Offset(300)), ("-42px", Align::Offset(-42)), ("Center", Align::Center), ("centre", Align::Center), ("stretch", Align::Stretch), ("", Align::Stretch)] {
            assert_eq!(text.parse::<Align>().unwrap(), a, "{text}");
        }
        assert!("left".parse::<Align>().is_err());
        assert!("1.5".parse::<Align>().is_err());
        for a in [Align::Offset(-7), Align::Offset(0), Align::Center, Align::Stretch] {
            assert_eq!(a.to_string().parse::<Align>().unwrap(), a);
            assert_eq!(a.mirror().mirror(), a);
        }
        assert_eq!(Align::Offset(300).mirror(), Align::Offset(-300));
        assert_eq!(Align::Offset(i32::MIN).mirror(), Align::Offset(i32::MAX));
    }

    #[test]
    fn stretch_maps_whole_edges_like_before() {
        // A 1080p desktop with a 1440p one on its right.
        let s = Seam::new(one(1920, 1080), one(2560, 1440), Side::Right, Align::Stretch);
        assert_eq!(s.enter(Pos::new(1919.0, 540.0)), Some(Pos::new(0.0, 720.0)));
        assert_eq!(s.exit(Pos::new(-3.0, 720.0), 2.0), Some(Pos::new(1917.0, 540.0)));
        assert_eq!(s.shared(), vec![(0.0, 1080.0)]);
    }

    #[test]
    fn offset_maps_one_to_one_and_only_where_edges_overlap() {
        // A small laptop below us, its left edge 300 px right of ours.
        let s = Seam::new(one(1920, 1080), one(1280, 800), Side::Bottom, Align::Offset(300));
        assert_eq!(s.enter(Pos::new(1000.0, 1079.0)), Some(Pos::new(700.0, 0.0)));
        assert_eq!(s.enter(Pos::new(300.0, 1079.0)), Some(Pos::new(0.0, 0.0)));
        assert_eq!(s.enter(Pos::new(299.5, 1079.0)), None, "left of the laptop");
        assert_eq!(s.enter(Pos::new(1580.0, 1079.0)), None, "right of the laptop");
        assert_eq!(s.shared(), vec![(300.0, 1580.0)]);
        // And back up: the laptop's top edge maps to ours shifted by 300.
        assert_eq!(s.exit(Pos::new(700.0, -1.0), 2.0), Some(Pos::new(1000.0, 1077.0)));

        // The mirror, from the laptop: we are on top, our left edge 300 px
        // left of its.
        let m = Seam::new(one(1280, 800), one(1920, 1080), Side::Top, Align::Offset(300).mirror());
        assert_eq!(m.enter(Pos::new(700.0, 0.0)), Some(Pos::new(1000.0, 1079.0)));
        assert_eq!(m.shared(), vec![(0.0, 1280.0)], "all of the laptop's top edge faces us");

        // A laptop hanging off the end: only the overlap crosses, both ways.
        let s = Seam::new(one(1920, 1080), one(1280, 800), Side::Bottom, Align::Offset(1500));
        assert_eq!(s.shared(), vec![(1500.0, 1920.0)]);
        assert_eq!(s.exit(Pos::new(100.0, -1.0), 0.0), Some(Pos::new(1600.0, 1079.0)));
        assert_eq!(s.exit(Pos::new(500.0, -1.0), 0.0), None, "under nothing of ours");
        assert_eq!(s.home_near(Pos::new(900.0, 400.0), 2.0), Pos::new(1920.0 - IN, 1077.0));
    }

    #[test]
    fn center_lines_up_the_middles() {
        let s = Seam::new(one(1920, 1080), one(1280, 800), Side::Left, Align::Center);
        // 1080 tall here, 800 there: theirs starts 140 px below ours.
        assert_eq!(s.shared(), vec![(140.0, 940.0)]);
        assert_eq!(s.enter(Pos::new(0.0, 540.0)), Some(Pos::new(1279.0, 400.0)));
        assert_eq!(s.enter(Pos::new(0.0, 100.0)), None);
        let m = Seam::new(one(1280, 800), one(1920, 1080), Side::Right, Align::Center);
        assert_eq!(m.enter(Pos::new(1279.0, 400.0)), Some(Pos::new(0.0, 540.0)));
    }

    #[test]
    fn multi_monitor_crossing_arrives_on_the_right_monitor() {
        // Us: a 1440p monitor with a portrait 1080x1920 one on its right,
        // starting 240 px higher; only the portrait one reaches our right
        // edge.
        let us = Desktop::new(vec![Rect::new(0, 0, 2560, 1440), Rect::new(2560, -240, 1080, 1920)]);
        // Them, to our right: two 1080p monitors stacked, the lower one
        // its primary at the origin, the upper one above it.
        let them = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(0, -1080, 1920, 1080)]);
        let s = Seam::new(us.clone(), them.clone(), Side::Right, Align::Offset(-600));
        // From the portrait monitor (not our primary), near its top.
        let at = Pos::new(3639.0, -200.0);
        let p = s.enter(at).unwrap();
        assert_eq!(p, Pos::new(0.0, -200.0 + 240.0 + 600.0 - 1080.0));
        assert_eq!(them.screen_at(p), Some(&Rect::new(0, -1080, 1920, 1080)), "arrives on the upper monitor");
        let back = s.exit(Pos::new(-1.0, p.y), 0.0).unwrap();
        assert_eq!(back, at);
        assert_eq!(us.screen_at(back), Some(&Rect::new(2560, -240, 1080, 1920)));
    }

    #[test]
    fn stretch_skips_gaps_between_monitors() {
        // Two monitors along the bottom with a gap: the neighbour below
        // spreads over both, never into the gap.
        let us = Desktop::new(vec![Rect::new(0, 0, 1000, 800), Rect::new(1200, 0, 1000, 800)]);
        let s = Seam::new(us.clone(), one(1000, 600), Side::Bottom, Align::Stretch);
        assert_eq!(s.enter(Pos::new(999.0, 799.0)).unwrap().x, 499.5);
        assert_eq!(s.enter(Pos::new(1200.0, 799.0)).unwrap().x, 500.0);
        let back = s.exit(Pos::new(500.0, -1.0), 0.0).unwrap();
        assert_eq!(back, Pos::new(1200.0, 799.0));
        assert!(us.screen_at(back).is_some());
    }

    fn dist(a: Pos, b: Pos) -> f64 {
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
    }

    /// A tiny deterministic generator, so failures reproduce.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> i32 {
            (self.next() % n) as i32
        }
        fn pick<T: Copy>(&mut self, of: &[T]) -> T {
            of[self.next() as usize % of.len()]
        }
    }

    /// A connected desktop of 1 to 4 monitors of assorted sizes (logical
    /// pixels, as a scaled monitor reports), each placed against a side of
    /// one already there at a random offset, never overlapping; the
    /// primary is not always the one at the origin's corner.
    fn random_desktop(rng: &mut Rng) -> Desktop {
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

    fn random_align(rng: &mut Rng) -> Align {
        match rng.below(4) {
            0 => Align::Stretch,
            1 => Align::Center,
            _ => Align::Offset(rng.below(6000) - 3000),
        }
    }

    /// For many random pairs of desktops and alignments: every crossing
    /// lands on a monitor, at the far side's facing edge; crossing there
    /// and straight back returns to the same point (within rounding); and
    /// with 1:1 alignments, a point crosses exactly when the two edges
    /// overlap there.
    #[test]
    fn random_layouts_cross_and_return() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut crossed = 0;
        for _ in 0..3000 {
            let (us, them) = (random_desktop(&mut rng), random_desktop(&mut rng));
            let side = rng.pick(&Side::ALL);
            let align = random_align(&mut rng);
            let s = Seam::new(us.clone(), them.clone(), side, align);
            let mirror = Seam::new(them.clone(), us.clone(), side.opposite(), align.mirror());
            assert_eq!(s.shared().is_empty(), mirror.shared().is_empty());
            let ub = us.bounds();
            let (lo, hi) = if side.along_x() { (ub.x, ub.right()) } else { (ub.y, ub.bottom()) };
            for _ in 0..40 {
                let u = f64::from(lo) + f64::from(rng.below((hi - lo) as u64 * 4)) / 4.0;
                let at = us.edge_point(side, u, 0.0);
                // `at` is on the boundary line (the edge point of a gap is
                // moved onto a monitor set back from it).
                let on_edge = within(&us.edge_spans(side), u);
                let shared = s.shared().iter().any(|&(a, b)| u >= a && u < b);
                let Some(p) = s.enter(at) else {
                    assert!(align.is_exact() && !(on_edge && shared), "{side} {align} {u}: {us:?} {them:?}");
                    continue;
                };
                crossed += 1;
                assert!(them.screen_at(p).is_some(), "lands on a monitor: {p:?} {them:?}");
                let tb = them.bounds();
                let line = match side.opposite() {
                    Side::Left => p.x == f64::from(tb.x),
                    Side::Right => p.x == f64::from(tb.right() - 1),
                    Side::Top => p.y == f64::from(tb.y),
                    Side::Bottom => p.y == f64::from(tb.bottom() - 1),
                };
                assert!(line, "arrives on the facing edge, not inside: {p:?} {them:?}");
                // Through the far side's facing edge, straight back.
                let back = s.exit(p, 0.0).expect("the way back is open");
                assert!(us.screen_at(back).is_some());
                if on_edge {
                    assert!(dist(back, at) < 1e-6, "{u}: {at:?} -> {p:?} -> {back:?}");
                }
                // The mirror seam (what the other computer computes from
                // the synced layout) agrees.
                if align.is_exact() {
                    assert_eq!(mirror.enter(p).map(|q| Desktop::along(side, q)), Some(Desktop::along(side, back)));
                }
            }
        }
        assert!(crossed > 10_000, "only {crossed} crossings");
    }
}
