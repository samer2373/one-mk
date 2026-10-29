//! Platform-independent pieces shared by the backends: coalescing queued
//! injection, receiver-side key auto-repeat, the `LocalInput` throttle and
//! reconnect backoff. Nothing here touches an OS API, so it is unit-tested
//! on every OS.

// Each backend uses a different subset.
#![allow(dead_code)]

use crate::Inject;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// ------------------------------------------------------------- coalescing

/// Appends `batch` to `out`, dropping stale moves: only the last `MoveTo`
/// before each key/button/scroll "barrier" (and the last one at the end) is
/// kept, so every barrier still happens at the position that preceded it,
/// in order, while a burst of queued moves costs one move. A run of
/// relative moves becomes one move by their sum (none of it is dropped).
pub(crate) fn coalesce(batch: impl IntoIterator<Item = Inject>, out: &mut Vec<Inject>) {
    let mut pending = None;
    for m in batch {
        match (m, &mut pending) {
            (Inject::MoveTo(_), Some(Inject::MoveBy { .. })) => {
                out.extend(pending.take());
                pending = Some(m);
            }
            (Inject::MoveTo(_), _) => pending = Some(m),
            (Inject::MoveBy { dx, dy }, Some(Inject::MoveBy { dx: px, dy: py })) => {
                *px = px.saturating_add(dx);
                *py = py.saturating_add(dy);
            }
            (Inject::MoveBy { .. }, _) => {
                out.extend(pending.take());
                pending = Some(m);
            }
            _ => {
                out.extend(pending.take());
                out.push(m);
            }
        }
    }
    out.extend(pending);
}

/// `first` plus everything already queued in `rx`, coalesced into `out`.
pub(crate) fn drain_coalesced(first: Inject, rx: &mut mpsc::UnboundedReceiver<Inject>, out: &mut Vec<Inject>) {
    coalesce(std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok())), out);
}

// ------------------------------------------------------------- key repeat

const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const KEY_LEFTALT: u32 = 56;
const KEY_CAPSLOCK: u32 = 58;
const KEY_NUMLOCK: u32 = 69;
const KEY_SCROLLLOCK: u32 = 70;
const KEY_RIGHTCTRL: u32 = 97;
const KEY_RIGHTALT: u32 = 100;
const KEY_LEFTMETA: u32 = 125;
const KEY_RIGHTMETA: u32 = 126;
const KEY_FN: u32 = 464;

/// Modifiers and lock keys never auto-repeat.
pub(crate) fn is_repeatable(code: u32) -> bool {
    !matches!(
        code,
        KEY_LEFTCTRL
            | KEY_LEFTSHIFT
            | KEY_RIGHTSHIFT
            | KEY_LEFTALT
            | KEY_CAPSLOCK
            | KEY_NUMLOCK
            | KEY_SCROLLLOCK
            | KEY_RIGHTCTRL
            | KEY_RIGHTALT
            | KEY_LEFTMETA
            | KEY_RIGHTMETA
            | KEY_FN
    )
}

/// Receiver-side key auto-repeat. Senders drop their OS's repeats (they
/// would arrive late and bunched up after a network hiccup), and injection
/// APIs like `SendInput` and `CGEventPost` don't repeat by themselves, so
/// the receiving machine repeats the most recently pressed key with its own
/// delay and rate, like a local keyboard.
#[derive(Debug)]
pub(crate) struct KeyRepeat {
    /// Keys held down by the peer, in press order.
    held: Vec<u32>,
    /// The key being repeated and when it next fires.
    repeating: Option<(u32, Instant)>,
    /// `None` when the user turned key repeat off.
    timing: Option<(Duration, Duration)>,
}

impl KeyRepeat {
    pub(crate) fn new(timing: Option<(Duration, Duration)>) -> Self {
        Self { held: Vec::new(), repeating: None, timing }
    }

    /// `(delay, interval)`, or `None` to not repeat at all. Applies from
    /// the next key press.
    pub(crate) fn set_timing(&mut self, timing: Option<(Duration, Duration)>) {
        self.timing = timing.map(|(d, i)| (d, i.max(Duration::from_millis(5))));
    }

    /// Records a key event from the peer. Returns false if it must not be
    /// injected: a second "down" for a key already held is an auto-repeat
    /// the sender should have dropped (repeating Caps Lock would toggle it
    /// again and again).
    pub(crate) fn key(&mut self, code: u32, down: bool, now: Instant) -> bool {
        if down {
            if self.held.contains(&code) {
                return false;
            }
            self.held.push(code);
            if is_repeatable(code)
                && let Some((delay, _)) = self.timing
            {
                self.repeating = Some((code, now + delay));
            }
        } else {
            self.held.retain(|&k| k != code);
            if self.repeating.is_some_and(|(k, _)| k == code) {
                self.repeating = None;
            }
        }
        true
    }

    /// When `poll` next has something to do.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.repeating.map(|(_, t)| t)
    }

    /// The key to send another "down" for, if one is due at `now`.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<u32> {
        let (code, due) = self.repeating?;
        let (_, interval) = self.timing?;
        if now < due {
            return None;
        }
        // If we fell behind (a stall), don't fire a burst to catch up.
        let next = if now - due > interval { now + interval } else { due + interval };
        self.repeating = Some((code, next));
        Some(code)
    }

    /// Forgets everything (the peer let go of this machine).
    pub(crate) fn clear(&mut self) {
        self.held.clear();
        self.repeating = None;
    }
}

// ------------------------------------------------------------ local input

/// At most one `Event::LocalInput` per this many milliseconds.
pub(crate) const LOCAL_INPUT_EVERY_MS: u64 = 100;

/// Milliseconds since the first call; never 0.
pub(crate) fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Whether a peer controls this machine right now, exactly as the daemon
/// says with `Inject::Controlled`. Set by the injection thread, read by the
/// capture side, which reports local input only while it is true. There is
/// no timeout: however long the peer stays idle, touching this machine's
/// own mouse or keyboard still takes control back.
#[derive(Debug, Default)]
pub(crate) struct Controlled(AtomicBool);

impl Controlled {
    pub(crate) fn set(&self, controlled: bool) {
        self.0.store(controlled, Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Applies and removes the `Inject::Controlled` messages in `batch`.
    /// Returns true if control ended, so the caller can forget key repeat.
    pub(crate) fn apply(&self, batch: &mut Vec<Inject>) -> bool {
        let mut ended = false;
        batch.retain(|m| match *m {
            Inject::Controlled(c) => {
                self.set(c);
                ended |= !c;
                false
            }
            _ => true,
        });
        ended
    }
}

/// Decides when real local input becomes an `Event::LocalInput`.
#[derive(Debug, Default)]
pub(crate) struct LocalInputGate {
    last_emit: Option<u64>,
}

impl LocalInputGate {
    /// Real input happened at `now`; `controlled` is `Controlled::get`.
    pub(crate) fn check(&mut self, now: u64, controlled: bool) -> bool {
        if !controlled || self.last_emit.is_some_and(|t| now.saturating_sub(t) < LOCAL_INPUT_EVERY_MS) {
            return false;
        }
        self.last_emit = Some(now);
        true
    }
}

// ---------------------------------------------------------------- backoff

/// Delays between attempts to re-create a lost session: 1 s, 2 s, 4 s,
/// 8 s, 16 s, then every 30 s.
#[derive(Debug, Default)]
pub(crate) struct Backoff {
    step: usize,
}

impl Backoff {
    const STEPS: [u64; 6] = [1, 2, 4, 8, 16, 30];

    pub(crate) fn next_delay(&mut self) -> Duration {
        let s = Self::STEPS[self.step.min(Self::STEPS.len() - 1)];
        self.step += 1;
        Duration::from_secs(s)
    }

    pub(crate) fn reset(&mut self) {
        self.step = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::Pos;

    fn mv(x: f64) -> Inject {
        Inject::MoveTo(Pos::new(x, 0.0))
    }

    #[test]
    fn coalesce_keeps_last_move_before_each_barrier() {
        let click = Inject::Button { button: 0x110, down: true };
        let key = Inject::Key { code: 30, down: true };
        let mut out = Vec::new();
        coalesce([mv(1.0), mv(2.0), mv(3.0), click, mv(4.0), key, Inject::ScrollStop, mv(5.0), mv(6.0)], &mut out);
        assert_eq!(out, vec![mv(3.0), click, mv(4.0), key, Inject::ScrollStop, mv(6.0)]);

        out.clear();
        coalesce([mv(1.0), mv(2.0)], &mut out);
        assert_eq!(out, vec![mv(2.0)]);

        out.clear();
        coalesce([click, click], &mut out);
        assert_eq!(out, vec![click, click]);

        // Relative moves add up; an absolute move before them still goes
        // first, and one after them replaces nothing that came before.
        let by = |dx, dy| Inject::MoveBy { dx, dy };
        out.clear();
        coalesce([mv(1.0), by(1, 2), by(3, -4), click, by(5, 5), mv(2.0), mv(3.0), by(i32::MAX, 0), by(1, 0)], &mut out);
        assert_eq!(out, vec![mv(1.0), by(4, -2), click, by(5, 5), mv(3.0), by(i32::MAX, 0)]);
    }

    #[test]
    fn drain_coalesces_the_queue() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        for m in [mv(2.0), mv(3.0), Inject::ScrollStop, mv(4.0)] {
            tx.send(m).unwrap();
        }
        let mut out = Vec::new();
        drain_coalesced(mv(1.0), &mut rx, &mut out);
        assert_eq!(out, vec![mv(3.0), Inject::ScrollStop, mv(4.0)]);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn repeat_fires_after_delay_then_at_rate() {
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        let mut r = KeyRepeat::new(None);
        r.set_timing(Some((ms(500), ms(30))));
        assert!(r.key(30, true, t0));
        assert_eq!(r.deadline(), Some(t0 + ms(500)));
        assert_eq!(r.poll(t0 + ms(499)), None);
        assert_eq!(r.poll(t0 + ms(500)), Some(30));
        assert_eq!(r.deadline(), Some(t0 + ms(530)));
        // A long stall does not cause a burst.
        assert_eq!(r.poll(t0 + ms(900)), Some(30));
        assert_eq!(r.deadline(), Some(t0 + ms(930)));
        // A newer key takes over; releasing the older one doesn't stop it.
        assert!(r.key(31, true, t0 + ms(1000)));
        assert!(r.key(30, false, t0 + ms(1001)));
        assert_eq!(r.deadline(), Some(t0 + ms(1500)));
        assert!(r.key(31, false, t0 + ms(1600)));
        assert_eq!(r.deadline(), None);
        assert_eq!(r.poll(t0 + ms(5000)), None);
    }

    #[test]
    fn modifiers_and_locks_do_not_repeat_and_duplicates_are_dropped() {
        let t0 = Instant::now();
        let mut r = KeyRepeat::new(None);
        r.set_timing(Some((Duration::from_millis(300), Duration::from_millis(30))));
        for code in [KEY_LEFTSHIFT, KEY_CAPSLOCK, KEY_NUMLOCK, KEY_LEFTMETA, KEY_FN] {
            assert!(r.key(code, true, t0));
            assert_eq!(r.deadline(), None, "key {code}");
            // A repeated Caps Lock "down" must not toggle it again.
            assert!(!r.key(code, true, t0));
            assert!(r.key(code, false, t0));
        }
        // Shift held while a letter repeats keeps it repeating.
        assert!(r.key(30, true, t0));
        assert!(r.key(KEY_LEFTSHIFT, true, t0));
        assert!(r.deadline().is_some());
        // Repeat turned off.
        let mut off = KeyRepeat::new(None);
        assert!(off.key(30, true, t0));
        assert_eq!(off.deadline(), None);
        r.clear();
        assert_eq!(r.deadline(), None);
        assert!(r.key(30, true, t0));
    }

    #[test]
    fn local_input_only_while_controlled_and_throttled() {
        let mut g = LocalInputGate::default();
        assert!(!g.check(1000, false));
        assert!(g.check(1000, true));
        assert!(!g.check(1050, true));
        assert!(g.check(1100, true));
        // No timeout: an hour later it still counts while controlled.
        assert!(g.check(1100 + 3_600_000, true));
        assert!(!g.check(1100 + 7_200_000, false));
        assert!(now_ms() >= 1);
    }

    #[test]
    fn controlled_follows_the_daemon() {
        let c = Controlled::default();
        assert!(!c.get());
        let key = Inject::Key { code: 30, down: false };
        let mut batch = vec![Inject::Controlled(true), mv(1.0)];
        assert!(!c.apply(&mut batch));
        assert!(c.get());
        assert_eq!(batch, vec![mv(1.0)]);
        // Released keys first, then control ends.
        let mut batch = vec![key, Inject::Controlled(false)];
        assert!(c.apply(&mut batch));
        assert!(!c.get());
        assert_eq!(batch, vec![key]);
    }

    #[test]
    fn backoff_steps() {
        let mut b = Backoff::default();
        let secs: Vec<u64> = (0..8).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 30, 30, 30]);
        b.reset();
        assert_eq!(b.next_delay().as_secs(), 1);
    }
}
