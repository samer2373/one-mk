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
//! started the command; the backend says which, [`Detector::seed`]) and
//! auto-repeat are ignored, though a modifier held from before counts with
//! the next key. A mouse button is the button alone: modifiers held with it
//! are left out and said ([`Found::dropped`]), as button rules have none.
//! Esc alone cancels.
//!
//! Where the device comes separately (Windows, macOS) a press waits
//! [`DEVICE_WAIT_MS`] for the reports around it, matched by the backend's
//! own event times where both say: one device is the answer, two are none.

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
    /// Modifiers held with a mouse button, left out of it (sided bits): a
    /// button rule (`one-mk buttons map`) is for the button alone.
    pub dropped: u8,
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
        Trigger { groups: groups(self.mods), sided: 0, key: self.code as u16 }
    }

    /// The modifiers left out of a mouse button, as a rule would write them (`ctrl+shift`).
    pub fn dropped_text(&self) -> Option<String> {
        if self.dropped == 0 {
            return None;
        }
        let t = Trigger { groups: groups(self.dropped), sided: 0, key: 30 }.to_string();
        Some(t.trim_end_matches("+a").to_owned())
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

/// Sided modifier bits as groups (either side).
fn groups(mods: u8) -> u8 {
    (mods | (mods >> 4)) & 0xf
}

/// A key or mouse button going down or up, as a backend reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    pub code: u32,
    pub down: bool,
    /// The backend says this is the system's auto-repeat of a held key.
    pub repeat: bool,
    /// Its device, where the event itself says (X11).
    pub device: Option<DeviceRef>,
    /// When it happened by the backend's own clock, in milliseconds, where
    /// it says (the hook's and Raw Input's tick count on Windows; the tap's
    /// and IOHIDManager's event time on macOS). A press and a device report
    /// that both have one are matched by these, not by when they arrived.
    pub at: Option<u64>,
}

impl Press {
    pub fn new(code: u32, down: bool) -> Self {
        Press { code, down, ..Default::default() }
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

/// When something happened: by the backend's clock where it said, and when
/// it reached the daemon.
#[derive(Clone, Copy, Debug)]
struct When {
    native: Option<u64>,
    received: u64,
}

impl When {
    /// Within the device wait of each other: by the backend's clock where
    /// both have it, else by arrival.
    fn near(self, other: When) -> bool {
        match (self.native, other.native) {
            (Some(a), Some(b)) => a.abs_diff(b) <= DEVICE_WAIT_MS,
            _ => self.received.abs_diff(other.received) <= DEVICE_WAIT_MS,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Hint {
    code: u32,
    device: DeviceRef,
    when: When,
}

/// A press waiting for the device reports around it.
#[derive(Clone, Copy, Debug)]
struct Pending {
    found: Found,
    when: When,
    until: u64,
}

/// A key held: when it went down, and whether it was held before detecting
/// began (its release is no press).
#[derive(Clone, Copy, Debug)]
struct Down {
    code: u32,
    when: When,
    before: bool,
}

/// One detection: what is held, and what waits for its device.
#[derive(Debug, Default)]
pub struct Detector {
    /// Device reports come separately (`PressDevice`): a press waits a
    /// little for its own.
    hints_expected: bool,
    /// Mouse buttons don't count.
    keys_only: bool,
    /// Keys and buttons held (since detecting began, or from before).
    down: Vec<Down>,
    held: u8,
    /// A modifier pressed with nothing else since: let go, it is the press.
    lone: Option<u32>,
    pending: Option<Pending>,
    hints: VecDeque<Hint>,
}

impl Detector {
    /// `hints_expected`: the backend reports devices apart from presses;
    /// `keys_only`: mouse buttons don't count.
    pub fn new(hints_expected: bool, keys_only: bool) -> Self {
        Detector { hints_expected, keys_only, ..Default::default() }
    }

    /// Keys and buttons already held when detecting began (as the backend
    /// read them): their auto-repeat and release are no press, and the
    /// modifiers among them count for the next one (Ctrl held, then Tab, is
    /// Ctrl+Tab).
    pub fn seed(&mut self, codes: &[u32], now: u64) {
        for &code in codes {
            self.hold_before(code, When { native: None, received: now });
        }
    }

    fn hold_before(&mut self, code: u32, when: When) {
        if self.down.iter().any(|d| d.code == code) {
            return;
        }
        self.down.push(Down { code, when, before: true });
        self.held |= keynames::mod_bit(code);
    }

    /// A key or button went down or up (`away`: while this computer
    /// controlled another one; `now`: when it arrived).
    pub fn press(&mut self, p: Press, away: bool, now: u64) -> Step {
        if self.pending.is_some() || (self.keys_only && is_button(p.code)) {
            return Step::Wait;
        }
        let when = When { native: p.at, received: now };
        let code = p.code;
        let bit = keynames::mod_bit(code);
        if p.down {
            if self.down.iter().any(|d| d.code == code) {
                return Step::Wait; // auto-repeat
            }
            if p.repeat {
                // The repeat of a key held since before: never a press.
                self.hold_before(code, when);
                return Step::Wait;
            }
            self.down.push(Down { code, when, before: false });
            if bit != 0 {
                self.held |= bit;
                self.lone = Some(code);
                return Step::Wait;
            }
            self.lone = None;
            if code == 1 && self.held == 0 {
                return Step::Cancel;
            }
            // A button rule is for the button alone: modifiers held with it are left out.
            let (mods, dropped) = if is_button(code) { (0, self.held) } else { (self.held, 0) };
            return self.finish(Found { code, mods, device: p.device, away, dropped }, when, now);
        }
        let Some(at) = self.down.iter().position(|d| d.code == code) else { return Step::Wait };
        let held = self.down.remove(at);
        if bit == 0 {
            return Step::Wait;
        }
        self.held &= !bit;
        if !held.before && self.lone == Some(code) {
            self.lone = None;
            return self.finish(Found { code, mods: self.held, device: p.device, away, dropped: 0 }, held.when, now);
        }
        Step::Wait
    }

    /// The device a key or button went down on (Windows Raw Input, macOS
    /// IOHIDManager), which may come before or after the press itself.
    pub fn device(&mut self, p: Press, device: DeviceRef, now: u64) -> Step {
        if !p.down || p.repeat {
            return Step::Wait;
        }
        if self.hints.len() == HINTS {
            self.hints.pop_front();
        }
        self.hints.push_back(Hint { code: p.code, device, when: When { native: p.at, received: now } });
        Step::Wait
    }

    /// When [`Detector::tick`] must be called next, if a press waits.
    pub fn deadline(&self) -> Option<u64> {
        self.pending.map(|p| p.until)
    }

    /// The wait for devices is over: the press goes with the one device
    /// reported around it, or with none (none came, or two did).
    pub fn tick(&mut self, now: u64) -> Step {
        match self.pending {
            Some(p) if now >= p.until => {
                self.pending = None;
                Step::Found(self.attribute(p.found, p.when))
            }
            _ => Step::Wait,
        }
    }

    /// The device of a press: the one device that reported the same key
    /// going down near it. That report is used up; two devices are no answer.
    fn attribute(&mut self, found: Found, when: When) -> Found {
        let near = |h: &Hint| h.code == found.code && h.when.near(when);
        let mut devices = self.hints.iter().filter(|h| near(h)).map(|h| h.device);
        let Some(first) = devices.next() else { return found };
        if devices.any(|d| d != first) {
            return found;
        }
        if let Some(i) = self.hints.iter().position(near) {
            self.hints.remove(i);
        }
        Found { device: Some(first), ..found }
    }

    /// `when`: when it went down (a lone modifier is decided on its release).
    fn finish(&mut self, found: Found, when: When, now: u64) -> Step {
        if found.device.is_some() || !self.hints_expected {
            return Step::Found(found);
        }
        let until = when.received + DEVICE_WAIT_MS;
        if now >= until {
            return Step::Found(self.attribute(found, when));
        }
        self.pending = Some(Pending { found, when, until });
        Step::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: u32 = 29;
    const ALT: u32 = 56;
    const TAB: u32 = 15;

    fn down(code: u32) -> Press {
        Press::new(code, true)
    }
    fn up(code: u32) -> Press {
        Press::new(code, false)
    }
    fn at(p: Press, ms: u64) -> Press {
        Press { at: Some(ms), ..p }
    }

    #[test]
    fn a_key_comes_out_with_the_modifiers_held() {
        let mut d = Detector::new(false, false);
        assert_eq!(d.press(down(CTRL), false, 0), Step::Wait);
        assert_eq!(d.press(down(ALT), false, 1), Step::Wait);
        let Step::Found(f) = d.press(Press { device: Some(DeviceRef(4)), ..down(TAB) }, false, 2) else { panic!() };
        assert_eq!((f.code, f.device), (TAB, Some(DeviceRef(4))));
        assert_eq!(f.trigger().to_string(), "ctrl+alt+tab");
        assert!(!f.is_button() && f.unusable().is_none() && f.dropped_text().is_none());
    }

    #[test]
    fn a_modifier_alone_is_itself_and_held_keys_from_before_are_ignored() {
        let mut d = Detector::new(false, false);
        // The Enter that started the command goes up after detecting began.
        assert_eq!(d.press(up(28), false, 0), Step::Wait);
        assert_eq!(d.press(down(97), false, 1), Step::Wait);
        let Step::Found(f) = d.press(up(97), false, 2) else { panic!() };
        assert_eq!(f.trigger().to_string(), "rctrl");
        // Auto-repeat is one press.
        let mut d = Detector::new(false, false);
        assert_eq!(d.press(down(ALT), false, 0), Step::Wait);
        assert_eq!(d.press(down(ALT), false, 30), Step::Wait);
        assert!(matches!(d.press(up(ALT), false, 60), Step::Found(Found { code: ALT, mods: 0, .. })));
    }

    #[test]
    fn keys_held_when_it_began_never_win_and_their_modifiers_count() {
        // Seeded by the backend: Ctrl and A were held. A's repeat and release are no press;
        // Ctrl's release is no lone Ctrl; Ctrl+Tab is Ctrl+Tab.
        let mut d = Detector::new(false, false);
        d.seed(&[CTRL, 30], 0);
        assert_eq!(d.press(down(30), false, 1), Step::Wait);
        assert_eq!(d.press(up(30), false, 2), Step::Wait);
        let Step::Found(f) = d.press(down(TAB), false, 3) else { panic!() };
        assert_eq!(f.trigger().to_string(), "ctrl+tab");
        let mut d = Detector::new(false, false);
        d.seed(&[CTRL], 0);
        assert_eq!(d.press(up(CTRL), false, 1), Step::Wait, "held from before: not a lone Ctrl");
        // Ctrl held from before and Esc: Ctrl+Esc is detected, not a cancel.
        let mut d = Detector::new(false, false);
        d.seed(&[CTRL], 0);
        assert!(matches!(d.press(down(1), false, 1), Step::Found(Found { code: 1, mods: keynames::LCTRL, .. })));
        // Unseeded, the backend's repeat flag says the same: the first repeat of a key held
        // since before is no press (and a repeating Ctrl counts as held).
        let mut d = Detector::new(false, false);
        assert_eq!(d.press(Press { repeat: true, ..down(30) }, false, 0), Step::Wait);
        assert_eq!(d.press(Press { repeat: true, ..down(CTRL) }, false, 1), Step::Wait);
        assert_eq!(d.press(up(30), false, 2), Step::Wait);
        let Step::Found(f) = d.press(down(TAB), false, 3) else { panic!() };
        assert_eq!(f.trigger().to_string(), "ctrl+tab");
    }

    #[test]
    fn esc_alone_cancels_and_with_a_modifier_is_detected() {
        assert_eq!(Detector::new(false, false).press(down(1), false, 0), Step::Cancel);
        let mut d = Detector::new(false, false);
        d.press(down(42), false, 0);
        assert!(matches!(d.press(down(1), false, 1), Step::Found(Found { code: 1, mods: keynames::LSHIFT, .. })));
    }

    #[test]
    fn mouse_buttons_and_buttons_past_the_eighth() {
        let mut d = Detector::new(false, false);
        let Step::Found(f) = d.press(down(0x115), true, 0) else { panic!() };
        assert!(f.is_button() && f.away && f.unusable().is_none());
        assert_eq!(keynames::button_number(f.code), Some(6));
        let mut d = Detector::new(false, false);
        let Step::Found(f) = d.press(down(BEYOND_BUTTONS + 2), false, 0) else { panic!() };
        assert!(f.is_button() && f.unusable().is_some());
    }

    #[test]
    fn a_button_with_modifiers_held_is_the_button_alone() {
        // `keymap add -- ctrl+button4` and the key editor refuse a button; `buttons map`
        // takes a button without modifiers. So the press is button 4, and says Ctrl was held.
        let mut d = Detector::new(false, false);
        d.press(down(CTRL), false, 0);
        d.press(down(42), false, 1);
        let Step::Found(f) = d.press(down(0x113), false, 2) else { panic!() };
        assert_eq!((f.code, f.mods, f.trigger().to_string()), (0x113, 0, "button4".to_owned()));
        assert_eq!(f.dropped_text().as_deref(), Some("ctrl+shift"));
        assert!(!f.trigger().has_mods());
    }

    #[test]
    fn a_press_waits_out_the_device_window_from_either_side() {
        // The device report comes first.
        let mut d = Detector::new(true, false);
        assert_eq!(d.device(down(183), DeviceRef(7), 100), Step::Wait);
        assert_eq!(d.press(down(183), false, 110), Step::Wait);
        assert_eq!(d.deadline(), Some(110 + DEVICE_WAIT_MS));
        assert!(matches!(d.tick(110 + DEVICE_WAIT_MS), Step::Found(Found { device: Some(DeviceRef(7)), .. })));
        // It comes after.
        let mut d = Detector::new(true, false);
        assert_eq!(d.press(down(183), false, 100), Step::Wait);
        assert_eq!(d.device(down(30), DeviceRef(1), 105), Step::Wait, "another key's report");
        assert_eq!(d.device(down(183), DeviceRef(7), 110), Step::Wait);
        assert_eq!(d.tick(150), Step::Wait);
        assert!(matches!(d.tick(160), Step::Found(Found { code: 183, device: Some(DeviceRef(7)), .. })));
        // Never: it goes without one.
        let mut d = Detector::new(true, false);
        d.press(down(183), false, 100);
        assert_eq!(d.tick(150), Step::Wait);
        assert!(matches!(d.tick(160), Step::Found(Found { device: None, .. })));
        assert_eq!(d.deadline(), None);
        // A stale report doesn't count.
        let mut d = Detector::new(true, false);
        d.device(down(183), DeviceRef(7), 0);
        d.press(down(183), false, 1000);
        assert!(matches!(d.tick(1100), Step::Found(Found { device: None, .. })));
        // Nor a repeat's.
        let mut d = Detector::new(true, false);
        d.device(Press { repeat: true, ..down(183) }, DeviceRef(7), 100);
        d.press(down(183), false, 100);
        assert!(matches!(d.tick(200), Step::Found(Found { device: None, .. })));
    }

    #[test]
    fn two_devices_at_once_leave_it_unknown_and_a_report_is_used_once() {
        // The same key on two keyboards within the window: which one is not known.
        let mut d = Detector::new(true, false);
        d.device(down(30), DeviceRef(1), 100);
        d.device(down(30), DeviceRef(2), 120);
        d.press(down(30), false, 110);
        assert!(matches!(d.tick(200), Step::Found(Found { device: None, .. })));
        // Native times decide over arrival: keyboard 1's report arrived nearer the press, but
        // happened long before it (a backlog); keyboard 2's happened with it.
        let mut d = Detector::new(true, false);
        d.device(at(down(30), 1_000), DeviceRef(1), 100);
        d.device(at(down(30), 5_000), DeviceRef(2), 400);
        d.press(at(down(30), 5_003), false, 101);
        assert!(matches!(d.tick(500), Step::Found(Found { device: Some(DeviceRef(2)), .. })));
        // One report names one press: the lone modifier decided on release does not reuse
        // a report another press already took.
        let mut d = Detector::new(true, false);
        d.device(down(CTRL), DeviceRef(1), 0);
        let found = Found { code: CTRL, mods: 0, device: None, away: false, dropped: 0 };
        let when = When { native: None, received: 0 };
        assert_eq!(d.attribute(found, when).device, Some(DeviceRef(1)));
        assert_eq!(d.attribute(found, when).device, None, "used up");
    }

    #[test]
    fn keys_only_lets_clicks_pass() {
        let mut d = Detector::new(false, true);
        assert_eq!(d.press(down(0x110), false, 0), Step::Wait);
        assert_eq!(d.press(down(BEYOND_BUTTONS), false, 1), Step::Wait);
        assert!(matches!(d.press(down(30), false, 2), Step::Found(Found { code: 30, .. })));
    }

    #[test]
    fn support_says_where_local_keys_are_seen() {
        assert_eq!(detect_support("x11"), Ok(None));
        assert_eq!(detect_support("portal"), Err(LOCAL_UNSEEN));
        assert!(detect_support("null").is_err());
        assert!(detect_support("").is_err());
    }
}
