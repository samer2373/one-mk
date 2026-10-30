//! Bounded hook admission and acknowledged local injection. No native APIs.
//! The single capture producer closes admission before fencing its queue.
//! The worker must drain correlation before calling `Engine::fence`.

use crate::hotkeys::{Action, Bindings, ChordMods, EMERGENCY, Matcher, Verdict};
use crate::localmap_handoff::{Destination, Handoff, HookGate, Owner, PhysicalKey, Route, State};
use arc_swap::ArcSwap;
use crossbeam_queue::ArrayQueue;
use one_mk_proto::device::DeviceRef;
use one_mk_proto::remap::{Emit, Keymap, KeymapSet, Mapper};
use one_mk_proto::{HeldKeys, Mods};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const CAPACITY: usize = 64;
pub const LEASE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub maps: KeymapSet,
    pub hotkeys: Bindings,
    pub count: usize,
}

impl Default for Rules {
    fn default() -> Self {
        Self { maps: KeymapSet::new(Keymap::default()), hotkeys: Bindings::default(), count: 0 }
    }
}

/// Writers retain retired snapshots until all full reader references leave.
/// Loading/dropping a reader reference therefore never destroys a rule table.
/// Keep this owner alive until the native hook is removed.
pub struct Snapshots {
    slot: ArcSwap<Rules>,
    retired: Vec<Arc<Rules>>,
}

impl Default for Snapshots {
    fn default() -> Self {
        let snapshots = Self { slot: ArcSwap::from_pointee(Rules::default()), retired: Vec::new() };
        drop(snapshots.load());
        snapshots
    }
}

impl Snapshots {
    pub fn load(&self) -> Arc<Rules> {
        self.slot.load_full()
    }

    /// Only call outside callbacks, on the capture producer thread.
    pub fn replace(&mut self, rules: Arc<Rules>) {
        self.retired.push(self.slot.swap(rules));
        self.retired.retain(|old| Arc::strong_count(old) != 1);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub keyboard_type: u32,
    pub sequence: u64,
    pub code: u32,
    pub down: bool,
    pub at: Instant,
}

#[derive(Default)]
pub struct Physical {
    down: HeldKeys,
    owned: HeldKeys,
    shortcuts: HeldKeys,
    sources: Vec<(Option<DeviceRef>, HeldKeys)>,
    passed: HeldKeys,
    remote: HeldKeys,
    matcher: Matcher,
}

impl Physical {
    pub fn correlated(&mut self, code: u32, down: bool, device: Option<DeviceRef>) -> bool {
        if let Some((_, held)) = self.sources.iter_mut().find(|(source, _)| *source == device) {
            held.update(code, down);
        } else if down {
            if self.sources.len() >= CAPACITY { return false; }
            let mut held = HeldKeys::new();
            held.update(code, true);
            self.sources.push((device, held));
        }
        self.sources.retain(|(_, held)| !held.is_empty());
        true
    }

    pub fn baseline(&mut self, held: impl Iterator<Item = u32>) {
        self.passed = HeldKeys::new();
        self.down = HeldKeys::new();
        self.matcher.reset();
        for code in held.filter(|code| !(0x110..=0x11f).contains(code)) {
            if !self.owned.is_held(code) && !self.remote.is_held(code) { self.passed.update(code, true); }
            self.down.update(code, true);
        }
        for code in self.owned.clone().iter() {
            if !self.down.is_held(code) { self.owned.update(code, false); }
        }
        for code in self.shortcuts.clone().iter() {
            if !self.down.is_held(code) { self.shortcuts.update(code, false); }
        }
        for (_, held) in &mut self.sources {
            for code in held.clone().iter() {
                if !self.down.is_held(code) { held.update(code, false); }
            }
        }
        self.sources.retain(|(_, held)| !held.is_empty());
    }

    pub fn key(
        &mut self, key: PhysicalKey, code: u32, now: Instant, rules: &Rules, admission: &mut Admission,
        mut action: impl FnMut(Action) -> bool,
    ) -> bool {
        if key.injected { return false; }
        if let Some(swallow) = self.shortcut(code, key.down, rules, admission, &mut action) { return swallow; }
        self.route(key, code, now, admission)
    }

    pub fn shortcut(
        &mut self, code: u32, down: bool, rules: &Rules, admission: &Admission,
        mut action: impl FnMut(Action) -> bool,
    ) -> Option<bool> {
        let mods = Mods::from_keys(self.down.iter());
        self.down.update(code, down);
        if self.shortcuts.is_held(code) {
            if !down { self.shortcuts.update(code, false); }
            return Some(true);
        }
        let emergency = down && code == EMERGENCY.key && ChordMods::from_held(mods).contains(EMERGENCY.mods);
        let verdict = self.matcher.key(&rules.hotkeys, mods, code, down);
        if emergency || matches!(verdict, Verdict::Fire(_)) {
            let shortcut = if emergency {
                admission.fail();
                Action::Home
            } else if let Verdict::Fire(shortcut) = verdict { shortcut } else { return None };
            let queued = action(shortcut);
            if !emergency && queued { self.shortcuts.update(code, true); }
            if emergency { self.passed.update(code, true); }
            return Some(!emergency && queued);
        }
        None
    }

    fn route(&mut self, key: PhysicalKey, code: u32, now: Instant, admission: &mut Admission) -> bool {
        let owner = if self.passed.is_held(code) { Owner::Os }
            else if self.owned.is_held(code) || !key.down && self.sources.iter().any(|(_, held)| held.is_held(code)) { Owner::Local }
            else if self.remote.is_held(code) { Owner::Remote } else { Owner::Unknown };
        let route = admission.gate.decide(PhysicalKey {
            owner, eligible: key.eligible && self.passed.is_empty(), healthy: key.healthy && admission.healthy(now), ..key
        });
        let swallow = match route {
            Route::Local => admission.admit(code, key.down, now) || owner == Owner::Local,
            Route::Suppress => {
                if !key.down && owner == Owner::Local { admission.submit(code, false, now); }
                true
            }
            _ => false,
        };
        if key.down && owner == Owner::Unknown {
            if swallow {
                self.owned.update(code, true);
            } else { self.passed.update(code, true); }
        } else if !key.down {
            self.owned.update(code, false);
            self.passed.update(code, false);
            self.remote.update(code, false);
        }
        swallow
    }

    pub fn remote_key(&mut self, code: u32, down: bool) {
        self.down.update(code, down);
        self.remote.update(code, down);
    }

    pub fn os_owned(&self, code: u32) -> bool { self.passed.is_held(code) }

    pub fn os_release(&mut self, code: u32) {
        self.passed.update(code, false);
        self.down.update(code, false);
    }

    pub fn suppress_owned(&mut self, code: u32, down: bool, admission: &mut Admission) -> bool {
        if self.owned.is_held(code) || !down && self.sources.iter().any(|(_, held)| held.is_held(code)) {
            if !down {
                self.owned.update(code, false);
                admission.submit(code, false, Instant::now());
            }
            return true;
        }
        false
    }
}

/// The callback only uses bounded, preallocated, lock-free admission.
/// The capture loop consumes these keys, correlates devices, then sends work.
pub struct Admission {
    keys: ArrayQueue<Key>,
    feedback: Arc<Feedback>,
    submitted: u64,
    oldest: Option<Instant>,
    gate: Arc<HookGate>,
    pub keyboard_type: u32,
}

pub struct Feedback {
    health: AtomicU64,
    acknowledged: AtomicU64,
    submitted: AtomicU64,
}

impl Feedback {
    pub fn fail(&self) {
        if self.health.try_update(Ordering::AcqRel, Ordering::Acquire, |state| state.checked_add(2).map(|next| next | 1)).is_err() {
            self.health.fetch_or(1, Ordering::Release);
        }
    }
    pub fn acknowledge(&self, sequence: u64) { self.acknowledged.fetch_max(sequence, Ordering::Release); }
    pub fn healthy(&self) -> bool { self.token() & 1 == 0 }
    pub fn token(&self) -> u64 { self.health.load(Ordering::Acquire) }
    fn recover(&self, expected: u64) -> bool {
        expected < u64::MAX && self.health.compare_exchange(expected, expected & !1, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
    pub fn outstanding(&self) -> Option<u64> {
        let submitted = self.submitted.load(Ordering::Acquire);
        (self.acknowledged.load(Ordering::Acquire) < submitted).then_some(submitted)
    }
    pub fn acknowledged(&self, sequence: u64) -> bool {
        self.acknowledged.load(Ordering::Acquire) >= sequence
    }
}

#[derive(Default)]
pub struct WatchLease {
    pending: Option<(u64, Instant)>,
}

impl WatchLease {
    pub fn deadline(&mut self, feedback: &Feedback, now: Instant) -> Option<Instant> {
        let outstanding = feedback.outstanding();
        if outstanding.is_none() { self.pending = None; }
        if self.pending.is_none() { self.pending = outstanding.map(|sequence| (sequence, now + LEASE)); }
        self.pending.map(|(_, deadline)| deadline)
    }

    pub fn expired(&mut self, feedback: &Feedback, now: Instant) -> bool {
        if self.pending.is_some_and(|(_, deadline)| now >= deadline) {
            return self.pending.take().is_some_and(|(sequence, _)| !feedback.acknowledged(sequence));
        }
        false
    }
}

impl Admission {
    pub fn new(gate: Arc<HookGate>) -> Self {
        Self {
            keys: ArrayQueue::new(CAPACITY),
            feedback: Arc::new(Feedback { health: AtomicU64::new(0), acknowledged: AtomicU64::new(0), submitted: AtomicU64::new(0) }),
            submitted: 0, oldest: None, gate, keyboard_type: 0,
        }
    }

    pub fn healthy(&self, now: Instant) -> bool {
        self.feedback.healthy()
            && self.oldest.is_none_or(|oldest| self.feedback.acknowledged.load(Ordering::Acquire) == self.submitted
                || now.saturating_duration_since(oldest) < LEASE)
    }

    pub fn fail(&self) {
        self.feedback.fail();
        self.gate.close();
    }

    #[cfg(test)]
    fn recover(&mut self) {
        self.oldest = None;
        self.feedback.recover(self.feedback.token());
    }

    pub fn admit(&mut self, code: u32, down: bool, at: Instant) -> bool {
        if !self.healthy(at) {
            self.fail();
            return false;
        }
        self.submit(code, down, at)
    }

    fn submit(&mut self, code: u32, down: bool, at: Instant) -> bool {
        let acknowledged = self.feedback.acknowledged.load(Ordering::Acquire);
        if self.submitted.saturating_sub(acknowledged) >= CAPACITY as u64 {
            self.fail();
            return false;
        }
        let Some(sequence) = self.submitted.checked_add(1) else {
            self.fail();
            return false;
        };
        if self.keys.push(Key { keyboard_type: self.keyboard_type, sequence, code, down, at }).is_err() {
            self.fail();
            return false;
        }
        if acknowledged == self.submitted { self.oldest = Some(at); }
        self.submitted = sequence;
        self.feedback.submitted.store(sequence, Ordering::Release);
        true
    }

    pub fn pop(&self) -> Option<Key> {
        self.keys.pop()
    }

    pub fn watermark(&self) -> u64 {
        self.submitted
    }

    pub fn acknowledge(&self, sequence: u64) {
        self.feedback.acknowledge(sequence);
    }

    pub fn pending(&self) -> bool {
        self.feedback.acknowledged.load(Ordering::Acquire) != self.submitted
    }

    pub fn feedback(&self) -> Arc<Feedback> { self.feedback.clone() }
}

/// Return the delivered prefix (or conservatively posted downs retained for cleanup).
/// Calls are serialized by the worker, including recovery and fences.
pub trait Injector {
    fn inject(&mut self, keys: &[(u32, bool)]) -> usize;
    fn settled(&self) -> bool { true }
    fn keyboard_type(&mut self, _keyboard_type: u32) {}
}

pub struct Engine<I: Injector> {
    mapper: Mapper,
    handoff: Handoff,
    injector: I,
    held: HeldKeys,
    sequence: u64,
    failed: bool,
}

impl<I: Injector> Engine<I> {
    pub fn new(injector: I) -> Self {
        Self {
            mapper: Mapper::with_set(KeymapSet::new(Keymap::default())), handoff: Handoff::default(),
            injector, held: HeldKeys::new(), sequence: 0, failed: false,
        }
    }

    pub fn gate(&self) -> Arc<HookGate> {
        self.handoff.hook_gate()
    }

    pub fn deadline(&self) -> Option<Instant> {
        (self.handoff.state() == State::Local && !self.failed).then(|| self.mapper.deadline()).flatten()
    }

    fn emit(&mut self, events: Vec<Emit>) -> bool {
        let mut keys = Vec::with_capacity(events.len());
        for event in events {
            match event {
                Emit::Key { code, down } => keys.push((code, down)),
                Emit::Action(_) => return false,
            }
        }
        let sent = self.injector.inject(&keys);
        for &(code, down) in keys.iter().take(sent) {
            if down || self.injector.settled() { self.held.update(code, down); }
        }
        sent == keys.len() && self.injector.settled()
    }

    /// Failed releases stay in the ledger even after Mapper releases its state.
    pub fn fail(&mut self) -> bool {
        self.handoff.fail(|| {});
        self.failed = true;
        self.mapper.release_all(&mut |_| {});
        let releases: Vec<_> = self.held.iter().map(|code| (code, false)).collect();
        let sent = self.injector.inject(&releases);
        if self.injector.settled() {
            for &(code, _) in releases.iter().take(sent) { self.held.update(code, false); }
        }
        self.held.is_empty() && self.injector.settled()
    }

    pub fn key(&mut self, key: Key, device: Option<DeviceRef>) -> bool {
        if key.sequence != self.sequence.saturating_add(1) {
            self.fail();
            return false;
        }
        self.sequence = key.sequence;
        self.injector.keyboard_type(key.keyboard_type);
        if self.handoff.state() != State::Local || self.failed {
            if !key.down { self.mapper.key_from(key.code, false, device, key.at, &mut |_| {}); }
            return true;
        }
        let mut events = Vec::new();
        self.mapper.key_from(key.code, key.down, device, key.at, &mut |event| events.push(event));
        if !self.emit(events) { self.fail(); return false; }
        true
    }

    pub fn admitted_key(&mut self, key: Key, device: Option<DeviceRef>, feedback: &Feedback) -> bool {
        if !feedback.healthy() { self.fail(); }
        let success = self.key(key, device);
        if !success { feedback.fail(); }
        feedback.acknowledge(key.sequence);
        success
    }

    pub fn timer(&mut self, now: Instant) -> bool {
        if self.deadline().is_none() { return true; }
        let mut events = Vec::new();
        self.mapper.timer(now, &mut |event| events.push(event));
        if !self.emit(events) { self.fail(); return false; }
        true
    }

    pub fn baseline(&mut self, held: impl Iterator<Item = u32>) {
        let mut physical = HeldKeys::new();
        for code in held { physical.update(code, true); }
        self.mapper.reconcile_physical(&physical, &mut |_| {});
    }

    pub fn recover_fence(&mut self, watermark: u64, destination: Destination, rules: Option<&Rules>, feedback: &Feedback, health: u64) -> bool {
        let success = feedback.token() == health && self.fence(watermark, destination, rules) && feedback.recover(health);
        if !success { feedback.fail(); self.fail(); }
        success
    }

    /// An empty batch still fences the entire admitted sequence. No queued key
    /// may bypass correlation and arrive after this acknowledgement.
    pub fn fence(&mut self, watermark: u64, destination: Destination, rules: Option<&Rules>) -> bool {
        self.handoff.hook_gate().close();
        if watermark < self.sequence || watermark != self.sequence && !self.failed || !self.fail() { return false; }
        self.sequence = watermark;
        if let Some(rules) = rules { self.mapper.set_keymaps(rules.maps.clone(), &mut |_| {}); }
        let Some(ticket) = self.handoff.begin(destination, || {}) else { return false };
        self.failed = false;
        self.handoff.acknowledge(ticket)
    }
}

impl<I: Injector> Drop for Engine<I> {
    fn drop(&mut self) {
        self.fail();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::remap::{Keymap, RuleSpec, Target, What};
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Record {
        held: HashSet<u32>,
        downs: usize,
        ups: usize,
        limit: Option<usize>,
    }

    struct Fake(Arc<Mutex<Record>>);

    impl Injector for Fake {
        fn inject(&mut self, keys: &[(u32, bool)]) -> usize {
            let mut record = self.0.lock().unwrap();
            let sent = record.limit.unwrap_or(keys.len()).min(keys.len());
            for &(code, down) in keys.iter().take(sent) {
                if down {
                    if record.held.insert(code) { record.downs += 1; }
                } else {
                    assert!(record.held.remove(&code), "unmatched injected up {code}");
                    record.ups += 1;
                }
            }
            sent
        }
    }

    fn rules() -> Rules {
        let map = Keymap::compile(&[RuleSpec {
            from: "capslock".parse().unwrap(), layer: None,
            what: What::To("lctrl+a".parse().unwrap()),
        }], Target { os: one_mk_proto::os::WINDOWS, swap: false }, Duration::from_millis(200)).unwrap().map;
        Rules { maps: KeymapSet::new(map), count: 1, hotkeys: Bindings::default() }
    }

    #[test]
    fn unobserved_up_after_mapped_release_is_os_owned() {
        let mut handoff = Handoff::default();
        let ticket = handoff.begin(Destination::Local, || {}).unwrap();
        assert!(handoff.acknowledge(ticket));
        let mut admission = Admission::new(handoff.hook_gate());
        let mut physical = Physical::default();
        let now = Instant::now();
        assert!(physical.key(physical_key(true), 56, now, &rules(), &mut admission, |_| true));
        assert!(physical.key(physical_key(false), 56, now, &rules(), &mut admission, |_| true));
        assert!(!physical.key(physical_key(false), 56, now, &rules(), &mut admission, |_| true));
    }

    #[test]
    fn source_tracking_exhaustion_is_explicit_and_recovers_after_release() {
        let mut physical = Physical::default();
        for device in 0..CAPACITY {
            assert!(physical.correlated(58, true, Some(DeviceRef(device as u32))));
        }
        assert!(!physical.correlated(58, true, Some(DeviceRef(CAPACITY as u32))));
        assert!(physical.correlated(58, true, Some(DeviceRef(0))));
        assert!(physical.correlated(58, false, Some(DeviceRef(0))));
        assert!(physical.correlated(58, true, Some(DeviceRef(CAPACITY as u32))));
        assert_eq!(physical.sources.len(), CAPACITY);
    }

    fn physical_key(down: bool) -> PhysicalKey {
        PhysicalKey { down, injected: false, shortcut: false, owner: Owner::Unknown, eligible: true, healthy: true }
    }

    #[test]
    fn ordinary_hotkey_preserves_worker_health() {
        let engine = Engine::new(Fake(Arc::new(Mutex::new(Record::default()))));
        let mut admission = Admission::new(engine.gate());
        let mut physical = Physical::default();
        let mut rules = rules();
        rules.hotkeys = Bindings::new(vec![(Action::Cycle, "ctrl+alt+right".parse().unwrap())]).unwrap();
        for code in [29, 56, 106] {
            physical.key(physical_key(true), code, Instant::now(), &rules, &mut admission, |_| true);
        }
        assert!(admission.feedback().healthy());
    }

    #[test]
    fn owned_up_during_failure_retires_the_source_tombstone() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record));
        let rules = rules();
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        let mut admission = Admission::new(engine.gate());
        let mut physical = Physical::default();
        let now = Instant::now();
        assert!(physical.key(physical_key(true), 58, now, &rules, &mut admission, |_| true));
        let key = admission.pop().unwrap();
        physical.correlated(key.code, key.down, None);
        assert!(engine.admitted_key(key, None, &admission.feedback()));
        admission.fail();
        assert!(engine.fail());
        assert!(physical.key(physical_key(false), 58, now, &rules, &mut admission, |_| true));
        let release = admission.pop().expect("owned up must retire its source even when failed");
        physical.correlated(release.code, release.down, None);
        assert!(engine.admitted_key(release, None, &admission.feedback()));
        assert!(!physical.key(physical_key(false), 58, now, &rules, &mut admission, |_| true));
    }

    #[test]
    fn shortcut_release_never_enters_the_mapper() {
        for action in [Action::Cycle, Action::Jump(1), Action::LockPointer, Action::Home, Action::Broadcast, Action::FetchClipboard] {
            let engine = Engine::new(Fake(Arc::new(Mutex::new(Record::default()))));
            let mut admission = Admission::new(engine.gate());
            let mut physical = Physical::default();
            let rules = Rules { hotkeys: Bindings::new(vec![(action, "ctrl+alt+right".parse().unwrap())]).unwrap(), ..Rules::default() };
            for code in [29, 56, 106] {
                physical.key(physical_key(true), code, Instant::now(), &rules, &mut admission, |_| true);
            }
            assert!(physical.key(physical_key(false), 106, Instant::now(), &rules, &mut admission, |_| true));
            assert!(admission.pop().is_none());
            assert!(admission.feedback().healthy());
        }
    }

    #[test]
    fn held_enter_is_os_owned_and_mouse_baseline_cannot_disable_mapping() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record));
        let rules = rules();
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        let mut admission = Admission::new(engine.gate());
        let mut physical = Physical::default();
        physical.baseline([28, 0x110].into_iter());
        let now = Instant::now();
        for down in [true, true, false] {
            assert!(!physical.key(physical_key(down), 28, now, &rules, &mut admission, |_| true));
        }
        assert!(physical.key(physical_key(true), 58, now, &rules, &mut admission, |_| true));
    }

    #[test]
    fn failed_release_remains_owned_until_a_later_successful_retry() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let rules = rules();
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        assert!(engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: Instant::now() }, None));
        record.lock().unwrap().limit = Some(0);
        for _ in 0..3 {
            assert!(!engine.fail());
            assert!(!record.lock().unwrap().held.is_empty());
        }
        record.lock().unwrap().limit = None;
        assert!(engine.fail());
        assert!(record.lock().unwrap().held.is_empty());
        assert!(engine.fence(1, Destination::Local, Some(&rules)));
    }

    #[test]
    fn closed_home_gate_matches_main_for_random_repeats_and_unowned_ups() {
        let mut random = 0x7249_6291_u64;
        let handoff = Handoff::default();
        let mut admission = Admission::new(handoff.hook_gate());
        let mut physical = Physical::default();
        let rules = Rules::default();
        for _ in 0..100_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let code = [14, 28, 30, 42, 125][(random as usize >> 4) % 5];
            let key = physical_key(random & 1 != 0);
            assert!(!physical.key(key, code, Instant::now(), &rules, &mut admission, |_| true));
            assert!(admission.pop().is_none());
        }
    }

    #[test]
    fn fence_recovers_sequence_gaps_without_repressing_held_keys() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let rules = rules();
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        let now = Instant::now();
        assert!(engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: now }, None));
        assert!(engine.fence(1, Destination::Local, Some(&rules)));
        let downs = record.lock().unwrap().downs;
        assert!(engine.key(Key { keyboard_type: 0, sequence: 2, code: 58, down: true, at: now }, None));
        assert_eq!(record.lock().unwrap().downs, downs);
        assert!(!engine.key(Key { keyboard_type: 0, sequence: 4, code: 58, down: false, at: now }, None));
        assert!(engine.fence(4, Destination::Local, Some(&rules)));
        assert!(engine.key(Key { keyboard_type: 0, sequence: 5, code: 58, down: false, at: now }, None));
    }

    #[test]
    fn independent_watchdog_is_idle_until_work_and_later_admissions_do_not_postpone_it() {
        let engine = Engine::new(Fake(Arc::new(Mutex::new(Record::default()))));
        let mut admission = Admission::new(engine.gate());
        let feedback = admission.feedback();
        let mut lease = WatchLease::default();
        let now = Instant::now();
        assert_eq!(lease.deadline(&feedback, now), None);
        assert!(admission.admit(58, true, now));
        assert_eq!(lease.deadline(&feedback, now), Some(now + LEASE));
        assert!(admission.admit(58, false, now + LEASE / 2));
        assert_eq!(lease.deadline(&feedback, now + LEASE / 2), Some(now + LEASE));
        assert!(!lease.expired(&feedback, now + LEASE / 2));
        assert!(lease.expired(&feedback, now + LEASE));
        feedback.acknowledge(2);
        feedback.acknowledge(1);
        assert!(feedback.acknowledged(2));
        assert_eq!(lease.deadline(&feedback, now + LEASE), None);
        assert!(!lease.expired(&feedback, now + LEASE * 10));
    }

    #[test]
    fn local_validation_refuses_actions_and_unsupported_outputs() {
        assert!(rules().maps.supports_local_keys(|_| true));
        assert!(!rules().maps.supports_local_keys(|code| code != 29));
        let map = Keymap::compile(&[RuleSpec { from: "capslock".parse().unwrap(), layer: None,
            what: What::To("switch next".parse().unwrap()),
        }], Target { os: one_mk_proto::os::WINDOWS, swap: false }, Duration::from_millis(200)).unwrap().map;
        assert!(!map.supports_local_keys(|_| true));
    }

    #[test]
    fn unreceipted_post_never_acknowledges_even_an_empty_remote_fence() {
        struct Posted { settled: bool }
        impl Injector for Posted {
            fn inject(&mut self, keys: &[(u32, bool)]) -> usize { keys.len() }
            fn settled(&self) -> bool { self.settled }
        }
        let mut engine = Engine::new(Posted { settled: true });
        assert!(engine.fence(0, Destination::Local, Some(&rules())));
        engine.injector.settled = false;
        assert!(!engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: Instant::now() }, None));
        assert!(!engine.fence(1, Destination::Remote, None));
        assert_eq!(engine.handoff.state(), State::Disabled);
        engine.injector.settled = true;
        assert!(engine.fence(1, Destination::Remote, None));
    }

    #[test]
    fn macos_modifier_streams_health_failure_and_fences_use_the_production_gate() {
        let map = Keymap::compile(&[RuleSpec { from: "lshift".parse().unwrap(), layer: None,
            what: What::To("lctrl".parse().unwrap()),
        }], Target { os: one_mk_proto::os::MACOS, swap: false }, Duration::from_millis(200)).unwrap().map;
        let rules = Rules { maps: KeymapSet::new(map), count: 1, ..Rules::default() };
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let mut admission = Admission::new(engine.gate());
        let feedback = admission.feedback();
        let mut hook = Physical::default();
        let mut flags = crate::macos_localmap_logic::Flags::default();
        let mut modifiers = crate::macos_logic::Modifiers::default();
        let mut random = 0x7220_7173_u64;
        let now = Instant::now();
        assert!(engine.recover_fence(0, Destination::Local, Some(&rules), &feedback, feedback.token()));
        for _ in 0..30_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            match random % 19 {
                0 | 1 => {
                    admission.fail();
                    while let Some(key) = admission.pop() { engine.admitted_key(key, None, &feedback); }
                    assert!(engine.fail());
                }
                2 | 3 => {
                    admission.gate.close();
                    while let Some(key) = admission.pop() { engine.admitted_key(key, None, &feedback); }
                    let destination = if random & 32 == 0 { Destination::Remote } else { Destination::Local };
                    assert!(engine.recover_fence(admission.watermark(), destination, Some(&rules), &feedback, feedback.token()));
                }
                _ => {
                    let down = random & 32 != 0;
                    modifiers.update(42, down);
                    let decoded = crate::macos_logic::modifier_down(0x38, modifiers.flags()).unwrap();
                    assert_eq!(decoded, down);
                    let swallowed = hook.key(physical(decoded, true), 42, now, &rules, &mut admission, |_| true);
                    flags.key(42, decoded, swallowed);
                    while let Some(key) = admission.pop() { assert!(engine.admitted_key(key, None, &feedback)); }
                }
            }
            if engine.handoff.state() == State::Remote { assert!(record.lock().unwrap().held.is_empty()); }
        }
        assert!(engine.fail());
        flags.key(42, false, true);
        assert_eq!(flags.apply(crate::macos_logic::flags::SHIFT | crate::macos_logic::flags::DEV_LSHIFT, 0), 0);
        let record = record.lock().unwrap();
        assert_eq!(record.downs, record.ups);
    }

    #[test]
    fn recovery_discards_failed_queue_and_rejects_stale_generation() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let mut admission = Admission::new(engine.gate());
        let feedback = admission.feedback();
        assert!(engine.recover_fence(0, Destination::Local, Some(&rules()), &feedback, feedback.token()));
        assert!(admission.admit(58, true, Instant::now()));
        admission.fail();
        let stale = feedback.token();
        assert!(engine.admitted_key(admission.pop().unwrap(), None, &feedback));
        assert_eq!(record.lock().unwrap().downs, 0);
        feedback.fail();
        assert!(!engine.recover_fence(1, Destination::Local, Some(&rules()), &feedback, stale));
        assert!(!feedback.healthy());
        assert_eq!(engine.handoff.state(), State::Disabled);
        assert!(engine.recover_fence(1, Destination::Local, Some(&rules()), &feedback, feedback.token()));
        assert!(feedback.healthy());
        assert!(admission.admit(58, true, Instant::now()));
        assert!(engine.admitted_key(admission.pop().unwrap(), None, &feedback));
        assert!(record.lock().unwrap().downs > 0);
        assert!(engine.fail());
        let record = record.lock().unwrap();
        assert_eq!(record.downs, record.ups);
    }

    #[test]
    fn failure_during_release_cannot_reopen_recovery_gate() {
        struct FailOnRelease { feedback: Arc<Feedback>, fail: bool }
        impl Injector for FailOnRelease {
            fn inject(&mut self, keys: &[(u32, bool)]) -> usize {
                if self.fail && keys.iter().any(|(_, down)| !down) { self.feedback.fail(); }
                keys.len()
            }
        }
        let gate = Handoff::default().hook_gate();
        let admission = Admission::new(gate);
        let feedback = admission.feedback();
        let mut engine = Engine::new(FailOnRelease { feedback: feedback.clone(), fail: false });
        assert!(engine.recover_fence(0, Destination::Local, Some(&rules()), &feedback, feedback.token()));
        assert!(engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: Instant::now() }, None));
        engine.injector.fail = true;
        let health = feedback.token();
        assert!(!engine.recover_fence(1, Destination::Local, None, &feedback, health));
        assert!(!feedback.healthy());
        assert_eq!(engine.handoff.state(), State::Disabled);
        assert!(engine.held.is_empty());
        assert!(!feedback.recover(health));
    }

    #[test]
    fn queued_down_and_partial_release_cannot_acknowledge_remote() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        assert!(engine.fence(0, Destination::Local, Some(&rules())));
        let gate = engine.gate();
        gate.close();
        assert!(!engine.fence(1, Destination::Remote, None));
        assert!(record.lock().unwrap().held.is_empty());
        assert!(engine.fence(0, Destination::Local, None));
        assert!(engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: Instant::now() }, None));
        record.lock().unwrap().limit = Some(1);
        assert!(!engine.fence(1, Destination::Remote, None));
        assert_eq!(engine.handoff.state(), State::Disabled);
        assert_eq!(record.lock().unwrap().held.len(), 1);
        record.lock().unwrap().limit = None;
        assert!(engine.fence(1, Destination::Remote, None));
        assert!(record.lock().unwrap().held.is_empty());
    }

    #[test]
    fn admission_is_bounded_and_lease_has_no_idle_expiry() {
        let handoff = Handoff::default();
        let mut admission = Admission::new(handoff.hook_gate());
        let now = Instant::now();
        assert!(admission.healthy(now + Duration::from_secs(86400)));
        for _ in 0..CAPACITY { assert!(admission.admit(30, true, now)); }
        assert!(!admission.admit(30, true, now));
        assert!(!admission.healthy(now));
        admission.recover();
        admission.acknowledge(admission.watermark());
        while admission.pop().is_some() {}
        assert!(admission.admit(30, false, now));
        assert!(!admission.healthy(now + LEASE));
    }

    #[test]
    fn randomized_queue_fences_failures_and_device_sources_balance() {
        for seed in 1..=64_u64 {
            let record = Arc::new(Mutex::new(Record::default()));
            let mut engine = Engine::new(Fake(record.clone()));
            assert!(engine.fence(0, Destination::Local, Some(&rules())));
            let mut random = seed;
            let mut sequence = 0;
            for _ in 0..4096 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                match random % 12 {
                    0 => { engine.fence(sequence, Destination::Remote, None); }
                    1 => { engine.fence(sequence, Destination::Local, Some(&rules())); }
                    2 => { engine.fail(); }
                    3 => { engine.timer(Instant::now()); }
                    _ => {
                        sequence += 1;
                        engine.key(Key {
                            keyboard_type: 0, sequence, code: [58, 30, 42, 100][((random >> 8) % 4) as usize],
                            down: random & 64 != 0, at: Instant::now(),
                        }, Some(DeviceRef(1 + ((random >> 16) % 3) as u32)));
                    }
                }
                if engine.handoff.state() == State::Remote {
                    assert!(record.lock().unwrap().held.is_empty());
                    assert!(engine.deadline().is_none());
                }
            }
            assert!(engine.fail());
            let record = record.lock().unwrap();
            assert!(record.held.is_empty());
            assert_eq!(record.downs, record.ups, "seed {seed}");
        }
    }

    #[test]
    fn snapshots_keep_borrowed_old_generation_until_off_hook_reclamation() {
        let mut snapshots = Snapshots::default();
        let old = snapshots.load();
        snapshots.replace(Arc::new(rules()));
        assert_eq!(old.count, 0);
        assert_eq!(snapshots.load().count, 1);
        assert_eq!(snapshots.retired.len(), 1);
        drop(old);
        snapshots.replace(Arc::new(Rules::default()));
        assert!(snapshots.retired.is_empty());
    }

    fn physical(down: bool, healthy: bool) -> PhysicalKey {
        PhysicalKey { down, injected: false, shortcut: false, eligible: true, healthy, owner: Owner::Unknown }
    }

    #[test]
    fn physical_emergency_passes_with_mapped_modifiers_and_escape() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let specs: Vec<_> = [("lctrl", "z"), ("lalt", "x"), ("lshift", "c"), ("esc", "a")].into_iter()
            .map(|(from, to)| RuleSpec { from: from.parse().unwrap(), layer: None, what: What::To(to.parse().unwrap()) }).collect();
        let rules = Rules { maps: KeymapSet::new(Keymap::compile(&specs, Target { os: one_mk_proto::os::WINDOWS, swap: false }, Duration::from_millis(200)).unwrap().map), count: specs.len(), hotkeys: Bindings::default() };
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        let mut admission = Admission::new(engine.gate());
        let mut hook = Physical::default();
        let now = Instant::now();
        for code in [29, 56, 42] {
            assert!(hook.key(physical(true, true), code, now, &rules, &mut admission, |_| panic!("not a shortcut")));
            let key = admission.pop().unwrap();
            assert!(engine.key(key, None));
            admission.acknowledge(key.sequence);
        }
        let mut actions = Vec::new();
        assert!(!hook.key(physical(true, true), 1, now, &rules, &mut admission, |action| { actions.push(action); true }));
        assert_eq!(actions, [Action::Home]);
        assert!(engine.fail());
        assert!(!hook.key(physical(false, false), 1, now, &rules, &mut admission, |_| true));
        for code in [29, 56, 42] {
            assert!(hook.key(physical(false, false), code, now, &rules, &mut admission, |_| true));
        }
        assert!(!hook.key(physical(true, false), 30, now, &rules, &mut admission, |_| true));
        assert!(record.lock().unwrap().held.is_empty());
    }

    #[test]
    fn overlapping_keyboards_release_both_sources_even_after_first_hook_up() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let rules = rules();
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        let mut admission = Admission::new(engine.gate());
        let mut hook = Physical::default();
        let now = Instant::now();
        for (device, down) in [(1, true), (2, true), (1, false), (2, false)] {
            assert!(hook.key(physical(down, true), 58, now, &rules, &mut admission, |_| true));
            let key = admission.pop().unwrap();
            hook.correlated(key.code, key.down, Some(DeviceRef(device)));
            assert!(engine.key(key, Some(DeviceRef(device))));
            admission.acknowledge(key.sequence);
        }
        let record = record.lock().unwrap();
        assert!(record.held.is_empty());
        assert_eq!(record.downs, record.ups);
    }

    #[test]
    fn actual_hook_queue_and_worker_randomized_failure_recovery() {
        for seed in 1..=64_u64 {
            let record = Arc::new(Mutex::new(Record::default()));
            let mut engine = Engine::new(Fake(record.clone()));
            let rules = rules();
            assert!(engine.fence(0, Destination::Local, Some(&rules)));
            let mut admission = Admission::new(engine.gate());
            let mut hook = Physical::default();
            let mut random = seed;
            let now = Instant::now();
            for _ in 0..4096 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                match random % 16 {
                    0..=2 => {
                        admission.gate.close();
                        while let Some(key) = admission.pop() {
                            assert!(engine.admitted_key(key, Some(DeviceRef(1)), &admission.feedback()));
                        }
                        let destination = if random & 32 == 0 { Destination::Local } else { Destination::Remote };
                        let feedback = admission.feedback();
                        engine.recover_fence(admission.watermark(), destination, Some(&rules), &feedback, feedback.token());
                    }
                    3 => {
                        admission.fail();
                        assert!(engine.fail());
                    }
                    4 => {
                        while let Some(key) = admission.pop() {
                            assert!(engine.admitted_key(key, Some(DeviceRef(1)), &admission.feedback()));
                        }
                    }
                    _ => {
                        let code = [58, 30, 42, 100][((random >> 8) % 4) as usize];
                        let down = random & 64 != 0;
                        let before = admission.watermark();
                        let had_owner = hook.owned.is_held(code) || hook.remote.is_held(code) || hook.passed.is_held(code);
                        if engine.handoff.state() == State::Remote {
                            if !hook.suppress_owned(code, down, &mut admission) && !hook.passed.is_held(code) {
                                hook.remote_key(code, down);
                            } else if !down { hook.passed.update(code, false); }
                            assert!(admission.watermark() == before || !down && admission.watermark() == before + 1);
                            continue;
                        }
                        let swallowed = hook.key(physical(down, true), code, now, &rules, &mut admission, |_| true);
                        if swallowed && !had_owner { assert!(admission.watermark() > before); }
                    }
                }
                if engine.handoff.state() == State::Remote {
                    assert!(record.lock().unwrap().held.is_empty());
                }
            }
            engine.fail();
            let record = record.lock().unwrap();
            assert_eq!(record.downs, record.ups, "seed {seed}");
        }
    }

    #[test]
    fn dual_role_owns_the_only_mapper_deadline_and_fence_cancels_tap() {
        let map = Keymap::compile(&[RuleSpec {
            from: "capslock".parse().unwrap(), layer: None,
            what: What::TapHold { tap: "esc".parse().unwrap(), hold: "lctrl".parse().unwrap(), timeout: None },
        }], Target { os: one_mk_proto::os::WINDOWS, swap: false }, Duration::from_millis(200)).unwrap().map;
        let rules = Rules { maps: KeymapSet::new(map), ..Rules::default() };
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        assert!(engine.deadline().is_none());
        let now = Instant::now();
        assert!(engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: now }, None));
        assert_eq!(engine.deadline(), Some(now + Duration::from_millis(200)));
        assert!(engine.fence(1, Destination::Remote, None));
        assert!(engine.deadline().is_none());
        assert_eq!(record.lock().unwrap().downs, 0);
        assert!(engine.fence(1, Destination::Local, Some(&rules)));
        assert!(engine.key(Key { keyboard_type: 0, sequence: 2, code: 58, down: true, at: now }, None));
        assert!(engine.timer(now + Duration::from_millis(200)));
        assert!(engine.deadline().is_none());
        assert!(record.lock().unwrap().held.is_empty());
        assert!(engine.key(Key { keyboard_type: 0, sequence: 3, code: 58, down: false, at: now }, None));
        assert!(engine.key(Key { keyboard_type: 0, sequence: 4, code: 58, down: true, at: now }, None));
        assert!(engine.timer(now + Duration::from_millis(200)));
        let held = record.lock().unwrap().held.contains(&29);
        assert!(engine.fail());
        assert!(held);
    }

    #[test]
    fn engine_unwind_releases_successfully_injected_keys() {
        let record = Arc::new(Mutex::new(Record::default()));
        let outside = record.clone();
        let unwind = std::panic::catch_unwind(move || {
            let mut engine = Engine::new(Fake(record));
            assert!(engine.fence(0, Destination::Local, Some(&rules())));
            assert!(engine.key(Key { keyboard_type: 0, sequence: 1, code: 58, down: true, at: Instant::now() }, None));
            panic!("worker panic after injection");
        });
        assert!(unwind.is_err());
        assert!(outside.lock().unwrap().held.is_empty());
    }

    #[test]
    fn unconfirmed_releases_retain_obligations_until_recovery() {
        struct LostUp { confirmed: bool }
        impl Injector for LostUp {
            fn inject(&mut self, keys: &[(u32, bool)]) -> usize { keys.len() }
            fn settled(&self) -> bool { self.confirmed }
        }
        let mut engine = Engine::new(LostUp { confirmed: false });
        engine.held.update(29, true);
        assert!(!engine.fail());
        assert!(engine.held.is_held(29));
        assert!(!engine.fence(0, Destination::Remote, None));
        assert_ne!(engine.handoff.state(), State::Remote);
        engine.injector.confirmed = true;
        assert!(engine.fence(0, Destination::Remote, None));
        assert!(!engine.held.is_held(29));
        assert_eq!(engine.handoff.state(), State::Remote);
    }

    #[test]
    fn missed_up_baseline_retires_only_absent_tombstones() {
        let record = Arc::new(Mutex::new(Record::default()));
        let mut engine = Engine::new(Fake(record.clone()));
        let rules = rules();
        assert!(engine.fence(0, Destination::Local, Some(&rules)));
        let now = Instant::now();
        assert!(engine.key(Key { code: 58, down: true, at: now, sequence: 1, keyboard_type: 0 }, None));
        assert!(engine.fail());
        engine.baseline(std::iter::once(58));
        assert!(engine.fence(1, Destination::Local, Some(&rules)));
        let downs = record.lock().unwrap().downs;
        assert!(engine.key(Key { code: 58, down: true, at: now, sequence: 2, keyboard_type: 0 }, None));
        assert_eq!(record.lock().unwrap().downs, downs, "held tombstone must not be repressed");
        assert!(engine.fail());
        engine.baseline(std::iter::empty());
        assert!(engine.fence(2, Destination::Local, Some(&rules)));
        let downs = record.lock().unwrap().downs;
        assert!(engine.key(Key { code: 58, down: true, at: now, sequence: 3, keyboard_type: 0 }, None));
        assert!(record.lock().unwrap().downs > downs, "fresh press must not be an old repeat");
    }
}
