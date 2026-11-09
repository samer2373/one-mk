//! `one-mk install` / `uninstall`: start one-mk when you log in. The work
//! is in the `one-mk-ctl` crate, which the tray uses too.

use crate::control;
use anyhow::Result;
use one_mk_ctl::install::{self as ctl, Launch};
use std::path::Path;

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
/// that fails is said and the others still run.
pub async fn uninstall(dir: &Path) -> Result<()> {
    println!("{}", ctl::uninstall()?);
    crate::integrate::remove()?;
    let mut failed = false;
    match crate::live_wallpaper::turn_off_for_uninstall(dir).await {
        Ok(()) => println!("oneMK's wallpaper is off; your own wallpaper is in place."),
        Err(e) => { failed = true; eprintln!("Could not put your own wallpaper back: {e:#}. Run `one-mk wallpaper off` while oneMK runs, or set it again in your desktop's settings."); }
    }
    if cfg!(target_os = "linux") {
        match crate::wallpaper::extension::remove().await {
            Ok(message) => println!("{message}"),
            Err(e) => { failed = true; eprintln!("{e:#}"); }
        }
    }
    anyhow::ensure!(!failed, "Uninstall finished, but the steps above need attention");
    Ok(())
}
