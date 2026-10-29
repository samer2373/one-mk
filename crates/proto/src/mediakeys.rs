//! Multimedia keys: volume, transport, brightness and launch keys.
//!
//! On the wire they are ordinary `Control::Key` codes in the canonical
//! space (Linux evdev `KEY_*`), so a build without this table still passes
//! them on and a peer that doesn't know one drops it, never getting it
//! stuck: a release of a key that was never pressed is not forwarded, and
//! `HeldKeys` releases every held key when control moves.
//!
//! The OSes don't agree on how these keys are *delivered*, which is what
//! this table is for:
//!
//! | | capture | injection |
//! |---|---|---|
//! | Linux | libei key events (evdev codes) | libei key events |
//! | Windows | `WH_KEYBOARD_LL` virtual keys `VK_MEDIA_*`, `VK_VOLUME_*`, `VK_BROWSER_*`, `VK_LAUNCH_*` | `SendInput` by virtual key |
//! | macOS | `NX_SYSDEFINED` events (subtype 8; key in `data1`), not key events | the same, posted with `CGEventPost` |
//!
//! Windows has no virtual keys for brightness, fast-forward or eject, and
//! macOS has none for launch or browser keys: those only travel between
//! the OSes that have them. A key the receiver cannot make is dropped.

use crate::os;

/// One media key and what each OS calls it.
#[derive(Clone, Copy, Debug)]
pub struct MediaKey {
    pub name: &'static str,
    /// Canonical wire code: Linux evdev `KEY_*`.
    pub evdev: u32,
    /// Windows virtual-key code (`VK_*`), if Windows has one.
    pub win_vk: Option<u16>,
    /// macOS `NX_KEYTYPE_*` (`IOKit/hidsystem/ev_keymap.h`), if macOS has one.
    pub mac_nx: Option<u8>,
    /// Whether holding the key repeats it. Volume and brightness do; a
    /// repeated play/pause or mute would toggle back and forth.
    pub repeats: bool,
}

const fn key(name: &'static str, evdev: u32, win_vk: Option<u16>, mac_nx: Option<u8>, repeats: bool) -> MediaKey {
    MediaKey { name, evdev, win_vk, mac_nx, repeats }
}

/// The canonical mapping. One row per key; `evdev`, `win_vk` and `mac_nx`
/// are each unique within the table (a test checks).
#[rustfmt::skip]
pub const MEDIA_KEYS: &[MediaKey] = &[
    //  name                evdev  Windows VK    macOS NX_KEYTYPE
    key("mute",              113, Some(0xAD), Some(7),  false), // KEY_MUTE, VK_VOLUME_MUTE, NX_KEYTYPE_MUTE
    key("volume down",       114, Some(0xAE), Some(1),  true),  // KEY_VOLUMEDOWN, VK_VOLUME_DOWN, NX_KEYTYPE_SOUND_DOWN
    key("volume up",         115, Some(0xAF), Some(0),  true),  // KEY_VOLUMEUP, VK_VOLUME_UP, NX_KEYTYPE_SOUND_UP
    key("play/pause",        164, Some(0xB3), Some(16), false), // KEY_PLAYPAUSE, VK_MEDIA_PLAY_PAUSE, NX_KEYTYPE_PLAY
    key("stop",              166, Some(0xB2), None,     false), // KEY_STOPCD, VK_MEDIA_STOP
    key("next track",        163, Some(0xB0), Some(17), false), // KEY_NEXTSONG, VK_MEDIA_NEXT_TRACK, NX_KEYTYPE_NEXT
    key("previous track",    165, Some(0xB1), Some(18), false), // KEY_PREVIOUSSONG, VK_MEDIA_PREV_TRACK, NX_KEYTYPE_PREVIOUS
    key("fast forward",      208, None,       Some(19), false), // KEY_FASTFORWARD, NX_KEYTYPE_FAST
    key("rewind",            168, None,       Some(20), false), // KEY_REWIND, NX_KEYTYPE_REWIND
    key("eject",             161, None,       Some(14), false), // KEY_EJECTCD, NX_KEYTYPE_EJECT
    key("microphone mute",   248, None,       None,     false), // KEY_MICMUTE
    key("brightness down",   224, None,       Some(3),  true),  // KEY_BRIGHTNESSDOWN, NX_KEYTYPE_BRIGHTNESS_DOWN
    key("brightness up",     225, None,       Some(2),  true),  // KEY_BRIGHTNESSUP, NX_KEYTYPE_BRIGHTNESS_UP
    key("keyboard light down",   229, None,   Some(22), true),  // KEY_KBDILLUMDOWN, NX_KEYTYPE_ILLUMINATION_DOWN
    key("keyboard light up",     230, None,   Some(21), true),  // KEY_KBDILLUMUP, NX_KEYTYPE_ILLUMINATION_UP
    key("keyboard light toggle", 228, None,   Some(23), false), // KEY_KBDILLUMTOGGLE, NX_KEYTYPE_ILLUMINATION_TOGGLE
    key("launch mail",       155, Some(0xB4), None,     false), // KEY_MAIL, VK_LAUNCH_MAIL
    key("launch media",      226, Some(0xB5), None,     false), // KEY_MEDIA, VK_LAUNCH_MEDIA_SELECT
    key("launch computer",   157, Some(0xB6), None,     false), // KEY_COMPUTER, VK_LAUNCH_APP1
    key("launch calculator", 140, Some(0xB7), None,     false), // KEY_CALC, VK_LAUNCH_APP2
    key("browser back",      158, Some(0xA6), None,     false), // KEY_BACK, VK_BROWSER_BACK
    key("browser forward",   159, Some(0xA7), None,     false), // KEY_FORWARD, VK_BROWSER_FORWARD
    key("browser refresh",   173, Some(0xA8), None,     false), // KEY_REFRESH, VK_BROWSER_REFRESH
    key("browser stop",      128, Some(0xA9), None,     false), // KEY_STOP, VK_BROWSER_STOP
    key("browser search",    217, Some(0xAA), None,     false), // KEY_SEARCH, VK_BROWSER_SEARCH
    key("browser favorites", 156, Some(0xAB), None,     false), // KEY_BOOKMARKS, VK_BROWSER_FAVORITES
    key("browser home",      172, Some(0xAC), None,     false), // KEY_HOMEPAGE, VK_BROWSER_HOME
];

/// Keys a receiver without them makes with a near relative, so a Mac's
/// F7/F9 (fast-forward, rewind) still skip tracks on Windows, and Linux
/// keyboards with separate play and pause keys toggle playback. Only used
/// when the receiver has nothing for the key itself; `(code, stands in for)`.
pub const FALLBACKS: &[(u32, u32)] = &[
    (208, 163), // KEY_FASTFORWARD -> next track
    (168, 165), // KEY_REWIND -> previous track
    (207, 164), // KEY_PLAY -> play/pause
    (200, 164), // KEY_PLAYCD -> play/pause
    (201, 164), // KEY_PAUSECD -> play/pause
];

/// The media key with canonical code `code`.
pub fn media_key(code: u32) -> Option<&'static MediaKey> {
    MEDIA_KEYS.iter().find(|k| k.evdev == code)
}

/// Whether `code` is a key this table covers (or a fallback for one).
pub fn is_media(code: u32) -> bool {
    media_key(code).is_some() || FALLBACKS.iter().any(|&(c, _)| c == code)
}

/// Whether holding `code` makes it repeat. True for every key that is not
/// a media key that toggles or steps once per press.
pub fn repeats(code: u32) -> bool {
    media_key(code).is_none_or(|k| k.repeats)
        && !FALLBACKS.iter().any(|&(c, _)| c == code)
}

/// The key a receiver should use for `code` when `has` says whether it has
/// something for a given key: the key itself, else its fallback.
fn resolve(code: u32, has: impl Fn(&MediaKey) -> bool) -> Option<&'static MediaKey> {
    media_key(code).filter(|k| has(k)).or_else(|| {
        let &(_, to) = FALLBACKS.iter().find(|&&(c, _)| c == code)?;
        media_key(to).filter(|k| has(k))
    })
}

/// The Windows virtual key to inject for canonical `code`.
pub fn to_win_vk(code: u32) -> Option<u16> {
    resolve(code, |k| k.win_vk.is_some()).and_then(|k| k.win_vk)
}

/// The macOS `NX_KEYTYPE_*` to inject for canonical `code`.
pub fn to_mac_nx(code: u32) -> Option<u8> {
    resolve(code, |k| k.mac_nx.is_some()).and_then(|k| k.mac_nx)
}

/// The canonical code for a Windows virtual key.
pub fn from_win_vk(vk: u16) -> Option<u32> {
    MEDIA_KEYS.iter().find(|k| k.win_vk == Some(vk)).map(|k| k.evdev)
}

/// The canonical code for a macOS `NX_KEYTYPE_*`.
pub fn from_mac_nx(nx: u8) -> Option<u32> {
    MEDIA_KEYS.iter().find(|k| k.mac_nx == Some(nx)).map(|k| k.evdev)
}

/// How an OS names a media key: what its capture reports and its injection
/// takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Native {
    /// Linux evdev code.
    Evdev(u32),
    WinVk(u16),
    MacNx(u8),
}

/// The canonical code for `native` as captured on OS `os` (see [`os`]).
pub fn from_native(os: u8, native: Native) -> Option<u32> {
    match (os, native) {
        (os::LINUX, Native::Evdev(c)) => Some(c),
        (os::WINDOWS, Native::WinVk(vk)) => from_win_vk(vk),
        (os::MACOS, Native::MacNx(nx)) => from_mac_nx(nx),
        _ => None,
    }
}

/// What OS `os` must inject for canonical `code`. Linux passes every code
/// on and lets the compositor decide.
pub fn to_native(os: u8, code: u32) -> Option<Native> {
    match os {
        os::LINUX => Some(Native::Evdev(code)),
        os::WINDOWS => to_win_vk(code).map(Native::WinVk),
        os::MACOS => to_mac_nx(code).map(Native::MacNx),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const OSES: [u8; 3] = [os::LINUX, os::WINDOWS, os::MACOS];

    /// What OS `os` reports for a key, as the table says it has one.
    fn native_of(os: u8, k: &MediaKey) -> Option<Native> {
        match os {
            os::LINUX => Some(Native::Evdev(k.evdev)),
            os::WINDOWS => k.win_vk.map(Native::WinVk),
            _ => k.mac_nx.map(Native::MacNx),
        }
    }

    #[test]
    fn columns_are_unique_and_in_range() {
        let evdev: HashSet<_> = MEDIA_KEYS.iter().map(|k| k.evdev).collect();
        let vks: Vec<_> = MEDIA_KEYS.iter().filter_map(|k| k.win_vk).collect();
        let nxs: Vec<_> = MEDIA_KEYS.iter().filter_map(|k| k.mac_nx).collect();
        assert_eq!(evdev.len(), MEDIA_KEYS.len());
        assert_eq!(vks.iter().collect::<HashSet<_>>().len(), vks.len());
        assert_eq!(nxs.iter().collect::<HashSet<_>>().len(), nxs.len());
        // HeldKeys covers 0..0x300, and buttons live at 0x110..0x120.
        assert!(evdev.iter().all(|&c| c < 0x300 && !(0x110..0x120).contains(&c)));
        // Windows media keys are VK 0xA6..=0xB7.
        assert!(vks.iter().all(|vk| (0xA6..=0xB7).contains(vk)));
        for &(from, to) in FALLBACKS {
            assert!(media_key(to).is_some(), "fallback target {to}");
            assert!(!evdev.contains(&from) || media_key(from).is_some_and(|k| k.win_vk.is_none() || k.mac_nx.is_none()));
        }
    }

    /// Every key, captured on any OS that has it and injected on any OS:
    /// it arrives as the same key, or (if the receiver has none) as the
    /// fallback, or not at all; never as a different key.
    #[test]
    fn every_key_round_trips_between_every_pair_of_oses() {
        for k in MEDIA_KEYS {
            for src in OSES {
                let Some(native) = native_of(src, k) else { continue };
                let wire = from_native(src, native).unwrap_or_else(|| panic!("{} not captured on {src}", k.name));
                assert_eq!(wire, k.evdev, "{} from {src}", k.name);
                for dst in OSES {
                    let sent = to_native(dst, wire);
                    match native_of(dst, k) {
                        Some(n) => assert_eq!(sent, Some(n), "{} {src}->{dst}", k.name),
                        None => {
                            // Not on the receiver: a stand-in or nothing.
                            if let Some(Native::WinVk(_) | Native::MacNx(_)) = sent {
                                let &(_, to) = FALLBACKS
                                    .iter()
                                    .find(|&&(c, _)| c == wire)
                                    .unwrap_or_else(|| panic!("{} {src}->{dst} became {sent:?}", k.name));
                                assert_eq!(sent, to_native(dst, to), "{} {src}->{dst}", k.name);
                            }
                            // And back again: whatever it became still maps to a key.
                            if let Some(n) = sent {
                                assert!(from_native(dst, n).is_some());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_but_the_table_is_captured() {
        for vk in 0..=0xFFu16 {
            if let Some(code) = from_win_vk(vk) {
                assert_eq!(media_key(code).and_then(|k| k.win_vk), Some(vk));
            }
        }
        for nx in 0..=0xFFu8 {
            if let Some(code) = from_mac_nx(nx) {
                assert_eq!(media_key(code).and_then(|k| k.mac_nx), Some(nx));
            }
        }
        assert_eq!(from_mac_nx(4), None); // NX_KEYTYPE_CAPS_LOCK is not a media key
        assert_eq!(from_mac_nx(6), None); // NX_KEYTYPE_POWER is never forwarded
    }

    #[test]
    fn fallbacks_stand_in_only_when_needed() {
        // Windows has no fast-forward: next track. A Mac has one: itself.
        assert_eq!(to_win_vk(208), Some(0xB0));
        assert_eq!(to_mac_nx(208), Some(19));
        assert_eq!(to_win_vk(168), Some(0xB1));
        // Linux keyboards with separate play and pause.
        for code in [200, 201, 207] {
            assert_eq!(to_win_vk(code), Some(0xB3));
            assert_eq!(to_mac_nx(code), Some(16));
        }
        // No Windows or macOS key for brightness on Windows, none for launch keys on a Mac.
        assert_eq!(to_win_vk(225), None);
        assert_eq!(to_mac_nx(155), None);
        // Not a media key at all.
        assert_eq!(to_win_vk(30), None);
        assert_eq!(to_native(os::LINUX, 30), Some(Native::Evdev(30)));
    }

    #[test]
    fn only_stepping_keys_repeat() {
        for code in [114, 115, 224, 225] {
            assert!(repeats(code), "{code}");
        }
        for code in [113, 163, 164, 165, 166, 200, 208, 248, 155] {
            assert!(!repeats(code), "{code}");
        }
        assert!(repeats(30)); // an ordinary key
        assert!(is_media(201) && is_media(164) && !is_media(30));
    }
}
