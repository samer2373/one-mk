//! Local injection ownership fence. This is not a native hook implementation.
//! See `docs/LOCAL-REMAP.md` for the required native acknowledgement contract.

use std::sync::atomic::{AtomicU64, Ordering};

static TICKETS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

impl Ticket {
    fn next(counter: &AtomicU64) -> Option<Self> {
        counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |serial| serial.checked_add(1)).ok().map(Self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Local,
    Handoff { ticket: Ticket, destination: Destination },
    Remote,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Unknown,
    Os,
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalKey {
    pub down: bool,
    pub injected: bool,
    pub shortcut: bool,
    pub eligible: bool,
    pub healthy: bool,
    pub owner: Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Pass,
    Suppress,
    Local,
    Remote,
    Shortcut,
}

pub struct Handoff {
    state: State,
}

impl Default for Handoff {
    fn default() -> Self {
        Self { state: State::Disabled }
    }
}

impl Handoff {
    pub fn state(&self) -> State {
        self.state
    }

    /// Close both gates before releasing any local output. The caller submits
    /// a fence after these releases through the same serialized injector.
    /// Exhausting the ticket namespace leaves interception disabled forever.
    #[must_use]
    pub fn begin(&mut self, destination: Destination, release: impl FnOnce()) -> Option<Ticket> {
        self.state = State::Disabled;
        release();
        let ticket = Ticket::next(&TICKETS)?;
        self.state = State::Handoff { ticket, destination };
        Some(ticket)
    }

    /// Only the serialized injector may acknowledge a completed batch. A queue
    /// send, callback timeout, or partial injection is not a successful batch.
    pub fn acknowledge(&mut self, ticket: Ticket) -> bool {
        let State::Handoff { ticket: expected, destination } = self.state else { return false };
        if expected != ticket {
            return false;
        }
        self.state = match destination {
            Destination::Local => State::Local,
            Destination::Remote => State::Remote,
        };
        true
    }

    /// Invalidate the outstanding fence before cleanup. Recovery requires a
    /// fresh begin/ack pair; a late acknowledgement cannot re-enable input.
    pub fn fail(&mut self, release: impl FnOnce()) {
        self.state = State::Disabled;
        release();
    }

    /// Constant-size, allocation-free policy. `owner` is the original physical
    /// down's owner, retained per device and device-table generation until up.
    /// Local/Shortcut mean queue work, not permission to block in the callback.
    pub fn decide(&self, key: PhysicalKey) -> Route {
        if key.injected {
            return Route::Pass;
        }
        if key.shortcut {
            return Route::Shortcut;
        }
        match key.owner {
            Owner::Os => {
                if key.down && self.state == State::Remote { Route::Suppress } else { Route::Pass }
            }
            Owner::Local => {
                if self.state == State::Local && key.healthy { Route::Local } else { Route::Suppress }
            }
            Owner::Remote => {
                if self.state == State::Remote { Route::Remote } else { Route::Suppress }
            }
            Owner::Unknown => {
                if !key.down {
                    Route::Pass
                } else if self.state == State::Remote {
                    Route::Remote
                } else if self.state == State::Local && key.healthy && key.eligible {
                    Route::Local
                } else {
                    Route::Pass
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::device::DeviceRef;
    use one_mk_proto::remap::{Emit, Keymap, KeymapSet, Mapper, RuleSpec, Target, What};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn key(owner: Owner, down: bool) -> PhysicalKey {
        PhysicalKey { owner, down, injected: false, shortcut: false, eligible: true, healthy: true }
    }

    #[test]
    fn every_state_and_transition_event() {
        for initial in [State::Disabled, State::Local, State::Remote,
            State::Handoff { ticket: Ticket(7), destination: Destination::Local },
            State::Handoff { ticket: Ticket(7), destination: Destination::Remote }]
        {
            for destination in [Destination::Local, Destination::Remote] {
                let mut gate = Handoff { state: initial };
                assert!(!gate.acknowledge(Ticket(u64::MAX)));
                assert_eq!(gate.state(), initial);
                let mut releases = 0;
                let ticket = gate.begin(destination, || releases += 1).unwrap();
                assert_eq!(releases, 1);
                assert_eq!(gate.state(), State::Handoff { ticket, destination });
                assert!(!gate.acknowledge(Ticket(u64::MAX)));
                assert!(gate.acknowledge(ticket));
                assert_eq!(gate.state(), match destination { Destination::Local => State::Local, Destination::Remote => State::Remote });
                assert!(!gate.acknowledge(ticket));
                gate.fail(|| releases += 1);
                assert_eq!(gate.state(), State::Disabled);
                assert_eq!(releases, 2);
                assert!(!gate.acknowledge(ticket));
            }
            let mut gate = Handoff { state: initial };
            gate.fail(|| {});
            assert_eq!(gate.state(), State::Disabled);
            assert!(!gate.acknowledge(Ticket(7)));
        }
    }

    #[test]
    fn held_key_matrix_and_all_bypass_combinations() {
        for state in [State::Local, State::Remote, State::Disabled,
            State::Handoff { ticket: Ticket(1), destination: Destination::Remote }]
        {
            let gate = Handoff { state };
            for owner in [Owner::Unknown, Owner::Os, Owner::Local, Owner::Remote] {
                for down in [false, true] {
                    for healthy in [false, true] {
                        for eligible in [false, true] {
                            let event = PhysicalKey { healthy, eligible, ..key(owner, down) };
                            let expected = match owner {
                                Owner::Os if down && state == State::Remote => Route::Suppress,
                                Owner::Os => Route::Pass,
                                Owner::Local if state == State::Local && healthy => Route::Local,
                                Owner::Local => Route::Suppress,
                                Owner::Remote if state == State::Remote => Route::Remote,
                                Owner::Remote => Route::Suppress,
                                Owner::Unknown if !down => Route::Pass,
                                Owner::Unknown if state == State::Remote => Route::Remote,
                                Owner::Unknown if state == State::Local && healthy && eligible => Route::Local,
                                Owner::Unknown => Route::Pass,
                            };
                            assert_eq!(gate.decide(event), expected, "{state:?} {event:?}");
                            assert_eq!(gate.decide(PhysicalKey { shortcut: true, ..event }), Route::Shortcut);
                            assert_eq!(gate.decide(PhysicalKey { injected: true, ..event }), Route::Pass);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn superseded_failed_and_exhausted_tickets_never_open_the_gate() {
        let mut gate = Handoff::default();
        let old = gate.begin(Destination::Remote, || {}).unwrap();
        let current = gate.begin(Destination::Local, || {}).unwrap();
        assert!(!gate.acknowledge(old));
        gate.fail(|| {});
        assert!(!gate.acknowledge(current));
        assert_eq!(Ticket::next(&AtomicU64::new(u64::MAX)), None);
        assert_eq!(gate.state(), State::Disabled);
        let mut other = Handoff::default();
        let foreign = other.begin(Destination::Local, || {}).unwrap();
        let own = gate.begin(Destination::Remote, || {}).unwrap();
        assert!(!gate.acknowledge(foreign));
        assert!(!other.acknowledge(own));
        assert!(gate.acknowledge(own));
    }

    #[test]
    fn cleanup_panic_keeps_gate_closed() {
        let mut gate = Handoff::default();
        let ticket = gate.begin(Destination::Local, || {}).unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = gate.begin(Destination::Remote, || panic!("injection worker failed"));
        })).is_err());
        assert_eq!(gate.state(), State::Disabled);
        assert!(!gate.acknowledge(ticket));
    }

    struct Harness {
        gate: Handoff,
        mapper: Mapper,
        owners: HashMap<(u32, u32), Owner>,
        injected: HashMap<u32, bool>,
        downs: usize,
        ups: usize,
    }

    impl Harness {
        fn new() -> Self {
            let compile = |from: &str, to: &str| Keymap::compile(&[RuleSpec {
                from: from.parse().unwrap(), layer: None, what: What::To(to.parse().unwrap()),
            }], Target { os: one_mk_proto::os::WINDOWS, swap: false }, Duration::from_millis(200)).unwrap().map;
            let mut set = KeymapSet::new(compile("a", "b"));
            set.add(&[DeviceRef(2)], compile("a", "c"));
            Self {
                gate: Handoff::default(), mapper: Mapper::with_set(set), owners: HashMap::new(),
                injected: HashMap::new(), downs: 0, ups: 0,
            }
        }

        fn emitted(&mut self, events: Vec<Emit>) {
            for event in events {
                let Emit::Key { code, down } = event else { panic!("unexpected output") };
                let previous = self.injected.insert(code, down).unwrap_or(false);
                if down {
                    if !previous { self.downs += 1; }
                } else {
                    assert!(previous, "unmatched injected release: {code}");
                    self.ups += 1;
                }
            }
        }

        fn begin(&mut self, destination: Destination) -> Ticket {
            let mut events = Vec::new();
            let ticket = self.gate.begin(destination, || self.mapper.release_all(&mut |event| events.push(event))).unwrap();
            self.emitted(events);
            assert!(self.mapper.is_idle());
            ticket
        }

        fn fail(&mut self) {
            let mut events = Vec::new();
            self.gate.fail(|| self.mapper.release_all(&mut |event| events.push(event)));
            self.emitted(events);
        }

        fn physical(&mut self, device: u32, down: bool, healthy: bool, shortcut: bool) {
            if !healthy {
                self.fail();
            }
            let identity = (device, 30);
            let owner = self.owners.get(&identity).copied().unwrap_or(Owner::Unknown);
            let route = self.gate.decide(PhysicalKey { healthy, shortcut, ..key(owner, down) });
            if shortcut {
                assert_eq!(route, Route::Shortcut);
                self.fail();
            }
            let mut events = Vec::new();
            if route == Route::Local {
                assert_eq!(self.gate.state(), State::Local);
                assert!(healthy);
                self.mapper.key_from(30, down, Some(DeviceRef(device)), Instant::now(), &mut |event| events.push(event));
            } else if owner == Owner::Local && !down {
                self.mapper.key_from(30, false, Some(DeviceRef(device)), Instant::now(), &mut |event| events.push(event));
                assert!(events.is_empty());
            }
            self.emitted(events);
            if down && owner == Owner::Unknown {
                self.owners.insert(identity, match route {
                    Route::Local => Owner::Local,
                    Route::Remote => Owner::Remote,
                    _ => Owner::Os,
                });
            } else if !down {
                self.owners.remove(&identity);
            }
            if self.gate.state() == State::Remote {
                assert!(self.mapper.is_idle());
                assert!(self.injected.values().all(|down| !down));
            }
        }
    }

    #[test]
    fn randomized_device_key_failure_and_acknowledgement_streams() {
        for seed in 1..=128_u64 {
            let mut random = seed;
            let mut harness = Harness::new();
            let mut ticket = harness.begin(Destination::Local);
            assert!(harness.gate.acknowledge(ticket));
            for _ in 0..2048 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                match random % 13 {
                    0 => ticket = harness.begin(Destination::Local),
                    1 => ticket = harness.begin(Destination::Remote),
                    2 => { harness.gate.acknowledge(ticket); }
                    3..=5 => harness.fail(),
                    6 => {
                        harness.fail();
                        harness.owners.clear();
                        harness.mapper = Mapper::with_set(harness.mapper.keymaps().clone());
                    }
                    _ => harness.physical(1 + ((random >> 8) % 4) as u32, random & 32 != 0, random & 64 != 0, random % 97 == 0),
                }
            }
            harness.fail();
            assert_eq!(harness.downs, harness.ups, "seed {seed}");
            assert!(harness.injected.values().all(|down| !down));
        }
    }
}
