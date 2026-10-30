//! Layout learning (`[edge] learn`, see `docs/EDGE-PUSH.md`): the pointer
//! pushed twice against an edge where no computer sits says one is there.
//! The state machines here are pure (time is passed in), so they are
//! tested without a daemon; `app/push.rs` drives them.

use crate::settings::Learn;
use one_mk_proto::Side;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Two pushes this close together (start to start) say "a computer is there".
pub const PUSH_WINDOW: Duration = Duration::from_secs(2);
/// Captures closer together than this are one push: holding the pointer
/// against the edge makes the backend report it again and again.
pub const DEBOUNCE: Duration = Duration::from_millis(350);
/// At most one suggestion (or placement) per computer this often.
pub const COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// How long a suggestion is shown in `one-mk status` and the window.
pub const SHOWN_FOR: Duration = Duration::from_secs(5 * 60);

/// Counts pushes against the edges where no computer sits.
#[derive(Debug, Default)]
pub struct Pushes {
    last: Option<Push>,
}

#[derive(Debug)]
struct Push {
    side: Side,
    /// When this push began.
    started: Instant,
    /// The latest capture that was part of it.
    latest: Instant,
}

impl Pushes {
    /// The pointer was caught by an edge on `side` at `now`. True when that
    /// makes two pushes against that side within [`PUSH_WINDOW`]; counting
    /// then starts over. A push against another side starts over too.
    pub fn hit(&mut self, side: Side, now: Instant) -> bool {
        if let Some(p) = self.last.as_mut().filter(|p| p.side == side) {
            if now.saturating_duration_since(p.latest) < DEBOUNCE {
                p.latest = now; // still the same push
                return false;
            }
            if now.saturating_duration_since(p.started) <= PUSH_WINDOW {
                self.last = None;
                return true;
            }
        }
        self.last = Some(Push { side, started: now, latest: now });
        false
    }

    /// Forgets the push in progress (the pointer went elsewhere).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// When each computer was last suggested, so none is more than once per
/// [`COOLDOWN`].
#[derive(Debug)]
pub struct Cooldown<K> {
    last: HashMap<K, Instant>,
}

impl<K> Default for Cooldown<K> {
    fn default() -> Self {
        Self { last: HashMap::new() }
    }
}

impl<K: Eq + Hash + Copy> Cooldown<K> {
    pub fn ready(&self, key: K, now: Instant) -> bool {
        self.last.get(&key).is_none_or(|t| now.saturating_duration_since(*t) >= COOLDOWN)
    }

    /// `key` was suggested at `now`. Old entries go, so this stays as small
    /// as the number of computers suggested in the last [`COOLDOWN`].
    pub fn note(&mut self, key: K, now: Instant) {
        self.last.retain(|_, t| now.saturating_duration_since(*t) < COOLDOWN);
        self.last.insert(key, now);
    }
}

/// An online computer, as far as learning cares.
#[derive(Clone, Copy, Debug)]
pub struct Online<K> {
    pub key: K,
    pub side: Side,
    /// Its place is only a guess (`Device::learnable`).
    pub learnable: bool,
    pub connected: Instant,
}

/// Which computer a push against `pushed` is about: of the online ones
/// whose place is only a guess and not on that side already, the one that
/// connected last (so with one online, that one). None if there is none.
pub fn candidate<K: Copy>(online: &[Online<K>], pushed: Side) -> Option<K> {
    online.iter().filter(|p| p.learnable && p.side != pushed).max_by_key(|p| p.connected).map(|p| p.key)
}

/// What to do about a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Suggest,
    Place,
}

/// `auto` places a computer only beside this one (left or right). Above
/// and below are where the pointer goes for menus, title bars and the
/// taskbar, so a push there is suggested and never acted on.
pub fn act(mode: Learn, pushed: Side) -> Option<Act> {
    match mode {
        Learn::Off => None,
        Learn::Auto if !pushed.along_x() => Some(Act::Place),
        Learn::Auto | Learn::Suggest => Some(Act::Suggest),
    }
}

/// A suggestion on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown<K> {
    pub key: K,
    pub side: Side,
    pub at: Instant,
}

impl<K> Shown<K> {
    pub fn current(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) < SHOWN_FOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn two_pushes_within_two_seconds() {
        let t0 = Instant::now();
        let mut p = Pushes::default();
        assert!(!p.hit(Side::Left, t0));
        assert!(p.hit(Side::Left, ms(t0, 1500)));
        // Counting starts over.
        assert!(!p.hit(Side::Left, ms(t0, 2500)));
        assert!(p.hit(Side::Left, ms(t0, 4500)), "exactly 2 s apart still counts");
        assert!(!p.hit(Side::Left, ms(t0, 10_000)));
        assert!(!p.hit(Side::Left, ms(t0, 12_001)), "more than 2 s apart: a new first push");
        assert!(p.hit(Side::Left, ms(t0, 13_000)));
    }

    #[test]
    fn holding_against_the_edge_is_one_push() {
        let t0 = Instant::now();
        let mut p = Pushes::default();
        // The backend reports the edge every 50 ms while the pointer pushes on it.
        for i in 0..30 {
            assert!(!p.hit(Side::Right, ms(t0, i * 50)), "capture {i}");
        }
        // Let go and push again: that was a long push (1.45 s), still in the window.
        assert!(p.hit(Side::Right, ms(t0, 1900)));
        // A push held longer than the window, then another: not two quick ones.
        let t1 = ms(t0, 10_000);
        for i in 0..50 {
            assert!(!p.hit(Side::Right, ms(t1, i * 60)));
        }
        assert!(!p.hit(Side::Right, ms(t1, 3500)));
        assert!(p.hit(Side::Right, ms(t1, 4000)));
    }

    #[test]
    fn another_edge_starts_over() {
        let t0 = Instant::now();
        let mut p = Pushes::default();
        assert!(!p.hit(Side::Left, t0));
        assert!(!p.hit(Side::Top, ms(t0, 500)));
        assert!(!p.hit(Side::Left, ms(t0, 1000)), "the top push broke the pair");
        assert!(p.hit(Side::Left, ms(t0, 1600)));
        assert!(!p.hit(Side::Left, ms(t0, 2000)));
        p.reset();
        assert!(!p.hit(Side::Left, ms(t0, 2500)), "reset forgets the first push");
    }

    #[test]
    fn time_going_backwards_never_panics_or_fires() {
        let t0 = Instant::now() + Duration::from_secs(60);
        let mut p = Pushes::default();
        assert!(!p.hit(Side::Left, t0));
        assert!(!p.hit(Side::Left, t0 - Duration::from_secs(30)), "counts as the same push");
        let mut c = Cooldown::default();
        c.note(1u8, t0);
        assert!(!c.ready(1, t0 - Duration::from_secs(30)));
    }

    /// Random captures on random sides at random gaps: a conclusion only
    /// ever follows a capture on the same side as one at most 2 s before,
    /// with a pause of at least the debounce in between.
    #[test]
    fn random_pushes_never_conclude_without_two_quick_ones() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let t0 = Instant::now();
        let mut now = 0u64;
        let mut p = Pushes::default();
        let mut log: Vec<(u64, Side)> = Vec::new();
        let mut fired = 0;
        for _ in 0..20_000 {
            now += next() % 1500;
            let side = Side::ALL[(next() % 4) as usize];
            if p.hit(side, ms(t0, now)) {
                fired += 1;
                let gap_before = log.last().map(|&(t, s)| (now - t, s));
                assert!(matches!(gap_before, Some((g, s)) if s == side && g >= DEBOUNCE.as_millis() as u64), "{gap_before:?}");
                assert!(log.iter().rev().take_while(|(_, s)| *s == side).any(|&(t, _)| now - t <= 2000));
                log.clear();
            } else {
                log.push((now, side));
            }
        }
        assert!(fired > 100, "only {fired}");
    }

    #[test]
    fn at_most_one_suggestion_per_computer_per_ten_minutes() {
        let t0 = Instant::now();
        let mut c = Cooldown::default();
        assert!(c.ready('a', t0));
        c.note('a', t0);
        assert!(!c.ready('a', ms(t0, 60_000)));
        assert!(c.ready('b', ms(t0, 60_000)), "per computer");
        assert!(!c.ready('a', ms(t0, 599_999)));
        assert!(c.ready('a', ms(t0, 600_000)));
        c.note('b', ms(t0, 600_000));
        assert_eq!(c.last.len(), 1, "old entries go");
    }

    #[test]
    fn the_candidate_is_a_guessed_computer_not_already_there() {
        let t0 = Instant::now();
        let on = |key, side, learnable, at| Online { key, side, learnable, connected: ms(t0, at) };
        // One online, guessed on the right: pushing left names it.
        assert_eq!(candidate(&[on('a', Side::Right, true, 0)], Side::Left), Some('a'));
        assert_eq!(candidate(&[on('a', Side::Left, true, 0)], Side::Left), None, "already there");
        assert_eq!(candidate(&[on('a', Side::Right, false, 0)], Side::Left), None, "placed by the user");
        assert_eq!(candidate::<char>(&[], Side::Left), None);
        // Several: the guessed one that connected last.
        let several = [on('a', Side::Right, true, 0), on('b', Side::Right, false, 900), on('c', Side::Right, true, 500), on('d', Side::Left, true, 950)];
        assert_eq!(candidate(&several, Side::Left), Some('c'));
        assert_eq!(candidate(&several, Side::Top), Some('d'));
    }

    #[test]
    fn auto_places_only_beside_this_computer() {
        assert_eq!(act(Learn::Off, Side::Left), None);
        assert_eq!(act(Learn::Suggest, Side::Left), Some(Act::Suggest));
        assert_eq!(act(Learn::Auto, Side::Left), Some(Act::Place));
        assert_eq!(act(Learn::Auto, Side::Right), Some(Act::Place));
        assert_eq!(act(Learn::Auto, Side::Top), Some(Act::Suggest));
        assert_eq!(act(Learn::Auto, Side::Bottom), Some(Act::Suggest));
    }

    #[test]
    fn a_suggestion_shows_for_five_minutes() {
        let t0 = Instant::now();
        let s = Shown { key: 1, side: Side::Left, at: t0 };
        assert!(s.current(ms(t0, 299_999)));
        assert!(!s.current(ms(t0, 300_000)));
    }
}
