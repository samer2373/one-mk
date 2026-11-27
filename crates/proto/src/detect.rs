//! Finding out which key or mouse button someone means by pressing it
//! (`one-mk input-devices detect`, the window's "Detect key"): the presses
//! the input backend reports go in, the first one that makes a trigger
//! comes out, as the key rules see it (a code, the modifiers held with it,
//! the device). Pure: the daemon feeds it and keeps the time (milliseconds).
//!
//! What counts, as in the window's "Press new key": a key or button going
//! down, with the modifiers held then (a mouse button that sends Ctrl+Alt+Tab
//! comes out as `ctrl+alt+tab`); a modifier pressed and let go alone is that
//! modifier. A key already down when detecting began (the Enter that
//! started the command) and auto-repeat are ignored. Esc alone cancels.

use crate::device::DeviceRef;
use crate::keynames::{self, Trigger};
use std::collections::VecDeque;

/// How long a press waits for its device (`PressDevice`, from Raw Input or
/// IOHIDManager), and how old a device report may be to count.
pub const DEVICE_WAIT_MS: u64 = 60;
/// Device reports kept, at most.
const HINTS: usize = 32;

/// The first code past the mouse buttons the rules know: codes from here to
/// [`BEYOND_BUTTONS_END`] stand for a mouse button past the eighth, which a
/// backend reports so it can be named, though no rule can use it.
pub const BEYOND_BUTTONS: u32 = keynames::BTN_LEFT + keynames::BUTTONS;
pub const BEYOND_BUTTONS_END: u32 = keynames::BTN_LEFT + 16;

/// Whether a backend reports this computer's own presses while the pointer
/// is here: `Ok` where it does (with what isn't tested), `Err(why)` where it
/// sees input only while controlling another computer, or never.
pub fn detect_support(backend: &str) -> Result<Option<&'static str>, &'static str> {
    match backend {
        "x11" => Ok(None),
        "portal" | "wlroots" => Err(LOCAL_UNSEEN),
        "null" => Err("the null input backend sees no keyboard or mouse"),
        "native" if cfg!(windows) => Ok(Some("untested on real Windows hardware (compile-checked only)")),
        "native" if cfg!(target_os = "macos") => Ok(Some("untested on a real Mac (compile-checked only)")),
        _ => Err("this input backend doesn't report this computer's own keys"),
    }
}

/// Why a Wayland desktop's keys can't be detected while the pointer is here.
pub const LOCAL_UNSEEN: &str = "on this desktop oneMK sees keys only while it controls another computer";

/// A press, found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    /// The key or button's evdev code, as the mapper sees it.
    pub code: u32,
    /// The modifiers held with it (sided bits, `keynames::LCTRL` ...).
    pub mods: u8,
    pub device: Option<DeviceRef>,
    /// It was pressed while this computer controlled another one.
    pub away: bool,
}

/// A mouse button (one the rules know, or one past the eighth).
fn is_button(code: u32) -> bool {
    keynames::is_button(code) || (BEYOND_BUTTONS..BEYOND_BUTTONS_END).contains(&code)
}

impl Found {
    /// A mouse button (one the rules know, or one past the eighth).
    pub fn is_button(&self) -> bool {
        is_button(self.code)
    }

    /// The rule trigger it makes: either side of each modifier held, as
    /// `ctrl+alt+tab` means (a mouse whose button sends a shortcut may use
    /// either Ctrl).
    pub fn trigger(&self) -> Trigger {
        Trigger { groups: (self.mods | (self.mods >> 4)) & 0xf, sided: 0, key: self.code as u16 }
    }

    /// Why no rule can use it, if none can.
    pub fn unusable(&self) -> Option<&'static str> {
        if (BEYOND_BUTTONS..BEYOND_BUTTONS_END).contains(&self.code) {
            Some("oneMK can change mouse buttons 1 to 8, and this one is further on")
        } else if self.code == 0 || self.code >= keynames::CODE_LIMIT {
            Some("this key has no code oneMK can use in a rule")
        } else {
            None
        }
    }
}

/// What a press or a tick came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing yet.
    Wait,
    Found(Found),
    /// Esc, pressed alone.
    Cancel,
}

#[derive(Clone, Copy, Debug)]
struct Hint {
    code: u32,
    device: DeviceRef,
    at: u64,
}

/// One detection: what is held, and what waits for its device.
#[derive(Debug, Default)]
pub struct Detector {
    /// Device reports come separately (`PressDevice`): a press waits a
    /// little for its own.
    hints_expected: bool,
    /// Mouse buttons don't count.
    keys_only: bool,
    /// Keys and buttons seen going down since detecting began, and when.
    down: Vec<(u32, u64)>,
    held: u8,
    /// A modifier pressed with nothing else since: let go, it is the press.
    lone: Option<u32>,
    pending: Option<(Found, u64, u64)>,
    hints: VecDeque<Hint>,
}

impl Detector {
    /// `hints_expected`: the backend reports devices apart from presses;
    /// `keys_only`: mouse buttons don't count.
    pub fn new(hints_expected: bool, keys_only: bool) -> Self {
        Detector { hints_expected, keys_only, ..Default::default() }
    }

    /// A key or button went down or up (`device` if the event said).
    pub fn press(&mut self, code: u32, down: bool, device: Option<DeviceRef>, away: bool, now: u64) -> Step {
        if self.pending.is_some() || (self.keys_only && is_button(code)) {
            return Step::Wait;
        }
        let bit = keynames::mod_bit(code);
        if down {
            if self.down.iter().any(|(c, _)| *c == code) {
                return Step::Wait; // auto-repeat
            }
            self.down.push((code, now));
            if bit != 0 {
                self.held |= bit;
                self.lone = Some(code);
                return Step::Wait;
            }
            self.lone = None;
            if code == 1 && self.held == 0 {
                return Step::Cancel;
            }
            return self.finish(Found { code, mods: self.held, device, away }, now, now);
        }
        let Some(at) = self.down.iter().position(|(c, _)| *c == code) else { return Step::Wait };
        let (_, pressed) = self.down.remove(at);
        if bit == 0 {
            return Step::Wait;
        }
        self.held &= !bit;
        if self.lone.take() == Some(code) {
            return self.finish(Found { code, mods: self.held, device, away }, pressed, now);
        }
        Step::Wait
    }

    /// The device a press came from (Windows, macOS), which may come
    /// before or after the press itself.
    pub fn device(&mut self, code: u32, down: bool, device: DeviceRef, now: u64) -> Step {
        if !down {
            return Step::Wait;
        }
        if let Some((found, pressed, _)) = self.pending
            && found.code == code
            && pressed.abs_diff(now) <= DEVICE_WAIT_MS
        {
            self.pending = None;
            return Step::Found(Found { device: Some(device), ..found });
        }
        if self.hints.len() == HINTS {
            self.hints.pop_front();
        }
        self.hints.push_back(Hint { code, device, at: now });
        Step::Wait
    }

    /// When [`Detector::tick`] must be called next, if a press waits.
    pub fn deadline(&self) -> Option<u64> {
        self.pending.map(|(_, _, until)| until)
    }

    /// No device came in time: the press goes without one.
    pub fn tick(&mut self, now: u64) -> Step {
        match self.pending {
            Some((found, _, until)) if now >= until => {
                self.pending = None;
                Step::Found(found)
            }
            _ => Step::Wait,
        }
    }

    /// `pressed`: when it went down (a lone modifier is decided on its release).
    fn finish(&mut self, found: Found, pressed: u64, now: u64) -> Step {
        if found.device.is_some() || !self.hints_expected {
            return Step::Found(found);
        }
        let hint = self.hints.iter().rev().find(|h| h.code == found.code && h.at.abs_diff(pressed) <= DEVICE_WAIT_MS);
        if let Some(h) = hint {
            return Step::Found(Found { device: Some(h.device), ..found });
        }
        self.pending = Some((found, pressed, now.max(pressed + DEVICE_WAIT_MS)));
        Step::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: u32 = 29;
    const ALT: u32 = 56;
    const TAB: u32 = 15;

    #[test]
    fn a_key_comes_out_with_the_modifiers_held() {
        let mut d = Detector::new(false, false);
        assert_eq!(d.press(CTRL, true, None, false, 0), Step::Wait);
        assert_eq!(d.press(ALT, true, None, false, 1), Step::Wait);
        let Step::Found(f) = d.press(TAB, true, Some(DeviceRef(4)), false, 2) else { panic!() };
        assert_eq!((f.code, f.device), (TAB, Some(DeviceRef(4))));
        assert_eq!(f.trigger().to_string(), "ctrl+alt+tab");
        assert!(!f.is_button() && f.unusable().is_none());
    }

    #[test]
    fn a_modifier_alone_is_itself_and_held_keys_from_before_are_ignored() {
        let mut d = Detector::new(false, false);
        // The Enter that started the command goes up after detecting began.
        assert_eq!(d.press(28, false, None, false, 0), Step::Wait);
        assert_eq!(d.press(97, true, None, false, 1), Step::Wait);
        let Step::Found(f) = d.press(97, false, None, false, 2) else { panic!() };
        assert_eq!(f.trigger().to_string(), "rctrl");
        // Auto-repeat is one press.
        let mut d = Detector::new(false, false);
        assert_eq!(d.press(ALT, true, None, false, 0), Step::Wait);
        assert_eq!(d.press(ALT, true, None, false, 30), Step::Wait);
        assert!(matches!(d.press(ALT, false, None, false, 60), Step::Found(Found { code: ALT, mods: 0, .. })));
    }

    #[test]
    fn esc_alone_cancels_and_with_a_modifier_is_detected() {
        assert_eq!(Detector::new(false, false).press(1, true, None, false, 0), Step::Cancel);
        let mut d = Detector::new(false, false);
        d.press(42, true, None, false, 0);
        assert!(matches!(d.press(1, true, None, false, 1), Step::Found(Found { code: 1, mods: keynames::LSHIFT, .. })));
    }

    #[test]
    fn mouse_buttons_and_buttons_past_the_eighth() {
        let mut d = Detector::new(false, false);
        let Step::Found(f) = d.press(0x115, true, None, true, 0) else { panic!() };
        assert!(f.is_button() && f.away && f.unusable().is_none());
        assert_eq!(keynames::button_number(f.code), Some(6));
        let mut d = Detector::new(false, false);
        let Step::Found(f) = d.press(BEYOND_BUTTONS + 2, true, None, false, 0) else { panic!() };
        assert!(f.is_button() && f.unusable().is_some());
    }

    #[test]
    fn a_press_waits_briefly_for_its_device_from_either_side() {
        // The device report comes first.
        let mut d = Detector::new(true, false);
        assert_eq!(d.device(183, true, DeviceRef(7), 100), Step::Wait);
        assert!(matches!(d.press(183, true, None, false, 110), Step::Found(Found { device: Some(DeviceRef(7)), .. })));
        // It comes after.
        let mut d = Detector::new(true, false);
        assert_eq!(d.press(183, true, None, false, 100), Step::Wait);
        assert_eq!(d.deadline(), Some(100 + DEVICE_WAIT_MS));
        assert_eq!(d.device(30, true, DeviceRef(1), 105), Step::Wait, "another key's report");
        assert!(matches!(
            d.device(183, true, DeviceRef(7), 110),
            Step::Found(Found { code: 183, device: Some(DeviceRef(7)), .. })
        ));
        // Never: it goes without one.
        let mut d = Detector::new(true, false);
        d.press(183, true, None, false, 100);
        assert_eq!(d.tick(150), Step::Wait);
        assert!(matches!(d.tick(160), Step::Found(Found { device: None, .. })));
        assert_eq!(d.deadline(), None);
        // A stale report doesn't count.
        let mut d = Detector::new(true, false);
        d.device(183, true, DeviceRef(7), 0);
        assert_eq!(d.press(183, true, None, false, 1000), Step::Wait);
    }

    #[test]
    fn keys_only_lets_clicks_pass() {
        let mut d = Detector::new(false, true);
        assert_eq!(d.press(0x110, true, None, false, 0), Step::Wait);
        assert_eq!(d.press(BEYOND_BUTTONS, true, None, false, 1), Step::Wait);
        assert!(matches!(d.press(30, true, None, false, 2), Step::Found(Found { code: 30, .. })));
    }

    #[test]
    fn support_says_where_local_keys_are_seen() {
        assert_eq!(detect_support("x11"), Ok(None));
        assert_eq!(detect_support("portal"), Err(LOCAL_UNSEEN));
        assert!(detect_support("null").is_err());
        assert!(detect_support("").is_err());
    }
}
