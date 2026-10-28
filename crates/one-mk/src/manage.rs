//! The commands that talk to a running daemon or change its settings:
//! `status`, `takeback` and `layout`.

use crate::control::{self, Request};
use anyhow::{Result, bail};
use one_mk_net::{Device, Swap, TrustStore};
use one_mk_proto::{Side, os};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unknown_device(device: &str) -> anyhow::Error {
    anyhow::anyhow!("no paired computer called {device:?}; see `one-mk devices` for names and ids")
}

/// Where a computer sits, for sentences like "Connected to B (on your right)."
pub fn on_your(side: Side) -> &'static str {
    match side {
        Side::Left => "on your left",
        Side::Right => "on your right",
        Side::Top => "above",
        Side::Bottom => "below",
    }
}

/// How the person at this computer takes control back from a peer. On
/// Linux the portals can't tell their own mouse and keyboard from injected
/// input, so only the command (or the idle rule) works there.
pub fn takeback_hint() -> &'static str {
    if cfg!(target_os = "linux") {
        "To take it back, run `one-mk takeback` here (it also comes back by itself after a minute without input from the other computer)."
    } else {
        "To take it back, use this computer's mouse or keyboard, or run `one-mk takeback`."
    }
}

pub fn os_name(code: u8) -> &'static str {
    match code {
        os::LINUX => "Linux",
        os::WINDOWS => "Windows",
        os::MACOS => "macOS",
        _ => "unknown OS",
    }
}

/// What `one-mk status` shows, as the daemon reports it.
#[derive(Debug, Default, PartialEq)]
pub struct Status {
    pub name: String,
    pub id: String,
    pub port: u16,
    /// The paired computer this one is controlling right now.
    pub controlling: Option<String>,
    /// The paired computer controlling this one right now.
    pub controlled_by: Option<String>,
    pub peers: Vec<PeerStatus>,
}

#[derive(Debug, PartialEq)]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub side: Side,
    pub os: u8,
    pub online: bool,
}

impl Status {
    /// One `key<TAB>value` line per fact (names never contain tabs or
    /// newlines: they are sanitized).
    pub fn encode(&self) -> String {
        let mut out = format!("ok\nname\t{}\nid\t{}\nport\t{}\n", self.name, self.id, self.port);
        for (key, who) in [("controlling", &self.controlling), ("controlled_by", &self.controlled_by)] {
            if let Some(w) = who {
                out += &format!("{key}\t{w}\n");
            }
        }
        for p in &self.peers {
            out += &format!("peer\t{}\t{}\t{}\t{}\t{}\n", p.id, p.side, p.os, u8::from(p.online), p.name);
        }
        out
    }

    pub fn decode(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != "ok" {
            return None;
        }
        let mut s = Status::default();
        for line in lines {
            let f: Vec<&str> = line.splitn(6, '\t').collect();
            match f[..] {
                ["name", v] => s.name = v.into(),
                ["id", v] => s.id = v.into(),
                ["port", v] => s.port = v.parse().ok()?,
                ["controlling", v] => s.controlling = Some(v.into()),
                ["controlled_by", v] => s.controlled_by = Some(v.into()),
                ["peer", id, side, os, online, name] => s.peers.push(PeerStatus {
                    id: id.into(),
                    name: name.into(),
                    side: side.parse().ok()?,
                    os: os.parse().ok()?,
                    online: online == "1",
                }),
                _ => {}
            }
        }
        Some(s)
    }

    pub fn render(&self) -> String {
        let mut out = format!("one-mk is running on {} (id {}).\n", self.name, self.id);
        out += &match (&self.controlling, &self.controlled_by) {
            (Some(p), _) => format!("The mouse and keyboard are controlling {p} right now.\n"),
            (_, Some(p)) => format!("{p} is controlling this computer right now. {}\n", takeback_hint()),
            _ => "The mouse and keyboard are on this computer.\n".to_owned(),
        };
        if self.peers.is_empty() {
            out += "No computers are paired yet. Run `one-mk pair` on this and the other computer.\n";
        }
        for p in &self.peers {
            let state = if p.online { "online" } else { "offline" };
            out += &format!("  {:<24} {:<10} {:<14} {state}\n", p.name, os_name(p.os), on_your(p.side));
        }
        out
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_survives_the_wire() {
        let s = Status {
            name: "desk".into(),
            id: "abcd-ef01".into(),
            port: 24900,
            controlling: Some("mac mini".into()),
            controlled_by: None,
            peers: vec![
                PeerStatus { id: "1234-5678".into(), name: "mac mini".into(), side: Side::Right, os: os::MACOS, online: true },
                PeerStatus { id: "9999-0000".into(), name: "old pc".into(), side: Side::Top, os: 0, online: false },
            ],
        };
        assert_eq!(Status::decode(&s.encode()), Some(s));
        assert_eq!(Status::decode("error unknown request\n"), None);
    }

    #[test]
    fn status_reads_plainly() {
        let s = Status {
            name: "desk".into(),
            id: "abcd-ef01".into(),
            port: 24900,
            controlling: Some("mac mini".into()),
            controlled_by: None,
            peers: vec![PeerStatus { id: "1".into(), name: "mac mini".into(), side: Side::Right, os: os::MACOS, online: true }],
        };
        let text = s.render();
        assert!(text.starts_with("one-mk is running on desk (id abcd-ef01).\n"), "{text}");
        assert!(text.contains("controlling mac mini right now"));
        assert!(text.contains("mac mini") && text.contains("macOS") && text.contains("on your right") && text.contains("online"));

        let controlled = Status { controlling: None, controlled_by: Some("laptop".into()), ..s };
        let text = controlled.render();
        assert!(text.contains("laptop is controlling this computer right now."), "{text}");
        assert!(text.contains("one-mk takeback"), "{text}");
    }
}
