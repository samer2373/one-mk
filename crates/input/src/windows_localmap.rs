//! EXPERIMENTAL, COMPILE-CHECKED ONLY; native Windows execution is UNTESTED.

use crate::hotkeys::Action;
use crate::localmap_handoff::{Destination, HookGate, Owner, PhysicalKey};
use crate::localmap_runtime::{Admission, Engine, Feedback, Injector, Key, Physical, Rules, Snapshots, WatchLease};
use crate::win_logic::{WinKey, evdev_to_win};
use crate::{DeviceRef, Event};
use crossbeam_queue::ArrayQueue;
use std::collections::VecDeque;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{CancelWaitableTimer, CreateEventW, CreateWaitableTimerExW, SetEvent, SetWaitableTimer, WaitForMultipleObjects, WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS, INFINITE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};

pub(crate) static PHYSICAL_HOTKEYS: AtomicBool = AtomicBool::new(false);

enum Work {
    Key(Key, Option<DeviceRef>),
    Fence { serial: u64, watermark: u64, health: u64, destination: Destination, rules: Arc<Rules>, baseline: Option<Vec<u32>> },
    Fail,
    Retire,
    Stop,
}

pub(crate) struct Completion {
    pub serial: u64,
    pub destination: Destination,
    pub success: bool,
}

struct Native;

struct Wake {
    event: usize,
    timer: usize,
    stopped: AtomicBool,
}

impl Wake {
    fn signal(&self) {
        // SAFETY: this Arc owns the event for the duration of the call.
        unsafe { SetEvent(self.event as HANDLE) };
    }
}

impl Drop for Wake {
    fn drop(&mut self) {
        // SAFETY: the last owner closes the event after both threads exit.
        unsafe { CloseHandle(self.event as HANDLE); CloseHandle(self.timer as HANDLE); }
    }
}

impl Injector for Native {
    fn inject(&mut self, keys: &[(u32, bool)]) -> usize {
        let mut inputs = Vec::<INPUT>::with_capacity(keys.len());
        for &(code, down) in keys {
            let Some(WinKey::Scan { scan, extended }) = evdev_to_win(code) else { return 0 };
            let flags = KEYEVENTF_SCANCODE | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 }
                | if down { 0 } else { KEYEVENTF_KEYUP };
            inputs.push(super::windows::key(0, scan, flags));
        }
        if inputs.is_empty() { return 0; }
        for (index, input) in inputs.iter().enumerate() {
            // SAFETY: one initialized INPUT; a return of one identifies an exact successful event.
            if unsafe { SendInput(1, input, size_of::<INPUT>() as i32) } != 1 { return index; }
        }
        inputs.len()
    }
}

pub(crate) struct Local {
    pub admission: Admission,
    pub gate: Arc<HookGate>,
    pub snapshots: Snapshots,
    pub waiting: VecDeque<Key>,
    pub pending: Option<(u64, Instant)>,
    pub enabled: bool,
    pub paused: bool,
    pub activation: Option<(crate::Side, i32, i32)>,
    pub input: bool,
    pub inactive: String,
    physical: Physical,
    baseline: Option<Vec<u32>>,
    actions: ArrayQueue<Action>,
    serial: u64,
    work: mpsc::Sender<Work>,
    completions: mpsc::Receiver<Completion>,
    thread: Option<std::thread::JoinHandle<()>>,
    desktop_hook: HWINEVENTHOOK,
    wake: Arc<Wake>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    failed: bool,
    pub recovery: u64,
}

pub(crate) struct Recovery {
    work: mpsc::Sender<Work>,
    thread: Option<std::thread::JoinHandle<()>>,
    desktop_hook: HWINEVENTHOOK,
}

impl Recovery {
    pub fn retry(&self) { let _ = self.work.send(Work::Fail); }
    pub fn finished(&self) -> bool { self.thread.as_ref().is_none_or(|thread| thread.is_finished()) }
}

impl Drop for Recovery {
    fn drop(&mut self) {
        unsafe { UnhookWinEvent(self.desktop_hook) };
        let _ = self.work.send(Work::Stop);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

impl Local {
    pub fn start(hwnd: HWND) -> std::io::Result<Self> {
        let engine = Engine::new(Native);
        let gate = engine.gate();
        let admission = Admission::new(gate.clone());
        let feedback = admission.feedback();
        // SAFETY: unnamed auto-reset event, no borrowed pointers retained.
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if event.is_null() { return Err(std::io::Error::last_os_error()); }
        let timer = unsafe { CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS) };
        if timer.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { CloseHandle(event) };
            return Err(error);
        }
        let wake = Arc::new(Wake { event: event as usize, timer: timer as usize, stopped: AtomicBool::new(false) });
        let (work, receiver) = mpsc::channel();
        let (completion, completions) = mpsc::channel();
        let hwnd = hwnd as usize;
        let worker_feedback = feedback.clone();
        let thread = std::thread::Builder::new().name("one-mk-local-keys".into()).spawn(move || {
            let feedback = worker_feedback;
            let mut engine = engine;
            run(&mut engine, &feedback, receiver, |done| {
                    let _ = completion.send(done);
                    // SAFETY: posting to the capture window, possibly already gone.
                    unsafe { PostMessageW(hwnd as HWND, super::windows::WM_APP_LOCAL, 0, 0) };
            });
            feedback.fail();
            engine.fail();
            unsafe { PostMessageW(hwnd as HWND, super::windows::WM_APP_LOCAL, 0, 0) };
        })?;
        // SAFETY: callback is static and delivered on the installing message-loop thread.
        let desktop_hook = unsafe { SetWinEventHook(3, 0x20, std::ptr::null_mut(), Some(super::windows::desktop_changed), 0, 0, 0) };
        if desktop_hook.is_null() {
            let _ = work.send(Work::Stop);
            let _ = thread.join();
            return Err(std::io::Error::last_os_error());
        }
        let watchdog_wake = wake.clone();
        let watchdog_work = work.clone();
        let watchdog_gate = gate.clone();
        let watchdog = std::thread::Builder::new().name("one-mk-local-lease".into()).spawn(move || {
            let mut lease = WatchLease::default();
            loop {
                if watchdog_wake.stopped.load(Ordering::Acquire) { break; }
                let deadline = lease.deadline(&feedback, Instant::now());
                let timeout = deadline.map_or(INFINITE, |at| at.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128 - 1) as u32);
                let timer = watchdog_wake.timer as HANDLE;
                let armed = unsafe {
                    if deadline.is_some() { SetWaitableTimer(timer, &-10_000, 0, None, std::ptr::null(), 0) != 0 }
                    else { CancelWaitableTimer(timer); true }
                };
                if !armed {
                    feedback.fail();
                    watchdog_gate.close();
                    let _ = watchdog_work.send(Work::Fail);
                    unsafe { PostMessageW(hwnd as HWND, super::windows::WM_APP_LOCAL, 0, 0) };
                    break;
                }
                // SAFETY: event is retained by this thread's Arc.
                let handles = [watchdog_wake.event as HANDLE, timer];
                let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout) };
                if result == 1 {
                    unsafe { PostMessageW(hwnd as HWND, super::windows::WM_APP_LOCAL, 0, 0) };
                }
                if result == WAIT_TIMEOUT {
                    if lease.expired(&feedback, Instant::now()) {
                        feedback.fail();
                        watchdog_gate.close();
                        let _ = watchdog_work.send(Work::Fail);
                        // SAFETY: wake only; cleanup does not depend on this window responding.
                        unsafe { PostMessageW(hwnd as HWND, super::windows::WM_APP_LOCAL, 0, 0) };
                        // Disarm until new hook work or shutdown; no periodic idle polling.
                        unsafe { WaitForSingleObject(watchdog_wake.event as HANDLE, INFINITE) };
                    }
                } else if result != 0 && result != 1 && result != WAIT_TIMEOUT {
                    feedback.fail();
                    watchdog_gate.close();
                    let _ = watchdog_work.send(Work::Fail);
                    break;
                }
            }
        });
        let watchdog = match watchdog {
            Ok(watchdog) => watchdog,
            Err(error) => {
                // SAFETY: undo our own registration on startup failure.
                unsafe { UnhookWinEvent(desktop_hook) };
                let _ = work.send(Work::Stop);
                let _ = thread.join();
                return Err(error);
            }
        };
        PHYSICAL_HOTKEYS.store(true, Ordering::Release);
        Ok(Self {
            admission, gate, snapshots: Snapshots::default(), waiting: VecDeque::new(), pending: None,
            enabled: false, paused: false, activation: None, input: false, inactive: "preference off".into(), physical: Physical::default(), baseline: None,
            actions: ArrayQueue::new(crate::localmap_runtime::CAPACITY), serial: 0, work, completions,
            thread: Some(thread), desktop_hook, wake, watchdog: Some(watchdog), failed: false, recovery: 0,
        })
    }

    pub fn supported(rules: &Rules) -> bool {
        rules.maps.supports_local_keys(|code| matches!(evdev_to_win(code), Some(WinKey::Scan { .. })))
    }

    pub fn retire(mut self) -> Recovery {
        self.gate.close();
        self.admission.fail();
        let _ = self.work.send(Work::Retire);
        Recovery {
            work: self.work.clone(), thread: self.thread.take(),
            desktop_hook: std::mem::replace(&mut self.desktop_hook, std::ptr::null_mut()),
        }
    }

    pub fn baseline(&mut self, held: impl Iterator<Item = u32>) {
        let held: Vec<_> = held.collect();
        self.physical.baseline(held.iter().copied());
        self.baseline = Some(held);
    }

    /// Physical shortcuts run before any remapping, with fixed-size state.
    pub fn hook(&mut self, code: u32, down: bool, now: Instant) -> bool {
        self.input = true;
        let rules = self.snapshots.load();
        let swallow = self.physical.key(PhysicalKey {
            down, injected: false, shortcut: false, owner: Owner::Unknown,
            eligible: matches!(evdev_to_win(code), Some(WinKey::Scan { .. })),
            healthy: self.enabled && !self.paused,
        }, code, now, &rules, &mut self.admission, |action| self.actions.push(action).is_ok());
        if self.admission.pending() { self.wake.signal(); }
        swallow
    }

    pub fn forward(&mut self, event: &Event, sequence: Option<u64>) -> bool {
        let Some(sequence) = sequence else { return false };
        let Event::Key { code, down, device } = *event else { return false };
        let Some(key) = self.waiting.pop_front() else { self.fail(); return true };
        if key.sequence != sequence { self.fail(); return true; }
        if key.code == code && key.down == down && !self.physical.correlated(code, down, device) {
            self.fail();
            return true;
        }
        if key.code != code || key.down != down || self.work.send(Work::Key(key, device)).is_err() {
            self.fail();
        }
        true
    }

    pub fn fail(&mut self) {
        self.admission.fail();
        self.admission.acknowledge(self.admission.watermark());
        self.activation = None;
        self.pending = None;
        if !self.failed { let _ = self.work.send(Work::Fail); }
        self.failed = true;
    }

    pub fn retry_releases(&mut self) {
        self.fail();
        let _ = self.work.send(Work::Fail);
    }

    pub fn fence(&mut self, destination: Destination) {
        if let Some(held) = &self.baseline { self.physical.baseline(held.iter().copied()); }
        let Some(serial) = self.serial.checked_add(1) else { self.fail(); return };
        self.serial = serial;
        self.failed = false;
        self.recovery = self.admission.feedback().token();
        self.pending = Some((serial, Instant::now()));
        if self.work.send(Work::Fence {
            serial, watermark: self.admission.watermark(), health: self.recovery, destination, rules: self.snapshots.load(), baseline: self.baseline.take(),
        }).is_err() { self.fail(); }
    }

    pub fn completion(&mut self) -> Option<Completion> {
        while let Ok(done) = self.completions.try_recv() {
            if self.pending.is_some_and(|(serial, _)| serial == done.serial) {
                self.pending = None;
                if !done.success { self.fail(); }
                return Some(done);
            }
        }
        None
    }

    pub fn action(&self) -> Option<Action> { self.actions.pop() }

    pub fn remote_key(&mut self, code: u32, down: bool) -> bool {
        self.physical.remote_key(code, down);
        false
    }

    pub fn shortcut(&mut self, code: u32, down: bool) -> Option<bool> {
        let rules = self.snapshots.load();
        self.physical.shortcut(code, down, &rules, &self.admission, |action| self.actions.push(action).is_ok())
    }

    pub fn os_owned(&self, code: u32) -> bool { self.physical.os_owned(code) }

    pub fn os_release(&mut self, code: u32) { self.physical.os_release(code); }

    pub fn suppress_owned(&mut self, code: u32, down: bool) -> bool {
        let swallowed = self.physical.suppress_owned(code, down, &mut self.admission);
        if self.admission.pending() { self.wake.signal(); }
        swallowed
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        PHYSICAL_HOTKEYS.store(false, Ordering::Release);
        // SAFETY: our own WinEvent registration; the capture hook has already been removed.
        if !self.desktop_hook.is_null() { unsafe { UnhookWinEvent(self.desktop_hook) }; }
        self.wake.stopped.store(true, Ordering::Release);
        self.wake.signal();
        if let Some(watchdog) = self.watchdog.take() { let _ = watchdog.join(); }
        self.admission.fail();
        if let Some(thread) = self.thread.take() {
            let _ = self.work.send(Work::Stop);
            let _ = thread.join();
        }
    }
}

fn run(
    engine: &mut Engine<Native>, feedback: &Feedback, receiver: mpsc::Receiver<Work>,
    mut complete: impl FnMut(Completion),
) {
    let mut retired = false;
    loop {
        let next = match engine.deadline() {
            Some(deadline) => match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(work) => Some(work),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !feedback.healthy() { engine.fail(); }
                    else if !engine.timer(Instant::now()) { feedback.fail(); }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            },
            None => receiver.recv().ok(),
        };
        match next {
            Some(Work::Key(key, device)) => {
                engine.admitted_key(key, device, feedback);
            }
            Some(Work::Fence { serial, watermark, health, destination, rules, baseline }) => {
                if let Some(held) = baseline { engine.baseline(held.into_iter()); }
                let success = engine.recover_fence(watermark, destination, Some(&rules), feedback, health);
                complete(Completion { serial, destination, success });
            }
            Some(Work::Fail) => { if engine.fail() && retired { break; } }
            Some(Work::Retire) => { retired = true; if engine.fail() { break; } }
            Some(Work::Stop) | None => break,
        }
    }
}
