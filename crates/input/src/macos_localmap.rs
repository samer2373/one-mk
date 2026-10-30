//! EXPERIMENTAL, COMPILE-CHECKED ONLY; native macOS execution is UNTESTED.

use crate::hotkeys::Action;
use crate::localmap_handoff::{Destination, HookGate, Owner, PhysicalKey};
use crate::localmap_runtime::{Admission, Engine, Feedback, Injector, Key, Physical, Rules, Snapshots, WatchLease, LEASE};
use crate::macos::{Inbox, Msg};
use crate::macos_localmap_logic::{self as logic, Flags};
use crate::{DeviceRef, Event};
use crossbeam_queue::ArrayQueue;
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventType};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

enum Work {
    Key(Key, Option<DeviceRef>),
    Fence { serial: u64, watermark: u64, health: u64, destination: Destination, rules: Arc<Rules> },
    Fail,
    Stop,
}

pub(crate) struct Completion {
    pub destination: Destination,
    pub success: bool,
    health: u64,
    serial: u64,
}

#[derive(Default)]
struct Receipts {
    receipt: logic::Receipt,
    output_flags: AtomicU64,
    physical_flags: AtomicU64,
    worker: OnceLock<std::thread::Thread>,
    feedback: OnceLock<Arc<Feedback>>,
    held: AtomicBool,
    physical_held: AtomicBool,
}

struct Native {
    receipts: Arc<Receipts>,
    sequence: u32,
    settled: bool,
    mods: crate::macos_logic::Modifiers,
    held: one_mk_proto::HeldKeys,
}

impl Injector for Native {
    fn inject(&mut self, keys: &[(u32, bool)]) -> usize {
        if keys.is_empty() { return 0; }
        let Some(source) = CGEventSource::new(CGEventSourceStateID::Private) else { return 0 };
        for (index, &(code, down)) in keys.iter().enumerate() {
            if down && (self.receipts.feedback.get().is_none_or(|feedback| !feedback.healthy())
                || crate::macos::secure_input() || crate::macos::session_inactive()
                || !crate::macos::ax_trusted(false) || !objc2_core_graphics::CGPreflightListenEventAccess()) { return index; }
            let Some(kc) = crate::macos_logic::evdev_to_mac(code) else { self.settled = false; return index };
            let Some(sequence) = self.sequence.checked_add(1) else { self.settled = false; return index };
            let Some(event) = CGEvent::new_keyboard_event(Some(&source), kc, down) else {
                return index;
            };
            self.sequence = sequence;
            self.held.update(code, down);
            self.receipts.held.store(!self.held.is_empty(), Ordering::Release);
            self.mods.update(code, down);
            let flags = self.mods.flags();
            self.receipts.output_flags.store(flags, Ordering::Release);
            if crate::macos_logic::is_mac_modifier(kc) { CGEvent::set_type(Some(&event), CGEventType::FlagsChanged); }
            CGEvent::set_flags(Some(&event), CGEventFlags(flags | self.receipts.physical_flags.load(Ordering::Acquire)));
            CGEvent::set_integer_value_field(Some(&event), CGEventField::EventSourceUserData, (logic::TAG | u64::from(sequence)) as i64);
            self.receipts.receipt.arm(sequence);
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
            let deadline = Instant::now() + LEASE;
            while !self.receipts.receipt.seen(sequence) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    self.settled = false;
                    return index + usize::from(down);
                }
                std::thread::park_timeout(remaining);
            }
            self.settled = true;
        }
        keys.len()
    }

    fn settled(&self) -> bool { self.settled }
}

pub(crate) struct Dispatch {
    work: mpsc::Sender<Work>,
    waiting: VecDeque<Key>,
    feedback: Arc<Feedback>,
}

pub(crate) struct Interlock {
    gate: Arc<HookGate>,
    feedback: Arc<Feedback>,
    work: mpsc::Sender<Work>,
}

impl Interlock {
    pub fn fail(&self) {
        self.gate.close();
        self.feedback.fail();
        let _ = self.work.send(Work::Fail);
    }
}

impl Dispatch {
    pub fn fail(&self) {
        self.feedback.fail();
        let _ = self.work.send(Work::Fail);
    }

    pub fn queue(&mut self, key: Key) { self.waiting.push_back(key); }

    pub fn forward(&mut self, event: &Event) -> bool {
        let Event::Key { code, down, device } = *event else { return false };
        let Some(key) = self.waiting.pop_front() else { return false };
        if key.code != code || key.down != down || self.work.send(Work::Key(key, device)).is_err() {
            self.feedback.fail();
            let _ = self.work.send(Work::Fail);
        }
        true
    }
}

pub(crate) struct Local {
    pub admission: Admission,
    pub gate: Arc<HookGate>,
    pub snapshots: Snapshots,
    pub enabled: bool,
    pub paused: bool,
    pub inactive: String,
    pub activation: Option<(crate::Side, one_mk_proto::Pos)>,
    pub pending: Option<(u64, Instant, u64)>,
    physical: Physical,
    down: one_mk_proto::HeldKeys,
    flags: Flags,
    receipts: Arc<Receipts>,
    actions: ArrayQueue<Action>,
    serial: u64,
    work: mpsc::Sender<Work>,
    completions: mpsc::Receiver<Completion>,
    thread: Option<std::thread::JoinHandle<()>>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    failed: bool,
}

impl Local {
    pub fn interlock(&self) -> Interlock {
        Interlock { gate: self.gate.clone(), feedback: self.admission.feedback(), work: self.work.clone() }
    }

    pub fn start(inbox: Arc<Inbox>) -> std::io::Result<(Self, Dispatch)> {
        let receipts = Arc::new(Receipts::default());
        let mut engine = Engine::new(Native { receipts: receipts.clone(), sequence: 0, settled: true, mods: Default::default(), held: one_mk_proto::HeldKeys::new() });
        let gate = engine.gate();
        let admission = Admission::new(gate.clone());
        let feedback = admission.feedback();
        let _ = receipts.feedback.set(feedback.clone());
        let (work, receiver) = mpsc::channel();
        let (completion, completions) = mpsc::channel();
        let worker_receipts = receipts.clone();
        let worker_feedback = feedback.clone();
        let worker_inbox = inbox.clone();
        let thread = std::thread::Builder::new().name("one-mk-local-keys".into()).spawn(move || {
            let _ = worker_receipts.worker.set(std::thread::current());
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&mut engine, &worker_feedback, receiver, |done| {
                    let _ = completion.send(done);
                }, || worker_inbox.signal());
            }));
            worker_feedback.fail();
            engine.fail();
            worker_inbox.post(Msg::Local);
        })?;
        let stopped = Arc::new(AtomicBool::new(false));
        let watchdog_stopped = stopped.clone();
        let watchdog_feedback = feedback.clone();
        let watchdog_work = work.clone();
        let watchdog_gate = gate.clone();
        let watchdog_receipts = receipts.clone();
        let watchdog = std::thread::Builder::new().name("one-mk-local-lease".into()).spawn(move || {
            let mut lease = WatchLease::default();
            while !watchdog_stopped.load(Ordering::Acquire) {
                if (watchdog_receipts.held.load(Ordering::Acquire) || watchdog_receipts.physical_held.load(Ordering::Acquire))
                    && watchdog_feedback.healthy() && (crate::macos::secure_input() || crate::macos::session_inactive()) {
                    watchdog_feedback.fail();
                    watchdog_gate.close();
                    let _ = watchdog_work.send(Work::Fail);
                    inbox.post(Msg::Local);
                }
                if lease.expired(&watchdog_feedback, Instant::now()) {
                    watchdog_feedback.fail();
                    watchdog_gate.close();
                    let _ = watchdog_work.send(Work::Fail);
                    inbox.post(Msg::Local);
                    std::thread::park();
                } else if let Some(deadline) = lease.deadline(&watchdog_feedback, Instant::now()) {
                    std::thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
                } else if watchdog_receipts.held.load(Ordering::Acquire) || watchdog_receipts.physical_held.load(Ordering::Acquire) {
                    std::thread::park_timeout(LEASE);
                } else { std::thread::park(); }
            }
        });
        let watchdog = match watchdog {
            Ok(watchdog) => watchdog,
            Err(error) => { let _ = work.send(Work::Stop); let _ = thread.join(); return Err(error); }
        };
        let dispatch = Dispatch { work: work.clone(), waiting: VecDeque::new(), feedback };
        Ok((Self {
            admission, gate, snapshots: Snapshots::default(), enabled: false, paused: false,
            inactive: "preference off".into(), activation: None, pending: None,
            physical: Physical::default(), down: one_mk_proto::HeldKeys::new(), flags: Flags::default(), receipts, actions: ArrayQueue::new(16),
            serial: 0, work, completions, thread: Some(thread), watchdog: Some(watchdog), stopped, failed: false,
        }, dispatch))
    }

    pub fn supported(rules: &Rules) -> bool { rules.maps.supports_local_keys(logic::eligible) }

    pub fn baseline(&mut self, held: impl Iterator<Item = u32>) {
        let held: Vec<_> = held.collect();
        self.flags = Flags::default();
        self.down = one_mk_proto::HeldKeys::new();
        for &code in &held { self.flags.key(code, true, false); self.down.update(code, true); }
        self.receipts.physical_held.store(!self.down.is_empty(), Ordering::Release);
        self.physical.baseline(held.into_iter());
    }

    pub fn receipt(&self, sequence: u32) {
        if self.receipts.receipt.observe(sequence) && let Some(worker) = self.receipts.worker.get() { worker.unpark(); }
    }

    pub fn held(&self) -> bool { self.receipts.held.load(Ordering::Acquire) }

    pub fn hook(&mut self, code: u32, down: bool, injected: bool, flags: u64, remote: bool) -> bool {
        if injected { return false; }
        self.down.update(code, down);
        self.receipts.physical_held.store(!self.down.is_empty(), Ordering::Release);
        let rules = self.snapshots.load();
        let swallowed = if remote {
            if let Some(swallow) = self.physical.shortcut(code, down, &rules, &self.admission, |action| self.actions.push(action).is_ok()) { swallow }
            else if self.physical.suppress_owned(code, down) { true }
            else {
                if self.physical.os_owned(code) {
                    if !down { self.physical.os_release(code); }
                } else { self.physical.remote_key(code, down); }
                false
            }
        } else {
            self.physical.key(PhysicalKey {
                down, injected, shortcut: false, owner: Owner::Unknown, eligible: logic::eligible(code),
                healthy: self.enabled && !self.paused,
            }, code, Instant::now(), &rules, &mut self.admission, |action| self.actions.push(action).is_ok())
        };
        if !injected {
            self.flags.key(code, down, swallowed);
            self.receipts.physical_flags.store(self.flags.apply(flags, 0), Ordering::Release);
        }
        if let Some(watchdog) = &self.watchdog { watchdog.thread().unpark(); }
        swallowed
    }

    pub fn flags(&self, original: u64) -> u64 {
        let physical = self.flags.apply(original, 0);
        self.receipts.physical_flags.store(physical, Ordering::Release);
        physical | self.receipts.output_flags.load(Ordering::Acquire)
    }

    pub fn action(&self) -> Option<Action> { self.actions.pop() }

    pub fn fail(&mut self) {
        self.admission.fail();
        self.activation = None;
        self.pending = None;
        self.paused = true;
        self.receipts.physical_held.store(false, Ordering::Release);
        if !self.failed { let _ = self.work.send(Work::Fail); }
        self.failed = true;
    }

    pub fn fence(&mut self, destination: Destination) {
        let Some(serial) = self.serial.checked_add(1) else { self.fail(); return };
        self.serial = serial;
        let health = self.admission.feedback().token();
        self.pending = Some((serial, Instant::now(), health));
        self.failed = false;
        if self.work.send(Work::Fence { serial, watermark: self.admission.watermark(), health, destination, rules: self.snapshots.load() }).is_err() { self.fail(); }
    }

    pub fn completion(&mut self) -> Option<Completion> {
        while let Ok(done) = self.completions.try_recv() {
            if self.pending.is_some_and(|(serial, _, _)| serial == done.serial) {
                self.pending = None;
                let healthy = self.admission.feedback().token() == done.health;
                return Some(Completion { success: done.success && healthy, ..done });
            }
        }
        None
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        self.admission.fail();
        self.stopped.store(true, Ordering::Release);
        if let Some(watchdog) = self.watchdog.take() { watchdog.thread().unpark(); let _ = watchdog.join(); }
        let _ = self.work.send(Work::Stop);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

fn run(engine: &mut Engine<Native>, feedback: &Feedback, receiver: mpsc::Receiver<Work>, mut complete: impl FnMut(Completion), notify: impl Fn()) {
    loop {
        let next = match engine.deadline() {
            Some(deadline) => match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(work) => Some(work),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !feedback.healthy() { engine.fail(); }
                    else if !engine.timer(Instant::now()) { feedback.fail(); }
                    notify();
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            },
            None => receiver.recv().ok(),
        };
        match next {
            Some(Work::Key(key, device)) => { engine.admitted_key(key, device, feedback); }
            Some(Work::Fence { serial, watermark, health, destination, rules }) => {
                let success = engine.recover_fence(watermark, destination, Some(&rules), feedback, health);
                complete(Completion { serial, destination, success, health: health & !1 });
            }
            Some(Work::Fail) => { engine.fail(); }
            Some(Work::Stop) | None => break,
        }
        notify();
    }
}
