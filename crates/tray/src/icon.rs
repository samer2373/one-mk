//! The tray icons: the brand's flat keycap with the pointer on its top,
//! pre-drawn per state, bar (white ink for a dark bar, black ink for a light
//! one) and size in docs/brand/tray and embedded as raw RGBA by
//! scripts/make-icons.py, so there is no image decoder at run time.
//! Cobalt is the one signal colour: the pointer is lit only while
//! computers are connected.

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

/// Which bar the icon sits on: it is drawn in white ink for a dark bar and in black ink for a
/// light one. Linux panels give no way to ask, so there it is always `Dark` (GNOME's top bar,
/// Plasma's default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub enum Bar {
    Dark,
    Light,
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
    ($name:expr) => {
        Raw(include_bytes!(concat!("../assets/", $name, ".rgba")))
    };
}

/// The six drawings of one state on one bar: 16, 22 and 24 px, each also at 2x.
macro_rules! set {
    ($bar:literal, $state:literal) => {
        [
            raw!(concat!($bar, "-", $state, "-16")),
            raw!(concat!($bar, "-", $state, "-22")),
            raw!(concat!($bar, "-", $state, "-24")),
            raw!(concat!($bar, "-", $state, "-32")),
            raw!(concat!($bar, "-", $state, "-44")),
            raw!(concat!($bar, "-", $state, "-48")),
        ]
    };
}

fn set(state: IconState, bar: Bar) -> [Raw; 6] {
    match (bar, state) {
        (Bar::Dark, IconState::Off) => set!("dark", "off"),
        (Bar::Dark, IconState::Idle) => set!("dark", "idle"),
        (Bar::Dark, IconState::Connected) => set!("dark", "connected"),
        (Bar::Dark, IconState::Controlling) => set!("dark", "controlling"),
        (Bar::Light, IconState::Off) => set!("light", "off"),
        (Bar::Light, IconState::Idle) => set!("light", "idle"),
        (Bar::Light, IconState::Connected) => set!("light", "connected"),
        (Bar::Light, IconState::Controlling) => set!("light", "controlling"),
    }
}

/// The sizes there are drawings for (Linux offers them all to the panel).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SIZES: [u32; 6] = [16, 22, 24, 32, 44, 48];

/// `size` x `size` pixels, RGBA, not premultiplied. `size` must be one of [`SIZES`].
pub fn rgba(size: u32, state: IconState, bar: Bar) -> Vec<u8> {
    let raw = set(state, bar).into_iter().find(|r| r.size() == size).expect("a size from SIZES");
    raw.pixels().to_vec()
}

/// The bar the icon will sit on. macOS and Windows say whether the system is in dark or light
/// mode; this is read when the icon is set, so a change shows from the next status change.
/// Linux panels do not say, so it stays `Dark` there.
#[cfg(target_os = "macos")]
pub fn bar() -> Bar {
    // `defaults` prints "Dark" in dark mode and fails in light mode.
    let dark = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"Dark"));
    if dark { Bar::Dark } else { Bar::Light }
}

/// See the macOS version. SystemUsesLightTheme is the taskbar's theme: 0x1 light, 0x0 dark.
#[cfg(windows)]
pub fn bar() -> Bar {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let light = std::process::Command::new("reg")
        .args(["query", key, "/v", "SystemUsesLightTheme"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("0x1"));
    if light { Bar::Light } else { Bar::Dark }
}

/// 32 x 32 black-and-alpha template for the macOS menu bar, which recolours
/// it to match light or dark mode. Only the states without the lit cobalt pointer have one.
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
    const BARS: [Bar; 2] = [Bar::Dark, Bar::Light];

    fn pixels(a: &[u8]) -> &[[u8; 4]] {
        a.as_chunks::<4>().0
    }

    /// The lit pointer, in the brand's cobalt (a blue: far more blue than red or green).
    fn has_cobalt(a: &[u8]) -> bool {
        pixels(a).iter().any(|p| p[3] > 200 && p[2] > 200 && p[0] < 110 && p[1] < 130)
    }

    #[test]
    fn every_drawing_is_complete_and_transparent_in_the_corner() {
        for bar in BARS {
            for state in ALL {
                for size in SIZES {
                    let a = rgba(size, state, bar);
                    assert_eq!(a.len(), (size * size * 4) as usize, "{bar:?} {state:?} {size}");
                    assert_eq!(a[3], 0, "{bar:?} {state:?} {size}: corner is transparent");
                    assert!(pixels(&a).iter().any(|p| p[3] > 100), "{bar:?} {state:?} {size}: something is drawn");
                }
            }
        }
    }

    #[test]
    fn states_differ_and_a_lit_pointer_means_computers_are_connected() {
        for bar in BARS {
            for (i, a) in ALL.iter().enumerate() {
                for b in &ALL[i + 1..] {
                    assert_ne!(rgba(32, *a, bar), rgba(32, *b, bar), "{bar:?} {a:?} vs {b:?}");
                }
            }
            for state in ALL {
                let lit = matches!(state, IconState::Connected | IconState::Controlling);
                assert_eq!(has_cobalt(&rgba(48, state, bar)), lit, "{bar:?} {state:?}");
            }
        }
    }

    #[test]
    fn ink_is_white_on_a_dark_bar_and_black_on_a_light_one() {
        for state in ALL {
            for (bar, white) in [(Bar::Dark, true), (Bar::Light, false)] {
                // The ink pixels are the visible ones that are not the blue pointer.
                let a = rgba(48, state, bar);
                let ink: Vec<_> = pixels(&a).iter().filter(|p| p[3] > 100 && !(p[2] > 200 && p[0] < 110)).collect();
                assert!(!ink.is_empty(), "{bar:?} {state:?}");
                assert!(ink.iter().all(|p| (p[0] > 200) == white && (p[0] > 200 || p[0] < 55)), "{bar:?} {state:?}");
            }
        }
    }

    #[test]
    fn off_is_dimmer_than_idle() {
        let weight = |s: IconState| pixels(&rgba(48, s, Bar::Dark)).iter().map(|p| u32::from(p[3])).sum::<u32>();
        assert!(weight(IconState::Off) * 2 < weight(IconState::Idle));
    }

    #[test]
    fn templates_are_black_with_alpha_only() {
        for state in [IconState::Off, IconState::Idle] {
            let t = template_rgba(state).unwrap();
            assert_eq!(t.len(), 32 * 32 * 4);
            assert!(pixels(&t).iter().all(|p| p[..3] == [0, 0, 0]));
            assert!(pixels(&t).iter().any(|p| p[3] > 100));
        }
        assert!(template_rgba(IconState::Connected).is_none());
    }
}
