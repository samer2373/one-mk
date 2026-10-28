//! Commands that steer a running daemon: `switch`, `lock-pointer` and
//! `lock-all`. Replies are a status word on the first line, then details.

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

/// `one-mk lock-all`.
pub async fn lock_all(dir: &Path) -> Result<()> {
    let Some((word, detail)) = ask(dir, Request::LockAll).await? else { return Ok(()) };
    if word != "ok" {
        bail!("unexpected answer from one-mk: {word:?}");
    }
    match detail.parse::<usize>().unwrap_or(0) {
        0 => println!("Locking this computer (no other computer is connected)."),
        1 => println!("Locking this computer and asking the connected one to lock too."),
        n => println!("Locking this computer and asking the {n} connected ones to lock too."),
    }
    Ok(())
}
