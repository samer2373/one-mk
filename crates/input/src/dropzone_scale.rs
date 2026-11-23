// The drop zones come in the space the daemon sees (`win_logic::Space::logical`: physical pixels
// divided by the primary monitor's scale), but the Windows hook reports, and the window is placed
// in, physical pixels. This is the way back, for a rectangle.

use one_mk_proto::Rect;

/// A rectangle in the reported space to physical pixels. Edges are scaled, not sizes, as
/// `Space::new` does, so a strip stays against the edge it was built for, and at least 1 pixel.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(super) fn to_physical(rect: Rect, scale: f64) -> Rect {
    if !scale.is_finite() || scale <= 0.0 { return rect; }
    let s = |v: i32| (f64::from(v) * scale).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let (x, y) = (s(rect.x), s(rect.y));
    Rect::new(x, y, (s(rect.right()) - x).max(1), (s(rect.bottom()) - y).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win_logic::Space;

    #[test]
    fn a_right_edge_strip_stays_at_the_right_edge_of_a_scaled_screen() {
        // One 3840x2160 monitor at 200%: the daemon sees 1920x1080.
        let space = Space::new(vec![Rect::new(0, 0, 3840, 2160)], 192);
        let logical = space.logical().bounds();
        assert_eq!((logical.w, logical.h), (1920, 1080));
        // The strip the daemon builds inside the right edge, 8 reported pixels wide.
        let zone = Rect::new(logical.right() - 8, 100, 8, 400);
        let physical = to_physical(zone, space.scale());
        assert_eq!(physical, Rect::new(3824, 200, 16, 800));
        assert_eq!(physical.right(), space.phys().bounds().right());
        // The last physical pixel column is inside it; the middle of the screen is not.
        assert!(physical.contains(one_mk_proto::Pos::new(3839.0, 300.0)));
        assert!(!physical.contains(one_mk_proto::Pos::new(1919.0, 300.0)));
    }

    #[test]
    fn unscaled_screens_and_odd_scales_keep_a_visible_strip() {
        let zone = Rect::new(5, 6, 7, 8);
        assert_eq!(to_physical(zone, 1.0), zone);
        assert_eq!(to_physical(zone, 0.0), zone);
        assert!(to_physical(Rect::new(0, 0, 1, 1), 1.25).w >= 1);
        // A secondary monitor to the left (negative coordinates) scales from the same origin.
        assert_eq!(to_physical(Rect::new(-960, 0, 8, 100), 2.0), Rect::new(-1920, 0, 16, 200));
    }
}
