//! The parts of one-mk that talk to an already running daemon or to the
//! operating system's login settings, without any networking or async
//! runtime: shared by the `one-mk` command line and the optional
//! `one-mk-tray`.

pub mod client;
pub mod install;
pub mod proc;
pub mod status;

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Where identity and paired devices live (the same folder `one-mk` uses).
/// On Windows this is the local (non-roaming) AppData.
pub fn default_home() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "one-mk").context("no home directory")?;
    Ok(dirs.config_local_dir().to_path_buf())
}
