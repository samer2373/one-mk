//! Cross-platform checks on the pure logic of the backends. Linux, Windows
//! and macOS each have their own key and wheel conventions; these tests
//! follow one key press or wheel notch from where it is captured on one OS to
//! where it is injected on the others, so that a table or a sign that is
//! right in its own module but disagrees with its neighbour is caught here.
//! The Windows and macOS logic is plain Rust (it is compiled on every OS for
//! this), so these run on any machine; what they cannot show is how the real
//! OS APIs behave.

use crate::common_logic::{
    Backoff, Controlled, KeyRepeat, LOCAL_INPUT_EVERY_MS, LocalInputGate, coalesce, is_repeatable,
};
use crate::win_logic::{self, SenderKeys, Space, WinKey};
use crate::{Inject, linux_logic, macos_logic};
use one_mk_proto::{Edge, Pos, Rect, Side, keynames, mediakeys, swap_ctrl_meta};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

fn name(code: u32) -> String {
    format!("{code}={}", keynames::key_name(code))
}

/// Every evdev code a Windows keyboard can report through the hook.
fn windows_capturable() -> BTreeSet<u32> {
    let mut codes = BTreeSet::new();
    for scan in 0..=0xFFu16 {
        for extended in [false, true] {
            codes.extend(win_logic::scan_to_evdev(scan, extended));
        }
    }
    codes
}

fn mac_deliverable(code: u32) -> bool {
    macos_logic::evdev_to_mac(code).is_some() || mediakeys::to_mac_nx(code).is_some()
}

// -------------------------------------------------------------- key tables

/// What a Windows keyboard types is delivered to a Linux X session in full, and to a Mac
/// except for the keys a Mac has no equivalent of. New gaps fail here; closing one never does.
#[test]
fn a_windows_keyboard_reaches_linux_and_macos_except_for_keys_they_lack() {
    let codes = windows_capturable();
    assert!(codes.len() > 120, "{} keys", codes.len());
    let to_x: Vec<_> =
        codes.iter().filter(|&&c| linux_logic::evdev_to_xkeycode(c).is_none()).map(|&c| name(c)).collect();
    assert!(to_x.is_empty(), "undeliverable to X: {to_x:?}");
    // Num Lock (it would toggle the Mac's keypad into arrows), JIS kana keys, power keys,
    // browser and launcher keys, and F21-F24 have no Mac key.
    let documented: BTreeSet<u32> =
        [69, 90, 91, 116, 128, 140, 142, 143, 155, 156, 157, 158, 159, 166, 172, 173, 191, 192, 193, 194, 217, 226]
            .into();
    let to_mac: BTreeSet<u32> = codes.iter().copied().filter(|&c| !mac_deliverable(c)).collect();
    let unexpected: Vec<_> = to_mac.difference(&documented).map(|&c| name(c)).collect();
    assert!(unexpected.is_empty(), "newly undeliverable to a Mac: {unexpected:?}");
    // The letters, digits, punctuation, editing and arrow keys, the keypad and all of F1-F20
    // are never among the gaps.
    for code in (1..=88u32).chain(96..=111).chain(183..=190) {
        if windows_capturable().contains(&code) && code != 69 {
            assert!(mac_deliverable(code), "{} must reach a Mac", name(code));
        }
    }
}

#[test]
fn a_mac_keyboard_reaches_windows_and_linux_except_fn_and_clear() {
    let mac: BTreeSet<u32> = macos_logic::KEYMAP.iter().map(|&(_, e)| e).collect();
    let to_win: BTreeSet<u32> = mac.iter().copied().filter(|&c| win_logic::evdev_to_win(c).is_none()).collect();
    let to_x: BTreeSet<u32> = mac.iter().copied().filter(|&c| linux_logic::evdev_to_xkeycode(c).is_none()).collect();
    // Fn has no Windows key; Clear (the keypad key where a PC has Num Lock) is code 355, past
    // what X can number. The JIS keypad comma (95) is the keypad comma Windows types as scan code 0x7E.
    assert!(to_win.is_subset(&[464, 355].into()), "{:?}", to_win.iter().map(|&c| name(c)).collect::<Vec<_>>());
    assert!(to_x.is_subset(&[464, 355].into()), "{:?}", to_x.iter().map(|&c| name(c)).collect::<Vec<_>>());
    // The Mac's own function and navigation keys are all there.
    for (kc, evdev) in macos_logic::KEYMAP {
        if !matches!(evdev, 464 | 355) {
            assert!(win_logic::evdev_to_win(*evdev).is_some(), "mac key {kc:#x} = {}", name(*evdev));
        }
    }
}

/// The key on a Mac's JIS keyboard with a keypad (kVK_JIS_KeypadComma, 0x5F) is typed on Windows as the
/// keypad comma: scan code 0x7E, which is what a Windows keyboard's own keypad comma reports.
#[test]
fn the_mac_jis_keypad_comma_is_typed_on_windows_as_the_keypad_comma() {
    let evdev = macos_logic::mac_to_evdev(0x5F).expect("the Mac key is known");
    assert_eq!(evdev, 95, "KEY_KPJPCOMMA");
    assert_eq!(win_logic::evdev_to_win(evdev), Some(WinKey::Scan { scan: 0x7E, extended: false }));
    assert_eq!(win_logic::evdev_to_win(121), win_logic::evdev_to_win(evdev), "the same key as KEY_KPCOMMA");
    // It is a way in only: a Windows keypad comma stays KEY_KPCOMMA (121), as before.
    assert_eq!(win_logic::scan_to_evdev(0x7E, false), Some(121));
    assert_eq!(win_logic::hook_key_to_evdev(0xC2, 0x7E, false), Some(121));
}

/// An X session sends keycodes that no Windows or Mac keyboard has: those are dropped, not
/// turned into some other key, and the keys they share round-trip exactly.
#[test]
fn linux_keys_are_delivered_as_themselves_or_not_at_all() {
    for keycode in 0..=255u32 {
        let Some(evdev) = linux_logic::xkeycode_to_evdev(keycode) else { continue };
        assert_eq!(linux_logic::evdev_to_xkeycode(evdev), Some(keycode as u8));
        // Print Screen's second code (210), the JIS half/full-width key (85) and the JIS keypad comma (95)
        // are sent as their Windows twin and come back as that twin: a one-way alias, not a mix-up.
        if let (Some(WinKey::Scan { scan, extended }), false) =
            (win_logic::evdev_to_win(evdev), [210, 85, 95].contains(&evdev))
        {
            assert_eq!(
                win_logic::scan_to_evdev(scan, extended),
                Some(evdev),
                "{} through a Windows scan code",
                name(evdev)
            );
        }
        if let Some(mac) = macos_logic::evdev_to_mac(evdev) {
            // The aliases (Print Screen to F13, Menu to the context-menu key) come back as the
            // key macOS itself reports, which is a different evdev code: the same key either way.
            let back = macos_logic::mac_to_evdev(mac).unwrap();
            assert_eq!(macos_logic::evdev_to_mac(back), Some(mac), "{} through the Mac keycode {mac:#x}", name(evdev));
        }
    }
    assert_eq!(linux_logic::evdev_to_xkeycode(247), Some(255));
    assert_eq!(linux_logic::evdev_to_xkeycode(248), None, "past the last X keycode");
    assert_eq!(linux_logic::evdev_to_xkeycode(u32::MAX), None, "no overflow");
    assert_eq!(linux_logic::xkeycode_to_evdev(7), None);
}

#[test]
fn every_key_a_person_can_name_in_a_rule_is_deliverable_or_known_not_to_be() {
    // Rules may name any of these; the ones a target cannot type are dropped there. This pins
    // which, so a rule that stops working shows up here first.
    let named: Vec<u32> = keynames::all_names().map(|(c, _, _)| c).filter(|&c| c < 0x100).collect();
    assert!(named.len() > 150);
    let no_win: BTreeSet<u32> = named.iter().copied().filter(|&c| win_logic::evdev_to_win(c).is_none()).collect();
    let expected_no_win: BTreeSet<u32> =
        [101, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 161, 224, 225, 228, 229, 230, 248].into();
    assert!(
        no_win.is_subset(&expected_no_win),
        "newly missing on Windows: {:?}",
        no_win.difference(&expected_no_win).map(|&c| name(c)).collect::<Vec<_>>()
    );
    // The keys every keyboard has are never missing anywhere.
    for key in [
        "a",
        "z",
        "0",
        "9",
        "enter",
        "esc",
        "space",
        "tab",
        "backspace",
        "capslock",
        "lctrl",
        "rctrl",
        "lshift",
        "rshift",
        "lalt",
        "ralt",
        "lmeta",
        "rmeta",
        "left",
        "right",
        "up",
        "down",
        "home",
        "end",
        "pageup",
        "pagedown",
        "delete",
        "insert",
        "f1",
        "f12",
        "kp0",
        "kp9",
        "kpenter",
    ] {
        let code = keynames::parse_key(key).unwrap();
        assert!(win_logic::evdev_to_win(code).is_some(), "{key} on Windows");
        assert!(macos_logic::evdev_to_mac(code).is_some(), "{key} on a Mac");
        assert!(linux_logic::evdev_to_xkeycode(code).is_some(), "{key} on X");
    }
}

// ----------------------------------------------- Ctrl/Cmd, AltGr, Caps Lock

#[test]
fn ctrl_c_on_a_pc_keyboard_is_cmd_c_on_the_mac_and_back() {
    use macos_logic::{Modifiers, flags};
    // PC -> Mac. Windows reports left Ctrl as scan 0x1D and right Ctrl as the extended 0x1D.
    let (lctrl, rctrl) = (
        win_logic::hook_key_to_evdev(0xA2, 0x1D, false).unwrap(),
        win_logic::hook_key_to_evdev(0xA3, 0x1D, true).unwrap(),
    );
    assert_eq!((lctrl, rctrl), (29, 97));
    let mut mac = Modifiers::default();
    assert!(mac.update(swap_ctrl_meta(lctrl), true));
    assert_eq!(macos_logic::evdev_to_mac(swap_ctrl_meta(lctrl)), Some(0x37), "left Ctrl types as left Command");
    assert_eq!(macos_logic::evdev_to_mac(swap_ctrl_meta(rctrl)), Some(0x36));
    let held = mac.flags();
    assert!(held & flags::COMMAND != 0 && held & flags::DEV_LCMD != 0, "Command is held");
    assert_eq!(held & (flags::CONTROL | flags::ALTERNATE | flags::SHIFT), 0, "and nothing else");
    assert_eq!(macos_logic::evdev_to_mac(46), Some(0x08), "C is the same key");
    mac.update(swap_ctrl_meta(lctrl), false);
    assert_eq!(mac.flags() & flags::COMMAND, 0, "released with it");
    // Mac -> PC: Command is Ctrl, and the Control key becomes the Windows key.
    for (mac_kc, windows) in [
        (0x37u16, WinKey::Scan { scan: 0x1D, extended: false }),
        (0x36, WinKey::Scan { scan: 0x1D, extended: true }),
        (0x3B, WinKey::Scan { scan: 0x5B, extended: true }),
        (0x3E, WinKey::Scan { scan: 0x5C, extended: true }),
        // Option is Alt, on both sides, and is not swapped.
        (0x3A, WinKey::Scan { scan: 0x38, extended: false }),
        (0x3D, WinKey::Scan { scan: 0x38, extended: true }),
        (0x38, WinKey::Scan { scan: 0x2A, extended: false }),
    ] {
        let evdev = macos_logic::mac_to_evdev(mac_kc).unwrap();
        assert_eq!(win_logic::evdev_to_win(swap_ctrl_meta(evdev)), Some(windows), "mac key {mac_kc:#x}");
    }
    // Swapping twice is the identity for every code, and only the four modifiers move.
    for code in 0..0x300 {
        assert_eq!(swap_ctrl_meta(swap_ctrl_meta(code)), code);
        assert_eq!(swap_ctrl_meta(code) != code, [29, 97, 125, 126].contains(&code), "{code}");
    }
}

#[test]
fn altgr_is_one_key_on_every_os() {
    // Windows sends AltGr as a fake left Ctrl (scan bit 9 set) followed by the extended right Alt;
    // forwarding the fake Ctrl would make every AltGr character arrive as Ctrl+AltGr.
    assert_eq!(win_logic::hook_key_to_evdev(0xA2, 0x21D, false), None, "the fake Ctrl is dropped");
    assert_eq!(win_logic::hook_key_to_evdev(0xA5, 0x38, true), Some(100), "AltGr is right Alt");
    assert_eq!(win_logic::hook_key_to_evdev(0xA2, 0x1D, false), Some(29), "a real left Ctrl still counts");
    assert_eq!(win_logic::hook_key_to_evdev(0xA4, 0x38, false), Some(56), "left Alt is not AltGr");
    // Typed on Windows, AltGr is delivered as the right-hand Alt: extended 0x38; a Mac's right
    // Option; X's right Alt (keycode 108).
    assert_eq!(win_logic::evdev_to_win(100), Some(WinKey::Scan { scan: 0x38, extended: true }));
    assert_eq!(macos_logic::evdev_to_mac(100), Some(0x3D));
    assert_eq!(linux_logic::evdev_to_xkeycode(100), Some(108));
    // On a Mac, right Option held makes the Alternate flag with the right-hand device bit.
    let mut mac = macos_logic::Modifiers::default();
    mac.update(100, true);
    assert_eq!(mac.flags() & macos_logic::flags::DEV_ALL, macos_logic::flags::DEV_RALT);
    assert!(mac.flags() & macos_logic::flags::ALTERNATE != 0);
    // A held-key sender forwards AltGr's key down once, however long it repeats.
    let mut sender = SenderKeys::default();
    assert!(sender.forward(100, true));
    assert!(!sender.forward(100, true) && !sender.forward(100, true));
    assert!(sender.forward(100, false));
}

#[test]
fn caps_lock_toggles_exactly_once_per_press_however_it_repeats() {
    let mut sender = SenderKeys::default();
    let mut mac = macos_logic::Modifiers::default();
    let mut toggles = 0;
    let mut state = false;
    for _round in 0..3 {
        // A PC keyboard repeats a held Caps Lock: 1 press, 9 repeats, 1 release.
        for i in 0..10 {
            if sender.forward(58, true) {
                if !mac.is_caps_repeat(58, true) {
                    mac.update(58, true);
                    let now = mac.flags() & macos_logic::flags::ALPHA_SHIFT != 0;
                    toggles += usize::from(now != state);
                    state = now;
                }
            } else {
                assert!(i > 0, "the first press is always forwarded");
            }
        }
        sender.forward(58, false);
        mac.update(58, false);
    }
    assert_eq!(toggles, 3, "three presses, three toggles");
    assert!(state, "an odd number of presses leaves it on");
    // The receiver's own repeat never repeats it either, nor any other lock or modifier.
    for code in [58u32, 69, 70, 29, 42, 54, 56, 97, 100, 125, 126, 464] {
        assert!(!is_repeatable(code), "{}", name(code));
    }
    for code in [30u32, 28, 14, 57, 103, 105, 106, 108] {
        assert!(is_repeatable(code), "{}", name(code));
    }
}

// ---------------------------------------------------------------- scrolling

/// One notch of "scroll up" and "scroll right", from every source OS to every target OS.
#[test]
fn up_and_right_mean_the_same_on_every_os() {
    // Windows captures a wheel notch up as +120 and right as +120 on the horizontal wheel.
    let (win_up, win_right) = (win_logic::wheel_to_wire(120, false), win_logic::wheel_to_wire(120, true));
    // A Mac captures a mouse-wheel notch as lines: positive Y is up, positive X is left.
    let mac_up = macos_logic::scroll_to_wire(&macos_logic::MacScroll { line_y: 1, ..Default::default() });
    let mac_left = macos_logic::scroll_to_wire(&macos_logic::MacScroll { line_x: 1, ..Default::default() });
    let from_mac = |events: [Option<crate::Event>; 2]| match &events[0] {
        Some(crate::Event::ScrollDiscrete { dx, dy, .. }) => (*dx, *dy),
        other => panic!("{other:?}"),
    };
    assert_eq!(win_up, (0, -120), "wire: negative dy is up");
    assert_eq!(win_right, (120, 0), "wire: positive dx is right");
    assert_eq!(from_mac(mac_up), (0, -120));
    assert_eq!(from_mac(mac_left), (-120, 0), "the Mac's positive horizontal is left");
    // Delivered to Windows: wheel +120 is up; hwheel +120 is right.
    assert_eq!(win_logic::wire_to_wheel(0, -120), (0, 120));
    assert_eq!(win_logic::wire_to_wheel(120, 0), (120, 0));
    // Delivered to a Mac: wheel1 positive is up; wheel2 positive is left.
    let (up, _) = macos_logic::wire_discrete_to_mac_lines(0, -120);
    let (_, left) = macos_logic::wire_discrete_to_mac_lines(-120, 0);
    let (_, right) = macos_logic::wire_discrete_to_mac_lines(120, 0);
    assert!(up > 0.0 && left > 0.0 && right < 0.0, "{up} {left} {right}");
    assert_eq!(macos_logic::wire_to_mac_scroll(0.0, -48.0), (48.0, -0.0));
    // Delivered to X11: button 4 is up, 5 down, 6 left, 7 right.
    assert_eq!(linux_logic::notch_buttons(0, -1).collect::<Vec<_>>(), [4]);
    assert_eq!(linux_logic::notch_buttons(0, 2).collect::<Vec<_>>(), [5, 5]);
    assert_eq!(linux_logic::notch_buttons(-1, 0).collect::<Vec<_>>(), [6]);
    assert_eq!(linux_logic::notch_buttons(3, 0).collect::<Vec<_>>(), [7, 7, 7]);
    // Smooth scrolling agrees too: the same pixels up are up on each.
    let mut windows = win_logic::SmoothScroll::default();
    let mut x = linux_logic::Notches::default();
    let (_, win_v) = windows.push(0.0, -96.0);
    let (_, x_v) = x.smooth(0.0, -96.0);
    assert!(win_v > 0 && x_v < 0, "Windows counts up as positive, the wire and X as negative");
    // Neither side keeps a remainder that leaks into the other direction.
    for dy in [-1.0f64, 1.0] {
        let mut windows = win_logic::SmoothScroll::default();
        let mut x = linux_logic::Notches::default();
        let (mut wt, mut xt) = (0i64, 0i64);
        for _ in 0..1000 {
            wt += i64::from(windows.push(0.0, dy * 7.3).1);
            xt += i64::from(x.smooth(0.0, dy * 7.3).1);
        }
        // 7300 px is 152 notches of 48 px; every notch is 120 Windows units.
        assert!((wt.abs() - 152 * 120).abs() <= 120, "{wt}");
        assert!((xt.abs() - 152).abs() <= 1, "{xt}");
        assert_eq!(wt.signum(), -dy as i64);
        assert_eq!(xt.signum(), dy as i64);
    }
}

// --------------------------------------------------- Windows scaling and DPI

fn random_layout(rng: &mut Rng) -> (Vec<Rect>, u32) {
    let sizes =
        [(1920, 1080), (3840, 2160), (2560, 1440), (1366, 768), (1440, 900), (2880, 1800), (1080, 1920), (7680, 4320)];
    let (w, h) = rng.pick(&sizes);
    let mut screens = vec![Rect::new(0, 0, w, h)];
    for _ in 0..rng.below(3) {
        let (w2, h2) = rng.pick(&sizes);
        let last = *screens.last().unwrap();
        let slide = rng.below(5) as i32 * 200 - 400;
        screens.push(match rng.below(4) {
            0 => Rect::new(last.right(), last.y + slide, w2, h2),
            1 => Rect::new(last.x - w2, last.y + slide, w2, h2),
            2 => Rect::new(last.x + slide, last.bottom(), w2, h2),
            _ => Rect::new(last.x + slide, last.y - h2, w2, h2),
        });
    }
    let dpi = rng.pick(&[0u32, 96, 120, 144, 168, 192, 216, 240, 288, 384]);
    (screens, dpi)
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// The reported (logical) desktop is the physical one divided by one scale, whatever the mix of
/// monitors: pixels map back to where they came from, no monitor overlaps another, and monitors
/// that touched still touch.
#[test]
fn a_mixed_monitor_desktop_is_scaled_without_overlaps_or_gaps() {
    let mut rng = Rng(0x0D91_2024);
    let mut checked = 0;
    for _ in 0..4000 {
        let (screens, dpi) = random_layout(&mut rng);
        if screens.iter().enumerate().any(|(i, a)| screens.iter().skip(i + 1).any(|b| overlap(a, b))) {
            continue;
        }
        checked += 1;
        let space = Space::new(screens.clone(), dpi);
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
        assert!((space.scale() - scale).abs() < 1e-12);
        let logical = space.logical();
        for (i, (p, l)) in screens.iter().zip(&logical.screens).enumerate() {
            assert!(l.w >= 1 && l.h >= 1);
            assert!((f64::from(l.w) * scale - f64::from(p.w)).abs() <= 2.0 * scale + 1.0, "{p:?} -> {l:?} at {scale}");
            for (j, q) in logical.screens.iter().enumerate().skip(i + 1) {
                assert!(!overlap(l, q), "scaled monitors {i} and {j} overlap: {logical:?} from {screens:?} at {scale}");
            }
        }
        for (i, (a, b)) in screens.iter().zip(screens.iter().skip(1)).enumerate() {
            let touch_x = a.right() == b.x || b.right() == a.x;
            let touch_y = a.bottom() == b.y || b.bottom() == a.y;
            if (touch_x && a.y < b.bottom() && b.y < a.bottom()) || (touch_y && a.x < b.right() && b.x < a.right()) {
                let (l, m) = (&logical.screens[i], &logical.screens[i + 1]);
                let gap = if l.right().min(m.right()) <= l.x.max(m.x) {
                    (l.right() - m.x).abs().min((m.right() - l.x).abs())
                } else {
                    0
                };
                let gap_y = if l.bottom().min(m.bottom()) <= l.y.max(m.y) {
                    (l.bottom() - m.y).abs().min((m.bottom() - l.y).abs())
                } else {
                    0
                };
                assert!(
                    gap == 0 && gap_y == 0,
                    "monitors that touched are {gap}/{gap_y} px apart after scaling: {screens:?} at {scale}"
                );
            }
        }
        // Every physical pixel maps into the reported space and back to within one pixel.
        for _ in 0..50 {
            let s = screens[rng.below(screens.len() as u64) as usize];
            let (x, y) = (s.x + rng.below(s.w as u64) as i32, s.y + rng.below(s.h as u64) as i32);
            let there = space.to_logical(x, y);
            assert!(logical.screen_at(there).is_some(), "{there:?} is on a reported monitor");
            let (bx, by) = space.to_phys(there);
            assert!(
                (bx - x).abs() <= scale.ceil() as i32 && (by - y).abs() <= scale.ceil() as i32,
                "({x},{y}) -> {there:?} -> ({bx},{by}) at {scale}"
            );
            assert!(
                screens.iter().any(|r| (r.x..r.right()).contains(&bx) && (r.y..r.bottom()).contains(&by)),
                "lands on a monitor"
            );
        }
        // Absolute SendInput coordinates stay in range across the whole virtual desktop.
        let b = space.phys().bounds();
        for (x, y) in [(b.x, b.y), (b.right() - 1, b.bottom() - 1)] {
            let (nx, ny) = space.normalize(x, y);
            assert!((0..=65535).contains(&nx) && (0..=65535).contains(&ny));
        }
    }
    assert!(checked > 1500, "{checked} layouts checked");
}

/// Differential test: whether the pointer at a physical pixel is "at an armed edge" is decided
/// by `Space::edge_hit` from rounded scaled rectangles; here it is compared with a plain
/// physical-pixel definition (the last pixel of a monitor with no monitor just beyond it).
#[test]
fn edges_are_hit_exactly_where_the_cursor_cannot_go_further() {
    let mut rng = Rng(0xED6E_5EED);
    let mut hits = 0;
    let mut layouts = 0;
    for _ in 0..1500 {
        let (screens, dpi) = random_layout(&mut rng);
        if screens.iter().enumerate().any(|(i, a)| screens.iter().skip(i + 1).any(|b| overlap(a, b))) {
            continue;
        }
        layouts += 1;
        let space = Space::new(screens.clone(), dpi);
        let scale = space.scale();
        let mut armed: Vec<Edge> = Vec::new();
        for side in Side::ALL {
            armed.extend(space.logical().exposed_edges(side));
        }
        let on_screen =
            |x: i32, y: i32| screens.iter().any(|r| (r.x..r.right()).contains(&x) && (r.y..r.bottom()).contains(&y));
        // Near the ends of an edge's span, rounding to logical pixels may legitimately differ
        // by a pixel; those few pixels are left out.
        let slack = scale.ceil() as i32 * 2 + 2;
        let near_a_seam = |r: &Rect, side: Side, x: i32, y: i32| {
            let along = if side.along_x() { y } else { x };
            screens.iter().filter(|o| *o != r).any(|o| {
                let (lo, hi) = if side.along_x() { (o.y, o.bottom()) } else { (o.x, o.right()) };
                (along - lo).abs() <= slack || (along - hi).abs() <= slack
            })
        };
        for r in &screens {
            for _ in 0..80 {
                let (x, y) = (r.x + rng.below(r.w as u64) as i32, r.y + rng.below(r.h as u64) as i32);
                let (dx, dy) = (rng.below(3) as f64 - 1.0, rng.below(3) as f64 - 1.0);
                // The sides this pixel is the end of the cursor's reach on.
                let mut expected = Vec::new();
                if x == r.x && !on_screen(x - 1, y) && !near_a_seam(r, Side::Left, x, y) {
                    expected.push(Side::Left);
                }
                if x == r.right() - 1 && !on_screen(x + 1, y) && !near_a_seam(r, Side::Right, x, y) {
                    expected.push(Side::Right);
                }
                if y == r.y && !on_screen(x, y - 1) && !near_a_seam(r, Side::Top, x, y) {
                    expected.push(Side::Top);
                }
                if y == r.bottom() - 1 && !on_screen(x, y + 1) && !near_a_seam(r, Side::Bottom, x, y) {
                    expected.push(Side::Bottom);
                }
                let got = space.edge_hit(&armed, x, y, dx, dy);
                let interior = x > r.x && x < r.right() - 1 && y > r.y && y < r.bottom() - 1;
                if interior {
                    assert_eq!(got, None, "interior pixel ({x},{y}) of {r:?} reported {got:?}: {screens:?} at {scale}");
                }
                if let Some(side) = got {
                    hits += 1;
                    let at_border = match side {
                        Side::Left => x == r.x,
                        Side::Right => x == r.right() - 1,
                        Side::Top => y == r.y,
                        Side::Bottom => y == r.bottom() - 1,
                    };
                    assert!(
                        at_border
                            || near_a_seam(r, side, x, y)
                            || screens.iter().any(|o| o.x <= x
                                && x < o.right()
                                && o.y <= y
                                && y < o.bottom()
                                && (match side {
                                    Side::Left => x == o.x,
                                    Side::Right => x == o.right() - 1,
                                    Side::Top => y == o.y,
                                    Side::Bottom => y == o.bottom() - 1,
                                })),
                        "({x},{y}) reported as {side:?} but is not on that edge of any monitor: {screens:?}"
                    );
                }
                if expected.len() == 1 {
                    assert_eq!(
                        got,
                        Some(expected[0]),
                        "pixel ({x},{y}) of {r:?} is the end of the reach on {:?}: {screens:?} at {scale}",
                        expected[0]
                    );
                } else if expected.len() > 1 {
                    assert!(
                        got.is_some_and(|s| expected.contains(&s)),
                        "corner ({x},{y}): {got:?} not in {expected:?}"
                    );
                }
            }
        }
    }
    assert!(layouts > 600 && hits > 200, "{layouts} layouts, {hits} hits");
}

/// `SendInput` takes a position as 0..=65535 over the virtual desktop and the OS converts it back
/// to a pixel by truncating (some Windows versions round): both must land on the intended pixel,
/// on every pixel of desktops up to six 8K monitors wide.
#[test]
fn absolute_coordinates_land_on_the_pixel_asked_for_whichever_way_windows_converts() {
    for size in
        [1, 2, 3, 640, 1366, 1920, 2560, 3000, 3440, 3840, 5120, 7680, 8192, 11_520, 15_360, 19_200, 30_720, 32_767]
    {
        for offset in 0..size {
            let n = i64::from(win_logic::normalize_axis(offset, size));
            assert!((0..=65535).contains(&n), "{offset}/{size}");
            if size == 1 {
                assert_eq!(n, 0);
                continue;
            }
            let truncated = n * i64::from(size) / 65536;
            let rounded = (n * i64::from(size) + 32768) / 65536;
            assert_eq!(truncated, i64::from(offset), "truncating: {offset} of {size} gave {n}");
            assert_eq!(rounded, i64::from(offset), "rounding: {offset} of {size} gave {n}");
        }
    }
    // Out-of-range input is held to the desktop, not wrapped.
    assert_eq!(win_logic::normalize_axis(-5, 1920), 0);
    assert_eq!(win_logic::normalize_axis(5000, 1920), win_logic::normalize_axis(1919, 1920));
    assert_eq!(win_logic::normalize_axis(i32::MIN, 1920), 0);
    assert_eq!(win_logic::normalize_axis(i32::MAX, 1920), win_logic::normalize_axis(1919, 1920));
    assert_eq!(win_logic::normalize_axis(10, 0), 0);
    assert_eq!(win_logic::normalize_axis(10, -4), 0);
}

// ------------------------------------------------------------ common logic

/// What the pointer does, as a person sees it: the position at the moment of each click or key.
type Place = (i64, i64);

fn replay(events: &[Inject]) -> (Vec<(Inject, Place)>, Place) {
    let mut at = (0i64, 0i64);
    let mut log = Vec::new();
    for event in events {
        match *event {
            Inject::MoveTo(p) => at = (p.x as i64, p.y as i64),
            Inject::MoveBy { dx, dy } => at = (at.0 + i64::from(dx), at.1 + i64::from(dy)),
            other => log.push((other, at)),
        }
    }
    (log, at)
}

#[test]
fn coalescing_never_changes_where_anything_happens() {
    let mut rng = Rng(0x00C0_A1E5);
    let mut reduced = 0usize;
    let mut total = 0usize;
    for _ in 0..4000 {
        let events: Vec<Inject> = (0..rng.below(40))
            .map(|_| match rng.below(8) {
                0 | 1 => Inject::MoveTo(Pos::new(rng.below(2000) as f64, rng.below(1200) as f64)),
                2 | 3 => Inject::MoveBy { dx: rng.below(41) as i32 - 20, dy: rng.below(41) as i32 - 20 },
                4 => Inject::Button { button: 0x110 + rng.below(3) as u32, down: rng.below(2) == 0 },
                5 => Inject::Key { code: 1 + rng.below(100) as u32, down: rng.below(2) == 0 },
                6 => Inject::ScrollDiscrete { dx: 0, dy: 120 - rng.below(3) as i32 * 120 },
                _ => Inject::Scroll { dx: 0.0, dy: rng.below(10) as f64 },
            })
            .collect();
        let mut out = Vec::new();
        coalesce(events.iter().copied(), &mut out);
        assert_eq!(replay(&events), replay(&out), "{events:?} -> {out:?}");
        assert!(out.len() <= events.len());
        // No two moves in a row that could have been one.
        assert!(
            !out.windows(2).any(|w| matches!(
                (w[0], w[1]),
                (Inject::MoveTo(_), Inject::MoveTo(_)) | (Inject::MoveBy { .. }, Inject::MoveBy { .. })
            )),
            "{out:?}"
        );
        reduced += events.len() - out.len();
        total += events.len();
    }
    assert!(reduced * 20 > total, "a mixed stream shrinks: {reduced} of {total}");
    // Relative moves that sum past what fits stop at the limit instead of wrapping to the other way.
    let mut out = Vec::new();
    coalesce([Inject::MoveBy { dx: i32::MAX, dy: i32::MIN }, Inject::MoveBy { dx: 5, dy: -5 }], &mut out);
    assert_eq!(out, [Inject::MoveBy { dx: i32::MAX, dy: i32::MIN }]);
    // Nothing queued is nothing sent.
    out.clear();
    coalesce([], &mut out);
    assert!(out.is_empty());
}

/// Receiver-side key repeat against a plain model, over random press, release and time steps.
#[test]
fn key_repeat_follows_the_last_pressed_key_and_stops_with_it() {
    let mut rng = Rng(0x4E57_BEA7);
    let t0 = Instant::now();
    for _ in 0..300 {
        let delay = Duration::from_millis(100 + rng.below(400));
        let interval = Duration::from_millis(10 + rng.below(90));
        let mut repeat = KeyRepeat::new(Some((delay, interval)));
        let mut held: Vec<u32> = Vec::new();
        let mut pressed_at: Option<(u32, Duration)> = None;
        let mut now = Duration::ZERO;
        let mut fired = 0;
        for _ in 0..200 {
            match rng.below(4) {
                0 => {
                    let code = rng.pick(&[30u32, 31, 32, 29, 42, 58, 14, 105]);
                    let accepted = repeat.key(code, true, t0 + now);
                    assert_eq!(accepted, !held.contains(&code), "a second down of {code} is dropped");
                    if accepted {
                        held.push(code);
                        if is_repeatable(code) {
                            pressed_at = Some((code, now));
                        }
                    }
                }
                1 => {
                    if let Some(&code) = held.first() {
                        assert!(repeat.key(code, false, t0 + now));
                        held.retain(|&c| c != code);
                        if pressed_at.is_some_and(|(c, _)| c == code) {
                            pressed_at = None;
                        }
                    }
                }
                2 => {
                    now += Duration::from_millis(rng.below(300));
                    while let Some(code) = repeat.poll(t0 + now) {
                        fired += 1;
                        let (expected, since) = pressed_at.expect("a repeat with no repeating key");
                        assert_eq!(code, expected, "only the most recently pressed repeatable key repeats");
                        assert!(held.contains(&code), "a released key never repeats");
                        assert!(now >= since + delay, "not before the delay: {now:?} vs {since:?}+{delay:?}");
                        if fired > 10_000 {
                            panic!("repeat never catches up");
                        }
                    }
                }
                _ => {
                    assert_eq!(repeat.deadline().is_some(), pressed_at.is_some());
                }
            }
        }
        repeat.clear();
        assert_eq!(repeat.deadline(), None);
        assert_eq!(repeat.poll(t0 + now + Duration::from_secs(10)), None, "nothing repeats after the peer lets go");
        assert!(repeat.key(30, true, t0 + now), "and a key held before can be pressed again");
    }
    // Turned off: nothing ever repeats; the interval is never shorter than 5 ms.
    let mut off = KeyRepeat::new(None);
    off.key(30, true, t0);
    assert_eq!((off.deadline(), off.poll(t0 + Duration::from_secs(60))), (None, None));
    let mut fast = KeyRepeat::new(None);
    fast.set_timing(Some((Duration::from_millis(1), Duration::ZERO)));
    fast.key(30, true, t0);
    assert_eq!(fast.poll(t0 + Duration::from_millis(1)), Some(30));
    assert_eq!(fast.poll(t0 + Duration::from_millis(2)), None, "the interval is at least 5 ms");
    assert_eq!(fast.poll(t0 + Duration::from_millis(6)), Some(30));
}

#[test]
fn control_messages_are_taken_out_of_a_batch_and_tell_when_control_ended() {
    let controlled = Controlled::default();
    let key = Inject::Key { code: 30, down: true };
    let mut batch = vec![key, Inject::Controlled(true), key, Inject::MoveTo(Pos::new(1.0, 2.0))];
    assert!(!controlled.apply(&mut batch), "taking control is not the end of it");
    assert!(controlled.get());
    assert_eq!(batch, [key, key, Inject::MoveTo(Pos::new(1.0, 2.0))], "everything else stays, in order");
    let mut batch = vec![Inject::Controlled(false), key];
    assert!(controlled.apply(&mut batch));
    assert!(!controlled.get());
    assert_eq!(batch, [key]);
    // The last word wins within a batch, and an end anywhere in it is reported.
    let mut batch = vec![Inject::Controlled(false), Inject::Controlled(true)];
    assert!(controlled.apply(&mut batch));
    assert!(controlled.get() && batch.is_empty());
    let mut none = vec![key];
    assert!(!controlled.apply(&mut none) && none == [key]);
}

#[test]
fn local_input_is_reported_only_while_controlled_and_at_most_ten_times_a_second() {
    let mut gate = LocalInputGate::default();
    assert!(!gate.check(1, false), "never while nobody controls this computer");
    assert!(gate.check(1000, true));
    assert!(!gate.check(1000 + LOCAL_INPUT_EVERY_MS - 1, true));
    assert!(gate.check(1000 + LOCAL_INPUT_EVERY_MS, true), "the boundary itself passes");
    assert!(!gate.check(5, true), "a clock that went back is throttled, not reported");
    assert!(!gate.check(1100 + 50, false));
    assert!(gate.check(5000, true));
}

#[test]
fn reconnecting_backs_off_and_starts_over() {
    let mut backoff = Backoff::default();
    let steps: Vec<u64> = (0..9).map(|_| backoff.next_delay().as_secs()).collect();
    assert_eq!(steps, [1, 2, 4, 8, 16, 30, 30, 30, 30]);
    backoff.reset();
    assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    // Hammering a dead session for a day never overflows or speeds up.
    for _ in 0..100_000 {
        backoff.next_delay();
    }
    assert_eq!(backoff.next_delay().as_secs(), 30);
}

// -------------------------------------------------------------- stuck keys

/// A peer that lets go (or disappears) with keys held: whatever the injection side pressed is
/// released, last pressed first, each exactly once, buttons included.
#[test]
fn nothing_stays_pressed_after_the_peer_lets_go() {
    let mut rng = Rng(0x0057_AC0F);
    for _ in 0..500 {
        let mut held = linux_logic::Held::default();
        let mut down: BTreeSet<(bool, u32)> = BTreeSet::new();
        for _ in 0..rng.below(60) {
            let event = if rng.below(4) == 0 {
                Inject::Button { button: 0x110 + rng.below(5) as u32, down: rng.below(2) == 0 }
            } else {
                Inject::Key { code: 1 + rng.below(120) as u32, down: rng.below(3) != 0 }
            };
            held.update(&event);
            match event {
                Inject::Key { code, down: true } => down.insert((false, code)),
                Inject::Key { code, down: false } => down.remove(&(false, code)),
                Inject::Button { button, down: true } => down.insert((true, button)),
                Inject::Button { button, down: false } => down.remove(&(true, button)),
                _ => unreachable!(),
            };
        }
        assert_eq!(held.is_empty(), down.is_empty());
        let releases = held.release();
        let released: BTreeSet<(bool, u32)> = releases
            .iter()
            .map(|r| match *r {
                Inject::Key { code, down: false } => (false, code),
                Inject::Button { button, down: false } => (true, button),
                other => panic!("{other:?} is not a release"),
            })
            .collect();
        assert_eq!(released, down, "exactly what was held is released");
        assert_eq!(releases.len(), down.len(), "each once");
        assert!(held.is_empty() && held.release().is_empty(), "and a second release has nothing left to do");
    }
}

/// Keys pressed before capture began are not the peer's: their release must not travel.
#[test]
fn a_key_held_from_before_the_capture_is_not_forwarded_when_it_goes_up() {
    let mut pressed = linux_logic::Pressed::default();
    assert!(!pressed.update(29, false), "Ctrl was already down when the pointer left: its release stays here");
    assert!(pressed.update(30, true));
    assert!(!pressed.update(30, true), "an auto-repeat is dropped");
    assert!(pressed.update(30, false));
    assert!(!pressed.update(30, false), "and a second release is a stray");
    let mut sender = SenderKeys::default();
    assert!(sender.forward(30, true) && !sender.forward(30, true) && sender.forward(30, false));
    sender.forward(31, true);
    sender.clear();
    assert!(sender.forward(31, true), "after the capture ends it starts over");
}
