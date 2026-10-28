//! The tray icons: the brand's Crossing mark (a laptop and a monitor with
//! the pointer on the seam between them), pre-drawn per state and size in
//! docs/brand/tray and embedded as raw RGBA by scripts/brand-assets.py, so
//! there is no image decoder at run time. Amber is the one signal colour:
//! it appears only when computers are connected.

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

/// One embedded image: width and height (u32 LE), then straight RGBA.
struct Raw(&'static [u8]);

impl Raw {
    fn size(&self) -> u32 {
        u32::from_le_bytes(self.0[..4].try_into().expect("header"))
    }
    fn pixels(&self) -> &'static [u8] {
        &self.0[8..]
    }
}

macro_rules! raw {
    ($name:literal) => {
        Raw(include_bytes!(concat!("../assets/", $name, ".rgba")))
    };
}

/// The drawings for one state, by pixel size: hand-tuned 22 px for the
/// GNOME/KDE panel, and the 16, 22 and 32 px drawings at 2x.
fn set(state: IconState) -> [Raw; 4] {
    match state {
        IconState::Off => [raw!("off-22"), raw!("off-16x2"), raw!("off-22x2"), raw!("off-32x2")],
        IconState::Idle => [raw!("idle-22"), raw!("idle-16x2"), raw!("idle-22x2"), raw!("idle-32x2")],
        IconState::Connected => {
            [raw!("connected-22"), raw!("connected-16x2"), raw!("connected-22x2"), raw!("connected-32x2")]
        }
        IconState::Controlling => {
            [raw!("controlling-22"), raw!("controlling-16x2"), raw!("controlling-22x2"), raw!("controlling-32x2")]
        }
    }
}

/// The sizes there are drawings for (Linux offers them all to the panel).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SIZES: [u32; 4] = [22, 32, 44, 64];

/// `size` x `size` pixels, RGBA, not premultiplied. `size` must be one of [`SIZES`].
pub fn rgba(size: u32, state: IconState) -> Vec<u8> {
    let raw = set(state).into_iter().find(|r| r.size() == size).expect("a size from SIZES");
    raw.pixels().to_vec()
}

/// 32 x 32 black-and-alpha template for the macOS menu bar, which recolours
/// it to match light or dark mode. Only the states without amber have one.
#[cfg(any(target_os = "macos", test))]
pub fn template_rgba(state: IconState) -> Option<Vec<u8>> {
    let raw = match state {
        IconState::Off => raw!("off-template-32"),
        IconState::Idle => raw!("idle-template-32"),
        _ => return None,
    };
    Some(raw.pixels().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [IconState; 4] = [IconState::Off, IconState::Idle, IconState::Connected, IconState::Controlling];

    fn has_amber(a: &[u8]) -> bool {
        a.as_chunks::<4>().0.iter().any(|p| p[3] > 128 && p[0] > 200 && p[1] > 140 && p[1] < 210 && p[2] < 120)
    }

    #[test]
    fn every_drawing_is_complete_and_transparent_in_the_corner() {
        for state in ALL {
            for size in SIZES {
                let a = rgba(size, state);
                assert_eq!(a.len(), (size * size * 4) as usize, "{state:?} {size}");
                assert_eq!(a[3], 0, "{state:?} {size}: corner is transparent");
                assert!(a.as_chunks::<4>().0.iter().any(|p| p[3] > 100), "{state:?} {size}: something is drawn");
            }
        }
    }

    #[test]
    fn states_differ_and_amber_means_connected() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(rgba(32, *a), rgba(32, *b), "{a:?} vs {b:?}");
            }
        }
        for state in ALL {
            let amber = matches!(state, IconState::Connected | IconState::Controlling);
            assert_eq!(has_amber(&rgba(64, state)), amber, "{state:?}");
        }
    }

    #[test]
    fn templates_are_black_with_alpha_only() {
        for state in [IconState::Off, IconState::Idle] {
            let t = template_rgba(state).unwrap();
            assert_eq!(t.len(), 32 * 32 * 4);
            assert!(t.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]));
        }
        assert!(template_rgba(IconState::Connected).is_none());
    }
}
