// SPDX-License-Identifier: MIT OR Apache-2.0

pub const NAME: &str = "org.onemk.WallpaperBridge1";
pub const PATH: &str = "/org/onemk/WallpaperBridge1";
pub const VERSION: u32 = 1;
pub const MAX_PAYLOAD: usize = 4096;
pub const XML: &str = include_str!("../../../extensions/gnome/one-mk-wallpaper@one-mk.org/bridge.xml");
pub type Monitor = (i32, i32, i32, i32, f64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum State {
    Starting, Playing, Paused, Failed,
    /// Playing a still or a long frame: no damage is expected until the
    /// renderer reports `Playing` again, so the Shell runs no damage watchdog.
    Holding,
}

impl TryFrom<u32> for State {
    type Error = &'static str;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Starting), 1 => Ok(Self::Playing),
            2 => Ok(Self::Paused), 3 => Ok(Self::Failed), 4 => Ok(Self::Holding),
            _ => Err("invalid renderer state"),
        }
    }
}

pub fn validate_reasons(reasons: u32) -> Result<(), &'static str> {
    if reasons & !15 == 0 { Ok(()) } else { Err("unknown pause reason bits") }
}

pub fn validate_layout(serial: u32, monitors: &[Monitor]) -> Result<(), &'static str> {
    if serial == 0 || monitors.len() > 16 { return Err("invalid layout serial/count"); }
    if monitors.is_empty() { return Ok(()); }
    let mut left = i64::MAX;
    let mut top = i64::MAX;
    let mut right = i64::MIN;
    let mut bottom = i64::MIN;
    let mut scale = 0.5_f64;
    for &(x, y, width, height, factor) in monitors {
        if width <= 0 || height <= 0 || !factor.is_finite() || !(0.5..=4.0).contains(&factor) {
            return Err("invalid monitor geometry/scale");
        }
        let end_x = x.checked_add(width).ok_or("monitor edge overflow")?;
        let end_y = y.checked_add(height).ok_or("monitor edge overflow")?;
        left = left.min(i64::from(x)); top = top.min(i64::from(y));
        right = right.max(i64::from(end_x)); bottom = bottom.max(i64::from(end_y));
        scale = scale.max(factor);
    }
    let width = ((right - left) as f64 * scale).ceil();
    let height = ((bottom - top) as f64 * scale).ceil();
    if width > 16384.0 || height > 16384.0 || width * height > 64_000_000.0 {
        return Err("physical canvas exceeds bridge budget");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_protocol_and_bounds() {
        assert!(XML.contains("a(iiiid)"));
        assert!(!XML.contains("property"));
        assert!(validate_layout(1, &[]).is_ok());
        assert!(validate_layout(0, &[]).is_err());
        assert!(validate_layout(1, &[(0, 0, 1920, 1080, 1.25)]).is_ok());
        for monitor in [(0, 0, 0, 1, 1.0), (i32::MAX, 0, 1, 1, 1.0),
            (0, 0, 1, 1, f64::NAN), (0, 0, 1, 1, 4.1),
            (0, 0, 16385, 1, 1.0), (0, 0, 8001, 8000, 1.0)] {
            assert!(validate_layout(1, &[monitor]).is_err());
        }
        assert!(validate_layout(1, &[(0, 0, 1, 1, 1.0); 17]).is_err());
        assert!(validate_reasons(15).is_ok());
        assert!(validate_reasons(16).is_err());
        assert_eq!(State::try_from(4), Ok(State::Holding));
        assert!(State::try_from(5).is_err());
    }
}
