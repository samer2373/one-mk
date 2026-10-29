//! Platform-independent parts of the macOS backend: key and button mapping,
//! modifier flags, edge detection, click counting and scroll conversion.
//! Nothing here touches macOS APIs, so it is unit-tested on every OS.

use crate::Event;
use one_mk_proto::{Desktop, Pos, Side};

// ------------------------------------------------------------------- keys

/// macOS virtual keycodes (`kVK_*` from HIToolbox/Events.h) and the Linux
/// evdev code (`KEY_*`) for the same physical key. One entry per key in
/// both directions; extra one-way aliases are in `EVDEV_ALIASES`.
#[rustfmt::skip]
pub const KEYMAP: &[(u16, u32)] = &[
    // Letters.
    (0x00, 30),  // A
    (0x0B, 48),  // B
    (0x08, 46),  // C
    (0x02, 32),  // D
    (0x0E, 18),  // E
    (0x03, 33),  // F
    (0x05, 34),  // G
    (0x04, 35),  // H
    (0x22, 23),  // I
    (0x26, 36),  // J
    (0x28, 37),  // K
    (0x25, 38),  // L
    (0x2E, 50),  // M
    (0x2D, 49),  // N
    (0x1F, 24),  // O
    (0x23, 25),  // P
    (0x0C, 16),  // Q
    (0x0F, 19),  // R
    (0x01, 31),  // S
    (0x11, 20),  // T
    (0x20, 22),  // U
    (0x09, 47),  // V
    (0x0D, 17),  // W
    (0x07, 45),  // X
    (0x10, 21),  // Y
    (0x06, 44),  // Z
    // Digit row.
    (0x12, 2),   // 1
    (0x13, 3),   // 2
    (0x14, 4),   // 3
    (0x15, 5),   // 4
    (0x17, 6),   // 5
    (0x16, 7),   // 6
    (0x1A, 8),   // 7
    (0x1C, 9),   // 8
    (0x19, 10),  // 9
    (0x1D, 11),  // 0
    (0x1B, 12),  // Minus
    (0x18, 13),  // Equal
    // Punctuation.
    (0x21, 26),  // LeftBracket
    (0x1E, 27),  // RightBracket
    (0x29, 39),  // Semicolon
    (0x27, 40),  // Quote -> KEY_APOSTROPHE
    (0x32, 41),  // Grave
    (0x2A, 43),  // Backslash
    (0x2B, 51),  // Comma
    (0x2F, 52),  // Period -> KEY_DOT
    (0x2C, 53),  // Slash
    (0x0A, 86),  // ISO_Section -> KEY_102ND (the extra ISO key)
    // Editing and whitespace.
    (0x24, 28),  // Return -> KEY_ENTER
    (0x30, 15),  // Tab
    (0x31, 57),  // Space
    (0x33, 14),  // Delete -> KEY_BACKSPACE
    (0x75, 111), // ForwardDelete -> KEY_DELETE
    (0x35, 1),   // Escape
    (0x72, 110), // Help -> KEY_INSERT (same position; macOS maps PC Insert to Help)
    // Modifiers.
    (0x38, 42),  // Shift -> KEY_LEFTSHIFT
    (0x3C, 54),  // RightShift
    (0x3B, 29),  // Control -> KEY_LEFTCTRL
    (0x3E, 97),  // RightControl
    (0x3A, 56),  // Option -> KEY_LEFTALT
    (0x3D, 100), // RightOption -> KEY_RIGHTALT
    (0x37, 125), // Command -> KEY_LEFTMETA
    (0x36, 126), // RightCommand -> KEY_RIGHTMETA
    (0x39, 58),  // CapsLock
    (0x3F, 464), // Function -> KEY_FN
    (0x6E, 127), // ContextualMenu -> KEY_COMPOSE (the PC "Menu" key)
    // Function keys.
    (0x7A, 59),  // F1
    (0x78, 60),  // F2
    (0x63, 61),  // F3
    (0x76, 62),  // F4
    (0x60, 63),  // F5
    (0x61, 64),  // F6
    (0x62, 65),  // F7
    (0x64, 66),  // F8
    (0x65, 67),  // F9
    (0x6D, 68),  // F10
    (0x67, 87),  // F11
    (0x6F, 88),  // F12
    (0x69, 183), // F13
    (0x6B, 184), // F14
    (0x71, 185), // F15
    (0x6A, 186), // F16
    (0x40, 187), // F17
    (0x4F, 188), // F18
    (0x50, 189), // F19
    (0x5A, 190), // F20
    // Navigation.
    (0x73, 102), // Home
    (0x77, 107), // End
    (0x74, 104), // PageUp
    (0x79, 109), // PageDown
    (0x7B, 105), // LeftArrow
    (0x7C, 106), // RightArrow
    (0x7D, 108), // DownArrow
    (0x7E, 103), // UpArrow
    // Keypad.
    (0x52, 82),  // Keypad0
    (0x53, 79),  // Keypad1
    (0x54, 80),  // Keypad2
    (0x55, 81),  // Keypad3
    (0x56, 75),  // Keypad4
    (0x57, 76),  // Keypad5
    (0x58, 77),  // Keypad6
    (0x59, 71),  // Keypad7
    (0x5B, 72),  // Keypad8
    (0x5C, 73),  // Keypad9
    (0x41, 83),  // KeypadDecimal -> KEY_KPDOT
    (0x43, 55),  // KeypadMultiply -> KEY_KPASTERISK
    (0x45, 78),  // KeypadPlus
    (0x4E, 74),  // KeypadMinus
    (0x4B, 98),  // KeypadDivide -> KEY_KPSLASH
    (0x4C, 96),  // KeypadEnter
    (0x51, 117), // KeypadEquals -> KEY_KPEQUAL
    // KeypadClear sits where PC keyboards have Num Lock, but sending Num
    // Lock would toggle it on the peer and turn its keypad into arrows.
    (0x47, 355), // KeypadClear -> KEY_CLEAR
    // Media keys that exist as plain keycodes.
    (0x48, 115), // VolumeUp
    (0x49, 114), // VolumeDown
    (0x4A, 113), // Mute
    // JIS keyboards.
    (0x5D, 124), // JIS_Yen -> KEY_YEN
    (0x5E, 89),  // JIS_Underscore -> KEY_RO
    (0x5F, 95),  // JIS_KeypadComma -> KEY_KPJPCOMMA
    (0x68, 122), // JIS_Kana (HID LANG1) -> KEY_HANGEUL (Linux's LANG1)
    (0x66, 123), // JIS_Eisu (HID LANG2) -> KEY_HANJA (Linux's LANG2)
];

/// evdev keys without a key of their own on a Mac, sent where macOS itself
/// puts the equivalent PC key.
#[rustfmt::skip]
pub const EVDEV_ALIASES: &[(u32, u16)] = &[
    (99, 0x69),  // KEY_SYSRQ (Print Screen) -> F13
    (70, 0x6B),  // KEY_SCROLLLOCK -> F14
    (119, 0x71), // KEY_PAUSE -> F15
    (138, 0x72), // KEY_HELP -> Help
    (139, 0x6E), // KEY_MENU -> ContextualMenu
    (121, 0x2B), // KEY_KPCOMMA -> Comma
    (92, 0x68),  // KEY_HENKAN -> Kana
    (94, 0x66),  // KEY_MUHENKAN -> Eisu
    (93, 0x68),  // KEY_KATAKANAHIRAGANA -> Kana
    (85, 0x32),  // KEY_ZENKAKUHANKAKU -> Grave (same position)
];

pub fn mac_to_evdev(kc: u16) -> Option<u32> {
    KEYMAP.iter().find(|&&(m, _)| m == kc).map(|&(_, e)| e)
}

pub fn evdev_to_mac(code: u32) -> Option<u16> {
    KEYMAP
        .iter()
        .find(|&&(_, e)| e == code)
        .map(|&(m, _)| m)
        .or_else(|| EVDEV_ALIASES.iter().find(|&&(e, _)| e == code).map(|&(_, m)| m))
}

// ------------------------------------------------------------- media keys

/// `NX_SUBTYPE_AUX_CONTROL_BUTTONS`: the `NX_SYSDEFINED` subtype that
/// carries media keys (volume, brightness, play, eject, ...). They are not
/// key events on a Mac, and have no `kVK_*` code.
pub const NX_SUBTYPE_AUX_CONTROL_BUTTONS: i16 = 8;
const NX_KEYSTATE_DOWN: i64 = 0xA;
const NX_KEYSTATE_UP: i64 = 0xB;

/// A media key event as the HID system reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaEvent {
    /// `NX_KEYTYPE_*`.
    pub nx: u8,
    pub down: bool,
    /// An auto-repeat: the receiving side repeats by itself, so these are dropped.
    pub repeat: bool,
}

/// Decodes the `data1` of an `NX_SYSDEFINED` event: key type in bits 16-31,
/// key state (0xA down, 0xB up) in bits 8-15, bit 0 set for auto-repeat.
pub fn parse_media(subtype: i16, data1: i64) -> Option<MediaEvent> {
    if subtype != NX_SUBTYPE_AUX_CONTROL_BUTTONS {
        return None;
    }
    let nx = u8::try_from((data1 >> 16) & 0xFFFF).ok()?;
    let down = match (data1 >> 8) & 0xFF {
        NX_KEYSTATE_DOWN => true,
        NX_KEYSTATE_UP => false,
        _ => return None,
    };
    Some(MediaEvent { nx, down, repeat: data1 & 1 != 0 })
}

/// The `data1` for a media key event to post (the inverse of [`parse_media`]).
pub fn media_data1(nx: u8, down: bool, repeat: bool) -> i64 {
    let state = if down { NX_KEYSTATE_DOWN } else { NX_KEYSTATE_UP };
    (i64::from(nx) << 16) | (state << 8) | i64::from(repeat)
}

/// The canonical key for a media key event, if it is a key we forward.
/// Repeats give `None`.
pub fn media_to_evdev(ev: MediaEvent) -> Option<(u32, bool)> {
    if ev.repeat {
        return None;
    }
    one_mk_proto::mediakeys::from_mac_nx(ev.nx).map(|code| (code, ev.down))
}

// -------------------------------------------------------------- modifiers

/// `CGEventFlags` bits. The device-dependent bits (from IOKit's
/// `NX_DEVICE*KEYMASK`) tell left and right modifiers apart.
pub mod flags {
    pub const ALPHA_SHIFT: u64 = 0x0001_0000;
    pub const SHIFT: u64 = 0x0002_0000;
    pub const CONTROL: u64 = 0x0004_0000;
    pub const ALTERNATE: u64 = 0x0008_0000;
    pub const COMMAND: u64 = 0x0010_0000;
    pub const NUMERIC_PAD: u64 = 0x0020_0000;
    pub const SECONDARY_FN: u64 = 0x0080_0000;
    pub const NON_COALESCED: u64 = 0x0000_0100;

    pub const DEV_LCTL: u64 = 0x0000_0001;
    pub const DEV_LSHIFT: u64 = 0x0000_0002;
    pub const DEV_RSHIFT: u64 = 0x0000_0004;
    pub const DEV_LCMD: u64 = 0x0000_0008;
    pub const DEV_RCMD: u64 = 0x0000_0010;
    pub const DEV_LALT: u64 = 0x0000_0020;
    pub const DEV_RALT: u64 = 0x0000_0040;
    pub const DEV_RCTL: u64 = 0x0000_2000;
    pub const DEV_ALL: u64 = DEV_LCTL | DEV_LSHIFT | DEV_RSHIFT | DEV_LCMD | DEV_RCMD | DEV_LALT | DEV_RALT | DEV_RCTL;
}

pub const MAC_CAPS_LOCK: u16 = 0x39;
const KEY_CAPSLOCK: u32 = 58;

/// evdev modifier code -> (generic flag, device-dependent flag).
#[rustfmt::skip]
const MODIFIERS: &[(u32, u64, u64)] = &[
    (42, flags::SHIFT, flags::DEV_LSHIFT),
    (54, flags::SHIFT, flags::DEV_RSHIFT),
    (29, flags::CONTROL, flags::DEV_LCTL),
    (97, flags::CONTROL, flags::DEV_RCTL),
    (56, flags::ALTERNATE, flags::DEV_LALT),
    (100, flags::ALTERNATE, flags::DEV_RALT),
    (125, flags::COMMAND, flags::DEV_LCMD),
    (126, flags::COMMAND, flags::DEV_RCMD),
    (464, flags::SECONDARY_FN, 0),
];

fn modifier_bits(evdev: u32) -> Option<(u64, u64)> {
    MODIFIERS.iter().find(|m| m.0 == evdev).map(|m| (m.1, m.2))
}

/// Whether the shortcut tap may act on a key: not one we injected (a peer
/// typing here: its chords are for programs here, or were the peer's own
/// shortcuts already), and not while a peer controls this computer at all.
pub fn hotkey_applies(injected_by_us: bool, controlled: bool) -> bool {
    !injected_by_us && !controlled
}

/// Whether a mac keycode is a modifier, which macOS reports and expects as
/// `FlagsChanged` events rather than key down/up.
pub fn is_mac_modifier(kc: u16) -> bool {
    kc == MAC_CAPS_LOCK || mac_to_evdev(kc).and_then(modifier_bits).is_some()
}

/// For a captured `FlagsChanged` event: is the modifier `kc` now down?
/// `None` for caps lock (a toggle, see `MAC_CAPS_LOCK`) and non-modifiers.
pub fn modifier_down(kc: u16, event_flags: u64) -> Option<bool> {
    let (generic, device) = modifier_bits(mac_to_evdev(kc)?)?;
    if device != 0 && event_flags & flags::DEV_ALL != 0 {
        Some(event_flags & device != 0)
    } else {
        Some(event_flags & generic != 0)
    }
}

/// Modifier state of injected input, turned into event flags so that
/// shortcuts like Cmd+C and Shift+click work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    held: [bool; MODIFIERS.len()],
    caps: bool,
    caps_held: bool,
}

impl Modifiers {
    /// Records a key event. Returns true if `evdev` is a modifier (including
    /// caps lock, which toggles on press).
    pub fn update(&mut self, evdev: u32, down: bool) -> bool {
        if evdev == KEY_CAPSLOCK {
            // Only a real press toggles; repeated "down"s while it is held
            // (PC keyboards auto-repeat Caps Lock) must not.
            if down && !self.caps_held {
                self.caps = !self.caps;
            }
            self.caps_held = down;
            return true;
        }
        match MODIFIERS.iter().position(|m| m.0 == evdev) {
            Some(i) => {
                self.held[i] = down;
                true
            }
            None => false,
        }
    }

    pub fn flags(&self) -> u64 {
        let mut f = flags::NON_COALESCED;
        for (i, m) in MODIFIERS.iter().enumerate() {
            if self.held[i] {
                f |= m.1 | m.2;
            }
        }
        if self.caps {
            f |= flags::ALPHA_SHIFT;
        }
        f
    }

    /// Whether this Caps Lock event is a repeat of one already held, and
    /// so must not be posted (each posted press toggles Caps Lock).
    pub fn is_caps_repeat(&self, evdev: u32, down: bool) -> bool {
        evdev == KEY_CAPSLOCK && down && self.caps_held
    }
}

// ------------------------------------------------------------- key repeat

/// macOS key repeat settings (`InitialKeyRepeat`, `KeyRepeat` in the global
/// preferences domain) are in units of 15 ms.
const REPEAT_UNIT_MS: u64 = 15;

/// `(delay, interval)` for synthesized key repeat from the user's settings,
/// or `None` if key repeat is turned off. Missing values use macOS's
/// defaults.
pub fn repeat_timing(initial: Option<i64>, rate: Option<i64>) -> Option<(std::time::Duration, std::time::Duration)> {
    let initial = initial.unwrap_or(25);
    let rate = rate.unwrap_or(6);
    // "Off" in System Settings stores a huge value (300000).
    if !(1..=1000).contains(&rate) || !(1..=1000).contains(&initial) {
        return None;
    }
    let ms = |units: i64| std::time::Duration::from_millis(units as u64 * REPEAT_UNIT_MS);
    Some((ms(initial), ms(rate)))
}

/// Flags real keyboards add to some keys: keypad keys and arrows carry
/// NumericPad, and function/navigation keys carry SecondaryFn.
pub fn key_extra_flags(kc: u16) -> u64 {
    match kc {
        0x7B..=0x7E => flags::NUMERIC_PAD | flags::SECONDARY_FN,
        0x41 | 0x43 | 0x45 | 0x47 | 0x4B | 0x4C | 0x4E | 0x51..=0x59 | 0x5B | 0x5C => flags::NUMERIC_PAD,
        0x40 | 0x4F | 0x50 | 0x5A | 0x60..=0x65 | 0x67 | 0x69..=0x6B | 0x6D | 0x6F | 0x71..=0x79 | 0x7A => {
            flags::SECONDARY_FN
        }
        _ => 0,
    }
}

// ---------------------------------------------------------------- buttons

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
pub const BTN_SIDE: u32 = 0x113;
pub const BTN_EXTRA: u32 = 0x114;
const BTN_FORWARD: u32 = 0x115;
const BTN_BACK: u32 = 0x116;

/// evdev button -> macOS button number (0 left, 1 right, 2 middle, 3 back, 4 forward).
pub fn evdev_to_mac_button(button: u32) -> Option<u32> {
    match button {
        BTN_LEFT => Some(0),
        BTN_RIGHT => Some(1),
        BTN_MIDDLE => Some(2),
        BTN_SIDE | BTN_BACK => Some(3),
        BTN_EXTRA | BTN_FORWARD => Some(4),
        _ => None,
    }
}

pub fn mac_to_evdev_button(n: u32) -> Option<u32> {
    match n {
        0 => Some(BTN_LEFT),
        1 => Some(BTN_RIGHT),
        2 => Some(BTN_MIDDLE),
        3 => Some(BTN_SIDE),
        4 => Some(BTN_EXTRA),
        _ => None,
    }
}

/// `CGEventType` values (down, up, dragged) for a macOS button number.
pub fn mouse_event_types(n: u32) -> (u32, u32, u32) {
    match n {
        0 => (1, 2, 6),
        1 => (3, 4, 7),
        _ => (25, 26, 27),
    }
}

/// Counts double and triple clicks: a press of the same button soon after
/// and near the previous one continues the sequence.
#[derive(Clone, Debug)]
pub struct ClickCounter {
    interval: f64,
    slop: f64,
    last: Option<(u32, f64, Pos, i64)>,
    pressed: [i64; 8],
}

impl ClickCounter {
    /// `interval` in seconds, `slop` in points.
    pub fn new(interval: f64, slop: f64) -> Self {
        Self { interval, slop, last: None, pressed: [1; 8] }
    }

    /// A press of button `n` at time `t` (seconds); returns its click count.
    pub fn press(&mut self, n: u32, t: f64, pos: Pos) -> i64 {
        let count = match self.last {
            Some((b, lt, lp, c))
                if b == n
                    && t - lt <= self.interval
                    && t >= lt
                    && (pos.x - lp.x).abs() <= self.slop
                    && (pos.y - lp.y).abs() <= self.slop =>
            {
                c + 1
            }
            _ => 1,
        };
        self.last = Some((n, t, pos, count));
        if let Some(p) = self.pressed.get_mut(n as usize) {
            *p = count;
        }
        count
    }

    /// The release carries the same count as its press.
    pub fn release(&self, n: u32) -> i64 {
        self.pressed.get(n as usize).copied().unwrap_or(1)
    }
}

// ----------------------------------------------------------------- scroll

/// Splits fractional deltas into whole units, carrying the remainder.
#[derive(Clone, Copy, Debug, Default)]
pub struct Accum {
    x: f64,
    y: f64,
}

impl Accum {
    pub fn take(&mut self, dx: f64, dy: f64) -> (i32, i32) {
        self.x += dx;
        self.y += dy;
        let (ix, iy) = (self.x.trunc(), self.y.trunc());
        self.x -= ix;
        self.y -= iy;
        (ix as i32, iy as i32)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Wire scroll (positive dy = down, positive dx = right) to macOS wheel
/// values (positive wheel1 = up, positive wheel2 = left): (wheel1, wheel2).
pub fn wire_to_mac_scroll(dx: f64, dy: f64) -> (f64, f64) {
    (-dy, -dx)
}

/// Lines a synthesized line-scroll event moves per wheel notch from a peer.
/// A posted line event skips macOS's wheel acceleration and moves one line
/// (about 10 points), far less than a native notch, which goes through
/// that acceleration. Three lines is what a notch scrolls on Windows and
/// GNOME, and close to a native Mac notch at the default scroll speed.
pub const LINES_PER_NOTCH: f64 = 3.0;

/// Wire discrete scroll (120 per notch) to macOS line deltas (wheel1, wheel2).
pub fn wire_discrete_to_mac_lines(dx: i32, dy: i32) -> (f64, f64) {
    let notches = |v: i32| f64::from(v) / 120.0 * LINES_PER_NOTCH;
    wire_to_mac_scroll(notches(dx), notches(dy))
}

/// A captured macOS scroll event, as read from its fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MacScroll {
    pub continuous: bool,
    pub line_x: i64,
    pub line_y: i64,
    pub point_x: f64,
    pub point_y: f64,
    /// `kCGScrollWheelEventScrollPhase`.
    pub phase: u32,
    /// `kCGScrollWheelEventMomentumPhase`.
    pub momentum: u32,
}

const PHASE_ENDED: u32 = 4;
const PHASE_CANCELLED: u32 = 8;

/// Turns a captured scroll event into wire events. Follows libei semantics:
/// smooth deltas while fingers are down, then `ScrollStop` when they lift.
/// The receiving side decides on kinetic scrolling, so macOS's own momentum
/// events are dropped.
pub fn scroll_to_wire(s: &MacScroll) -> [Option<Event>; 2] {
    if s.continuous {
        if s.momentum != 0 {
            return [None, None];
        }
        let delta = (s.point_x != 0.0 || s.point_y != 0.0).then(|| Event::Scroll { dx: -s.point_x, dy: -s.point_y });
        let stop = (s.phase & (PHASE_ENDED | PHASE_CANCELLED) != 0).then_some(Event::ScrollStop);
        [delta, stop]
    } else {
        let ev = (s.line_x != 0 || s.line_y != 0).then(|| Event::ScrollDiscrete {
            dx: (-s.line_x).saturating_mul(120).clamp(i32::MIN.into(), i32::MAX.into()) as i32,
            dy: (-s.line_y).saturating_mul(120).clamp(i32::MIN.into(), i32::MAX.into()) as i32,
        });
        [ev, None]
    }
}

// ------------------------------------------------------------------ edges

/// The cursor is clamped to the screen, so it counts as "at" an edge when
/// within this many points of it.
const EDGE_SLOP: f64 = 1.0;

/// Whether the pointer at `p` (after moving by `dx, dy`) is pushing against
/// one of the armed `sides`. Only outer edges of the whole desktop count,
/// matching the Linux backend's pointer barriers, so moving between
/// monitors or into the gap beside a smaller monitor never triggers.
pub fn edge_hit(desktop: &Desktop, sides: &[Side], p: Pos, dx: f64, dy: f64) -> Option<Side> {
    // Prefer the axis the pointer is mostly moving along (matters in corners).
    let horizontal_first = dx.abs() >= dy.abs();
    let is_horizontal = |s: &Side| matches!(s, Side::Left | Side::Right);
    let first = sides.iter().filter(|s| is_horizontal(s) == horizontal_first);
    let second = sides.iter().filter(|s| is_horizontal(s) != horizontal_first);
    first.chain(second).copied().find(|&side| {
        let pushing = match side {
            Side::Left => dx < 0.0,
            Side::Right => dx > 0.0,
            Side::Top => dy < 0.0,
            Side::Bottom => dy > 0.0,
        };
        pushing
            && desktop.outer_edges(side).any(|r| {
                let (x, y, right, bottom) = (f64::from(r.x), f64::from(r.y), f64::from(r.right()), f64::from(r.bottom()));
                let along_y = p.y >= y && p.y < bottom;
                let along_x = p.x >= x && p.x < right;
                match side {
                    Side::Left => along_y && p.x < x + EDGE_SLOP,
                    Side::Right => along_y && p.x >= right - EDGE_SLOP,
                    Side::Top => along_x && p.y < y + EDGE_SLOP,
                    Side::Bottom => along_x && p.y >= bottom - EDGE_SLOP,
                }
            })
    })
}

// ------------------------------------------------------- relative motion

/// Where a relative move (gaming mode) leaves the cursor: moved by the
/// delta, and if that is off every display, stopped at the edge of the one
/// it was on, as macOS stops it. Tracked here rather than read back after
/// posting, since `CGEventPost` is asynchronous and the location read
/// right after can still be the old one.
pub fn moved_by(desktop: Option<&Desktop>, from: Pos, dx: f64, dy: f64) -> Pos {
    let to = Pos::new(from.x + dx, from.y + dy);
    let Some(d) = desktop.filter(|d| d.screen_at(to).is_none()) else { return to };
    d.screen_at(from).map_or_else(|| d.clamp(to), |r| r.clamp(to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_ignore_injected_keys_and_controlled_time() {
        assert!(hotkey_applies(false, false), "the user's own keys at home");
        assert!(!hotkey_applies(true, false), "our injected keys");
        assert!(!hotkey_applies(false, true), "anything while a peer controls this computer");
        assert!(!hotkey_applies(true, true));
    }

    #[test]
    fn relative_moves_stay_on_the_displays() {
        use one_mk_proto::Rect;
        // A 1440x900 display with a 1920x1080 one to its right, tops aligned.
        let d = Desktop::new(vec![Rect::new(0, 0, 1440, 900), Rect::new(1440, 0, 1920, 1080)]);
        let p = moved_by(Some(&d), Pos::new(100.0, 100.0), 10.0, -5.0);
        assert_eq!(p, Pos::new(110.0, 95.0));
        // Pushing past the left edge stops on it; the deltas still go out.
        assert_eq!(moved_by(Some(&d), p, -500.0, 0.0), Pos::new(0.0, 95.0));
        // Onto the other display, and below the first one's bottom only there.
        assert_eq!(moved_by(Some(&d), Pos::new(1400.0, 800.0), 100.0, 200.0), Pos::new(1500.0, 1000.0));
        let q = moved_by(Some(&d), Pos::new(1400.0, 800.0), 0.0, 200.0);
        assert!(q.y < 900.0 && q.y > 899.0 && q.x == 1400.0, "stopped at its display's bottom, {q:?}");
        // Unknown displays: just the sum.
        assert_eq!(moved_by(None, Pos::new(1.0, 2.0), 3.0, 4.0), Pos::new(4.0, 6.0));
    }
    use one_mk_proto::Rect;
    use std::collections::HashSet;

    #[test]
    fn keymap_is_one_to_one() {
        let macs: HashSet<u16> = KEYMAP.iter().map(|k| k.0).collect();
        let evs: HashSet<u32> = KEYMAP.iter().map(|k| k.1).collect();
        assert_eq!(macs.len(), KEYMAP.len(), "duplicate mac keycode");
        assert_eq!(evs.len(), KEYMAP.len(), "duplicate evdev code");
        for &(m, e) in KEYMAP {
            assert_eq!(mac_to_evdev(m), Some(e));
            assert_eq!(evdev_to_mac(e), Some(m));
        }
        for &(e, m) in EVDEV_ALIASES {
            assert!(!evs.contains(&e), "alias {e} shadows a primary mapping");
            assert!(macs.contains(&m), "alias {e} targets an unmapped mac key");
            assert_eq!(evdev_to_mac(e), Some(m));
        }
    }

    #[test]
    fn keymap_covers_the_keyboard() {
        // Letters A-Z.
        let letters = [30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44];
        // Digits 1-0, F1-F12, F13-F20, modifiers, navigation, keypad.
        let digits = 2..=11;
        let f1_10 = 59..=68;
        let f13_20 = 183..=190;
        let mods = [29, 42, 54, 56, 58, 97, 100, 125, 126];
        let nav = [1, 14, 15, 28, 57, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111];
        let keypad = [55, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 96, 98, 117];
        let all = letters
            .into_iter()
            .chain(digits)
            .chain(f1_10)
            .chain([87, 88])
            .chain(f13_20)
            .chain(mods)
            .chain(nav)
            .chain(keypad);
        for code in all {
            assert!(evdev_to_mac(code).is_some(), "evdev {code} unmapped");
        }
        assert_eq!(evdev_to_mac(30), Some(0x00)); // A
        assert_eq!(evdev_to_mac(125), Some(0x37)); // left Command
        assert_eq!(evdev_to_mac(126), Some(0x36)); // right Command
        assert_eq!(evdev_to_mac(86), Some(0x0A)); // ISO section
        assert_eq!(mac_to_evdev(0x7A), Some(59)); // F1
        assert_eq!(mac_to_evdev(0x24), Some(28)); // Return
    }

    #[test]
    fn media_key_events() {
        // Volume up pressed and released, as the HID system reports it.
        assert_eq!(parse_media(8, 0x0000_0A00), Some(MediaEvent { nx: 0, down: true, repeat: false }));
        assert_eq!(parse_media(8, 0x0000_0B00), Some(MediaEvent { nx: 0, down: false, repeat: false }));
        // Play (16), pressed, the repeat flag set.
        assert_eq!(parse_media(8, 0x0010_0A01), Some(MediaEvent { nx: 16, down: true, repeat: true }));
        // Other subtypes and states are not media keys.
        assert_eq!(parse_media(7, 0x0010_0A00), None);
        assert_eq!(parse_media(8, 0x0010_0C00), None);
        for nx in 0..=40u8 {
            for (down, repeat) in [(true, false), (false, false), (true, true)] {
                let d = media_data1(nx, down, repeat);
                assert_eq!(parse_media(8, d), Some(MediaEvent { nx, down, repeat }), "nx {nx}");
            }
        }
        // To the canonical code: volume up, play/pause; repeats and caps lock are dropped.
        assert_eq!(media_to_evdev(parse_media(8, 0x0000_0A00).unwrap()), Some((115, true)));
        assert_eq!(media_to_evdev(parse_media(8, 0x0010_0B00).unwrap()), Some((164, false)));
        assert_eq!(media_to_evdev(parse_media(8, 0x0010_0A01).unwrap()), None);
        assert_eq!(media_to_evdev(parse_media(8, media_data1(4, true, false)).unwrap()), None);
    }

    #[test]
    fn modifier_flags() {
        let mut m = Modifiers::default();
        assert!(m.update(125, true)); // left Command
        assert!(m.update(54, true)); // right Shift
        assert!(!m.update(30, true)); // A is not a modifier
        let f = m.flags();
        assert_eq!(f & flags::COMMAND, flags::COMMAND);
        assert_eq!(f & flags::SHIFT, flags::SHIFT);
        assert_eq!(f & flags::DEV_LCMD, flags::DEV_LCMD);
        assert_eq!(f & flags::DEV_RSHIFT, flags::DEV_RSHIFT);
        assert_eq!(f & (flags::CONTROL | flags::ALTERNATE | flags::ALPHA_SHIFT), 0);
        m.update(125, false);
        assert_eq!(m.flags() & flags::COMMAND, 0);

        // Caps lock toggles on press only.
        m.update(58, true);
        m.update(58, false);
        assert_ne!(m.flags() & flags::ALPHA_SHIFT, 0);
        m.update(58, true);
        assert_eq!(m.flags() & flags::ALPHA_SHIFT, 0);
    }

    #[test]
    fn captured_modifiers() {
        // Both shifts down, then left released: right shift's device bit stays.
        let both = flags::SHIFT | flags::DEV_LSHIFT | flags::DEV_RSHIFT;
        assert_eq!(modifier_down(0x38, both), Some(true));
        let right_only = flags::SHIFT | flags::DEV_RSHIFT;
        assert_eq!(modifier_down(0x38, right_only), Some(false));
        assert_eq!(modifier_down(0x3C, right_only), Some(true));
        // Synthetic events without device bits fall back to the generic flag.
        assert_eq!(modifier_down(0x37, flags::COMMAND), Some(true));
        assert_eq!(modifier_down(0x37, 0), Some(false));
        assert_eq!(modifier_down(MAC_CAPS_LOCK, flags::ALPHA_SHIFT), None);
        assert_eq!(modifier_down(0x00, 0), None);
        assert!(is_mac_modifier(0x3B) && is_mac_modifier(MAC_CAPS_LOCK) && !is_mac_modifier(0x31));
    }

    #[test]
    fn extra_flags() {
        assert_eq!(key_extra_flags(0x7E), flags::NUMERIC_PAD | flags::SECONDARY_FN); // Up
        assert_eq!(key_extra_flags(0x52), flags::NUMERIC_PAD); // Keypad0
        assert_eq!(key_extra_flags(0x7A), flags::SECONDARY_FN); // F1
        assert_eq!(key_extra_flags(0x00), 0);
    }

    #[test]
    fn buttons() {
        for b in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA] {
            assert_eq!(mac_to_evdev_button(evdev_to_mac_button(b).unwrap()), Some(b));
        }
        assert_eq!(evdev_to_mac_button(BTN_BACK), Some(3));
        assert_eq!(mouse_event_types(0), (1, 2, 6));
        assert_eq!(mouse_event_types(4), (25, 26, 27));
    }

    #[test]
    fn click_counting() {
        let mut c = ClickCounter::new(0.5, 4.0);
        let p = Pos::new(100.0, 100.0);
        assert_eq!(c.press(0, 0.0, p), 1);
        assert_eq!(c.release(0), 1);
        assert_eq!(c.press(0, 0.2, Pos::new(102.0, 101.0)), 2);
        assert_eq!(c.release(0), 2);
        assert_eq!(c.press(0, 0.4, p), 3);
        // Too slow.
        assert_eq!(c.press(0, 1.0, p), 1);
        // Too far.
        assert_eq!(c.press(0, 1.1, Pos::new(120.0, 100.0)), 1);
        // A different button starts over.
        assert_eq!(c.press(1, 1.2, Pos::new(120.0, 100.0)), 1);
        assert_eq!(c.press(0, 1.3, Pos::new(120.0, 100.0)), 1);
    }

    #[test]
    fn accum() {
        let mut a = Accum::default();
        assert_eq!(a.take(0.4, -0.4), (0, 0));
        assert_eq!(a.take(0.7, -0.7), (1, -1));
        assert_eq!(a.take(2.0, 0.0), (2, 0));
        a.reset();
        assert_eq!(a.take(0.5, 0.5), (0, 0));
    }

    #[test]
    fn scroll_signs() {
        // Wire "down" is negative on macOS's wheel axis.
        assert_eq!(wire_to_mac_scroll(0.0, 10.0), (-10.0, -0.0));
        assert_eq!(wire_to_mac_scroll(5.0, 0.0), (-0.0, -5.0));

        // A wheel notch "up" on macOS (+1 line) is -120 on the wire.
        let wheel = MacScroll { line_y: 1, ..Default::default() };
        assert_eq!(scroll_to_wire(&wheel), [Some(Event::ScrollDiscrete { dx: 0, dy: -120 }), None]);

        // Trackpad: smooth deltas, then a stop when the fingers lift.
        let pad = MacScroll { continuous: true, point_y: -12.0, phase: 2, ..Default::default() };
        assert_eq!(scroll_to_wire(&pad), [Some(Event::Scroll { dx: -0.0, dy: 12.0 }), None]);
        let end = MacScroll { continuous: true, phase: 4, ..Default::default() };
        assert_eq!(scroll_to_wire(&end), [None, Some(Event::ScrollStop)]);
        let momentum = MacScroll { continuous: true, point_y: 3.0, momentum: 2, ..Default::default() };
        assert_eq!(scroll_to_wire(&momentum), [None, None]);
    }

    fn mac_dual() -> Desktop {
        // Main 1440x900 at the origin; a taller external monitor to its
        // left (negative x), its top 300 points higher.
        Desktop::new(vec![Rect::new(0, 0, 1440, 900), Rect::new(-1920, -300, 1920, 1080)])
    }

    #[test]
    fn edges() {
        let d = mac_dual();
        let all = Side::ALL;
        // Pushing right at the right edge of the main display.
        assert_eq!(edge_hit(&d, &all, Pos::new(1439.0, 450.0), 3.0, 0.0), Some(Side::Right));
        assert_eq!(edge_hit(&d, &all, Pos::new(1439.5, 450.0), 3.0, 0.0), Some(Side::Right));
        // At the edge but moving away.
        assert_eq!(edge_hit(&d, &all, Pos::new(1439.0, 450.0), -3.0, 0.0), None);
        // Not armed.
        assert_eq!(edge_hit(&d, &[Side::Left], Pos::new(1439.0, 450.0), 3.0, 0.0), None);
        // Left edge of the external monitor.
        assert_eq!(edge_hit(&d, &all, Pos::new(-1920.0, 0.0), -1.0, 0.0), Some(Side::Left));
        // Moving between monitors is not an edge.
        assert_eq!(edge_hit(&d, &all, Pos::new(0.0, 450.0), -2.0, 0.0), None);
        // Top of the main display is below the desktop's top: not an outer edge.
        assert_eq!(edge_hit(&d, &all, Pos::new(700.0, 0.0), 0.0, -2.0), None);
        // Top of the external monitor is.
        assert_eq!(edge_hit(&d, &all, Pos::new(-700.0, -300.0), 0.0, -2.0), Some(Side::Top));
        // Bottom of the main display (900) is the desktop's bottom (780 for the external).
        assert_eq!(edge_hit(&d, &all, Pos::new(700.0, 899.0), 0.0, 1.0), Some(Side::Bottom));
        assert_eq!(edge_hit(&d, &all, Pos::new(-700.0, 779.0), 0.0, 1.0), None);
        // In a corner, the dominant direction wins.
        assert_eq!(edge_hit(&d, &all, Pos::new(1439.0, 899.0), 1.0, 5.0), Some(Side::Bottom));
        assert_eq!(edge_hit(&d, &all, Pos::new(1439.0, 899.0), 5.0, 1.0), Some(Side::Right));
    }

    #[test]
    fn caps_lock_repeats_do_not_toggle() {
        let mut m = Modifiers::default();
        assert!(!m.is_caps_repeat(58, true));
        m.update(58, true);
        assert_ne!(m.flags() & flags::ALPHA_SHIFT, 0);
        // A PC peer's auto-repeat: more downs while held.
        assert!(m.is_caps_repeat(58, true));
        m.update(58, true);
        m.update(58, true);
        assert_ne!(m.flags() & flags::ALPHA_SHIFT, 0);
        assert!(!m.is_caps_repeat(58, false));
        m.update(58, false);
        assert!(!m.is_caps_repeat(58, true));
        m.update(58, true);
        assert_eq!(m.flags() & flags::ALPHA_SHIFT, 0);
        assert!(!m.is_caps_repeat(30, true));
    }

    #[test]
    fn keypad_clear_is_not_num_lock() {
        // Sending Num Lock would toggle it on a PC peer.
        assert_ne!(mac_to_evdev(0x47), Some(69));
        assert_eq!(mac_to_evdev(0x47), Some(355));
        // A PC's Num Lock doesn't type Clear on the Mac either.
        assert_eq!(evdev_to_mac(69), None);
        assert_eq!(evdev_to_mac(355), Some(0x47));
    }

    #[test]
    fn mac_repeat_settings() {
        use std::time::Duration;
        assert_eq!(repeat_timing(Some(15), Some(2)), Some((Duration::from_millis(225), Duration::from_millis(30))));
        assert_eq!(repeat_timing(None, None), Some((Duration::from_millis(375), Duration::from_millis(90))));
        assert_eq!(repeat_timing(Some(25), Some(300_000)), None); // repeat off
        assert_eq!(repeat_timing(Some(0), Some(2)), None);
    }

    #[test]
    fn wheel_notch_scrolls_like_a_native_notch() {
        // One notch down from a peer: three lines down (negative wheel1).
        assert_eq!(wire_discrete_to_mac_lines(0, 120), (-3.0, -0.0));
        assert_eq!(wire_discrete_to_mac_lines(-120, 0), (-0.0, 3.0));
        // High-resolution wheels: an eighth of a notch accumulates.
        let mut a = Accum::default();
        let mut lines = 0;
        for _ in 0..8 {
            let (v, h) = wire_discrete_to_mac_lines(0, 15);
            lines += a.take(h, v).1;
        }
        assert_eq!(lines, -3);
    }
}
