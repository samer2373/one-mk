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

use one_mk_proto::{Desktop, Edge, Pos, Rect, Side, mediakeys};

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

    /// Which armed edge, if any, the physical pixel `(x, y)` is touching,
    /// with `(dx, dy)` the pointer's recent direction of movement.
    ///
    /// `edges` are in the reported space; each belongs to the screen whose
    /// reported rect has that edge line, and counts only along its span
    /// (half a reported pixel of slack for rounding). Windows keeps the
    /// cursor on the union of the monitors, so "touching" means sitting on
    /// the last pixel row or column; positions past the edge (if a hook
    /// reports them) count too, unless they are on another screen. In a
    /// corner, where two armed edges touch, the one the pointer is moving
    /// towards more strongly wins.
    pub(crate) fn edge_hit(&self, edges: &[Edge], x: i32, y: i32, dx: f64, dy: f64) -> Option<Side> {
        let on = |r: &Rect| (r.x..r.right()).contains(&x) && (r.y..r.bottom()).contains(&y);
        let along = |e: &Edge, v: i32| {
            let v = f64::from(v) / self.scale;
            v >= f64::from(e.lo) - 0.5 && v < f64::from(e.hi) + 0.5
        };
        let touching = |e: &Edge| {
            self.phys.screens.iter().zip(&self.logical.screens).any(|(r, l)| {
                let at = match e.side {
                    Side::Left => l.x == e.at && x <= r.x && (r.y..r.bottom()).contains(&y) && along(e, y),
                    Side::Right => l.right() == e.at && x >= r.right() - 1 && (r.y..r.bottom()).contains(&y) && along(e, y),
                    Side::Top => l.y == e.at && y <= r.y && (r.x..r.right()).contains(&x) && along(e, x),
                    Side::Bottom => l.bottom() == e.at && y >= r.bottom() - 1 && (r.x..r.right()).contains(&x) && along(e, x),
                };
                at && !self.phys.screens.iter().any(|s| s != r && on(s))
            })
        };
        let towards = |side: Side| match side {
            Side::Left => -dx,
            Side::Right => dx,
            Side::Top => -dy,
            Side::Bottom => dy,
        };
        let mut best: Option<Side> = None;
        for side in edges.iter().filter(|e| touching(e)).map(|e| e.side) {
            if best.is_none_or(|b| towards(side) > towards(b)) {
                best = Some(side);
            }
        }
        best
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

// -------------------------------------------------------------- crossing

/// The pointer's recent direction of movement: a decaying sum of deltas,
/// so that in a corner the edge it was heading for still wins after the
/// cursor got pinned there (the pinned axis then reports no movement).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Heading {
    dx: f64,
    dy: f64,
    last: Option<(i32, i32)>,
}

impl Heading {
    pub(crate) fn update(&mut self, x: i32, y: i32) -> (f64, f64) {
        if let Some((lx, ly)) = self.last {
            self.dx = self.dx * 0.5 + f64::from(x - lx);
            self.dy = self.dy * 0.5 + f64::from(y - ly);
        }
        self.last = Some((x, y));
        (self.dx, self.dy)
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Whether to activate at an edge: never while a mouse button is held (a
/// drag towards the edge must not carry the pointer, and the dragged file,
/// to another machine), and not again after a refused activation until the
/// pointer has left the edge, or the cursor would flicker against it.
#[derive(Debug)]
pub(crate) struct EdgeGate {
    armed: bool,
}

impl Default for EdgeGate {
    fn default() -> Self {
        Self { armed: true }
    }
}

impl EdgeGate {
    /// `hit` is [`Space::edge_hit`] for a local mouse move.
    pub(crate) fn check(&mut self, hit: Option<Side>, buttons_held: bool) -> Option<Side> {
        match hit {
            None => {
                self.armed = true;
                None
            }
            Some(_) if !self.armed || buttons_held => None,
            Some(side) => Some(side),
        }
    }

    /// The pointer was handed back at the edge (refused, or capture ended
    /// on its own): wait until it leaves the edge.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

/// `MSLLHOOKSTRUCT::flags` bits.
pub(crate) const LLMHF_INJECTED: u32 = 0x1;
pub(crate) const LLMHF_LOWER_IL_INJECTED: u32 = 0x2;

/// Mouse messages Windows synthesizes from pen and touch input carry this
/// signature in `dwExtraInfo` (`MI_WP_SIGNATURE`, masked by
/// `SIGNATURE_MASK`).
const MI_WP_SIGNATURE: usize = 0xFF51_5700;

/// Mouse input that is not from a mouse: injected by software, or made up
/// from pen or touch. It must neither start a crossing nor be forwarded.
pub(crate) fn is_synthetic_mouse(flags: u32, extra: usize) -> bool {
    flags & (LLMHF_INJECTED | LLMHF_LOWER_IL_INJECTED) != 0 || extra & !0xFF == MI_WP_SIGNATURE
}

/// What the capture guard timer should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Watch {
    /// The cursor is where we parked it.
    Fine,
    /// Something moved it; put it back.
    Rewarp,
    /// It keeps getting moved: our hooks were probably removed silently
    /// (Windows does that to hooks that are slow under load). Install them
    /// again.
    Reinstall,
    /// Still moving after reinstalling: give the pointer back for good.
    GiveUp,
}

/// Watches the parked cursor on each guard tick while captured.
#[derive(Debug, Default)]
pub(crate) struct HookWatchdog {
    strikes: u8,
    reinstalled: bool,
}

impl HookWatchdog {
    /// Consecutive ticks with the cursor off the park point before acting.
    const STRIKES: u8 = 3;

    pub(crate) fn tick(&mut self, moved_away: bool) -> Watch {
        if !moved_away {
            self.strikes = 0;
            return Watch::Fine;
        }
        self.strikes += 1;
        if self.strikes < Self::STRIKES {
            return Watch::Rewarp;
        }
        self.strikes = 0;
        if self.reinstalled {
            Watch::GiveUp
        } else {
            self.reinstalled = true;
            Watch::Reinstall
        }
    }

    /// A new activation starts with a clean slate.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

// --------------------------------------------------------------- held keys

/// A set of evdev codes below 0x200 (all keys and mouse buttons we map).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held([u64; 8]);

impl Held {
    pub(crate) fn set(&mut self, code: u32, down: bool) {
        if let Some(word) = self.0.get_mut(code as usize / 64) {
            let bit = 1u64 << (code % 64);
            if down { *word |= bit } else { *word &= !bit }
        }
    }

    pub(crate) fn has(&self, code: u32) -> bool {
        self.0.get(code as usize / 64).is_some_and(|w| w & (1u64 << (code % 64)) != 0)
    }

    pub(crate) fn clear(&mut self) {
        self.0 = [0; 8];
    }
}

/// Keys forwarded to a peer while captured. Windows repeats a held key by
/// sending more "down" events; those are dropped, and the receiver repeats
/// by itself. That includes Caps Lock, whose repeats would toggle it on a
/// peer over and over.
#[derive(Debug, Default)]
pub(crate) struct SenderKeys(Held);

impl SenderKeys {
    /// Whether to forward this key event.
    pub(crate) fn forward(&mut self, code: u32, down: bool) -> bool {
        if down && self.0.has(code) {
            return false;
        }
        self.0.set(code, down);
        true
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

pub(crate) const VK_LBUTTON: u16 = 0x01;
pub(crate) const VK_RBUTTON: u16 = 0x02;
pub(crate) const VK_MBUTTON: u16 = 0x04;
pub(crate) const VK_XBUTTON1: u16 = 0x05;
pub(crate) const VK_XBUTTON2: u16 = 0x06;

/// A virtual key reported held by `GetAsyncKeyState` to an evdev code.
/// `scan_ex` is `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX)`: the scan code
/// with 0xE0/0xE1 in the high byte for extended keys. `swapped` is
/// `SM_SWAPBUTTON`: `GetAsyncKeyState` reports physical buttons, the hooks
/// logical ones.
pub(crate) fn vk_to_evdev(vk: u16, scan_ex: u32, swapped: bool) -> Option<u32> {
    match vk {
        VK_LBUTTON => Some(if swapped { BTN_RIGHT } else { BTN_LEFT }),
        VK_RBUTTON => Some(if swapped { BTN_LEFT } else { BTN_RIGHT }),
        VK_MBUTTON => Some(BTN_MIDDLE),
        VK_XBUTTON1 => Some(BTN_SIDE),
        VK_XBUTTON2 => Some(BTN_EXTRA),
        // Generic Shift/Ctrl/Alt: the left/right specific keys are read too.
        0x10..=0x12 => None,
        _ => {
            let prefix = scan_ex >> 8;
            hook_key_to_evdev(u32::from(vk), scan_ex & 0xFF, prefix == 0xE0 || prefix == 0xE1)
        }
    }
}

/// Key repeat delay and interval from `SPI_GETKEYBOARDDELAY` (0..=3:
/// 250 ms to 1 s) and `SPI_GETKEYBOARDSPEED` (0..=31: about 2.5 to 30
/// repeats per second).
pub(crate) fn repeat_timing(delay: u32, speed: u32) -> (std::time::Duration, std::time::Duration) {
    let delay_ms = 250 * (u64::from(delay.min(3)) + 1);
    let hz = 2.5 + f64::from(speed.min(31)) * (27.5 / 31.0);
    (std::time::Duration::from_millis(delay_ms), std::time::Duration::from_secs_f64(1.0 / hz))
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
    // Media keys are typed by virtual key: that is what a media key is to
    // the shell, whatever scan code the keyboard layout maps it to.
    if let Some(vk) = mediakeys::to_win_vk(code) {
        let scan = mediakeys::from_win_vk(vk)
            .and_then(|c| KEY_TABLE.iter().find(|&&(_, _, k)| k == c))
            .map_or(0, |&(scan, _, _)| scan);
        return Some(WinKey::Vk { vk, scan, extended: true });
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
    // Media keys are told by virtual key: some keyboards and drivers give
    // them scan codes that are not the standard E0 ones.
    if let Some(code) = vk_media_to_evdev(vk as u16) {
        return Some(code);
    }
    if scan != 0 {
        return scan_to_evdev(scan as u16, extended);
    }
    None
}

fn vk_media_to_evdev(vk: u16) -> Option<u32> {
    mediakeys::from_win_vk(vk)
}

// ------------------------------------------------------------- fullscreen

/// `GWL_STYLE` bits of a window.
pub(crate) const WS_CAPTION: u32 = 0x00C0_0000;
pub(crate) const WS_THICKFRAME: u32 = 0x0004_0000;
pub(crate) const WS_MAXIMIZE: u32 = 0x0100_0000;

/// Window classes of the shell: the desktop (wallpaper and icons, which
/// covers the monitor) and the taskbars. Never fullscreen.
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// A rectangle as Win32 gives it: left, top, right, bottom (exclusive).
pub(crate) type Ltrb = [i32; 4];

/// Whether the foreground window is fullscreen: it covers its whole
/// monitor and has neither a caption nor the resizable frame of a
/// maximized window. `rect` is the window as drawn (DWM's extended frame
/// bounds; `GetWindowRect` reaches about 8 px past them on every side),
/// `monitor` the monitor's full rect (not its work area).
///
/// A maximized window covers the monitor too when the taskbar hides
/// itself or sits on another monitor, and without DWM's bounds its rect
/// overshoots the work area anyway, hence the style checks: games and
/// players go fullscreen as borderless popups, which a maximized window
/// (even one drawing its own title bar) is not.
pub(crate) fn is_fullscreen(class: &str, style: u32, rect: Ltrb, monitor: Ltrb) -> bool {
    let [l, t, r, b] = rect;
    let [ml, mt, mr, mb] = monitor;
    let covers = l <= ml && t <= mt && r >= mr && b >= mb;
    let captioned = style & WS_CAPTION == WS_CAPTION;
    let maximized = style & WS_MAXIMIZE != 0 && style & WS_THICKFRAME != 0;
    covers && !captioned && !maximized && !SHELL_CLASSES.contains(&class)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the daemon arms for peers without an offset: the outer edges.
    fn armed(s: &Space, sides: &[Side]) -> Vec<Edge> {
        let d = s.logical();
        sides.iter().flat_map(|&side| d.outer_edges(side).map(move |r| Edge::of(side, r))).collect()
    }

    #[test]
    fn segments_in_a_scaled_space() {
        // At 150%: a laptop (1920x1200 reported) and a monitor right of it
        // (1280x720 reported). Arm the laptop's right edge below the monitor.
        let s = Space::new(vec![Rect::new(0, 0, 2880, 1800), Rect::new(2880, 0, 1920, 1080)], 144);
        let e = [Edge { side: Side::Right, at: 1920, lo: 720, hi: 1200 }];
        assert_eq!(s.edge_hit(&e, 2879, 1500, 1.0, 0.0), Some(Side::Right));
        assert_eq!(s.edge_hit(&e, 2879, 1799, 1.0, 0.0), Some(Side::Right));
        // Beside the monitor the pointer just moves over to it.
        assert_eq!(s.edge_hit(&e, 2879, 500, 1.0, 0.0), None);
        // The monitor's own right edge is not armed.
        assert_eq!(s.edge_hit(&e, 4799, 500, 1.0, 0.0), None);
    }

    #[test]
    fn fullscreen_is_a_borderless_window_covering_its_monitor() {
        const POPUP: u32 = 0x8000_0000;
        const OVERLAPPED: u32 = WS_CAPTION | WS_THICKFRAME | 0x0008_0000 | 0x0002_0000 | 0x0001_0000;
        let mon = [1920, 0, 3840, 1080];
        // A borderless game or a video player, on the second monitor.
        assert!(is_fullscreen("UnityWndClass", POPUP, mon, mon));
        // Exclusive fullscreen may report a larger rect.
        assert!(is_fullscreen("SDL_app", POPUP, [1910, -10, 3850, 1090], mon));
        // Not the whole monitor.
        assert!(!is_fullscreen("SDL_app", POPUP, [1920, 0, 3840, 1040], mon));
        assert!(!is_fullscreen("SDL_app", POPUP, [0, 0, 1920, 1080], mon), "the other monitor");
        // Maximized with the taskbar hidden: DWM's bounds are the monitor,
        // GetWindowRect 8 px past it; neither is fullscreen.
        let maximized = OVERLAPPED | WS_MAXIMIZE;
        assert!(!is_fullscreen("Notepad", maximized, mon, mon));
        assert!(!is_fullscreen("Notepad", maximized, [1912, -8, 3848, 1088], mon));
        // Maximized with its own title bar (no caption style), as Electron
        // apps and browsers draw it.
        assert!(!is_fullscreen("Chrome_WidgetWin_1", (maximized & !WS_CAPTION) | POPUP, mon, mon));
        // A captioned window resized over the whole monitor.
        assert!(!is_fullscreen("Notepad", OVERLAPPED, [1912, -8, 3848, 1088], mon));
        // A browser's F11 fullscreen drops the caption and the frame.
        assert!(is_fullscreen("Chrome_WidgetWin_1", POPUP, mon, mon));
        // The desktop and the taskbars.
        for shell in ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
            assert!(!is_fullscreen(shell, POPUP, mon, mon), "{shell}");
        }
    }

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
            (99, 0x37, true),
        ];
        for (code, scan, extended) in expect {
            assert_eq!(evdev_to_win(code), Some(WinKey::Scan { scan, extended }), "key {code}");
        }
        // Media keys are typed by virtual key, with their usual scan code.
        let media = [
            (113, 0xAD, 0x20), (114, 0xAE, 0x2E), (115, 0xAF, 0x30), (164, 0xB3, 0x22),
            (163, 0xB0, 0x19), (165, 0xB1, 0x10), (166, 0xB2, 0x24),
        ];
        for (code, vk, scan) in media {
            assert_eq!(evdev_to_win(code), Some(WinKey::Vk { vk, scan, extended: true }), "key {code}");
        }
        assert_eq!(evdev_to_win(0x110), None); // buttons are not keys
    }

    /// Every media key the table gives Windows is captured from its virtual
    /// key, with any scan code or none, and typed back as the same key.
    #[test]
    fn media_keys_match_the_canonical_table() {
        for k in mediakeys::MEDIA_KEYS {
            let Some(vk) = k.win_vk else {
                // No Windows key: only a stand-in (or nothing) is typed.
                if let Some(WinKey::Vk { vk, .. }) = evdev_to_win(k.evdev) {
                    assert_eq!(mediakeys::to_win_vk(k.evdev), Some(vk));
                }
                continue;
            };
            let Some(WinKey::Vk { vk: sent, scan, extended }) = evdev_to_win(k.evdev) else {
                panic!("{} is not typed by virtual key", k.name)
            };
            assert_eq!((sent, extended), (vk, true), "{}", k.name);
            for s in [0, u32::from(scan), 0x123] {
                let ext = hook_key_to_evdev(u32::from(vk), s, true);
                assert_eq!(ext, Some(k.evdev), "{} scan {s:#x}", k.name);
            }
            assert_eq!(vk_to_evdev(vk, 0xE000 | u32::from(scan), false), Some(k.evdev), "{}", k.name);
        }
        // What a Mac or Linux keyboard has beyond Windows.
        assert_eq!(evdev_to_win(208), Some(WinKey::Vk { vk: 0xB0, scan: 0x19, extended: true }));
        assert_eq!(evdev_to_win(225), None); // brightness: no Windows key
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
        let all = armed(&s, &Side::ALL);
        // Right edge of the 2560 monitor.
        assert_eq!(s.edge_hit(&all, 4479, 700, 0.0, 0.0), Some(Side::Right));
        assert_eq!(s.edge_hit(&all, 4478, 700, 0.0, 0.0), None);
        // Inner edge between primary and the right monitor: nothing.
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Right]), 1919, 500, 0.0, 0.0), None);
        // Left edge of the left monitor, within its span only.
        assert_eq!(s.edge_hit(&all, -1280, 600, 0.0, 0.0), Some(Side::Left));
        // Top: the primary and right monitor are at y=0; the left one is not.
        assert_eq!(s.edge_hit(&all, 100, 0, 0.0, 0.0), Some(Side::Top));
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Top]), -500, 200, 0.0, 0.0), None);
        // Bottom: the desktop ends at y=1440, so only the right monitor
        // counts; the bottoms of the primary (1080) and the left monitor
        // (1224) face gaps.
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Bottom]), 3000, 1439, 0.0, 0.0), Some(Side::Bottom));
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Bottom]), 500, 1079, 0.0, 0.0), None);
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Bottom]), -500, 1223, 0.0, 0.0), None);
        // Unarmed sides never trigger.
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Left]), 4479, 700, 0.0, 0.0), None);
        // Past the edge (unclipped hook positions) still counts.
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Right]), 4600, 700, 0.0, 0.0), Some(Side::Right));
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

    #[test]
    fn corner_picks_side_by_direction() {
        let s = Space::new(trio(), 96);
        let both = armed(&s, &[Side::Top, Side::Right]);
        // Top-right corner of the right monitor.
        assert_eq!(s.edge_hit(&both, 4479, 0, 5.0, -1.0), Some(Side::Right));
        assert_eq!(s.edge_hit(&both, 4479, 0, 1.0, -5.0), Some(Side::Top));
        // Order of the armed sides doesn't matter.
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Right, Side::Top]), 4479, 0, 1.0, -5.0), Some(Side::Top));
        // Only one armed: that one, whatever the direction.
        assert_eq!(s.edge_hit(&armed(&s, &[Side::Right]), 4479, 0, 0.0, -9.0), Some(Side::Right));
    }

    #[test]
    fn heading_survives_a_pinned_axis() {
        let s = Space::new(trio(), 96);
        let both = armed(&s, &[Side::Top, Side::Right]);
        // Moving up along the right edge (x pinned) into the corner: Top.
        let mut h = Heading::default();
        for y in [40, 30, 20, 10, 0] {
            h.update(4479, y);
        }
        let (dx, dy) = h.update(4479, 0); // pinned in the corner
        assert_eq!(s.edge_hit(&both, 4479, 0, dx, dy), Some(Side::Top));
        // Moving right along the top edge into the corner: Right.
        h.reset();
        for x in [4440, 4450, 4460, 4470, 4479] {
            h.update(x, 0);
        }
        let (dx, dy) = h.update(4479, 0);
        assert_eq!(s.edge_hit(&both, 4479, 0, dx, dy), Some(Side::Right));
    }

    #[test]
    fn no_crossing_while_dragging() {
        let mut g = EdgeGate::default();
        assert_eq!(g.check(Some(Side::Right), true), None);
        // Dropped the file at the edge, then pushed again: now it crosses.
        assert_eq!(g.check(Some(Side::Right), false), Some(Side::Right));
    }

    #[test]
    fn refused_crossing_waits_for_the_pointer_to_leave() {
        let mut g = EdgeGate::default();
        assert_eq!(g.check(Some(Side::Left), false), Some(Side::Left));
        // No peer took it: the pointer comes straight back.
        g.disarm();
        assert_eq!(g.check(Some(Side::Left), false), None);
        assert_eq!(g.check(Some(Side::Left), false), None);
        assert_eq!(g.check(None, false), None);
        assert_eq!(g.check(Some(Side::Left), false), Some(Side::Left));
    }

    #[test]
    fn pen_touch_and_injected_mouse_input() {
        assert!(!is_synthetic_mouse(0, 0));
        assert!(!is_synthetic_mouse(0, 0x316d_6b21));
        assert!(is_synthetic_mouse(LLMHF_INJECTED, 0));
        assert!(is_synthetic_mouse(LLMHF_LOWER_IL_INJECTED, 0));
        // Pen (0xFF515700 | flags) and touch (bit 7 set) signatures.
        assert!(is_synthetic_mouse(0, 0xFF51_5700));
        assert!(is_synthetic_mouse(0, 0xFF51_5780));
        assert!(!is_synthetic_mouse(0, 0xFF51_5600));
    }

    #[test]
    fn watchdog_reinstalls_then_gives_up() {
        let mut w = HookWatchdog::default();
        assert_eq!(w.tick(false), Watch::Fine);
        assert_eq!(w.tick(true), Watch::Rewarp);
        assert_eq!(w.tick(true), Watch::Rewarp);
        // A good tick in between starts the count over.
        assert_eq!(w.tick(false), Watch::Fine);
        assert_eq!(w.tick(true), Watch::Rewarp);
        assert_eq!(w.tick(true), Watch::Rewarp);
        assert_eq!(w.tick(true), Watch::Reinstall);
        assert_eq!(w.tick(true), Watch::Rewarp);
        assert_eq!(w.tick(true), Watch::Rewarp);
        assert_eq!(w.tick(true), Watch::GiveUp);
        w.reset();
        for _ in 0..2 {
            w.tick(true);
        }
        assert_eq!(w.tick(true), Watch::Reinstall);
    }

    #[test]
    fn sender_drops_key_repeats_including_caps_lock() {
        const CAPS: u32 = 58;
        let mut k = SenderKeys::default();
        assert!(k.forward(CAPS, true));
        // Typematic repeats of Caps Lock (and every other key) are dropped.
        assert!(!k.forward(CAPS, true));
        assert!(!k.forward(CAPS, true));
        assert!(k.forward(CAPS, false));
        assert!(k.forward(CAPS, true));
        assert!(k.forward(30, true));
        assert!(!k.forward(30, true));
        k.clear();
        assert!(k.forward(30, true));
    }

    #[test]
    fn held_set() {
        let mut h = Held::default();
        h.set(30, true);
        h.set(BTN_SIDE, true);
        assert!(h.has(30) && h.has(BTN_SIDE) && !h.has(31));
        h.set(30, false);
        assert!(!h.has(30));
        h.set(0x1FF, true);
        h.set(0x200, true); // out of range: ignored
        assert!(h.has(0x1FF) && !h.has(0x200));
        h.clear();
        assert_eq!(h, Held::default());
    }

    #[test]
    fn async_key_state_to_evdev() {
        assert_eq!(vk_to_evdev(0xA0, 0x2A, false), Some(42)); // left Shift
        assert_eq!(vk_to_evdev(0xA3, 0xE01D, false), Some(97)); // right Ctrl
        assert_eq!(vk_to_evdev(0x5B, 0xE05B, false), Some(125)); // left Windows
        assert_eq!(vk_to_evdev(0x2D, 0xE052, false), Some(110)); // Insert
        assert_eq!(vk_to_evdev(0x60, 0x52, false), Some(82)); // keypad 0
        assert_eq!(vk_to_evdev(VK_NUMLOCK, 0xE045, false), Some(69));
        assert_eq!(vk_to_evdev(0x10, 0x2A, false), None); // generic Shift
        assert_eq!(vk_to_evdev(VK_LBUTTON, 0, false), Some(BTN_LEFT));
        assert_eq!(vk_to_evdev(VK_LBUTTON, 0, true), Some(BTN_RIGHT));
        assert_eq!(vk_to_evdev(VK_XBUTTON2, 0, false), Some(BTN_EXTRA));
    }

    #[test]
    fn keyboard_repeat_settings() {
        let (d, i) = repeat_timing(1, 31);
        assert_eq!(d.as_millis(), 500);
        assert_eq!(i.as_millis(), 33);
        let (d, i) = repeat_timing(0, 0);
        assert_eq!(d.as_millis(), 250);
        assert_eq!(i.as_millis(), 400);
        assert_eq!(repeat_timing(9, 99), repeat_timing(3, 31));
    }
}
