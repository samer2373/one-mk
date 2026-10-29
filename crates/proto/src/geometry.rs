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

    /// Position along `side` as a fraction in [0, 1], used to carry the
    /// pointer across machines whose desktops have different sizes.
    pub fn edge_fraction(&self, side: Side, p: Pos) -> f64 {
        let b = self.bounds();
        let t = match side {
            Side::Left | Side::Right => (p.y - f64::from(b.y)) / f64::from(b.h),
            Side::Top | Side::Bottom => (p.x - f64::from(b.x)) / f64::from(b.w),
        };
        t.clamp(0.0, 1.0)
    }

    /// A point just inside `side` at fraction `t`, `inset` pixels from the edge.
    pub fn point_on_edge(&self, side: Side, t: f64, inset: f64) -> Pos {
        let b = self.bounds();
        let along_y = f64::from(b.y) + t.clamp(0.0, 1.0) * f64::from(b.h);
        let along_x = f64::from(b.x) + t.clamp(0.0, 1.0) * f64::from(b.w);
        let p = match side {
            Side::Left => Pos::new(f64::from(b.x) + inset, along_y),
            Side::Right => Pos::new(f64::from(b.right()) - 1.0 - inset, along_y),
            Side::Top => Pos::new(along_x, f64::from(b.y) + inset),
            Side::Bottom => Pos::new(along_x, f64::from(b.bottom()) - 1.0 - inset),
        };
        self.clamp(p)
    }

    /// Where `p` is along the `side` edge: y for left and right, x for top
    /// and bottom.
    pub fn along(side: Side, p: Pos) -> f64 {
        if side.along_x() { p.x } else { p.y }
    }

    /// The point `along` the `side` edge of the desktop's bounds, `inset`
    /// pixels inwards (the same line [`Desktop::point_on_edge`] uses),
    /// moved onto the nearest monitor if it is not on one.
    pub fn edge_point(&self, side: Side, along: f64, inset: f64) -> Pos {
        let b = self.bounds();
        let p = match side {
            Side::Left => Pos::new(f64::from(b.x) + inset, along),
            Side::Right => Pos::new(f64::from(b.right()) - 1.0 - inset, along),
            Side::Top => Pos::new(along, f64::from(b.y) + inset),
            Side::Bottom => Pos::new(along, f64::from(b.bottom()) - 1.0 - inset),
        };
        self.clamp(p)
    }

    /// The stretches of the `side` edge that lie on the desktop's outer
    /// boundary (the line through its outermost monitors), as sorted,
    /// merged `[start, end)` ranges along the edge (see [`Desktop::along`]).
    /// Only these can touch a neighbour on that side: a monitor set back
    /// from the line (one arm of an L-shaped desktop, a shorter monitor
    /// beside a taller one) faces a gap, exactly as it would beside a real
    /// monitor there.
    pub fn edge_spans(&self, side: Side) -> Vec<(i32, i32)> {
        let mut spans: Vec<(i32, i32)> = self
            .outer_edges(side)
            .map(|r| if side.along_x() { (r.x, r.right()) } else { (r.y, r.bottom()) })
            .filter(|(a, b)| a < b)
            .collect();
        spans.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        merged
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
}

fn dist2(a: Pos, b: Pos) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

#[cfg(test)]
mod tests {
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
    fn outer_edges() {
        let d = dual();
        assert_eq!(d.outer_edges(Side::Right).count(), 1);
        assert_eq!(d.outer_edges(Side::Top).count(), 2);
        assert_eq!(d.outer_edges(Side::Bottom).count(), 1);
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
    fn edge_spans_of_an_l_shaped_desktop() {
        // A wide monitor on top, a narrower one below it, flush left, and a
        // third to the right of the narrow one, set back from the right.
        let d = Desktop::new(vec![
            Rect::new(0, 0, 2560, 1440),
            Rect::new(0, 1440, 1280, 1024),
            Rect::new(1280, 1440, 800, 600),
        ]);
        assert_eq!(d.edge_spans(Side::Left), vec![(0, 2464)]);
        assert_eq!(d.edge_spans(Side::Right), vec![(0, 1440)], "the lower monitors don't reach the right");
        assert_eq!(d.edge_spans(Side::Top), vec![(0, 2560)]);
        assert_eq!(d.edge_spans(Side::Bottom), vec![(0, 1280)], "the short monitor stops above the bottom");
        // Two monitors side by side with a gap along the shared bottom line.
        let d = Desktop::new(vec![Rect::new(0, 0, 1000, 800), Rect::new(1200, 0, 1000, 800)]);
        assert_eq!(d.edge_spans(Side::Bottom), vec![(0, 1000), (1200, 2200)]);
        let p = d.edge_point(Side::Bottom, 1100.0, 2.0);
        assert!(d.screen_at(p).is_some(), "a point in the gap moves onto a monitor: {p:?}");
    }

    #[test]
    fn side_parse() {
        assert_eq!("Right".parse::<Side>().unwrap(), Side::Right);
        assert!("up".parse::<Side>().is_err());
    }
}
