//! macOS: which app has the focus (the Accessibility API, then
//! `NSRunningApplication`), locking the screen (`CGSession`, else the login
//! framework), keeping it awake (`caffeinate`) and putting the display to
//! sleep (`pmset`), and hearing when another app comes to the front (an
//! Accessibility observer). Compiled only; not yet run on a real Mac.
//! (Whether the focused window is fullscreen is not probed yet, so the
//! fullscreen guard and gaming mode `auto` see "not fullscreen".)

use super::{DisplaySleeper, FocusProbe, Inhibitor, ScreenLocker};
use objc2_app_kit::NSRunningApplication;
use objc2_core_foundation::{CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext, CFString, kCFRunLoopDefaultMode};
use one_mk_proto::Focus;
use std::ffi::c_void;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Locks the session: the old `CGSession -suspend` tool where it still
/// exists (it shows the login window), else `SACLockScreenImmediate` from
/// the private `login` framework (what the menu bar's Lock Screen calls;
/// found with `dlopen`, so a macOS without it is an error, not a crash).
/// Nothing else: putting the display to sleep only locks with "require
/// password" set, and would look like a lock that didn't happen (and set
/// off sleep sync), so without either this says locking is unsupported.
pub(super) struct Suspend;

const CGSESSION: &str = "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession";
const LOGIN_FRAMEWORK: &std::ffi::CStr = c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login";

impl ScreenLocker for Suspend {
    fn lock(&mut self) -> anyhow::Result<()> {
        if Path::new(CGSESSION).exists() {
            let mut child = Command::new(CGSESSION)
                .arg("-suspend")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            // Reaped when it exits (at once), or it would stay a zombie.
            std::thread::Builder::new().name("one-mk-lock".into()).spawn(move || {
                let _ = child.wait();
            })?;
            return Ok(());
        }
        match sac_lock_screen() {
            // What it returns is undocumented; the lock itself is reported
            // by the screen lock notifications.
            Some(code) => {
                tracing::debug!(code, "SACLockScreenImmediate");
                Ok(())
            }
            None => anyhow::bail!(
                "locking the screen is not supported on this version of macOS (neither CGSession nor \
                 SACLockScreenImmediate is there); lock it with Control+Command+Q"
            ),
        }
    }
}

unsafe extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char) -> *mut std::ffi::c_void;
}

/// `RTLD_LAZY | RTLD_LOCAL` on macOS.
const RTLD_LAZY_LOCAL: std::ffi::c_int = 0x1 | 0x4;

/// Calls `SACLockScreenImmediate()`, if the login framework has it.
fn sac_lock_screen() -> Option<std::ffi::c_int> {
    // SAFETY: dlopen/dlsym with valid C strings; the framework stays loaded
    // (never dlclose'd), and the symbol, if there, is `int (void)`.
    unsafe {
        let handle = dlopen(LOGIN_FRAMEWORK.as_ptr(), RTLD_LAZY_LOCAL);
        if handle.is_null() {
            return None;
        }
        let sym = dlsym(handle, c"SACLockScreenImmediate".as_ptr());
        if sym.is_null() {
            return None;
        }
        let lock: unsafe extern "C" fn() -> std::ffi::c_int = std::mem::transmute(sym);
        Some(lock())
    }
}

/// Keeps the display and the system awake with a `caffeinate -d -i` child
/// (the supported way to hold power assertions without linking IOKit),
/// killed and reaped when the guard is dropped. `-w` ties it to us, so it
/// also ends if this process is killed.
pub(super) struct Caffeinate;

struct Hold(Option<std::process::Child>);

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Inhibitor for Caffeinate {
    fn inhibit(&mut self, _reason: &str) -> anyhow::Result<Box<dyn Send>> {
        let child = Command::new("caffeinate")
            .args(["-d", "-i", "-w", &std::process::id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Box::new(Hold(Some(child))))
    }
}

/// `pmset displaysleepnow`.
pub(super) struct DisplaySleepNow;

impl DisplaySleeper for DisplaySleepNow {
    fn sleep_display(&mut self) -> anyhow::Result<()> {
        let mut child = Command::new("pmset")
            .arg("displaysleepnow")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        std::thread::Builder::new().name("one-mk-sleep".into()).spawn(move || {
            let _ = child.wait();
        })?;
        Ok(())
    }
}

/// The app with the keyboard focus, by its bundle id and name. Asked of
/// the Accessibility API (which one-mk needs anyway to type here): unlike
/// `NSWorkspace.frontmostApplication`, which a process without an AppKit
/// run loop never sees change, it is right whenever asked. Unknown (`None`)
/// without the permission.
///
/// While watched ([`FocusProbe::watch`]), a thread with a run loop of its
/// own holds an Accessibility observer on the app with the focus, for
/// `AXApplicationDeactivated`: another app came to the front. (The
/// system-wide element can't be observed, and `NSWorkspace` activation
/// notifications need an AppKit main run loop a daemon doesn't have.)
/// Each probe moves the observer to the app it found, so it follows the
/// focus from app to app. The thread sleeps in its run loop: no polling.
#[derive(Default)]
pub(super) struct Frontmost {
    watcher: Option<Arc<Watch>>,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementCopyAttributeValue(element: *const c_void, attribute: *const c_void, value: *mut *const c_void) -> i32;
    fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXObserverCreate(pid: i32, callback: AXObserverCallback, observer: *mut *const c_void) -> i32;
    fn AXObserverAddNotification(observer: *const c_void, element: *const c_void, notification: *const c_void, refcon: *mut c_void) -> i32;
    fn AXObserverGetRunLoopSource(observer: *const c_void) -> *const c_void;
}

type AXObserverCallback = unsafe extern "C" fn(observer: *const c_void, element: *const c_void, notification: *const c_void, refcon: *mut c_void);

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// The process with the keyboard focus.
fn focused_pid() -> Option<i32> {
    // The value of kAXFocusedApplicationAttribute (a literal: no data symbol).
    let attr = CFString::from_static_str("AXFocusedApplication");
    // SAFETY: AX calls with valid pointers; each object created or copied
    // here is released once, and only if non-null.
    unsafe {
        let wide = AXUIElementCreateSystemWide();
        if wide.is_null() {
            return None;
        }
        // An app that doesn't answer must not stall the daemon.
        AXUIElementSetMessagingTimeout(wide, 0.1);
        let mut app: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(wide, (&*attr as *const CFString).cast(), &mut app);
        CFRelease(wide);
        if err != 0 || app.is_null() {
            return None;
        }
        let mut pid = 0;
        let err = AXUIElementGetPid(app, &mut pid);
        CFRelease(app);
        (err == 0 && pid > 0).then_some(pid)
    }
}

impl FocusProbe for Frontmost {
    fn focus(&mut self) -> Option<Focus> {
        let pid = focused_pid()?;
        if let Some(w) = &self.watcher {
            w.follow(pid);
        }
        objc2::rc::autoreleasepool(|_| {
            let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
            let apps: Vec<String> = [app.bundleIdentifier(), app.localizedName()].into_iter().flatten().map(|s| s.to_string()).collect();
            Some(Focus { fullscreen: false, apps })
        })
    }

    fn watch(&mut self, notify: Option<Box<dyn Fn() + Send>>) {
        if let Some(old) = self.watcher.take() {
            old.stop();
        }
        if let Some(notify) = notify {
            self.watcher = Watch::start(notify);
        }
    }
}

impl Drop for Frontmost {
    fn drop(&mut self) {
        if let Some(w) = self.watcher.take() {
            w.stop();
        }
    }
}

/// The watching thread's inbox, shared with the probe.
#[derive(Default)]
struct Watch {
    /// The app to observe (the one the newest probe found).
    pid: Mutex<Option<i32>>,
    quit: AtomicBool,
    waker: OnceLock<Waker>,
}

/// Wakes the watching thread's run loop.
struct Waker {
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
}

// SAFETY: CFRunLoopSourceSignal and CFRunLoopWakeUp are documented as
// thread-safe; the waker is used for nothing else.
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

impl Watch {
    fn start(notify: Box<dyn Fn() + Send>) -> Option<Arc<Watch>> {
        let watch = Arc::new(Watch::default());
        let w = watch.clone();
        let spawned = std::thread::Builder::new().name("one-mk-focus-watch".into()).spawn(move || watch_thread(&w, notify));
        if let Err(e) = spawned {
            tracing::debug!("not watching the focus: {e}");
            return None;
        }
        Some(watch)
    }

    /// Observes `pid` from now on, if it isn't already.
    fn follow(&self, pid: i32) {
        if self.pid.lock().unwrap_or_else(|e| e.into_inner()).replace(pid) != Some(pid) {
            self.wake();
        }
    }

    fn stop(&self) {
        self.quit.store(true, Ordering::Release);
        self.wake();
    }

    fn wake(&self) {
        // Before the run loop is up, the thread reads the inbox itself.
        if let Some(w) = self.waker.get() {
            w.source.signal();
            w.run_loop.wake_up();
        }
    }
}

/// The watching thread's own state, borrowed by its run loop's callbacks
/// (which run only inside `CFRunLoop::run` on this thread).
struct Watcher<'a> {
    watch: &'a Watch,
    notify: Box<dyn Fn() + Send>,
    run_loop: CFRetained<CFRunLoop>,
    observed: std::cell::RefCell<Option<Observed>>,
}

/// An observer on one app, with its run loop source added.
struct Observed {
    pid: i32,
    observer: *const c_void,
    app: *const c_void,
}

const DEACTIVATED: &str = "AXApplicationDeactivated";

impl Watcher<'_> {
    /// Moves the observer to the app the inbox names, or stops.
    fn sync(&self) {
        if self.watch.quit.load(Ordering::Acquire) {
            self.run_loop.stop();
            return;
        }
        let Some(pid) = *self.watch.pid.lock().unwrap_or_else(|e| e.into_inner()) else { return };
        if self.observed.borrow().as_ref().is_some_and(|o| o.pid == pid) {
            return;
        }
        self.unobserve();
        // SAFETY: AX calls with valid pointers; the observer and the app
        // element are released in `unobserve`, the refcon (`self`) outlives
        // the run loop the observer's source is in.
        unsafe {
            let mut observer: *const c_void = std::ptr::null();
            if AXObserverCreate(pid, on_deactivated, &mut observer) != 0 || observer.is_null() {
                tracing::debug!(pid, "no focus observer (Accessibility not granted?)");
                return;
            }
            let app = AXUIElementCreateApplication(pid);
            let name = CFString::from_static_str(DEACTIVATED);
            let refcon = self as *const Watcher as *mut c_void;
            if app.is_null() || AXObserverAddNotification(observer, app, (&*name as *const CFString).cast(), refcon) != 0 {
                tracing::debug!(pid, "the focused app can't be observed");
                if !app.is_null() {
                    CFRelease(app);
                }
                CFRelease(observer);
                return;
            }
            let source = AXObserverGetRunLoopSource(observer).cast::<CFRunLoopSource>();
            if let Some(source) = source.as_ref() {
                self.run_loop.add_source(Some(source), kCFRunLoopDefaultMode);
            }
            *self.observed.borrow_mut() = Some(Observed { pid, observer, app });
        }
    }

    fn unobserve(&self) {
        let Some(o) = self.observed.borrow_mut().take() else { return };
        // SAFETY: the source belongs to the observer, which is still
        // alive; each object is released once.
        unsafe {
            let source = AXObserverGetRunLoopSource(o.observer).cast::<CFRunLoopSource>();
            if let Some(source) = source.as_ref() {
                self.run_loop.remove_source(Some(source), kCFRunLoopDefaultMode);
            }
            CFRelease(o.app);
            CFRelease(o.observer);
        }
    }
}

unsafe extern "C" fn on_deactivated(_: *const c_void, _: *const c_void, _: *const c_void, refcon: *mut c_void) {
    // SAFETY: `refcon` is the `Watcher` running the run loop this is called from.
    let w = unsafe { &*(refcon as *const Watcher) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (w.notify)()));
}

unsafe extern "C-unwind" fn on_inbox(info: *mut c_void) {
    // SAFETY: `info` is the `Watcher` running the run loop this is called from.
    let w = unsafe { &*(info as *const Watcher) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.sync()));
}

fn watch_thread(watch: &Watch, notify: Box<dyn Fn() + Send>) {
    let Some(run_loop) = CFRunLoop::current() else { return };
    let watcher = Watcher { watch, notify, run_loop: run_loop.clone(), observed: Default::default() };
    let info = &watcher as *const Watcher as *mut c_void;
    let mut ctx = CFRunLoopSourceContext {
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
    // SAFETY: the context is copied; `info` outlives the source, which is
    // invalidated before `watcher` is dropped.
    let Some(source) = (unsafe { CFRunLoopSource::new(None, 0, &mut ctx) }) else { return };
    // SAFETY: an immutable CF constant.
    run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
    let _ = watch.waker.set(Waker { source: source.clone(), run_loop: run_loop.clone() });
    if let Some(pid) = focused_pid() {
        watch.pid.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(pid);
    }
    watcher.sync();
    if !watch.quit.load(Ordering::Acquire) {
        CFRunLoop::run();
    }
    watcher.unobserve();
    source.invalidate();
}
