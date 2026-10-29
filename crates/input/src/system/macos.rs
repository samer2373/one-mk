//! macOS: locking the screen, keeping it awake (`caffeinate`) and putting the
//! display to sleep (`pmset`). Compiled only; not yet run on a real Mac.
//! (What has the focus is not probed yet, so the fullscreen guard and
//! gaming mode `auto` treat it as unknown.)

use super::{DisplaySleeper, Inhibitor, ScreenLocker};
use std::path::Path;
use std::process::{Command, Stdio};

/// Shows the login window, which locks the session, where the old
/// `CGSession` tool still exists; elsewhere puts the displays to sleep,
/// which locks if "Require password after the display is turned off" is
/// set (the default).
pub(super) struct Suspend;

const CGSESSION: &str = "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession";

impl ScreenLocker for Suspend {
    fn lock(&mut self) -> anyhow::Result<()> {
        let mut cmd = if Path::new(CGSESSION).exists() {
            let mut c = Command::new(CGSESSION);
            c.arg("-suspend");
            c
        } else {
            let mut c = Command::new("pmset");
            c.arg("displaysleepnow");
            c
        };
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        // Reaped when it exits (at once), or it would stay a zombie.
        std::thread::Builder::new().name("one-mk-lock".into()).spawn(move || {
            let _ = child.wait();
        })?;
        Ok(())
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
