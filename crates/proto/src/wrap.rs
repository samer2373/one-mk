//! Edges where no computer sits (see `docs/EDGE-PUSH.md`): which pieces of
//! this desktop's outline lead nowhere, and where the pointer comes back
//! in when it wraps round through one of them.

use crate::{Desktop, Edge, Pos, Side};

/// The pieces of `desktop`'s outline on `sides` (every exposed edge, see
/// [`Desktop::exposed_edges`], so also one set back from the bounding box)
/// that no edge in `taken` covers. `taken` are the edges leading to other
/// computers: nothing here ever overlaps one of them.
pub fn free_edges(desktop: &Desktop, sides: &[Side], taken: &[Edge]) -> Vec<Edge> {
    let mut out = Vec::new();
    for &side in sides {
        for e in desktop.exposed_edges(side) {
            let mut spans = vec![(e.lo, e.hi)];
            for t in taken.iter().filter(|t| t.side == side && t.at == e.at) {
                spans = spans
                    .into_iter()
                    .flat_map(|(lo, hi)| [(lo, hi.min(t.lo)), (lo.max(t.hi), hi)])
                    .filter(|(lo, hi)| lo < hi)
                    .collect();
            }
            out.extend(spans.into_iter().map(|(lo, hi)| Edge { side, at: e.at, lo, hi }));
        }
    }
    out
}

/// Where the pointer comes back in after leaving `desktop` through its
/// `side` edge at `p`: on the same row (left and right edges) or column
/// (top and bottom), `inset` px inside the far end of the next run of
/// screens in the direction it went, and from the last run round to the
/// first. On a desktop whose rows are one run each (any desktop without a
/// hole or a notch), that is simply the opposite edge at the same height:
/// out through the right of the rightmost monitor, in at the left of the
/// leftmost one on that row. Wrapping back the other way returns to within
/// `inset` px of `p`. The result is always on a screen.
pub fn wrap_point(desktop: &Desktop, side: Side, p: Pos, inset: f64) -> Pos {
    const IN: f64 = 1.0 / 256.0;
    if !(p.x.is_finite() && p.y.is_finite()) {
        return desktop.primary().center();
    }
    let p = desktop.clamp(p);
    let rows = side.along_x();
    // Along the edge, and across it (the direction of travel).
    let (along, across) = if rows { (p.x, p.y) } else { (p.y, p.x) };
    let mut runs: Vec<(f64, f64)> = desktop
        .screens
        .iter()
        .filter_map(|r| {
            let (lo, hi, from, to) =
                if rows { (r.x, r.right(), r.y, r.bottom()) } else { (r.y, r.bottom(), r.x, r.right()) };
            (f64::from(lo) <= along && along < f64::from(hi)).then(|| (f64::from(from), f64::from(to)))
        })
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(runs.len());
    for (lo, hi) in runs {
        match merged.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    let n = merged.len();
    let Some(here) = merged.iter().position(|&(lo, hi)| lo <= across && across < hi) else {
        return p;
    };
    let forward = matches!(side, Side::Right | Side::Bottom);
    let next = if forward { (here + 1) % n } else { (here + n - 1) % n };
    let Some(&(lo, hi)) = merged.get(next) else { return p };
    let to = if forward { (lo + inset).min(hi - IN) } else { (hi - 1.0 - inset).max(lo) };
    desktop.clamp(if rows { Pos::new(along, to) } else { Pos::new(to, along) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;
    use crate::geometry::tests::{Rng, random_desktop};

    const INSET: f64 = 2.0;

    fn dual() -> Desktop {
        // 1920x1080 on the left, a taller 1440x2560 portrait monitor on the right.
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1440, 2560)])
    }

    #[test]
    fn one_monitor_wraps_straight_across() {
        let d = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
        assert_eq!(wrap_point(&d, Side::Right, Pos::new(1919.0, 300.0), INSET), Pos::new(2.0, 300.0));
        assert_eq!(wrap_point(&d, Side::Left, Pos::new(0.0, 300.0), INSET), Pos::new(1917.0, 300.0));
        assert_eq!(wrap_point(&d, Side::Top, Pos::new(700.5, 0.0), INSET), Pos::new(700.5, 1077.0));
        assert_eq!(wrap_point(&d, Side::Bottom, Pos::new(700.5, 1079.0), INSET), Pos::new(700.5, 2.0));
    }

    #[test]
    fn the_leftmost_monitor_on_that_row_takes_it() {
        let d = dual();
        // Beside both monitors: in at the small one's left edge.
        assert_eq!(wrap_point(&d, Side::Right, Pos::new(3359.0, 500.0), INSET), Pos::new(2.0, 500.0));
        // Below the small one: only the tall one is on that row, so its own
        // left edge (the outline there) is where the pointer comes in.
        assert_eq!(wrap_point(&d, Side::Right, Pos::new(3359.0, 2000.0), INSET), Pos::new(1922.0, 2000.0));
        assert_eq!(wrap_point(&d, Side::Left, Pos::new(1920.0, 2000.0), INSET), Pos::new(3357.0, 2000.0));
        // Down through the small one's bottom: round to the top of that column.
        assert_eq!(wrap_point(&d, Side::Bottom, Pos::new(100.0, 1079.0), INSET), Pos::new(100.0, 2.0));
    }

    #[test]
    fn a_gap_in_a_row_is_jumped() {
        // A U: two towers joined by a wide monitor below; the top rows have a gap.
        let d = Desktop::new(vec![Rect::new(0, 0, 1000, 2000), Rect::new(0, 2000, 3000, 1000), Rect::new(2000, 0, 1000, 2000)]);
        assert_eq!(wrap_point(&d, Side::Right, Pos::new(999.0, 500.0), INSET), Pos::new(2002.0, 500.0), "into the gap: next tower");
        assert_eq!(wrap_point(&d, Side::Right, Pos::new(2999.0, 500.0), INSET), Pos::new(2.0, 500.0), "the last: round to the first");
        assert_eq!(wrap_point(&d, Side::Left, Pos::new(2000.0, 500.0), INSET), Pos::new(997.0, 500.0));
    }

    #[test]
    fn hostile_positions_still_land_on_a_screen() {
        let d = dual();
        for p in [
            Pos::new(f64::NAN, 3.0),
            Pos::new(3.0, f64::INFINITY),
            Pos::new(f64::NEG_INFINITY, f64::NAN),
            Pos::new(1e300, -1e300),
            Pos::new(-5.0, 5000.0),
            Pos::new(2500.0, 1500.0),
        ] {
            for side in Side::ALL {
                let q = wrap_point(&d, side, p, INSET);
                assert!(d.screen_at(q).is_some(), "{p:?} {side} -> {q:?}");
            }
        }
        // An inset wider than the monitor stays on it.
        let thin = Desktop::new(vec![Rect::new(0, 0, 3, 100)]);
        assert!(thin.screen_at(wrap_point(&thin, Side::Right, Pos::new(2.0, 50.0), 50.0)).is_some());
        assert!(thin.screen_at(wrap_point(&thin, Side::Left, Pos::new(0.0, 50.0), 50.0)).is_some());
    }

    /// The point just inside `e` (on its last row or column of pixels) at `v` along it.
    fn just_inside(e: &Edge, v: f64) -> Pos {
        let line = match e.side {
            Side::Left | Side::Top => f64::from(e.at),
            Side::Right | Side::Bottom => f64::from(e.at - 1),
        };
        if e.side.along_x() { Pos::new(v, line) } else { Pos::new(line, v) }
    }

    /// For many random multi-monitor and L-shaped desktops, out through
    /// every exposed edge: the pointer lands on a screen, on the same row
    /// or column, right inside the outline on the other side, and wrapping
    /// back returns it to within the inset of where it left.
    #[test]
    fn random_desktops_wrap_onto_a_screen_and_back() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let mut checked = 0;
        for _ in 0..3000 {
            let d = random_desktop(&mut rng);
            let side = rng.pick(&Side::ALL);
            for e in d.exposed_edges(side) {
                let v = f64::from(e.lo) + f64::from(rng.below((e.hi - e.lo) as u64)) + 0.5;
                let p = just_inside(&e, v);
                assert!(d.screen_at(p).is_some(), "{p:?} on {d:?}");
                let q = wrap_point(&d, side, p, INSET);
                assert!(d.screen_at(q).is_some(), "lands on a screen: {p:?} {side} -> {q:?} on {d:?}");
                let (pa, qa) = if side.along_x() { (p.x, q.x) } else { (p.y, q.y) };
                assert_eq!(pa, qa, "same row or column");
                // One step further back (towards where it came in from) is off the desktop.
                let back = INSET + 1.0;
                let beyond = match side {
                    Side::Right => Pos::new(q.x - back, q.y),
                    Side::Left => Pos::new(q.x + back, q.y),
                    Side::Bottom => Pos::new(q.x, q.y - back),
                    Side::Top => Pos::new(q.x, q.y + back),
                };
                let wide = |r: &Rect| r.w as f64 > back && r.h as f64 > back;
                if d.screens.iter().all(wide) {
                    assert!(d.screen_at(beyond).is_none(), "right inside the outline: {q:?} {side} on {d:?}");
                }
                let home = wrap_point(&d, side.opposite(), q, INSET);
                let dist = (home.x - p.x).hypot(home.y - p.y);
                assert!(dist <= INSET + 1e-9, "wrap and back: {p:?} -> {q:?} -> {home:?} on {d:?}");
                checked += 1;
            }
        }
        assert!(checked > 5000, "only {checked} wraps checked");
    }

    #[test]
    fn free_edges_leave_out_what_leads_to_a_computer() {
        let d = dual();
        let peer = Edge { side: Side::Right, at: 3360, lo: 500, hi: 1500 };
        let free = free_edges(&d, &Side::ALL, &[peer]);
        let right: Vec<_> = free.iter().filter(|e| e.side == Side::Right).copied().collect();
        let piece = |lo, hi| Edge { side: Side::Right, at: 3360, lo, hi };
        assert_eq!(right, vec![piece(0, 500), piece(1500, 2560)]);
        // The small monitor's bottom is free; its right edge is not on
        // the outline at all (the tall monitor is beyond it).
        assert!(free.contains(&Edge { side: Side::Bottom, at: 1080, lo: 0, hi: 1920 }));
        assert!(!free.iter().any(|e| e.side == Side::Right && e.at == 1920));
        // Left: the small monitor's, and the tall one's below it.
        let left = vec![Edge { side: Side::Left, at: 0, lo: 0, hi: 1080 }, Edge { side: Side::Left, at: 1920, lo: 1080, hi: 2560 }];
        assert_eq!(free_edges(&d, &[Side::Left], &[]), left);
        assert!(free_edges(&d, &[], &[]).is_empty());
    }

    /// For random desktops and random computers placed against them: a
    /// free edge never overlaps an edge leading to a computer, and the two
    /// together cover the whole outline.
    #[test]
    fn random_free_edges_never_hide_a_neighbour() {
        let mut rng = Rng(0x1357_9BDF_2468_ACE0);
        for _ in 0..2000 {
            let (us, them) = (random_desktop(&mut rng), random_desktop(&mut rng));
            let side = rng.pick(&Side::ALL);
            let mine = us.exposed_edges(side);
            let e = mine[rng.below(mine.len() as u64) as usize];
            let theirs = them.exposed_edges(side.opposite());
            let t = theirs[rng.below(theirs.len() as u64) as usize];
            let across = e.at - t.at;
            let along = e.lo - t.lo + rng.below((e.hi - e.lo + t.hi - t.lo) as u64) - (t.hi - t.lo) + 1;
            let offset = if side.along_x() { (along, across) } else { (across, along) };
            let taken = crate::touching(&us, &them, side, offset);
            let free = free_edges(&us, &Side::ALL, &taken);
            for f in &free {
                assert!(f.lo < f.hi);
                for t in &taken {
                    let overlap = f.side == t.side && f.at == t.at && f.lo < t.hi && t.lo < f.hi;
                    assert!(!overlap, "{f:?} hides {t:?} on {us:?}");
                }
            }
            let len = |v: &[Edge], s: Side| v.iter().filter(|e| e.side == s).map(|e| i64::from(e.hi - e.lo)).sum::<i64>();
            for s in Side::ALL {
                let outline = len(&us.exposed_edges(s), s);
                assert_eq!(len(&free, s) + len(&taken, s), outline, "{s} on {us:?} with {taken:?}");
            }
        }
    }
}
