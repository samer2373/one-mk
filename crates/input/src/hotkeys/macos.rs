//! macOS: a keyboard-only event tap on a thread of its own, at the session
//! level, after the capture tap. It needs the Accessibility permission,
//! which one-mk needs anyway; Carbon's `RegisterEventHotKey` would need none
//! but only delivers to the main thread's event loop, which a daemon
//! doesn't run.
//!
//! While the pointer is on another computer the capture tap swallows every
//! key first, so these fire only at home. Keys we inject (a peer typing
//! here) never fire them, and while a peer controls this computer
//! (`pause`) no key does: a chord typed for a program here reaches it. The tap is idle between key
//! presses; it lives as long as the process (a tap that couldn't be created
//! is tried again when the bindings change).

use super::{Availability, Bindings, Chord, ChordMods, Platform};
use crate::Event;
use crate::macos_logic as logic;
use objc2_core_foundation::{CFMachPort, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType,
};
use one_mk_proto::HeldKeys;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::ptr::{NonNull, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct Shared {
    bindings: Mutex<Bindings>,
    /// Keys whose press fired a shortcut: their repeats and release are
    /// swallowed too.
    swallowed: Mutex<HeldKeys>,
    events: mpsc::UnboundedSender<Event>,
    running: AtomicBool,
    /// A peer controls this computer: let every key through.
    paused: AtomicBool,
    /// The tap, while its thread runs (to turn it back on when macOS turns
    /// it off for being slow).
    tap: AtomicPtr<CFMachPort>,
}

pub(super) struct Tap(Arc<Shared>);

impl Platform for Tap {
    fn set(&mut self, bindings: Bindings) {
        *self.0.bindings.lock().unwrap_or_else(|e| e.into_inner()) = bindings;
        if !self.0.running.load(Ordering::Acquire) {
            spawn(self.0.clone());
        }
    }

    fn pause(&mut self, paused: bool) {
        self.0.paused.store(paused, Ordering::Release);
    }
}

pub(super) fn start(bindings: Bindings, events: mpsc::UnboundedSender<Event>) -> Tap {
    let shared = Arc::new(Shared {
        bindings: Mutex::new(bindings),
        swallowed: Mutex::new(HeldKeys::new()),
        events,
        running: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        tap: AtomicPtr::new(null_mut()),
    });
    spawn(shared.clone());
    Tap(shared)
}

fn spawn(shared: Arc<Shared>) {
    if shared.bindings.lock().unwrap_or_else(|e| e.into_inner()).is_empty() || shared.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let s = shared.clone();
    if let Err(e) = std::thread::Builder::new().name("one-mk-hotkeys".into()).spawn(move || tap_thread(s)) {
        tracing::warn!("keyboard shortcuts: no thread: {e}");
        shared.running.store(false, Ordering::Release);
    }
}

fn tap_thread(shared: Arc<Shared>) {
    let unavailable = |why: &str| {
        let _ = shared.events.send(Event::Hotkeys(Availability::Unavailable(why.into())));
        shared.running.store(false, Ordering::Release);
    };
    let mask = [CGEventType::KeyDown, CGEventType::KeyUp].iter().fold(0u64, |m, t| m | (1u64 << t.0));
    // The tap holds a pointer to `shared` for as long as it lives (the life
    // of the process): leak one reference for it.
    let info = Arc::into_raw(shared.clone()) as *mut c_void;
    // SAFETY: `on_tap` matches CGEventTapCallBack; `info` stays valid.
    let tap = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            mask,
            Some(on_tap),
            info,
        )
    };
    let Some(tap) = tap else {
        // SAFETY: the reference leaked above, never handed to a tap.
        drop(unsafe { Arc::from_raw(info as *const Shared) });
        return unavailable("allow one-mk under System Settings > Privacy & Security > Accessibility");
    };
    let (Some(source), Some(run_loop)) = (CFMachPort::new_run_loop_source(None, Some(&tap), 0), CFRunLoop::current()) else {
        return unavailable("could not listen to the keyboard");
    };
    // SAFETY: an immutable CF constant.
    run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
    CGEvent::tap_enable(&tap, true);
    shared.tap.store(&*tap as *const CFMachPort as *mut CFMachPort, Ordering::Release);
    let _ = shared.events.send(Event::Hotkeys(Availability::Active("through macOS".into())));
    CFRunLoop::run();
    shared.tap.store(null_mut(), Ordering::Release);
    shared.running.store(false, Ordering::Release);
}

/// True to let the event through.
fn on_event(shared: &Shared, ty: CGEventType, ev: &CGEvent) -> bool {
    let e = Some(ev);
    let Some(code) = logic::mac_to_evdev(CGEvent::integer_value_field(e, CGEventField::KeyboardEventKeycode) as u16) else {
        return true;
    };
    let mut swallowed = shared.swallowed.lock().unwrap_or_else(|e| e.into_inner());
    if !logic::hotkey_applies(
        CGEvent::integer_value_field(e, CGEventField::EventSourceUserData) == crate::macos::OUR_EVENTS
            || crate::macos_localmap_logic::receipt(CGEvent::integer_value_field(e, CGEventField::EventSourceUserData)).is_some(),
        shared.paused.load(Ordering::Acquire),
    ) {
        // Still let the release of a key that fired go: it was swallowed
        // when it went down, and the program here must not see it alone.
        return !(ty == CGEventType::KeyUp && swallowed.update(code, false));
    }
    if ty == CGEventType::KeyUp {
        // `update` says whether it was held, i.e. whether it fired.
        return !swallowed.update(code, false);
    }
    if CGEvent::integer_value_field(e, CGEventField::KeyboardEventAutorepeat) != 0 {
        return !swallowed.is_held(code);
    }
    let chord = Chord { mods: ChordMods::from_mac_flags(CGEvent::flags(e).0), key: code };
    let Some(action) = shared.bindings.lock().unwrap_or_else(|e| e.into_inner()).action(chord) else { return true };
    swallowed.update(code, true);
    let _ = shared.events.send(Event::Hotkey(action));
    false
}

unsafe extern "C-unwind" fn on_tap(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: NonNull<CGEvent>,
    info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: `info` is the leaked `Shared`; the event is valid for this call.
    let (shared, ev) = unsafe { (&*(info as *const Shared), event.as_ref()) };
    if ty == CGEventType::TapDisabledByTimeout || ty == CGEventType::TapDisabledByUserInput {
        let tap = shared.tap.load(Ordering::Acquire);
        if !tap.is_null() {
            // SAFETY: set while the tap's thread runs, and this runs on it.
            CGEvent::tap_enable(unsafe { &*tap }, true);
        }
        return event.as_ptr();
    }
    let keep = std::panic::catch_unwind(AssertUnwindSafe(|| on_event(shared, ty, ev))).unwrap_or(true);
    if keep { event.as_ptr() } else { null_mut() }
}
