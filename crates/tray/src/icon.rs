//! The tray icons, drawn in code (no image files, no image decoder): two
//! screen outlines side by side on a transparent background, with a bridge
//! over their top edges. Amber is the one signal colour: it appears only
//! when computers are connected.

const GREY: [f32; 3] = [0xe8 as f32, 0xe6 as f32, 0xe1 as f32];
const AMBER: [f32; 3] = [0xff as f32, 0xb5 as f32, 0x47 as f32];

/// What the icon says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// The daemon is not running.
    Off,
    /// Running, but no computer is online.
    Idle,
    /// At least one computer is online.
    Connected,
    /// This computer controls another, or is being controlled.
    Controlling,
}

/// Coverage (0..=1) of a rounded rectangle at pixel centre (x, y), anti-aliased over one pixel.
fn rrect(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    let (dx, dy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    let dist = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
    (0.5 - dist).clamp(0.0, 1.0)
}

/// Coverage of a rounded rectangle's outline of width `w`.
fn outline(x: f32, y: f32, r: (f32, f32, f32, f32), rad: f32, w: f32) -> f32 {
    let inner = (r.0 + w, r.1 + w, r.2 - w, r.3 - w);
    rrect(x, y, r, rad).min(1.0 - rrect(x, y, inner, (rad - w).max(0.0)))
}

/// Coverage of the upper half of a ring (the bridge) centred on (cx, cy).
fn arc(x: f32, y: f32, (cx, cy): (f32, f32), radius: f32, w: f32) -> f32 {
    let ring = (0.5 - ((x - cx).hypot(y - cy) - radius).abs() + w / 2.0).clamp(0.0, 1.0);
    ring * (cy - y + 0.5).clamp(0.0, 1.0)
}

/// Source-over of a colour with coverage `a` onto a straight-alpha pixel.
fn over(dst: &mut [f32; 4], c: [f32; 3], a: f32) {
    let out = a + dst[3] * (1.0 - a);
    if out > 0.0 {
        for i in 0..3 {
            dst[i] = (c[i] * a + dst[i] * dst[3] * (1.0 - a)) / out;
        }
    }
    dst[3] = out;
}

/// Coverage of the two screens' outlines, the bridge, and the second screen's body.
fn shapes(s: f32, x: f32, y: f32) -> [f32; 4] {
    let (w, top, bottom) = (0.09 * s, 0.38 * s, 0.80 * s);
    let left = (0.07 * s, top, 0.43 * s, bottom);
    let right = (0.57 * s, top, 0.93 * s, bottom);
    let rad = 0.09 * s;
    let bridge = arc(x, y, (0.5 * s, top + w / 2.0), 0.25 * s, w);
    [outline(x, y, left, rad, w), outline(x, y, right, rad, w), bridge, rrect(x, y, right, rad)]
}

/// `size` x `size` pixels, RGBA, not premultiplied.
pub fn rgba(size: u32, state: IconState) -> Vec<u8> {
    let s = size as f32;
    let screens = if state == IconState::Off { 0.45 } else { 1.0 };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let [l, r, bridge, body] = shapes(s, px as f32 + 0.5, py as f32 + 0.5);
            let mut p = [0.0; 4];
            over(&mut p, GREY, l * screens);
            over(&mut p, GREY, r * screens);
            match state {
                IconState::Off => {}
                IconState::Idle => over(&mut p, GREY, bridge * 0.35),
                _ => over(&mut p, AMBER, bridge),
            }
            if state == IconState::Controlling {
                over(&mut p, AMBER, body);
            }
            out.extend(p[..3].iter().map(|c| c.round() as u8));
            out.push((p[3] * 255.0).round() as u8);
        }
    }
    out
}

/// A macOS template image: black shapes, only the alpha matters.
#[cfg(any(target_os = "macos", test))]
pub fn template_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let [l, r, bridge, _] = shapes(s, px as f32 + 0.5, py as f32 + 0.5);
            out.extend([0, 0, 0, (l.max(r).max(bridge) * 255.0).round() as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use IconState::*;

    const ALL: [IconState; 4] = [Off, Idle, Connected, Controlling];

    fn has_amber(a: &[u8]) -> bool {
        a.as_chunks::<4>().0.iter().any(|p| p[3] > 200 && p[0] > 240 && (0xa0..0xc0).contains(&p[1]) && p[2] < 0x60)
    }

    #[test]
    fn icons_have_the_right_size_and_transparent_corners() {
        for st in ALL {
            let a = rgba(32, st);
            assert_eq!(a.len(), 32 * 32 * 4);
            assert_eq!(a[3], 0, "corner is transparent");
            assert!(a.as_chunks::<4>().0.iter().any(|p| p[3] > 100), "{st:?} has solid pixels");
        }
        assert_eq!(template_rgba(22).len(), 22 * 22 * 4);
    }

    #[test]
    fn states_differ() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(rgba(32, *a), rgba(32, *b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn amber_only_when_connected() {
        for st in ALL {
            assert_eq!(has_amber(&rgba(64, st)), matches!(st, Connected | Controlling), "{st:?}");
        }
    }

    #[test]
    fn template_is_black_with_alpha_only() {
        let t = template_rgba(32);
        assert!(t.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]));
        assert!(t.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
        assert_eq!(t[3], 0);
    }
}
