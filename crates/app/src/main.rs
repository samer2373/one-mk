//! `one-mk-app`: the one·mk window. It shows your computers on a desk,
//! lets you arrange them, pair new ones and send files by dropping them on
//! a computer. It only talks to the running `one-mk` (through its control
//! channel and the `one-mk` program), so closing it changes nothing, and it
//! costs nothing when it isn't open.
//!
//! `one-mk-app [--home DIR]`. `ONE_MK_REDUCE_MOTION=1` (or the desktop's
//! own setting) keeps movement to fades.

// No console window on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod daemon;
mod motion;
mod pairing;
mod rtt;
mod scene;
mod style;
mod tune;
mod view;
mod widgets;

use std::path::PathBuf;

fn parse_args() -> Result<Option<PathBuf>, String> {
    let mut home = std::env::var_os("ONE_MK_HOME").map(PathBuf::from);
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--home") => home = Some(it.next().ok_or("--home needs a folder")?.into()),
            _ => return Err(format!("unknown option {a:?}\nusage: one-mk-app [--home DIR]")),
        }
    }
    Ok(home)
}

fn main() {
    let mut args = Vec::new();
    let mut input = std::env::args_os().skip(1);
    while let Some(arg) = input.next() {
        if arg == "--home" { let _ = input.next(); } else { args.push(arg); }
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("one·mk window — arrange computers, pair and manage sharing\n\nUsage: one-mk-app [OPTIONS]\n\nOptions:\n      --home DIR  Use this settings folder (or ONE_MK_HOME)\n  -h, --help      Show this help\n  -V, --version   Show the version\n\nClosing the window does not stop sharing. Use Stop service to stop sharing entirely.");
        return;
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("one-mk-app {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let home = match parse_args() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("one-mk-app: {e}");
            std::process::exit(2);
        }
    };
    #[allow(unused_mut)]
    let mut size = [980.0, 660.0];
    // Development aid (see app.rs `shot`): ONE_MK_APP_SIZE=760x540.
    #[cfg(debug_assertions)]
    if let Some((w, h)) = std::env::var("ONE_MK_APP_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }) {
        size = [w, h];
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("one·mk")
            .with_app_id("one-mk")
            .with_inner_size(size)
            .with_icon(window_icon())
            .with_min_inner_size([820.0, 540.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "one-mk",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, home, size).map_err(|e| e.to_string())?))),
    );
    if let Err(e) = result {
        eprintln!("one-mk-app: {e}");
        std::process::exit(1);
    }
}

/// The app icon, for the title bar, task switcher and dock.
fn window_icon() -> std::sync::Arc<egui::IconData> {
    let ([width, height], rgba) = app::embedded(include_bytes!("../assets/icon-256.rgba"));
    std::sync::Arc::new(egui::IconData { rgba: rgba.to_vec(), width: width as u32, height: height as u32 })
}
