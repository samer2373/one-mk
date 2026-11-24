//! Changes to `settings.toml` the window makes itself, without running
//! `one-mk`: settings only the window reads (`[window] appearance`), so the
//! choice is saved even when the service program is missing, older than
//! the window, or not running. It takes the same lock as every `one-mk`
//! command (`settings.toml.lock`), changes only its own key, and replaces
//! the file whole (a new file renamed over it), as the daemon does.

use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const FILE: &str = "settings.toml";
const LOCK_FILE: &str = "settings.toml.lock";

/// How long the window waits for its turn to change the file: as long as
/// a `one-mk` command does (`one_mk_net::COMMAND_LOCK_WAIT`).
pub const LOCK_WAIT: Duration = Duration::from_secs(10);

/// Saves Settings → Appearance (`system`, `light` or `dark`) as
/// `[window] appearance`, keeping everything else in the file as it is.
pub fn set_window_appearance(dir: &Path, appearance: &str) -> Result<()> {
    if !matches!(appearance, "system" | "light" | "dark") {
        bail!("{appearance:?} is not an appearance (system, light or dark)");
    }
    update(dir, LOCK_WAIT, |table| {
        let window = table.entry("window").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let Some(window) = window.as_table_mut() else { bail!("[window] in {FILE} is not a table; fix or remove it, then choose again") };
        window.insert("appearance".into(), toml::Value::String(appearance.into()));
        Ok(())
    })
}

/// Waits for the settings lock, reads the file (missing: empty), applies
/// `change` and writes it back whole. A file that can't be read or parsed
/// is left alone and reported: never replaced by one that lost its settings.
fn update(dir: &Path, wait: Duration, change: impl FnOnce(&mut toml::Table) -> Result<()>) -> Result<()> {
    crate::private::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let _lock = lock(&dir.join(LOCK_FILE), wait)?;
    let path = dir.join(FILE);
    let mut table = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() > crate::MAX_SETTINGS_BYTES => bail!("{} is too large to be settings; fix or remove it, then choose again", path.display()),
        Ok(_) => {
            // Bounded by what is read, not only by the size seen a moment ago.
            let mut text = String::new();
            let file = std::fs::File::open(&path).with_context(|| format!("reading {}", path.display()))?;
            file.take(crate::MAX_SETTINGS_BYTES + 1).read_to_string(&mut text).with_context(|| format!("reading {}", path.display()))?;
            if text.len() as u64 > crate::MAX_SETTINGS_BYTES { bail!("{} is too large to be settings; fix or remove it, then choose again", path.display()) }
            toml::from_str::<toml::Table>(&text).with_context(|| format!("{} can't be read; fix or remove it, then choose again", path.display()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    change(&mut table)?;
    let text = toml::to_string_pretty(&table)?;
    // What is saved must be loadable: pretty printing and the new key can grow a file that was just under the cap.
    if text.len() as u64 > crate::MAX_SETTINGS_BYTES {
        bail!("{} would be too large to load as settings with this change; fix or remove it, then choose again", path.display());
    }
    static N: AtomicU32 = AtomicU32::new(0);
    // Through a symlink (a settings file kept in a dotfiles repo) the file it points to is replaced,
    // not the link, as the daemon and the commands do; the new file is made beside that one so the
    // rename stays on one file system.
    let target = match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()),
        _ => path.clone(),
    };
    let folder = target.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(dir).to_owned();
    let tmp = folder.join(format!("{FILE}.{}.{}.window.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let write = || -> Result<()> {
        use std::io::Write;
        let mut file = crate::private::create_new(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(text.as_bytes()).with_context(|| format!("writing {}", tmp.display()))?;
        // On disk before it replaces the old file: a crash leaves one or the other.
        file.sync_all().with_context(|| format!("flushing {}", tmp.display()))?;
        drop(file);
        std::fs::rename(&tmp, &target).with_context(|| format!("saving {}", target.display()))?;
        sync_dir(&folder);
        Ok(())
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The exclusive advisory lock every writer of `settings.toml` takes (the
/// daemon's and the commands' `one_mk_net::lock_file`), waited for at most
/// `wait`. Released when the file is dropped.
fn lock(path: &Path, wait: Duration) -> Result<std::fs::File> {
    let file = crate::private::lock_file(path).with_context(|| format!("opening {}", path.display()))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Err(std::fs::TryLockError::WouldBlock) => {
                bail!("another one-mk command is holding the lock {} (waited {wait:.0?}); try again when it finishes", path.display())
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e).with_context(|| format!("locking {}", path.display())),
        }
    }
}

/// Flushes the folder so the renamed file survives a crash (Unix; a
/// rename is durable once it returns on Windows). Best effort: the file
/// itself is already saved.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(folder) = std::fs::File::open(dir) {
        let _ = folder.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn appearance_is_saved_into_a_missing_file() {
        let d = dir();
        let home = d.path().join("home");
        set_window_appearance(&home, "dark").unwrap();
        let table: toml::Table = toml::from_str(&std::fs::read_to_string(home.join(FILE)).unwrap()).unwrap();
        assert_eq!(table["window"]["appearance"].as_str(), Some("dark"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(home.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_settings_symlink_is_kept_and_its_target_updated() {
        let d = dir();
        let (home, repo) = (d.path().join("home"), d.path().join("dotfiles"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("settings.toml"), "gaming = \"auto\"\n").unwrap();
        std::os::unix::fs::symlink(repo.join("settings.toml"), home.join(FILE)).unwrap();
        set_window_appearance(&home, "dark").unwrap();
        assert!(std::fs::symlink_metadata(home.join(FILE)).unwrap().file_type().is_symlink());
        let table: toml::Table = toml::from_str(&std::fs::read_to_string(repo.join("settings.toml")).unwrap()).unwrap();
        assert_eq!(table["window"]["appearance"].as_str(), Some("dark"));
        assert_eq!(table["gaming"].as_str(), Some("auto"));
        let stray = |d: &Path| std::fs::read_dir(d).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!stray(&home) && !stray(&repo));
    }

    #[test]
    fn appearance_keeps_every_other_setting() {
        let d = dir();
        let text = "settings_version = 3\ngaming = \"auto\"\n\n[clipboard]\ntext = false\nmax_mb = 9\n\n[window]\nappearance = \"light\"\nfuture = 1\n\n[edge.left]\ndwell_ms = 300\n";
        std::fs::write(d.path().join(FILE), text).unwrap();
        set_window_appearance(d.path(), "dark").unwrap();
        let after: toml::Table = toml::from_str(&std::fs::read_to_string(d.path().join(FILE)).unwrap()).unwrap();
        let mut want: toml::Table = toml::from_str(text).unwrap();
        want["window"].as_table_mut().unwrap().insert("appearance".into(), "dark".into());
        assert_eq!(after, want);
        let left: Vec<_> = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(left.iter().all(|n| n == FILE || n == LOCK_FILE), "stray files: {left:?}");
    }

    #[test]
    fn a_change_that_would_push_the_file_over_the_reader_s_cap_is_refused() {
        let d = dir();
        // Compact and just under the cap, but one entry per line once pretty printed.
        let mut text = String::from("x = [");
        while (text.len() as u64) < crate::MAX_SETTINGS_BYTES - 16 { text.push_str("\"a\","); }
        text.push_str("]\n");
        assert!((text.len() as u64) < crate::MAX_SETTINGS_BYTES);
        std::fs::write(d.path().join(FILE), &text).unwrap();
        assert!(set_window_appearance(d.path(), "dark").is_err());
        assert_eq!(std::fs::read_to_string(d.path().join(FILE)).unwrap(), text);
        let left: Vec<_> = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(left.iter().all(|n| n == FILE || n == LOCK_FILE), "stray files: {left:?}");
    }

    #[test]
    fn an_unreadable_file_is_left_alone() {
        let d = dir();
        let broken = "gaming = \"auto\"\n[clipboard\n";
        std::fs::write(d.path().join(FILE), broken).unwrap();
        let e = set_window_appearance(d.path(), "light").unwrap_err();
        assert!(format!("{e:#}").contains("can't be read"), "{e:#}");
        assert_eq!(std::fs::read_to_string(d.path().join(FILE)).unwrap(), broken);
        std::fs::write(d.path().join(FILE), "window = 3\n").unwrap();
        assert!(set_window_appearance(d.path(), "light").is_err());
        assert_eq!(std::fs::read_to_string(d.path().join(FILE)).unwrap(), "window = 3\n");
        assert!(set_window_appearance(d.path(), "purple").is_err());
    }

    #[test]
    fn a_held_lock_is_waited_for_then_reported() {
        let d = dir();
        let held = lock(&d.path().join(LOCK_FILE), Duration::ZERO).unwrap();
        let started = Instant::now();
        let e = update(d.path(), Duration::from_millis(150), |_| Ok(())).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert!(format!("{e:#}").contains("holding the lock"), "{e:#}");
        assert!(!d.path().join(FILE).exists());
        drop(held);
        set_window_appearance(d.path(), "light").unwrap();
    }

    #[test]
    fn writers_at_once_all_land() {
        let d = dir();
        std::fs::write(d.path().join(FILE), "gaming = \"auto\"\n").unwrap();
        let path = d.path().to_owned();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    update(&path, LOCK_WAIT, |t| {
                        t.insert(format!("k{i}"), toml::Value::Integer(i));
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        let after: toml::Table = toml::from_str(&std::fs::read_to_string(path.join(FILE)).unwrap()).unwrap();
        assert_eq!(after.len(), 9);
    }
}
