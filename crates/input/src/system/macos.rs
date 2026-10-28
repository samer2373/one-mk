//! macOS: locking the screen. Compiled only; not yet run on a real Mac.
//! (What has the focus is not probed yet, so the fullscreen guard and
//! gaming mode `auto` treat it as unknown.)

use super::ScreenLocker;
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
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        Ok(())
    }
}
