//! macOS backend built on Quartz Event Services.
//!
//! - Capture: a `CGEventTap` at the HID level on its own thread with a
//!   CFRunLoop. When the pointer pushes against an armed outer edge, the
//!   cursor is frozen and hidden and every input event is swallowed and
//!   sent to the daemon until it releases the pointer.
//! - Injection: `CGEventPost` on a second thread, with events from our own
//!   `CGEventSource` (tagged so our tap ignores them).
//! - Permissions: Accessibility and Input Monitoring. While either is
//!   missing we tell the user what to do, open the right System Settings
//!   pane and check again every second, so sharing starts as soon as the
//!   switch is flipped, with no restart.
//!
//! Coordinates are global display points (CGDisplayBounds), origin at the
//! top-left of the main display, the same space as `CGEventGetLocation`.
//! Platform-independent logic lives in `macos_logic`.

use crate::macos_logic::{self as logic, MacScroll};
use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::Result;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFBoolean, CFDictionary, CFMachPort, CFRetained, CFRunLoop, CFRunLoopSource,
    CFRunLoopSourceContext, CFRunLoopTimer, CFRunLoopTimerContext, CFString, CGPoint, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGAssociateMouseAndMouseCursorPosition, CGDirectDisplayID, CGDisplayBounds, CGDisplayChangeSummaryFlags,
    CGDisplayHideCursor, CGDisplayRegisterReconfigurationCallback, CGDisplayShowCursor, CGError, CGEvent,
    CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType, CGGetActiveDisplayList, CGMouseButton,
    CGPreflightListenEventAccess, CGRequestListenEventAccess, CGScrollEventUnit, CGWarpMouseCursorPosition,
    kCGNullDirectDisplay,
};
use one_mk_proto::{Desktop, Pos, Rect, Side};
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::ptr::{NonNull, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::sync::mpsc;

/// Stamped on every event we post (`kCGEventSourceUserData`) so our own
/// tap lets them through untouched.
const OUR_EVENTS: i64 = 0x6f6e_655f_6d6b; // "one_mk"

/// macOS's default double-click interval and a few points of slop.
const DOUBLE_CLICK_SECS: f64 = 0.5;
const DOUBLE_CLICK_SLOP: f64 = 4.0;

const KEY_CAPSLOCK: u32 = 58;

const ACCESSIBILITY_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const INPUT_MONITORING_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

pub(crate) async fn open(_state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    let trusted = Arc::new(AtomicBool::new(ax_trusted(false)));

    let (tx, t) = (ev_tx.clone(), trusted.clone());
    std::thread::Builder::new()
        .name("one-mk-capture".into())
        .spawn(move || capture_thread(tx, cap_rx, t))?;
    std::thread::Builder::new()
        .name("one-mk-inject".into())
        .spawn(move || inject_thread(ev_tx, inj_rx, trusted))?;
    Ok(Backend { events: ev_rx, capture: cap_tx, inject: inj_tx })
}

// ------------------------------------------------------------ permissions

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: Option<&CFDictionary>) -> u8;
}

/// Accessibility permission (needed to post events and to swallow them in
/// an event tap). With `prompt`, macOS shows its own "allow" dialog.
fn ax_trusted(prompt: bool) -> bool {
    // The value of kAXTrustedCheckOptionPrompt; using the literal avoids
    // linking a data symbol.
    let key = CFString::from_static_str("AXTrustedCheckOptionPrompt");
    let dict = CFDictionary::<CFString, CFBoolean>::from_slices(&[&*key], &[CFBoolean::new(prompt)]);
    // SAFETY: a valid CFDictionary, borrowed for the duration of the call.
    unsafe { AXIsProcessTrustedWithOptions(Some(dict.as_opaque())) != 0 }
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

enum Msg {
    Cmd(CaptureCmd),
    Displays,
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
struct Inbox {
    queue: Mutex<Vec<Msg>>,
    waker: OnceLock<Waker>,
}

impl Inbox {
    fn post(&self, msg: Msg) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
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
    tx: mpsc::UnboundedSender<Event>,
    /// Accessibility granted; read by the injection thread.
    trusted: Arc<AtomicBool>,
    inbox: Arc<Inbox>,
    st: RefCell<State>,
}

struct State {
    desktop: Option<Desktop>,
    sides: Vec<Side>,
    /// Input goes to a peer: events are swallowed and the cursor is frozen.
    active: bool,
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
}

impl State {
    fn new() -> Self {
        Self {
            desktop: None,
            sides: Vec::new(),
            active: false,
            armed: true,
            cursor_hidden: false,
            tap: None,
            local_keys: [false; 128],
            problem: None,
            asked_accessibility: false,
            asked_input_monitoring: false,
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
        if let Some(p) = pos {
            let _ = CGWarpMouseCursorPosition(CGPoint::new(p.x, p.y));
        }
        // Re-associating also ends the short input freeze that follows a warp.
        let _ = CGAssociateMouseAndMouseCursorPosition(true);
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

fn capture_thread(tx: mpsc::UnboundedSender<Event>, mut cmds: mpsc::UnboundedReceiver<CaptureCmd>, trusted: Arc<AtomicBool>) {
    allow_cursor_hiding_in_background();
    let Some(run_loop) = CFRunLoop::current() else {
        let _ = tx.send(Event::Error("Can't share this Mac's mouse: no run loop for input capture.".into()));
        return;
    };
    // SAFETY: an immutable CF constant.
    let modes = unsafe { kCFRunLoopCommonModes };

    let inbox = Arc::new(Inbox::default());
    let ctx: &'static Ctx = Box::leak(Box::new(Ctx { tx, trusted, inbox: inbox.clone(), st: RefCell::new(State::new()) }));
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
    // posts to the inbox; the once-a-second check below is the fallback.
    let inbox_ptr = Arc::into_raw(inbox.clone()) as *mut c_void;
    // SAFETY: the callback matches the signature and `inbox_ptr` is never freed.
    let err = unsafe { CGDisplayRegisterReconfigurationCallback(Some(on_display_change), inbox_ptr) };
    if err != CGError::Success {
        tracing::debug!(?err, "display reconfiguration callback not registered");
    }

    // Permission and display checks once a second.
    let mut timer_ctx = CFRunLoopTimerContext { version: 0, info, retain: None, release: None, copyDescription: None };
    // SAFETY: the context is copied; `info` is 'static.
    let timer = unsafe { CFRunLoopTimer::new(None, CFAbsoluteTimeGetCurrent() + 1.0, 1.0, 0, 0, Some(on_timer), &mut timer_ctx) };
    match &timer {
        Some(t) => run_loop.add_timer(Some(t), modes),
        None => tracing::warn!("permission check timer not created"),
    }

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
    tracing::debug!("input capture stopped");
}

impl Ctx {
    /// Runs once a second: keeps the screen list current, and waits for
    /// permission before installing the event tap.
    fn tick(&'static self) {
        let Ok(mut st) = self.st.try_borrow_mut() else { return };
        st.refresh_desktop(&self.tx);
        let ax = ax_trusted(false);
        self.trusted.store(ax, Ordering::Relaxed);

        let problem = if !ax {
            Some(Problem::Accessibility)
        } else if st.tap.is_some() {
            None
        } else if !CGPreflightListenEventAccess() {
            Some(Problem::InputMonitoring)
        } else if let Err(e) = self.install_tap(&mut st) {
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
                if st.active {
                    // Permission revoked mid-capture: give the pointer back.
                    st.deactivate(None);
                    let _ = self.tx.send(Event::Deactivated);
                }
                let _ = self.tx.send(Event::Error(p.message().into()));
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
            None if st.tap.is_some() => tracing::info!("input capture ready"),
            None => {}
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
        let mask = types.iter().map(|t| t.0).chain(gestures).fold(0u64, |m, t| m | (1u64 << t));
        // SAFETY: `on_tap` matches CGEventTapCallBack; `self` is 'static.
        let tap = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::HIDEventTap,
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
            Msg::Cmd(CaptureCmd::SetSides(sides)) => st.sides = sides,
            Msg::Cmd(CaptureCmd::Release { pos }) => st.deactivate(pos),
            Msg::Displays => st.refresh_desktop(&self.tx),
            Msg::Quit => {
                if st.active || st.cursor_hidden {
                    st.deactivate(None);
                }
                if let Some(rl) = CFRunLoop::current() {
                    rl.stop();
                }
            }
        }
    }

    /// Returns true to let the event through, false to swallow it.
    fn on_event(&self, ty: CGEventType, ev: &CGEvent) -> bool {
        if ty == CGEventType::TapDisabledByTimeout || ty == CGEventType::TapDisabledByUserInput {
            tracing::debug!(?ty, "event tap disabled by macOS; re-enabling");
            if let Ok(st) = self.st.try_borrow()
                && let Some(tap) = &st.tap
            {
                CGEvent::tap_enable(tap, true);
            }
            return true;
        }
        let e = Some(ev);
        if CGEvent::integer_value_field(e, CGEventField::EventSourceUserData) == OUR_EVENTS {
            return true;
        }
        let Ok(mut st) = self.st.try_borrow_mut() else { return true };
        if st.active { self.on_active(&mut st, ty, ev) } else { self.on_idle(&mut st, ty, ev) }
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
                match logic::edge_hit(desktop, &st.sides, p, dx, dy) {
                    Some(side) if st.armed => {
                        let pos = desktop.clamp(p);
                        if self.tx.send(Event::Activated { side, pos }).is_ok() {
                            st.activate();
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

    /// Pointer is on a peer: forward everything, swallow everything.
    fn on_active(&self, st: &mut State, ty: CGEventType, ev: &CGEvent) -> bool {
        let e = Some(ev);
        let send = |event: Event| {
            let _ = self.tx.send(event);
        };
        use CGEventType as T;
        match ty {
            T::MouseMoved | T::LeftMouseDragged | T::RightMouseDragged | T::OtherMouseDragged => {
                let (dx, dy) = mouse_delta(ev);
                if dx != 0.0 || dy != 0.0 {
                    send(Event::Motion { dx, dy });
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
                    send(Event::Button { button, down });
                }
            }
            T::ScrollWheel => {
                for event in logic::scroll_to_wire(&read_scroll(ev)).into_iter().flatten() {
                    send(event);
                }
            }
            T::KeyDown => {
                // The receiving side does its own key repeat.
                let repeat = CGEvent::integer_value_field(e, CGEventField::KeyboardEventAutorepeat) != 0;
                if !repeat && let Some(code) = logic::mac_to_evdev(keycode(ev)) {
                    send(Event::Key { code, down: true });
                }
            }
            T::KeyUp => {
                let kc = keycode(ev);
                if st.take_local_key(kc) {
                    return true;
                }
                if let Some(code) = logic::mac_to_evdev(kc) {
                    send(Event::Key { code, down: false });
                }
            }
            T::FlagsChanged => {
                let kc = keycode(ev);
                if kc == logic::MAC_CAPS_LOCK {
                    // macOS reports caps lock as a toggle: one event per press.
                    send(Event::Key { code: KEY_CAPSLOCK, down: true });
                    send(Event::Key { code: KEY_CAPSLOCK, down: false });
                } else if let Some(down) = logic::modifier_down(kc, CGEvent::flags(e).0) {
                    if !down && st.take_local_key(kc) {
                        return true;
                    }
                    if let Some(code) = logic::mac_to_evdev(kc) {
                        send(Event::Key { code, down });
                    }
                }
            }
            _ => {}
        }
        false
    }
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
    let keep = std::panic::catch_unwind(AssertUnwindSafe(|| ctx.on_event(ty, ev))).unwrap_or(true);
    if keep { event.as_ptr() } else { null_mut() }
}

unsafe extern "C-unwind" fn on_inbox(info: *mut c_void) {
    // SAFETY: `info` is the leaked `Ctx`.
    let ctx = unsafe { &*(info as *const Ctx) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        for msg in ctx.inbox.take() {
            ctx.handle(msg);
        }
    }));
}

unsafe extern "C-unwind" fn on_timer(_timer: *mut CFRunLoopTimer, info: *mut c_void) {
    // SAFETY: `info` is the leaked `Ctx`, which lives forever.
    let ctx: &'static Ctx = unsafe { &*(info as *const Ctx) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| ctx.tick()));
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

fn inject_thread(tx: mpsc::UnboundedSender<Event>, mut rx: mpsc::UnboundedReceiver<Inject>, trusted: Arc<AtomicBool>) {
    let Some(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
        let _ = tx.send(Event::Error("Other computers can't control this Mac: couldn't create an input source.".into()));
        return;
    };
    CGEventSource::set_user_data(Some(&source), OUR_EVENTS);
    // By default, posting an event mutes the local mouse and keyboard for
    // a quarter second. Don't.
    CGEventSource::set_local_events_suppression_interval(Some(&source), 0.0);
    let mut inj = Injector::new(source);
    tracing::info!("input injection ready");

    let mut warned = false;
    while let Some(msg) = rx.blocking_recv() {
        if !trusted.load(Ordering::Relaxed) {
            if !warned {
                tracing::warn!("ignoring input from peers until Accessibility permission is granted");
                warned = true;
            }
            continue;
        }
        warned = false;
        inj.apply(msg);
    }
}

struct Injector {
    source: CFRetained<CGEventSource>,
    pos: CGPoint,
    /// Held buttons, bit n = macOS button number n.
    buttons: u32,
    mods: logic::Modifiers,
    clicks: logic::ClickCounter,
    smooth: logic::Accum,
    wheel: logic::Accum,
    epoch: Instant,
}

impl Injector {
    fn new(source: CFRetained<CGEventSource>) -> Self {
        // Start from wherever the cursor is.
        let pos = CGEvent::new(None).map_or(CGPoint::new(0.0, 0.0), |e| CGEvent::location(Some(&e)));
        Self {
            source,
            pos,
            buttons: 0,
            mods: logic::Modifiers::default(),
            clicks: logic::ClickCounter::new(DOUBLE_CLICK_SECS, DOUBLE_CLICK_SLOP),
            smooth: logic::Accum::default(),
            wheel: logic::Accum::default(),
            epoch: Instant::now(),
        }
    }

    fn post(&self, ev: &CGEvent, extra_flags: u64) {
        CGEvent::set_flags(Some(ev), CGEventFlags(self.mods.flags() | extra_flags));
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(ev));
    }

    fn mouse_event(&self, ty: u32, button: u32) -> Option<CFRetained<CGEvent>> {
        let ev = CGEvent::new_mouse_event(Some(&self.source), CGEventType(ty), self.pos, CGMouseButton(button))?;
        if button >= 2 {
            CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventButtonNumber, i64::from(button));
        }
        Some(ev)
    }

    fn apply(&mut self, msg: Inject) {
        match msg {
            Inject::MoveTo(p) => {
                let (dx, dy) = (p.x - self.pos.x, p.y - self.pos.y);
                self.pos = CGPoint::new(p.x, p.y);
                // With a button held, apps expect drags, not moves
                // (selection, drag and drop).
                let held = (0..32).find(|n| self.buttons & (1 << n) != 0);
                let (ty, button) = match held {
                    Some(n) => (logic::mouse_event_types(n).2, n),
                    None => (CGEventType::MouseMoved.0, 0),
                };
                let Some(ev) = self.mouse_event(ty, button) else { return };
                CGEvent::set_double_value_field(Some(&ev), CGEventField::MouseEventDeltaX, dx);
                CGEvent::set_double_value_field(Some(&ev), CGEventField::MouseEventDeltaY, dy);
                self.post(&ev, 0);
            }
            Inject::Button { button, down } => {
                let Some(n) = logic::evdev_to_mac_button(button) else {
                    tracing::debug!(button, "unsupported mouse button");
                    return;
                };
                let (down_ty, up_ty, _) = logic::mouse_event_types(n);
                let count = if down {
                    let t = self.epoch.elapsed().as_secs_f64();
                    self.clicks.press(n, t, Pos::new(self.pos.x, self.pos.y))
                } else {
                    self.clicks.release(n)
                };
                let Some(ev) = self.mouse_event(if down { down_ty } else { up_ty }, n) else { return };
                CGEvent::set_integer_value_field(Some(&ev), CGEventField::MouseEventClickState, count);
                if down {
                    self.buttons |= 1 << n;
                } else {
                    self.buttons &= !(1 << n);
                }
                self.post(&ev, 0);
            }
            Inject::Scroll { dx, dy } => {
                let (v, h) = logic::wire_to_mac_scroll(dx, dy);
                let (h, v) = self.smooth.take(h, v);
                self.scroll(CGScrollEventUnit::Pixel, v, h);
            }
            Inject::ScrollDiscrete { dx, dy } => {
                let (v, h) = logic::wire_to_mac_scroll(f64::from(dx) / 120.0, f64::from(dy) / 120.0);
                let (h, v) = self.wheel.take(h, v);
                self.scroll(CGScrollEventUnit::Line, v, h);
            }
            Inject::ScrollStop => self.smooth.reset(),
            Inject::Key { code, down } => {
                self.mods.update(code, down);
                if code == KEY_CAPSLOCK && !down {
                    return; // a toggle: one event per press
                }
                let Some(kc) = logic::evdev_to_mac(code) else {
                    tracing::debug!(code, "no Mac key for this key code");
                    return;
                };
                let Some(ev) = CGEvent::new_keyboard_event(Some(&self.source), kc, down) else { return };
                if logic::is_mac_modifier(kc) {
                    CGEvent::set_type(Some(&ev), CGEventType::FlagsChanged);
                }
                self.post(&ev, logic::key_extra_flags(kc));
            }
        }
    }

    fn scroll(&self, unit: CGScrollEventUnit, vertical: i32, horizontal: i32) {
        if vertical == 0 && horizontal == 0 {
            return;
        }
        if let Some(ev) = CGEvent::new_scroll_wheel_event2(Some(&self.source), unit, 2, vertical, horizontal, 0) {
            self.post(&ev, 0);
        }
    }
}
