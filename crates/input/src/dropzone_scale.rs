// The drop zones come in the space the daemon sees (`win_logic::Space::logical`: physical pixels
// divided by the primary monitor's scale), but the Windows hook reports, and the window is placed
// in, physical pixels. This is the way back, for a rectangle.

use one_mk_proto::Rect;

/// A rectangle in the reported space to physical pixels. Edges are scaled, not sizes, as
/// `Space::new` does, so a strip stays against the edge it was built for, and at least 1 pixel.
/// `screens` are the monitors in physical pixels: an edge that is where one of them ends in the
/// reported space snaps to that physical edge, since scaling back a rounded edge can land a
/// pixel short of it (1920 / 2.25 rounds to 853, and 853 * 2.25 is 1919), outside the screen.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(super) fn to_physical(rect: Rect, scale: f64, screens: &[Rect]) -> Rect {
    if !scale.is_finite() || scale <= 0.0 { return rect; }
    let down = |v: i32| (f64::from(v) / scale).round() as i32;
    let edge = |v: i32, physical: &dyn Fn(&Rect) -> [i32; 2]| {
        screens.iter().flat_map(physical).find(|&p| down(p) == v)
            .unwrap_or_else(|| (f64::from(v) * scale).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
    };
    let across = |r: &Rect| [r.x, r.right()];
    let down_the = |r: &Rect| [r.y, r.bottom()];
    let (x, y) = (edge(rect.x, &across), edge(rect.y, &down_the));
    Rect::new(x, y, (edge(rect.right(), &across) - x).max(1), (edge(rect.bottom(), &down_the) - y).max(1))
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
        let physical = to_physical(zone, space.scale(), &space.phys().screens);
        assert_eq!(physical, Rect::new(3824, 200, 16, 800));
        assert_eq!(physical.right(), space.phys().bounds().right());
        // The last physical pixel column is inside it; the middle of the screen is not.
        assert!(physical.contains(one_mk_proto::Pos::new(3839.0, 300.0)));
        assert!(!physical.contains(one_mk_proto::Pos::new(1919.0, 300.0)));
    }

    #[test]
    fn a_strip_at_the_edge_of_a_fractionally_scaled_screen_reaches_the_last_pixel() {
        // 1920x1080 primary at 225% (DPI 216) next to a second monitor: the reported edges are rounded.
        let screens = vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 2560, 1440)];
        let space = Space::new(screens.clone(), 216);
        let logical = space.logical().screens[0];
        assert_eq!(logical.right(), 853);
        let zone = Rect::new(logical.right() - 8, 0, 8, logical.h);
        let physical = to_physical(zone, space.scale(), &screens);
        // Plain rounding gave 1919 here, and the pixel the pointer is on at the edge missed the strip.
        assert_eq!(physical.right(), 1920);
        assert!(physical.contains(one_mk_proto::Pos::new(1919.0, 500.0)));
        assert_eq!((physical.y, physical.bottom()), (0, 1080));
        // The strip against the far monitor's right edge and the bottom edge of the taller one.
        let far = space.logical().screens[1];
        let zone = Rect::new(far.right() - 8, far.bottom() - 8, 8, 8);
        let physical = to_physical(zone, space.scale(), &screens);
        assert_eq!((physical.right(), physical.bottom()), (4480, 1440));
    }

    #[test]
    fn unscaled_screens_and_odd_scales_keep_a_visible_strip() {
        let zone = Rect::new(5, 6, 7, 8);
        assert_eq!(to_physical(zone, 1.0, &[]), zone);
        assert_eq!(to_physical(zone, 0.0, &[]), zone);
        assert!(to_physical(Rect::new(0, 0, 1, 1), 1.25, &[]).w >= 1);
        // A secondary monitor to the left (negative coordinates) scales from the same origin.
        assert_eq!(to_physical(Rect::new(-960, 0, 8, 100), 2.0, &[]), Rect::new(-1920, 0, 16, 200));
    }
}
