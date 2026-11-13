use anyhow::{Context, Result};
use one_mk_wallpaper::{Media, PlayerConfig};
use std::io::BufRead;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

mod picture;
#[cfg(any(target_os = "macos", windows, test))]
mod playable;

#[derive(Debug)]
enum Input { End, Offset(i64), Pause(bool) }

fn input() -> Receiver<Input> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line == "quit" { break; }
            if let Some(offset) = line.strip_prefix("offset ").and_then(|offset| offset.parse().ok()) {
                let _ = sender.send(Input::Offset(offset));
                wake();
            }
            // Pause holds the frame on screen; play goes on in step.
            if line == "pause" || line == "play" {
                let _ = sender.send(Input::Pause(line == "pause"));
                wake();
            }
        }
        let _ = sender.send(Input::End);
        wake();
    });
    receiver
}

fn wake() {
    #[cfg(target_os = "macos")]
    macos::wake();
    #[cfg(target_os = "linux")]
    linux::wake();
    #[cfg(windows)]
    windows::wake();
}

fn main() -> Result<()> {
    // zbus' blocking API (the GNOME bridge) drives its calls on a Tokio
    // runtime that starts one worker per CPU and keeps them all for the life
    // of the process; one is all a few D-Bus calls need. Set before any
    // thread exists, so nothing reads the environment meanwhile.
    #[cfg(target_os = "linux")]
    if std::env::var_os("TOKIO_WORKER_THREADS").is_none() {
        // SAFETY: the first statement of `main`; no other thread runs yet.
        unsafe { std::env::set_var("TOKIO_WORKER_THREADS", "1") };
    }
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.first().is_some_and(|argument| argument.to_string_lossy().starts_with("--picture-")) {
        return picture::command(&arguments);
    }
    if arguments.first().is_some_and(|argument| argument == "--still") {
        #[cfg(target_os = "linux")]
        {
            let video = Path::new(arguments.get(1).context("Missing video")?);
            let output = Path::new(arguments.get(2).context("Missing still output")?);
            let width = arguments.get(3).and_then(|width| width.to_str()?.parse().ok()).context("Missing still width")?;
            return linux::still(video, output, width);
        }
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("A first-frame still is only needed on Linux");
    }
    if arguments.first().is_some_and(|argument| argument == "--prepare") {
        let path = Path::new(arguments.get(1).context("Choose a video first")?);
        let output = Path::new(arguments.get(2).context("Missing prepared video path")?);
        let target: one_mk_wallpaper::ImportTarget = serde_json::from_str(&arguments.get(3).context("Missing video size")?.to_string_lossy())?;
        anyhow::ensure!(target.width >= 2 && target.height >= 2 && target.width <= 16384 && target.height <= 16384, "Invalid video size");
        #[cfg(target_os = "linux")]
        return linux::prepare(path, output, target);
        #[cfg(target_os = "macos")]
        return macos::prepare(path, output, target);
        #[cfg(windows)]
        return windows::prepare(path, output, target);
    }
    if arguments.first().is_some_and(|argument| argument == "--probe") {
        let path = arguments.get(1).context("Choose a video first")?;
        let poster = arguments.get(2).map(Path::new);
        let media: Media = platform_probe(Path::new(path), poster)?;
        println!("{}", serde_json::to_string(&media)?);
        return Ok(());
    }
    let path = arguments.first().context("Missing wallpaper plan")?;
    let config: PlayerConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    anyhow::ensure!(config.plan.valid(), "Invalid wallpaper plan");
    if !config.plan.picture { one_mk_wallpaper::availability().map_err(anyhow::Error::msg)?; }
    let result = platform_run(config, input());
    if let Err(error) = &result { one_mk_wallpaper::event(one_mk_wallpaper::Playback::Error(error.to_string())); }
    result
}

fn platform_probe(path: &Path, poster: Option<&Path>) -> Result<Media> {
    #[cfg(target_os = "linux")]
    return linux::probe(path, poster);
    #[cfg(target_os = "macos")]
    return macos::probe(path, poster);
    #[cfg(windows)]
    return windows::probe(path, poster);
}

fn platform_run(config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux::run(config, receiver);
    #[cfg(target_os = "macos")]
    return macos::run(config, receiver);
    #[cfg(windows)]
    return windows::run(config, receiver);
}
