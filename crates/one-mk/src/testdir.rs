//! Scratch folders for unit tests that do not pile up in `/tmp`. Tests used to
//! name folders `/tmp/one-mk-<name>-<pid>` and delete them by hand at the end
//! (or not): every run left the ones whose test failed or forgot, a few
//! hundred folders after a few weeks of work.
//!
//! [`scratch`] and [`scratch_path`] return a plain `PathBuf` (so a test can
//! hand it to a daemon or clone it into a task) under a folder of this test
//! program's own, and a guard removes that whole folder when the program
//! exits, also after a failing test. (`TempDir` of the `tempfile` crate is
//! the same idea for a test that can keep a guard in scope; it does not fit
//! where the path itself is passed around.)

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
static REGISTER: Once = Once::new();

/// This test program's own folder, made on first use and removed at exit.
fn root() -> PathBuf {
    let mut root = ROOT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    root.get_or_insert_with(|| {
        let prefix = format!("one-mk-tests-{}-", std::process::id());
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix);
        // macOS's temporary folder is long (`/var/folders/xx/.../T/`), and a
        // control socket's path may be at most 103 characters there: keep the
        // tests' folders short.
        let made = if cfg!(target_os = "macos") && Path::new("/tmp").is_dir() { builder.tempdir_in("/tmp") } else { builder.tempdir() }.expect("a scratch folder").keep();
        // Private whatever the umask: the tests of code that refuses a path
        // others could change look at every folder above theirs.
        #[cfg(unix)]
        std::fs::set_permissions(&made, std::os::unix::fs::PermissionsExt::from_mode(0o700)).expect("a private scratch folder");
        REGISTER.call_once(|| {
            #[cfg(unix)]
            // SAFETY: registers a plain function with no arguments that only removes a folder.
            unsafe { libc::atexit(remove_root) };
        });
        made
    }).clone()
}

#[cfg(unix)]
extern "C" fn remove_root() {
    // Never wait for a lock at exit: a thread that holds it would hang the program.
    if let Ok(mut root) = ROOT.try_lock() && let Some(root) = root.take() {
        let _ = std::fs::remove_dir_all(root);
    }
}

/// A new, empty, existing folder named after `tag`.
pub fn scratch(tag: &str) -> PathBuf {
    let path = scratch_path(tag);
    std::fs::create_dir_all(&path).expect("a scratch folder");
    path
}

/// A path for a folder that does not exist yet (its parent does and is
/// unique), for tests of code that creates it.
pub fn scratch_path(tag: &str) -> PathBuf {
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let parent = root().join(format!("{}-{tag}", COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    // Private all the way down, as the old folders under a sticky `/tmp` were
    // for the tests of code that refuses a path others could change.
    #[allow(unused_mut)] // only changed on Unix
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&parent).expect("a scratch folder");
    parent.join("folder")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_unique_where_asked_and_absent_where_asked() {
        let (a, b) = (scratch("same-tag"), scratch("same-tag"));
        assert!(a.is_dir() && b.is_dir() && a != b);
        std::fs::write(a.join("file"), "x").unwrap();
        let fresh = scratch_path("not-made");
        assert!(!fresh.exists() && fresh.parent().unwrap().is_dir());
        // All of them under one folder named for this program, which `remove_root` deletes.
        assert!(a.starts_with(root()) && fresh.starts_with(root()));
        assert!(root().file_name().unwrap().to_string_lossy().starts_with("one-mk-tests-"));
    }
}
