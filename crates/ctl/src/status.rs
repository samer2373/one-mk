//! What `one-mk status` shows, as the daemon reports it, and its wire format.

use one_mk_proto::{Side, os};

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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub name: String,
    pub id: String,
    pub port: u16,
    /// The paired computer this one is controlling right now.
    pub controlling: Option<String>,
    /// The paired computer controlling this one right now.
    pub controlled_by: Option<String>,
    pub peers: Vec<PeerStatus>,
    /// `one-mk lock-pointer` is on: edges lead nowhere.
    pub pointer_locked: bool,
    /// This computer's screen is locked (as far as the OS tells).
    pub screen_locked: bool,
    /// Gaming mode: `off`, `on` or `auto` (empty from older daemons).
    pub gaming: String,
    /// Relative motion is in use right now (gaming mode).
    pub relative: bool,
    /// Locking one computer locks the others.
    pub lock_sync: bool,
    /// The edge rules in force, in words; empty for none.
    pub edges: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub side: Side,
    pub os: u8,
    pub online: bool,
    /// Ctrl/Cmd swapping for keys sent to it: `auto`, `on` or `off`.
    pub swap: String,
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
            out += &format!("peer\t{}\t{}\t{}\t{}\t{}\t{}\n", p.id, p.side, p.os, u8::from(p.online), p.swap, p.name);
        }
        for (key, on) in [
            ("pointer_locked", self.pointer_locked),
            ("screen_locked", self.screen_locked),
            ("relative", self.relative),
            ("lock_sync", self.lock_sync),
        ] {
            if on {
                out += &format!("{key}\t1\n");
            }
        }
        for (key, v) in [("gaming", &self.gaming), ("edges", &self.edges)] {
            if !v.is_empty() {
                out += &format!("{key}\t{v}\n");
            }
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
            let f: Vec<&str> = line.splitn(7, '\t').collect();
            match f[..] {
                ["name", v] => s.name = v.into(),
                ["id", v] => s.id = v.into(),
                ["port", v] => s.port = v.parse().ok()?,
                ["controlling", v] => s.controlling = Some(v.into()),
                ["controlled_by", v] => s.controlled_by = Some(v.into()),
                ["pointer_locked", v] => s.pointer_locked = v == "1",
                ["screen_locked", v] => s.screen_locked = v == "1",
                ["relative", v] => s.relative = v == "1",
                ["lock_sync", v] => s.lock_sync = v == "1",
                ["gaming", v] => s.gaming = v.into(),
                ["edges", v] => s.edges = v.into(),
                ["peer", id, side, os, online, swap, name] => s.peers.push(PeerStatus {
                    id: id.into(),
                    name: name.into(),
                    side: side.parse().ok()?,
                    os: os.parse().ok()?,
                    online: online == "1",
                    swap: swap.into(),
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
        if self.pointer_locked {
            out += "The pointer is locked to this computer: edges lead nowhere (`one-mk lock-pointer off`).\n";
        }
        if self.screen_locked {
            out += "The screen is locked.\n";
        }
        if !self.edges.is_empty() {
            out += &format!("Edges: {}.\n", self.edges);
        }
        if !self.gaming.is_empty() && self.gaming != "off" {
            let now = if self.relative { " (relative motion now)" } else { "" };
            out += &format!("Gaming mode: {}{now}.\n", self.gaming);
        }
        if self.lock_sync {
            out += "Lock sync is on: locking any computer locks the others.\n";
        }
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
                PeerStatus { id: "1234-5678".into(), name: "mac mini".into(), side: Side::Right, os: os::MACOS, online: true, swap: "auto".into() },
                PeerStatus { id: "9999-0000".into(), name: "old pc".into(), side: Side::Top, os: 0, online: false, swap: "off".into() },
            ],
            ..Status::default()
        };
        assert_eq!(Status::decode(&s.encode()), Some(s.clone()));
        let s = Status {
            pointer_locked: true,
            screen_locked: true,
            gaming: "auto".into(),
            relative: true,
            lock_sync: true,
            edges: "dead corners 20 px, dwell 300 ms".into(),
            ..s
        };
        assert_eq!(Status::decode(&s.encode()), Some(s.clone()));
        let text = s.render();
        for want in ["locked to this computer", "screen is locked", "Edges: dead corners 20 px", "Gaming mode: auto (relative", "Lock sync is on"] {
            assert!(text.contains(want), "{want}: {text}");
        }
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
            peers: vec![PeerStatus { id: "1".into(), name: "mac mini".into(), side: Side::Right, os: os::MACOS, online: true, swap: "auto".into() }],
            ..Status::default()
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
