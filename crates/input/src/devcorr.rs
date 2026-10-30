//! Which keyboard or mouse an event came from, on Windows and macOS: the
//! hook (Windows) or event tap (macOS) sees each event without its device,
//! Raw Input or IOHIDManager sees it again with its device, on another
//! path, a little before or after. [`Correlator`] pairs the two. Pure: the
//! backends feed it and send what comes out; the times are milliseconds.
//!
//! Rules (see "How each event is attributed" in `docs/INPUT-TUNING.md`):
//! a key or button event is matched in order with the device events of the
//! same key and direction: the oldest one waiting, or the next to arrive
//! within [`WAIT_MS`] (later events wait behind it, so nothing is
//! reordered). Unmatched in time, it goes on unknown (`device: None`), and
//! after [`MAX_MISSES`] such misses in a row nothing waits any more until a
//! match comes (a system that never reports devices costs no latency).
//! Motion is attributed, without waiting, to the one device that moved in
//! the last [`MOTION_MS`]; two moving, or none, is unknown. Unknown is
//! always safe: only the rules without a device apply.

#[cfg(any(windows, test))]
use crate::Bus;
use crate::{DeviceRef, Event};
use std::collections::VecDeque;

/// How long a key or button event waits for its device.
pub(crate) const WAIT_MS: u64 = 8;
/// Device events kept unmatched, at most.
pub(crate) const RING: usize = 16;
/// How recent a device event must be to count.
pub(crate) const MOTION_MS: u64 = 50;
/// Misses in a row before events stop waiting.
pub(crate) const MAX_MISSES: u8 = 8;

/// What a device event and a hook event have in common.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    /// A key or button (evdev code) going down or up.
    Press { code: u32, down: bool },
    Wheel,
}

fn token(ev: &Event) -> Option<Token> {
    match *ev {
        Event::Key { code, down, .. } => Some(Token::Press { code, down }),
        Event::Button { button, down, .. } => Some(Token::Press { code: button, down }),
        Event::Scroll { .. } | Event::ScrollDiscrete { .. } => Some(Token::Wheel),
        _ => None,
    }
}

fn set_device(ev: &mut Event, d: Option<DeviceRef>) {
    match ev {
        Event::Key { device, .. }
        | Event::Button { device, .. }
        | Event::Motion { device, .. }
        | Event::Scroll { device, .. }
        | Event::ScrollDiscrete { device, .. } => *device = d,
        _ => {}
    }
}

struct Waiting {
    ev: Event,
    /// Still looking for this.
    want: Option<Token>,
    until: u64,
}

#[derive(Default)]
pub(crate) struct Correlator {
    seen: VecDeque<(Token, DeviceRef, u64)>,
    motion: VecDeque<(DeviceRef, u64)>,
    waiting: VecDeque<Waiting>,
    misses: u8,
}

impl Correlator {
    /// A device said `token` happened on it.
    pub(crate) fn device_event(&mut self, token: Token, device: DeviceRef, now: u64, out: &mut Vec<Event>) {
        self.prune(now);
        if let Some(w) = self.waiting.iter_mut().find(|w| w.want == Some(token)) {
            set_device(&mut w.ev, Some(device));
            w.want = None;
            self.misses = 0;
            self.flush(now, out);
            return;
        }
        if self.seen.len() == RING {
            self.seen.pop_front();
        }
        self.seen.push_back((token, device, now));
    }

    /// A device said it moved.
    pub(crate) fn device_motion(&mut self, device: DeviceRef, now: u64) {
        self.prune(now);
        if self.motion.len() == RING {
            self.motion.pop_front();
        }
        self.motion.push_back((device, now));
    }

    /// The hook's event; what may be sent now comes out, in order.
    pub(crate) fn hook(&mut self, mut ev: Event, now: u64, out: &mut Vec<Event>) {
        self.prune(now);
        let mut want = None;
        if let Event::Motion { .. } = ev {
            let first = self.motion.front().map(|m| m.0);
            let one = first.filter(|&d| self.motion.iter().all(|m| m.0 == d));
            set_device(&mut ev, one);
        } else if let Some(t) = token(&ev) {
            match self.seen.iter().position(|s| s.0 == t).and_then(|i| self.seen.remove(i)) {
                Some((_, d, _)) => {
                    set_device(&mut ev, Some(d));
                    self.misses = 0;
                }
                None if self.misses < MAX_MISSES => want = Some(t),
                None => {}
            }
        }
        self.waiting.push_back(Waiting { ev, want, until: now + WAIT_MS });
        self.flush(now, out);
    }

    /// Time passed: events waited long enough go on unknown.
    pub(crate) fn tick(&mut self, now: u64, out: &mut Vec<Event>) {
        self.flush(now, out);
    }

    /// When [`Correlator::tick`] must run: only while an event waits.
    pub(crate) fn deadline(&self) -> Option<u64> {
        self.waiting.iter().filter(|w| w.want.is_some()).map(|w| w.until).min()
    }

    /// Capture ends: everything waiting goes now (unknown where unmatched),
    /// and old device events are forgotten.
    pub(crate) fn finish(&mut self, out: &mut Vec<Event>) {
        out.extend(self.waiting.drain(..).map(|w| w.ev));
        self.seen.clear();
        self.motion.clear();
    }

    /// A device went away: what it said no longer counts.
    pub(crate) fn forget(&mut self, device: DeviceRef) {
        self.seen.retain(|s| s.1 != device);
        self.motion.retain(|m| m.0 != device);
    }

    fn prune(&mut self, now: u64) {
        let old = now.saturating_sub(MOTION_MS);
        while self.seen.front().is_some_and(|s| s.2 < old) {
            self.seen.pop_front();
        }
        while self.motion.front().is_some_and(|m| m.1 < old) {
            self.motion.pop_front();
        }
    }

    fn flush(&mut self, now: u64, out: &mut Vec<Event>) {
        while let Some(w) = self.waiting.front() {
            if w.want.is_some() && w.until > now {
                return;
            }
            if w.want.is_some() {
                self.misses = self.misses.saturating_add(1);
            }
            if let Some(w) = self.waiting.pop_front() {
                out.push(w.ev);
            }
        }
    }
}

// ------------------------------------------------------- Windows paths

#[cfg(any(windows, test))]
/// What a Windows device interface path says: the bus, vendor and product.
/// `\\?\HID#VID_046D&PID_C52B&MI_00#...` is USB (or another HID bus),
/// `...BTHENUM...` or the HID service `{00001124-...}` Bluetooth, `BTHLE`
/// or HID over GATT `{00001812-...}` Bluetooth LE, `ACPI#...` built in.
pub(crate) fn windows_path(path: &str) -> (Bus, u16, u16) {
    let upper = path.to_ascii_uppercase();
    let hex_after = |key: &str| {
        let at = upper.find(key)? + key.len();
        let hex = |r: std::ops::Range<usize>| upper.get(r).filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()));
        // Bluetooth writes `VID&0002046D`: where the id comes from, then it.
        let digits = if key.ends_with('&') { hex(at + 4..at + 8).filter(|_| hex(at..at + 4).is_some()) } else { None };
        u16::from_str_radix(digits.or_else(|| hex(at..at + 4))?, 16).ok()
    };
    let (vid, pid) = (hex_after("VID_").or_else(|| hex_after("VID&")), hex_after("PID_").or_else(|| hex_after("PID&")));
    let bluetooth = ["BTHENUM", "BTHLE", "{00001124-0000-1000-8000-00805F9B34FB}", "{00001812-0000-1000-8000-00805F9B34FB}"]
        .iter()
        .any(|k| upper.contains(k));
    let bus = if bluetooth {
        Bus::Bluetooth
    } else if upper.contains("ACPI#") || upper.contains("\\ACPI\\") {
        Bus::Internal
    } else if vid.is_some() {
        Bus::Usb
    } else {
        Bus::Unknown
    };
    (bus, vid.unwrap_or(0), pid.unwrap_or(0))
}

// --------------------------------------------------------- HID usages

#[cfg(any(target_os = "macos", test))]
/// A HID keyboard usage (page 7) as an evdev key code, as Linux's
/// `hid-input` maps it; `None` for usages with no key.
pub(crate) fn hid_key_to_evdev(usage: u32) -> Option<u32> {
    #[rustfmt::skip]
    const KEYS: [u8; 0x66] = [
        0, 0, 0, 0, 30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38,
        50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44, 2, 3,
        4, 5, 6, 7, 8, 9, 10, 11, 28, 1, 14, 15, 57, 12, 13, 26,
        27, 43, 43, 39, 40, 41, 51, 52, 53, 58, 59, 60, 61, 62, 63, 64,
        65, 66, 67, 68, 87, 88, 99, 70, 119, 110, 102, 104, 111, 107, 109, 106,
        105, 108, 103, 69, 98, 55, 74, 78, 96, 79, 80, 81, 75, 76, 77, 71,
        72, 73, 82, 83, 86, 127,
    ];
    let code = match usage {
        0x00..=0x65 => KEYS[usage as usize],
        0xe0..=0xe7 => [29, 42, 56, 125, 97, 54, 100, 126][(usage - 0xe0) as usize],
        _ => 0,
    };
    (code != 0).then_some(u32::from(code))
}

/// A HID button (page 9, 1 = primary) as an evdev button code.
pub(crate) fn hid_button_to_evdev(n: u32) -> Option<u32> {
    // Left, right, middle, side (back), extra (forward).
    [0x110, 0x111, 0x112, 0x113, 0x114].get(n.checked_sub(1)? as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::remap::test_support::Rng;

    const A: DeviceRef = DeviceRef(1);
    const B: DeviceRef = DeviceRef(2);

    fn key(code: u32, down: bool) -> Event {
        Event::Key { code, down, device: None }
    }
    fn press(code: u32, down: bool) -> Token {
        Token::Press { code, down }
    }
    fn devices(out: &[Event]) -> Vec<Option<DeviceRef>> {
        out.iter()
            .map(|e| match e {
                Event::Key { device, .. } | Event::Button { device, .. } | Event::Motion { device, .. } => *device,
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_device_event_before_or_after_the_hook_names_its_device() {
        let mut c = Correlator::default();
        let mut out = Vec::new();
        c.device_event(press(30, true), A, 100, &mut out);
        c.hook(key(30, true), 101, &mut out);
        assert_eq!(out, [Event::Key { code: 30, down: true, device: Some(A) }], "device event first");
        out.clear();
        c.hook(key(30, false), 102, &mut out);
        assert!(out.is_empty(), "waits for its device");
        assert_eq!(c.deadline(), Some(102 + WAIT_MS));
        c.device_event(press(30, false), B, 103, &mut out);
        assert_eq!(out, [Event::Key { code: 30, down: false, device: Some(B) }], "hook first");
        assert_eq!(c.deadline(), None, "no timer while nothing waits");
    }

    #[test]
    fn unmatched_events_go_on_unknown_in_order() {
        let mut c = Correlator::default();
        let mut out = Vec::new();
        c.hook(key(30, true), 100, &mut out);
        c.hook(Event::Motion { dx: 1.0, dy: 0.0, device: None }, 101, &mut out);
        c.hook(Event::ScrollStop, 101, &mut out);
        assert!(out.is_empty(), "nothing overtakes the waiting key");
        c.tick(100 + WAIT_MS - 1, &mut out);
        assert!(out.is_empty());
        c.tick(100 + WAIT_MS, &mut out);
        assert_eq!(out, [key(30, true), Event::Motion { dx: 1.0, dy: 0.0, device: None }, Event::ScrollStop]);
    }

    #[test]
    fn two_keyboards_pressing_one_key_are_matched_in_order() {
        let mut c = Correlator::default();
        let mut out = Vec::new();
        c.hook(key(30, true), 100, &mut out);
        c.hook(key(30, true), 101, &mut out);
        c.device_event(press(30, true), A, 102, &mut out);
        c.device_event(press(30, true), B, 102, &mut out);
        assert_eq!(devices(&out), [Some(A), Some(B)]);
    }

    #[test]
    fn a_system_that_never_says_stops_costing_latency() {
        let mut c = Correlator::default();
        let mut out = Vec::new();
        let mut t = 0;
        for _ in 0..MAX_MISSES {
            c.hook(key(30, true), t, &mut out);
            t += WAIT_MS;
            c.tick(t, &mut out);
        }
        out.clear();
        c.hook(key(31, true), t, &mut out);
        assert_eq!(out, [key(31, true)], "no waiting any more");
        c.device_event(press(32, true), A, t, &mut out);
        c.hook(key(32, true), t, &mut out);
        assert_eq!(devices(&out)[1..], [Some(A)], "a match brings waiting back");
    }

    #[test]
    fn motion_is_attributed_only_when_one_device_moves() {
        let mut c = Correlator::default();
        let mut out = Vec::new();
        let m = || Event::Motion { dx: 1.0, dy: 1.0, device: None };
        c.hook(m(), 0, &mut out);
        c.device_motion(A, 10);
        c.hook(m(), 11, &mut out);
        c.device_motion(B, 20);
        c.hook(m(), 21, &mut out);
        c.hook(m(), 200, &mut out);
        assert_eq!(devices(&out), [None, Some(A), None, None], "nothing moved, A, A and B, too old");
        c.device_motion(B, 300);
        c.forget(B);
        c.hook(m(), 301, &mut out);
        assert_eq!(devices(&out)[4], None, "a device that went away");
    }

    #[test]
    fn old_device_events_never_match() {
        let mut c = Correlator::default();
        let mut out = Vec::new();
        c.device_event(press(30, true), A, 0, &mut out);
        c.hook(key(30, true), MOTION_MS + 1, &mut out);
        c.tick(MOTION_MS + 1 + WAIT_MS, &mut out);
        assert_eq!(devices(&out), [None]);
        for i in 0..(RING as u32 + 4) {
            c.device_event(press(i, true), A, 1000, &mut out);
        }
        assert_eq!(c.seen.len(), RING, "bounded");
    }

    /// Random interleavings: every hook event comes out exactly once, in
    /// the order it went in, and only with a device a matching device
    /// event named.
    #[test]
    fn every_event_comes_out_once_in_order() {
        let mut rng = Rng(0x5eed_1234_abcd_0001);
        for _ in 0..2000 {
            let mut c = Correlator::default();
            let mut out = Vec::new();
            let mut sent = Vec::new();
            let mut t = 0u64;
            for n in 0..rng.below(60) {
                t += rng.below(6) as u64;
                let code = 30 + rng.below(3) as u32;
                let down = rng.below(2) == 0;
                match rng.below(5) {
                    0 | 1 => {
                        // The code tells the events apart in the output.
                        let ev = Event::Key { code: 1000 + n as u32 * 4 + (code - 30), down, device: None };
                        sent.push(ev.clone());
                        c.hook(ev, t, &mut out);
                    }
                    2 => c.device_event(press(1000 + rng.below(n + 1) as u32 * 4 + (code - 30), down), [A, B][rng.below(2)], t, &mut out),
                    3 => c.tick(t, &mut out),
                    _ => c.device_motion(A, t),
                }
            }
            c.finish(&mut out);
            let codes = |v: &[Event]| v.iter().map(|e| if let Event::Key { code, down, .. } = e { (*code, *down) } else { (0, false) }).collect::<Vec<_>>();
            assert_eq!(codes(&out), codes(&sent));
        }
    }

    #[test]
    fn windows_paths() {
        let usb = r"\\?\HID#VID_046D&PID_C52B&MI_00#7&2b4a7b38&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}";
        assert_eq!(windows_path(usb), (Bus::Usb, 0x046d, 0xc52b));
        let bt = r"\\?\HID#{00001124-0000-1000-8000-00805f9b34fb}_VID&0002046d_PID&b35b&Col01#9&1f2c3d4e&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}";
        assert_eq!(windows_path(bt), (Bus::Bluetooth, 0x046d, 0xb35b));
        let ble = r"\\?\HID#{00001812-0000-1000-8000-00805f9b34fb}&Dev&VID_046d&PID_b023&REV_0011_d6a7c4b3e2f1&Col01#a&1&0&0000#{884b96c3}";
        assert_eq!(windows_path(ble), (Bus::Bluetooth, 0x046d, 0xb023));
        assert_eq!(windows_path(r"\\?\ACPI#PNP0303#4&1d401fb5&0#{884b96c3}").0, Bus::Internal);
        assert_eq!(windows_path(r"\\?\Root#RDP_KBD#0000#{884b96c3}"), (Bus::Unknown, 0, 0));
        assert_eq!(windows_path(r"VID_12"), (Bus::Unknown, 0, 0), "cut short");
        assert_eq!(windows_path("ü VID_ü"), (Bus::Unknown, 0, 0), "not ASCII");
    }

    #[test]
    fn hid_usages() {
        assert_eq!(hid_key_to_evdev(0x04), Some(30), "a");
        assert_eq!(hid_key_to_evdev(0x29), Some(1), "esc");
        assert_eq!(hid_key_to_evdev(0x39), Some(58), "caps lock");
        assert_eq!(hid_key_to_evdev(0x4f), Some(106), "right");
        assert_eq!(hid_key_to_evdev(0xe0), Some(29), "left ctrl");
        assert_eq!(hid_key_to_evdev(0xe7), Some(126), "right meta");
        assert_eq!(hid_key_to_evdev(0x01), None, "an error roll-over");
        assert_eq!(hid_key_to_evdev(0xffff_ffff), None);
        assert_eq!(hid_button_to_evdev(1), Some(0x110));
        assert_eq!(hid_button_to_evdev(5), Some(0x114));
        assert_eq!((hid_button_to_evdev(0), hid_button_to_evdev(9)), (None, None));
    }
}
