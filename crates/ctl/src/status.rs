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

/// Where a computer sits and, if it was placed, where its (0,0) is in
/// ours, for sentences like "B is now below, its (0,0) at 300,1080 of
/// yours."
pub fn placement(side: Side, offset: Option<(i32, i32)>) -> String {
    match offset {
        None => on_your(side).to_owned(),
        Some((x, y)) => format!("{}, its (0,0) at {x},{y} of yours", on_your(side)),
    }
}

/// "Is B on your left?" / "Is B above this computer?": the question layout
/// learning asks.
pub fn is_it_there(name: &str, side: Side) -> String {
    let at = match side {
        Side::Left | Side::Right => on_your(side),
        Side::Top => "above this computer",
        Side::Bottom => "below this computer",
    };
    format!("Is {name} {at}?")
}

/// How to name a computer on the command line: its name where that is
/// safe to type as it is (quoted if it has spaces), else its id.
pub fn device_arg(name: &str, id: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    if !name.is_empty() && name.chars().all(plain) {
        name.to_owned()
    } else if !name.trim().is_empty() && name.chars().all(|c| plain(c) || c == ' ') && name.trim() == name {
        format!("\"{name}\"")
    } else {
        id.to_owned()
    }
}

/// A computer layout learning thinks is on `side` (see `docs/EDGE-PUSH.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub id: String,
    pub name: String,
    pub side: Side,
    /// How to name it in `one-mk layout` (see [`device_arg`]); the name is
    /// not always unique.
    pub arg: String,
}

impl Suggestion {
    /// The command that places it there.
    pub fn command(&self) -> String {
        format!("one-mk layout {} {}", self.arg, self.side)
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
    pub backend: InputBackend,
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
    /// Keyboard shortcuts: `on`, `off`, `starting` or `unavailable` (empty
    /// from older daemons), and the details in words.
    pub hotkeys: String,
    pub hotkeys_detail: String,
    /// What the input backend needs the user to know or do, one line each
    /// (e.g. approve the desktop's permission dialog); empty when it works.
    pub input: Vec<String>,
    /// one-mk draws the pointer here right now: a peer controls this
    /// computer and it shows none of its own (no mouse attached).
    pub pointer_overlay: bool,
    /// Input tuning that isn't in force and why, one line each (rules for
    /// an app on a computer that doesn't say which app has its focus).
    pub tuning: Vec<String>,
    /// Where the computers are, in detail (boxed: most readers never look,
    /// and it would double the size of every `Status`).
    pub layout: Box<LayoutStatus>,
    /// This computer's monitors, in its logical pixels.
    pub screens: Vec<Rect>,
    /// (peer id, offset): where a peer's (0,0) sits in this computer's
    /// pixels, for the peers that have one. Kept here rather than on
    /// [`PeerStatus`] so its literals elsewhere stay valid.
    pub offsets: Vec<(String, (i32, i32))>,
    /// A computer that seems to be somewhere else than where it is placed:
    /// the pointer was pushed twice against an edge where no computer sits.
    /// Shown for a few minutes.
    pub suggestion: Option<Suggestion>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum InputBackend {
    #[default]
    Unknown,
    Null,
    Portal,
    Native,
    X11,
    Wlroots,
}

impl InputBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "",
            Self::Null => "null",
            Self::Portal => "portal",
            Self::Native => "native",
            Self::X11 => "x11",
            Self::Wlroots => "wlroots",
        }
    }
}

impl From<&str> for InputBackend {
    fn from(value: &str) -> Self {
        match value {
            "null" => Self::Null,
            "portal" => Self::Portal,
            "native" => Self::Native,
            "x11" => Self::X11,
            "wlroots" => Self::Wlroots,
            _ => Self::Unknown,
        }
    }
}

/// About the layout, beyond where the computers are (each peer's side
/// and monitors, [`Status::screens`] and [`Status::offsets`]). Empty from
/// daemons that predate it.
///
/// To change it: `one-mk layout <id> <side> --offset DX,DY|none` (which
/// applies at once and syncs the mirror image to the peer), and
/// `one-mk profile save|use|list|delete|auto`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutStatus {
    /// The layout profile last put in place (`one-mk profile use`, or
    /// switched to automatically), if any.
    pub profile: Option<String>,
    /// Every saved layout profile, by name.
    pub profiles: Vec<String>,
    /// Problems with the layout, one line each (two computers claiming the
    /// same stretch of edge, one whose edge doesn't face ours anywhere).
    pub warnings: Vec<String>,
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

/// 125 -> "2 min 5 s".
pub fn human_duration(secs: u64) -> String {
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
    /// Its monitors, in its own logical pixels; empty while it is offline.
    pub screens: Vec<Rect>,
}

/// What to do where the desktop has no global shortcuts.
pub const HOTKEYS_FALLBACK: &str = "bind `one-mk switch next` (and `one-mk switch here`) to a key in your desktop settings";

impl Status {
    /// The keyboard shortcuts, in one line; `None` if there is nothing to say.
    pub fn hotkeys_line(&self) -> Option<String> {
        let d = &self.hotkeys_detail;
        Some(match self.hotkeys.as_str() {
            "on" => format!("Keyboard shortcuts work {d} (`one-mk hotkeys` lists them)."),
            "starting" => format!("Keyboard shortcuts are being set up: {d}."),
            "unavailable" => format!(
                "Keyboard shortcuts work only while the pointer is on another computer ({d}); here, {HOTKEYS_FALLBACK}."
            ),
            _ if !d.is_empty() => format!("Keyboard shortcuts are off: {d}."),
            _ => return None,
        })
    }

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
        // Lines of their own, which readers from before them skip.
        let l = &self.layout;
        if let Some(p) = &l.profile {
            out += &format!("profile\t{p}\n");
        }
        if !l.profiles.is_empty() {
            out += &format!("profiles\t{}\n", l.profiles.join("\t"));
        }
        for (key, on) in [
            ("pointer_locked", self.pointer_locked),
            ("screen_locked", self.screen_locked),
            ("relative", self.relative),
            ("lock_sync", self.lock_sync),
            ("pointer_overlay", self.pointer_overlay),
        ] {
            if on {
                out += &format!("{key}\t1\n");
            }
        }
        for (key, v) in [("gaming", self.gaming.as_str()), ("edges", self.edges.as_str()), ("backend", self.backend.as_str())] {
            if !v.is_empty() {
                out += &format!("{key}\t{v}\n");
            }
        }
        if !self.hotkeys.is_empty() {
            out += &format!("hotkeys\t{}\t{}\n", self.hotkeys, self.hotkeys_detail);
        }
        for line in &self.input {
            out += &format!("input\t{}\n", line.replace(['\t', '\n', '\r'], " "));
        }
        for line in &self.layout.warnings {
            out += &format!("warning\t{}\n", line.replace(['\t', '\n', '\r'], " "));
        }
        for line in &self.tuning {
            out += &format!("tuning\t{}\n", line.replace(['\t', '\n', '\r'], " "));
        }
        if let Some(g) = &self.suggestion {
            let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
            out += &format!("suggest\t{}\t{}\t{}\t{}\n", clean(&g.id), g.side, clean(&g.arg), clean(&g.name));
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
            if let Some(names) = line.strip_prefix("profiles\t") {
                s.layout.profiles = names.split('\t').map(str::to_owned).collect();
                continue;
            }
            let f: Vec<&str> = line.splitn(8, '\t').collect();
            match f[..] {
                ["name", v] => s.name = v.into(),
                ["id", v] => s.id = v.into(),
                ["port", v] => s.port = v.parse().ok()?,
                ["backend", value] => s.backend = value.into(),
                ["controlling", v] => s.controlling = Some(v.into()),
                ["controlled_by", v] => s.controlled_by = Some(v.into()),
                ["pointer_locked", v] => s.pointer_locked = v == "1",
                ["screen_locked", v] => s.screen_locked = v == "1",
                ["relative", v] => s.relative = v == "1",
                ["lock_sync", v] => s.lock_sync = v == "1",
                ["pointer_overlay", v] => s.pointer_overlay = v == "1",
                ["gaming", v] => s.gaming = v.into(),
                ["edges", v] => s.edges = v.into(),
                ["hotkeys", state, detail] => (s.hotkeys, s.hotkeys_detail) = (state.into(), detail.into()),
                ["profile", v] => s.layout.profile = Some(v.into()),
                ["input", v] => s.input.push(v.into()),
                ["warning", v] => s.layout.warnings.push(v.into()),
                ["tuning", v] => s.tuning.push(v.into()),
                ["suggest", id, side, arg, name] => {
                    if let Ok(side) = side.parse() {
                        s.suggestion = Some(Suggestion { id: id.into(), name: name.into(), side, arg: arg.into() });
                    }
                }
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
        if !self.backend.as_str().is_empty() {
            out += &format!("Input backend: {}.\n", self.backend.as_str());
        }
        out += &match (&self.controlling, &self.controlled_by) {
            (Some(p), _) => format!("The mouse and keyboard are controlling {p} right now.\n"),
            (_, Some(p)) => format!("{p} is controlling this computer right now. {}\n", takeback_hint()),
            _ => "The mouse and keyboard are on this computer.\n".to_owned(),
        };
        for line in self.input.iter().chain(&self.layout.warnings).chain(&self.tuning) {
            out += &format!("{line}\n");
        }
        if let Some(g) = &self.suggestion {
            out += &format!(
                "{} You pushed the pointer against that edge twice. To place it there, run `{}` (`one-mk edge --dismiss` hides this).\n",
                is_it_there(&g.name, g.side),
                g.command()
            );
        }
        if self.pointer_overlay {
            out += "one-mk draws the pointer here: this computer shows none of its own (`one-mk pointer-overlay` to change).\n";
        }
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
        if let Some(line) = self.hotkeys_line() {
            out += &format!("{line}\n");
        }
        if let Some(p) = &self.layout.profile {
            out += &format!("Layout profile: {p}.\n");
        }
        if self.peers.is_empty() {
            out += "No computers are paired yet. Run `one-mk pair` on this and the other computer.\n";
        }
        for p in &self.peers {
            let state = if p.online { "online" } else { "offline" };
            out += &format!("  {:<24} {:<10} {:<14} {state}\n", p.name, os_name(p.os), on_your(p.side));
            if let Some(offset) = self.offset_of(&p.id) {
                out += &format!("  {:<24} {}\n", "", placement(p.side, Some(offset)));
            }
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
    fn suggestions_name_the_computer_safely() {
        assert_eq!(device_arg("laptop", "ab12-cd34"), "laptop");
        assert_eq!(device_arg("Sam's Mac", "ab12-cd34"), "ab12-cd34", "a quote would need escaping");
        assert_eq!(device_arg("studio pc", "ab12-cd34"), "\"studio pc\"");
        assert_eq!(device_arg(" pc", "ab12-cd34"), "ab12-cd34");
        assert_eq!(device_arg("", "ab12-cd34"), "ab12-cd34");
        for hostile in ["$(rm -rf ~)", "a`b`", "a;b", "x\ny", "名前", "a|b", "%PATH%"] {
            assert_eq!(device_arg(hostile, "ab12-cd34"), "ab12-cd34", "{hostile}");
        }
        assert_eq!(is_it_there("B", Side::Top), "Is B above this computer?");
        assert_eq!(is_it_there("B", Side::Right), "Is B on your right?");
    }

    #[test]
    fn every_backend_roundtrips() {
        for name in ["", "null", "portal", "native", "x11", "wlroots"] {
            let backend = InputBackend::from(name);
            assert_eq!(backend.as_str(), name);
            let status = Status { backend, ..Status::default() };
            assert_eq!(Status::decode(&status.encode()).unwrap().backend, backend);
        }
    }

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
            ..Status::default()
        };
        assert_eq!(Status::decode(&s.encode()), Some(s.clone()));
        assert_eq!(s.offset_of("1234-5678"), Some((3840, -98)));
        assert_eq!(s.offset_of("9999-0000"), None);
        // A line this reader can't make sense of only costs that offset.
        let text = s.encode() + "offset\t9999-0000\tleft\t0\n";
        assert_eq!(Status::decode(&text).unwrap().offsets, s.offsets);
        let s = Status {
            pointer_locked: true,
            screen_locked: true,
            gaming: "auto".into(),
            relative: true,
            lock_sync: true,
            edges: "dead corners 20 px, dwell 300 ms".into(),
            input: vec!["Waiting for your approval: allow one-mk in the desktop's dialog.".into()],
            tuning: vec!["Rule for app code not active: mac mini does not report its focused app (why).".into()],
            backend: InputBackend::Null,
            pointer_overlay: true,
            ..s
        };
        assert_eq!(Status::decode(&s.encode()), Some(s.clone()));
        let text = s.render();
        assert!(!Status::default().render().contains("draws the pointer"));
        for want in ["Input backend: null", "one-mk draws the pointer here", "locked to this computer", "screen is locked", "Edges: dead corners 20 px", "Gaming mode: auto (relative", "Lock sync is on", "Waiting for your approval", "Rule for app code not active"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        assert_eq!(Status::decode("error unknown request\n"), None);
        // A line can't break the format.
        let odd = Status { input: vec!["a\tb\nc".into()], ..Status::default() };
        assert_eq!(Status::decode(&odd.encode()).unwrap().input, ["a b c"]);
        let suggestion = Suggestion { id: "1234-5678".into(), name: "mac mini".into(), side: Side::Left, arg: "\"mac mini\"".into() };
        let suggested = Status { suggestion: Some(suggestion), ..Status::default() };
        assert_eq!(Status::decode(&suggested.encode()), Some(suggested.clone()));
        let text = suggested.render();
        assert!(text.contains("Is mac mini on your left? You pushed the pointer against that edge twice. To place it there, run `one-mk layout \"mac mini\" left`"), "{text}");
        // A side this reader doesn't know only costs the suggestion.
        let odd = suggested.encode().replace("suggest\t1234-5678\tleft", "suggest\t1234-5678\tnorth");
        assert_eq!(Status::decode(&odd).unwrap().suggestion, None);
        let layout = LayoutStatus { warnings: vec!["layout of B does not overlap yours".into()], ..LayoutStatus::default() };
        let warned = Status { layout: Box::new(layout), ..Status::default() };
        assert_eq!(Status::decode(&warned.encode()).unwrap(), warned);
        assert!(warned.render().contains("does not overlap"));
    }

    /// What a layout editor reads: our monitors, each peer's monitors and
    /// offset, and the profiles.
    #[test]
    fn layout_survives_the_wire_and_old_readers_skip_it() {
        let laptop = PeerStatus { id: "1234-5678".into(), name: "laptop".into(), side: Side::Bottom, os: os::LINUX, online: true, swap: "auto".into(), screens: vec![Rect::new(0, 0, 1280, 800)] };
        let old = PeerStatus { id: "9999-0000".into(), name: "old pc".into(), online: false, screens: vec![], ..laptop.clone() };
        let s = Status {
            name: "desk".into(),
            peers: vec![laptop, old],
            screens: vec![Rect::new(0, 0, 2560, 1440), Rect::new(-1920, 200, 1920, 1080)],
            offsets: vec![("1234-5678".into(), (300, 1440))],
            layout: Box::new(LayoutStatus {
                profile: Some("home office".into()),
                profiles: vec!["home office".into(), "café".into()],
                warnings: vec![],
            }),
            ..Status::default()
        };
        let encoded = s.encode();
        assert_eq!(Status::decode(&encoded), Some(s.clone()));
        let text = s.render();
        assert!(text.contains("below, its (0,0) at 300,1440 of yours"), "{text}");
        assert!(text.contains("Layout profile: home office."), "{text}");
        // What a reader from before these lines makes of it: the rest.
        let before: Vec<&str> = encoded.lines().filter(|l| l.starts_with("ok") || l.starts_with("name") || l.starts_with("peer\t")).collect();
        let older = Status::decode(&(before.join("\n") + "\n")).unwrap();
        assert_eq!((older.peers.len(), older.offsets.len(), older.layout.profile), (2, 0, None));
        assert_eq!(placement(Side::Top, None), "above");
        assert_eq!(placement(Side::Left, Some((-1920, 40))), "on your left, its (0,0) at -1920,40 of yours");
    }

    #[test]
    fn shortcut_state_survives_the_wire_and_reads_plainly() {
        let s = Status { hotkeys: "unavailable".into(), hotkeys_detail: "this desktop has no GlobalShortcuts portal".into(), ..Status::default() };
        assert_eq!(Status::decode(&s.encode()), Some(s.clone()));
        let line = s.hotkeys_line().unwrap();
        assert!(line.contains("only while the pointer is on another computer") && line.contains("`one-mk switch next`"), "{line}");
        assert!(s.render().contains(&line));
        let on = Status { hotkeys: "on".into(), hotkeys_detail: "through Windows".into(), ..Status::default() };
        assert_eq!(on.hotkeys_line().unwrap(), "Keyboard shortcuts work through Windows (`one-mk hotkeys` lists them).");
        assert_eq!(Status::decode(&on.encode()), Some(on));
        // Older daemons say nothing; off without a reason says nothing either.
        assert_eq!(Status::default().hotkeys_line(), None);
        assert_eq!(Status { hotkeys: "off".into(), ..Status::default() }.hotkeys_line(), None);
        assert!(!Status::default().encode().contains("hotkeys"));
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
        // What the daemon encodes is what `one-mk status` decodes (8 tab-separated fields).
        let encoded = s.encode();
        assert_eq!(Status::decode(&encoded).unwrap().transfers, s.transfers);
        // A reader from before transfers existed ignores lines it doesn't know.
        let old: Vec<&str> = encoded.lines().filter(|l| !l.starts_with("transfer")).collect();
        assert!(Status::decode(&(old.join("\n") + "\n")).unwrap().transfers.is_empty());
        // A damaged transfer line is skipped; the rest of the status still reads.
        let damaged = Status::decode("ok\nname\tdesk\ntransfer\tx\ts\t1\t1\t1\t1\tpeer\n").unwrap();
        assert_eq!((damaged.name.as_str(), damaged.transfers.len()), ("desk", 0));
    }
}

/// Returned only by Request::Invitation, never by ordinary status/logs.
/// This is a bearer credential; deliberately has no Debug implementation.
pub struct InvitationStatus {
    pub link: String,
    pub expires: u64,
}
impl Drop for InvitationStatus {
    fn drop(&mut self) { zeroize::Zeroize::zeroize(&mut self.link); }
}
impl InvitationStatus {
    pub fn encode(&self) -> String { format!("invitation\t{}\t{}\n", self.expires, self.link) }
    pub fn decode(text: &str) -> Option<Self> {
        let mut fields = text.trim_end_matches('\n').split('\t');
        if fields.next()? != "invitation" { return None; }
        let expires = fields.next()?.parse().ok()?;
        let link = fields.next()?;
        if fields.next().is_some() || link.len() > 1600 || !link.starts_with("onemk://pair?") || link.chars().any(char::is_whitespace) { return None; }
        Some(Self { link: link.to_owned(), expires })
    }
}
#[cfg(test)]
mod invitation_tests {
    use super::*;
    #[test]
    fn invitation_status_roundtrip() {
        let s = InvitationStatus { link: "onemk://pair?v=1&d=test".into(), expires: 123 };
        let got = InvitationStatus::decode(&s.encode()).unwrap();
        assert_eq!(got.link, s.link);
        assert_eq!(got.expires, s.expires);
        for bad in ["none\n", "invitation\tx\tonemk://pair?x", "invitation\t1\thttps://bad", "invitation\t1\tonemk://pair?x\textra"] { assert!(InvitationStatus::decode(bad).is_none()); }
    }
}
