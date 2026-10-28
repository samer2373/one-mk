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

    #[test]
    fn side_parse() {
        assert_eq!("Right".parse::<Side>().unwrap(), Side::Right);
        assert!("up".parse::<Side>().is_err());
    }
}
