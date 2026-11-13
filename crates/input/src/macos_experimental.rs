//! macOS backend built on Quartz Event Services.
//!
//! - Capture: a `CGEventTap` at the session level on its own thread with a
//!   CFRunLoop. When the pointer pushes against an armed edge segment
//!   (outer or an inner exposed edge), the cursor is frozen and hidden and
//!   every input event is swallowed and sent to the daemon until it
//!   releases the pointer.
//! - Injection: `CGEventPost` on a second thread, with events from our own
//!   `CGEventSource` (tagged so our tap ignores them).
//! - Permissions: Accessibility and Input Monitoring. While either is
//!   missing we tell the user what to do, open the right System Settings
//!   pane and check again every second, so sharing starts as soon as the
//!   switch is flipped, with no restart.
//! - The once-a-second check runs only while a permission is missing or
//!   input is captured (then it also watches for Secure Input, a locked
//!   screen, a switched-away session and sleep, which all end the crossing).
//!   Otherwise display changes come from the reconfiguration callback and
//!   the thread sleeps, so idle CPU stays near zero.
//! - Injected keys are auto-repeated here with the user's own key repeat
//!   settings (senders drop their repeats).
//! - Real input while a peer is in control here is reported as
//!   `Event::LocalInput`.
//!
//! Coordinates are global display points (CGDisplayBounds), origin at the
//! top-left of the main display, the same space as `CGEventGetLocation`.
//! Platform-independent logic lives in `macos_logic`.

use crate::common_logic::{Controlled, KeyRepeat, LocalInputGate, drain_coalesced, is_repeatable, now_ms};
use crate::macos_logic::{self as logic, MacScroll};
use crate::localmap_handoff::Destination;
use crate::macos_localmap::Local;
use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::Result;
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFBoolean, CFDictionary, CFMachPort, CFNotificationCenter, CFNotificationName,
    CFNotificationSuspensionBehavior, CFNumber, CFPreferencesCopyAppValue, CFRetained, CFRunLoop, CFRunLoopSource,
    CFRunLoopSourceContext, CFRunLoopTimer, CFRunLoopTimerContext, CFString, CFType, CGPoint,
    kCFPreferencesAnyApplication, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGAssociateMouseAndMouseCursorPosition, CGDirectDisplayID, CGDisplayBounds, CGDisplayChangeSummaryFlags,
    CGDisplayHideCursor, CGDisplayMoveCursorToPoint, CGGetDisplaysWithPoint, CGDisplayRegisterReconfigurationCallback, CGDisplayShowCursor, CGError, CGEvent,
    CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType, CGGetActiveDisplayList, CGMouseButton,
    CGPreflightListenEventAccess, CGRequestListenEventAccess, CGScrollEventUnit, CGSessionCopyCurrentDictionary,
    CGWarpMouseCursorPosition, kCGNullDirectDisplay,
};
use one_mk_proto::{Desktop, Edge, Mods, Pos, Rect, Side, mediakeys};
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::ptr::{NonNull, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// `NX_SYSDEFINED`: the `CGEventType` of media keys.
const NX_SYSDEFINED: u32 = 14;

/// Stamped on every event we post (`kCGEventSourceUserData`) so our own
/// tap lets them through untouched.
pub(crate) const OUR_EVENTS: i64 = 0x6f6e_655f_6d6b; // "one_mk"

/// macOS's default double-click interval and a few points of slop.
const DOUBLE_CLICK_SECS: f64 = 0.5;
const DOUBLE_CLICK_SLOP: f64 = 4.0;

const KEY_CAPSLOCK: u32 = 58;

/// A gap this much longer than the 1 s check interval means the Mac slept.
const SLEPT_SECS: f64 = 5.0;

const ACCESSIBILITY_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const INPUT_MONITORING_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

pub(crate) async fn open(state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    let trusted = Arc::new(AtomicBool::new(ax_trusted(false)));
    let controlled = Arc::new(Controlled::default());

    let (focus, locker) = crate::system::native(&ev_tx);
    let (tx, t, c) = (ev_tx.clone(), trusted.clone(), controlled.clone());
    std::thread::Builder::new()
        .name("one-mk-capture".into())
        .spawn(move || capture_thread(tx, cap_rx, t, c, state_dir))?;
    crate::typing::watch_native(ev_tx.clone());
    std::thread::Builder::new()
        .name("one-mk-inject".into())
        .spawn(move || inject_thread(ev_tx, inj_rx, trusted, controlled))?;
    Ok(Backend { keyboard: keyboard_layout(), name: "native", events: ev_rx, capture: cap_tx, inject: inj_tx, focus, locker })
}

// ------------------------------------------------------------ permissions

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: Option<&CFDictionary>) -> u8;
}

/// Accessibility permission (needed to post events and to swallow them in
/// an event tap). With `prompt`, macOS shows its own "allow" dialog.
pub(crate) fn ax_trusted(prompt: bool) -> bool {
    // The value of kAXTrustedCheckOptionPrompt; using the literal avoids
    // linking a data symbol.
    let key = CFString::from_static_str("AXTrustedCheckOptionPrompt");
    let dict = CFDictionary::<CFString, CFBoolean>::from_slices(&[&*key], &[CFBoolean::new(prompt)]);
    // SAFETY: a valid CFDictionary, borrowed for the duration of the call.
    unsafe { AXIsProcessTrustedWithOptions(Some(dict.as_opaque())) != 0 }
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

/// A password field or an app like Terminal with "Secure Keyboard Entry"
/// has Secure Input on: our event tap then gets no key events, so typing
/// on a peer would silently go nowhere.
pub(crate) fn secure_input() -> bool {
    // SAFETY: no arguments; only reads state.
    unsafe { IsSecureEventInputEnabled() != 0 }
}

const SECURE_INPUT_MESSAGE: &str = "A password field or an app using Secure Keyboard Entry is active on this Mac, \
     so macOS hides the keyboard from one-mk and the pointer stays here. Leave the password field \
     (or turn off Secure Keyboard Entry in that app) to control your other computers.";

/// The screen is locked, or another user's session is on the console.
pub(crate) fn session_inactive() -> bool {
    let Some(dict) = CGSessionCopyCurrentDictionary() else { return false };
    let get = |key: &'static str| -> Option<bool> {
        let key = CFString::from_static_str(key);
        // SAFETY: a valid dictionary and CFString key; the value, if any,
        // is a CF object owned by the dictionary, alive while we use it.
        unsafe {
            let v = dict.value((&*key as *const CFString).cast());
            if v.is_null() {
                return None;
            }
            (*(v as *const CFType)).downcast_ref::<CFBoolean>().map(|b| b.as_bool())
        }
    };
    get("CGSSessionScreenIsLocked") == Some(true) || get("kCGSSessionOnConsoleKey") == Some(false)
}

fn open_settings(url: &'static str) {
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new("/usr/bin/open").arg(url).status() {
            tracing::warn!("could not open System Settings: {e}");
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    Accessibility,
    InputMonitoring,
    TapFailed,
}

impl Problem {
    fn message(self) -> &'static str {
        match self {
            Problem::Accessibility => {
                "one-mk needs Accessibility permission to share this Mac's mouse and keyboard. \
                 Open System Settings > Privacy & Security > Accessibility and turn on one-mk \
                 (if you started it from a terminal, turn on Terminal or iTerm instead). \
                 Sharing starts by itself as soon as it's allowed; no restart needed."
            }
            Problem::InputMonitoring => {
                "one-mk needs Input Monitoring permission to send this Mac's mouse and keyboard \
                 to your other computers. Open System Settings > Privacy & Security > Input Monitoring \
                 and turn on one-mk (or the terminal app you started it from). If macOS offers to \
                 quit and reopen, choose Later: sharing starts by itself as soon as it's allowed."
            }
            Problem::TapFailed => {
                "one-mk couldn't start watching this Mac's mouse and keyboard even though permission \
                 seems to be granted. Try turning one-mk off and on again in System Settings > \
                 Privacy & Security > Accessibility and Input Monitoring. Retrying every second."
            }
        }
    }
}

// ---------------------------------------------------------------- capture

pub(crate) enum Msg {
    Cmd(CaptureCmd),
    Displays,
    /// The screen was locked.
    Locked,
    /// The screen was unlocked.
    Unlocked,
    Local,
    Session(bool),
    RestoreSystem,
    Quit,
}

/// Wakes the capture run loop from other threads.
struct Waker {
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
}

// SAFETY: CFRunLoopSourceSignal and CFRunLoopWakeUp are documented as
// thread-safe; the waker is used for nothing else.
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

/// Messages for the capture thread, posted from the command thread and
/// from the display reconfiguration callback.
#[derive(Default)]
pub(crate) struct Inbox {
    queue: Mutex<Vec<Msg>>,
    waker: OnceLock<Waker>,
    interlock: Mutex<Option<crate::macos_localmap::Interlock>>,
}

impl Inbox {
    pub(crate) fn post(&self, msg: Msg) {
        if matches!(msg, Msg::Locked | Msg::Unlocked | Msg::Session(_))
            && let Some(interlock) = self.interlock.lock().unwrap_or_else(|error| error.into_inner()).as_ref() {
            interlock.fail();
        }
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
        self.signal();
    }

    pub(crate) fn signal(&self) {
        if let Some(w) = self.waker.get() {
            w.source.signal();
            w.run_loop.wake_up();
        }
    }

    fn take(&self) -> Vec<Msg> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Everything the capture callbacks share. Leaked, and used only on the
/// capture thread (callbacks run on its run loop, one at a time).
struct Ctx {
    system: RefCell<Option<crate::macos_hidmap_native::Native>>,
    system_plan: RefCell<Option<crate::macos_hidmap::Plan>>,
    tx: mpsc::UnboundedSender<Event>,
    /// Accessibility granted; read by the injection thread.
    trusted: Arc<AtomicBool>,
    /// Whether a peer controls this Mac (set by the daemon).
    controlled: Arc<Controlled>,
    inbox: Arc<Inbox>,
    st: RefCell<State>,
    /// Which keyboard or mouse input came from (IOHIDManager).
    hid: &'static crate::macos_devices::Hid,
}

struct State {
    desktop: Option<Desktop>,
    edges: Vec<Edge>,
    /// Input goes to a peer: events are swallowed and the cursor is frozen.
    active: bool,
    /// The current (or last) activation id.
    id: u32,
    /// The 1 s check timer, and whether it is scheduled.
    timer: Option<CFRetained<CFRunLoopTimer>>,
    timer_on: bool,
    /// When the check last ran, to notice sleep.
    last_tick: f64,
    /// The user has been told that Secure Input blocks crossing.
    secure_reported: bool,
    local_gate: LocalInputGate,
    /// False after a refused activation, until the pointer leaves the edge,
    /// so pushing against an edge with no peer doesn't spam the daemon.
    armed: bool,
    cursor_hidden: bool,
    tap: Option<CFRetained<CFMachPort>>,
    /// Keys held on this Mac when capture started; their releases are let
    /// through so local apps don't see them stuck.
    local_keys: [bool; 128],
    problem: Option<Problem>,
    asked_accessibility: bool,
    asked_input_monitoring: bool,
    local_rules: Option<Local>,
    local_recovery: Option<crate::macos_localmap::Recovery>,
    local_posts: bool,
    local_power: Option<crate::macos_localmap_power::Power>,
    session_paused: bool,
    quitting: bool,
    restore_ticket: Option<u64>,
    pending_rules: Option<CaptureCmd>,
    /// TEST PROBE: when the pointer last came home, and how many mouse events
    /// after it were logged.
    released_at: Option<std::time::Instant>,
    probe_logged: u32,
}

impl State {
    fn new() -> Self {
        Self {
            desktop: None,
            edges: Vec::new(),
            active: false,
            id: 0,
            timer: None,
            timer_on: true,
            last_tick: 0.0,
            secure_reported: false,
            local_gate: LocalInputGate::default(),
            armed: true,
            cursor_hidden: false,
            tap: None,
            local_keys: [false; 128],
            problem: None,
            asked_accessibility: false,
            asked_input_monitoring: false,
            local_rules: None,
            local_recovery: None,
            local_posts: false,
            local_power: None,
            session_paused: false,
            quitting: false,
            restore_ticket: None,
            pending_rules: None,
            released_at: None,
            probe_logged: 0,
        }
    }

    fn refresh_desktop(&mut self, tx: &mpsc::UnboundedSender<Event>) {
        let Some(d) = read_desktop() else { return };
        if self.desktop.as_ref() != Some(&d) {
            tracing::debug!(screens = ?d.screens, "displays changed");
            self.desktop = Some(d.clone());
            let _ = tx.send(Event::Desktop(d));
        }
    }

    fn activate(&mut self) {
        self.active = true;
        // Freeze the cursor; the mouse keeps producing deltas.
        let _ = CGAssociateMouseAndMouseCursorPosition(false);
        if !self.cursor_hidden {
            let _ = CGDisplayHideCursor(kCGNullDirectDisplay);
            self.cursor_hidden = true;
        }
    }

    fn deactivate(&mut self, pos: Option<Pos>) {
        // TEST PROBE: ONE_MK_RETURN_TEST_FILE = path of a file holding "none" | "assoc-first" | "post-move" | "both" | (unset: today's behaviour).
        let variant = std::env::var("ONE_MK_RETURN_TEST_FILE").ok().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default().trim().to_owned();
        let assoc_first = variant == "assoc-first" || variant == "both";
        let post_move = variant == "post-move" || variant == "both";
        if assoc_first {
            let _ = CGAssociateMouseAndMouseCursorPosition(true);
        }
        if let Some(p) = pos {
            move_cursor(p);
        }
        // Re-associating also ends the short input freeze that follows a warp.
        let _ = CGAssociateMouseAndMouseCursorPosition(true);
        if post_move && let Some(p) = pos
            && let Some(ev) = CGEvent::new_mouse_event(None, CGEventType::MouseMoved, CGPoint::new(p.x, p.y), CGMouseButton(0)) {
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::EventSourceUserData, OUR_EVENTS);
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev));
        }
        self.released_at = Some(std::time::Instant::now());
        self.probe_logged = 0;
        tracing::debug!(variant = %variant, "probe: pointer released");
        if self.cursor_hidden {
            let _ = CGDisplayShowCursor(kCGNullDirectDisplay);
            self.cursor_hidden = false;
        }
        self.active = false;
        self.armed = pos.is_some();
    }

    fn set_local_key(&mut self, kc: u16, down: bool) {
        if let Some(k) = self.local_keys.get_mut(usize::from(kc)) {
            *k = down;
        }
    }

    /// If `kc` was held locally when capture started, forget it and return true.
    fn take_local_key(&mut self, kc: u16) -> bool {
        self.local_keys.get_mut(usize::from(kc)).is_some_and(std::mem::take)
    }
}

fn capture_thread(
    tx: mpsc::UnboundedSender<Event>,
    mut cmds: mpsc::UnboundedReceiver<CaptureCmd>,
    trusted: Arc<AtomicBool>,
    controlled: Arc<Controlled>,
    state_dir: PathBuf,
) {
    allow_cursor_hiding_in_background();
    let Some(run_loop) = CFRunLoop::current() else {
        let _ = tx.send(Event::Error("Can't share this Mac's mouse: no run loop for input capture.".into()));
        return;
    };
    // SAFETY: an immutable CF constant.
    let modes = unsafe { kCFRunLoopCommonModes };

    let inbox = Arc::new(Inbox::default());
    let system = match crate::macos_hidmap_native::Native::start(state_dir, inbox.clone()) {
        Ok(system) => Some(system),
        Err(error) => {
            let _ = tx.send(Event::Status { part: "local-key-rules", problem: Some(format!("Local key rules unavailable: {error}. Sharing still works.")) });
            None
        }
    };
    let hid = crate::macos_devices::Hid::start(tx.clone(), &run_loop);
    let ctx: &'static Ctx =
        Box::leak(Box::new(Ctx { system: RefCell::new(system), system_plan: RefCell::new(None), tx, trusted, controlled, inbox: inbox.clone(), st: RefCell::new(State::new()), hid }));
    let info = ctx as *const Ctx as *mut c_void;

    // Commands arrive through a custom run loop source.
    let mut source_ctx = CFRunLoopSourceContext {
        version: 0,
        info,
        retain: None,
        release: None,
        copyDescription: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        perform: Some(on_inbox),
    };
    // SAFETY: the context is copied by CFRunLoopSourceCreate; `info` is 'static.
    let Some(source) = (unsafe { CFRunLoopSource::new(None, 0, &mut source_ctx) }) else {
        let _ = ctx.tx.send(Event::Error("Can't share this Mac's mouse: couldn't set up input capture.".into()));
        return;
    };
    run_loop.add_source(Some(&source), modes);
    let _ = inbox.waker.set(Waker { source, run_loop: run_loop.clone() });

    // Display changes. The callback may run on another thread, so it only
    // posts to the inbox.
    let inbox_ptr = Arc::into_raw(inbox.clone()) as *mut c_void;
    // SAFETY: the callback matches the signature and `inbox_ptr` is never freed.
    let err = unsafe { CGDisplayRegisterReconfigurationCallback(Some(on_display_change), inbox_ptr) };
    if err != CGError::Success {
        tracing::debug!(?err, "display reconfiguration callback not registered");
    }

    // Screen lock. Delivery depends on run loops we don't control, so the
    // check timer also polls the session while captured.
    if let Some(center) = CFNotificationCenter::distributed_center() {
        for (name, callback) in [
            ("com.apple.screenIsLocked", on_screen_locked as Callback),
            ("com.apple.screenIsUnlocked", on_screen_unlocked),
        ] {
            let name = CFString::from_static_str(name);
            // SAFETY: the callback matches the signature; `inbox_ptr` is never
            // freed, so the observer stays valid for the life of the process.
            unsafe {
                center.add_observer(
                    inbox_ptr,
                    Some(callback),
                    Some(&name),
                    std::ptr::null(),
                    CFNotificationSuspensionBehavior::DeliverImmediately,
                );
            }
        }
    }

    // Permission checks, and checks while captured, once a second; only
    // scheduled while needed (see `Ctx::update_timer`).
    let mut timer_ctx = CFRunLoopTimerContext { version: 0, info, retain: None, release: None, copyDescription: None };
    // SAFETY: the context is copied; `info` is 'static.
    let timer = unsafe { CFRunLoopTimer::new(None, CFAbsoluteTimeGetCurrent() + 1.0, 1.0, 0, 0, Some(on_timer), &mut timer_ctx) };
    match &timer {
        Some(t) => run_loop.add_timer(Some(t), modes),
        None => tracing::warn!("permission check timer not created"),
    }
    ctx.st.borrow_mut().timer = timer;

    let cmd_inbox = inbox.clone();
    let spawned = std::thread::Builder::new().name("one-mk-capture-cmd".into()).spawn(move || {
        while let Some(cmd) = cmds.blocking_recv() {
            cmd_inbox.post(Msg::Cmd(cmd));
        }
        cmd_inbox.post(Msg::Quit);
    });
    if let Err(e) = spawned {
        let _ = ctx.tx.send(Event::Error(format!("Can't share this Mac's mouse: {e}")));
        return;
    }

    ctx.tick();
    CFRunLoop::run();
    ctx.restore_system(&mut ctx.st.borrow_mut());
    ctx.system.borrow_mut().take();
    ctx.hid.local(None);
    *ctx.inbox.interlock.lock().unwrap_or_else(|error| error.into_inner()) = None;
    ctx.st.borrow_mut().local_power = None;
    drop(ctx.st.borrow_mut().local_rules.take());
    tracing::debug!("input capture stopped");
}

impl Ctx {
    fn local_status(&self, status: &str) {
        let _ = self.tx.send(Event::Status {
            part: "local-key-rules",
            problem: Some(format!("Local key rules {status}")),
        });
    }

    fn local_drain(&self, st: &mut State) {
        if let Some(local) = st.local_rules.as_mut() {
            while let Some(key) = local.admission.pop() { self.hid.local_key(key); }
        }
    }

    fn local_fence(&self, st: &mut State, destination: Destination) {
        let Some(local) = st.local_rules.as_ref() else { return };
        local.gate.close();
        if destination == Destination::Remote { local.admission.fail(); }
        self.local_drain(st);
        self.hid.finish_local();
        if let Some(local) = st.local_rules.as_mut() { local.fence(destination); }
    }

    fn local_pause(&self, st: &mut State, reason: &str) {
        self.inbox.post(Msg::RestoreSystem);
        if let Some(recovery) = &st.local_recovery { recovery.retry(); }
        if let Some(local) = st.local_rules.as_mut() { local.admission.fail(); }
        self.local_drain(st);
        self.hid.finish_local();
        if let Some(local) = st.local_rules.as_mut() {
            local.fail();
            local.inactive = reason.strip_prefix("inactive: ").unwrap_or(reason).into();
            self.local_status(reason);
        }
    }

    fn local_pump(&self, st: &mut State) {
        self.system_pump(st);
        self.restore_ack(st);
        let system_ready = self.system.borrow().as_ref().is_some_and(|system| !system.pending() && system.error.is_none());
        let idle = !(0..128u16).any(|code| CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code))
            && !(0..5u32).any(|button| CGEventSource::button_state(CGEventSourceStateID::HIDSystemState, CGMouseButton(button)))
            && st.local_rules.as_ref().is_none_or(|local| (local.paused || !local.physical_held()) && !local.held() && local.pending.is_none());
        if system_ready && idle && let Some(command) = st.pending_rules.take() { self.inbox.post(Msg::Cmd(command)); }
        if system_ready && idle && !st.quitting && !st.session_paused && !session_inactive() && !secure_input()
            && st.tap.is_some() && st.problem.is_none()
            && let Some(local) = st.local_rules.as_mut() && local.enabled && local.paused {
            st.local_keys.fill(false);
            local.paused = false;
            local.baseline(std::iter::empty());
            self.local_fence(st, if st.active { Destination::Remote } else { Destination::Local });
        }
        self.local_drain(st);
        if st.local_recovery.as_ref().is_some_and(|recovery| recovery.finished()) {
            st.local_recovery = None;
            if st.local_rules.is_none() { st.local_power = None; }
        }
        if self.tx.is_closed() && st.local_rules.as_ref().is_some_and(|local| !local.paused) {
            self.local_pause(st, "inactive: daemon disconnected");
        }
        let Some(local) = st.local_rules.as_mut() else { return };
        let emergency = local.admission.take_emergency();
        let mut action = None;
        while let Some(next) = local.action() { action = Some(next); }
        let feedback = local.admission.feedback();
        if local.pending.is_some_and(|(_, start, _)| start.elapsed() >= crate::localmap_runtime::LEASE * 4)
            || !feedback.healthy() && local.pending.is_none_or(|(_, _, health)| health != feedback.token()) && !local.paused
        {
            self.local_pause(st, "inactive: local input was interrupted; release all keys to resume automatically");
            if st.quitting {
                self.local_status("inactive: some keys could not be released while stopping; release all keys and restart one-mk");
                if self.system.borrow().as_ref().is_none_or(|system| !system.pending())
                    && let Some(run_loop) = CFRunLoop::current() { run_loop.stop(); }
            }
        } else if let Some(done) = local.completion() {
            if st.quitting {
                if !done.success { self.local_status("inactive: some keys could not be released while stopping; release all keys and restart one-mk"); }
                if self.system.borrow().as_ref().is_none_or(|system| !system.pending())
                    && let Some(run_loop) = CFRunLoop::current() { run_loop.stop(); }
                return;
            }
            if !done.success {
                self.local_pause(st, "inactive: macOS did not confirm a key release; release all keys to resume automatically");
            } else if done.destination == Destination::Remote {
                let activation = local.activation;
                self.local_status("inactive: forwarding to another computer");
                if self.restore_system(st) {
                    if let Some(local) = st.local_rules.as_mut() { local.activation = None; }
                    if let Some((side, pos)) = activation { self.start_now(st, side, pos); }
                }
            } else if local.enabled && !local.paused {
                let rules = local.snapshots.load();
                if self.system_plan.borrow().as_ref() != Some(&rules.system) {
                    local.gate.close();
                    if let Some(system) = self.system.borrow_mut().as_mut() { system.request(Some(rules.system.clone())); }
                    return;
                }
                self.local_status(&format!("active ({})", rules.count));
            } else {
                self.local_status(&format!("inactive: {}", local.inactive));
            }
        }
        if emergency {
            if let Some(local) = st.local_rules.as_mut() { local.enabled = false; }
            self.local_pause(st, "inactive: emergency off");
            let _ = self.tx.send(Event::LocalRulesOff);
            action = Some(crate::hotkeys::Action::Home);
        }
        if let Some(action) = action { let _ = self.tx.send(Event::Hotkey(action)); }
        if st.quitting && self.system.borrow().as_ref().is_none_or(|system| !system.pending())
            && st.local_rules.as_ref().is_none_or(|local| !local.held())
            && let Some(run_loop) = CFRunLoop::current() { run_loop.stop(); }
        self.update_timer(st);
    }

    fn restore_system(&self, _st: &mut State) -> bool {
        self.system.borrow_mut().as_mut().is_none_or(|system| system.request(None))
    }

    fn restore_ack(&self, st: &mut State) {
        if self.system.borrow().as_ref().is_some_and(|system| system.pending())
            || st.local_rules.as_ref().is_some_and(|local| local.held())
            || st.local_recovery.as_ref().is_some_and(|recovery| !recovery.finished()) { return; }
        if let Some(ticket) = st.restore_ticket.take() {
            let error = self.system.borrow().as_ref().map_or_else(|| Some("Keyboard mapping service unavailable; restoration is not confirmed".into()), |system| system.error.clone());
            let _ = self.tx.send(Event::LocalRulesRestored { ticket, error });
        }
    }

    fn system_pump(&self, st: &mut State) {
        let completion = self.system.borrow_mut().as_mut().and_then(|system| system.poll());
        let Some(completion) = completion else { return };
        match completion.result {
            Err(reason) => {
                tracing::warn!(%reason, "local keyboard mappings unavailable; capture continues");
                if let Some(local) = st.local_rules.as_mut() { local.fail(); }
                self.local_status(&format!("unavailable: {reason}"));
            }
            Ok(plan) => {
                *self.system_plan.borrow_mut() = plan;
                if !completion.restored.is_empty() {
                    let observed: Vec<_> = (0..128u16).filter(|&code| st.local_keys[usize::from(code)]).filter_map(logic::mac_to_evdev).collect();
                    let keys = crate::macos_hidmap::restored_keys(&completion.restored, &observed, &self.hid.physical_keys());
                    st.local_keys.fill(false);
                    for code in keys.into_iter().filter_map(logic::evdev_to_mac) { st.set_local_key(code, true); }
                }
                if let Some(local) = st.local_rules.as_mut() {
                    if let Some((side, pos)) = local.activation.take() { self.start_now(st, side, pos); }
                    else if local.enabled && !local.paused && !st.active && !st.quitting {
                        let held = (0..128u16).filter(|&code| CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code)).filter_map(logic::mac_to_evdev);
                        local.baseline(held);
                        self.local_fence(st, Destination::Local);
                    }
                }
            }
        }
        self.restore_ack(st);
        if st.quitting && self.system.borrow().as_ref().is_none_or(|system| !system.pending())
            && st.local_rules.as_ref().is_none_or(|local| !local.held())
            && let Some(run_loop) = CFRunLoop::current() { run_loop.stop(); }
    }

    /// Runs once a second: keeps the screen list current, and waits for
    /// permission before installing the event tap.
    fn tick(&'static self) {
        let Ok(mut st) = self.st.try_borrow_mut() else { return };
        let now = CFAbsoluteTimeGetCurrent();
        let slept = st.timer_on && st.last_tick > 0.0 && now - st.last_tick > SLEPT_SECS;
        st.last_tick = now;
        st.refresh_desktop(&self.tx);
        self.check_permissions(&mut st);
        if st.local_rules.is_some() && (slept || secure_input() || session_inactive() || st.problem.is_some()) {
            self.local_pause(&mut st, "inactive: this Mac slept, locked or stopped allowing input; release all keys to resume automatically");
        }
        self.local_pump(&mut st);
        if st.active {
            if slept {
                tracing::info!("the Mac slept while captured; giving the pointer back");
                self.end(&mut st);
            } else if secure_input() {
                self.end(&mut st);
                let _ = self.tx.send(Event::Error(SECURE_INPUT_MESSAGE.into()));
            } else if session_inactive() {
                tracing::info!("screen locked or session switched while captured; giving the pointer back");
                self.end(&mut st);
            }
        }
        self.update_timer(&mut st);
    }

    /// Schedules the 1 s check only while it has something to watch.
    fn update_timer(&self, st: &mut State) {
        let want = st.problem.is_some() || st.tap.is_none() || st.active || st.pending_rules.is_some() || st.quitting
            || st.local_rules.as_ref().is_some_and(|local| local.enabled && local.paused)
            || st.local_rules.as_ref().is_some_and(|local| local.pending.is_some() || local.held());
        if want == st.timer_on {
            return;
        }
        let Some(timer) = &st.timer else { return };
        let now = CFAbsoluteTimeGetCurrent();
        // A repeating timer keeps its interval from whatever date is set;
        // "never" is a date a century away.
        timer.set_next_fire_date(if want { now + 1.0 } else { now + 3.0e9 });
        st.timer_on = want;
        st.last_tick = now;
    }

    /// Ends capture on our own initiative, telling the daemon.
    fn end(&self, st: &mut State) {
        if st.active {
            st.deactivate(None);
            self.hid.capture(false);
            let _ = self.tx.send(Event::Deactivated { id: st.id });
        }
        self.update_timer(st);
    }

    /// Waits for permission before installing the event tap.
    fn check_permissions(&'static self, st: &mut State) {
        let ax = ax_trusted(false);
        self.trusted.store(ax, Ordering::Relaxed);

        let problem = if !ax {
            Some(Problem::Accessibility)
        } else if !CGPreflightListenEventAccess() {
            Some(Problem::InputMonitoring)
        } else if st.tap.is_some() {
            None
        } else if let Err(e) = self.install_tap(st) {
            tracing::warn!("event tap: {e}");
            Some(Problem::TapFailed)
        } else {
            None
        };

        if problem == st.problem {
            return;
        }
        match problem {
            Some(p) => {
                tracing::warn!(?p, "input permission missing");
                // Permission revoked mid-capture: give the pointer back.
                self.end(st);
                self.local_pause(st, "inactive: Accessibility or Input Monitoring permission missing");
                let _ = self.tx.send(Event::Error(p.message().into()));
                // The window's permission guide reads this from the status.
                let _ = self.tx.send(Event::Status { part: "permission", problem: Some(p.message().into()) });
                match p {
                    Problem::Accessibility if !st.asked_accessibility => {
                        st.asked_accessibility = true;
                        ax_trusted(true);
                        open_settings(ACCESSIBILITY_PANE);
                    }
                    Problem::InputMonitoring if !st.asked_input_monitoring => {
                        st.asked_input_monitoring = true;
                        CGRequestListenEventAccess();
                        open_settings(INPUT_MONITORING_PANE);
                    }
                    _ => {}
                }
            }
            None => {
                if st.tap.is_some() {
                    tracing::info!("input capture ready");
                }
                let _ = self.tx.send(Event::Status { part: "permission", problem: None });
            }
        }
        st.problem = problem;
    }

    fn install_tap(&'static self, st: &mut State) -> Result<(), &'static str> {
        use CGEventType as T;
        let types = [
            T::LeftMouseDown,
            T::LeftMouseUp,
            T::RightMouseDown,
            T::RightMouseUp,
            T::MouseMoved,
            T::LeftMouseDragged,
            T::RightMouseDragged,
            T::KeyDown,
            T::KeyUp,
            T::FlagsChanged,
            T::ScrollWheel,
            T::OtherMouseDown,
            T::OtherMouseUp,
            T::OtherMouseDragged,
        ];
        // Trackpad gestures (rotate, begin/end, gesture, magnify, swipe,
        // smart magnify, pressure), swallowed while the pointer is away.
        let gestures = [18u32, 19, 20, 29, 30, 31, 32, 34];
        // Media keys (volume, brightness, play, ...) are system-defined
        // events, not key events.
        let mask = types.iter().map(|t| t.0).chain(gestures).chain([NX_SYSDEFINED]).fold(0u64, |m, t| m | (1u64 << t));
        // SAFETY: `on_tap` matches CGEventTapCallBack; `self` is 'static.
        let tap = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                mask,
                Some(on_tap),
                self as *const Ctx as *mut c_void,
            )
        }
        .ok_or("CGEventTapCreate failed")?;
        let source = CFMachPort::new_run_loop_source(None, Some(&tap), 0).ok_or("no run loop source for the event tap")?;
        let run_loop = CFRunLoop::current().ok_or("no run loop")?;
        // SAFETY: an immutable CF constant.
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        CGEvent::tap_enable(&tap, true);
        st.tap = Some(tap);
        Ok(())
    }

    fn handle(&self, msg: Msg) {
        let Ok(mut st) = self.st.try_borrow_mut() else {
            tracing::warn!("capture state busy; dropping a command");
            return;
        };
        match msg {
            Msg::Cmd(CaptureCmd::LocalRules { rules, inactive }) => {
                let system = self.system.borrow();
                if system.as_ref().is_none_or(|system| system.error.is_some()) {
                    let reason = system.as_ref().and_then(|system| system.error.as_deref()).unwrap_or("Keyboard mapping worker unavailable; sharing still works").to_owned();
                    drop(system);
                    self.local_status(&format!("unavailable: {reason}"));
                    return;
                }
                let held = st.local_rules.as_ref().is_some_and(|local| (!local.paused && local.physical_held()) || local.held() || local.pending.is_some())
                    || (0..128u16).any(|code| CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code))
                    || (0..5u32).any(|button| CGEventSource::button_state(CGEventSourceStateID::HIDSystemState, CGMouseButton(button)));
                if system.as_ref().is_some_and(|system| system.pending()) || held {
                    st.pending_rules = Some(CaptureCmd::LocalRules { rules, inactive });
                    self.update_timer(&mut st);
                    return;
                }
                drop(system);
                st.local_keys.fill(false);
                if rules.as_ref().is_none_or(|rules| rules.count == 0 || !Local::supported(rules)) {
                    self.local_pause(&mut st, "inactive: local rules disabled");
                    self.hid.local(None);
                    *self.inbox.interlock.lock().unwrap_or_else(|error| error.into_inner()) = None;
                    if let Some(local) = st.local_rules.take() { st.local_recovery = Some(local.retire()); }
                    if st.local_recovery.is_none() { st.local_power = None; }
                    self.hid.capture(st.active);
                    self.update_timer(&mut st);
                    let problem = inactive.map(|reason| format!("Local key rules inactive: {reason}")).or_else(|| rules.as_ref().filter(|rules| rules.count > 0).map(|_| {
                        "Local key rules inactive: a conditional Caps Lock rule, Fn, media/function keys or actions are not supported yet".into()
                    }));
                    let _ = self.tx.send(Event::Status { part: "local-key-rules", problem });
                    return;
                }
                if st.local_recovery.as_ref().is_some_and(|recovery| !recovery.finished()) {
                    if let Some(recovery) = &st.local_recovery { recovery.retry(); }
                    st.pending_rules = Some(CaptureCmd::LocalRules { rules, inactive });
                    self.update_timer(&mut st);
                    self.local_status("inactive: macOS has not confirmed that every key is released; release all keys to resume automatically");
                    return;
                }
                st.local_recovery = None;
                if st.local_rules.is_none() && rules.is_some() {
                    st.local_power = CFRunLoop::current().as_deref()
                        .and_then(|run_loop| crate::macos_localmap_power::Power::start(self.inbox.clone(), run_loop));
                    if st.local_power.is_none() {
                        self.local_status("inactive: macOS could not notify one-mk about sleep; restart one-mk to try again");
                        return;
                    }
                    match Local::start(self.inbox.clone()) {
                        Ok((mut local, dispatch)) => {
                            st.local_posts = true;
                            *self.inbox.interlock.lock().unwrap_or_else(|error| error.into_inner()) = Some(local.interlock());
                            for code in 0..128u16 { st.set_local_key(code, CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code)); }
                            local.baseline((0..128u16).filter(|&code| st.local_keys[usize::from(code)]).filter_map(logic::mac_to_evdev));
                            self.hid.local(Some(dispatch));
                            self.hid.capture(true);
                            st.local_rules = Some(local);
                        }
                        Err(error) => { self.local_status(&format!("inactive: worker startup failed: {error}")); return; }
                    }
                }
                let rules = rules.unwrap_or_default();
                let supported = Local::supported(&rules);
                let active = st.tap.is_some() && st.problem.is_none() && !st.session_paused && !session_inactive() && !secure_input();
                let held: Vec<_> = (0..128u16).filter(|&code| CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code))
                    .filter_map(logic::mac_to_evdev).collect();
                if let Some(local) = st.local_rules.as_mut() {
                    local.gate.close();
                    local.baseline(held.into_iter());
                    local.enabled = supported && rules.count > 0;
                    local.paused = !active;
                    local.inactive = if !supported { "conditional Caps Lock rules, Fn, media/function keys and actions are not supported yet".into() }
                        else { inactive.unwrap_or_else(|| "preference off or no local rules".into()) };
                    local.snapshots.replace(if supported { rules } else { Arc::default() });
                    if active {
                        let destination = if st.active { Destination::Remote } else { Destination::Local };
                        self.local_fence(&mut st, destination);
                    } else { self.local_pause(&mut st, "inactive: permissions, secure input or session state prevents activation"); }
                } else { self.local_status("inactive: preference off"); }
            }
            Msg::Local => self.local_pump(&mut st),
            Msg::RestoreSystem => { self.restore_system(&mut st); }
            Msg::Cmd(CaptureCmd::RestoreLocalRules(ticket)) => {
                if let Some(local) = st.local_rules.as_mut() { local.enabled = false; }
                self.local_pause(&mut st, "inactive: local rules stopped");
                st.pending_rules = None;
                st.restore_ticket = Some(ticket);
                self.restore_system(&mut st);
                self.restore_ack(&mut st);
            }
            Msg::Session(active) => {
                if st.local_rules.is_none() && st.local_recovery.is_none() { return; }
                st.session_paused = !active;
                self.local_pause(&mut st, "inactive: this Mac slept or changed users; release all keys to resume automatically");
                self.end(&mut st);
            }
            Msg::Cmd(CaptureCmd::DeviceRules(enabled)) => self.hid.rules(enabled),
            Msg::Cmd(CaptureCmd::SetEdges(edges)) => st.edges = edges,
            Msg::Cmd(CaptureCmd::Release { id, pos }) => {
                // A release for an earlier activation is stale.
                if st.active && st.id == id {
                    st.deactivate(pos);
                    self.hid.capture(false);
                    self.local_fence(&mut st, Destination::Local);
                    self.update_timer(&mut st);
                }
            }
            Msg::Displays => st.refresh_desktop(&self.tx),
            Msg::Locked => {
                st.session_paused = true;
                self.local_pause(&mut st, "inactive: screen locked; unlock and release all keys to resume automatically");
                let _ = self.tx.send(Event::ScreenLocked(true));
                self.end(&mut st);
            }
            Msg::Unlocked => {
                st.session_paused = false;
                if let Some(recovery) = &st.local_recovery { recovery.retry(); }
                let _ = self.tx.send(Event::ScreenLocked(false));
            }
            Msg::Cmd(CaptureCmd::Activate { side }) => {
                let at = CGEvent::new(None).map(|e| CGEvent::location(Some(&e)));
                if !st.active
                    && st.tap.is_some()
                    && let (Some(desktop), Some(at)) = (st.desktop.clone(), at)
                {
                    self.start(&mut st, side, desktop.clamp(Pos::new(at.x, at.y)));
                }
            }
            Msg::Quit => {
                if let Some(local) = st.local_rules.as_mut() { local.enabled = false; }
                st.pending_rules = None;
                self.local_pause(&mut st, "inactive: shutting down");
                self.restore_system(&mut st);
                if st.active || st.cursor_hidden {
                    st.deactivate(None);
                    self.hid.capture(false);
                }
                st.quitting = true;
                if self.system.borrow().as_ref().is_none_or(|system| !system.pending())
                    && st.local_rules.as_ref().is_none_or(|local| !local.held())
                    && let Some(run_loop) = CFRunLoop::current() { run_loop.stop(); }
                self.update_timer(&mut st);
            }
        }
    }

    /// Returns true to let the event through, false to swallow it.
    fn on_event(&self, ty: CGEventType, ev: &CGEvent) -> bool {
        if ty == CGEventType::TapDisabledByTimeout || ty == CGEventType::TapDisabledByUserInput {
            tracing::debug!(?ty, "event tap disabled by macOS; re-enabling");
            if let Ok(mut st) = self.st.try_borrow_mut() {
                let held = st.local_rules.as_ref().is_some_and(|local| local.held());
                self.local_pause(&mut st, "inactive: macOS interrupted local input; release all keys to resume automatically");
                if held { self.end(&mut st); }
                if let Some(tap) = &st.tap { CGEvent::tap_enable(tap, true); }
            }
            return true;
        }
        let e = Some(ev);
        if let Some(sequence) = crate::macos_localmap_logic::receipt(CGEvent::integer_value_field(e, CGEventField::EventSourceUserData))
            && let Ok(st) = self.st.try_borrow() {
            if let Some(local) = &st.local_rules { local.receipt(sequence); return true; }
            if let Some(recovery) = &st.local_recovery { recovery.receipt(sequence); return true; }
            if st.local_posts { return true; }
        }
        if CGEvent::integer_value_field(e, CGEventField::EventSourceUserData) == OUR_EVENTS {
            return true;
        }
        let Ok(mut st) = self.st.try_borrow_mut() else { return true };
        // TEST PROBE: what the tap sees in the 3 s after the pointer came home.
        if !st.active && st.probe_logged < 120 && let Some(t) = st.released_at {
            if t.elapsed() < std::time::Duration::from_secs(3) {
                if matches!(ty, CGEventType::MouseMoved | CGEventType::LeftMouseDragged | CGEventType::ScrollWheel | CGEventType::LeftMouseDown | CGEventType::KeyDown) {
                    let loc = CGEvent::location(Some(ev));
                    let (dx, dy) = mouse_delta(ev);
                    let src = CGEvent::integer_value_field(Some(ev), CGEventField::EventSourceStateID);
                    st.probe_logged += 1;
                    tracing::debug!(ms = t.elapsed().as_millis() as u64, ty = ty.0, x = loc.x, y = loc.y, dx, dy, src, n = st.probe_logged, "probe: event after release");
                }
            } else {
                tracing::debug!(events = st.probe_logged, "probe: 3 s window over");
                st.released_at = None;
            }
        }
        // Real input (from the HID system, not posted by software) while a
        // peer is in control here.
        if !st.active && CGEvent::integer_value_field(e, CGEventField::EventSourceStateID)
            == i64::from(CGEventSourceStateID::HIDSystemState.0)
            && st.local_gate.check(now_ms(), self.controlled.get())
        {
            let _ = self.tx.send(Event::LocalInput);
        }
        let key = match ty {
            CGEventType::KeyDown | CGEventType::KeyUp => logic::mac_to_evdev(keycode(ev)).map(|code| (code, ty == CGEventType::KeyDown)),
            CGEventType::FlagsChanged => logic::modifier_down(keycode(ev), CGEvent::flags(e).0)
                .and_then(|down| logic::mac_to_evdev(keycode(ev)).map(|code| (code, down))),
            CGEventType::LeftMouseDown | CGEventType::LeftMouseUp => Some((logic::BTN_LEFT, ty == CGEventType::LeftMouseDown)),
            CGEventType::RightMouseDown | CGEventType::RightMouseUp => Some((logic::BTN_RIGHT, ty == CGEventType::RightMouseDown)),
            CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => Some((0x110 + CGEvent::integer_value_field(e, CGEventField::MouseEventButtonNumber) as u32, ty == CGEventType::OtherMouseDown)),
            _ => None,
        };
        if let Some((code, down)) = key {
            let injected = CGEvent::integer_value_field(e, CGEventField::EventSourceStateID) != i64::from(CGEventSourceStateID::HIDSystemState.0);
            if !injected && st.local_rules.is_some() && (secure_input() || st.session_paused) {
                self.local_pause(&mut st, "inactive: a password field or the lock screen is hiding input; leave it and release all keys to resume automatically");
            }
            let remote = st.active;
            if let Some(local) = st.local_rules.as_mut() {
                local.admission.click_state = CGEvent::integer_value_field(e, CGEventField::MouseEventClickState);
                local.admission.keyboard_type = CGEvent::integer_value_field(e, CGEventField::KeyboardEventKeyboardType) as u32;
                let swallowed = local.hook(code, down, injected, CGEvent::flags(e).0, remote);
                self.inbox.signal();
                if swallowed { return false; }
                // A posted event (System Events, automation) carries the
                // modifiers its program meant; held keys only explain a
                // keyboard's own.
                let original = CGEvent::flags(e).0;
                CGEvent::set_flags(e, CGEventFlags(if injected { local.posted_flags(original) } else { local.flags(original) }));
            }
            if injected && st.local_rules.is_some() { return true; }
        }
        if key.is_none() && ty.0 != NX_SYSDEFINED && let Some(local) = st.local_rules.as_ref() {
            let posted = CGEvent::integer_value_field(e, CGEventField::EventSourceStateID) != i64::from(CGEventSourceStateID::HIDSystemState.0);
            let original = CGEvent::flags(e).0;
            CGEvent::set_flags(e, CGEventFlags(if posted { local.posted_flags(original) } else { local.flags(original) }));
        }
        if st.active {
            return self.on_active(&mut st, ty, ev);
        }

        self.on_idle(&mut st, ty, ev)
    }

    /// Pointer is on this Mac: watch for edge hits and track held keys.
    fn on_idle(&self, st: &mut State, ty: CGEventType, ev: &CGEvent) -> bool {
        let e = Some(ev);
        match ty {
            CGEventType::KeyDown | CGEventType::KeyUp => st.set_local_key(keycode(ev), ty == CGEventType::KeyDown),
            CGEventType::FlagsChanged => {
                let kc = keycode(ev);
                if let Some(down) = logic::modifier_down(kc, CGEvent::flags(e).0) {
                    st.set_local_key(kc, down);
                }
            }
            // Only plain moves: never switch in the middle of a drag.
            CGEventType::MouseMoved => {
                let Some(desktop) = &st.desktop else { return true };
                let loc = CGEvent::location(e);
                let p = Pos::new(loc.x, loc.y);
                let (dx, dy) = mouse_delta(ev);
                // Only the armed segments: the parts of edges that face a peer.
                match logic::edge_hit(&st.edges, p, dx, dy) {
                    Some(_) if st.armed && secure_input() => {
                        // Keys wouldn't reach the peer; don't cross.
                        st.armed = false;
                        if !std::mem::replace(&mut st.secure_reported, true) {
                            let _ = self.tx.send(Event::Error(SECURE_INPUT_MESSAGE.into()));
                        }
                    }
                    Some(side) if st.armed => {
                        let pos = desktop.clamp(p);
                        if self.start(st, side, pos) {
                            return false;
                        }
                    }
                    Some(_) => {}
                    None => st.armed = true,
                }
            }
            _ => {}
        }
        true
    }

    /// Starts capturing for `side`, telling the daemon which modifiers are
    /// held (for modifier-to-cross). False if the daemon is gone.
    fn start(&self, st: &mut State, side: Side, pos: Pos) -> bool {
        if st.local_rules.as_ref().is_some_and(|local| local.paused && !local.held()) {
            return self.start_now(st, side, pos);
        }
        if let Some(local) = st.local_rules.as_ref() {
            if local.activation.is_some() { return false; }
            self.local_fence(st, Destination::Remote);
            if let Some(local) = st.local_rules.as_mut() && local.pending.is_some() {
                local.activation = Some((side, pos));
            }
            return false;
        }
        self.start_now(st, side, pos)
    }

    fn start_now(&self, st: &mut State, side: Side, pos: Pos) -> bool {
        let held = (0..128u16).filter(|&k| st.local_keys[usize::from(k)]).filter_map(logic::mac_to_evdev);
        let _ = self.tx.send(Event::Modifiers(Mods::from_keys(held)));
        let id = st.id.wrapping_add(1);
        if self.tx.send(Event::Activated { id, side, pos }).is_err() {
            return false;
        }
        st.id = id;
        st.secure_reported = false;
        st.activate();
        self.hid.capture(true);
        self.update_timer(st);
        true
    }

    /// Pointer is on a peer: forward everything, swallow everything.
    fn on_active(&self, st: &mut State, ty: CGEventType, ev: &CGEvent) -> bool {
        let e = Some(ev);
        // In order, once IOHID has said (or not) which device it was.
        let send = |event: Event| self.hid.send(event);
        use CGEventType as T;
        match ty {
            T::MouseMoved | T::LeftMouseDragged | T::RightMouseDragged | T::OtherMouseDragged => {
                let (dx, dy) = mouse_delta(ev);
                if dx != 0.0 || dy != 0.0 {
                    send(Event::Motion { dx, dy, device: None });
                }
            }
            T::LeftMouseDown | T::LeftMouseUp | T::RightMouseDown | T::RightMouseUp | T::OtherMouseDown | T::OtherMouseUp => {
                let n = match ty {
                    T::LeftMouseDown | T::LeftMouseUp => 0,
                    T::RightMouseDown | T::RightMouseUp => 1,
                    _ => CGEvent::integer_value_field(e, CGEventField::MouseEventButtonNumber) as u32,
                };
                let down = matches!(ty, T::LeftMouseDown | T::RightMouseDown | T::OtherMouseDown);
                if let Some(button) = logic::mac_to_evdev_button(n) {
                    send(Event::Button { button, down, device: None });
                }
            }
            T::ScrollWheel => {
                for event in logic::scroll_to_wire(&read_scroll(ev)).into_iter().flatten() {
                    send(event);
                }
            }
            T::KeyDown => {
                if !st.local_keys.get(usize::from(keycode(ev))).copied().unwrap_or(false)
                    && let Some(code) = logic::mac_to_evdev(keycode(ev)) {
                    send(Event::Key { code, down: true, device: None });
                }
            }
            T::KeyUp => {
                let kc = keycode(ev);
                if st.take_local_key(kc) {
                    return true;
                }
                if let Some(code) = logic::mac_to_evdev(kc) {
                    send(Event::Key { code, down: false, device: None });
                }
            }
            T::FlagsChanged => {
                let kc = keycode(ev);
                if kc == logic::MAC_CAPS_LOCK {
                    // macOS reports caps lock as a toggle: one event per press.
                    send(Event::Key { code: KEY_CAPSLOCK, down: true, device: None });
                    send(Event::Key { code: KEY_CAPSLOCK, down: false, device: None });
                } else if let Some(down) = logic::modifier_down(kc, CGEvent::flags(e).0) {
                    if !down && st.take_local_key(kc) {
                        // Held since before the capture: released here, but
                        // the daemon counted it as held (`Event::Modifiers`).
                        if let Some(code) = logic::mac_to_evdev(kc) {
                            send(Event::LocalKeyUp { code });
                        }
                        return true;
                    }
                    if let Some(code) = logic::mac_to_evdev(kc) {
                        send(Event::Key { code, down, device: None });
                    }
                }
            }
            t if t.0 == NX_SYSDEFINED => {
                if let Some(media) = read_media(ev) {
                    // Everything else system-defined (power button, ...) is swallowed
                    // with the rest; so is a repeat, as the receiver repeats by itself.
                    if let Some((code, down)) = logic::media_to_evdev(media) {
                        send(Event::Key { code, down, device: None });
                    }
                }
            }
            _ => {}
        }
        false
    }
}

/// The media key event in an `NX_SYSDEFINED` event, if it is one.
/// AppKit objects made here are autoreleased, and this runs on the tap's
/// thread, which has no pool of its own: without this one they would leak.
fn read_media(ev: &CGEvent) -> Option<logic::MediaEvent> {
    objc2::rc::autoreleasepool(|_| {
        let ns = NSEvent::eventWithCGEvent(ev)?;
        if ns.r#type() != NSEventType::SystemDefined {
            return None;
        }
        logic::parse_media(ns.subtype().0, ns.data1() as i64)
    })
}

fn keycode(ev: &CGEvent) -> u16 {
    CGEvent::integer_value_field(Some(ev), CGEventField::KeyboardEventKeycode) as u16
}

/// Post-acceleration motion in points, with sub-point precision.
fn mouse_delta(ev: &CGEvent) -> (f64, f64) {
    let e = Some(ev);
    (
        CGEvent::double_value_field(e, CGEventField::MouseEventDeltaX),
        CGEvent::double_value_field(e, CGEventField::MouseEventDeltaY),
    )
}

fn read_scroll(ev: &CGEvent) -> MacScroll {
    let e = Some(ev);
    let int = |f| CGEvent::integer_value_field(e, f);
    let dbl = |f| CGEvent::double_value_field(e, f);
    MacScroll {
        continuous: int(CGEventField::ScrollWheelEventIsContinuous) != 0,
        line_y: int(CGEventField::ScrollWheelEventDeltaAxis1),
        line_x: int(CGEventField::ScrollWheelEventDeltaAxis2),
        point_y: dbl(CGEventField::ScrollWheelEventPointDeltaAxis1),
        point_x: dbl(CGEventField::ScrollWheelEventPointDeltaAxis2),
        phase: int(CGEventField::ScrollWheelEventScrollPhase) as u32,
        momentum: int(CGEventField::ScrollWheelEventMomentumPhase) as u32,
    }
}

unsafe extern "C-unwind" fn on_tap(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: NonNull<CGEvent>,
    info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: `info` is the leaked `Ctx`; the event is valid for this call.
    let (ctx, ev) = unsafe { (&*(info as *const Ctx), event.as_ref()) };
    let keep = std::panic::catch_unwind(AssertUnwindSafe(|| ctx.on_event(ty, ev))).unwrap_or_else(|_| {
        if let Ok(mut st) = ctx.st.try_borrow_mut() { ctx.local_pause(&mut st, "inactive: local input stopped unexpectedly; release all keys to resume automatically"); }
        true
    });
    if keep { event.as_ptr() } else { null_mut() }
}

unsafe extern "C-unwind" fn on_inbox(info: *mut c_void) {
    // SAFETY: `info` is the leaked `Ctx`.
    let ctx = unsafe { &*(info as *const Ctx) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        for msg in ctx.inbox.take() {
            ctx.handle(msg);
        }
        if let Ok(mut st) = ctx.st.try_borrow_mut() { ctx.local_pump(&mut st); }
    }));
}

unsafe extern "C-unwind" fn on_timer(_timer: *mut CFRunLoopTimer, info: *mut c_void) {
    // SAFETY: `info` is the leaked `Ctx`, which lives forever.
    let ctx: &'static Ctx = unsafe { &*(info as *const Ctx) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| ctx.tick()));
}

type Callback = unsafe extern "C-unwind" fn(
    *mut CFNotificationCenter,
    *mut c_void,
    *const CFNotificationName,
    *const c_void,
    *const CFDictionary,
);

unsafe extern "C-unwind" fn on_screen_unlocked(
    _center: *mut CFNotificationCenter,
    observer: *mut c_void,
    _name: *const CFNotificationName,
    _object: *const c_void,
    _info: *const CFDictionary,
) {
    // SAFETY: `observer` is a leaked `Arc<Inbox>`; `Inbox` is Sync.
    let inbox = unsafe { &*(observer as *const Inbox) };
    inbox.post(Msg::Unlocked);
}

unsafe extern "C-unwind" fn on_screen_locked(
    _center: *mut CFNotificationCenter,
    observer: *mut c_void,
    _name: *const CFNotificationName,
    _object: *const c_void,
    _info: *const CFDictionary,
) {
    // SAFETY: `observer` is a leaked `Arc<Inbox>`; `Inbox` is Sync.
    let inbox = unsafe { &*(observer as *const Inbox) };
    inbox.post(Msg::Locked);
}

unsafe extern "C-unwind" fn on_display_change(_display: CGDirectDisplayID, flags: CGDisplayChangeSummaryFlags, info: *mut c_void) {
    if flags.contains(CGDisplayChangeSummaryFlags::BeginConfigurationFlag) {
        return;
    }
    // SAFETY: `info` is a leaked `Arc<Inbox>`; `Inbox` is Sync.
    let inbox = unsafe { &*(info as *const Inbox) };
    inbox.post(Msg::Displays);
}

/// All active displays in global points. Mirrored displays share bounds
/// and are listed once.
fn read_desktop() -> Option<Desktop> {
    let mut ids: [CGDirectDisplayID; 32] = [0; 32];
    let mut n = 0u32;
    // SAFETY: both pointers are valid; the list holds `ids.len()` entries.
    let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut n) };
    if err != CGError::Success {
        return None;
    }
    let mut screens: Vec<Rect> = Vec::new();
    for &id in &ids[..(n as usize).min(ids.len())] {
        let b = CGDisplayBounds(id);
        let r = Rect::new(
            b.origin.x.round() as i32,
            b.origin.y.round() as i32,
            b.size.width.round() as i32,
            b.size.height.round() as i32,
        );
        if r.w > 0 && r.h > 0 && !screens.contains(&r) {
            screens.push(r);
        }
    }
    (!screens.is_empty()).then(|| Desktop::new(screens))
}

/// A background process can't hide the cursor unless it sets this
/// connection property (what Synergy and Deskflow do). The functions are
/// private, so they're looked up at run time and skipped if missing.
fn allow_cursor_hiding_in_background() {
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    type DefaultConnection = unsafe extern "C" fn() -> i32;
    type SetProperty = unsafe extern "C" fn(i32, i32, *const c_void, *const c_void) -> i32;

    // SAFETY: dlsym with NUL-terminated names; results checked for null.
    let (conn, set) = unsafe {
        (dlsym(RTLD_DEFAULT, c"_CGSDefaultConnection".as_ptr()), dlsym(RTLD_DEFAULT, c"CGSSetConnectionProperty".as_ptr()))
    };
    if conn.is_null() || set.is_null() {
        tracing::debug!("CGSSetConnectionProperty unavailable; the cursor may stay visible while on a peer");
        return;
    }
    let key = CFString::from_static_str("SetsCursorInBackground");
    let value = CFBoolean::new(true);
    // SAFETY: the long-standing signatures of these CoreGraphics functions;
    // key and value are valid CF objects for the duration of the call.
    let err = unsafe {
        let conn: DefaultConnection = std::mem::transmute::<*mut c_void, DefaultConnection>(conn);
        let set: SetProperty = std::mem::transmute::<*mut c_void, SetProperty>(set);
        let cid = conn();
        set(cid, cid, (&*key as *const CFString).cast(), (value as *const CFBoolean).cast())
    };
    if err != 0 {
        tracing::debug!(err, "SetsCursorInBackground not accepted");
    }
}

// --------------------------------------------------------------- injection

include!("macos_inject.rs");
