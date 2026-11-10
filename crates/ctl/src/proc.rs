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
    spawn_watched(exe, args, log).map(drop)
}

/// A program started by [`spawn_watched`]: whether it has ended yet.
#[derive(Clone, Debug)]
pub struct Watched(std::sync::Arc<std::sync::Mutex<Option<std::process::ExitStatus>>>);

impl Watched {
    /// How it ended, once it has (`None` while it runs).
    pub fn ended(&self) -> Option<std::process::ExitStatus> {
        *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// [`spawn_detached`], telling the caller when the program ends, so one
/// that fails at once is known at once rather than waited for.
pub fn spawn_watched(exe: &Path, args: &[String], log: Option<&Path>) -> std::io::Result<Watched> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null());
    // Only this account reads the log (it names computers and networks).
    let sink = || {
        log.and_then(|p| crate::private::append(p).ok())
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
    let watched = Watched(Default::default());
    let ended = watched.0.clone();
    std::thread::spawn(move || {
        if let Ok(status) = child.wait() {
            *ended.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(status);
        }
    });
    Ok(watched)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_ends_at_once_is_seen_to_end() {
        let dir = std::env::temp_dir().join(format!("one-mk-proc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("out.log");
        let quick = spawn_watched(Path::new("/bin/sh"), &["-c".into(), "echo gone; exit 3".into()], Some(&log)).unwrap();
        let slow = spawn_watched(Path::new("/bin/sh"), &["-c".into(), "sleep 3".into()], None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while quick.ended().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(quick.ended().and_then(|s| s.code()), Some(3));
        assert!(slow.ended().is_none(), "still running");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "gone\n");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&log).unwrap().permissions().mode() & 0o777, 0o600, "a new log is private");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(all(test, unix))]
mod daemon_log_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// `daemon.log` holds what the service printed (names of computers and
    /// networks): private from the first byte, whoever started it and
    /// whatever their umask.
    #[test]
    fn the_daemon_log_is_created_private() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("daemon.log");
        spawn_detached(Path::new("/bin/sh"), &["-c".to_owned(), "echo started".to_owned()], Some(&log)).unwrap();
        assert_eq!(std::fs::metadata(&log).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
