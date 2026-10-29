//! The commands that talk to a running daemon or change its settings:
//! `status`, `takeback` and `layout`.
//!
//! Layout data lives in `devices.toml`, one record per paired computer:
//! `side` ([`Side`]), `align` ([`Align`], how its edge lines up with ours
//! along that side) and `layout_version` (when either was last chosen).
//! [`set_layout`] is the one way to change them here: it bumps the version,
//! so the change wins when the two computers compare layouts, and a running
//! daemon applies it on `Request::Reload` and sends the peer the mirror
//! image. Profiles (`profiles.rs`) go through it too.

use crate::control::{self, Request};
use anyhow::{Result, bail};
use one_mk_net::{Device, Swap, TrustStore};
use one_mk_proto::{Align, Side};
pub use one_mk_ctl::status::{LayoutStatus, PeerStatus, Place, Status, on_your, placement, takeback_hint};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unknown_device(device: &str) -> anyhow::Error {
    anyhow::anyhow!("no paired computer called {device:?}; see `one-mk devices` for names and ids")
}

pub fn parse_align(value: &str) -> Result<Align, String> {
    if value.trim().is_empty() {
        return Err("--offset cannot be empty; use center, stretch or a number from -100000 to 100000 logical pixels".into());
    }
    let align: Align = value.parse()?;
    if matches!(align, Align::Offset(offset) if !(-100_000..=100_000).contains(&offset)) {
        return Err("--offset must be between -100000 and 100000 logical pixels".into());
    }
    Ok(align)
}

pub async fn status(dir: &Path) -> Result<()> {
    let reply = control::request(dir, Request::Status).await?;
    match reply.as_deref().and_then(Status::decode) {
        Some(s) => {
            print!("{}", s.render());
            if let Some(line) = crate::settings::Settings::load(dir).ok().and_then(|s| s.input.summary()) {
                println!("{line}");
            }
        }
        None if reply.is_some() => bail!("unexpected answer from one-mk: {reply:?}"),
        None => {
            println!("one-mk is not running.");
            println!("Start it with `one-mk run`, or make it start when you log in with `one-mk install`.");
        }
    }
    let settings = crate::settings::Settings::load(dir)?;
    if settings.hooks.blocked.is_some() {
        println!("{}", settings.hooks.describe());
    }
    println!("Clipboard: {}.", settings.clipboard.summary());
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

/// Changes a paired device's side, alignment and/or Ctrl/Cmd setting in
/// `devices.toml`. A new side or alignment gets a version newer than any
/// seen, so it wins when the two computers compare layouts. A new side
/// without an alignment is stretched; the same side keeps its alignment.
pub fn set_layout(dir: &Path, device: &str, side: Option<Side>, align: Option<Align>, swap: Option<Swap>) -> Result<Device> {
    set_layout_at(dir, device, side, align, swap, now_ms())
}

/// [`set_layout`] with the clock (milliseconds since 1970) given, so a test
/// can make two computers choose under the same version.
pub fn set_layout_at(dir: &Path, device: &str, side: Option<Side>, align: Option<Align>, swap: Option<Swap>, now: u64) -> Result<Device> {
    if let Some(align) = align {
        parse_align(&align.to_string()).map_err(anyhow::Error::msg)?;
    }
    let mut trust = TrustStore::load(dir)?;
    let Some(found) = trust.find(device) else { return Err(unknown_device(device)) };
    let Some(fp) = found.fingerprint() else { bail!("corrupt fingerprint for {device:?} in devices.toml") };
    let dev = trust.get_mut(&fp).expect("found above");
    if side.is_some() || align.is_some() {
        // An offset chosen for one side means something else on another,
        // so a new side without one starts stretched.
        let new_side = side.unwrap_or_else(|| dev.side());
        let keep = if new_side == dev.side() { dev.align() } else { Align::Stretch };
        place_at(dev, new_side, align.unwrap_or(keep), now);
    }
    if let Some(swap) = swap {
        dev.swap_ctrl_cmd = swap;
    }
    let dev = dev.clone();
    trust.save()?;
    Ok(dev)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Puts `dev` on `side`, lined up as `align`, under a new layout version:
/// `now`, but always above the one it has. A layout received from the peer
/// is stored under its version, so a change made after receiving it is
/// above that too.
pub fn place(dev: &mut Device, side: Side, align: Align) {
    place_at(dev, side, align, now_ms());
}

/// [`place`] with the clock given.
pub fn place_at(dev: &mut Device, side: Side, align: Align, now: u64) {
    dev.side = side.to_string();
    dev.set_align(align);
    dev.layout_version = now.max(dev.layout_version.saturating_add(1));
}

pub async fn layout(dir: &Path, device: &str, side: Option<Side>, align: Option<Align>, swap: Option<Swap>) -> Result<()> {
    if side.is_none() && align.is_none() && swap.is_none() {
        bail!(
            "say where {device:?} is (left, right, top or bottom), how it lines up (--offset PX, center or stretch) \
             and/or use --swap-ctrl-cmd on|off|auto"
        );
    }
    let dev = set_layout(dir, device, side, align, swap)?;
    let name = one_mk_proto::sanitize_name(&dev.name);
    let moved = side.is_some() || align.is_some();
    if moved {
        println!("{name} is now {}.", placement(dev.side(), dev.align()));
        if dev.align().is_exact() {
            println!("The pointer crosses only where your edges overlap, like between two monitors.");
        }
    }
    match swap {
        Some(Swap::On) => println!("Ctrl and Cmd are swapped for keys sent to {name}."),
        Some(Swap::Off) => println!("Ctrl and Cmd are not swapped for keys sent to {name}."),
        Some(Swap::Auto) => println!("Ctrl and Cmd are swapped for keys sent to {name} only between a Mac and another computer."),
        None => {}
    }
    if control::request(dir, Request::Reload).await?.is_some() {
        println!("Applied right away.{}", if moved { " Its computer picks up the change too." } else { "" });
    } else {
        println!("one-mk is not running; this takes effect the next time it starts.");
    }
    Ok(())
}
