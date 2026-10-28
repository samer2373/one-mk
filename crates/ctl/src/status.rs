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
    /// File transfers in flight, to and from this computer.
    pub transfers: Vec<TransferStatus>,
}

/// One file transfer in flight (a whole `one-mk send`, not a single file).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferStatus {
    /// Tells transfers apart; 0 if the sender is an older version.
    pub id: u64,
    /// True when this computer sends, false when it receives.
    pub sending: bool,
    /// The other computer.
    pub peer: String,
    /// Files in the transfer, as far as known.
    pub files: u32,
    pub done: u64,
    /// Bytes in the transfer, as far as known (can grow while files are still being offered).
    pub total: u64,
    /// Recent speed in bytes per second; 0 when stalled.
    pub rate: u64,
}

impl TransferStatus {
    /// 0 to 100, or `None` while the total is not known.
    pub fn percent(&self) -> Option<u8> {
        (self.total > 0).then(|| (u128::from(self.done.min(self.total)) * 100 / u128::from(self.total)) as u8)
    }

    /// "Receiving 3 files from desk: 45% (12.3 MB of 27.1 MB), 8.1 MB/s"
    pub fn render(&self) -> String {
        let files = if self.files == 1 { "1 file".to_owned() } else { format!("{} files", self.files) };
        let (verb, prep) = if self.sending { ("Sending", "to") } else { ("Receiving", "from") };
        let what = if self.files == 0 { "files".to_owned() } else { files };
        let mut out = format!("{verb} {what} {prep} {}: ", self.peer);
        match self.percent() {
            Some(p) => out += &format!("{p}% ({} of {})", human_bytes(self.done), human_bytes(self.total)),
            None => out += &human_bytes(self.done),
        }
        if self.rate > 0 {
            out += &format!(", {}/s", human_bytes(self.rate));
            if self.total > self.done {
                out += &format!(", {} left", human_duration((self.total - self.done) / self.rate));
            }
        } else {
            out += ", stalled";
        }
        out
    }
}

/// 1234567 -> "1.2 MB" (decimal units, like the rest of what one-mk prints).
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

fn human_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
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
        for t in &self.transfers {
            out += &format!(
                "transfer\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                t.id,
                if t.sending { 's' } else { 'r' },
                t.files,
                t.done,
                t.total,
                t.rate,
                t.peer
            );
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
                // Cosmetic: a line that doesn't parse is skipped, not fatal.
                ["transfer", id, dir, files, done, total, rate, peer] => s.transfers.extend(|| -> Option<_> {
                    Some(TransferStatus {
                        id: id.parse().ok()?,
                        sending: dir == "s",
                        peer: peer.into(),
                        files: files.parse().ok()?,
                        done: done.parse().ok()?,
                        total: total.parse().ok()?,
                        rate: rate.parse().ok()?,
                    })
                }()),
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
        for t in &self.transfers {
            out += &format!("{}\n", t.render());
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

    #[test]
    fn transfers_read_plainly() {
        let t = TransferStatus { id: 1, sending: false, peer: "desk".into(), files: 3, done: 12_300_000, total: 27_100_000, rate: 8_100_000 };
        assert_eq!(t.render(), "Receiving 3 files from desk: 45% (12.3 MB of 27.1 MB), 8.1 MB/s, 1 s left");
        let t = TransferStatus { sending: true, files: 1, done: 0, total: 1000, rate: 0, ..t };
        assert_eq!(t.render(), "Sending 1 file to desk: 0% (0 B of 1.0 KB), stalled");
        // Total not known yet (an older sender): no percentage, no guess.
        let t = TransferStatus { files: 0, total: 0, done: 2_000_000, rate: 1_000_000, ..t };
        assert_eq!(t.render(), "Sending files to desk: 2.0 MB, 1.0 MB/s");
        assert_eq!(t.percent(), None);
        // More done than total (a total that grows as offers arrive) never exceeds 100%.
        assert_eq!(TransferStatus { done: 9, total: 3, ..t }.percent(), Some(100));
        assert_eq!(human_duration(125), "2 min 5 s");
        assert_eq!(human_duration(7300), "2 h 1 min");
    }

    #[test]
    fn status_shows_transfers_and_old_readers_skip_them() {
        let s = Status {
            name: "desk".into(),
            transfers: vec![TransferStatus { id: 2, sending: true, peer: "mac mini".into(), files: 2, done: 1, total: 4, rate: 1 }],
            ..Default::default()
        };
        let text = s.render();
        assert!(text.contains("Sending 2 files to mac mini: 25%"), "{text}");
        // A reader from before transfers existed ignores lines it doesn't know.
        let encoded = s.encode();
        let old: Vec<&str> = encoded.lines().filter(|l| !l.starts_with("transfer")).collect();
        assert!(Status::decode(&(old.join("\n") + "\n")).unwrap().transfers.is_empty());
        // A damaged transfer line is skipped; the rest of the status still reads.
        let damaged = Status::decode("ok\nname\tdesk\ntransfer\tx\ts\t1\t1\t1\t1\tpeer\n").unwrap();
        assert_eq!((damaged.name.as_str(), damaged.transfers.len()), ("desk", 0));
    }
}
