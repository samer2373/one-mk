//! What `one-mk status` shows, as the daemon reports it, and its wire format.

use one_mk_proto::{Rect, Side, os};

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
    /// This computer's monitors, in its logical pixels.
    pub screens: Vec<Rect>,
    /// (peer id, offset): where a peer's (0,0) sits in this computer's
    /// pixels, for the peers that have one. Kept here rather than on
    /// [`PeerStatus`] so its literals elsewhere stay valid.
    pub offsets: Vec<(String, (i32, i32))>,
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
    /// Its monitors, in its own logical pixels; empty while it is offline.
    pub screens: Vec<Rect>,
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
        // Monitors go on lines of their own, after the peers they belong
        // to: older readers skip lines they don't know.
        let screen = |who: &str, r: &Rect| format!("screen\t{who}\t{}\t{}\t{}\t{}\n", r.x, r.y, r.w, r.h);
        for r in &self.screens {
            out += &screen("me", r);
        }
        for p in &self.peers {
            for r in &p.screens {
                out += &screen(&p.id, r);
            }
        }
        for (id, (x, y)) in &self.offsets {
            out += &format!("offset\t{id}\t{x}\t{y}\n");
        }
        out
    }

    /// Where `peer`'s (0,0) sits in this computer's pixels, if it was placed.
    pub fn offset_of(&self, peer: &str) -> Option<(i32, i32)> {
        self.offsets.iter().find(|(id, _)| id == peer).map(|(_, o)| *o)
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
                ["peer", id, side, os, online, swap, name] => s.peers.push(PeerStatus {
                    id: id.into(),
                    name: name.into(),
                    side: side.parse().ok()?,
                    os: os.parse().ok()?,
                    online: online == "1",
                    swap: swap.into(),
                    screens: vec![],
                }),
                ["screen", who, x, y, w, h] => {
                    // A line this reader can't make sense of only costs a monitor.
                    let (Ok(x), Ok(y), Ok(w), Ok(h)) = (x.parse(), y.parse(), w.parse(), h.parse()) else { continue };
                    let screens = if who == "me" { Some(&mut s.screens) } else { s.peers.iter_mut().find(|p| p.id == who).map(|p| &mut p.screens) };
                    if let Some(screens) = screens {
                        screens.push(Rect::new(x, y, w, h));
                    }
                }
                ["offset", id, x, y] => {
                    if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                        s.offsets.push((id.into(), (x, y)));
                    }
                }
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
                PeerStatus { id: "1234-5678".into(), name: "mac mini".into(), side: Side::Right, os: os::MACOS, online: true, swap: "auto".into(), screens: vec![Rect::new(0, 0, 1920, 1080)] },
                PeerStatus { id: "9999-0000".into(), name: "old pc".into(), side: Side::Top, os: 0, online: false, swap: "off".into(), screens: vec![] },
            ],
            screens: vec![Rect::new(0, 0, 1920, 1080), Rect::new(-2592, 98, 1512, 982), Rect::new(-1080, -840, 1080, 1920), Rect::new(1920, 0, 1920, 1080)],
            offsets: vec![("1234-5678".into(), (3840, -98))],
        };
        assert_eq!(Status::decode(&s.encode()), Some(s.clone()));
        assert_eq!(s.offset_of("1234-5678"), Some((3840, -98)));
        assert_eq!(s.offset_of("9999-0000"), None);
        // A line this reader can't make sense of only costs that offset.
        let text = s.encode() + "offset\t9999-0000\tleft\t0\n";
        assert_eq!(Status::decode(&text).unwrap().offsets, s.offsets);
        assert_eq!(Status::decode("error unknown request\n"), None);
    }

    #[test]
    fn status_from_an_older_daemon_still_reads() {
        let old = "ok\nname\tdesk\nid\tabcd-ef01\nport\t24800\npeer\t1111-2222\tleft\t3\t1\tauto\tMacBook Air\n";
        let s = Status::decode(old).unwrap();
        assert!(s.screens.is_empty());
        assert_eq!(s.peers.len(), 1);
        assert!(s.peers[0].screens.is_empty());
        assert!(s.offsets.is_empty());
        assert_eq!(s.peers[0].name, "MacBook Air");
    }

    #[test]
    fn status_reads_plainly() {
        let s = Status {
            name: "desk".into(),
            id: "abcd-ef01".into(),
            port: 24900,
            controlling: Some("mac mini".into()),
            controlled_by: None,
            peers: vec![PeerStatus { id: "1".into(), name: "mac mini".into(), side: Side::Right, os: os::MACOS, online: true, swap: "auto".into(), screens: vec![] }],
            screens: vec![],
            offsets: vec![],
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
