//! The tray icons, drawn in code (no image files, no image decoder): a
//! rounded square in the app's pink with two screens side by side. When the
//! daemon is not running the square is grey and the second screen faded.

/// Coverage (0..=1) of a rounded rectangle at pixel centre (x, y), anti-aliased over one pixel.
fn rrect(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    let (dx, dy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    let dist = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
    (0.5 - dist).clamp(0.0, 1.0)
}

/// `size` x `size` pixels, RGBA, not premultiplied.
pub fn rgba(size: u32, connected: bool) -> Vec<u8> {
    let s = size as f32;
    let bg = if connected { [0xdb, 0x27, 0x77] } else { [0x6b, 0x72, 0x80] };
    let (sw, sh) = (0.30 * s, 0.34 * s);
    let top = 0.5 * s - sh / 2.0;
    let left = (0.15 * s, top, 0.15 * s + sw, top + sh);
    let right = (0.85 * s - sw, top, 0.85 * s, top + sh);
    let second = if connected { 1.0 } else { 0.4 };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let square = rrect(x, y, (0.0, 0.0, s, s), 0.22 * s);
            let white = rrect(x, y, left, 0.06 * s).max(rrect(x, y, right, 0.06 * s) * second);
            for c in bg {
                out.push((c as f32 * (1.0 - white) + 255.0 * white).round() as u8);
            }
            out.push((square * 255.0).round() as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_have_the_right_size_and_rounded_corners() {
        let a = rgba(32, true);
        assert_eq!(a.len(), 32 * 32 * 4);
        assert_eq!(a[3], 0, "corner is transparent");
        let mid = (16 * 32 + 16) * 4;
        assert_eq!(a[mid + 3], 255, "centre is opaque");
        assert_ne!(rgba(32, true), rgba(32, false));
    }
}
