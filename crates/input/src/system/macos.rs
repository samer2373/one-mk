//! macOS: locking the screen (`CGSession`, else the login framework),
//! keeping it awake (`caffeinate`) and putting the display to sleep
//! (`pmset`). Compiled only; not yet run on a real Mac.
//! (What has the focus is not probed yet, so the fullscreen guard and
//! gaming mode `auto` treat it as unknown.)

use super::{DisplaySleeper, Inhibitor, ScreenLocker};
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
