//! Platform-independent half of the Windows backend: coordinate spaces, edge
//! detection, `SendInput` normalization, scroll conversion and the mapping
//! between Windows scan codes and evdev key codes.
//!
//! Nothing here calls Win32, so it is compiled and unit-tested on every OS.
//!
//! # Coordinate space
//!
//! Low-level hooks, `SetCursorPos`, `EnumDisplayMonitors` (for a per-monitor
//! DPI aware v2 process) and `SendInput` with `MOUSEEVENTF_VIRTUALDESK` all
//! work in *physical* pixels of the virtual desktop. Windows has no single
//! logical space when monitors use different scale factors: each monitor's
//! logical rect is its physical rect divided by its own scale, and those
//! rects overlap or leave gaps.
//!
//! So the backend works in physical pixels internally and reports to the
//! daemon a *uniformly* scaled space: physical pixels divided by the primary
//! monitor's scale factor ([`Space`]). Because one factor applies to the
//! whole desktop, the mapping is linear and monitor adjacency is preserved
//! exactly, and on the common single-scale setup (a laptop at 150%, or all
//! monitors at 100%) it is exactly Windows' logical pixels, matching the
//! logical pixels the Linux backend reports. `Event::Desktop`, `Activated`,
//! `Motion` deltas and `Inject::MoveTo` all use this one space.
//!
//! Effect on mixed-DPI setups: on a monitor whose scale differs from the
//! primary's, the pointer moves at that monitor's physical rate instead of
//! its logical one. E.g. primary at 100% and a 4K monitor at 200%: on the 4K
//! monitor a remote pointer covers the same number of *physical* pixels per
//! unit of motion, so it looks half as fast relative to the UI there, and the
//! 4K monitor appears twice as large to peers. Geometry stays consistent, so
//! edges, clamping and click positions remain exact.

// On other OSes only the tests use this module.
#![cfg_attr(not(windows), allow(dead_code))]

use one_mk_proto::{Desktop, Pos, Rect, Side};

// ------------------------------------------------------------ evdev codes

pub(crate) const BTN_LEFT: u32 = 0x110;
pub(crate) const BTN_RIGHT: u32 = 0x111;
pub(crate) const BTN_MIDDLE: u32 = 0x112;
pub(crate) const BTN_SIDE: u32 = 0x113;
pub(crate) const BTN_EXTRA: u32 = 0x114;

// Virtual-key codes needed by the pure mapping (values from WinUser.h).
pub(crate) const VK_PAUSE: u16 = 0x13;
pub(crate) const VK_NUMLOCK: u16 = 0x90;

/// One notch of a mouse wheel in Windows wheel units and in libei/evdev
/// high-resolution discrete units (both are 120ths of a notch).
pub(crate) const WHEEL_DELTA: i32 = 120;

/// Smooth (pixel) scroll to wheel units: one notch per 48 px. A Windows
/// notch scrolls three lines, roughly 45-60 px in most apps, so content moves
/// about as far as the fingers did on the peer's touchpad.
pub(crate) const PIXELS_PER_NOTCH: f64 = 48.0;

// --------------------------------------------------------------- geometry

/// The monitors in physical pixels plus the uniform scale to the reported
/// space. See the module docs for why.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Space {
    phys: Desktop,
    scale: f64,
    logical: Desktop,
}

impl Space {
    /// `screens` in physical pixels; `primary_dpi` is the effective DPI of
    /// the primary monitor (96 = 100%).
    pub(crate) fn new(screens: Vec<Rect>, primary_dpi: u32) -> Self {
        let scale = if primary_dpi == 0 { 1.0 } else { f64::from(primary_dpi) / 96.0 };
        let phys = Desktop::new(screens);
        // Scale edges, not sizes, so monitors that touch physically still
        // touch after scaling.
        let s = |v: i32| (f64::from(v) / scale).round() as i32;
        let logical = Desktop::new(
            phys.screens
                .iter()
                .map(|r| {
                    let (x, y) = (s(r.x), s(r.y));
                    Rect::new(x, y, (s(r.right()) - x).max(1), (s(r.bottom()) - y).max(1))
                })
                .collect(),
        );
        Self { phys, scale, logical }
    }

    pub(crate) fn phys(&self) -> &Desktop {
        &self.phys
    }

    /// What the daemon sees.
    pub(crate) fn logical(&self) -> &Desktop {
        &self.logical
    }

    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    /// A physical pixel to the reported space, clamped onto a screen.
    pub(crate) fn to_logical(&self, x: i32, y: i32) -> Pos {
        self.logical.clamp(Pos::new(f64::from(x) / self.scale, f64::from(y) / self.scale))
    }

    /// A physical delta (from the hook) to the reported space.
    pub(crate) fn delta_to_logical(&self, dx: i32, dy: i32) -> (f64, f64) {
        (f64::from(dx) / self.scale, f64::from(dy) / self.scale)
    }

    /// A reported-space position to the nearest physical pixel on a screen.
    pub(crate) fn to_phys(&self, p: Pos) -> (i32, i32) {
        let x = (p.x * self.scale).round();
        let y = (p.y * self.scale).round();
        let x = x.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        let y = y.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        clamp_px(&self.phys, x, y)
    }

    /// Which armed edge, if any, the physical pixel `(x, y)` is touching.
    ///
    /// Only edges on the outer boundary of the desktop count (the same rule
    /// as [`Desktop::outer_edges`]), and only along the span of the screen
    /// that owns that edge, so a monitor edge that faces a gap next to a
    /// larger monitor never triggers. Windows keeps the cursor on the union
    /// of the monitors, so "touching" means sitting on the last pixel row or
    /// column; positions past the edge (if a hook reports them) count too.
    pub(crate) fn edge_hit(&self, sides: &[Side], x: i32, y: i32) -> Option<Side> {
        sides.iter().copied().find(|&side| {
            self.phys.outer_edges(side).any(|r| match side {
                Side::Left => x <= r.x && (r.y..r.bottom()).contains(&y),
                Side::Right => x >= r.right() - 1 && (r.y..r.bottom()).contains(&y),
                Side::Top => y <= r.y && (r.x..r.right()).contains(&x),
                Side::Bottom => y >= r.bottom() - 1 && (r.x..r.right()).contains(&x),
            })
        })
    }

    /// Where to park the hidden cursor while input is captured: the centre
    /// of the screen it is on (or the largest screen), as far as possible
    /// from every edge so per-event deltas are never clipped.
    pub(crate) fn park_point(&self, x: i32, y: i32) -> (i32, i32) {
        let screens = &self.phys.screens;
        let r = screens
            .iter()
            .find(|r| (r.x..r.right()).contains(&x) && (r.y..r.bottom()).contains(&y))
            .or_else(|| screens.iter().max_by_key(|r| i64::from(r.w) * i64::from(r.h)))
            .expect("a desktop has at least one screen");
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    /// Absolute coordinates for `SendInput` with `MOUSEEVENTF_ABSOLUTE |
    /// MOUSEEVENTF_VIRTUALDESK`. See [`normalize_axis`].
    pub(crate) fn normalize(&self, x: i32, y: i32) -> (i32, i32) {
        let b = self.phys.bounds();
        (normalize_axis(x - b.x, b.w), normalize_axis(y - b.y, b.h))
    }
}

/// Maps pixel `offset` (0-based, from the virtual desktop origin) on an axis
/// `size` pixels long to 0..=65535.
///
/// Windows converts back with `pixel = n * size / 65536` (truncating; some
/// versions round). `ceil(offset * 65536 / size)` is the smallest `n` that
/// lands on `offset` either way: its error is below `size / 65536` < 1/2 px.
/// The naive `offset * 65535 / size` lands one pixel short on most of the
/// axis, and the right/bottom edge would never be reachable.
pub(crate) fn normalize_axis(offset: i32, size: i32) -> i32 {
    if size <= 1 {
        return 0;
    }
    let offset = i64::from(offset.clamp(0, size - 1));
    let size = i64::from(size);
    ((offset * 65536 + size - 1) / size).min(65535) as i32
}

/// Nearest pixel that is on a screen.
fn clamp_px(d: &Desktop, x: i32, y: i32) -> (i32, i32) {
    d.screens
        .iter()
        .map(|r| (x.clamp(r.x, r.right() - 1), y.clamp(r.y, r.bottom() - 1)))
        .min_by_key(|&(cx, cy)| {
            let (dx, dy) = (i64::from(cx) - i64::from(x), i64::from(cy) - i64::from(y));
            dx * dx + dy * dy
        })
        .unwrap_or((x, y))
}

// ----------------------------------------------------------------- scroll

/// A wheel event from the hook (signed delta from `mouseData`) to the wire's
/// discrete scroll. Windows: positive vertical = away from the user (up),
/// positive horizontal = right. Wire (libei/evdev): positive dy = down,
/// positive dx = right.
pub(crate) fn wheel_to_wire(delta: i16, horizontal: bool) -> (i32, i32) {
    let d = i32::from(delta);
    if horizontal { (d, 0) } else { (0, -d) }
}

/// Wire discrete scroll to `(hwheel, wheel)` for `SendInput`.
pub(crate) fn wire_to_wheel(dx: i32, dy: i32) -> (i32, i32) {
    (dx, dy.saturating_neg())
}

/// Accumulates smooth (pixel) scrolling and returns whole wheel units to
/// send, as `(hwheel, wheel)` in Windows' sign convention. Windows accepts
/// wheel deltas smaller than a notch, so fine scrolling stays fine.
#[derive(Default, Debug)]
pub(crate) struct SmoothScroll {
    h: f64,
    v: f64,
}

impl SmoothScroll {
    pub(crate) fn push(&mut self, dx: f64, dy: f64) -> (i32, i32) {
        let k = f64::from(WHEEL_DELTA) / PIXELS_PER_NOTCH;
        self.h += dx * k;
        self.v -= dy * k;
        let (h, v) = (self.h.trunc(), self.v.trunc());
        self.h -= h;
        self.v -= v;
        (h as i32, v as i32)
    }

    pub(crate) fn stop(&mut self) {
        *self = Self::default();
    }
}

// ------------------------------------------------------------------- keys

/// How to type an evdev key with `SendInput`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WinKey {
    /// `KEYEVENTF_SCANCODE`, plus `KEYEVENTF_EXTENDEDKEY` for E0 keys.
    Scan { scan: u16, extended: bool },
    /// Keys whose scan code sequence can't be expressed in a `KEYBDINPUT`
    /// (Pause is E1 1D 45) or is ambiguous (Num Lock vs Pause), sent by
    /// virtual-key code with the matching scan code.
    Vk { vk: u16, scan: u16, extended: bool },
}

const KEY_NUMLOCK: u32 = 69;
const KEY_PAUSE: u32 = 119;

/// Set 1 scan codes 0x01..=0x53 and 0x56..=0x58 (without E0) are equal to
/// their evdev codes: Linux key codes were numbered after the PC keyboard.
/// That covers Esc, digits, letters, punctuation, Backspace, Tab, Enter,
/// left Ctrl/Shift/Alt, right Shift, Space, Caps Lock, F1-F12, Num Lock,
/// Scroll Lock, the numpad (except Enter and /) and the ISO 102nd key.
fn is_identity_scan(scan: u16) -> bool {
    matches!(scan, 0x01..=0x53 | 0x56..=0x58)
}

/// `(scan, extended, evdev)` for everything that is not an identity code.
/// When a code appears twice, the first entry is the one used for injection.
#[rustfmt::skip]
const KEY_TABLE: &[(u16, bool, u32)] = &[
    // Non-extended codes beyond the identity range.
    (0x59, false, 117), // keypad =                 KEY_KPEQUAL
    (0x64, false, 183), // F13                      KEY_F13
    (0x65, false, 184), // F14
    (0x66, false, 185), // F15
    (0x67, false, 186), // F16
    (0x68, false, 187), // F17
    (0x69, false, 188), // F18
    (0x6A, false, 189), // F19
    (0x6B, false, 190), // F20
    (0x6C, false, 191), // F21
    (0x6D, false, 192), // F22
    (0x6E, false, 193), // F23
    (0x76, false, 194), // F24
    (0x70, false, 93),  // Katakana/Hiragana        KEY_KATAKANAHIRAGANA
    (0x73, false, 89),  // JIS \ _ (Ro)             KEY_RO
    (0x77, false, 91),  // Hiragana                 KEY_HIRAGANA
    (0x78, false, 90),  // Katakana                 KEY_KATAKANA
    (0x79, false, 92),  // Henkan (convert)         KEY_HENKAN
    (0x7B, false, 94),  // Muhenkan (no convert)    KEY_MUHENKAN
    (0x7D, false, 124), // JIS Yen                  KEY_YEN
    (0x7E, false, 121), // keypad , (Brazil)        KEY_KPCOMMA
    (0xF1, false, 123), // Hanja                    KEY_HANJA
    (0xF2, false, 122), // Han/Yeong                KEY_HANGEUL
    // Extended (E0) keys.
    (0x1C, true, 96),   // keypad Enter             KEY_KPENTER
    (0x1D, true, 97),   // right Ctrl               KEY_RIGHTCTRL
    (0x35, true, 98),   // keypad /                 KEY_KPSLASH
    (0x37, true, 99),   // Print Screen             KEY_SYSRQ
    (0x38, true, 100),  // right Alt / AltGr        KEY_RIGHTALT
    (0x47, true, 102),  // Home                     KEY_HOME
    (0x48, true, 103),  // Up                       KEY_UP
    (0x49, true, 104),  // Page Up                  KEY_PAGEUP
    (0x4B, true, 105),  // Left                     KEY_LEFT
    (0x4D, true, 106),  // Right                    KEY_RIGHT
    (0x4F, true, 107),  // End                      KEY_END
    (0x50, true, 108),  // Down                     KEY_DOWN
    (0x51, true, 109),  // Page Down                KEY_PAGEDOWN
    (0x52, true, 110),  // Insert                   KEY_INSERT
    (0x53, true, 111),  // Delete                   KEY_DELETE
    (0x5B, true, 125),  // left Windows             KEY_LEFTMETA
    (0x5C, true, 126),  // right Windows            KEY_RIGHTMETA
    (0x5D, true, 127),  // Menu / Apps              KEY_COMPOSE
    (0x5E, true, 116),  // Power                    KEY_POWER
    (0x5F, true, 142),  // Sleep                    KEY_SLEEP
    (0x63, true, 143),  // Wake                     KEY_WAKEUP
    (0x20, true, 113),  // Mute                     KEY_MUTE
    (0x2E, true, 114),  // Volume Down              KEY_VOLUMEDOWN
    (0x30, true, 115),  // Volume Up                KEY_VOLUMEUP
    (0x22, true, 164),  // Play/Pause               KEY_PLAYPAUSE
    (0x24, true, 166),  // Stop media               KEY_STOPCD
    (0x10, true, 165),  // Previous track           KEY_PREVIOUSSONG
    (0x19, true, 163),  // Next track               KEY_NEXTSONG
    (0x6D, true, 226),  // Media select             KEY_MEDIA
    (0x6C, true, 155),  // Mail                     KEY_MAIL
    (0x21, true, 140),  // Calculator               KEY_CALC
    (0x6B, true, 157),  // My Computer              KEY_COMPUTER
    (0x65, true, 217),  // Browser Search           KEY_SEARCH
    (0x32, true, 172),  // Browser Home             KEY_HOMEPAGE
    (0x6A, true, 158),  // Browser Back             KEY_BACK
    (0x69, true, 159),  // Browser Forward          KEY_FORWARD
    (0x68, true, 128),  // Browser Stop             KEY_STOP
    (0x67, true, 173),  // Browser Refresh          KEY_REFRESH
    (0x66, true, 156),  // Browser Favorites        KEY_BOOKMARKS
    // Receive-only aliases.
    (0x54, false, 99),  // Alt+Print Screen         KEY_SYSRQ
    (0x46, true, 119),  // Ctrl+Pause (Break)       KEY_PAUSE
    // Send-only aliases.
    (0x37, true, 210),  // KEY_PRINT                -> Print Screen
    (0x29, false, 85),  // KEY_ZENKAKUHANKAKU       -> JIS half/full width (` position)
];

/// The send-only aliases in [`KEY_TABLE`]; never used when reading scan codes.
fn is_send_only(evdev: u32) -> bool {
    matches!(evdev, 210 | 85)
}

/// Set 1 scan code (with the E0 prefix as `extended`) to evdev.
pub(crate) fn scan_to_evdev(scan: u16, extended: bool) -> Option<u32> {
    if !extended && is_identity_scan(scan) {
        return Some(u32::from(scan));
    }
    KEY_TABLE
        .iter()
        .find(|&&(s, e, code)| s == scan && e == extended && !is_send_only(code))
        .map(|&(_, _, code)| code)
}

/// evdev code to how `SendInput` should type it.
pub(crate) fn evdev_to_win(code: u32) -> Option<WinKey> {
    match code {
        KEY_PAUSE => return Some(WinKey::Vk { vk: VK_PAUSE, scan: 0x45, extended: false }),
        KEY_NUMLOCK => return Some(WinKey::Vk { vk: VK_NUMLOCK, scan: 0x45, extended: true }),
        _ => {}
    }
    if let Ok(scan) = u16::try_from(code)
        && is_identity_scan(scan)
    {
        return Some(WinKey::Scan { scan, extended: false });
    }
    KEY_TABLE
        .iter()
        .find(|&&(_, _, c)| c == code)
        .map(|&(scan, extended, _)| WinKey::Scan { scan, extended })
}

/// A `KBDLLHOOKSTRUCT` to an evdev code, or `None` to ignore the event.
pub(crate) fn hook_key_to_evdev(vk: u32, scan: u32, extended: bool) -> Option<u32> {
    // The fake left Ctrl that Windows sends before AltGr has bit 9 set in
    // its scan code. Forwarding it would make AltGr arrive as Ctrl+AltGr.
    if scan & 0x200 != 0 {
        return None;
    }
    // Pause (E1 1D 45) and Num Lock (45) both report scan 0x45; only the
    // virtual key tells them apart.
    match vk as u16 {
        VK_PAUSE => return Some(KEY_PAUSE),
        VK_NUMLOCK => return Some(KEY_NUMLOCK),
        _ => {}
    }
    if scan != 0 {
        return scan_to_evdev(scan as u16, extended);
    }
    // Some keyboards (and HID consumer-control buttons on some drivers)
    // deliver media keys with no scan code.
    vk_media_to_evdev(vk as u16)
}

fn vk_media_to_evdev(vk: u16) -> Option<u32> {
    Some(match vk {
        0xA6 => 158, // VK_BROWSER_BACK
        0xA7 => 159, // VK_BROWSER_FORWARD
        0xA8 => 173, // VK_BROWSER_REFRESH
        0xA9 => 128, // VK_BROWSER_STOP
        0xAA => 217, // VK_BROWSER_SEARCH
        0xAB => 156, // VK_BROWSER_FAVORITES
        0xAC => 172, // VK_BROWSER_HOME
        0xAD => 113, // VK_VOLUME_MUTE
        0xAE => 114, // VK_VOLUME_DOWN
        0xAF => 115, // VK_VOLUME_UP
        0xB0 => 163, // VK_MEDIA_NEXT_TRACK
        0xB1 => 165, // VK_MEDIA_PREV_TRACK
        0xB2 => 166, // VK_MEDIA_STOP
        0xB3 => 164, // VK_MEDIA_PLAY_PAUSE
        0xB4 => 155, // VK_LAUNCH_MAIL
        0xB5 => 226, // VK_LAUNCH_MEDIA_SELECT
        0xB6 => 157, // VK_LAUNCH_APP1 (My Computer)
        0xB7 => 140, // VK_LAUNCH_APP2 (Calculator)
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trio() -> Vec<Rect> {
        // A 1920x1080 primary, a 2560x1440 to its right (top-aligned) and a
        // 1280x1024 to the left of the primary at a negative origin, lower.
        vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 2560, 1440), Rect::new(-1280, 200, 1280, 1024)]
    }

    #[test]
    fn identity_keys() {
        assert_eq!(scan_to_evdev(0x01, false), Some(1)); // Esc
        assert_eq!(scan_to_evdev(0x1E, false), Some(30)); // A
        assert_eq!(scan_to_evdev(0x1D, false), Some(29)); // left Ctrl
        assert_eq!(scan_to_evdev(0x36, false), Some(54)); // right Shift
        assert_eq!(scan_to_evdev(0x3B, false), Some(59)); // F1
        assert_eq!(scan_to_evdev(0x44, false), Some(68)); // F10
        assert_eq!(scan_to_evdev(0x57, false), Some(87)); // F11
        assert_eq!(scan_to_evdev(0x58, false), Some(88)); // F12
        assert_eq!(scan_to_evdev(0x52, false), Some(82)); // keypad 0
        assert_eq!(scan_to_evdev(0x56, false), Some(86)); // ISO 102nd
    }

    #[test]
    fn extended_keys_differ_from_plain() {
        assert_eq!(scan_to_evdev(0x1D, true), Some(97)); // right Ctrl
        assert_eq!(scan_to_evdev(0x38, true), Some(100)); // right Alt
        assert_eq!(scan_to_evdev(0x1C, true), Some(96)); // keypad Enter
        assert_eq!(scan_to_evdev(0x52, true), Some(110)); // Insert, not keypad 0
        assert_eq!(scan_to_evdev(0x48, true), Some(103)); // Up, not keypad 8
        assert_eq!(scan_to_evdev(0x5B, true), Some(125)); // left Windows
        assert_eq!(scan_to_evdev(0x5C, true), Some(126)); // right Windows
        // Fake shifts around extended keys (E0 2A / E0 36) are dropped.
        assert_eq!(scan_to_evdev(0x2A, true), None);
        assert_eq!(scan_to_evdev(0x36, true), None);
    }

    #[test]
    fn function_keys_f1_to_f24() {
        let evdev: Vec<u32> = (59..=68).chain([87, 88]).chain(183..=194).collect();
        assert_eq!(evdev.len(), 24);
        for code in evdev {
            let Some(WinKey::Scan { scan, extended }) = evdev_to_win(code) else { panic!("F key {code}") };
            assert_eq!(scan_to_evdev(scan, extended), Some(code));
        }
    }

    #[test]
    fn every_key_round_trips() {
        let mut codes: Vec<u32> = (1..=0x53).chain(0x56..=0x58).collect();
        codes.extend(KEY_TABLE.iter().map(|&(_, _, c)| c));
        for code in codes {
            let win = evdev_to_win(code).unwrap_or_else(|| panic!("no Windows key for {code}"));
            let back = match win {
                WinKey::Scan { scan, extended } => scan_to_evdev(scan, extended),
                WinKey::Vk { vk, scan, extended } => hook_key_to_evdev(u32::from(vk), u32::from(scan), extended),
            };
            match code {
                210 => assert_eq!(back, Some(99)),     // KEY_PRINT types Print Screen
                85 => assert_eq!(back, Some(41)),      // Zenkaku/Hankaku is the ` key
                _ => assert_eq!(back, Some(code), "key {code} via {win:?}"),
            }
        }
    }

    #[test]
    fn scan_codes_are_unique_in_receive_direction() {
        for (i, &(s, e, c)) in KEY_TABLE.iter().enumerate() {
            if is_send_only(c) {
                continue;
            }
            assert!(e || !is_identity_scan(s), "{s:#x} shadows an identity code");
            for &(s2, e2, c2) in &KEY_TABLE[i + 1..] {
                if !is_send_only(c2) {
                    assert!(!(s == s2 && e == e2), "duplicate scan {s:#x} ext={e}");
                }
            }
        }
    }

    #[test]
    fn pause_numlock_altgr() {
        // Pause and Num Lock share scan 0x45; the VK decides.
        assert_eq!(hook_key_to_evdev(u32::from(VK_PAUSE), 0x45, false), Some(KEY_PAUSE));
        assert_eq!(hook_key_to_evdev(u32::from(VK_NUMLOCK), 0x45, true), Some(KEY_NUMLOCK));
        assert!(matches!(evdev_to_win(KEY_PAUSE), Some(WinKey::Vk { vk: VK_PAUSE, .. })));
        // AltGr's synthetic left Ctrl is ignored, the real one is not.
        assert_eq!(hook_key_to_evdev(0xA2, 0x21D, false), None);
        assert_eq!(hook_key_to_evdev(0xA2, 0x1D, false), Some(29));
        // Media key without a scan code falls back to the VK.
        assert_eq!(hook_key_to_evdev(0xAF, 0, false), Some(115));
        assert_eq!(hook_key_to_evdev(0xAF, 0x30, true), Some(115));
        assert_eq!(hook_key_to_evdev(0x41, 0, false), None);
    }

    #[test]
    fn modifiers_and_media() {
        let expect = [
            (42, 0x2A, false), (54, 0x36, false), (29, 0x1D, false), (97, 0x1D, true),
            (56, 0x38, false), (100, 0x38, true), (125, 0x5B, true), (126, 0x5C, true),
            (113, 0x20, true), (114, 0x2E, true), (115, 0x30, true), (164, 0x22, true),
            (163, 0x19, true), (165, 0x10, true), (166, 0x24, true), (99, 0x37, true),
        ];
        for (code, scan, extended) in expect {
            assert_eq!(evdev_to_win(code), Some(WinKey::Scan { scan, extended }), "key {code}");
        }
        assert_eq!(evdev_to_win(0x110), None); // buttons are not keys
    }

    #[test]
    fn normalize_hits_every_pixel() {
        for size in [1, 2, 800, 1366, 1920, 3840, 5120, 7680, 1080 + 1440] {
            for x in 0..size {
                let n = normalize_axis(x, size);
                assert!((0..=65535).contains(&n));
                let floor = (i64::from(n) * i64::from(size)) >> 16;
                let round = (i64::from(n) * i64::from(size) + 32768) >> 16;
                assert_eq!(floor, i64::from(x), "size {size} x {x} n {n}");
                assert_eq!(round, i64::from(x), "size {size} x {x} n {n} (rounding)");
            }
            assert_eq!(normalize_axis(0, size), 0);
        }
        assert_eq!(normalize_axis(1919, 1920), 65502);
        assert_eq!(normalize_axis(5000, 1920), normalize_axis(1919, 1920));
    }

    #[test]
    fn normalize_with_negative_origin() {
        let s = Space::new(trio(), 96);
        // Virtual desktop spans x -1280..4480 (5760 wide), y 0..1440.
        assert_eq!(s.normalize(-1280, 0), (0, 0));
        let (nx, ny) = s.normalize(0, 1439);
        assert_eq!((i64::from(nx) * 5760) >> 16, 1280);
        assert_eq!((i64::from(ny) * 1440) >> 16, 1439);
        assert_eq!(s.normalize(4479, 1439).0, normalize_axis(5759, 5760));
    }

    #[test]
    fn edges_only_on_outer_boundary() {
        let s = Space::new(trio(), 96);
        let all = Side::ALL;
        // Right edge of the 2560 monitor.
        assert_eq!(s.edge_hit(&all, 4479, 700), Some(Side::Right));
        assert_eq!(s.edge_hit(&all, 4478, 700), None);
        // Inner edge between primary and the right monitor: nothing.
        assert_eq!(s.edge_hit(&[Side::Right], 1919, 500), None);
        // Left edge of the left monitor, within its span only.
        assert_eq!(s.edge_hit(&all, -1280, 600), Some(Side::Left));
        // Top: the primary and right monitor are at y=0; the left one is not.
        assert_eq!(s.edge_hit(&all, 100, 0), Some(Side::Top));
        assert_eq!(s.edge_hit(&[Side::Top], -500, 200), None);
        // Bottom: the desktop ends at y=1440, so only the right monitor
        // counts; the bottoms of the primary (1080) and the left monitor
        // (1224) face gaps.
        assert_eq!(s.edge_hit(&[Side::Bottom], 3000, 1439), Some(Side::Bottom));
        assert_eq!(s.edge_hit(&[Side::Bottom], 500, 1079), None);
        assert_eq!(s.edge_hit(&[Side::Bottom], -500, 1223), None);
        // Unarmed sides never trigger.
        assert_eq!(s.edge_hit(&[Side::Left], 4479, 700), None);
        // Past the edge (unclipped hook positions) still counts.
        assert_eq!(s.edge_hit(&[Side::Right], 4600, 700), Some(Side::Right));
    }

    #[test]
    fn scaled_space_is_linear_and_round_trips() {
        // Laptop at 150% (2880x1800 physical) with a 1920x1080 monitor right of it.
        let s = Space::new(vec![Rect::new(0, 0, 2880, 1800), Rect::new(2880, 0, 1920, 1080)], 144);
        assert_eq!(s.scale(), 1.5);
        assert_eq!(s.logical().screens, vec![Rect::new(0, 0, 1920, 1200), Rect::new(1920, 0, 1280, 720)]);
        for (x, y) in [(0, 0), (2879, 1799), (2880, 0), (4799, 1079), (1234, 567)] {
            let p = s.to_logical(x, y);
            assert_eq!(s.to_phys(p), (x, y));
        }
        assert_eq!(s.delta_to_logical(3, -6), (2.0, -4.0));
        // Off-screen positions land on the nearest screen pixel, never in the gap.
        assert_eq!(s.to_phys(Pos::new(2500.0, 1000.0)), (3750, 1079));
        assert_eq!(s.to_phys(Pos::new(-50.0, -50.0)), (0, 0));
    }

    #[test]
    fn odd_scales_keep_monitors_adjacent() {
        let s = Space::new(vec![Rect::new(0, 0, 1366, 768), Rect::new(1366, 0, 1366, 768)], 120);
        let l = &s.logical().screens;
        assert_eq!(l[0].right(), l[1].x);
    }

    #[test]
    fn park_point_is_screen_centre() {
        let s = Space::new(trio(), 96);
        assert_eq!(s.park_point(4479, 10), (1920 + 1280, 720));
        assert_eq!(s.park_point(-1280, 600), (-640, 712));
        assert_eq!(s.park_point(99999, 99999), (3200, 720)); // largest screen
    }

    #[test]
    fn scroll_signs() {
        // Wheel forward (+120 on Windows) is scroll up: negative dy on the wire.
        assert_eq!(wheel_to_wire(120, false), (0, -120));
        assert_eq!(wheel_to_wire(-120, false), (0, 120));
        assert_eq!(wheel_to_wire(120, true), (120, 0));
        // And back: wire dy +120 (down) is wheel -120.
        assert_eq!(wire_to_wheel(0, 120), (0, -120));
        assert_eq!(wire_to_wheel(-120, 0), (-120, 0));
    }

    #[test]
    fn smooth_scroll_accumulates() {
        let mut s = SmoothScroll::default();
        // 48 px down is one notch down.
        assert_eq!(s.push(0.0, 48.0), (0, -120));
        // Sub-unit motion accumulates instead of being lost.
        let mut total = 0;
        for _ in 0..10 {
            total += s.push(0.0, 0.1).1;
        }
        assert_eq!(total, -2); // 1 px = 2.5 units
        s.stop();
        assert_eq!(s.push(12.0, 0.0), (30, 0));
    }
}
