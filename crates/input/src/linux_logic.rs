//! Platform-independent half of the X11 and wlroots backends: which Linux
//! backend to use, button and key code tables, scroll conversion, keys
//! held down, edge geometry and the edge "push" that starts capture.
//!
//! Nothing here talks to a display server, so it is compiled and
//! unit-tested on every OS.

// On other OSes only the tests use this module.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::{Event, Inject};
use one_mk_proto::{Desktop, Edge, Pos, Rect, Side};

pub(crate) const BTN_LEFT: u32 = 0x110;
pub(crate) const BTN_RIGHT: u32 = 0x111;
pub(crate) const BTN_MIDDLE: u32 = 0x112;
pub(crate) const BTN_SIDE: u32 = 0x113;
pub(crate) const BTN_EXTRA: u32 = 0x114;

/// One wheel notch in discrete scroll units (120ths of a notch, as libei,
/// evdev's hi-res wheel and Windows count them).
pub(crate) const NOTCH: i32 = 120;
/// Smooth (pixel) scrolling per notch, as the Windows backend counts it.
pub(crate) const PIXELS_PER_NOTCH: f64 = 48.0;

// ---------------------------------------------------------------- choice

/// The Linux input backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxBackend {
    /// XDG desktop portals and libei (GNOME, KDE).
    Portal,
    /// wlroots protocols (Sway, Hyprland, labwc, river, Wayfire).
    Wlroots,
    /// X11: XInput2, XFixes barriers and XTest.
    X11,
}

impl LinuxBackend {
    pub fn name(self) -> &'static str {
        match self {
            LinuxBackend::Portal => "portal",
            LinuxBackend::Wlroots => "wlroots",
            LinuxBackend::X11 => "x11",
        }
    }
}

/// What the session offers, as far as the backend choice cares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// Both the RemoteDesktop and the InputCapture portal answered and
    /// can do keyboard and pointer. `None`: not asked (or no answer).
    pub portals: Option<bool>,
    /// `WAYLAND_DISPLAY` is set.
    pub wayland: bool,
    /// The Wayland compositor's globals, if it could be reached.
    pub wayland_globals: Option<Vec<String>>,
    /// `DISPLAY` is set.
    pub x11: bool,
}

/// The wlroots protocols injection and capture need.
pub(crate) const WLR_NEEDED: [&str; 4] = [
    "zwlr_virtual_pointer_manager_v1",
    "zwp_virtual_keyboard_manager_v1",
    "zwlr_layer_shell_v1",
    "zwp_relative_pointer_manager_v1",
];

/// The needed wlroots protocols `globals` lacks.
pub(crate) fn wlr_missing(globals: &[String]) -> Vec<&'static str> {
    WLR_NEEDED.into_iter().filter(|n| !globals.iter().any(|g| g == n)).collect()
}

/// Picks the backend for `native`. On Wayland: the portals where the
/// desktop has them (they are its own, sanctioned way), else the wlroots
/// protocols if the compositor has them. X11 only outside Wayland
/// (through XWayland it sees neither the pointer over Wayland windows nor
/// can it type into them), and there always: GNOME's and KDE's portals
/// answer on X11 too, but their input capture only works on Wayland.
/// Returns why, for `doctor`.
pub fn choose(f: &SessionFacts) -> (LinuxBackend, String) {
    if !f.wayland && f.x11 {
        return (LinuxBackend::X11, "an X11 session (DISPLAY is set, WAYLAND_DISPLAY is not)".into());
    }
    if f.portals == Some(true) {
        return (LinuxBackend::Portal, "the desktop offers the RemoteDesktop and InputCapture portals".into());
    }
    if f.wayland {
        return match &f.wayland_globals {
            Some(g) if wlr_missing(g).is_empty() => (
                LinuxBackend::Wlroots,
                "no input portals, but the Wayland compositor offers the wlroots virtual input and layer-shell protocols"
                    .into(),
            ),
            Some(g) => (
                LinuxBackend::Portal,
                format!(
                    "Wayland without input portals, and the compositor lacks {} (needed for the wlroots backend)",
                    wlr_missing(g).join(", ")
                ),
            ),
            None => (LinuxBackend::Portal, "Wayland, but the compositor could not be asked what it offers".into()),
        };
    }
    (LinuxBackend::Portal, "no Wayland or X11 display found in the environment".into())
}

// ------------------------------------------------------- buttons and keys

/// What an X button number stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XButton {
    Button(u32),
    /// Buttons 4-7 are wheel notches: `(dx, dy)` in 120ths.
    Scroll(i32, i32),
}

pub(crate) fn x_button(b: u32) -> Option<XButton> {
    Some(match b {
        1 => XButton::Button(BTN_LEFT),
        2 => XButton::Button(BTN_MIDDLE),
        3 => XButton::Button(BTN_RIGHT),
        4 => XButton::Scroll(0, -NOTCH),
        5 => XButton::Scroll(0, NOTCH),
        6 => XButton::Scroll(-NOTCH, 0),
        7 => XButton::Scroll(NOTCH, 0),
        8 => XButton::Button(BTN_SIDE),
        9 => XButton::Button(BTN_EXTRA),
        _ => return None,
    })
}

pub(crate) fn evdev_to_x_button(code: u32) -> Option<u8> {
    Some(match code {
        BTN_LEFT => 1,
        BTN_MIDDLE => 2,
        BTN_RIGHT => 3,
        BTN_SIDE => 8,
        BTN_EXTRA => 9,
        _ => return None,
    })
}

/// X keycodes are evdev codes plus 8 (the evdev and libinput drivers and
/// XWayland all number them so), and fit in a byte.
pub(crate) fn evdev_to_xkeycode(code: u32) -> Option<u8> {
    u8::try_from(code.checked_add(8)?).ok()
}

pub(crate) fn xkeycode_to_evdev(keycode: u32) -> Option<u32> {
    keycode.checked_sub(8)
}

/// Keys and buttons injected and still held down, to let go of when
/// injection ends for any reason the daemon did not handle (it locks, it
/// exits, the display goes away). XTest keeps a key pressed after the
/// client that pressed it disconnects, so this is the only thing between
/// a crash of the daemon's peer and a stuck key.
#[derive(Debug, Default)]
pub(crate) struct Held {
    keys: Vec<u32>,
    buttons: Vec<u32>,
}

impl Held {
    pub(crate) fn update(&mut self, m: &Inject) {
        let (list, code, down) = match *m {
            Inject::Key { code, down } => (&mut self.keys, code, down),
            Inject::Button { button, down } => (&mut self.buttons, button, down),
            _ => return,
        };
        list.retain(|&c| c != code);
        if down {
            list.push(code);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }

    /// Releases for everything held, last pressed first; forgets them.
    pub(crate) fn release(&mut self) -> Vec<Inject> {
        let buttons = self.buttons.drain(..).rev().map(|button| Inject::Button { button, down: false });
        let keys = self.keys.drain(..).rev().map(|code| Inject::Key { code, down: false });
        buttons.chain(keys).collect()
    }
}

/// Keys and buttons pressed while capturing: a second press of a held key
/// is an auto-repeat (senders drop those), a release of something never
/// pressed was held before capture started (its press went to this
/// machine, so its release must not go to the peer either).
#[derive(Debug, Default)]
pub(crate) struct Pressed(Vec<u32>);

impl Pressed {
    /// Whether to forward this press or release.
    pub(crate) fn update(&mut self, code: u32, down: bool) -> bool {
        let held = self.0.contains(&code);
        if down && !held {
            self.0.push(code);
        } else if !down && held {
            self.0.retain(|&c| c != code);
        }
        down != held
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

// ----------------------------------------------------------------- scroll

/// Turns scrolling into wheel notches for injectors that only have whole
/// notches (XTest's buttons 4-7), carrying the remainders over.
#[derive(Debug, Default)]
pub(crate) struct Notches {
    /// In 120ths.
    x: f64,
    y: f64,
}

impl Notches {
    /// Discrete scrolling in 120ths; returns whole notches `(dx, dy)`.
    pub(crate) fn discrete(&mut self, dx: i32, dy: i32) -> (i32, i32) {
        self.add(f64::from(dx), f64::from(dy))
    }

    /// Smooth scrolling in pixels.
    pub(crate) fn smooth(&mut self, dx: f64, dy: f64) -> (i32, i32) {
        let k = f64::from(NOTCH) / PIXELS_PER_NOTCH;
        self.add(dx * k, dy * k)
    }

    fn add(&mut self, dx: f64, dy: f64) -> (i32, i32) {
        self.x += dx;
        self.y += dy;
        let n = f64::from(NOTCH);
        let (nx, ny) = ((self.x / n).trunc(), (self.y / n).trunc());
        self.x -= nx * n;
        self.y -= ny * n;
        (nx as i32, ny as i32)
    }

    pub(crate) fn stop(&mut self) {
        *self = Self::default();
    }
}

/// The X buttons for `(dx, dy)` notches: 4 up, 5 down, 6 left, 7 right,
/// each to be clicked once per entry.
pub(crate) fn notch_buttons(dx: i32, dy: i32) -> impl Iterator<Item = u8> {
    let v = std::iter::repeat_n(if dy < 0 { 4 } else { 5 }, dy.unsigned_abs().min(64) as usize);
    let h = std::iter::repeat_n(if dx < 0 { 6 } else { 7 }, dx.unsigned_abs().min(64) as usize);
    v.chain(h)
}

/// Scroll valuator motion (in the device's `increment` per notch) to
/// 120ths, carrying the rounding remainder.
#[derive(Debug, Default)]
pub(crate) struct ScrollAxis {
    rest: f64,
}

impl ScrollAxis {
    pub(crate) fn push(&mut self, delta: f64, increment: f64) -> i32 {
        if increment == 0.0 {
            return 0;
        }
        let v = delta / increment * f64::from(NOTCH) + self.rest;
        let whole = v.trunc();
        self.rest = v - whole;
        whole as i32
    }
}

/// Converts a frame of `wl_pointer` axis events into events for the peer:
/// wheels as discrete 120ths (from `axis_value120`, or `axis_discrete` on
/// older compositors), everything else as smooth pixels.
#[derive(Debug, Default)]
pub(crate) struct AxisFrame {
    /// `[horizontal, vertical]`.
    value: [f64; 2],
    v120: [i32; 2],
    stop: bool,
    wheel: bool,
}

impl AxisFrame {
    /// `axis`: 0 vertical, 1 horizontal (`wl_pointer.axis`).
    pub(crate) fn value(&mut self, axis: u32, v: f64) {
        self.value[axis_index(axis)] += v;
    }
    pub(crate) fn value120(&mut self, axis: u32, v: i32) {
        self.v120[axis_index(axis)] += v;
        self.wheel = true;
    }
    pub(crate) fn discrete(&mut self, axis: u32, steps: i32) {
        self.value120(axis, steps * NOTCH);
    }
    pub(crate) fn wheel_source(&mut self) {
        self.wheel = true;
    }
    pub(crate) fn stop(&mut self) {
        self.stop = true;
    }

    /// The events for this frame; starts the next one.
    pub(crate) fn finish(&mut self) -> Vec<Event> {
        let f = std::mem::take(self);
        let mut out = Vec::new();
        let [h, v] = f.v120;
        if h != 0 || v != 0 {
            out.push(Event::ScrollDiscrete { dx: h, dy: v });
        } else if !f.wheel && (f.value[0] != 0.0 || f.value[1] != 0.0) {
            out.push(Event::Scroll { dx: f.value[0], dy: f.value[1] });
        }
        if f.stop {
            out.push(Event::ScrollStop);
        }
        out
    }
}

fn axis_index(axis: u32) -> usize {
    // wl_pointer.axis: 0 vertical_scroll, 1 horizontal_scroll.
    if axis == 1 { 0 } else { 1 }
}

// --------------------------------------------------------------- geometry

/// Where the armed `edges` lie on `desktop`'s monitors: for each segment
/// and each monitor whose `side` edge is on the segment's line, that
/// monitor and the one-pixel strip just inside it along the part of the
/// segment on it. Inner exposed edges (a monitor's edge with no monitor
/// beyond it, set back from the bounding box) work like outer ones; a
/// segment on no monitor's edge gets nothing.
pub(crate) fn edge_strips(edges: &[Edge], desktop: &Desktop) -> Vec<(Side, Rect, Rect)> {
    let mut out = Vec::new();
    for e in edges {
        for r in &desktop.screens {
            let whole = Edge::of(e.side, r);
            let (lo, hi) = (e.lo.max(whole.lo), e.hi.min(whole.hi));
            if whole.at != e.at || lo >= hi {
                continue;
            }
            let mut strip = edge_strip(e.side, r);
            if e.side.along_x() {
                (strip.x, strip.w) = (lo, hi - lo);
            } else {
                (strip.y, strip.h) = (lo, hi - lo);
            }
            out.push((e.side, *r, strip));
        }
    }
    out
}

/// Whether the pointer at `p`, on the one-pixel strip just inside the
/// `side` edge, is on an armed segment of that side.
pub(crate) fn armed_at(edges: &[Edge], side: Side, p: Pos) -> bool {
    let (across, along) = if side.along_x() { (p.y, p.x) } else { (p.x, p.y) };
    edges.iter().any(|e| {
        let line = f64::from(e.at);
        e.side == side && along >= f64::from(e.lo) && along < f64::from(e.hi) && (across - line).abs() <= 1.0
    })
}

/// The middle pixel of `e`, on the monitor just inside it: where the
/// pointer goes for a switch (`CaptureCmd::Activate`) to that side.
pub(crate) fn segment_point(e: &Edge) -> Pos {
    let mid = f64::from(e.lo + e.hi - 1) / 2.0;
    match e.side {
        Side::Left => Pos::new(f64::from(e.at), mid),
        Side::Right => Pos::new(f64::from(e.at - 1), mid),
        Side::Top => Pos::new(mid, f64::from(e.at)),
        Side::Bottom => Pos::new(mid, f64::from(e.at - 1)),
    }
}

/// A pointer barrier on the `side` edge of monitor `r`, as XFixes wants it
/// `(x1, y1, x2, y2)`: XFixes blocks motion across the line between pixel
/// `x - 1` and `x`, so right and bottom barriers lie one past the last
/// pixel. Ends are inclusive.
pub(crate) fn barrier_line(side: Side, r: &Rect) -> (i32, i32, i32, i32) {
    let (x, y, right, bottom) = (r.x, r.y, r.right(), r.bottom());
    match side {
        Side::Left => (x, y, x, bottom - 1),
        Side::Right => (right, y, right, bottom - 1),
        Side::Top => (x, y, right - 1, y),
        Side::Bottom => (x, bottom, right - 1, bottom),
    }
}

/// XFixes barrier directions: the ones the pointer may still cross.
pub(crate) fn barrier_open_directions(side: Side) -> u32 {
    const POS_X: u32 = 1;
    const POS_Y: u32 = 2;
    const NEG_X: u32 = 4;
    const NEG_Y: u32 = 8;
    let all = POS_X | POS_Y | NEG_X | NEG_Y;
    all & !match side {
        Side::Left => NEG_X,
        Side::Right => POS_X,
        Side::Top => NEG_Y,
        Side::Bottom => POS_Y,
    }
}

/// The one-pixel strip along the `side` edge of monitor `r`.
pub(crate) fn edge_strip(side: Side, r: &Rect) -> Rect {
    match side {
        Side::Left => Rect::new(r.x, r.y, 1, r.h),
        Side::Right => Rect::new(r.right() - 1, r.y, 1, r.h),
        Side::Top => Rect::new(r.x, r.y, r.w, 1),
        Side::Bottom => Rect::new(r.x, r.bottom() - 1, r.w, 1),
    }
}

/// Whether motion `(dx, dy)` pushes out through the `side` edge.
pub(crate) fn pushes_out(side: Side, dx: f64, dy: f64) -> bool {
    match side {
        Side::Left => dx < 0.0,
        Side::Right => dx > 0.0,
        Side::Top => dy < 0.0,
        Side::Bottom => dy > 0.0,
    }
}

/// The edge surface the pointer is on (wlroots): capture starts when it
/// pushes on outwards, as a pointer barrier would. Just being on the edge
/// pixel is not enough, or a pointer the daemon puts back there would go
/// straight across again.
#[derive(Debug, Default)]
pub(crate) struct EdgePush {
    at: Option<(Side, Pos)>,
}

impl EdgePush {
    pub(crate) fn enter(&mut self, side: Side, pos: Pos) {
        self.at = Some((side, pos));
    }
    pub(crate) fn moved(&mut self, pos: Pos) {
        if let Some((_, p)) = self.at.as_mut() {
            *p = pos;
        }
    }
    pub(crate) fn leave(&mut self) {
        self.at = None;
    }
    /// Relative motion: the side and position to capture at, if this
    /// pushes out of the edge the pointer is on.
    pub(crate) fn push(&self, dx: f64, dy: f64) -> Option<(Side, Pos)> {
        self.at.filter(|&(side, _)| pushes_out(side, dx, dy))
    }
}

/// An absolute position to a virtual pointer's `motion_absolute`
/// arguments `(x, y, x_extent, y_extent)`: wlroots maps it onto the
/// bounding box of all outputs. Scaled up 8 times to keep sub-pixel
/// precision.
pub(crate) fn absolute_motion(desktop: &Desktop, p: Pos) -> (u32, u32, u32, u32) {
    const K: f64 = 8.0;
    let b = desktop.bounds();
    let p = desktop.clamp(p);
    let x = ((p.x - f64::from(b.x)) * K).round().max(0.0) as u32;
    let y = ((p.y - f64::from(b.y)) * K).round().max(0.0) as u32;
    let (w, h) = ((f64::from(b.w) * K) as u32, (f64::from(b.h) * K) as u32);
    (x.min(w.saturating_sub(1)), y.min(h.saturating_sub(1)), w.max(1), h.max(1))
}

/// A Wayland output's place in the layout, in logical pixels: from
/// xdg-output if the compositor has it, else from `wl_output` (position,
/// current mode, scale and transform).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct OutputGeometry {
    pub(crate) position: Option<(i32, i32)>,
    /// Current mode in physical pixels.
    pub(crate) mode: Option<(i32, i32)>,
    pub(crate) scale: i32,
    /// `wl_output.transform`: odd values turn it by 90 or 270 degrees.
    pub(crate) transform: u32,
    pub(crate) logical_position: Option<(i32, i32)>,
    pub(crate) logical_size: Option<(i32, i32)>,
}

impl OutputGeometry {
    pub(crate) fn rect(&self) -> Option<Rect> {
        if let (Some((x, y)), Some((w, h))) = (self.logical_position, self.logical_size) {
            return (w > 0 && h > 0).then(|| Rect::new(x, y, w, h));
        }
        let (x, y) = self.position?;
        let (mut w, mut h) = self.mode?;
        if self.transform % 2 == 1 {
            std::mem::swap(&mut w, &mut h);
        }
        let s = self.scale.max(1);
        let (w, h) = (w / s, h / s);
        (w > 0 && h > 0).then(|| Rect::new(x, y, w, h))
    }
}

/// The XKB modifier state of a virtual keyboard, which wlroots does not
/// work out from its keys: the client sends it. What the keys mean in a
/// standard keymap, for when libxkbcommon can't say what they mean in the
/// actual one (see `crate::xkb`). Bits as in every standard keymap: Shift
/// 0, Lock 1, Control 2, Mod1 (Alt) 3, Mod2 (Num Lock) 4, Mod4 (Super) 6,
/// Mod5 (AltGr) 7.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct XkbMods {
    held: Vec<u32>,
    pub(crate) locked: u32,
}

impl XkbMods {
    fn bit(code: u32) -> Option<u32> {
        Some(match code {
            42 | 54 => 1,        // Shift
            29 | 97 => 1 << 2,   // Control
            56 => 1 << 3,        // Alt
            125 | 126 => 1 << 6, // Super
            100 => 1 << 7,       // AltGr
            _ => return None,
        })
    }

    /// Records a key; returns true if the modifiers changed.
    pub(crate) fn key(&mut self, code: u32, down: bool) -> bool {
        let lock = match code {
            58 => Some(1 << 1), // Caps Lock
            69 => Some(1 << 4), // Num Lock
            _ => None,
        };
        if let Some(bit) = lock {
            if down && !self.held.contains(&code) {
                self.locked ^= bit;
                self.held.push(code);
                return true;
            }
            if !down {
                self.held.retain(|&c| c != code);
            }
            return false;
        }
        if Self::bit(code).is_none() {
            return false;
        }
        let before = self.depressed();
        self.held.retain(|&c| c != code);
        if down {
            self.held.push(code);
        }
        before != self.depressed()
    }

    pub(crate) fn depressed(&self) -> u32 {
        self.held.iter().filter_map(|&c| Self::bit(c)).fold(0, |a, b| a | b)
    }
}

/// The numbers of the bits set in an X key or button state bitmap (bit
/// `n` stands for key code or button `n`).
pub(crate) fn set_bits(bitmap: &[u8]) -> Vec<u32> {
    let bits = bitmap.iter().enumerate().flat_map(|(i, b)| (0..8).filter(move |j| b & (1 << j) != 0).map(move |j| i * 8 + j));
    bits.map(|n| n as u32).collect()
}

/// A 32.32 fixed-point number (XInput2) to f64.
pub(crate) fn fp3232(integral: i32, frac: u32) -> f64 {
    f64::from(integral) + f64::from(frac) / 4_294_967_296.0
}

/// A 16.16 fixed-point number (XInput2) to f64.
pub(crate) fn fp1616(v: i32) -> f64 {
    f64::from(v) / 65536.0
}

/// An absolute valuator's motion (a tablet, a VM's absolute pointer) to
/// pixels: the device's range spans the whole `extent`.
pub(crate) fn absolute_delta(from: f64, to: f64, min: f64, max: f64, extent: i32) -> f64 {
    if max <= min {
        return 0.0;
    }
    (to - from) / (max - min) * f64::from(extent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(portals: Option<bool>, wayland: Option<&[&str]>, x11: bool) -> SessionFacts {
        SessionFacts {
            portals,
            wayland: wayland.is_some(),
            wayland_globals: wayland.map(|g| g.iter().map(|s| s.to_string()).collect()),
            x11,
        }
    }

    #[test]
    fn backend_choice() {
        let wlr: &[&str] = &[
            "wl_compositor",
            "zwlr_layer_shell_v1",
            "zwlr_virtual_pointer_manager_v1",
            "zwp_virtual_keyboard_manager_v1",
            "zwp_relative_pointer_manager_v1",
        ];
        let gnome: &[&str] = &["wl_compositor", "zwp_relative_pointer_manager_v1"];
        // On Wayland the portals win wherever they exist, even on wlroots.
        assert_eq!(choose(&facts(Some(true), Some(wlr), true)).0, LinuxBackend::Portal);
        assert_eq!(choose(&facts(Some(true), Some(gnome), true)).0, LinuxBackend::Portal);
        // An X11 session is always X11: the portals' input capture is Wayland-only.
        assert_eq!(choose(&facts(Some(true), None, true)).0, LinuxBackend::X11);
        assert_eq!(choose(&facts(Some(false), Some(wlr), true)).0, LinuxBackend::Wlroots);
        assert_eq!(choose(&facts(None, Some(wlr), false)).0, LinuxBackend::Wlroots);
        // Wayland without the protocols: never X11 through XWayland.
        let (b, why) = choose(&facts(Some(false), Some(gnome), true));
        assert_eq!(b, LinuxBackend::Portal);
        assert!(why.contains("zwlr_layer_shell_v1") && why.contains("zwlr_virtual_pointer_manager_v1"), "{why}");
        assert_eq!(choose(&facts(Some(false), None, true)).0, LinuxBackend::X11);
        assert_eq!(choose(&facts(None, None, true)).0, LinuxBackend::X11);
        assert_eq!(choose(&facts(None, None, false)).0, LinuxBackend::Portal);
        let mut unreachable = facts(Some(false), None, true);
        unreachable.wayland = true;
        assert_eq!(choose(&unreachable).0, LinuxBackend::Portal);
    }

    #[test]
    fn x_buttons_round_trip() {
        for b in [1, 2, 3, 8, 9] {
            let Some(XButton::Button(code)) = x_button(b) else { panic!("{b}") };
            assert_eq!(evdev_to_x_button(code), Some(b as u8));
        }
        assert_eq!(x_button(1), Some(XButton::Button(BTN_LEFT)));
        assert_eq!(x_button(3), Some(XButton::Button(BTN_RIGHT)));
        assert_eq!(x_button(4), Some(XButton::Scroll(0, -120)));
        assert_eq!(x_button(7), Some(XButton::Scroll(120, 0)));
        assert_eq!(x_button(12), None);
        assert_eq!(evdev_to_x_button(0x118), None);
    }

    #[test]
    fn keycodes_are_evdev_plus_8() {
        assert_eq!(evdev_to_xkeycode(30), Some(38)); // KEY_A
        assert_eq!(evdev_to_xkeycode(247), Some(255));
        assert_eq!(evdev_to_xkeycode(248), None);
        assert_eq!(xkeycode_to_evdev(38), Some(30));
        assert_eq!(xkeycode_to_evdev(3), None);
    }

    #[test]
    fn held_lets_go_last_first() {
        let mut h = Held::default();
        for m in [
            Inject::Key { code: 29, down: true },
            Inject::Key { code: 30, down: true },
            Inject::Button { button: BTN_LEFT, down: true },
            Inject::Key { code: 30, down: false },
            Inject::Key { code: 46, down: true },
            Inject::MoveBy { dx: 1, dy: 1 },
        ] {
            h.update(&m);
        }
        assert_eq!(
            h.release(),
            vec![
                Inject::Button { button: BTN_LEFT, down: false },
                Inject::Key { code: 46, down: false },
                Inject::Key { code: 29, down: false },
            ]
        );
        assert!(h.is_empty() && h.release().is_empty());
    }

    #[test]
    fn pressed_drops_repeats_and_strays() {
        let mut p = Pressed::default();
        assert!(p.update(30, true));
        assert!(!p.update(30, true), "auto-repeat");
        assert!(p.update(30, false));
        assert!(!p.update(29, false), "released but pressed before capture");
        assert!(p.update(29, true));
        p.clear();
        assert!(!p.update(29, false));
    }

    #[test]
    fn notches_carry_remainders() {
        let mut n = Notches::default();
        assert_eq!(n.discrete(0, 60), (0, 0));
        assert_eq!(n.discrete(0, 60), (0, 1));
        assert_eq!(n.discrete(-240, -120), (-2, -1));
        assert_eq!(n.smooth(0.0, 24.0), (0, 0));
        assert_eq!(n.smooth(0.0, 24.0), (0, 1));
        n.smooth(30.0, 0.0);
        n.stop();
        assert_eq!(n.smooth(30.0, 0.0), (0, 0), "stop forgets the remainder");
        assert_eq!(notch_buttons(-1, 2).collect::<Vec<_>>(), vec![5, 5, 6]);
        assert_eq!(notch_buttons(1, -1).collect::<Vec<_>>(), vec![4, 7]);
        assert_eq!(notch_buttons(0, 0).count(), 0);
    }

    #[test]
    fn scroll_valuators_to_120ths() {
        let mut a = ScrollAxis::default();
        assert_eq!(a.push(15.0, 15.0), 120);
        assert_eq!(a.push(5.0, 15.0), 40);
        assert_eq!(a.push(-15.0, -15.0), 120, "inverted increment");
        assert_eq!(a.push(1.0, 0.0), 0);
        let mut b = ScrollAxis::default();
        assert_eq!(b.push(0.1, 1.0) + b.push(0.1, 1.0) + b.push(0.1, 1.0), 36);
    }

    #[test]
    fn axis_frames() {
        let mut f = AxisFrame::default();
        f.wheel_source();
        f.value(0, 15.0);
        f.value120(0, 120);
        assert_eq!(f.finish(), vec![Event::ScrollDiscrete { dx: 0, dy: 120 }]);
        f.wheel_source();
        f.value(1, -15.0);
        f.discrete(1, -1);
        assert_eq!(f.finish(), vec![Event::ScrollDiscrete { dx: -120, dy: 0 }]);
        f.value(0, 3.5);
        f.value(1, 1.0);
        assert_eq!(f.finish(), vec![Event::Scroll { dx: 1.0, dy: 3.5 }]);
        f.stop();
        assert_eq!(f.finish(), vec![Event::ScrollStop]);
        assert_eq!(f.finish(), vec![]);
    }

    #[test]
    fn barriers_sit_on_the_edges() {
        let r = Rect::new(1920, 0, 1440, 2560);
        assert_eq!(barrier_line(Side::Left, &r), (1920, 0, 1920, 2559));
        assert_eq!(barrier_line(Side::Right, &r), (3360, 0, 3360, 2559));
        assert_eq!(barrier_line(Side::Top, &r), (1920, 0, 3359, 0));
        assert_eq!(barrier_line(Side::Bottom, &r), (1920, 2560, 3359, 2560));
        assert_eq!(barrier_open_directions(Side::Left), 1 | 2 | 8);
        assert_eq!(barrier_open_directions(Side::Bottom), 1 | 4 | 8);
        assert_eq!(edge_strip(Side::Right, &r), Rect::new(3359, 0, 1, 2560));
        assert_eq!(edge_strip(Side::Bottom, &r), Rect::new(1920, 2559, 1440, 1));
    }

    #[test]
    fn edge_strips_follow_segments_and_preserve_output_origins() {
        let desktop = Desktop::new(vec![Rect::new(-800, -200, 800, 900), Rect::new(0, -200, 1200, 600)]);
        // A segment across both monitors' top edges is cut at the seam.
        let top = [Edge { side: Side::Top, at: -200, lo: -100, hi: 301 }];
        let strips = edge_strips(&top, &desktop);
        assert_eq!(strips, vec![
            (Side::Top, desktop.screens[0], Rect::new(-100, -200, 100, 1)),
            (Side::Top, desktop.screens[1], Rect::new(0, -200, 301, 1)),
        ]);
        for (side, output, strip) in &strips {
            assert_eq!(strip.y - output.y, 0);
            assert!(strip.x >= output.x && strip.right() <= output.right());
            assert!(strip.x >= -100 && strip.right() <= 301);
            assert_eq!(barrier_line(*side, strip), (strip.x, -200, strip.right() - 1, -200));
        }
        let right = [Edge { side: Side::Right, at: 1200, lo: 300, hi: 800 }];
        assert_eq!(edge_strips(&right, &desktop), vec![(Side::Right, desktop.screens[1], Rect::new(1199, 300, 1, 100))]);
        assert_eq!(barrier_line(Side::Right, &Rect::new(1199, 300, 1, 100)), (1200, 300, 1200, 399));
        let outer: Vec<Edge> = desktop.outer_edges(Side::Bottom).map(|r| Edge::of(Side::Bottom, r)).collect();
        let bottom = edge_strips(&outer, &desktop);
        assert_eq!(bottom.len(), 1);
        assert_eq!(bottom[0].2, Rect::new(-800, 699, 800, 1));
        assert_eq!(edge_strips(&[Edge { side: Side::Right, at: 1200, lo: 5, hi: 5 }], &desktop), vec![]);
        // Not on any monitor's edge line: nothing.
        assert_eq!(edge_strips(&[Edge { side: Side::Right, at: 1100, lo: 0, hi: 100 }], &desktop), vec![]);
    }

    /// An exposed edge set back from the bounding box (the shorter
    /// monitor's bottom) gets a strip and a barrier like an outer one.
    #[test]
    fn inner_exposed_edges_get_strips() {
        let desktop = Desktop::new(vec![Rect::new(-800, -200, 800, 900), Rect::new(0, -200, 1200, 600)]);
        let inner: Vec<Edge> = desktop.exposed_edges(Side::Bottom).into_iter().filter(|e| e.at == 400).collect();
        assert_eq!(inner, vec![Edge { side: Side::Bottom, at: 400, lo: 0, hi: 1200 }]);
        let strips = edge_strips(&inner, &desktop);
        assert_eq!(strips, vec![(Side::Bottom, desktop.screens[1], Rect::new(0, 399, 1200, 1))]);
        assert_eq!(barrier_line(Side::Bottom, &strips[0].2), (0, 400, 1199, 400));
        // The left monitor's right edge faces the right monitor for y
        // -200..400 and is exposed below that: only that part.
        let right: Vec<Edge> = desktop.exposed_edges(Side::Right).into_iter().filter(|e| e.at == 0).collect();
        assert_eq!(edge_strips(&right, &desktop), vec![(Side::Right, desktop.screens[0], Rect::new(-1, 400, 1, 300))]);
    }

    #[test]
    fn activation_stays_in_the_armed_segment() {
        let edges = [Edge { side: Side::Right, at: 800, lo: 300, hi: 501 }];
        assert!(!armed_at(&edges, Side::Right, Pos::new(799.0, 299.5)));
        assert!(armed_at(&edges, Side::Right, Pos::new(799.0, 300.0)));
        assert!(armed_at(&edges, Side::Right, Pos::new(799.5, 500.75)));
        assert!(!armed_at(&edges, Side::Right, Pos::new(799.0, 501.0)));
        assert!(!armed_at(&edges, Side::Left, Pos::new(799.0, 400.0)), "another side");
        assert!(!armed_at(&edges, Side::Right, Pos::new(500.0, 400.0)), "away from the line");
        assert_eq!(segment_point(&edges[0]), Pos::new(799.0, 400.0));
        let top = Edge { side: Side::Top, at: -200, lo: 0, hi: 1200 };
        assert_eq!(segment_point(&top), Pos::new(599.5, -200.0));
        assert!(armed_at(&[top], Side::Top, segment_point(&top)));
    }

    #[test]
    fn edge_push_needs_outward_motion() {
        let mut e = EdgePush::default();
        assert_eq!(e.push(-5.0, 0.0), None);
        e.enter(Side::Left, Pos::new(0.0, 100.0));
        assert_eq!(e.push(5.0, 0.0), None, "moving back in");
        assert_eq!(e.push(0.0, -3.0), None, "along the edge");
        e.moved(Pos::new(0.0, 97.0));
        assert_eq!(e.push(-1.0, 2.0), Some((Side::Left, Pos::new(0.0, 97.0))));
        e.leave();
        assert_eq!(e.push(-1.0, 0.0), None);
        assert!(pushes_out(Side::Bottom, 0.0, 0.5) && !pushes_out(Side::Top, 0.0, 0.5));
    }

    #[test]
    fn absolute_motion_spans_the_layout() {
        let d = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, -200, 1280, 1024)]);
        let (x, y, w, h) = absolute_motion(&d, Pos::new(1920.5, 0.0));
        assert_eq!((w, h), (3200 * 8, 1280 * 8));
        assert_eq!((x, y), (1920 * 8 + 4, 200 * 8));
        let (x, y, ..) = absolute_motion(&d, Pos::new(99999.0, -99999.0));
        assert_eq!((x, y), (3200 * 8 - 1, 0), "clamped onto a monitor");
    }

    #[test]
    fn output_geometry() {
        let mut o = OutputGeometry { position: Some((0, 0)), mode: Some((3840, 2160)), scale: 2, ..Default::default() };
        assert_eq!(o.rect(), Some(Rect::new(0, 0, 1920, 1080)));
        o.transform = 1;
        assert_eq!(o.rect(), Some(Rect::new(0, 0, 1080, 1920)));
        o.logical_position = Some((1920, 0));
        o.logical_size = Some((2560, 1440));
        assert_eq!(o.rect(), Some(Rect::new(1920, 0, 2560, 1440)), "xdg-output wins (fractional scale)");
        assert_eq!(OutputGeometry::default().rect(), None);
    }

    #[test]
    fn virtual_keyboard_modifiers() {
        let mut m = XkbMods::default();
        assert!(m.key(42, true));
        assert!(!m.key(42, true), "repeat");
        assert!(m.key(29, true));
        assert_eq!(m.depressed(), 1 | 4);
        assert!(m.key(42, false));
        assert!(!m.key(30, true), "not a modifier");
        assert!(m.key(58, true));
        assert_eq!(m.locked, 2);
        assert!(!m.key(58, false));
        assert!(m.key(58, true));
        assert_eq!(m.locked, 0, "Caps Lock toggles");
        assert!(m.key(100, true));
        assert_eq!(m.depressed(), 4 | 128);
    }

    #[test]
    fn bitmaps() {
        let mut keys = [0u8; 32];
        keys[4] = 0b0100_0001; // 32 and 38
        keys[31] = 0x80; // 255
        assert_eq!(set_bits(&keys), vec![32, 38, 255]);
        assert_eq!(set_bits(&[0; 4]), Vec::<u32>::new());
    }

    #[test]
    fn fixed_point() {
        assert_eq!(fp3232(-2, 1 << 31), -1.5);
        assert_eq!(fp3232(3, 0), 3.0);
        assert_eq!(fp1616(-65536 * 3 / 2), -1.5);
        assert_eq!(absolute_delta(0.0, 32767.0, 0.0, 65535.0, 1920), 32767.0 / 65535.0 * 1920.0);
        assert_eq!(absolute_delta(1.0, 2.0, 5.0, 5.0, 1920), 0.0);
    }
}
