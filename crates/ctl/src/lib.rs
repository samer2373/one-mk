//! The parts of one-mk that talk to an already running daemon or to the
//! operating system's login settings, shared by the command line, tray and
//! window. Wi-Fi observations use their own event-driven worker, not the
//! daemon's runtime.

pub mod analytics;
pub mod brand;
pub mod client;
pub mod install;
pub mod integrate;
pub mod macos_windows;
#[cfg(target_os = "macos")]
pub mod notification;
pub mod private;
pub mod proc;
pub mod settings_file;
pub mod setup;
pub mod status;
pub mod wallpaper;
pub mod update_logic;
pub mod wifi;
pub mod xdg;

use anyhow::{Context, Result};
use std::path::PathBuf;

/// The largest `settings.toml` one-mk reads. The daemon and the window share
/// it, so the window never shows a file the daemon refuses as applied.
pub const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;

/// Where identity and paired devices live (the same folder `one-mk` uses).
/// On Windows this is the local (non-roaming) AppData.
pub fn default_home() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "one-mk").context("no home directory")?;
    Ok(dirs.config_local_dir().to_path_buf())
}
