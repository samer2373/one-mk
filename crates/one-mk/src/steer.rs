//! Commands that steer a running daemon: `switch`, `lock-pointer`,
//! `lock-all` and `broadcast`. Replies are a status word on the first line,
//! then details.

use crate::control::{self, Request};
use anyhow::{Result, bail};
pub use one_mk_ctl::client::{SwitchTo, Toggle};
use std::path::Path;

/// Sends `req`; `None` (after saying so) if one-mk is not running.
async fn ask(dir: &Path, req: Request) -> Result<Option<(String, String)>> {
    let Some(reply) = control::request(dir, req).await? else {
        println!("one-mk is not running. Start it with `one-mk run` (or `one-mk install`).");
        return Ok(None);
    };
    let (word, rest) = reply.split_once('\n').unwrap_or((&reply, ""));
    if let Some(msg) = word.strip_prefix("error ") {
        bail!("{msg}");
    }
    Ok(Some((word.to_owned(), rest.trim().to_owned())))
}

/// `one-mk switch <computer|here|next>`.
pub async fn switch(dir: &Path, to: &str) -> Result<()> {
    let Some(to) = SwitchTo::parse(to) else { bail!("say which computer: a name or id, `here` or `next`") };
    let Some((word, detail)) = ask(dir, Request::Switch(to)).await? else { return Ok(()) };
    match word.as_str() {
        "ok" => println!("The mouse and keyboard are going to {detail}."),
        "already" => println!("The mouse and keyboard are already on {detail}."),
        "here" if detail.is_empty() => println!("The mouse and keyboard are already on this computer."),
        "here" => println!("The mouse and keyboard are back on this computer ({detail})."),
        _ => bail!("unexpected answer from one-mk: {word:?}"),
    }
    Ok(())
}

/// `one-mk lock-pointer on|off|toggle`.
pub async fn lock_pointer(dir: &Path, t: Toggle) -> Result<()> {
    let Some((word, detail)) = ask(dir, Request::LockPointer(t)).await? else { return Ok(()) };
    match (word.as_str(), detail.as_str()) {
        ("ok", "on") => println!("The pointer is locked to this computer: edges lead nowhere. `one-mk lock-pointer off` unlocks it."),
        ("ok", "off") => println!("The pointer is free to cross edges again."),
        _ => bail!("unexpected answer from one-mk: {word:?}"),
    }
    Ok(())
}

/// `one-mk broadcast [on|off|toggle] [--idle-off MINUTES]`: starts or stops
/// broadcast, or with neither says what it is doing; `--idle-off` changes
/// when it stops by itself.
pub async fn broadcast(dir: &Path, state: Option<Toggle>, idle_off: Option<u32>) -> Result<()> {
    use crate::settings::{MAX_IDLE_OFF_MINUTES, Settings};
    if let Some(minutes) = idle_off {
        if minutes > MAX_IDLE_OFF_MINUTES {
            bail!("--idle-off takes at most {MAX_IDLE_OFF_MINUTES} minutes (a day); 0 means it never stops by itself");
        }
        let b = Settings::update(dir, |s| {
            s.broadcast.idle_off_minutes = minutes;
            Ok(s.broadcast.clone())
        })?;
        println!("Broadcast now stops {}.", b.describe_idle_off());
        crate::settings::apply(dir).await?;
    }
    let Some(t) = state else {
        if idle_off.is_none() {
            show_broadcast(dir, &Settings::load(dir)?.broadcast).await?;
        }
        return Ok(());
    };
    let Some(reply) = control::request(dir, Request::Broadcast(t)).await? else {
        bail!("one-mk is not running, so nothing can be broadcast. Start it with `one-mk run` (or `one-mk install`)");
    };
    let mut lines = reply.lines();
    let (word, state) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""));
    if let Some(msg) = word.strip_prefix("error ") {
        bail!("{msg}");
    }
    match (word, state) {
        ("ok" | "already", "on") => {
            if word == "already" {
                println!("Broadcast is already on.");
            }
            // Said as `one-mk status` says it, with the shortcut in force.
            let status = control::request(dir, Request::Status).await?.and_then(|r| one_mk_ctl::status::Status::decode(&r));
            match status.and_then(|s| s.broadcast_line()) {
                Some(line) => println!("{line}"),
                None => println!("Broadcasting. `one-mk broadcast off` stops it."),
            }
        }
        ("ok", "off") => println!("Broadcast stopped. The mouse and keyboard are back on this computer."),
        ("already", "off") => println!("Broadcast is already off."),
        _ => bail!("unexpected answer from one-mk: {word:?}"),
    }
    Ok(())
}

/// `one-mk broadcast` alone: on or off, and how it stops.
async fn show_broadcast(dir: &Path, settings: &crate::settings::BroadcastSettings) -> Result<()> {
    let status = control::request(dir, Request::Status).await?.and_then(|r| one_mk_ctl::status::Status::decode(&r));
    match &status {
        None => println!("one-mk is not running, so broadcast is off."),
        Some(s) => match s.broadcast_line() {
            Some(line) => println!("{line}"),
            None => {
                let key = if s.broadcast_hotkey.is_empty() { String::new() } else { format!(" or {}", s.broadcast_hotkey) };
                println!("Broadcast is off. `one-mk broadcast on`{key} sends everything you type, click and scroll here to every connected computer at once.");
            }
        },
    }
    println!(
        "Broadcast stops {}; `one-mk broadcast --idle-off MINUTES` changes that (0 = never). Ctrl+Alt+Shift+Esc always stops it.",
        settings.describe_idle_off()
    );
    Ok(())
}

/// `one-mk lock-all`.
pub async fn lock_all(dir: &Path) -> Result<()> {
    let Some((word, detail)) = ask(dir, Request::LockAll).await? else { return Ok(()) };
    if word != "ok" {
        bail!("unexpected answer from one-mk: {word:?}");
    }
    // The number of computers asked, then why this one didn't lock, if so.
    let (n, failed) = detail.split_once('\n').map_or((detail.as_str(), None), |(n, e)| (n, Some(e.trim())));
    let n = n.trim().parse::<usize>().unwrap_or(0);
    let others = match n {
        0 => String::new(),
        1 => "the connected computer".to_owned(),
        n => format!("the {n} connected computers"),
    };
    match (failed, n) {
        (None, 0) => println!("Locking this computer (no other computer is connected)."),
        (None, _) => println!("Locking this computer and asking {others} to lock too."),
        (Some(e), 0) => bail!("could not lock this computer: {e}"),
        (Some(e), _) => bail!("asked {others} to lock, but could not lock this computer: {e}"),
    }
    Ok(())
}
