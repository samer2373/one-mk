//! Starting other programs without a console window or a tie to the caller.

use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(windows)]
mod flags {
    pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    pub const DETACHED_PROCESS: u32 = 0x0000_0008;
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
}

/// No console window flashing up (Windows); nothing to do elsewhere.
pub fn hide_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, flags::CREATE_NO_WINDOW);
    cmd
}

/// Runs a short command to completion, quietly; true if it succeeded.
pub fn run_quiet(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> bool {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hide_window(&mut cmd).status().is_ok_and(|s| s.success())
}

/// Starts `exe args` as its own process that outlives the caller, with no
/// window, and with its output appended to `log` (if it can be opened).
pub fn spawn_detached(exe: &Path, args: &[String], log: Option<&Path>) -> std::io::Result<()> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null());
    let sink = || {
        log.and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok())
            .map_or_else(Stdio::null, Stdio::from)
    };
    cmd.stdout(sink()).stderr(sink());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut cmd,
        flags::DETACHED_PROCESS | flags::CREATE_NEW_PROCESS_GROUP | flags::CREATE_NO_WINDOW,
    );
    // Reap it when it ends (no zombie), without making the caller wait.
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || drop(child.wait()));
    Ok(())
}
