//! Pure local keyboard adapter for a future native hook/tap integration.
//!
//! This module does not install a hook or inject input. A native caller must
//! serialize physical events, snapshot replacement, timers and release-all;
//! acknowledge injection completion before switching to remote capture; and
//! bypass all injected events before calling this adapter. In particular,
//! putting emitted keys on an unacknowledged background queue is NOT safe.

use crate::hotkeys::{Action, Bindings, ChordMods, EMERGENCY, Matcher, Verdict};
use one_mk_proto::remap::{Emit, Keymap, Mapper};
use one_mk_proto::{HeldKeys, Mods};
use std::time::{Duration, Instant};

/// Immutable rules and physical shortcuts, resolved outside the input hook.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub keymap: Keymap,
    pub hotkeys: Bindings,
}

/// Allocated outside the hook. Replacement returns the old allocation to
/// the caller so it can also be destroyed outside the hook.
pub struct Prepared {
    mapper: Mapper,
    hotkeys: Bindings,
}

impl Prepared {
    pub fn new(snapshot: Snapshot) -> Self {
        Self { mapper: Mapper::new(snapshot.keymap), hotkeys: snapshot.hotkeys }
    }
}

/// A bounded, event-driven health lease; reading it never locks or allocates.
/// A future platform bridge must publish acknowledgements outside callbacks.
/// An expired lease passes the first gesture through, not through the mapper.
#[derive(Clone, Copy, Debug, Default)]
pub struct Health {
    acknowledged: Option<Instant>,
}

impl Health {
    pub const LEASE: Duration = Duration::from_millis(100);

    pub fn acknowledge(&mut self, now: Instant) {
        self.acknowledged = Some(now);
    }

    pub fn healthy(&self, now: Instant) -> bool {
        self.acknowledged.is_some_and(|ack| now.checked_duration_since(ack).is_some_and(|age| age < Self::LEASE))
    }

    pub fn invalidate(&mut self) {
        self.acknowledged = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Pass,
    Swallow,
    Shortcut(Action),
}

/// One independent instance of the SAME mapper used for remote visits.
/// Hotkeys and the emergency chord always see physical keys first. The
/// callback must consume outputs in order; it must not block or allocate
/// if this method is used from a native input hook.
pub struct LocalMapper {
    prepared: Prepared,
    physical: HeldKeys,
    passed: HeldKeys,
    owned: HeldKeys,
    matcher: Matcher,
    draining: bool,
}

impl LocalMapper {
    pub fn new(prepared: Prepared) -> Self {
        Self {
            prepared,
            physical: HeldKeys::new(),
            passed: HeldKeys::new(),
            owned: HeldKeys::new(),
            matcher: Matcher::default(),
            draining: false,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.prepared.mapper.deadline()
    }

    pub fn is_idle(&self) -> bool {
        self.prepared.mapper.is_idle()
    }

    /// The native bridge must call this on failure, suspension, tap disable,
    /// secure-desktop entry or lease expiry. Already-swallowed physical
    /// presses stay swallowed (including repeats) until their release.
    pub fn disable(&mut self, out: &mut impl FnMut(Emit)) {
        self.prepared.mapper.release_all(out);
        self.draining = !self.physical.is_empty();
    }

    /// Use when the OS has reset physical state (session lock or tap reset).
    /// Unlike disable, this does not wait for key-ups the OS may hide forever.
    pub fn reset(&mut self, out: &mut impl FnMut(Emit)) {
        self.disable(out);
        let now = Instant::now();
        for code in self.physical.iter() {
            self.prepared.mapper.key(code, false, now, out);
        }
        self.physical = HeldKeys::new();
        self.passed = HeldKeys::new();
        self.owned = HeldKeys::new();
        self.matcher.reset();
        self.draining = false;
    }

    /// Install a fully prepared snapshot without locking, allocating or
    /// destroying either rule table. Mid-gesture changes fail open until
    /// all physical keys have come up; no old press changes meaning.
    pub fn replace(&mut self, prepared: Prepared, out: &mut impl FnMut(Emit)) -> Prepared {
        self.disable(out);
        std::mem::replace(&mut self.prepared, prepared)
    }

    pub fn timer(&mut self, now: Instant, healthy: bool, out: &mut impl FnMut(Emit)) {
        if !healthy {
            self.disable(out);
        } else if !self.draining {
            self.prepared.mapper.timer(now, out);
        }
    }

    /// `injected` includes our own tag AND other programs' synthetic input.
    /// `healthy` must also be false while forwarding or on a secure desktop.
    pub fn key(&mut self, code: u32, down: bool, injected: bool, healthy: bool, now: Instant, out: &mut impl FnMut(Emit)) -> Decision {
        if injected {
            return Decision::Pass;
        }
        if code >= one_mk_proto::keynames::CODE_LIMIT {
            self.disable(out);
            return Decision::Pass;
        }
        if self.physical.is_empty() {
            self.draining = false;
        }
        if !healthy {
            self.disable(out);
        }
        let mods = Mods::from_keys(self.physical.iter());
        let emergency = down && !self.physical.is_held(code) && code == EMERGENCY.key
            && ChordMods::from_held(mods) == EMERGENCY.mods;
        let verdict = self.matcher.key(&self.prepared.hotkeys, mods, code, down);
        self.physical.update(code, down);
        if emergency || matches!(verdict, Verdict::Fire(_)) {
            let action = if emergency { Action::Home } else if let Verdict::Fire(action) = verdict { action } else { unreachable!() };
            self.disable(out);
            self.owned.update(code, true);
            return Decision::Shortcut(action);
        }
        if self.owned.is_held(code) {
            if !down || (!self.draining && healthy) {
                self.prepared.mapper.key(code, down, now, out);
            }
            if !down {
                self.owned.update(code, false);
            }
            return Decision::Swallow;
        }
        if self.passed.is_held(code) {
            if !down { self.passed.update(code, false); }
            return Decision::Pass;
        }
        if !down || !healthy || self.draining || !self.passed.is_empty() || self.prepared.mapper.keymap().is_empty() {
            self.passed.update(code, down);
            return Decision::Pass;
        }
        self.owned.update(code, true);
        self.prepared.mapper.key(code, true, now, out);
        Decision::Swallow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::remap::{RuleSpec, Target, What};

    fn prepared(rules: &[(&str, &str)], hotkeys: Bindings) -> Prepared {
        let rules: Vec<_> = rules.iter().map(|(from, to)| RuleSpec {
            from: from.parse().unwrap(), layer: None, what: What::To(to.parse().unwrap()),
        }).collect();
        let keymap = Keymap::compile(&rules, Target { os: one_mk_proto::os::WINDOWS, swap: false }, Duration::from_millis(200)).unwrap().map;
        Prepared::new(Snapshot { keymap, hotkeys })
    }

    #[test]
    fn physical_emergency_survives_remapped_escape_and_modifiers() {
        let mut local = LocalMapper::new(prepared(&[("esc", "none"), ("lctrl", "a"), ("lalt", "b"), ("lshift", "c")], Bindings::default()));
        let now = Instant::now();
        let mut output = Vec::new();
        for code in [29, 56, 42] {
            assert_eq!(local.key(code, true, false, true, now, &mut |event| output.push(event)), Decision::Swallow);
        }
        assert_eq!(local.key(1, true, false, true, now, &mut |event| output.push(event)), Decision::Shortcut(Action::Home));
        assert!(local.is_idle());
        for code in [1, 42, 56, 29] {
            assert_eq!(local.key(code, false, false, true, now, &mut |event| output.push(event)), Decision::Swallow);
        }
        assert_eq!(output.iter().filter(|event| matches!(event, Emit::Key { down: true, .. })).count(), 3);
        assert_eq!(output.iter().filter(|event| matches!(event, Emit::Key { down: false, .. })).count(), 3);
    }

    #[test]
    fn user_shortcuts_are_physical_and_repeats_do_not_fire_twice() {
        let hotkeys = Bindings::new(vec![(Action::Cycle, "ctrl+h".parse().unwrap())]).unwrap();
        let mut local = LocalMapper::new(prepared(&[("h", "j"), ("lctrl", "none")], hotkeys));
        let now = Instant::now();
        local.key(29, true, false, true, now, &mut |_| {});
        assert_eq!(local.key(35, true, false, true, now, &mut |_| panic!("shortcut emitted a mapped key")), Decision::Shortcut(Action::Cycle));
        assert_eq!(local.key(35, true, false, true, now, &mut |_| panic!("shortcut repeated")), Decision::Swallow);
        assert_eq!(local.key(35, false, false, true, now, &mut |_| panic!("shortcut released a mapped key")), Decision::Swallow);
    }

    #[test]
    fn fail_open_does_not_turn_an_old_press_into_a_new_mapping() {
        let mut local = LocalMapper::new(prepared(&[("a", "b")], Bindings::default()));
        let now = Instant::now();
        assert_eq!(local.key(30, true, false, false, now, &mut |_| panic!("unhealthy")), Decision::Pass);
        assert_eq!(local.key(30, true, false, true, now, &mut |_| panic!("old repeat")), Decision::Pass);
        assert_eq!(local.key(30, false, false, true, now, &mut |_| panic!("old up")), Decision::Pass);
        let mut output = Vec::new();
        assert_eq!(local.key(30, true, false, true, now, &mut |event| output.push(event)), Decision::Swallow);
        local.disable(&mut |event| output.push(event));
        assert_eq!(local.key(30, true, false, true, now, &mut |_| panic!("retired repeat")), Decision::Swallow);
        assert_eq!(local.key(30, false, false, true, now, &mut |_| panic!("retired release")), Decision::Swallow);
        assert_eq!(output, [Emit::Key { code: 48, down: true }, Emit::Key { code: 48, down: false }]);
    }

    #[test]
    fn a_key_works_again_after_disable_or_a_hidden_session_key_up() {
        let mut local = LocalMapper::new(prepared(&[("a", "b")], Bindings::default()));
        let now = Instant::now();
        for reset in [false, true] {
            local.key(30, true, false, true, now, &mut |_| {});
            if reset {
                local.reset(&mut |_| {});
            } else {
                local.disable(&mut |_| {});
                local.key(30, false, false, true, now, &mut |_| {});
            }
            let mut output = Vec::new();
            local.key(30, true, false, true, now, &mut |event| output.push(event));
            local.key(30, false, false, true, now, &mut |event| output.push(event));
            assert_eq!(output, [Emit::Key { code: 48, down: true }, Emit::Key { code: 48, down: false }], "reset={reset}");
        }
    }

    #[test]
    fn injected_input_never_changes_physical_shortcuts_or_ownership() {
        let mut local = LocalMapper::new(prepared(&[("a", "b")], Bindings::default()));
        let now = Instant::now();
        for code in [29, 56, 42, 1, 30] {
            assert_eq!(local.key(code, true, true, true, now, &mut |_| panic!("injected input remapped")), Decision::Pass);
        }
        let mut output = Vec::new();
        local.key(30, true, false, true, now, &mut |event| output.push(event));
        local.key(30, false, false, true, now, &mut |event| output.push(event));
        assert_eq!(output, [Emit::Key { code: 48, down: true }, Emit::Key { code: 48, down: false }]);
    }

    #[test]
    fn snapshot_swap_and_session_reset_release_the_previous_mapper() {
        let mut local = LocalMapper::new(prepared(&[("a", "b")], Bindings::default()));
        let now = Instant::now();
        let mut output = Vec::new();
        local.key(30, true, false, true, now, &mut |event| output.push(event));
        let old = local.replace(prepared(&[("a", "c")], Bindings::default()), &mut |event| output.push(event));
        drop(old);
        local.key(30, false, false, true, now, &mut |_| panic!("old up remapped"));
        local.key(30, true, false, true, now, &mut |event| output.push(event));
        local.reset(&mut |event| output.push(event));
        assert!(local.is_idle());
        assert_eq!(output, [Emit::Key { code: 48, down: true }, Emit::Key { code: 48, down: false }, Emit::Key { code: 46, down: true }, Emit::Key { code: 46, down: false }]);
    }

    #[test]
    fn health_expires_without_a_periodic_timer() {
        let mut health = Health::default();
        let now = Instant::now();
        assert!(!health.healthy(now));
        health.acknowledge(now);
        assert!(health.healthy(now));
        assert!(!health.healthy(now + Health::LEASE));
        assert!(!health.healthy(now - Duration::from_millis(1)));
        health.invalidate();
        assert!(!health.healthy(now));
    }

    fn dual_role() -> Snapshot {
        let rules = vec![
            RuleSpec { from: "capslock".parse().unwrap(), layer: None, what: What::TapHold {
                tap: "esc".parse().unwrap(), hold: "lctrl".parse().unwrap(), timeout: None,
            } },
            RuleSpec { from: "ctrl+h".parse().unwrap(), layer: None, what: What::To("backspace".parse().unwrap()) },
            RuleSpec { from: "f1".parse().unwrap(), layer: None, what: What::To(one_mk_proto::remap::Output::sequence(&["ctrl+a", "a", "lctrl"]).unwrap()) },
            RuleSpec { from: "ralt".parse().unwrap(), layer: None, what: What::To("layer:nav".parse().unwrap()) },
            RuleSpec { from: "h".parse().unwrap(), layer: Some("nav".into()), what: What::To("left".parse().unwrap()) },
        ];
        let keymap = Keymap::compile(&rules, Target { os: one_mk_proto::os::MACOS, swap: false }, Duration::from_millis(200)).unwrap().map;
        Snapshot { keymap, hotkeys: Bindings::new(vec![(Action::Cycle, "ctrl+alt+1".parse().unwrap())]).unwrap() }
    }

    #[test]
    fn only_a_pending_dual_role_key_arms_the_local_timer() {
        let mut local = LocalMapper::new(Prepared::new(dual_role()));
        let now = Instant::now();
        let mut output = Vec::new();
        assert_eq!(local.deadline(), None);
        local.key(58, true, false, true, now, &mut |event| output.push(event));
        assert!(output.is_empty());
        assert_eq!(local.deadline(), Some(now + Duration::from_millis(200)));
        local.timer(now + Duration::from_millis(200), true, &mut |event| output.push(event));
        assert_eq!(output, [Emit::Key { code: 29, down: true }]);
        assert_eq!(local.deadline(), None);
        local.timer(now + Duration::from_millis(300), false, &mut |event| output.push(event));
        assert_eq!(output[1..], [Emit::Key { code: 29, down: false }]);
        local.key(58, false, false, true, now, &mut |_| panic!("cancelled hold"));
        local.key(58, true, false, true, now, &mut |_| {});
        local.disable(&mut |_| panic!("an undecided tap emits nothing on disable"));
        assert_eq!(local.deadline(), None);
        local.timer(now + Duration::from_secs(1), true, &mut |_| panic!("disabled timer"));
    }

    #[test]
    fn local_scope_preserves_the_shared_random_stream_invariant() {
        use one_mk_proto::remap::test_support::{Rng, Wire};
        use std::collections::BTreeSet;

        let keys = [58, 56, 125, 29, 97, 42, 35, 36, 16, 30, 46, 100, 57, 59, 60, 88, 183, 1, 2];
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let snapshots = [dual_role(), Snapshot::default()];
        for stream in 0..10_000 {
            let mut local = LocalMapper::new(Prepared::new(snapshots[stream % snapshots.len()].clone()));
            let mut wire = Wire::default();
            let mut physical = BTreeSet::new();
            let mut now = Instant::now();
            for _ in 0..rng.below(200) + 1 {
                let mut output = Vec::new();
                let mut repeat = false;
                match rng.below(20) {
                    0..=11 => {
                        let code = keys[rng.below(keys.len())];
                        let down = rng.below(2) == 0;
                        repeat = down && physical.contains(&code);
                        if down { physical.insert(code); } else { physical.remove(&code); }
                        let healthy = rng.below(10) != 0;
                        local.key(code, down, false, healthy, now, &mut |event| output.push(event));
                    }
                    12 => now += Duration::from_millis(rng.below(400) as u64),
                    13 | 14 => {
                        now += Duration::from_millis(rng.below(400) as u64);
                        local.timer(now, rng.below(4) != 0, &mut |event| output.push(event));
                    }
                    15 => local.disable(&mut |event| output.push(event)),
                    16 => {
                        local.reset(&mut |event| output.push(event));
                        physical.clear();
                    }
                    17 | 18 => {
                        drop(local.replace(Prepared::new(snapshots[rng.below(snapshots.len())].clone()), &mut |event| output.push(event)));
                    }
                    _ => {
                        local.key(keys[rng.below(keys.len())], true, true, true, now, &mut |event| output.push(event));
                    }
                }
                for event in output { wire.apply(event, repeat); }
            }
            for code in physical {
                local.key(code, false, false, true, now, &mut |event| wire.apply(event, false));
            }
            assert!(wire.down.is_empty(), "stream {stream}: {:?}", wire.down);
            assert!(local.is_idle(), "stream {stream}");
            assert_eq!(local.deadline(), None);
        }
    }
}
