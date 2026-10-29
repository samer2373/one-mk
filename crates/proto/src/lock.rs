//! Lock sync: `one-mk lock-all`, and (with `lock-sync on`) locking one
//! computer locks the others.
//!
//! `lock-all` is obeyed by every connected computer: the user asked for
//! exactly that. A lock spread by lock sync is followed only by computers
//! with lock sync on: turning it off on a computer means its screen is
//! not tied to the others', either way.
//!
//! Every lock event has a random id. A computer that locks because of a
//! peer remembers the id and passes the same id on, never a new one, and
//! drops ids it has seen. So a lock spreads over every link once and
//! stops: A locks, B and C lock, and nothing comes back to A, whatever the
//! links and however late each OS reports its own lock.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A lock this computer reports within this long after locking because of
/// a peer (or `lock-all`) is that lock, not the user locking it again.
pub const CAUSE_WINDOW: Duration = Duration::from_secs(15);
/// Lock ids remembered.
const SEEN: usize = 32;

/// What to do with a `Lock { id }` from a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnRemote {
    /// Seen before: drop it.
    Ignore,
    /// Lock this screen (unless it already is), and pass `id` on to the
    /// other peers if `forward`.
    Lock { lock_here: bool, forward: bool },
}

#[derive(Clone, Debug, Default)]
pub struct LockSync {
    seen: VecDeque<u64>,
    /// We locked (or are about to) because of this event, at this time.
    cause: Option<Instant>,
    /// The OS said this screen is locked.
    locked: bool,
}

impl LockSync {
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    fn remember(&mut self, id: u64) -> bool {
        if self.seen.contains(&id) {
            return false;
        }
        if self.seen.len() == SEEN {
            self.seen.pop_front();
        }
        self.seen.push_back(id);
        true
    }

    /// A peer asks us to lock: its `lock-all` if `all`, else its lock
    /// sync. `sync` is this computer's `lock-sync` setting: with it off,
    /// only a lock-all is obeyed; with it on, the request also goes on to
    /// the other peers (a lock from a computer not paired with all of ours).
    pub fn on_remote(&mut self, id: u64, all: bool, sync: bool, now: Instant) -> OnRemote {
        // Remembered either way, so a copy arriving over another link is
        // dropped too.
        if !self.remember(id) || !(all || sync) {
            return OnRemote::Ignore;
        }
        if !self.locked {
            self.cause = Some(now);
        }
        OnRemote::Lock { lock_here: !self.locked, forward: sync }
    }

    /// `one-mk lock-all` here, with a fresh `id`: send it to every peer.
    pub fn lock_all(&mut self, id: u64, now: Instant) -> u64 {
        self.remember(id);
        self.cause = Some(now);
        id
    }

    /// The OS says this screen just locked. Returns an id to send to every
    /// peer if lock sync is on and nobody asked for this lock (the user
    /// locked this computer). `fresh` is a new random id.
    pub fn on_local_locked(&mut self, sync: bool, fresh: u64, now: Instant) -> Option<u64> {
        if std::mem::replace(&mut self.locked, true) {
            return None; // told twice
        }
        let asked = self.cause.take().is_some_and(|t| now.saturating_duration_since(t) < CAUSE_WINDOW);
        (sync && !asked && self.remember(fresh)).then_some(fresh)
    }

    pub fn on_local_unlocked(&mut self) {
        self.locked = false;
        self.cause = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Computers linked as `links` (pairs of indexes), each with lock sync
    /// `sync[i]`. Every lock call makes that computer report "locked",
    /// after `lag` other deliveries. Returns how many `Lock` messages were
    /// sent, once nothing is left to deliver.
    struct Net {
        sync: Vec<LockSync>,
        on: Vec<bool>,
        links: Vec<(usize, usize)>,
        /// (to, from, id, all) messages, and (computer, steps left) lock reports.
        queue: VecDeque<(usize, usize, u64, bool)>,
        reports: Vec<(usize, usize)>,
        lag: usize,
        sent: usize,
        locks: Vec<usize>,
        next_id: u64,
    }

    impl Net {
        fn new(n: usize, links: &[(usize, usize)], on: &[bool], lag: usize) -> Self {
            Net {
                sync: vec![LockSync::default(); n],
                on: on.to_vec(),
                links: links.to_vec(),
                queue: VecDeque::new(),
                reports: Vec::new(),
                lag,
                sent: 0,
                locks: vec![0; n],
                next_id: 100,
            }
        }

        fn peers(&self, i: usize) -> Vec<usize> {
            self.links
                .iter()
                .filter_map(|&(a, b)| {
                    if a == i {
                        Some(b)
                    } else if b == i {
                        Some(a)
                    } else {
                        None
                    }
                })
                .collect()
        }

        fn send(&mut self, from: usize, id: u64, all: bool, except: Option<usize>) {
            for p in self.peers(from) {
                if Some(p) != except {
                    self.queue.push_back((p, from, id, all));
                    self.sent += 1;
                }
            }
        }

        fn lock_screen(&mut self, i: usize) {
            self.locks[i] += 1;
            self.reports.push((i, self.lag));
        }

        fn fresh(&mut self) -> u64 {
            self.next_id += 1;
            self.next_id
        }

        /// The user locks computer `i` by hand.
        fn user_locks(&mut self, i: usize) {
            self.lock_screen(i);
        }

        fn lock_all(&mut self, i: usize) {
            let id = self.fresh();
            let id = self.sync[i].lock_all(id, Instant::now());
            self.send(i, id, true, None);
            self.lock_screen(i);
        }

        fn tick_reports(&mut self, all: bool) {
            let mut due = Vec::new();
            self.reports.retain_mut(|(i, left)| {
                if all || *left == 0 {
                    due.push(*i);
                    false
                } else {
                    *left -= 1;
                    true
                }
            });
            for i in due {
                let fresh = self.fresh();
                if let Some(id) = self.sync[i].on_local_locked(self.on[i], fresh, Instant::now()) {
                    self.send(i, id, false, None);
                }
            }
        }

        fn run(&mut self) -> usize {
            for _ in 0..10_000 {
                self.tick_reports(false);
                match self.queue.pop_front() {
                    Some((to, from, id, all)) => match self.sync[to].on_remote(id, all, self.on[to], Instant::now()) {
                        OnRemote::Ignore => {}
                        OnRemote::Lock { lock_here, forward } => {
                            if lock_here {
                                self.lock_screen(to);
                            }
                            if forward {
                                self.send(to, id, all, Some(from));
                            }
                        }
                    },
                    None if self.reports.is_empty() => return self.sent,
                    None => self.tick_reports(true),
                }
            }
            panic!("lock messages never stopped: {} sent", self.sent);
        }
    }

    const ALL3: [(usize, usize); 3] = [(0, 1), (1, 2), (0, 2)];

    #[test]
    fn a_user_lock_spreads_once_and_stops() {
        for lag in [0, 1, 5, 50] {
            let mut net = Net::new(3, &ALL3, &[true; 3], lag);
            net.user_locks(0);
            let sent = net.run();
            assert!(net.sync.iter().all(LockSync::is_locked), "lag {lag}");
            assert_eq!(net.locks, vec![1, 1, 1], "each screen locked once (lag {lag})");
            // A tells B and C; each passes it to the other; nothing returns to A as new.
            assert!(sent <= 2 + 2 + 2, "{sent} messages (lag {lag})");
        }
    }

    #[test]
    fn lock_all_reaches_peers_without_lock_sync_and_does_not_echo() {
        let mut net = Net::new(3, &ALL3, &[false; 3], 3);
        net.lock_all(1);
        assert_eq!(net.run(), 2, "only the two requests");
        assert_eq!(net.locks, vec![1, 1, 1]);
        // With lock sync on everywhere, still no echo of the same lock.
        let mut net = Net::new(3, &ALL3, &[true; 3], 3);
        net.lock_all(1);
        net.run();
        assert_eq!(net.locks, vec![1, 1, 1]);
    }

    #[test]
    fn a_chain_is_crossed_when_the_middle_syncs() {
        // 0 - 1 - 2: 0 and 2 are not paired.
        let mut net = Net::new(3, &[(0, 1), (1, 2)], &[true, true, true], 2);
        net.user_locks(0);
        net.run();
        assert_eq!(net.locks, vec![1, 1, 1]);
        let mut net = Net::new(3, &[(0, 1), (1, 2)], &[true, true, false], 2);
        net.user_locks(0);
        net.run();
        assert_eq!(net.locks, vec![1, 1, 0], "2 doesn't follow without lock sync");
        let mut net = Net::new(3, &[(0, 1), (1, 2)], &[true, false, true], 2);
        net.user_locks(0);
        net.run();
        assert_eq!(net.locks, vec![1, 0, 0], "1 neither follows nor passes it on without lock sync");
        // A lock-all crosses the chain only where the middle syncs.
        let mut net = Net::new(3, &[(0, 1), (1, 2)], &[false, true, false], 2);
        net.lock_all(0);
        net.run();
        assert_eq!(net.locks, vec![1, 1, 1]);
        let mut net = Net::new(3, &[(0, 1), (1, 2)], &[false, false, false], 2);
        net.lock_all(0);
        net.run();
        assert_eq!(net.locks, vec![1, 1, 0]);
    }

    #[test]
    fn sync_off_neither_announces_nor_follows_but_obeys_lock_all() {
        let mut net = Net::new(2, &[(0, 1)], &[false, false], 0);
        net.user_locks(0);
        assert_eq!(net.run(), 0);
        assert_eq!(net.locks, vec![1, 0]);
        // 0 syncs, 1 doesn't: 1 stays unlocked when 0 locks...
        let mut net = Net::new(2, &[(0, 1)], &[true, false], 0);
        net.user_locks(0);
        assert_eq!(net.run(), 1);
        assert_eq!(net.locks, vec![1, 0]);
        // ...and 0 when 1 does, since 1 doesn't announce.
        net.user_locks(1);
        assert_eq!(net.run(), 1);
        assert_eq!(net.locks, vec![1, 1]);
        // But `lock-all` from either side locks both.
        let mut net = Net::new(2, &[(0, 1)], &[true, false], 0);
        net.lock_all(0);
        net.run();
        assert_eq!(net.locks, vec![1, 1]);
    }

    #[test]
    fn unlocking_and_locking_again_is_a_new_event() {
        let mut net = Net::new(2, &[(0, 1)], &[true, true], 1);
        net.user_locks(0);
        net.run();
        for s in &mut net.sync {
            s.on_local_unlocked();
        }
        net.user_locks(1);
        net.run();
        assert_eq!(net.locks, vec![2, 2]);
    }

    #[test]
    fn duplicate_reports_and_late_causes() {
        let t0 = Instant::now();
        let mut s = LockSync::default();
        assert_eq!(s.on_local_locked(true, 1, t0), Some(1));
        assert_eq!(s.on_local_locked(true, 2, t0), None, "reported twice");
        // Already locked when a peer asks: nothing to do here, still passed on.
        assert_eq!(s.on_remote(7, false, true, t0), OnRemote::Lock { lock_here: false, forward: true });
        assert_eq!(s.on_remote(7, false, true, t0), OnRemote::Ignore);
        s.on_local_unlocked();
        // A peer's lock that this OS reports only much later counts as the user's.
        assert_eq!(s.on_remote(8, false, true, t0), OnRemote::Lock { lock_here: true, forward: true });
        assert_eq!(s.on_local_locked(true, 9, t0 + CAUSE_WINDOW), Some(9));
        // Old ids are forgotten eventually, but only after many others.
        let mut s = LockSync::default();
        for id in 0..SEEN as u64 {
            s.on_remote(id, true, false, t0);
        }
        assert_eq!(s.on_remote(SEEN as u64 - 1, true, false, t0), OnRemote::Ignore);
        // With lock sync off a synced lock is ignored, and stays ignored
        // when it comes again over another link.
        let mut s = LockSync::default();
        assert_eq!(s.on_remote(1, false, false, t0), OnRemote::Ignore);
        assert_eq!(s.on_remote(1, true, false, t0), OnRemote::Ignore);
        assert_eq!(s.on_remote(2, true, false, t0), OnRemote::Lock { lock_here: true, forward: false });
    }
}
