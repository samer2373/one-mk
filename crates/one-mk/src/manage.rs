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

/// `DX,DY` or `none`, for `one-mk layout --offset`.
pub fn parse_offset(s: &str) -> Result<Option<(i32, i32)>> {
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let parsed = s.split_once(',').and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)));
    parsed.map(Some).ok_or_else(|| anyhow::anyhow!("expected --offset DX,DY (e.g. -1920,0) or none, not {s:?}"))
}

/// Changes a paired device's side, offset and/or Ctrl/Cmd setting in
/// `devices.toml`. A new side or offset gets a version newer than any
/// seen, so it wins when the two computers compare layouts; a new side
/// also drops an offset that was meant for the old side, unless a new
/// one is given.
pub fn set_layout(
    dir: &Path,
    device: &str,
    side: Option<Side>,
    offset: Option<Option<(i32, i32)>>,
    swap: Option<Swap>,
) -> Result<Device> {
    let mut trust = TrustStore::load(dir)?;
    let Some(found) = trust.find(device) else { return Err(unknown_device(device)) };
    let Some(fp) = found.fingerprint() else { bail!("corrupt fingerprint for {device:?} in devices.toml") };
    let dev = trust.get_mut(&fp).expect("found above");
    if side.is_some() || offset.is_some() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        dev.layout_version = now.max(dev.layout_version + 1);
    }
    if let Some(side) = side {
        if dev.side() != side {
            dev.offset = None;
        }
        dev.side = side.to_string();
    }
    if let Some(offset) = offset {
        dev.offset = offset;
    }
    if let Some(swap) = swap {
        dev.swap_ctrl_cmd = swap;
    }
    let dev = dev.clone();
    trust.save()?;
    Ok(dev)
}

pub async fn layout(dir: &Path, device: &str, side: Option<Side>, offset: Option<&str>, swap: Option<Swap>) -> Result<()> {
    if side.is_none() && offset.is_none() && swap.is_none() {
        bail!("say where {device:?} is (left, right, top or bottom), and/or use --offset DX,DY|none or --swap-ctrl-cmd on|off|auto");
    }
    let offset = offset.map(parse_offset).transpose()?;
    let dev = set_layout(dir, device, side, offset, swap)?;
    let name = one_mk_proto::sanitize_name(&dev.name);
    if let Some(side) = side {
        println!("{name} is now {}.", on_your(side));
    }
    match offset {
        Some(Some((x, y))) => println!("{name}'s (0,0) sits at {x},{y} of yours; the pointer crosses where the screens touch."),
        Some(None) => println!("{name} has no offset; the pointer crosses anywhere along its side."),
        None => {}
    }
    match swap {
        Some(Swap::On) => println!("Ctrl and Cmd are swapped for keys sent to {name}."),
        Some(Swap::Off) => println!("Ctrl and Cmd are not swapped for keys sent to {name}."),
        Some(Swap::Auto) => println!("Ctrl and Cmd are swapped for keys sent to {name} only between a Mac and another computer."),
        None => {}
    }
    if control::request(dir, Request::Reload).await?.is_some() {
        println!("Applied right away.{}", if side.is_some() || offset.is_some() { " Its computer picks up the change too." } else { "" });
    } else {
        println!("one-mk is not running; this takes effect the next time it starts.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_offset() {
        assert_eq!(super::parse_offset("-2592, 98").unwrap(), Some((-2592, 98)));
        assert_eq!(super::parse_offset("None").unwrap(), None);
        assert!(super::parse_offset("10").is_err());
    }
}
