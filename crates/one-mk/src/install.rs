//! `one-mk install` / `uninstall`: start one-mk when you log in. The work
//! is in the `one-mk-ctl` crate, which the tray uses too.

use crate::control;
use anyhow::Result;
use one_mk_ctl::install::{self as ctl, Launch};
use std::path::{Path, PathBuf};

/// `dir` is where this copy keeps its settings; the autostarted copy gets
/// `--home` only if `custom_home` says the user chose it.
pub async fn install(dir: &Path, custom_home: bool) -> Result<()> {
    let exe = one_mk_ctl::install::program_path()?;
    let running = control::request(dir, control::Request::Status).await.is_ok_and(|r| r.is_some());
    let launch = Launch {
        tray: ctl::sibling(&exe, "one-mk-tray"),
        exe,
        home: custom_home.then(|| ctl::absolute(dir)),
        // A second copy would only be refused: leave the running one alone.
        start_now: !running,
    };
    if running {
        println!("Note: one-mk is already running; the automatic copy takes over the next time you log in.");
    }
    println!("{}", ctl::install(&launch)?);
    // The file manager's "Send to" entries: nice to have, never a reason to fail.
    if let Err(e) = crate::integrate::run(dir, custom_home, false) {
        eprintln!("Could not add the \"Send to\" file manager entries: {e:#}");
    }
    Ok(())
}

/// Leaves nothing of oneMK's on the desktop: no login entry, no file manager
/// entries, the person's own wallpaper back, and no GNOME extension. A step
/// that fails is said and the others still run. The wallpaper is put back for
/// the settings folder this command runs with and, when that is the usual one,
/// also for the folder the login entry being removed starts oneMK with (what
/// `one-mk install --home X` registered): a bare `one-mk uninstall` removes
/// that entry, so it must not leave its wallpaper playing.
pub async fn uninstall(dir: &Path) -> Result<()> {
    let mut failed = false;
    // Read before the entry is removed.
    let registered = ctl::registered_home();
    match ctl::uninstall() {
        Ok(message) => println!("{message}"),
        Err(e) => { failed = true; eprintln!("Could not remove the login entry: {e:#}"); }
    }
    if let Err(e) = crate::integrate::remove() {
        failed = true;
        eprintln!("Could not remove the \"Send to\" file manager entries: {e:#}");
    }
    let explicit = one_mk_ctl::default_home().map_or(true, |usual| !same_folder(dir, &usual));
    failed |= put_wallpaper_back(dir, &wallpaper_homes(dir, registered.as_deref(), explicit)).await;
    if cfg!(target_os = "linux") {
        match crate::wallpaper::extension::remove().await {
            Ok(message) => println!("{message}"),
            Err(e) => { failed = true; eprintln!("{e:#}"); }
        }
    }
    anyhow::ensure!(!failed, "Uninstall finished, but the steps above need attention");
    Ok(())
}

/// Turns the wallpaper off for each folder in `homes` (`dir` is the command's own); whether a step failed.
async fn put_wallpaper_back(dir: &Path, homes: &[PathBuf]) -> bool {
    let mut failed = false;
    for home in homes {
        let own = same_folder(home, dir);
        match crate::live_wallpaper::turn_off_for_uninstall(home).await {
            Ok(()) if own => println!("oneMK's wallpaper is off; your own wallpaper is in place."),
            Ok(()) => println!("oneMK's wallpaper is off for {} as well; your own wallpaper is in place.", home.display()),
            Err(e) => {
                failed = true;
                let which = if own { String::new() } else { format!(" (settings folder {})", home.display()) };
                let run = if own { "`one-mk wallpaper off`".to_owned() } else { format!("`one-mk --home {} wallpaper off`", home.display()) };
                eprintln!("Could not put your own wallpaper back{which}: {e:#}. Run {run} while oneMK runs, or set it again in your desktop's settings.");
            }
        }
    }
    failed
}

/// The settings folders whose wallpaper goes off: this command's own, and the
/// login entry's, unless the person named a folder (`--home`): then only theirs.
fn wallpaper_homes(dir: &Path, registered: Option<&Path>, explicit: bool) -> Vec<PathBuf> {
    let mut homes = vec![dir.to_path_buf()];
    if let Some(registered) = registered && !explicit && !same_folder(registered, dir) {
        homes.push(registered.to_path_buf());
    }
    homes
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| ctl::absolute(p));
    canonical(a) == canonical(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logins_folder_is_cleaned_up_only_when_no_folder_was_named() {
        let (usual, custom) = (Path::new("/home/u/.local/one-mk"), Path::new("/tmp/second"));
        assert_eq!(wallpaper_homes(usual, Some(custom), false), vec![usual.to_path_buf(), custom.to_path_buf()]);
        // `--home X uninstall` is about X alone.
        assert_eq!(wallpaper_homes(custom, Some(usual), true), vec![custom.to_path_buf()]);
        // No custom entry, or the entry names this very folder (through `.`): once.
        assert_eq!(wallpaper_homes(usual, None, false), vec![usual.to_path_buf()]);
        assert_eq!(wallpaper_homes(usual, Some(Path::new("/home/u/.local/one-mk/.")), false), vec![usual.to_path_buf()]);
    }

    /// The registered folder is really asked: a oneMK holding it fails the uninstall, which says
    /// which folder, while the usual folder is still handled and does not fail alone.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_registered_folder_is_turned_off_too() {
        let (usual, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let homes = wallpaper_homes(usual.path(), Some(second.path()), false);
        assert!(!put_wallpaper_back(usual.path(), &homes).await, "nothing runs: both are off");
        let held = crate::control::InstanceLock::try_acquire(second.path()).unwrap().unwrap();
        assert!(put_wallpaper_back(usual.path(), &homes).await, "the registered folder is held by a oneMK that does not answer");
        assert!(!put_wallpaper_back(usual.path(), &homes[..1]).await, "the usual folder alone is fine");
        drop(held);
    }
}
