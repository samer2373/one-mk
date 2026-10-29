//! macOS: which app has the focus (the Accessibility API, then
//! `NSRunningApplication`), locking the screen (`CGSession`, else the login
//! framework), keeping it awake (`caffeinate`) and putting the display to
//! sleep (`pmset`). Compiled only; not yet run on a real Mac.
//! (Whether the focused window is fullscreen is not probed yet, so the
//! fullscreen guard and gaming mode `auto` see "not fullscreen".)

use super::{DisplaySleeper, FocusProbe, Inhibitor, ScreenLocker};
use objc2_app_kit::NSRunningApplication;
use objc2_core_foundation::CFString;
use one_mk_proto::Focus;
use std::ffi::c_void;
use std::path::Path;
use std::process::{Command, Stdio};

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
pub(super) struct Frontmost;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementCopyAttributeValue(element: *const c_void, attribute: *const c_void, value: *mut *const c_void) -> i32;
    fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
}

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
        objc2::rc::autoreleasepool(|_| {
            let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
            let apps: Vec<String> = [app.bundleIdentifier(), app.localizedName()].into_iter().flatten().map(|s| s.to_string()).collect();
            Some(Focus { fullscreen: false, apps })
        })
    }
}
