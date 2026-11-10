//! What every writer of one-mk's small settings files shares: a lock that
//! is never waited for without end, and flushing the folder after a file
//! was renamed into it.

use anyhow::{Context, Result};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long the daemon waits for its turn to change a settings file before
/// it gives up on that one change: its event loop (the idle and lease
/// timers, the take-back chord) must never hang on a command that stopped
/// while holding a lock.
pub const DAEMON_LOCK_WAIT: Duration = Duration::from_secs(2);

/// How long a `one-mk` command (or the window) waits for its turn. A command
/// holds a lock only for the moment it takes to read and write one file,
/// so a longer wait means one was stopped (Ctrl+Z, a debugger) holding it.
pub const COMMAND_LOCK_WAIT: Duration = Duration::from_secs(10);

/// Takes an exclusive advisory lock on `path` (created if missing), waiting
/// at most `wait`. Released when the file is dropped.
pub fn lock_file(path: &Path, wait: Duration) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path).with_context(|| format!("opening {}", path.display()))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Err(std::fs::TryLockError::WouldBlock) => anyhow::bail!(
                "another one-mk command is holding the lock {} (waited {:.0?}); try again when it finishes, or end that command",
                path.display(),
                wait
            ),
            Err(std::fs::TryLockError::Error(e)) => return Err(e).with_context(|| format!("locking {}", path.display())),
        }
    }
}

/// Flushes `dir` itself to disk, so a file just renamed into it is still
/// there after a crash or power loss. Where folders can't be flushed
/// (Windows, where a rename is durable once it returns; file systems that
/// refuse it), this does nothing; any other failure is an error.
pub fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let folder = std::fs::File::open(dir).with_context(|| format!("opening {} to flush it", dir.display()))?;
        match folder.sync_all() {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported || e.raw_os_error() == Some(libc_einval()) => {}
            Err(e) => return Err(e).with_context(|| format!("flushing {}", dir.display())),
        }
        #[cfg(debug_assertions)]
        SYNCED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(dir.to_owned());
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// EINVAL: what Linux and macOS answer for a folder that can't be flushed.
#[cfg(unix)]
const fn libc_einval() -> i32 {
    22
}

/// Every folder flushed so far (debug builds only), for tests.
#[cfg(all(unix, debug_assertions))]
static SYNCED: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Whether [`sync_dir`] flushed `dir` (debug builds on Unix only).
#[doc(hidden)]
#[cfg(all(unix, debug_assertions))]
pub fn was_synced(dir: &Path) -> bool {
    SYNCED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().any(|d| d == dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_is_waited_for_a_bounded_time() {
        let dir = std::env::temp_dir().join(format!("one-mk-lock-{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.lock");
        let held = lock_file(&path, Duration::ZERO).unwrap();
        let start = Instant::now();
        let err = lock_file(&path, Duration::from_millis(100)).unwrap_err();
        assert!(start.elapsed() >= Duration::from_millis(100) && start.elapsed() < Duration::from_secs(2));
        assert!(format!("{err:#}").contains("another one-mk command is holding the lock"), "{err:#}");
        drop(held);
        drop(lock_file(&path, Duration::ZERO).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn flushing_a_missing_folder_is_an_error() {
        let dir = std::env::temp_dir().join(format!("one-mk-sync-{}-{}", std::process::id(), rand::random::<u64>()));
        assert!(sync_dir(&dir).is_err());
        std::fs::create_dir_all(&dir).unwrap();
        sync_dir(&dir).unwrap();
        #[cfg(debug_assertions)]
        assert!(was_synced(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }
}
