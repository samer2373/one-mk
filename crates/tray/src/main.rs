//! `one-mk-tray`: an optional tray icon for one-mk. It shows whether
//! one-mk is connected, lets you move a computer to another side of the
//! screen, take control back, and turn "start at login" on or off. It talks
//! to the running `one-mk` only through its local control channel and the
//! `one-mk` program, so one-mk works the same without it.
//!
//! `one-mk-tray [--home DIR] [--start-daemon]`: with `--start-daemon` it
//! also starts `one-mk run` (hidden) if none is running; that is what the
//! Windows login entry uses.

// No console window on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod core;
#[cfg(any(windows, target_os = "macos"))]
mod desktop;
mod icon;
#[cfg(target_os = "linux")]
mod linux;
mod model;

use std::path::PathBuf;
use std::sync::Arc;

struct Args {
    home: Option<PathBuf>,
    start_daemon: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { home: std::env::var_os("ONE_MK_HOME").map(PathBuf::from), start_daemon: false };
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--home") => args.home = Some(it.next().ok_or("--home needs a folder")?.into()),
            Some("--start-daemon") => args.start_daemon = true,
            _ => return Err(format!("unknown option {a:?}\nusage: one-mk-tray [--home DIR] [--start-daemon]")),
        }
    }
    Ok(args)
}

fn main() {
    let result = parse_args().map_err(anyhow::Error::msg).and_then(|args| {
        let ctx = Arc::new(core::Ctx::new(args.home)?);
        #[cfg(target_os = "linux")]
        return linux::run(ctx, args.start_daemon);
        #[cfg(any(windows, target_os = "macos"))]
        return desktop::run(ctx, args.start_daemon);
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        anyhow::bail!("no tray support on this system")
    });
    if let Err(e) = result {
        eprintln!("one-mk-tray: {e:#}");
        std::process::exit(1);
    }
}
