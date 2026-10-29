//! Sleep sync (`sleep_sync` in settings): when this computer's display
//! goes to sleep, or it suspends, connected computers that also have sleep
//! sync on put their displays to sleep too.
//!
//! It is lock sync again with another verb, so it is [`LockSync`] with the
//! "all" (`lock-all`) case taken out: every event has a random id, a
//! computer that sleeps because of a peer passes the same id on and never
//! announces its own sleep as a new event, and ids seen before are dropped.
//! A sleep so spreads over every link once and stops, whatever the links
//! and however late each OS reports its display off.
//!
//! Waking is not synced: any input wakes a display, and the pointer
//! arriving on a computer is input.

use crate::lock::LockSync;
use std::time::Instant;

/// What to do with a `Sleep { id }` from a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnRemoteSleep {
    /// Seen before, or sleep sync is off here: drop it.
    Ignore,
    /// Pass `id` on to the other peers, and put this display to sleep
    /// unless it already is.
    Sleep { sleep_here: bool },
}

#[derive(Clone, Debug, Default)]
pub struct SleepSync(LockSync);

impl SleepSync {
    /// A peer asks us to sleep. `sync` is this computer's `sleep_sync`
    /// setting.
    pub fn on_remote(&mut self, id: u64, sync: bool, now: Instant) -> OnRemoteSleep {
        match self.0.on_remote(id, false, sync, now) {
            crate::OnRemote::Ignore => OnRemoteSleep::Ignore,
            crate::OnRemote::Lock { lock_here, .. } => OnRemoteSleep::Sleep { sleep_here: lock_here },
        }
    }

    /// The OS says this display just went to sleep (or the computer is
    /// suspending). Returns an id to send to every peer if sleep sync is on
    /// and nobody asked for this sleep. `fresh` is a new random id.
    pub fn on_local_asleep(&mut self, sync: bool, fresh: u64, now: Instant) -> Option<u64> {
        self.0.on_local_locked(sync, fresh, now)
    }

    pub fn on_local_awake(&mut self) {
        self.0.on_local_unlocked();
    }

    pub fn is_asleep(&self) -> bool {
        self.0.is_locked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CAUSE_WINDOW;

    #[test]
    fn a_peers_sleep_is_followed_once_and_never_announced_back() {
        let t = Instant::now();
        let mut b = SleepSync::default();
        assert_eq!(b.on_remote(7, true, t), OnRemoteSleep::Sleep { sleep_here: true });
        // B's display reports off a moment later: that is the peer's doing.
        assert_eq!(b.on_local_asleep(true, 99, t + CAUSE_WINDOW / 2), None);
        assert!(b.is_asleep());
        // The same event over another link is dropped, even once awake.
        b.on_local_awake();
        assert_eq!(b.on_remote(7, true, t), OnRemoteSleep::Ignore);
        // The user sleeping it by hand later is a new event.
        assert_eq!(b.on_local_asleep(true, 100, t + CAUSE_WINDOW * 2), Some(100));
    }

    #[test]
    fn sleep_sync_off_neither_follows_nor_announces() {
        let t = Instant::now();
        let mut c = SleepSync::default();
        assert_eq!(c.on_remote(1, false, t), OnRemoteSleep::Ignore);
        assert_eq!(c.on_local_asleep(false, 2, t), None);
        assert_eq!(c.on_remote(1, true, t), OnRemoteSleep::Ignore, "stays dropped");
    }

    #[test]
    fn already_asleep_is_still_passed_on() {
        let t = Instant::now();
        let mut s = SleepSync::default();
        assert_eq!(s.on_local_asleep(true, 1, t), Some(1));
        assert_eq!(s.on_remote(5, true, t), OnRemoteSleep::Sleep { sleep_here: false });
    }

    /// Three computers all linked: A's user sleeps it; B and C follow
    /// through their own links, pass it on, and nothing comes back to A as new.
    #[test]
    fn spreads_over_a_triangle_once() {
        let t = Instant::now();
        let mut n = [SleepSync::default(), SleepSync::default(), SleepSync::default()];
        let mut queue = std::collections::VecDeque::new();
        let mut sent = 0;
        let mut slept = [0; 3];
        let mut next_id = 100;
        let links = |i: usize| (0..3).filter(move |&j| j != i);
        // A (0) sleeps by hand.
        slept[0] += 1;
        let id = n[0].on_local_asleep(true, 50, t).unwrap();
        for p in links(0) {
            queue.push_back((p, 0, id));
            sent += 1;
        }
        while let Some((to, from, id)) = queue.pop_front() {
            if let OnRemoteSleep::Sleep { sleep_here } = n[to].on_remote(id, true, t) {
                if sleep_here {
                    slept[to] += 1;
                    // The OS reports it, and it counts as the peer's doing.
                    next_id += 1;
                    assert_eq!(n[to].on_local_asleep(true, next_id, t), None);
                }
                for p in links(to).filter(|&p| p != from) {
                    queue.push_back((p, to, id));
                    sent += 1;
                }
            }
        }
        assert_eq!(slept, [1, 1, 1]);
        assert!(sent <= 2 + 2 + 2, "{sent} messages");
    }
}
