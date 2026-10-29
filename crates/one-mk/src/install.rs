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

pub fn uninstall() -> Result<()> {
    println!("{}", ctl::uninstall()?);
    crate::integrate::remove()?;
    Ok(())
}
