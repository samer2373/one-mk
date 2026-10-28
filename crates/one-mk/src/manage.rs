//! The commands that talk to a running daemon or change its settings:
//! `status`, `takeback` and `layout`.

use crate::control::{self, Request};
use anyhow::{Result, bail};
use one_mk_net::{Device, Swap, TrustStore};
use one_mk_proto::Side;
pub use one_mk_ctl::status::{PeerStatus, Status, on_your, takeback_hint};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unknown_device(device: &str) -> anyhow::Error {
    anyhow::anyhow!("no paired computer called {device:?}; see `one-mk devices` for names and ids")
}

pub async fn status(dir: &Path) -> Result<()> {
    let reply = control::request(dir, Request::Status).await?;
    match reply.as_deref().and_then(Status::decode) {
        Some(s) => print!("{}", s.render()),
        None if reply.is_some() => bail!("unexpected answer from one-mk: {reply:?}"),
        None => {
            println!("one-mk is not running.");
            println!("Start it with `one-mk run`, or make it start when you log in with `one-mk install`.");
        }
    }
    Ok(())
}

/// `one-mk takeback`: makes the computer controlling this one let go.
pub async fn takeback(dir: &Path) -> Result<()> {
    let reply = control::request(dir, Request::TakeBack).await?;
    match reply.as_deref() {
        None => println!("one-mk is not running."),
        Some("none\n") => println!("No other computer is controlling this one."),
        Some(r) => match r.strip_prefix("ok\n") {
            Some(name) if !name.trim().is_empty() => println!("Took control back from {}.", name.trim()),
            Some(_) => println!("Took control back."),
            None => bail!("unexpected answer from one-mk: {r:?}"),
        },
    }
    Ok(())
}

/// Changes a paired device's side and/or Ctrl/Cmd setting in `devices.toml`.
/// A new side gets a version newer than any seen, so it wins when the two
/// computers compare layouts.
pub fn set_layout(dir: &Path, device: &str, side: Option<Side>, swap: Option<Swap>) -> Result<Device> {
    let mut trust = TrustStore::load(dir)?;
    let Some(found) = trust.find(device) else { return Err(unknown_device(device)) };
    let Some(fp) = found.fingerprint() else { bail!("corrupt fingerprint for {device:?} in devices.toml") };
    let dev = trust.get_mut(&fp).expect("found above");
    if let Some(side) = side {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        dev.side = side.to_string();
        dev.layout_version = now.max(dev.layout_version + 1);
    }
    if let Some(swap) = swap {
        dev.swap_ctrl_cmd = swap;
    }
    let dev = dev.clone();
    trust.save()?;
    Ok(dev)
}

pub async fn layout(dir: &Path, device: &str, side: Option<Side>, swap: Option<Swap>) -> Result<()> {
    if side.is_none() && swap.is_none() {
        bail!("say where {device:?} is (left, right, top or bottom) and/or use --swap-ctrl-cmd on|off|auto");
    }
    let dev = set_layout(dir, device, side, swap)?;
    let name = one_mk_proto::sanitize_name(&dev.name);
    if let Some(side) = side {
        println!("{name} is now {}.", on_your(side));
    }
    match swap {
        Some(Swap::On) => println!("Ctrl and Cmd are swapped for keys sent to {name}."),
        Some(Swap::Off) => println!("Ctrl and Cmd are not swapped for keys sent to {name}."),
        Some(Swap::Auto) => println!("Ctrl and Cmd are swapped for keys sent to {name} only between a Mac and another computer."),
        None => {}
    }
    if control::request(dir, Request::Reload).await?.is_some() {
        println!("Applied right away.{}", if side.is_some() { " Its computer picks up the change too." } else { "" });
    } else {
        println!("one-mk is not running; this takes effect the next time it starts.");
    }
    Ok(())
}
