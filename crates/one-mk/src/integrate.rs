//! `one-mk integrate [--remove]`: "Send to <computer>" entries in the file
//! manager. The files themselves are made in `one_mk_ctl::integrate`; this
//! reads the paired computers.

use anyhow::{Result, bail};
use one_mk_ctl::integrate::{self as ctl, Computer, Setup};
use one_mk_net::TrustStore;
use std::path::Path;

fn setup(dir: &Path, custom_home: bool) -> Result<Setup> {
    let trust = TrustStore::load(dir)?;
    Ok(Setup {
        exe: one_mk_ctl::install::program_path()?,
        home: custom_home.then(|| one_mk_ctl::install::absolute(dir)),
        computers: trust.devices().iter().filter_map(|d| Computer::new(&d.fingerprint, &d.name)).collect(),
    })
}

/// `one-mk integrate`, or `--remove`.
pub fn run(dir: &Path, custom_home: bool, remove: bool) -> Result<()> {
    if remove {
        println!("{}", removed(ctl::remove()?));
    } else {
        let done = ctl::install(&setup(dir, custom_home)?)?;
        println!("{}", done.message);
        if done.skipped > 0 {
            // Some of the menu is missing: say so in the exit code, too.
            bail!("{} \"Send to\" entr{} skipped: something of yours is in the way", done.skipped, if done.skipped == 1 { "y was" } else { "ies were" });
        }
    }
    Ok(())
}

/// Called by `uninstall`.
pub fn remove() -> Result<()> {
    let n = ctl::remove()?;
    if n > 0 {
        println!("{}", removed(n));
    }
    Ok(())
}

fn removed(n: usize) -> String {
    match n {
        0 => "There were no \"Send to\" file manager entries to remove.".to_owned(),
        n => format!("Removed {n} \"Send to\" file manager entr{}.", if n == 1 { "y" } else { "ies" }),
    }
}

/// After a computer is paired or forgotten: keeps the entries in step, if
/// they were ever added. Never fails the command it follows.
pub fn refresh(dir: &Path, custom_home: bool) {
    if !ctl::is_enabled() {
        return;
    }
    if let Err(e) = setup(dir, custom_home).and_then(|s| ctl::refresh(&s)) {
        tracing::warn!("could not update the \"Send to\" file manager entries: {e:#}");
    }
}
