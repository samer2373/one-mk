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

#[derive(Debug)]
enum Input { End, Offset(i64) }

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
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
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
    one_mk_wallpaper::availability().map_err(anyhow::Error::msg)?;
    let path = arguments.first().context("Missing wallpaper plan")?;
    let config: PlayerConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    anyhow::ensure!(config.plan.valid(), "Invalid wallpaper plan");
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
