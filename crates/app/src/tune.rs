//! The Keys & mouse tab, worked out without drawing anything: the
//! `[input]` rules as `settings.toml` has them, which of them apply to the
//! computers chosen, rules as sentences, the drawn keyboard and mouse, the
//! quick setups, the `one-mk keymap|pointer|scroll|buttons` commands that
//! make changes, the checks `one-mk keymap add` makes, keys as egui reports
//! them, and the "Test it" box. Names go through `one_mk_proto::keynames`
//! and `remap`, so the window, the command line and `settings.toml` agree.
//! Plain data in, plain data out: tested without a window.

use crate::view::{Entry, Val};
use one_mk_proto::device::Fingerprint;
use one_mk_proto::keynames::{self, Chord, Trigger};
use one_mk_proto::os;
use one_mk_proto::remap::{self, Action, Emit, Keymap, Mapper, Nav, Output, RuleSpec, SystemAction, What};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

// ---------- scopes ----------

/// Which computers a rule is for (`scope` in `settings.toml`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    /// Computers running this OS (one of `os`).
    Os(u8),
    /// One paired computer, by name (or id).
    Peer(String),
    /// One keyboard or mouse (its fingerprint, as `one-mk input-devices`
    /// lists it) on the computers the inner scope is for.
    Device(String, Box<Scope>),
}

/// A keyboard or mouse as people know it: its name, from its fingerprint.
pub fn device_name(fp: &str) -> String {
    match fp.parse::<Fingerprint>() {
        Ok(f) if !f.name.is_empty() => f.name,
        Ok(f) => format!("{} {}", f.bus.words(), f.ids()),
        Err(_) => fp.to_owned(),
    }
}

/// The keyboards and mice this computer has seen (`one-mk input-devices
/// --json`), and whether it can tell them apart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputDevices {
    pub available: bool,
    /// Why not, where they can't be told apart; what is untested where they can.
    pub note: Option<String>,
    pub devices: Vec<SeenDevice>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeenDevice {
    pub fingerprint: String,
    pub name: String,
    /// `usb`, `bluetooth`, `receiver`, `internal`, `virtual` or `unknown`.
    pub bus: String,
    /// "USB", "Bluetooth", "USB receiver"...
    pub connection: String,
    pub keyboard: bool,
    pub pointer: bool,
    pub connected: bool,
    /// Another device here has exactly this identity (two of one model
    /// without serial numbers): a rule for it applies to all of them.
    pub twins: bool,
}

/// Why keys can't be changed for one keyboard or mouse here, in plain words.
pub fn devices_unavailable_text(note: Option<&str>) -> String {
    match note {
        Some(n) if n.contains("merges") => {
            "Not on this desktop: it mixes all keyboards and mice together before oneMK sees them, so one-mk can't tell \
             which one you used. This works in an X11 session, on Windows and on macOS."
                .into()
        }
        Some(n) if !n.is_empty() => {
            let mut c = n.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>() + ".").unwrap_or_default()
        }
        _ => "This computer can't tell its keyboards and mice apart.".into(),
    }
}

/// What isn't tested about telling keyboards apart here, in plain words.
pub fn devices_untested_text(_note: &str) -> String {
    "Try your rule in the test box before using it on another computer.".into()
}

impl SeenDevice {
    /// "Connected · USB keyboard", "Asleep or unplugged · Bluetooth mouse":
    /// what matters most first, as the line may be cut.
    pub fn about(&self) -> String {
        let kind = match (self.keyboard, self.pointer) {
            (true, true) => "keyboard and mouse",
            (true, false) => "keyboard",
            (false, true) => "mouse",
            (false, false) => "input device",
        };
        let state = if self.connected { "Connected" } else { "Asleep or unplugged" };
        let how = match self.bus.as_str() {
            "unknown" | "virtual" | "" => String::new(),
            "internal" => "Built-in ".into(),
            _ => format!("{} ", self.connection),
        };
        let twins = if self.twins { " · Same ID as another" } else { "" };
        format!("{state} · {how}{kind}{twins}")
    }

    /// Made by software (X's XTEST devices): no one's keyboard or mouse.
    pub fn is_virtual(&self) -> bool {
        self.bus == "virtual"
    }
}

/// A paired computer, as far as scopes care.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub id: String,
    pub os: u8,
}

impl Peer {
    /// Whether `name` is the display text for two computers that share a
    /// name: `Studio (ab12-cd34)`. The daemon finds a computer by its own
    /// name or by an id, never by that text.
    fn name_is_decorated(&self) -> bool {
        !self.id.is_empty() && self.name.ends_with(&format!(" ({})", self.id))
    }

    /// Whether `query` (a name, or at least 4 characters of the id, as a
    /// saved scope or a command writes it) names this computer.
    pub fn matches(&self, query: &str) -> bool {
        peer_matches(query, &self.name, &self.id)
    }

    /// What names this computer in a saved rule's scope: its name, or its id
    /// when the name shown is the decorated one (see `name_is_decorated`).
    pub fn scope_arg(&self) -> String {
        if self.name_is_decorated() { self.id.clone() } else { self.name.clone() }
    }

    /// What names this computer after `switch`: its name, or its id when the
    /// name is decorated or reads as one of the reserved words (`here`,
    /// `next`), which a computer called that would otherwise be taken for.
    pub fn switch_arg(&self) -> String {
        let reserved = ["here", "next"].iter().any(|w| self.name.trim().eq_ignore_ascii_case(w));
        if reserved && !self.id.is_empty() { self.id.clone() } else { self.scope_arg() }
    }
}

pub fn os_word(o: u8) -> &'static str {
    match o {
        os::MACOS => "mac",
        os::WINDOWS => "windows",
        _ => "linux",
    }
}

/// "Macs", "Windows PCs", "Linux computers".
pub fn os_plural(o: u8) -> &'static str {
    match o {
        os::MACOS => "Macs",
        os::WINDOWS => "Windows PCs",
        _ => "Linux computers",
    }
}

fn parse_os(s: &str) -> Option<u8> {
    match s.trim().to_ascii_lowercase().as_str() {
        "mac" | "macos" | "osx" => Some(os::MACOS),
        "windows" | "win" => Some(os::WINDOWS),
        "linux" => Some(os::LINUX),
        _ => None,
    }
}

impl Scope {
    pub(crate) fn includes_local_os(&self, local_os: u8) -> bool {
        match self {
            Scope::All => true,
            Scope::Os(target) => *target == local_os,
            Scope::Device(_, target) => target.includes_local_os(local_os),
            Scope::Peer(_) => false,
        }
    }

    /// `--os mac` / `--peer=NAME`, for the command line.
    pub fn args(&self) -> Vec<String> {
        match self {
            Scope::All => vec![],
            Scope::Os(o) => vec![format!("--os={}", os_word(*o))],
            Scope::Peer(p) => vec![format!("--peer={p}")],
            Scope::Device(fp, on) => {
                let mut a = vec![format!("--device={fp}")];
                a.extend(on.args());
                a
            }
        }
    }

    /// "All computers", "Only Macs", "Only Studio PC", "Only Travel
    /// Keyboard", "Only Travel Keyboard, on Macs".
    pub fn label(&self) -> String {
        self.label_in(&[])
    }

    /// The same, with a computer named by its id (as a rule for one of two
    /// computers called alike is saved) shown by the name it has in `peers`.
    pub fn label_in(&self, peers: &[Peer]) -> String {
        match self {
            Scope::All => "All computers".into(),
            Scope::Os(o) => format!("Only {}", os_plural(*o)),
            Scope::Peer(p) => format!("Only {}", peers.iter().find(|x| peer_matches(p, &x.name, &x.id)).map_or(p.as_str(), |x| x.name.as_str())),
            Scope::Device(fp, on) => match &**on {
                Scope::All => format!("Only {}", device_name(fp)),
                Scope::Os(o) => format!("Only {}, on {}", device_name(fp), os_plural(*o)),
                on => format!("Only {}, on {}", device_name(fp), on.label_in(peers).trim_start_matches("Only ")),
            },
        }
    }

    /// How specific the scope is for a computer and device (`None`: not
    /// for them): a device beats a computer beats an OS beats everything,
    /// as in `one-mk`.
    pub(crate) fn rank(&self, f: &Facts) -> Option<u8> {
        match self {
            Scope::All => Some(0),
            Scope::Os(o) => (*o == f.os).then_some(1),
            Scope::Peer(p) => peer_matches(p, &f.name, &f.id).then_some(2),
            Scope::Device(fp, on) => {
                let rule = fp.parse::<Fingerprint>().ok()?;
                let device = f.device.as_deref()?.parse::<Fingerprint>().ok()?;
                rule.matches(&device).then_some(())?;
                Some(on.rank(f)? + 3)
            }
        }
    }

    /// From `scope = ...` (a string or an inline table) or a
    /// `[...scope]` table (`scope.os`, `scope.peer`, `scope.device`).
    fn read(get: &dyn Fn(&str) -> Option<Val>) -> Scope {
        let from_pair = |k: &str, v: &Val| match (k, v.as_str()) {
            ("os", Some(o)) => parse_os(o).map(Scope::Os),
            ("peer", Some(p)) if !p.trim().is_empty() => Some(Scope::Peer(p.trim().to_owned())),
            _ => None,
        };
        let mut pairs: Vec<(String, Val)> = match get("scope") {
            Some(Val::Table(kv)) => kv,
            _ => ["os", "peer", "device"].iter().filter_map(|k| Some(((*k).to_owned(), get(&format!("scope.{k}"))?))).collect(),
        };
        let device = pairs.iter().position(|(k, _)| k == "device").map(|i| pairs.remove(i).1);
        let on = pairs.first().and_then(|(k, v)| from_pair(k, v)).unwrap_or_default();
        match device.as_ref().and_then(Val::as_str).map(str::trim) {
            Some(d) if !d.is_empty() => Scope::Device(d.to_owned(), Box::new(on)),
            _ => on,
        }
    }

}

/// Like `one-mk`: the name, or at least 4 characters of the id.
fn peer_matches(query: &str, name: &str, id: &str) -> bool {
    let q = query.replace('-', "").to_ascii_lowercase();
    let id = id.replace('-', "").to_ascii_lowercase();
    name.eq_ignore_ascii_case(query) || (q.len() >= 4 && !id.is_empty() && id.starts_with(&q))
}

/// The computer input would go to: what decides which rules apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub name: String,
    pub id: String,
    pub os: u8,
    /// The keyboard or mouse input comes from (a fingerprint), if one
    /// was chosen: its rules apply on top.
    pub device: Option<String>,
    /// Ctrl and Cmd are swapped on the way to it (see [`swap_applies`]).
    pub swap: bool,
}

/// Whether Ctrl and Cmd are swapped between this computer (`local_os`) and
/// one running `peer_os` whose setting is `setting` (`auto`, `on`, `off`),
/// as the daemon decides it for a visit.
pub fn swap_applies(setting: &str, local_os: u8, peer_os: u8) -> bool {
    match setting {
        "on" => true,
        "off" => false,
        _ => local_os != peer_os && [local_os, peer_os].contains(&os::MACOS) && local_os != os::UNKNOWN && peer_os != os::UNKNOWN,
    }
}

impl Facts {
    /// The computers a choice in "For:" stands for. All computers: only
    /// the rules for every computer.
    pub fn of(scope: &Scope, peers: &[Peer]) -> Facts {
        match scope {
            Scope::All => Facts { name: String::new(), id: String::new(), os: os::UNKNOWN, device: None, swap: false },
            Scope::Os(o) => Facts { name: String::new(), id: String::new(), os: *o, device: None, swap: swap_applies("auto", os::local(), *o) },
            Scope::Peer(p) => match peers.iter().find(|x| peer_matches(p, &x.name, &x.id)) {
                Some(x) => Facts { name: x.name.clone(), id: x.id.clone(), os: x.os, device: None, swap: swap_applies("auto", os::local(), x.os) },
                None => Facts { name: p.clone(), id: String::new(), os: os::UNKNOWN, device: None, swap: false },
            },
            Scope::Device(fp, on) => Facts { device: Some(fp.clone()), ..Facts::of(on, peers) },
        }
    }

    /// The same, with the computer's own Ctrl/Cmd setting (`auto`, `on`,
    /// `off`) instead of the automatic one.
    pub fn with_swap_setting(self, setting: &str) -> Facts {
        let swap = swap_applies(setting, os::local(), self.os);
        Facts { swap, ..self }
    }
}

// ---------- the rules, as settings.toml has them ----------

/// A key or chord, or (a list) a sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum To {
    One(String),
    Many(Vec<String>),
}

impl To {
    pub fn describe(&self, target: u8) -> String {
        match self {
            To::One(text) if text.trim().eq_ignore_ascii_case("back") => "go back".into(),
            To::One(text) if text.trim().eq_ignore_ascii_case("forward") => "go forward".into(),
            To::One(text) if text == "desktop-left" => "Previous desktop".into(),
            To::One(text) if text == "desktop-right" => "Next desktop".into(),
            To::One(text) => text.parse::<SystemAction>().map_or_else(|_| text.clone(), |action| action.label_for(target).to_owned()),
            To::Many(steps) => steps.join(", then "),
        }
    }
    pub fn parse(&self) -> Result<Output, String> {
        match self {
            To::One(s) => s.parse().map_err(|e: keynames::NameError| e.0),
            To::Many(v) => Output::sequence(v).map_err(|e| e.0),
        }
    }

    fn read(v: &Val) -> Option<To> {
        match v {
            Val::Str(s) => Some(To::One(s.clone())),
            Val::List(_) => Some(To::Many(v.words())),
            _ => None,
        }
    }

    fn words(&self) -> Vec<String> {
        match self {
            To::One(s) => vec![s.clone()],
            To::Many(v) => v.clone(),
        }
    }
}

/// `[[input.keymap.rule]]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyRule {
    pub from: String,
    pub to: Option<To>,
    pub tap: Option<String>,
    pub hold: Option<String>,
    pub layer: Option<String>,
    pub timeout_ms: Option<u32>,
    pub scope: Scope,
    /// Only while this app has the focus on the computer typed on.
    pub app: Option<String>,
}

impl KeyRule {
    pub fn to(from: &str, to: &str) -> KeyRule {
        KeyRule { from: from.into(), to: Some(To::One(to.into())), ..Default::default() }
    }

    pub fn tap_hold(from: &str, tap: &str, hold: &str) -> KeyRule {
        KeyRule { from: from.into(), tap: Some(tap.into()), hold: Some(hold.into()), ..Default::default() }
    }

    /// The rule, parsed; errors say what is wrong, as `one-mk` says it.
    pub fn spec(&self) -> Result<RuleSpec, String> {
        let from: Trigger = self.from.parse().map_err(|e: keynames::NameError| format!("from: {e}"))?;
        let parse = |what: &str, s: &str| s.parse::<Output>().map_err(|e| format!("{what}: {e}"));
        let what = match (&self.to, &self.tap, &self.hold) {
            (Some(to), None, None) => What::To(to.parse().map_err(|e| format!("to: {e}"))?),
            (None, Some(tap), Some(hold)) => What::TapHold {
                tap: parse("tap", tap)?,
                hold: parse("hold", hold)?,
                timeout: self.timeout_ms.map(|ms| Duration::from_millis(u64::from(ms))),
            },
            (Some(_), _, _) => return Err("give `to`, or `tap` and `hold`, not both".into()),
            (None, None, None) => return Err("`to` is missing (or `tap` and `hold`)".into()),
            (None, _, _) => return Err("a tap/hold key needs both `tap` and `hold`".into()),
        };
        Ok(RuleSpec { from, layer: self.layer.as_ref().map(|l| l.to_ascii_lowercase()), what })
    }

    /// The same key, layer, scope and app (what `one-mk keymap add` replaces).
    fn same_place(&self, other: &KeyRule) -> bool {
        let layer = |r: &KeyRule| r.layer.as_ref().map(|l| l.to_ascii_lowercase());
        self.scope == other.scope
            && same_app(&self.app, &other.app)
            && layer(self) == layer(other)
            && matches!((self.from.parse::<Trigger>(), other.from.parse::<Trigger>()), (Ok(a), Ok(b)) if a == b)
    }
}

/// `[[input.button]]`: `from` a number (or a name), `to` what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonRule {
    pub from: String,
    pub to: To,
    pub scope: Scope,
    pub app: Option<String>,
}

impl ButtonRule {
    fn trigger(&self) -> Result<Trigger, String> {
        let t = match self.from.trim().parse::<u32>() {
            Ok(n) => keynames::button_code(n)
                .map(|c| Trigger::key(c as u16))
                .ok_or_else(|| format!("button {n}: buttons are numbered 1 to {}", keynames::BUTTONS))?,
            Err(_) => self.from.parse::<Trigger>().map_err(|e| e.0)?,
        };
        if !keynames::is_button(u32::from(t.key)) {
            return Err(format!("{} is a key, not a mouse button", keynames::key_name(u32::from(t.key))));
        }
        Ok(t)
    }

    fn spec(&self) -> Result<RuleSpec, String> {
        Ok(RuleSpec { from: self.trigger()?, layer: None, what: What::To(self.to.parse().map_err(|e| format!("to: {e}"))?) })
    }
}

/// `[[input.pointer]]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerRule {
    pub speed: Option<f64>,
    /// `system`, `off` or `curve`.
    pub accel: Option<String>,
    pub scope: Scope,
    pub app: Option<String>,
}

/// Notched wheels or touchpads (`device` in `[[input.scroll]]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Wheel,
    Touchpad,
}

impl Device {
    fn word(self) -> &'static str {
        match self {
            Device::Wheel => "wheel",
            Device::Touchpad => "touchpad",
        }
    }
}

/// `[[input.scroll]]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollRule {
    pub device: Option<Device>,
    pub reverse_vertical: Option<bool>,
    pub reverse_horizontal: Option<bool>,
    /// `on`, `off` or `auto`.
    pub natural: Option<String>,
    pub speed: Option<f64>,
    pub speed_x: Option<f64>,
    pub speed_y: Option<f64>,
    /// `auto`, `smooth` or `discrete`.
    pub mode: Option<String>,
    /// (modifier, behaviour) as written: `("shift", "horizontal")`.
    pub modifiers: Vec<(String, String)>,
    pub scope: Scope,
    pub app: Option<String>,
}

/// `[input]` in `settings.toml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    pub local_rules: bool,
    pub tap_ms: Option<u32>,
    pub keys: Vec<KeyRule>,
    pub pointer: Vec<PointerRule>,
    pub scroll: Vec<ScrollRule>,
    pub buttons: Vec<ButtonRule>,
    pub autoscroll: Vec<AutoscrollRow>,
    pub gestures: Vec<GestureRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoscrollRow {
    pub enabled: Option<bool>,
    pub speed: Option<f64>,
    pub button: Option<String>,
    pub scope: Scope,
    pub app: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GestureRow {
    pub button: String,
    pub direction: String,
    pub to: To,
    pub scope: Scope,
    pub app: Option<String>,
}

/// The repeats of an `[[array]]` table, each as a lookup by key (`scope.os`
/// for a table inside the repeat).
fn repeats<'a>(e: &'a [Entry], array: &str) -> Vec<Vec<(String, &'a Val)>> {
    let mut out: Vec<Vec<(String, &Val)>> = Vec::new();
    for x in e {
        let key = if x.table == array {
            x.key.clone()
        } else if let Some(sub) = x.table.strip_prefix(array).and_then(|r| r.strip_prefix('.')) {
            format!("{sub}.{}", x.key)
        } else {
            continue;
        };
        if out.len() <= x.index {
            out.resize_with(x.index + 1, Vec::new);
        }
        out[x.index].push((key, &x.val));
    }
    out.into_iter().filter(|entries| !entries.iter().any(|(key, value)| {
        key == "scope.local" || (key == "scope" && matches!(value, Val::Table(fields) if fields.iter().any(|(name, _)| name == "local")))
    })).collect()
}

impl Input {
    pub fn read(e: &[Entry]) -> Input {
        let tap_ms = e.iter().find(|x| x.table == "input.keymap" && x.key == "tap_ms").and_then(|x| x.val.as_int()).map(|n| n.clamp(0, 60_000) as u32);
        let local_rules = e.iter().find(|entry| entry.table == "input" && entry.key == "local_rules")
            .and_then(|entry| entry.val.as_bool()).unwrap_or(false);
        let mut input = Input { tap_ms, local_rules, ..Default::default() };
        let get = |key: &str| e.iter().find(|entry| {
            (entry.table == "input.autoscroll" && entry.key == key)
                || entry.table.strip_prefix("input.autoscroll.").is_some_and(|suffix| format!("{suffix}.{}", entry.key) == key)
        }).map(|entry| entry.val.clone());
        let local = get("scope.local").is_some() || matches!(get("scope"), Some(Val::Table(fields)) if fields.iter().any(|(name, _)| name == "local"));
        if !local && e.iter().any(|entry| entry.table == "input.autoscroll") {
            input.autoscroll.push(AutoscrollRow { enabled: get("enabled").and_then(|value| value.as_bool()),
                speed: get("speed").and_then(|value| value.as_f64()), button: get("button").and_then(|value| value.as_str().map(str::to_owned)), scope: Scope::read(&get),
                app: get("app").and_then(|value| value.as_str().map(str::to_owned)) });
        }
        for entries in repeats(e, "input.autoscroll.rule") {
            let get = |key: &str| entries.iter().find(|(name, _)| name == key).map(|(_, value)| (*value).clone());
            input.autoscroll.push(AutoscrollRow { enabled: get("enabled").and_then(|value| value.as_bool()),
                speed: get("speed").and_then(|value| value.as_f64()), button: get("button").and_then(|value| value.as_str().map(str::to_owned)), scope: Scope::read(&get),
                app: get("app").and_then(|value| value.as_str().map(str::to_owned)) });
        }
        for entries in repeats(e, "input.gesture") {
            let get = |key: &str| entries.iter().find(|(name, _)| name == key).map(|(_, value)| (*value).clone());
            if let (Some(Val::Str(button)), Some(Val::Str(direction)), Some(to)) = (get("button"), get("direction"), get("to").as_ref().and_then(To::read)) {
                input.gestures.push(GestureRow { button, direction, to, scope: Scope::read(&get),
                    app: get("app").and_then(|value| value.as_str().map(str::to_owned)) });
            }
        }
        for r in repeats(e, "input.keymap.rule") {
            let get = |k: &str| r.iter().find(|(key, _)| key == k).map(|(_, v)| (*v).clone());
            let text = |k: &str| get(k).and_then(|v| v.as_str().map(str::to_owned));
            let Some(from) = text("from") else { continue };
            input.keys.push(KeyRule {
                from,
                to: get("to").as_ref().and_then(To::read),
                tap: text("tap"),
                hold: text("hold"),
                layer: text("layer"),
                timeout_ms: get("timeout_ms").and_then(|v| v.as_int()).map(|n| n.clamp(0, 60_000) as u32),
                scope: Scope::read(&get),
                app: text("app"),
            });
        }
        for r in repeats(e, "input.button") {
            let get = |k: &str| r.iter().find(|(key, _)| key == k).map(|(_, v)| (*v).clone());
            let from = match get("from") {
                Some(Val::Int(n)) => n.to_string(),
                Some(Val::Str(s)) => s,
                _ => continue,
            };
            let Some(to) = get("to").as_ref().and_then(To::read) else { continue };
            input.buttons.push(ButtonRule { from, to, scope: Scope::read(&get), app: get("app").and_then(|v| v.as_str().map(str::to_owned)) });
        }
        for r in repeats(e, "input.pointer") {
            let get = |k: &str| r.iter().find(|(key, _)| key == k).map(|(_, v)| (*v).clone());
            input.pointer.push(PointerRule {
                speed: get("speed").and_then(|v| v.as_f64()),
                accel: get("acceleration").and_then(|v| v.as_str().map(str::to_ascii_lowercase)),
                scope: Scope::read(&get),
                app: get("app").and_then(|v| v.as_str().map(str::to_owned)),
            });
        }
        for r in repeats(e, "input.scroll") {
            let get = |k: &str| r.iter().find(|(key, _)| key == k).map(|(_, v)| (*v).clone());
            let text = |k: &str| get(k).and_then(|v| v.as_str().map(str::to_ascii_lowercase));
            let mut modifiers: Vec<(String, String)> = match get("modifiers") {
                Some(Val::Table(kv)) => kv.iter().filter_map(|(k, v)| Some((k.to_ascii_lowercase(), v.as_str()?.to_ascii_lowercase()))).collect(),
                _ => vec![],
            };
            for (k, v) in &r {
                if let (Some(m), Some(b)) = (k.strip_prefix("modifiers."), v.as_str()) {
                    modifiers.push((m.to_ascii_lowercase(), b.to_ascii_lowercase()));
                }
            }
            input.scroll.push(ScrollRule {
                device: match text("device").as_deref() {
                    Some("wheel") => Some(Device::Wheel),
                    Some("touchpad") => Some(Device::Touchpad),
                    _ => None,
                },
                reverse_vertical: get("reverse_vertical").and_then(|v| v.as_bool()),
                reverse_horizontal: get("reverse_horizontal").and_then(|v| v.as_bool()),
                natural: text("natural"),
                speed: get("speed").and_then(|v| v.as_f64()),
                speed_x: get("speed_x").and_then(|v| v.as_f64()),
                speed_y: get("speed_y").and_then(|v| v.as_f64()),
                mode: text("mode"),
                modifiers,
                scope: Scope::read(&get),
                app: text("app"),
            });
        }
        input
    }

    /// How long a tap/hold key waits, clamped as `one-mk` clamps it.
    pub fn tap(&self) -> Duration {
        Duration::from_millis(u64::from(self.tap_ms.unwrap_or(200).clamp(remap::MIN_TAP_MS, remap::MAX_TAP_MS)))
    }

    /// Key and button rules that apply to `f`, most specific scope first,
    /// one per key, layer and modifiers (like `one-mk`), with where each
    /// came from: `Ok(i)` key rule `i`, `Err(i)` button rule `i`.
    pub fn specs(&self, f: &Facts) -> Vec<(RuleSpec, Result<usize, usize>)> {
        let keys = self.keys.iter().enumerate().map(|(i, r)| (Ok(i), &r.scope, &r.app, r.spec()));
        let buttons = self.buttons.iter().enumerate().map(|(i, r)| (Err(i), &r.scope, &r.app, r.spec()));
        let mut found: Vec<(u8, usize, RuleSpec, Result<usize, usize>)> = keys
            .chain(buttons)
            .enumerate()
            .filter_map(|(n, (at, scope, app, spec))| Some((scope.rank(f)?, n, spec.ok().filter(|_| app.is_none())?, at)))
            .collect();
        found.sort_by_key(|(rank, n, ..)| (std::cmp::Reverse(*rank), *n));
        let mut seen: Vec<(Option<String>, Trigger)> = Vec::new();
        let mut out = Vec::new();
        for (_, _, spec, at) in found {
            let k = (spec.layer.clone(), spec.from);
            if !seen.contains(&k) {
                seen.push(k);
                out.push((spec, at));
            }
        }
        out
    }

    /// The keymap for `f`, compiled as a visit would compile it.
    pub fn keymap(&self, f: &Facts) -> Result<Keymap, String> {
        self.compile_for(f, f.os).map(|compiled| compiled.map)
    }

    pub fn test_keymap(&self, facts: &Facts) -> Result<Keymap, String> {
        // Keep filtering by the chosen scope. Only shortcut resolution
        // falls back to this computer's OS for an unspecified target.
        let os = if facts.os == os::UNKNOWN { os::local() } else { facts.os };
        self.compile_for(facts, os).map(|compiled| compiled.map)
    }

    fn compile_for(&self, facts: &Facts, os: u8) -> Result<remap::Compiled, String> {
        let specs: Vec<_> = self.specs(facts).into_iter().map(|(spec, _)| spec).collect();
        Keymap::compile(&specs, remap::Target { os, swap: facts.swap }, self.tap()).map_err(|e| e.0)
    }

    /// The key that switches layer `name` on, if one does.
    pub fn layer_key(&self, name: &str) -> Option<u16> {
        let is = |s: &Option<String>| s.as_deref().is_some_and(|t| t.trim().eq_ignore_ascii_case(&format!("layer:{name}")));
        self.keys
            .iter()
            .find(|r| is(&r.hold) || r.to.as_ref().is_some_and(|t| is(&Some(t.words().join(" ")))))
            .and_then(|r| r.from.parse::<Trigger>().ok())
            .map(|t| t.key)
    }
}

// ---------- names, for people ----------

/// This computer's names for its modifier keys.
pub fn meta_word() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd"
    } else if cfg!(windows) {
        "Win"
    } else {
        "Super"
    }
}
pub fn mouse_button_label(name: &str) -> String {
    keynames::parse_key(name).ok().filter(|code| keynames::is_button(*code))
        .map(|code| format!("the {}", key_label(code).replace(" click", " button").to_lowercase()))
        .unwrap_or_else(|| name.to_owned())
}

pub fn primary_mouse_button(name: &str) -> bool {
    name.trim().parse::<u32>().is_ok_and(|number| matches!(number, 1 | 2))
        || keynames::parse_key(name).is_ok_and(|code| matches!(code, 0x110 | 0x111))
}

#[cfg(test)]
#[test]
fn mouse_mode_primary_button_aliases_are_unsafe() {
    for name in ["1", "2", " 2 ", "button1", "button2", "leftbutton", "rightbutton", "mouse1", "mouse2"] {
        assert!(primary_mouse_button(name), "{name}");
    }
    for name in ["middle", "button4", "button5"] {
        assert!(!primary_mouse_button(name), "{name}");
    }
}

fn alt_word() -> &'static str {
    if cfg!(target_os = "macos") { "Option" } else { "Alt" }
}
fn ctrl_word() -> &'static str {
    if cfg!(target_os = "macos") { "Control" } else { "Ctrl" }
}

fn title(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A key's name for a sentence: "Caps Lock", "Left Alt", "F13", "A".
pub fn key_label(code: u32) -> String {
    let fixed = match code {
        133 => "Copy key",
        1 => "Esc",
        14 => "Backspace",
        15 => "Tab",
        28 => "Enter",
        57 => "Space",
        58 => "Caps Lock",
        41 => "`",
        86 => "Key left of Z",
        99 => "Print Screen",
        70 => "Scroll Lock",
        119 => "Pause",
        69 => "Num Lock",
        102 => "Home",
        103 => "Up",
        104 => "Page Up",
        105 => "Left",
        106 => "Right",
        107 => "End",
        108 => "Down",
        109 => "Page Down",
        110 => "Insert",
        111 => "Delete",
        127 => "Menu",
        96 => "Keypad Enter",
        55 => "Keypad *",
        74 => "Keypad -",
        78 => "Keypad +",
        98 => "Keypad /",
        83 => "Keypad .",
        117 => "Keypad =",
        0x1d0 => "Fn",
        0x110 => "Left click",
        0x111 => "Right click",
        0x112 => "Middle click",
        0x113 => "Back button",
        0x114 => "Forward button",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    let side = |left: bool| if left { "Left" } else { "Right" };
    match code {
        29 | 97 => return format!("{} {}", side(code == 29), ctrl_word()),
        42 | 54 => return format!("{} Shift", side(code == 42)),
        56 | 100 => return format!("{} {}", side(code == 56), alt_word()),
        125 | 126 => return format!("{} {}", side(code == 125), meta_word()),
        71..=73 | 75..=77 | 79..=82 => return format!("Keypad {}", keynames::key_name(code).trim_start_matches("kp")),
        0x115..=0x117 => return format!("Button {}", code - 0x110 + 1),
        _ => {}
    }
    let name = keynames::key_name(code);
    // A key with no name (a keyboard's own extra key, detected): its number.
    if name.starts_with("code:") {
        return format!("Key code {code}");
    }
    if name.chars().count() == 1 || name.starts_with('f') && name[1..].parse::<u8>().is_ok() {
        return name.to_uppercase();
    }
    title(&name)
}

/// A key's name on its drawn cap: short.
pub fn cap_label(code: u16) -> String {
    let s = match code {
        133 => "Copy key",
        14 => "Bksp",
        58 => "Caps",
        42 | 54 => "Shift",
        29 | 97 => "Ctrl",
        56 | 100 => if cfg!(target_os = "macos") { "Opt" } else { "Alt" },
        125 | 126 => if cfg!(target_os = "macos") { "Cmd" } else if cfg!(windows) { "Win" } else { "Sup" },
        57 => "",
        99 => "PrSc",
        70 => "ScLk",
        110 => "Ins",
        111 => "Del",
        104 => "PgUp",
        109 => "PgDn",
        103 => "↑",
        108 => "↓",
        105 => "←",
        106 => "→",
        69 => "Num",
        96 => "Ent",
        55 => "*",
        74 => "-",
        78 => "+",
        98 => "/",
        83 => ".",
        71..=73 | 75..=77 | 79..=82 => return keynames::key_name(u32::from(code)).trim_start_matches("kp").to_owned(),
        86 => "<>",
        102 => "Home",
        127 => "Mnu",
        119 => "Brk",
        _ => {
            // Punctuation by its character: `-`, `[`, `;`.
            let one = keynames::all_names().find(|(c, ..)| *c == u32::from(code)).and_then(|(_, _, names)| names.iter().find(|n| n.chars().count() == 1 && !n.chars().all(char::is_alphanumeric)));
            return one.map_or_else(|| key_label(u32::from(code)), |n| (*n).to_owned());
        }
    };
    s.to_owned()
}

/// Modifier bits as words: "Ctrl+Shift+", the right-hand ones said so.
fn mods_text(bits: u8) -> String {
    let names = [ctrl_word(), "Shift", alt_word(), meta_word()];
    let mut out = String::new();
    for i in 0..8 {
        if bits & (1 << i) != 0 {
            let side = if i >= 4 { "Right " } else { "" };
            out += &format!("{side}{}+", names[i % 4]);
        }
    }
    out
}

/// A chord as people write it: "Ctrl+Shift+Esc"; a left modifier alone
/// is just "Ctrl".
pub fn chord_text(c: &Chord) -> String {
    let key = u32::from(c.key);
    let k = match key {
        29 if c.mods == 0 => ctrl_word().to_owned(),
        42 if c.mods == 0 => "Shift".into(),
        56 if c.mods == 0 => alt_word().into(),
        125 if c.mods == 0 => meta_word().into(),
        _ => key_label(key),
    };
    format!("{}{k}", mods_text(c.mods))
}

/// A trigger as people write it: "Ctrl+H" (either Ctrl), "Right Alt".
pub fn trigger_text(t: &Trigger) -> String {
    let names = [ctrl_word(), "Shift", alt_word(), meta_word()];
    let mut out = String::new();
    for (g, n) in names.iter().enumerate() {
        if t.groups & (1 << g) != 0 {
            out += &format!("{n}+");
        }
    }
    out += &mods_text(t.sided);
    out + &key_label(u32::from(t.key))
}

fn action_text(a: &Action) -> String {
    match a {
        Action::Switch(to) if to == "next" => "switches to the next computer".into(),
        Action::Switch(to) if to == "here" => "comes back to this computer".into(),
        Action::Switch(to) => format!("switches to {to}"),
        Action::LockPointer(None) => "keeps the pointer here (on and off)".into(),
        Action::LockPointer(Some(true)) => "keeps the pointer here".into(),
        Action::LockPointer(Some(false)) => "lets the pointer cross again".into(),
        Action::LockAll => "locks every screen".into(),
        Action::Run(cmd) => format!("runs `{cmd}`"),
        Action::Scroll { .. } => format!("{}s", Output::Action(a.clone())),
    }
}

/// What an output does, as the end of a sentence: "Esc", "types Ctrl+A,
/// then Ctrl+C", "does nothing", "switches to the next computer".
pub fn out_text(o: &Output) -> String {
    out_text_for(o, os::UNKNOWN)
}

pub fn out_text_for(o: &Output, target: u8) -> String {
    match o {
        Output::Chord(c) => chord_text(c),
        Output::Sequence(s) => format!("types {}", s.iter().map(chord_text).collect::<Vec<_>>().join(", then ")),
        Output::None => "does nothing".into(),
        Output::Layer(l) => format!("hold it for the “{l}” keys"),
        Output::Nav(Nav::Back) => "Back (browser)".into(),
        Output::Nav(Nav::Forward) => "Forward (browser)".into(),
        Output::System(action) => action.label_for(target).into(),
        Output::Action(a) => action_text(a),
    }
}

/// The same, as the thing a tap or a hold gives: "Esc", "nothing", "the
/// “nav” keys".
fn out_noun(o: &Output, target: u8) -> String {
    match o {
        Output::None => "nothing".into(),
        Output::Layer(l) => format!("the “{l}” keys"),
        _ => out_text_for(o, target),
    }
}

/// Whether two rules' apps are the same (as `one-mk` compares them).
pub fn same_app(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
        _ => false,
    }
}

/// A rule's app in words: "only in Code", "in every app except Code".
pub fn app_text(app: &str) -> String {
    let app = app.trim();
    match app.strip_prefix('!') {
        Some(not) => format!("in every app except {}", not.trim_start()),
        None => format!("only in {app}"),
    }
}

/// A rule in one sentence: "Caps Lock → Esc when tapped, Ctrl when held",
/// "Ctrl+Space → F13, only in Code"; `Err` with the rule as written and
/// why it can't be used.
#[cfg(test)]
fn sentence(input: &Input, r: &KeyRule) -> Result<String, String> {
    sentence_for(input, r, scope_os(&r.scope).unwrap_or(os::UNKNOWN))
}

pub fn sentence_for(input: &Input, r: &KeyRule, target: u8) -> Result<String, String> {
    let spec = r.spec()?;
    let from = trigger_text(&spec.from);
    let from = match &spec.layer {
        Some(l) => match input.layer_key(l) {
            Some(k) => format!("{} + {from}", key_label(u32::from(k))),
            None => format!("In “{l}”: {from}"),
        },
        None => from,
    };
    let what = match &spec.what {
        What::To(o) => out_text_for(o, target),
        What::TapHold { tap, hold, timeout } => {
            let after = timeout.map_or(String::new(), |t| format!(" (after {} ms)", t.as_millis()));
            format!("{} when tapped, {} when held{after}", out_noun(tap, target), out_noun(hold, target))
        }
    };
    let app = r.app.as_deref().map_or(String::new(), |a| format!(", {}", app_text(a)));
    Ok(format!("{from} → {what}{app}"))
}

/// A rule as written, for one that can't be read.
pub fn as_written(r: &KeyRule) -> String {
    let what = match (&r.to, &r.tap, &r.hold) {
        (Some(t), _, _) => t.words().join(", "),
        (None, Some(t), Some(h)) => format!("tap {t}, hold {h}"),
        _ => "?".into(),
    };
    format!("{} → {what}", r.from)
}

/// What the rule says the key does, short, for its drawn cap: "Esc",
/// "Esc/Ctrl", "Off", "Next".
pub fn cap_target(what: &What, target: u8) -> String {
    let short = |o: &Output| match o {
        Output::Chord(c) if c.mods == 0 => cap_label(c.key),
        // A shortcut: its key, marked as one (the whole of it is on hover).
        Output::Chord(c) => format!("+{}", cap_label(c.key)),
        Output::Sequence(_) => "Keys…".into(),
        Output::None => "Off".into(),
        Output::Layer(_) => "Hold".into(),
        Output::Nav(Nav::Back) => "Back".into(),
        Output::Nav(Nav::Forward) => "Fwd".into(),
        Output::System(action) => match action {
            SystemAction::NextDesktop => "Next desk",
            SystemAction::PreviousDesktop => "Prev desk",
            SystemAction::Overview => action.label_for(target),
            SystemAction::ScreenshotArea | SystemAction::ScreenshotTool => "Capture",
            SystemAction::AppWindows if target == os::MACOS => action.label_for(target),
            SystemAction::AppWindows => "App wins",
            SystemAction::ShowDesktop => "Desktop",
            SystemAction::Launcher => "Launcher",
            SystemAction::Search => "Search",
            SystemAction::LockScreen => "Lock",
            SystemAction::FullScreen => "Full scr",
            SystemAction::Emoji => "Emoji",
            SystemAction::SwitchApp => "Swap app",
            SystemAction::CloseWindow => "Close win",
            _ => action.label(),
        }.into(),
        Output::Action(Action::Switch(_)) => "Switch".into(),
        Output::Action(Action::LockPointer(_)) => "Lock ptr".into(),
        Output::Action(Action::LockAll) => "Lock all".into(),
        Output::Action(Action::Run(_)) => "Run".into(),
        Output::Action(Action::Scroll { .. }) => "Scroll".into(),
    };
    match what {
        What::To(o) => short(o),
        What::TapHold { tap, hold, .. } => format!("{}/{}", short(tap), short(hold)),
    }
}

// ---------- the drawn keyboard ----------

/// A key on the drawing, in key units (1 = a letter key).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cap {
    pub code: u16,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The drawing's size in key units, with and without the keypad.
#[cfg_attr(not(test), allow(dead_code))] // The models' tests size the board with it.
pub fn keyboard_size(keypad: bool) -> (f32, f32) {
    (if keypad { 23.75 } else { 19.25 }, 6.5)
}

/// A compact keyboard from the evdev code space: the F row, the main block
/// (laid out by position, so it fits ANSI and ISO keyboards alike), the
/// editing keys and arrows, and the keypad if asked. On a Mac the bottom
/// row is Control, Option, Command, as Apple draws it.
pub fn keyboard(keypad: bool, mac: bool) -> Vec<Cap> {
    let mut caps = Vec::new();
    let mut row = |y: f32, x0: f32, keys: &[(u16, f32)]| {
        let mut x = x0;
        for &(code, w) in keys {
            if code != 0 {
                caps.push(Cap { code, x, y, w, h: 1.0 });
            }
            x += w;
        }
    };
    let one = |codes: std::ops::RangeInclusive<u16>| codes.map(|c| (c, 1.0)).collect::<Vec<_>>();
    // The F row: Esc, a gap, then F1-F4, F5-F8, F9-F12 in groups.
    row(0.0, 0.0, &[(1, 1.0), (0, 1.0)]);
    row(0.0, 2.0, &one(59..=62));
    row(0.0, 6.5, &one(63..=66));
    row(0.0, 11.0, &[(67, 1.0), (68, 1.0), (87, 1.0), (88, 1.0)]);
    row(0.0, 15.5, &[(99, 1.25), (70, 1.25), (119, 1.25)]);
    let r1: Vec<(u16, f32)> = [(41, 1.0)].into_iter().chain(one(2..=13)).chain([(14, 2.0)]).collect();
    row(1.5, 0.0, &r1);
    let r2: Vec<(u16, f32)> = [(15, 1.5)].into_iter().chain(one(16..=27)).chain([(43, 1.5)]).collect();
    row(2.5, 0.0, &r2);
    let r3: Vec<(u16, f32)> = [(58, 1.75)].into_iter().chain(one(30..=40)).chain([(28, 2.25)]).collect();
    row(3.5, 0.0, &r3);
    let r4: Vec<(u16, f32)> = [(42, 2.25)].into_iter().chain(one(44..=53)).chain([(54, 2.75)]).collect();
    row(4.5, 0.0, &r4);
    if mac {
        row(5.5, 0.0, &[(29, 1.25), (56, 1.25), (125, 1.5), (57, 7.0), (126, 1.5), (100, 1.25), (97, 1.25)]);
    } else {
        row(5.5, 0.0, &[(29, 1.25), (125, 1.25), (56, 1.25), (57, 6.25), (100, 1.25), (126, 1.25), (127, 1.25), (97, 1.25)]);
    }
    row(1.5, 15.5, &[(110, 1.25), (102, 1.25), (104, 1.25)]);
    row(2.5, 15.5, &[(111, 1.25), (107, 1.25), (109, 1.25)]);
    row(4.5, 16.75, &[(103, 1.25)]);
    row(5.5, 15.5, &[(105, 1.25), (108, 1.25), (106, 1.25)]);
    if keypad {
        row(1.5, 19.75, &[(69, 1.0), (98, 1.0), (55, 1.0), (74, 1.0)]);
        row(2.5, 19.75, &[(71, 1.0), (72, 1.0), (73, 1.0)]);
        row(3.5, 19.75, &[(75, 1.0), (76, 1.0), (77, 1.0)]);
        row(4.5, 19.75, &[(79, 1.0), (80, 1.0), (81, 1.0)]);
        row(5.5, 19.75, &[(82, 2.0), (83, 1.0)]);
        caps.push(Cap { code: 78, x: 22.75, y: 2.5, w: 1.0, h: 2.0 });
        caps.push(Cap { code: 96, x: 22.75, y: 4.5, w: 1.0, h: 2.0 });
    }
    caps
}

/// The key the arrows lead to from `from`: along its row (left, right),
/// or to the nearest key in the row above or below.
pub fn neighbour(caps: &[Cap], from: usize, dx: i8, dy: i8) -> Option<usize> {
    let c = caps.get(from)?;
    let (cx, cy) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
    let center = |k: &Cap| (k.x + k.w / 2.0, k.y + k.h / 2.0);
    let overlaps_row = |k: &Cap| k.y < c.y + c.h - 0.01 && k.y + k.h > c.y + 0.01;
    if dx != 0 {
        caps.iter()
            .enumerate()
            .filter(|(i, k)| *i != from && overlaps_row(k) && (center(k).0 - cx) * f32::from(dx) > 0.0)
            .min_by(|(_, a), (_, b)| (center(a).0 - cx).abs().total_cmp(&(center(b).0 - cx).abs()))
            .map(|(i, _)| i)
    } else {
        // The nearest row in that direction, then the key nearest across.
        let ahead = |k: &Cap| (center(k).1 - cy) * f32::from(dy) > 0.25;
        let row_dy = caps.iter().filter(|k| ahead(k)).map(|k| (center(k).1 - cy).abs()).min_by(f32::total_cmp)?;
        caps.iter()
            .enumerate()
            .filter(|(_, k)| ahead(k) && ((center(k).1 - cy).abs() - row_dy).abs() < 0.3)
            .min_by(|(_, a), (_, b)| {
                let d = |k: &Cap| if (k.x..k.x + k.w).contains(&cx) { 0.0 } else { (center(k).0 - cx).abs() };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, _)| i)
    }
}

// ---------- quick setups ----------

/// A quick setup: an id, its chip's words and the rules it adds.
pub struct Preset {
    pub id: &'static str,
    pub label: String,
    pub rules: Vec<KeyRule>,
}

pub fn presets(mac: bool) -> Vec<Preset> {
    let swap = |a: &str, b: &str, ra: &str, rb: &str| vec![KeyRule::to(a, b), KeyRule::to(b, a), KeyRule::to(ra, rb), KeyRule::to(rb, ra)];
    let nav = |k: &str, to: &str| KeyRule { layer: Some("nav".into()), ..KeyRule::to(k, to) };
    vec![
        Preset { id: "caps", label: "Caps Lock as Esc/Ctrl".into(), rules: vec![KeyRule::tap_hold("capslock", "esc", "lctrl")] },
        Preset { id: "alt-meta", label: format!("Swap {} and {}", alt_word(), meta_word()), rules: swap("lalt", "lmeta", "ralt", "rmeta") },
        Preset {
            id: "ctrl-cmd",
            label: if mac { "Swap Control and Cmd".into() } else { "Swap Ctrl and Cmd (for Mac keyboards)".into() },
            rules: swap("lctrl", "lmeta", "rctrl", "rmeta"),
        },
        Preset {
            id: "arrows",
            label: format!("Arrow keys on Right {} + HJKL", alt_word()),
            rules: vec![KeyRule::to("ralt", "layer:nav"), nav("h", "left"), nav("j", "down"), nav("k", "up"), nav("l", "right")],
        },
    ]
}

/// Whether every rule of a quick setup is there, for `scope`.
pub fn preset_on(input: &Input, p: &Preset, scope: &Scope) -> bool {
    p.rules.iter().all(|want| {
        let want = KeyRule { scope: scope.clone(), ..want.clone() };
        input.keys.iter().any(|have| have.same_place(&want) && have.spec().ok().map(|s| s.what) == want.spec().ok().map(|s| s.what))
    })
}

/// The commands that turn a quick setup on (or off: its rules removed).
pub fn preset_commands(p: &Preset, scope: &Scope, on: bool) -> Vec<Vec<String>> {
    p.rules
        .iter()
        .map(|r| {
            let r = KeyRule { scope: scope.clone(), ..r.clone() };
            if on { add_args(&r) } else { remove_args(&r) }
        })
        .collect()
}

// ---------- commands ----------

/// `one-mk keymap add [options] -- FROM TO...` (options first, then `--`,
/// so no name is ever taken for an option).
pub fn add_args(r: &KeyRule) -> Vec<String> {
    let mut a = vec!["keymap".to_owned(), "add".into()];
    if let (Some(t), Some(h)) = (&r.tap, &r.hold) {
        a.push(format!("--tap={t}"));
        a.push(format!("--hold={h}"));
        if let Some(ms) = r.timeout_ms {
            a.push(format!("--timeout-ms={ms}"));
        }
    }
    if let Some(l) = &r.layer {
        a.push(format!("--layer={l}"));
    }
    a.extend(r.scope.args());
    a.extend(r.app.as_ref().map(|app| format!("--app={app}")));
    a.push("--".into());
    a.push(r.from.clone());
    if let Some(to) = &r.to {
        a.extend(to.words());
    }
    a
}

/// `one-mk keymap remove`, for the rule's key, layer, scope and app.
pub fn remove_args(r: &KeyRule) -> Vec<String> {
    let mut a = vec!["keymap".to_owned(), "remove".into()];
    if let Some(l) = &r.layer {
        a.push(format!("--layer={l}"));
    }
    a.extend(r.scope.args());
    a.extend(r.app.as_ref().map(|app| format!("--app={app}")));
    a.extend(["--".into(), r.from.clone()]);
    a
}

/// `one-mk buttons map N TO`, or `unmap N` when `to` is `None`.
pub fn button_args(n: u32, to: Option<&str>, scope: &Scope) -> Vec<String> {
    let mut a = vec!["buttons".to_owned(), if to.is_some() { "map" } else { "unmap" }.into()];
    a.extend(scope.args());
    a.extend(["--".into(), n.to_string()]);
    a.extend(to.map(str::to_owned));
    a
}

pub fn pointer_args(speed: Option<f64>, accel: Option<&str>, scope: &Scope) -> Vec<String> {
    let mut a = vec!["pointer".to_owned()];
    if let Some(s) = speed {
        a.push(format!("--speed={}", trim_num(s)));
    }
    if let Some(m) = accel {
        a.push(format!("--accel={m}"));
    }
    a.extend(scope.args());
    a
}

/// One scroll setting changed with `one-mk scroll`.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollChange {
    ReverseVertical(bool),
    ReverseHorizontal(bool),
    Speed(f64),
    /// `auto`, `smooth`, `discrete`.
    Mode(&'static str),
    /// `None`: as on this computer (unset).
    Natural(Option<&'static str>),
    /// A modifier (`shift`) and what it does (`horizontal`, `x5`, or `off`: a
    /// setting of its own that does nothing, which a wider scope's setting
    /// cannot override; `unchanged` would forget the scope's own instead).
    Modifier(&'static str, &'static str),
}

fn trim_num(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The natural-scrolling value that does what leaving it unset does on this
/// computer: it sends scrolling as this computer's own system does.
fn natural_like_here() -> &'static str {
    if os::local() == os::MACOS { "on" } else { "off" }
}

/// What "As here" for natural scrolling asks, for the computers and kind
/// chosen (`f` stands for them): forgetting the setting where this scope has
/// one of its own and nothing broader would then supply one; where a broader
/// rule would (or supplies it already), a setting of its own that does what
/// unset does here.
pub fn natural_as_here(input: &Input, f: &Facts, scope: &Scope, device: Option<Device>) -> ScrollChange {
    // What would still apply once this scope's own setting is gone.
    let mut without = input.clone();
    for r in without.scroll.iter_mut().filter(|r| r.scope == *scope && r.device == device && r.app.is_none()) {
        r.natural = None;
    }
    if scroll_shown(&without, f, device).natural.is_some() { ScrollChange::Natural(Some(natural_like_here())) } else { ScrollChange::Natural(None) }
}

/// The commands for a scroll change. Natural scrolling is unset with one
/// `--clear-natural`, which forgets that setting in every table for the place
/// and leaves all the rest of them (per-axis speeds, speeds with many digits,
/// the other tables) as they are, in a single write.
pub fn scroll_commands(input: &Input, scope: &Scope, device: Option<Device>, change: &ScrollChange) -> Vec<Vec<String>> {
    let base = |a: &mut Vec<String>| {
        if let Some(d) = device {
            a.push(format!("--kind={}", d.word()));
        }
        a.extend(scope.args());
    };
    let mut a = vec!["scroll".to_owned()];
    match change {
        ScrollChange::Natural(None) => {
            if !input.scroll.iter().any(|r| r.scope == *scope && r.device == device && r.app.is_none() && r.natural.is_some()) {
                return vec![];
            }
            a.push("--clear-natural".into());
        }
        ScrollChange::ReverseVertical(v) => a.push(format!("--reverse-vertical={}", if *v { "on" } else { "off" })),
        ScrollChange::ReverseHorizontal(v) => a.push(format!("--reverse-horizontal={}", if *v { "on" } else { "off" })),
        ScrollChange::Speed(s) => a.push(format!("--speed={}", trim_num(*s))),
        ScrollChange::Mode(m) => a.push(format!("--mode={m}")),
        ScrollChange::Natural(Some(n)) => a.push(format!("--natural={n}")),
        ScrollChange::Modifier(m, b) => a.push(format!("--modifier={m}={b}")),
    }
    base(&mut a);
    vec![a]
}

// ---------- the checks `one-mk keymap add` makes ----------

/// Whether `one-mk keymap add` would take this rule, with its message if
/// not: the rule must read, a mouse button belongs under Mouse, a tap/hold
/// key must build on its own, and every rule for all computers must still
/// build together (layers, sizes).
pub fn check_add(input: &Input, rule: &KeyRule) -> Result<(), String> {
    let spec = rule.spec()?;
    if keynames::is_button(u32::from(spec.from.key)) {
        return Err(format!("{} is a mouse button: change it under Mouse", trigger_text(&spec.from)));
    }
    if let Some(app) = &rule.app {
        // As `--app` is checked: it must read, and be one the computers named could ever have.
        let pattern = app.parse::<one_mk_proto::appmatch::AppPattern>()?;
        if let Some(why) = scope_os(&rule.scope).and_then(|o| pattern.never_on(o)) {
            return Err(why);
        }
    }
    let any = remap::Target { os: os::UNKNOWN, swap: false };
    if let What::TapHold { .. } = spec.what {
        Keymap::compile(std::slice::from_ref(&spec), any, remap::DEFAULT_TAP).map_err(|e| e.0)?;
    }
    let mut all = input.clone();
    match all.keys.iter().position(|r| r.same_place(rule)) {
        Some(i) => all.keys[i] = rule.clone(),
        None => all.keys.push(rule.clone()),
    }
    all.keymap(&Facts::of(&Scope::All, &[])).map(drop)
}

/// The OS a scope is for, if it names one.
fn scope_os(scope: &Scope) -> Option<u8> {
    match scope {
        Scope::Os(o) => Some(*o),
        Scope::Device(_, on) => scope_os(on),
        Scope::All | Scope::Peer(_) => None,
    }
}

/// Whether `one-mk buttons map N TO` would take it: it must read, and the
/// left and right buttons must keep clicking (not nothing, not a layer).
pub fn check_button(n: u32, to: &str, target: u8) -> Result<(), String> {
    let spec = ButtonRule { from: n.to_string(), to: To::One(to.into()), scope: Scope::All, app: None }.spec()?;
    let unavailable = match spec.what {
        What::To(Output::System(action)) => action.missing_on(target).next().map(|platform| (action, platform)),
        _ => None,
    };
    if matches!(spec.from.key, 0x110 | 0x111) && !spec.from.has_mods()
        && (unavailable.is_some() || matches!(spec.what, What::To(Output::None | Output::Layer(_))))
    {
        if let Some((action, platform)) = unavailable {
            return Err(primary_action_problem(if spec.from.key == 0x110 { "left" } else { "right" }, action, platform));
        }
        return Err(format!(
            "the {} button must keep clicking: it can't be mapped to {to} (map it to another button or a key instead)",
            if spec.from.key == 0x110 { "left" } else { "right" }
        ));
    }
    Ok(())
}

// ---------- keys from the window ----------

/// The evdev code of a key as egui reports it (its physical position when
/// egui knows it). `None` for keys with no evdev code here.
///
/// Keys egui **cannot** report, so they are chosen from the list instead
/// (see [`NOT_REPORTED`]): the keypad (egui folds it into the main keys:
/// Keypad 7 arrives as 7, Keypad Enter as Enter), the Japanese and Korean
/// keys, Fn (usually handled inside the keyboard), brightness, keyboard
/// light and microphone keys, F25 and up, and anything the desktop takes
/// before the window sees it (Super on GNOME, Print Screen, media keys).
pub fn evdev_of(key: egui::Key) -> Option<u16> {
    use egui::Key as K;
    let letters = [K::A, K::B, K::C, K::D, K::E, K::F, K::G, K::H, K::I, K::J, K::K, K::L, K::M, K::N, K::O, K::P, K::Q, K::R, K::S, K::T, K::U, K::V, K::W, K::X, K::Y, K::Z];
    const LETTER_CODES: [u16; 26] = [30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44];
    if let Some(i) = letters.iter().position(|k| *k == key) {
        return Some(LETTER_CODES[i]);
    }
    let digits = [K::Num1, K::Num2, K::Num3, K::Num4, K::Num5, K::Num6, K::Num7, K::Num8, K::Num9, K::Num0];
    if let Some(i) = digits.iter().position(|k| *k == key) {
        return Some(2 + i as u16);
    }
    let f = [K::F1, K::F2, K::F3, K::F4, K::F5, K::F6, K::F7, K::F8, K::F9, K::F10, K::F11, K::F12];
    const F_CODES: [u16; 12] = [59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88];
    if let Some(i) = f.iter().position(|k| *k == key) {
        return Some(F_CODES[i]);
    }
    let f13 = [K::F13, K::F14, K::F15, K::F16, K::F17, K::F18, K::F19, K::F20, K::F21, K::F22, K::F23, K::F24];
    if let Some(i) = f13.iter().position(|k| *k == key) {
        return Some(183 + i as u16);
    }
    Some(match key {
        K::Escape => 1,
        K::Minus => 12,
        K::Equals => 13,
        // Only the keypad's + key is reported as Plus.
        K::Plus => 78,
        K::Backspace => 14,
        K::Tab => 15,
        K::OpenBracket | K::OpenCurlyBracket => 26,
        K::CloseBracket | K::CloseCurlyBracket => 27,
        K::Enter => 28,
        K::ControlLeft => 29,
        K::Semicolon | K::Colon => 39,
        K::Quote => 40,
        K::Backtick => 41,
        K::ShiftLeft => 42,
        K::Backslash | K::Pipe => 43,
        K::Comma => 51,
        K::Period => 52,
        K::Slash | K::Questionmark => 53,
        K::ShiftRight => 54,
        K::AltLeft => 56,
        K::Space => 57,
        K::CapsLock => 58,
        K::NumLock => 69,
        K::ScrollLock => 70,
        K::IntlBackslash => 86,
        K::ControlRight => 97,
        K::PrintScreen => 99,
        K::AltRight => 100,
        K::Home => 102,
        K::ArrowUp => 103,
        K::PageUp => 104,
        K::ArrowLeft => 105,
        K::ArrowRight => 106,
        K::End => 107,
        K::ArrowDown => 108,
        K::PageDown => 109,
        K::Insert => 110,
        K::Delete => 111,
        K::AudioVolumeMute => 113,
        K::AudioVolumeDown => 114,
        K::AudioVolumeUp => 115,
        K::Power => 116,
        K::Pause => 119,
        K::SuperLeft => 125,
        K::SuperRight => 126,
        K::Menu => 127,
        K::BrowserStop => 128,
        K::Copy => 133,
        K::Paste => 135,
        K::Cut => 137,
        K::Help => 138,
        K::LaunchApp2 => 140,
        K::Sleep => 142,
        K::LaunchMail => 155,
        K::BrowserFavorites => 156,
        K::LaunchApp1 => 157,
        K::BrowserBack => 158,
        K::BrowserForward => 159,
        K::Eject => 161,
        K::MediaTrackNext => 163,
        K::MediaPlayPause => 164,
        K::MediaTrackPrevious => 165,
        K::MediaStop => 166,
        K::BrowserHome => 172,
        K::BrowserRefresh => 173,
        K::BrowserSearch => 217,
        K::Fn => 0x1d0,
        K::Exclamationmark => 2,
        _ => return None,
    })
}

/// Keys that can't be pressed into the window (egui can't tell them
/// apart, or the desktop keeps them): pick them from the list.
pub const NOT_REPORTED: &str = "the keypad, Fn, the Japanese and Korean keys, brightness and F25 and up can't be pressed here, \
     nor keys your desktop keeps for itself: choose those from the list";

/// One key going down or up in the window, as an evdev code, from an egui
/// event. Ctrl+C, Ctrl+X and Ctrl+V reach egui as Copy, Cut and Paste
/// (their key-up comes as a key): they are C, X and V going down.
pub fn event_key(e: &egui::Event) -> Option<(u16, bool, bool)> {
    match e {
        egui::Event::Key { key, physical_key, pressed, repeat, .. } => {
            let code = physical_key.and_then(evdev_of).or_else(|| evdev_of(*key))?;
            Some((code, *pressed, *repeat))
        }
        egui::Event::Copy => Some((46, true, false)),
        egui::Event::Cut => Some((45, true, false)),
        // Ctrl+V with a picture (or nothing text) on the clipboard comes as one.
        egui::Event::Paste(_) | egui::Event::PasteImage(_) => Some((47, true, false)),
        _ => None,
    }
}

/// "Press the key you want": a key with the modifiers held with it, or a
/// modifier pressed and let go alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capture {
    held: u8,
    lone: Option<u16>,
}

impl Capture {
    /// Feeds one key; the chord once it is known.
    pub fn key(&mut self, code: u16, down: bool) -> Option<Chord> {
        let bit = keynames::mod_bit(u32::from(code));
        match (bit, down) {
            (0, true) => Some(Chord { mods: self.held, key: code }),
            (0, false) => None,
            (b, true) => {
                self.held |= b;
                self.lone = Some(code);
                None
            }
            (b, false) => {
                self.held &= !b;
                (self.lone == Some(code)).then_some(Chord::key(code))
            }
        }
    }

    pub fn holding(&self) -> bool {
        self.held != 0
    }
}

// ---------- the "Test it" box ----------

/// One thing in a line of the test box: "Ctrl+A", "Esc", with a note
/// ("tap 85 ms").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    pub note: Option<String>,
}

/// Keys as they go down and up, turned into chips: a key with the
/// modifiers held, or a modifier pressed alone.
#[derive(Clone, Debug, Default)]
pub struct ChipLog {
    held: u8,
    down: Vec<u16>,
    lone: Option<u16>,
    pub chips: VecDeque<Chip>,
}

/// How many chips a line keeps.
const CHIPS: usize = 8;

impl ChipLog {
    pub fn push(&mut self, text: String, note: Option<String>) {
        if self.chips.len() == CHIPS {
            self.chips.pop_front();
        }
        self.chips.push_back(Chip { text, note });
    }

    pub fn key(&mut self, code: u16, down: bool) {
        let bit = keynames::mod_bit(u32::from(code));
        if down {
            if self.down.contains(&code) {
                return; // a repeat
            }
            self.down.push(code);
            if bit != 0 {
                self.held |= bit;
                self.lone = Some(code);
            } else {
                self.lone = None;
                let held = self.held;
                self.push(chord_text(&Chord { mods: held, key: code }), None);
            }
        } else {
            self.down.retain(|c| *c != code);
            if bit != 0 {
                self.held &= !bit;
                if self.lone.take() == Some(code) {
                    self.push(chord_text(&Chord::key(code)), None);
                }
            }
        }
    }

    /// The keys down now.
    pub fn down(&self) -> &[u16] {
        &self.down
    }

    /// Notes the last chip (how a tap/hold key was decided).
    fn note_last(&mut self, note: String) {
        if let Some(c) = self.chips.back_mut() {
            c.note = Some(note);
        }
    }

    pub fn clear(&mut self) {
        *self = ChipLog::default();
    }
}

/// The mapper running in the window: keys typed into the box go through
/// the rules exactly as a visit would send them, and nothing is sent or
/// run. It says how each tap/hold key was decided.
pub struct Tester {
    mapper: Mapper,
    tap: Duration,
    /// Ctrl and Cmd are swapped on what is sent, as a visit does it.
    swap: bool,
    pub pressed: ChipLog,
    pub sent: ChipLog,
    /// How the last tap/hold key was decided, in a sentence.
    pub note: Option<String>,
    /// A tap/hold key not decided yet, since when, and how long the rule
    /// for it waits (its own time, else the general one).
    pending: Option<(u16, Instant, Duration)>,
}

impl Tester {
    pub fn new(map: Keymap, tap: Duration) -> Tester {
        Tester { mapper: Mapper::new(map), tap, swap: false, pressed: ChipLog::default(), sent: ChipLog::default(), note: None, pending: None }
    }

    /// Whether what is sent has Ctrl and Cmd swapped (the visit's setting).
    pub fn set_swap(&mut self, swap: bool) {
        self.swap = swap;
    }

    /// The rules changed: use the new ones from the next key on.
    pub fn set_keymap(&mut self, map: Keymap, tap: Duration) {
        self.tap = tap;
        let mut sink = vec![];
        self.mapper.set_keymap(map, &mut |e| sink.push(e));
        self.emit(sink);
    }

    pub fn key(&mut self, code: u16, down: bool, now: Instant) {
        self.pressed.key(code, down);
        let was = self.pending;
        let mut out = vec![];
        self.mapper.key(u32::from(code), down, now, &mut |e| out.push(e));
        let pending_now = self.mapper.deadline().is_some();
        if let Some((k, since, limit)) = was
            && !pending_now
        {
            let ms = now.saturating_duration_since(since).as_millis();
            let name = key_label(u32::from(k));
            self.note = Some(if k == code && !down && now.saturating_duration_since(since) < limit {
                format!("{name} was tapped: let go after {ms} ms, under {} ms.", limit.as_millis())
            } else if k == code && !down {
                format!("{name} was held: {} ms passed.", limit.as_millis())
            } else {
                format!("{name} was held: another key went down after {ms} ms.")
            });
            self.pending = None;
        }
        if was.is_none() && pending_now {
            // The mapper knows how long this rule waits (it may have its own time).
            let limit = self.mapper.deadline().map_or(self.tap, |d| d.saturating_duration_since(now));
            self.pending = Some((code, now, limit));
            self.note = Some(format!("{}: a tap or a hold? Let go within {} ms for a tap.", key_label(u32::from(code)), limit.as_millis()));
        }
        let decided = was.is_some() && self.pending.is_none();
        self.emit(out);
        if decided && let Some((k, ..)) = was {
            let word = if self.note.as_deref().is_some_and(|n| n.contains("tapped")) { "tap" } else { "hold" };
            let _ = k;
            self.sent.note_last(word.into());
        }
    }

    /// When [`Tester::tick`] must run: only while a tap/hold key is undecided.
    pub fn deadline(&self) -> Option<Instant> {
        self.mapper.deadline()
    }

    /// Time passed: a tap/hold key held past its time is held.
    pub fn tick(&mut self, now: Instant) {
        let mut out = vec![];
        self.mapper.timer(now, &mut |e| out.push(e));
        if let Some((k, _, limit)) = self.pending
            && self.mapper.deadline().is_none()
        {
            self.note = Some(format!("{} was held: {} ms passed.", key_label(u32::from(k)), limit.as_millis()));
            self.pending = None;
            self.emit(out);
            self.sent.note_last("hold".into());
            return;
        }
        self.emit(out);
    }

    /// How far a pending key is towards being held, 0 to 1.
    pub fn progress(&self, now: Instant) -> Option<f32> {
        let (_, since, limit) = self.pending?;
        Some((now.saturating_duration_since(since).as_secs_f32() / limit.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0))
    }

    /// The box lost the keyboard: everything is let go of.
    pub fn release(&mut self) {
        let mut out = vec![];
        self.mapper.release_all(&mut |e| out.push(e));
        self.pending = None;
        self.pressed = ChipLog { chips: std::mem::take(&mut self.pressed.chips), ..Default::default() };
        self.sent = ChipLog { chips: std::mem::take(&mut self.sent.chips), ..Default::default() };
    }

    pub fn clear(&mut self) {
        self.release();
        self.pressed.clear();
        self.sent.clear();
        self.note = None;
    }

    fn emit(&mut self, out: Vec<Emit>) {
        for e in out {
            match e {
                Emit::Key { code, down } => {
                    let code = if self.swap { one_mk_proto::swap_ctrl_meta(code) } else { code };
                    self.sent.key(code as u16, down);
                }
                Emit::Action(a) => self.sent.push(title(&action_text(&a)), Some("not run here".into())),
            }
        }
    }
}

// ---------- the mouse ----------

/// The buttons on the drawn mouse: number, name.
pub const BUTTONS: [(u32, &str); 5] = [(1, "Left button"), (2, "Right button"), (3, "Wheel click"), (4, "Back (side)"), (5, "Forward (side)")];

/// What a button does when nothing changes it.
pub fn button_default(n: u32) -> &'static str {
    match n {
        1 => "Left click",
        2 => "Right click",
        3 => "Middle click",
        _ => "Unchanged",
    }
}

/// Mouse button outputs in the picker.
const BUTTON_CHOICES: [(&str, &str); 5] = [
    ("Left click", "button1"),
    ("Right click", "button2"),
    ("Middle click", "button3"),
    ("Back button", "button4"),
    ("Forward button", "button5"),
];

/// What a button does, in words.
pub fn button_text(o: &Output, target: u8) -> String {
    if let Output::Chord(c) = o
        && c.mods == 0
        && let Some(n) = keynames::button_number(u32::from(c.key))
        && n <= 3
    {
        return button_default(n).into();
    }
    match o {
        Output::Action(a) => title(&action_text(a)),
        Output::Nav(Nav::Back) => "Go back".into(),
        Output::Nav(Nav::Forward) => "Go forward".into(),
        Output::None => "Nothing".into(),
        Output::Chord(_) | Output::Sequence(_) => title(&out_text(o)),
        _ => out_text_for(o, target),
    }
}

/// What each drawn button does for `f`: `(number, words, changed)`.
pub fn buttons_shown(input: &Input, f: &Facts) -> Vec<(u32, String, bool)> {
    let specs = input.specs(f);
    BUTTONS
        .iter()
        .map(|(n, _)| {
            let code = keynames::button_code(*n).expect("1 to 5") as u16;
            match specs.iter().find(|(s, _)| s.from == Trigger::key(code) && s.layer.is_none()) {
                Some((RuleSpec { what: What::To(o), .. }, _)) => (*n, button_text(o, f.os), true),
                _ => (*n, button_default(*n).into(), false),
            }
        })
        .collect()
}

/// Whether `scope` has a rule of its own (for no app) for mouse button `n`.
pub fn button_has_own_rule(input: &Input, scope: &Scope, n: u32) -> bool {
    let Some(code) = keynames::button_code(n) else { return false };
    input.buttons.iter().any(|b| b.scope == *scope && b.app.is_none() && b.trigger().is_ok_and(|t| t == Trigger::key(code as u16)))
}

/// Whether any rule (any scope, any app) is for mouse button `n`.
pub fn button_has_rule(input: &Input, n: u32) -> bool {
    let Some(code) = keynames::button_code(n) else { return false };
    input.buttons.iter().any(|b| b.trigger().is_ok_and(|t| t.key == code as u16))
}

/// Pointer acceleration this computer can do: only the system's, for now.
pub const RAW_MOTION: bool = false;

/// Pointer speed and acceleration for `f`, field by field, the most
/// specific last (like `one-mk`).
pub fn pointer_shown(input: &Input, f: &Facts) -> (f64, String) {
    let mut rules: Vec<(u8, &PointerRule)> = input.pointer.iter().filter(|r| r.app.is_none()).filter_map(|r| Some((r.scope.rank(f)?, r))).collect();
    rules.sort_by_key(|(rank, _)| *rank);
    let (mut speed, mut accel) = (None, None);
    for (_, r) in rules {
        speed = r.speed.or(speed);
        accel = r.accel.clone().or(accel);
    }
    (speed.unwrap_or(1.0).clamp(0.1, 10.0), accel.unwrap_or_else(|| "system".into()))
}

/// Scrolling as it applies to `f` and a kind of device (`None`: the rules
/// for both).
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollShown {
    pub reverse_vertical: bool,
    pub reverse_horizontal: bool,
    pub natural: Option<String>,
    pub speed: f64,
    pub mode: String,
    pub shift_sideways: bool,
    pub alt_faster: bool,
}

pub fn scroll_shown(input: &Input, f: &Facts, device: Option<Device>) -> ScrollShown {
    let mut rules: Vec<(u8, usize, &ScrollRule)> = input
        .scroll
        .iter()
        .enumerate()
        .filter(|(_, r)| r.app.is_none())
        .filter_map(|(i, r)| Some((r.scope.rank(f)?, i, r)))
        .collect();
    rules.sort_by_key(|(rank, i, r)| (*rank, r.device.is_some(), *i));
    let mut s = ScrollShown { reverse_vertical: false, reverse_horizontal: false, natural: None, speed: 1.0, mode: "auto".into(), shift_sideways: false, alt_faster: false };
    // As `one-mk` resolves a kind: a plain speed resets the axes set less
    // specifically, and up and down use the vertical speed.
    let (mut speed, mut speed_y): (Option<f64>, Option<f64>) = (None, None);
    for (_, _, r) in rules.iter().filter(|(_, _, r)| r.device.is_none() || r.device == device) {
        s.reverse_vertical = r.reverse_vertical.unwrap_or(s.reverse_vertical);
        s.reverse_horizontal = r.reverse_horizontal.unwrap_or(s.reverse_horizontal);
        s.natural = r.natural.clone().or(s.natural);
        if r.speed.is_some() {
            speed_y = None;
        }
        speed = r.speed.or(speed);
        speed_y = r.speed_y.or(speed_y);
        s.mode = r.mode.clone().unwrap_or(s.mode);
        // Modifiers too belong to the kind a rule names: a wheel's Shift is not the touchpad's.
        for (m, b) in &r.modifiers {
            match m.as_str() {
                "shift" => s.shift_sideways = b == "horizontal",
                "alt" | "option" => s.alt_faster = b.starts_with('x') && b != "x1",
                _ => {}
            }
        }
    }
    s.speed = speed_y.or(speed).unwrap_or(1.0).clamp(0.1, 10.0);
    s
}

/// Autoscroll as it applies to `f`: whether it is on, its speed and the
/// button that starts it (rules for apps, devices and the primary buttons
/// don't count, as `one-mk` ignores them).
pub fn autoscroll_shown(input: &Input, f: &Facts) -> (bool, f64, String) {
    let mut rows: Vec<_> = input.autoscroll.iter().filter(|row| row.app.is_none())
        .filter(|row| !matches!(row.scope, Scope::Device(..)))
        .filter(|row| !row.button.as_deref().is_some_and(primary_mouse_button))
        .filter_map(|row| Some((row.scope.rank(f)?, row))).collect();
    rows.sort_by_key(|(rank, _)| *rank);
    let (mut enabled, mut speed, mut button) = (false, 1.0, "middle");
    for (_, row) in rows {
        enabled = row.enabled.unwrap_or(enabled);
        speed = row.speed.unwrap_or(speed);
        button = row.button.as_deref().unwrap_or(button);
    }
    (enabled, speed, button.to_owned())
}

/// Why "side-button desktop swipes" can't be set up for `f`: autoscroll is
/// on and starts with the back button, which a swipe on it would shadow
/// (the words `one-mk gestures add` gives for the same clash).
pub fn side_swipe_conflict(input: &Input, f: &Facts) -> Option<String> {
    let (enabled, _, button) = autoscroll_shown(input, f);
    let code = match button.trim().parse::<u32>() {
        Ok(n) => keynames::button_code(n),
        Err(_) => keynames::parse_key(&button).ok(),
    };
    (enabled && code == keynames::button_code(4))
        .then(|| "gesture button4 left is reserved by autoscroll; choose another button or turn autoscroll off".to_owned())
}

/// A problem from `one-mk keymap check` in the window's words:
/// `input.keymap.rule #3: ...` becomes `Key rule 3: ...`.
fn primary_action_problem(button: &str, action: SystemAction, platform: u8) -> String {
    let works: Vec<_> = SystemAction::TARGETS.into_iter()
        .filter(|&target| action.shortcut(target).is_some()).collect();
    format!("The {button} button must keep clicking. {} does nothing on {}. {}.",
        action.label(), scope_platform(platform), system_scope_hint(&works))
}

pub fn system_action_problem(p: &str) -> Option<String> {
    if let Some((before, platform)) = p.split_once(" has no shortcut for --os ")
        && let Some((button, action)) = before.split_once(" button must keep clicking: ")
        && let Some(button) = button.split_whitespace().last()
        && let Ok(action) = action.parse()
        && let Some(platform) = parse_os(platform.trim())
    {
        return Some(primary_action_problem(button, action, platform));
    }
    p.contains("unknown system action ").then(|| "Unknown system action. Choose an action from the list.".into())
}

pub fn plain_problem(p: &str) -> String {
    if let Some(message) = system_action_problem(p) {
        return message;
    }
    if let Some((prefix, _)) = p.split_once(" is not a key name")
        && let Some(name) = prefix.rsplit('"').nth(1)
    {
        let folded = remap::fold_action_search(name);
        let suggestion = keyboard(true, false).iter().map(|cap| key_label(u32::from(cap.code))).find(|label| {
            let candidate = remap::fold_action_search(label);
            candidate.char_indices().any(|(index, character)| {
                let mut shortened = candidate.clone();
                shortened.replace_range(index..index + character.len_utf8(), "");
                shortened == folded
            })
        });
        return match suggestion {
            Some(label) => format!("'{name}' isn't a key. Did you mean {label}?"),
            None => format!("'{name}' isn't a key. Choose a key from the list."),
        };
    }
    let mut out = p.to_owned();
    for (table, word) in [("input.keymap.rule #", "Key rule "), ("input.button #", "Button rule "), ("input.pointer #", "Pointer setting "), ("input.scroll #", "Scroll setting ")] {
        out = out.replace(table, word);
    }
    title(&out)
}

// ---------- the list to choose from ----------

/// Sections of the action picker. Suggested and Mac each show a subset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChoiceGroup {
    #[default]
    Suggested,
    Desktop,
    Mac,
    Media,
    Editing,
    Mouse,
    OneMk,
    Keys,
}

impl ChoiceGroup {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Suggested => "Suggested",
            Self::Desktop => "Desktop & windows",
            Self::Mac => "Mac",
            Self::Media => "Media & volume",
            Self::Editing => "Editing",
            Self::Mouse => "Mouse buttons",
            Self::OneMk => "oneMK",
            Self::Keys => "All keys",
        }
    }
}

/// One thing a key or button can become, in the searchable list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub detail: String,
    /// As written in a rule (`esc`, `switch next`).
    pub value: String,
    pub group: ChoiceGroup,
    pub suggested: bool,
    pub mac: bool,
    pub available: bool,
    pub disabled_hint: Option<String>,
    /// Every name it answers to, folded for searching.
    names: Vec<String>,
    words: Vec<String>,
}

impl Choice {
    fn new(label: &str, detail: &str, value: &str, group: ChoiceGroup, aliases: &[&str]) -> Self {
        let names = [label, value].into_iter().chain(aliases.iter().copied()).map(remap::fold_action_search).collect();
        let words = label.split(|ch: char| !ch.is_alphanumeric()).filter(|word| !word.is_empty()).map(remap::fold_action_search).collect();
        Self { label: label.into(), detail: detail.into(), value: value.into(), group, suggested: false, mac: false, available: true, disabled_hint: None, names, words }
    }
}

pub fn platform_name(target: u8) -> &'static str {
    match target {
        os::MACOS => "Mac",
        os::WINDOWS => "Windows",
        os::LINUX => "Linux",
        _ => "an unknown OS",
    }
}

fn scope_platform(platform: u8) -> &'static str {
    match platform {
        os::MACOS => "Macs",
        os::WINDOWS => "Windows PCs",
        os::LINUX => "Linux",
        _ => "other computers",
    }
}

fn platform_list(platforms: &[u8], name: impl Fn(u8) -> &'static str) -> String {
    let names: Vec<_> = platforms.iter().map(|&platform| name(platform)).collect();
    match names.as_slice() {
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b] => format!("{a} and {b}"),
        _ => names.join(""),
    }
}

fn system_scope_hint(works: &[u8]) -> String {
    match works {
        [a] => format!("Choose For: Only {} to use it", os_plural(*a)),
        [a, b] => format!("Choose For: Only {} or Only {} to use it", os_plural(*a), os_plural(*b)),
        _ => "Choose a different action".into(),
    }
}

pub fn system_note(what: &What, target: u8) -> Option<String> {
    let warning = |output: &Output| {
        let Output::System(action) = output else { return None };
        let missing: Vec<_> = action.missing_on(target).collect();
        if missing.is_empty() { return None; }
        let works: Vec<_> = SystemAction::TARGETS.into_iter()
            .filter(|&platform| action.shortcut(platform).is_some()).collect();
        let name = if *action == SystemAction::AppWindows && works.contains(&os::MACOS) {
            action.label_for(os::MACOS)
        } else {
            action.label_for(target)
        };
        Some((name, missing, works))
    };
    let (name, missing, works) = match what {
        What::To(output) => warning(output)?,
        What::TapHold { tap, hold, .. } => match (warning(tap), warning(hold)) {
            (Some(a), Some(b)) if a != b => return Some(format!(
                "{} does nothing on {} when tapped. {} does nothing on {} when held.",
                a.0, platform_list(&a.1, scope_platform), b.0, platform_list(&b.1, scope_platform),
            )),
            (Some(note), _) | (_, Some(note)) => note,
            _ => return None,
        },
    };
    Some(format!("{name} does nothing on {}. {}.",
        platform_list(&missing, scope_platform), system_scope_hint(&works)))
}

pub fn button_rule_note(button: u32, target: u8) -> Option<String> {
    remap::extra_button_unavailable(button, target).then(|| {
        format!("Button {button} isn't forwarded on {}.", scope_platform(target))
    })
}

pub fn picker_available(choice: &Choice, primary: bool, target: u8) -> bool {
    choice.available && (!primary || (choice.value != "none"
        && !choice.value.parse::<SystemAction>().is_ok_and(|action| action.missing_on(target).next().is_some())))
}

fn shortcut_text(chord: Chord, target: u8) -> String {
    let (alt, meta) = match target {
        os::MACOS => ("Option", "Cmd"),
        os::WINDOWS => ("Alt", "Win"),
        _ => ("Alt", "Super"),
    };
    let mut words = Vec::new();
    for (i, name) in ["Ctrl", "Shift", alt, meta].into_iter().enumerate() {
        if chord.mods & (1 << i) != 0 {
            words.push(name.to_owned());
        }
    }
    words.push(match chord.key {
        103 => "↑".into(),
        105 => "←".into(),
        106 => "→".into(),
        108 => "↓".into(),
        125 => meta.into(),
        52 => ".".into(),
        key => key_label(u32::from(key)),
    });
    words.join("+")
}

fn system_choice(action: SystemAction, target: u8) -> Choice {
    use ChoiceGroup as Group;
    use SystemAction::*;
    let desktop = matches!(action, NextDesktop | PreviousDesktop | Overview | AppWindows | ShowDesktop);
    let group = match action {
        Copy | Cut | Paste | Undo | Redo | SelectAll | Find | Save | NewTab | CloseTab | NextTab | PreviousTab => Group::Editing,
        _ => Group::Desktop,
    };
    let describe = |platform| {
        let name = platform_name(platform);
        match action.shortcut(platform) {
            Some(chord) => format!("{} on {name}", shortcut_text(chord, platform)),
            None => format!("Not on {name}"),
        }
    };
    let (works, missing): (Vec<_>, Vec<_>) = SystemAction::TARGETS.into_iter()
        .partition(|&platform| action.shortcut(platform).is_some());
    let detail = if target == os::UNKNOWN && !missing.is_empty() {
        format!("{} · nothing on {}", platform_list(&works, platform_name), platform_list(&missing, platform_name))
    } else if target == os::UNKNOWN {
        SystemAction::TARGETS.map(describe).join("  ·  ")
    } else {
        describe(target)
    };
    let detail = match (target, action) {
        (os::MACOS | os::UNKNOWN, _) if desktop && (target != os::UNKNOWN || missing.is_empty()) => format!("{detail} · Enable Mission Control shortcuts on Mac"),
        (os::LINUX, ScreenshotTool) => format!("{detail} · Opens the screenshot UI on GNOME 42+"),
        (os::WINDOWS, ScreenshotTool) => format!("{detail} · Opens the Snipping Tool"),
        (os::LINUX, ShowDesktop) => format!("{detail} · Bind this in Keyboard Shortcuts"),
        (os::LINUX | os::WINDOWS, FullScreen) => format!("{detail} · In apps that use F11"),
        _ => detail,
    };
    let mut choice = Choice::new(action.label_for(target), &detail, &action.to_string(), group, &[]);
    choice.names.extend(action.search_names().map(remap::fold_action_search));
    choice.available = if SystemAction::TARGETS.contains(&target) {
        action.shortcut(target).is_some()
    } else {
        !works.is_empty()
    };
    if !missing.is_empty() {
        choice.disabled_hint = Some(system_scope_hint(&works));
    }
    choice.suggested = desktop || matches!(action, Search | ScreenshotArea | LockScreen | Copy | Paste);
    choice.mac = action.shortcut(os::MACOS).is_some()
        && (desktop || matches!(action, Search | ScreenshotTool | ForceQuit | Emoji | HideApp | QuitApp));
    choice
}

pub fn choices(buttons: bool, target: u8) -> Vec<Choice> {
    use ChoiceGroup as Group;
    let mut out = Vec::new();
    if buttons {
        out.extend(BUTTON_CHOICES.iter().map(|(label, value)| {
            Choice::new(label, "Mouse button", value, Group::Mouse, &[])
        }));
    }
    out.extend(SystemAction::ALL.iter().copied().filter(|&action| {
        action != SystemAction::ScreenshotArea || !matches!(target, os::WINDOWS | os::LINUX)
    }).map(|action| system_choice(action, target)));
    for (label, value) in [
        ("Switch to the next computer", "switch next"),
        ("Come back to this computer", "switch here"),
        ("Keep the pointer here (on and off)", "lock-pointer"),
        ("Lock every screen", "lock-all"),
        ("Nothing", "none"),
    ] {
        let aliases: &[&str] = match value {
            "none" => &["off", "disable"],
            "lock-pointer" | "lock-all" => &["lock"],
            _ => &[],
        };
        let mut choice = Choice::new(label, "oneMK action", value, Group::OneMk, aliases);
        choice.suggested = value == "switch next";
        out.push(choice);
    }
    for (label, value) in [("Back (browser)", "back"), ("Forward (browser)", "forward")] {
        let detail = match target {
            os::MACOS => if value == "back" { "Cmd+[ on Mac" } else { "Cmd+] on Mac" },
            os::LINUX | os::WINDOWS => if value == "back" { "Alt+←" } else { "Alt+→" },
            _ => if value == "back" { "Cmd+[ on Mac · Alt+← on Windows and Linux" } else { "Cmd+] on Mac · Alt+→ on Windows and Linux" },
        };
        let mut choice = Choice::new(label, detail, value, Group::Desktop, &[]);
        choice.suggested = true;
        out.push(choice);
    }
    for (code, name, names) in keynames::all_names() {
        if keynames::is_button(code) {
            continue;
        }
        let media = one_mk_proto::mediakeys::is_media(code);
        let (group, detail) = if media { (Group::Media, "Media key") } else { (Group::Keys, "") };
        let mut choice = Choice::new(&key_label(code), detail, name, group, names);
        choice.suggested = matches!(code, 1 | 28 | 57 | 113..=115 | 164);
        out.push(choice);
    }
    out
}

/// Rank exact names, prefixes, partial names and misspellings in order.
pub fn search<'a>(all: &'a [Choice], query: &str) -> Vec<&'a Choice> {
    let q = remap::fold_action_search(query.trim());
    if q.is_empty() {
        return all.iter().collect();
    }
    let mut hits: Vec<_> = all.iter().filter_map(|choice| {
        let rank = remap::action_search_rank(&choice.names, &choice.words, &q)?;
        Some((rank, choice))
    }).collect();
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter().map(|(_, choice)| choice).collect()
}

// ---------- speeds ----------

/// Where a speed sits on a slider from 0.1 to 10, evenly by ratio, so 1x
/// is in the middle.
pub fn speed_to_t(x: f64) -> f32 {
    ((x.clamp(0.1, 10.0).ln() - 0.1f64.ln()) / (10f64.ln() - 0.1f64.ln())) as f32
}

/// The speed at a point of the slider, rounded to a step people would
/// type (0.05 below 2, 0.1 below 5, 0.5 above).
pub fn t_to_speed(t: f32) -> f64 {
    let x = (0.1f64.ln() + f64::from(t.clamp(0.0, 1.0)) * (10f64.ln() - 0.1f64.ln())).exp();
    let step = if x < 2.0 { 0.05 } else if x < 5.0 { 0.1 } else { 0.5 };
    ((x / step).round() * step).clamp(0.1, 10.0)
}

/// "1.5x" typed: a speed from 0.1 to 10, or what to type instead.
pub fn parse_speed(s: &str) -> Result<f64, String> {
    let t = s.trim().trim_end_matches(['x', 'X', '×']).trim().replace(',', ".");
    match t.parse::<f64>() {
        Ok(x) if x.is_finite() && (0.1..=10.0).contains(&x) => Ok(x),
        _ => Err("Type a speed from 0.1 to 10, like 1.5.".into()),
    }
}

pub fn speed_text(x: f64) -> String {
    format!("{}×", trim_num(x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::read_toml;

    /// What `one-mk keymap|pointer|scroll|buttons` wrote (from a real run).
    const WRITTEN: &str = r#"[hotkeys]
enabled = true

[input.keymap]
tap_ms = 180

[[input.keymap.rule]]
from = "capslock"
tap = "esc"
hold = "lctrl"

[[input.keymap.rule]]
from = "lalt"
to = "lmeta"

[input.keymap.rule.scope]
os = "mac"

[[input.keymap.rule]]
from = "ralt"
to = "layer:nav"

[[input.keymap.rule]]
from = "h"
to = "left"
layer = "nav"

[[input.keymap.rule]]
from = "f2"
to = [
    "ctrl+a",
    "ctrl+c",
]

[[input.keymap.rule]]
from = "f13"
to = "switch next"
scope = { peer = "Studio PC" }

[[input.pointer]]
speed = 1.5

[input.pointer.scope]
os = "windows"

[[input.scroll]]
natural = "auto"

[input.scroll.modifiers]
alt = "x5"
shift = "horizontal"

[[input.scroll]]
device = "wheel"
speed = 2.0
mode = "discrete"
scope = { os = "linux" }

[[input.button]]
from = 4
to = "back"

[[input.button]]
from = 3
to = "lock-all"

[input.button.scope]
os = "mac"
"#;

    fn input() -> Input {
        Input::read(&read_toml(WRITTEN))
    }

    fn studio() -> Vec<Peer> {
        vec![Peer { name: "Studio PC".into(), id: "3333-4444".into(), os: os::WINDOWS }]
    }

    #[test]
    fn reads_the_input_tables_as_one_mk_writes_them() {
        let i = input();
        assert_eq!(i.tap_ms, Some(180));
        assert_eq!(i.keys.len(), 6);
        assert_eq!(i.keys[0], KeyRule::tap_hold("capslock", "esc", "lctrl"));
        assert_eq!(i.keys[1].scope, Scope::Os(os::MACOS));
        assert_eq!(i.keys[2].scope, Scope::All, "a table inside one repeat isn't the next one's");
        assert_eq!(i.keys[3].layer.as_deref(), Some("nav"));
        assert_eq!(i.keys[4].to, Some(To::Many(vec!["ctrl+a".into(), "ctrl+c".into()])));
        assert_eq!(i.keys[5].scope, Scope::Peer("Studio PC".into()));
        assert_eq!(i.pointer, vec![PointerRule { speed: Some(1.5), accel: None, scope: Scope::Os(os::WINDOWS), app: None }]);
        assert_eq!(i.scroll[0].modifiers, vec![("alt".into(), "x5".into()), ("shift".into(), "horizontal".into())]);
        assert_eq!((i.scroll[1].device, i.scroll[1].speed, i.scroll[1].scope.clone()), (Some(Device::Wheel), Some(2.0), Scope::Os(os::LINUX)));
        assert_eq!(i.buttons[0], ButtonRule { from: "4".into(), to: To::One("back".into()), scope: Scope::All, app: None });
        assert_eq!(i.buttons[1].scope, Scope::Os(os::MACOS));
        assert_eq!(Input::read(&read_toml("")), Input::default());
    }

    #[test]
    fn mouse_modes_read_scopes_and_describe_actions_in_plain_language() {
        let input = Input::read(&read_toml("[input.autoscroll]\nenabled=true\nbutton='middle'\n[[input.autoscroll.rule]]\nspeed=2.0\nscope={os='windows'}\n[[input.gesture]]\nbutton='button4'\ndirection='left'\nto='back'\napp='code*'\nscope={peer='Studio PC'}\n"));
        assert_eq!(input.autoscroll.len(), 2);
        assert_eq!(input.autoscroll[0].button.as_deref(), Some("middle"));
        assert_eq!(input.autoscroll[1].scope, Scope::Os(os::WINDOWS));
        assert_eq!(input.autoscroll[1].speed, Some(2.0));
        assert_eq!(input.gestures[0].scope, Scope::Peer("Studio PC".into()));
        assert_eq!(input.gestures[0].app.as_deref(), Some("code*"));
        assert_eq!(input.gestures[0].to.describe(os::UNKNOWN), "go back");
        assert_eq!(mouse_button_label("button4"), "the back button");
    }

    #[test]
    fn inactive_local_rules_do_not_appear_as_global_in_the_remote_editor() {
        for scope in ["scope = { local = true }", "[input.keymap.rule.scope]\nlocal = true"] {
            let text = format!("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\n{scope}\n");
            assert!(Input::read(&read_toml(&text)).keys.is_empty());
        }
        let text = "[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nscope = { local = true }\n[[input.keymap.rule]]\nfrom = \"a\"\nto = \"c\"\n";
        let input = Input::read(&read_toml(text));
        assert_eq!(input.keys, [KeyRule::to("a", "c")]);
    }

    #[test]
    fn local_switch_and_rule_badges_follow_preferences_and_os_scope() {
        assert!(!Input::read(&read_toml("")).local_rules);
        assert!(Input::read(&read_toml("[input]\nlocal_rules=true\n")).local_rules);
        assert!(Scope::All.includes_local_os(os::MACOS));
        assert!(Scope::Os(os::MACOS).includes_local_os(os::MACOS));
        assert!(!Scope::Os(os::WINDOWS).includes_local_os(os::MACOS));
        assert!(!Scope::Peer("Studio PC".into()).includes_local_os(os::MACOS));
        assert!(Scope::Device("keyboard".into(), Box::new(Scope::All)).includes_local_os(os::MACOS));
        assert!(!Scope::Device("keyboard".into(), Box::new(Scope::Os(os::WINDOWS))).includes_local_os(os::MACOS));
    }

    #[test]
    fn the_most_specific_scope_wins() {
        let mut i = input();
        i.keys.push(KeyRule { scope: Scope::Peer("studio pc".into()), ..KeyRule::to("capslock", "esc") });
        let first = |f: &Facts| i.specs(f).into_iter().find(|(s, _)| s.from.key == 58).map(|(s, at)| (s.what, at));
        let all = Facts::of(&Scope::All, &studio());
        assert!(matches!(first(&all), Some((What::TapHold { .. }, Ok(0)))));
        let pc = Facts::of(&Scope::Peer("Studio PC".into()), &studio());
        assert_eq!(pc.os, os::WINDOWS);
        assert_eq!(first(&pc).unwrap().1, Ok(6));
        // Rules for one OS only reach that OS; layers and buttons come along.
        let mac = Facts::of(&Scope::Os(os::MACOS), &[]);
        assert!(i.specs(&mac).iter().any(|(s, _)| s.from.key == 56));
        assert!(!i.specs(&all).iter().any(|(s, _)| s.from.key == 56));
        assert!(i.specs(&mac).iter().any(|(_, at)| *at == Err(1)));
        assert!(i.keymap(&all).is_ok());
    }

    #[test]
    fn rules_read_as_sentences() {
        let i = input();
        let s = |k: usize| sentence(&i, &i.keys[k]).unwrap();
        assert_eq!(s(0), format!("Caps Lock → Esc when tapped, {} when held", ctrl_word()));
        assert_eq!(s(1), format!("Left {} → {}", alt_word(), meta_word()));
        assert_eq!(s(2), format!("Right {} → hold it for the “nav” keys", alt_word()));
        assert_eq!(s(3), format!("Right {} + H → Left", alt_word()));
        assert_eq!(s(4), format!("F2 → types {}+A, then {}+C", ctrl_word(), ctrl_word()));
        assert_eq!(s(5), "F13 → switches to the next computer");
        assert_eq!(sentence(&i, &KeyRule::to("ctrl+h", "backspace")).unwrap(), format!("{}+H → Backspace", ctrl_word()));
        assert_eq!(sentence(&i, &KeyRule::to("rctrl", "none")).unwrap(), format!("Right {} → does nothing", ctrl_word()));
        assert_eq!(sentence(&i, &KeyRule::to("a", "ctrl+shift+esc")).unwrap(), format!("A → {}+Shift+Esc", ctrl_word()));
        let timed = KeyRule { timeout_ms: Some(300), ..KeyRule::tap_hold("space", "space", "layer:nav") };
        assert_eq!(sentence(&i, &timed).unwrap(), "Space → Space when tapped, the “nav” keys when held (after 300 ms)");
        let orphan = KeyRule { layer: Some("fn".into()), ..KeyRule::to("j", "down") };
        assert_eq!(sentence(&i, &orphan).unwrap(), "In “fn”: J → Down");
        assert!(sentence(&i, &KeyRule::to("capslok", "esc")).unwrap_err().contains("\"capslok\" is not a key name"));
        let in_app = KeyRule { app: Some("Code".into()), ..KeyRule::to("ctrl+space", "f13") };
        assert_eq!(sentence(&i, &in_app).unwrap(), format!("{}+Space → F13, only in Code", ctrl_word()));
        let but = KeyRule { app: Some("!code".into()), ..KeyRule::to("a", "b") };
        assert_eq!(sentence(&i, &but).unwrap(), "A → B, in every app except code");
        assert_eq!(add_args(&in_app), ["keymap", "add", "--app=Code", "--", "ctrl+space", "f13"]);
        assert_eq!(remove_args(&in_app), ["keymap", "remove", "--app=Code", "--", "ctrl+space"]);
        // A rule for an app is a rule of its own.
        assert!(!in_app.same_place(&KeyRule::to("ctrl+space", "f12")));
        assert!(in_app.same_place(&KeyRule { app: Some(" code ".into()), ..KeyRule::to("ctrl+space", "f12") }));
        assert_eq!(as_written(&KeyRule::to("capslok", "esc")), "capslok → esc");
        assert_eq!(cap_target(&i.keys[0].spec().unwrap().what, os::UNKNOWN), "Esc/Ctrl");
        assert_eq!(cap_target(&What::To("ctrl+shift+esc".parse().unwrap()), os::UNKNOWN), "+Esc");
    }

    #[test]
    fn the_drawn_keyboard_has_every_main_key_once_and_no_overlaps() {
        for (keypad, mac) in [(false, false), (true, false), (false, true)] {
            let caps = keyboard(keypad, mac);
            let (w, h) = keyboard_size(keypad);
            for (n, a) in caps.iter().enumerate() {
                assert!(a.x >= 0.0 && a.x + a.w <= w + 1e-4 && a.y + a.h <= h + 1e-4, "{a:?} outside");
                for b in &caps[n + 1..] {
                    assert_ne!(a.code, b.code, "{} twice", a.code);
                    let apart = a.x + a.w <= b.x + 1e-4 || b.x + b.w <= a.x + 1e-4 || a.y + a.h <= b.y + 1e-4 || b.y + b.h <= a.y + 1e-4;
                    assert!(apart, "{a:?} overlaps {b:?}");
                }
            }
            for code in (1..=54).chain(56..=58).chain([59, 68, 87, 88, 97, 100, 103, 105, 106, 108, 125, 126]) {
                assert!(caps.iter().any(|c| c.code == code), "missing {code}");
            }
            assert_eq!(caps.iter().any(|c| c.code == 76), keypad);
            // The main rows end flush at 15 units.
            for y in [1.5, 2.5, 3.5, 4.5, 5.5] {
                let end = caps.iter().filter(|c| c.y == y && c.x < 15.0).map(|c| c.x + c.w).fold(0.0, f32::max);
                assert!((end - 15.0).abs() < 1e-4, "row {y} ends at {end}");
            }
        }
        let caps = keyboard(false, false);
        let at = |code: u16| caps.iter().position(|c| c.code == code).unwrap();
        let code = |i: Option<usize>| i.map(|i| caps[i].code);
        assert_eq!(code(neighbour(&caps, at(30), 1, 0)), Some(31), "A → S");
        assert_eq!(code(neighbour(&caps, at(30), -1, 0)), Some(58), "A ← Caps Lock");
        assert_eq!(code(neighbour(&caps, at(58), -1, 0)), None, "nothing left of Caps Lock");
        assert_eq!(code(neighbour(&caps, at(30), 0, -1)), Some(16), "A ↑ Q");
        assert_eq!(code(neighbour(&caps, at(16), 0, -1)), Some(3), "Q ↑ 2 (under it, offset half a key)");
        assert_eq!(code(neighbour(&caps, at(48), 0, 1)), Some(57), "B ↓ Space");
        assert_eq!(code(neighbour(&caps, at(1), 0, 1)), Some(41), "Esc ↓ `");
        assert_eq!(code(neighbour(&caps, at(103), 0, 1)), Some(108), "Up ↓ Down");
        assert_eq!(code(neighbour(&caps, at(97), 0, 1)), None, "nothing below the bottom row");
        assert_eq!(cap_label(58), "Caps");
        assert_eq!(cap_label(14), "Bksp");
        assert_eq!(cap_label(125), if cfg!(target_os = "macos") { "Cmd" } else if cfg!(windows) { "Win" } else { "Sup" });
        assert_eq!([102, 104, 109, 69, 127].map(cap_label), ["Home", "PgUp", "PgDn", "Num", "Mnu"]);
        assert_eq!([12, 13, 26, 27, 43, 39, 40, 51, 52, 53].map(cap_label), ["-", "=", "[", "]", "\\", ";", "'", ",", ".", "/"]);
        assert_eq!(cap_label(30), "A");
        assert_eq!(cap_label(76), "5");
        assert_eq!(key_label(183), "F13");
        assert_eq!(key_label(115), "Volume up");
        assert_eq!(key_label(77), "Keypad 6");
    }

    #[test]
    fn quick_setups_expand_to_commands_and_show_when_on() {
        let p = presets(false);
        assert_eq!(p.iter().map(|p| p.id).collect::<Vec<_>>(), ["caps", "alt-meta", "ctrl-cmd", "arrows"]);
        assert_eq!(p[2].label, "Swap Ctrl and Cmd (for Mac keyboards)");
        let mac = Scope::Os(os::MACOS);
        let on = preset_commands(&p[0], &mac, true);
        assert_eq!(on, vec![vec!["keymap", "add", "--tap=esc", "--hold=lctrl", "--os=mac", "--", "capslock"]]);
        let arrows = preset_commands(&p[3], &Scope::All, true);
        assert_eq!(arrows.len(), 5);
        assert_eq!(arrows[0], ["keymap", "add", "--", "ralt", "layer:nav"]);
        assert_eq!(arrows[1], ["keymap", "add", "--layer=nav", "--", "h", "left"]);
        let off = preset_commands(&p[3], &Scope::Peer("-odd name".into()), false);
        assert_eq!(off[1], ["keymap", "remove", "--layer=nav", "--peer=-odd name", "--", "h"]);
        let i = input();
        assert!(preset_on(&i, &p[0], &Scope::All));
        assert!(!preset_on(&i, &p[0], &mac), "that rule is for every computer, not only Macs");
        assert!(!preset_on(&i, &p[3], &Scope::All), "only H is there");
        let mut full = i.clone();
        full.keys.extend(p[3].rules.iter().cloned());
        assert!(preset_on(&full, &p[3], &Scope::All));
        // Every quick setup is a valid keymap on its own and together.
        let mut all = Input::default();
        for preset in &p {
            for r in &preset.rules {
                check_add(&all, r).unwrap_or_else(|e| panic!("{}: {e}", preset.id));
                all.keys.retain(|x| !x.same_place(r));
                all.keys.push(r.clone());
            }
        }
    }

    #[test]
    fn checks_say_what_one_mk_keymap_add_would_say() {
        let i = input();
        assert_eq!(check_add(&i, &KeyRule::to("capslock", "esc")), Ok(()));
        assert!(check_add(&i, &KeyRule::to("capslok", "esc")).unwrap_err().starts_with("from: \"capslok\" is not a key name"));
        assert!(check_add(&i, &KeyRule::to("a", "ctrl+")).unwrap_err().contains("a key is missing after the last `+`"));
        assert!(check_add(&i, &KeyRule::to("button4", "esc")).unwrap_err().contains("mouse button: change it under Mouse"));
        assert!(check_add(&i, &KeyRule::tap_hold("ctrl+a", "a", "b")).is_err(), "a tap/hold key can't have modifiers");
        assert!(check_add(&i, &KeyRule::tap_hold("a", "layer:x", "b")).is_err(), "a tap can't be a layer");
        assert!(check_add(&i, &KeyRule { tap: Some("esc".into()), ..KeyRule::to("a", "b") }).unwrap_err().contains("not both"));
        // Nine layers are one too many, counted with the rules already there.
        let mut many = Input::default();
        for n in 0..8 {
            many.keys.push(KeyRule::to(&format!("f{}", n + 1), &format!("layer:l{n}")));
        }
        let e = check_add(&many, &KeyRule::to("f9", "layer:l8")).unwrap_err();
        assert!(e.contains("at most 8 layers"), "{e}");
        assert_eq!(check_button(4, "back", os::UNKNOWN), Ok(()));
        assert!(check_button(4, "switch", os::UNKNOWN).unwrap_err().contains("needs `next`"));
        assert!(check_button(9, "back", os::UNKNOWN).unwrap_err().contains("numbered 1 to 8"));
    }

    #[test]
    fn commands_for_buttons_pointer_and_scrolling() {
        assert_eq!(button_args(4, Some("back"), &Scope::All), ["buttons", "map", "--", "4", "back"]);
        assert_eq!(button_args(3, None, &Scope::Os(os::LINUX)), ["buttons", "unmap", "--os=linux", "--", "3"]);
        assert_eq!(pointer_args(Some(1.25), None, &Scope::All), ["pointer", "--speed=1.25"]);
        assert_eq!(pointer_args(None, Some("system"), &Scope::Peer("Studio PC".into())), ["pointer", "--accel=system", "--peer=Studio PC"]);
        let i = input();
        let one = |c: ScrollChange, d: Option<Device>| scroll_commands(&i, &Scope::All, d, &c);
        assert_eq!(one(ScrollChange::ReverseVertical(true), Some(Device::Touchpad)), [["scroll", "--reverse-vertical=on", "--kind=touchpad"]]);
        assert_eq!(one(ScrollChange::Modifier("shift", "off"), None), [["scroll", "--modifier=shift=off"]]);
        assert_eq!(one(ScrollChange::Speed(2.0), None), [["scroll", "--speed=2"]]);
        // Back to "as here": only natural is forgotten, in one command.
        assert_eq!(one(ScrollChange::Natural(None), None), [["scroll", "--clear-natural"]]);
        assert!(one(ScrollChange::Natural(None), Some(Device::Wheel)).is_empty(), "nothing to undo");
    }

    #[test]
    fn mouse_settings_resolve_per_computer() {
        let i = input();
        let all = Facts::of(&Scope::All, &[]);
        let win = Facts::of(&Scope::Os(os::WINDOWS), &[]);
        assert_eq!(pointer_shown(&i, &all), (1.0, "system".to_owned()));
        assert_eq!(pointer_shown(&i, &win).0, 1.5);
        let s = scroll_shown(&i, &all, None);
        assert_eq!((s.natural.as_deref(), s.shift_sideways, s.alt_faster, s.speed), (Some("auto"), true, true, 1.0));
        let linux = Facts::of(&Scope::Os(os::LINUX), &[]);
        assert_eq!(scroll_shown(&i, &linux, Some(Device::Wheel)).mode, "discrete");
        assert_eq!(scroll_shown(&i, &linux, Some(Device::Touchpad)).mode, "auto", "wheels only");
        let b = buttons_shown(&i, &all);
        assert_eq!(b[3], (4, "Go back".to_owned(), true));
        assert_eq!(b[2], (3, "Middle click".to_owned(), false));
        assert_eq!(buttons_shown(&i, &Facts::of(&Scope::Os(os::MACOS), &[]))[2].1, "Locks every screen");
        assert_eq!(button_text(&"button2".parse().unwrap(), os::UNKNOWN), "Right click");
        assert_eq!(button_text(&"ctrl+c".parse().unwrap(), os::UNKNOWN), format!("{}+C", ctrl_word()));
        for (_, to) in BUTTON_CHOICES {
            assert_eq!(check_button(5, to, os::UNKNOWN), Ok(()), "{to}");
        }
    }

    #[test]
    fn speeds_slide_evenly_by_ratio_and_parse_when_typed() {
        assert!((speed_to_t(1.0) - 0.5).abs() < 1e-6);
        assert_eq!(speed_to_t(0.1), 0.0);
        assert_eq!(speed_to_t(50.0), 1.0);
        assert_eq!(t_to_speed(0.5), 1.0);
        assert_eq!(t_to_speed(1.0), 10.0);
        assert_eq!(t_to_speed(0.0), 0.1);
        for x in [0.1, 0.5, 1.0, 1.5, 3.0, 7.5] {
            assert!((t_to_speed(speed_to_t(x)) - x).abs() < 1e-9, "{x}");
        }
        assert_eq!(parse_speed("1.5x"), Ok(1.5));
        assert_eq!(parse_speed(" 2,5 × "), Ok(2.5));
        assert!(parse_speed("11").is_err() && parse_speed("fast").is_err() && parse_speed("0").is_err());
        assert_eq!(speed_text(1.5), "1.5×");
        assert_eq!(speed_text(2.0), "2×");
    }

    #[test]
    fn egui_keys_become_evdev_codes() {
        use egui::Key as K;
        for (k, code) in [(K::A, 30), (K::Q, 16), (K::M, 50), (K::Num1, 2), (K::Num0, 11), (K::F1, 59), (K::F12, 88), (K::F13, 183), (K::F24, 194), (K::Escape, 1), (K::CapsLock, 58), (K::ControlLeft, 29), (K::ControlRight, 97), (K::AltRight, 100), (K::SuperLeft, 125), (K::ShiftRight, 54), (K::IntlBackslash, 86), (K::Menu, 127), (K::ArrowUp, 103), (K::MediaPlayPause, 164), (K::Backtick, 41), (K::Quote, 40), (K::Plus, 78), (K::Equals, 13)] {
            assert_eq!(evdev_of(k), Some(code), "{k:?}");
        }
        // Every key egui has a name for and that has a code here, round trip.
        for k in K::ALL {
            if let Some(c) = evdev_of(*k) {
                assert!(keynames::parse_key(&keynames::key_name(u32::from(c))).is_ok(), "{k:?}");
            }
        }
        assert_eq!(evdev_of(K::F25), None, "no evdev name");
        // The physical key wins: on AZERTY, the key labelled A is at Q.
        let e = egui::Event::Key { key: K::A, physical_key: Some(K::Q), pressed: true, repeat: false, modifiers: Default::default() };
        assert_eq!(event_key(&e), Some((16, true, false)));
        assert_eq!(event_key(&egui::Event::Copy), Some((46, true, false)));
        assert_eq!(event_key(&egui::Event::Text("a".into())), None);
    }

    #[test]
    fn problems_read_without_table_names() {
        assert_eq!(plain_problem("input.keymap.rule #3: from: \"capslok\" is not a key name"), "'capslok' isn't a key. Did you mean Caps Lock?");
        assert_eq!(plain_problem("input.button #1: button 9"), "Button rule 1: button 9");
        let cli = "input.button #1: the left button must keep clicking: os:hide-app has no shortcut for --os windows";
        assert_eq!(plain_problem(cli), "The left button must keep clicking. Hide app does nothing on Windows PCs. Choose For: Only Macs to use it.");
        assert_eq!(plain_problem("unknown system action \"os:mision\"; did you mean os:overview (Mission Control)?"),
            "Unknown system action. Choose an action from the list.");
        assert_eq!(plain_problem("rules for \"x\" never apply"), "Rules for \"x\" never apply");
    }

    #[test]
    fn the_list_finds_keys_by_any_name() {
        let keys = choices(false, os::UNKNOWN);
        let first = |q: &str| search(&keys, q).first().map(|c| c.value.clone());
        assert_eq!(first("esc").as_deref(), Some("esc"));
        assert_eq!(first("Escape").as_deref(), Some("esc"));
        assert_eq!(first("caps lock").as_deref(), Some("capslock"));
        assert_eq!(first("vol").as_deref(), Some("volume down"));
        assert_eq!(first("next comp").as_deref(), Some("switch next"));
        assert_eq!(first("-").as_deref(), Some("minus"));
        assert!(search(&keys, "zzzz").is_empty());
        assert_eq!(search(&keys, "").len(), keys.len());
        assert!(!keys.iter().any(|c| c.value == "button1"), "buttons are under Mouse");
        // Every value is something `one-mk keymap add` takes.
        for c in &keys {
            assert!(c.value.parse::<Output>().is_ok(), "{}", c.value);
        }
        let buttons = choices(true, os::UNKNOWN);
        assert_eq!(buttons[0].value, "button1");
        assert_eq!(search(&buttons, "play").iter().take(2).map(|c| c.value.as_str()).collect::<Vec<_>>(), ["play", "play/pause"], "the exact name first");
    }

    #[test]
    fn capture_takes_a_chord_or_a_modifier_alone() {
        let mut c = Capture::default();
        assert_eq!(c.key(29, true), None);
        assert_eq!(c.key(42, true), None);
        assert_eq!(c.key(1, true), Some("ctrl+shift+esc".parse().unwrap()));
        let mut c = Capture::default();
        assert_eq!(c.key(97, true), None);
        assert_eq!(c.key(97, false), Some(Chord::key(97)), "Right Ctrl alone");
        let mut c = Capture::default();
        c.key(29, true);
        assert_eq!(c.key(58, true).map(|c| chord_text(&c)), Some(format!("{}+Caps Lock", ctrl_word())));
    }

    #[test]
    fn the_test_box_shows_taps_holds_and_what_is_sent() {
        let i = input();
        let f = Facts::of(&Scope::All, &[]);
        let mut t = Tester::new(i.keymap(&f).unwrap(), i.tap());
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        // Caps Lock tapped: Esc.
        t.key(58, true, ms(0));
        assert!(t.deadline().is_some());
        assert!(t.note.as_deref().unwrap().contains("a tap or a hold"));
        assert!((t.progress(ms(90)).unwrap() - 0.5).abs() < 0.01);
        t.key(58, false, ms(85));
        assert_eq!(t.note.as_deref(), Some("Caps Lock was tapped: let go after 85 ms, under 180 ms."));
        assert_eq!(t.sent.chips.back(), Some(&Chip { text: "Esc".into(), note: Some("tap".into()) }));
        // Held with C: Ctrl+C (the other key decides it).
        t.key(58, true, ms(1000));
        t.key(46, true, ms(1050));
        assert_eq!(t.note.as_deref(), Some("Caps Lock was held: another key went down after 50 ms."));
        t.key(46, false, ms(1100));
        t.key(58, false, ms(1150));
        assert_eq!(t.sent.chips.back().unwrap().text, format!("{}+C", ctrl_word()));
        assert_eq!(t.pressed.chips.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Caps Lock", "Caps Lock", "C"]);
        // Held past its time: Ctrl, alone.
        t.key(58, true, ms(2000));
        t.tick(ms(2200));
        assert_eq!(t.note.as_deref(), Some("Caps Lock was held: 180 ms passed."));
        assert_eq!(t.deadline(), None);
        t.key(58, false, ms(2300));
        assert_eq!(t.sent.chips.back().unwrap().text, ctrl_word());
        // Actions are shown, never run.
        let mut j = i.clone();
        j.keys[5].scope = Scope::All;
        let mut t = Tester::new(j.keymap(&f).unwrap(), j.tap());
        t.key(183, true, ms(0));
        assert_eq!(t.sent.chips.back(), Some(&Chip { text: "Switches to the next computer".into(), note: Some("not run here".into()) }));
        // A layer: H is Left only while Right Alt is held.
        t.key(100, true, ms(10));
        t.key(35, true, ms(20));
        t.key(35, false, ms(30));
        t.key(100, false, ms(40));
        t.key(35, true, ms(50));
        let sent: Vec<&str> = t.sent.chips.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(sent, ["Switches to the next computer", "Left", "H"]);
        t.clear();
        assert!(t.sent.chips.is_empty() && t.note.is_none());
    }
    /// Rules for one keyboard or mouse, as `one-mk keymap add --device`
    /// writes them (a `[...scope]` table) or by hand (inline, with a
    /// computer too): read as such, never as rules for every computer.
    #[test]
    fn device_scopes_are_read_and_applied() {
        const KB: &str = "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard";
        let text = format!(
            r#"
[[input.keymap.rule]]
from = "capslock"
to = "esc"

[input.keymap.rule.scope]
device = "{KB}"

[[input.keymap.rule]]
from = "capslock"
to = "lctrl"
scope = {{ peer = "Studio PC", device = "{KB}" }}

[[input.keymap.rule]]
from = "capslock"
to = "f1"
"#
        );
        let i = Input::read(&read_toml(&text));
        let mine = Scope::Device(KB.into(), Box::new(Scope::All));
        assert_eq!(i.keys[0].scope, mine);
        assert_eq!(i.keys[1].scope, Scope::Device(KB.into(), Box::new(Scope::Peer("Studio PC".into()))));
        assert_eq!(i.keys[2].scope, Scope::All);
        assert_eq!(mine.label(), "Only Travel Keyboard");
        assert_eq!(i.keys[1].scope.label(), "Only Travel Keyboard, on Studio PC");
        assert_eq!(i.keys[1].scope.args(), [format!("--device={KB}"), "--peer=Studio PC".to_owned()]);
        let caps = |f: &Facts| i.specs(f).into_iter().find(|(s, _)| s.from == "capslock".parse().unwrap()).map(|(_, at)| at);
        let peers = [Peer { name: "Studio PC".into(), id: String::new(), os: os::WINDOWS }];
        assert_eq!(caps(&Facts::of(&Scope::All, &peers)), Some(Ok(2)), "other keyboards: the rule for every keyboard");
        assert_eq!(caps(&Facts::of(&mine, &peers)), Some(Ok(0)), "the travel keyboard, anywhere");
        assert_eq!(caps(&Facts::of(&Scope::Device(KB.into(), Box::new(Scope::Peer("Studio PC".into()))), &peers)), Some(Ok(1)));
        let other = Scope::Device("usb/0001:0002//Other".into(), Box::new(Scope::All));
        assert_eq!(caps(&Facts::of(&other, &peers)), Some(Ok(2)), "another keyboard");
    }

    #[test]
    fn devices_unavailable_reads_plainly() {
        let merged = "per-device rules are not available on this desktop: the desktop merges all keyboards and mice before oneMK sees them";
        assert!(devices_unavailable_text(Some(merged)).starts_with("Not on this desktop: it mixes all keyboards and mice together"));
        assert_eq!(devices_unavailable_text(Some("the null input backend captures no keyboard or mouse")), "The null input backend captures no keyboard or mouse.");
        assert!(!devices_unavailable_text(None).is_empty());
        assert_eq!(devices_untested_text(""), "Try your rule in the test box before using it on another computer.");
        let d = SeenDevice { bus: "bluetooth".into(), connection: "Bluetooth".into(), keyboard: true, ..Default::default() };
        assert_eq!(d.about(), "Asleep or unplugged · Bluetooth keyboard");
        let d = SeenDevice { bus: "unknown".into(), connection: "connection unknown".into(), pointer: true, connected: true, ..Default::default() };
        assert_eq!(d.about(), "Connected · mouse");
        let d = SeenDevice { bus: "internal".into(), connection: "built in".into(), keyboard: true, pointer: true, connected: true, ..Default::default() };
        assert_eq!(d.about(), "Connected · Built-in keyboard and mouse");
        assert!(SeenDevice { bus: "virtual".into(), ..Default::default() }.is_virtual());
        let twin = SeenDevice { bus: "usb".into(), connection: "USB".into(), pointer: true, connected: true, twins: true, ..Default::default() };
        assert_eq!(twin.about(), "Connected · USB mouse · Same ID as another");
    }

}

#[cfg(test)]
mod fixes {
    use super::*;

    fn peer(name: &str, id: &str, os: u8) -> Peer {
        Peer { name: name.into(), id: id.into(), os }
    }

    fn rows(rules: Vec<ScrollRule>) -> Input {
        Input { scroll: rules, ..Default::default() }
    }

    /// Two computers that call themselves alike are shown as `Studio (ab12-cd34)`, a name the
    /// daemon cannot find: what is saved or run names one by its id.
    #[test]
    fn a_computer_shown_with_its_id_is_named_by_its_id() {
        let twin = peer("Studio (ab12-cd34)", "ab12-cd34", os::LINUX);
        let plain = peer("Studio", "ee00-ff11", os::LINUX);
        assert_eq!(twin.scope_arg(), "ab12-cd34");
        assert_eq!(plain.scope_arg(), "Studio");
        // The id still picks it out, and the window still calls it by its name.
        let scope = Scope::Peer(twin.scope_arg());
        let peers = [twin.clone(), plain];
        assert_eq!(Facts::of(&scope, &peers).name, "Studio (ab12-cd34)");
        assert_eq!(scope.label_in(&peers), "Only Studio (ab12-cd34)");
        assert_eq!(scope.args(), ["--peer=ab12-cd34"]);
        // `switch Here` means "back here"; a computer called that is named by its id.
        assert_eq!(peer("Here", "ab12-cd34", os::LINUX).switch_arg(), "ab12-cd34");
        assert_eq!(peer(" NEXT ", "ab12-cd34", os::LINUX).switch_arg(), "ab12-cd34");
        assert_eq!(peer("Office", "ab12-cd34", os::LINUX).switch_arg(), "Office");
        assert_eq!(twin.switch_arg(), "ab12-cd34");
    }

    #[test]
    fn an_app_that_cannot_exist_on_the_computers_chosen_is_refused_before_anything_changes() {
        let mac = Scope::Os(os::MACOS);
        let rule = KeyRule { scope: mac.clone(), app: Some("chrome.exe".into()), ..KeyRule::to("capslock", "esc") };
        let why = check_add(&Input::default(), &rule).unwrap_err();
        assert!(why.contains("never matches on a Mac"), "{why}");
        // For Macs through a keyboard too; for Windows, or for all, it is fine.
        let device = KeyRule { scope: Scope::Device("usb/1:2//k".into(), Box::new(mac)), ..rule.clone() };
        assert!(check_add(&Input::default(), &device).is_err());
        for scope in [Scope::Os(os::WINDOWS), Scope::All] {
            assert!(check_add(&Input::default(), &KeyRule { scope, ..rule.clone() }).is_ok());
        }
    }

    #[test]
    fn the_left_and_right_buttons_cannot_be_given_nothing() {
        for n in [1, 2] {
            let why = check_button(n, "none", os::UNKNOWN).unwrap_err();
            assert!(why.contains("must keep clicking"), "{why}");
        }
        assert!(check_button(2, "layer:nav", os::UNKNOWN).is_err());
        for (n, to) in [(1, "button2"), (2, "esc"), (3, "none"), (4, "none"), (5, "back")] {
            assert!(check_button(n, to, os::UNKNOWN).is_ok(), "{n} {to}");
        }
    }

    #[test]
    fn natural_scrolling_is_unset_without_losing_the_rest_of_the_row() {
        let row = |speed: Option<f64>, speed_x: Option<f64>, speed_y: Option<f64>| ScrollRule {
            natural: Some("on".into()), speed, speed_x, speed_y, mode: Some("smooth".into()), ..Default::default()
        };
        // Whatever the row holds (a speed for each axis, a speed with many digits), one
        // command clears natural and nothing else: no reset, no rewrite of the rest.
        for r in [row(Some(0.75), None, None), row(None, Some(2.5), Some(0.75)), row(Some(1.234), None, None)] {
            let cmds = scroll_commands(&rows(vec![r]), &Scope::All, None, &ScrollChange::Natural(None));
            assert_eq!(cmds, [["scroll", "--clear-natural"]]);
        }
        let scoped = scroll_commands(&rows(vec![ScrollRule { scope: Scope::Os(os::LINUX), device: Some(Device::Wheel), ..row(None, Some(2.5), None) }]),
            &Scope::Os(os::LINUX), Some(Device::Wheel), &ScrollChange::Natural(None));
        assert_eq!(scoped, [["scroll", "--clear-natural", "--kind=wheel", "--os=linux"]]);
    }

    #[test]
    fn as_here_for_natural_scrolling_overrides_what_a_broader_rule_supplies() {
        let all = rows(vec![ScrollRule { natural: Some("on".into()), ..Default::default() }]);
        let laptop = Scope::Peer("Laptop".into());
        let facts = |scope: &Scope| Facts::of(scope, &[]);
        // Nothing of the laptop's own to forget: the command used to be empty.
        assert!(scroll_commands(&all, &laptop, None, &ScrollChange::Natural(None)).is_empty());
        let change = natural_as_here(&all, &facts(&laptop), &laptop, None);
        assert_eq!(change, ScrollChange::Natural(Some(natural_like_here())));
        let cmds = scroll_commands(&all, &laptop, None, &change);
        assert_eq!(cmds, vec![vec!["scroll".to_owned(), format!("--natural={}", natural_like_here()), "--peer=Laptop".to_owned()]]);
        // Where the scope has its own and nothing broader sets one, it is forgotten; where nothing is in force, nothing is asked.
        assert_eq!(natural_as_here(&all, &facts(&Scope::All), &Scope::All, None), ScrollChange::Natural(None));
        assert_eq!(natural_as_here(&Input::default(), &facts(&laptop), &laptop, None), ScrollChange::Natural(None));
    }

    #[test]
    fn as_here_keeps_this_computers_direction_when_a_broader_rule_would_flip_it() {
        // Everyone on, the laptop off: forgetting the laptop's own would put it on, not as here.
        let laptop = Scope::Peer("Laptop".into());
        let both = rows(vec![
            ScrollRule { natural: Some("on".into()), ..Default::default() },
            ScrollRule { natural: Some("off".into()), scope: laptop.clone(), ..Default::default() },
        ]);
        let change = natural_as_here(&both, &Facts::of(&laptop, &[]), &laptop, None);
        assert_eq!(change, ScrollChange::Natural(Some(natural_like_here())));
        // The kind's own row against a rule for both kinds at the same place.
        let kinds = rows(vec![
            ScrollRule { natural: Some("on".into()), ..Default::default() },
            ScrollRule { natural: Some("off".into()), device: Some(Device::Wheel), ..Default::default() },
        ]);
        let all = Facts::of(&Scope::All, &[]);
        assert_eq!(natural_as_here(&kinds, &all, &Scope::All, Some(Device::Wheel)), ScrollChange::Natural(Some(natural_like_here())));
        assert_eq!(natural_as_here(&kinds, &all, &Scope::All, None), ScrollChange::Natural(None));
    }

    #[test]
    fn unsetting_natural_scrolling_covers_every_table_of_a_place_in_one_command() {
        let tables = rows(vec![
            ScrollRule { natural: Some("on".into()), ..Default::default() },
            ScrollRule { speed_y: Some(2.0), mode: Some("discrete".into()), ..Default::default() },
        ]);
        let cmds = scroll_commands(&tables, &Scope::All, None, &ScrollChange::Natural(None));
        assert_eq!(cmds, [["scroll", "--clear-natural"]], "the other table's settings are not rewritten");
        // The natural value in a later table is found too.
        let later = rows(vec![
            ScrollRule { mode: Some("smooth".into()), ..Default::default() },
            ScrollRule { natural: Some("on".into()), ..Default::default() },
        ]);
        assert_eq!(scroll_commands(&later, &Scope::All, None, &ScrollChange::Natural(None)), [["scroll", "--clear-natural"]]);
        // No natural setting anywhere in the place: nothing is asked.
        let none = rows(vec![ScrollRule { mode: Some("smooth".into()), ..Default::default() }]);
        assert!(scroll_commands(&none, &Scope::All, None, &ScrollChange::Natural(None)).is_empty());
    }

    #[test]
    fn scroll_shows_what_the_daemon_resolves() {
        let f = Facts::of(&Scope::All, &[]);
        // Up and down use the vertical speed when a row has one.
        let both = rows(vec![ScrollRule { speed: Some(2.0), speed_y: Some(3.0), ..Default::default() }]);
        assert_eq!(scroll_shown(&both, &f, None).speed, 3.0);
        // A plain speed in a more specific row resets the axes set less specifically.
        let reset = rows(vec![
            ScrollRule { speed_y: Some(3.0), ..Default::default() },
            ScrollRule { speed: Some(1.5), scope: Scope::Os(os::LINUX), ..Default::default() },
        ]);
        let linux = Facts::of(&Scope::Os(os::LINUX), &[]);
        assert_eq!(scroll_shown(&reset, &linux, None).speed, 1.5);
        // A modifier written for wheels is the wheel's alone (the daemon keeps them apart), and one for everything is both's.
        let wheel = rows(vec![
            ScrollRule { device: Some(Device::Wheel), modifiers: vec![("shift".into(), "horizontal".into())], ..Default::default() },
            ScrollRule { device: Some(Device::Touchpad), modifiers: vec![("alt".into(), "x5".into())], ..Default::default() },
            ScrollRule { modifiers: vec![("alt".into(), "x2".into())], scope: Scope::Os(os::LINUX), ..Default::default() },
        ]);
        let linux = Facts::of(&Scope::Os(os::LINUX), &[]);
        let shown = |device| { let s = scroll_shown(&wheel, &linux, device); (s.shift_sideways, s.alt_faster) };
        assert_eq!(shown(Some(Device::Wheel)), (true, true));
        assert_eq!(shown(Some(Device::Touchpad)), (false, true), "the touchpad's Shift still scrolls normally");
        assert_eq!(shown(None), (false, true), "only what is written for both kinds");
    }

    #[test]
    fn the_test_box_waits_as_long_as_the_rule_does() {
        let rule = KeyRule { timeout_ms: Some(500), ..KeyRule::tap_hold("capslock", "esc", "lctrl") };
        let input = Input { keys: vec![rule], ..Default::default() };
        let mut t = Tester::new(input.keymap(&Facts::of(&Scope::All, &[])).unwrap(), Duration::from_millis(200));
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        t.key(58, true, ms(0));
        assert!(t.note.as_deref().unwrap().contains("within 500 ms"), "{:?}", t.note);
        assert!((t.progress(ms(250)).unwrap() - 0.5).abs() < 0.01, "the bar runs to the rule's own time");
        // Let go at 300 ms: under the rule's 500, so a tap (the general 200 would say held).
        t.key(58, false, ms(300));
        assert_eq!(t.note.as_deref(), Some("Caps Lock was tapped: let go after 300 ms, under 500 ms."));
        assert_eq!(t.sent.chips.back(), Some(&Chip { text: "Esc".into(), note: Some("tap".into()) }));
        // Held past 500: held, and it says so with the time it waited.
        t.key(58, true, ms(1000));
        t.tick(ms(1500));
        assert_eq!(t.note.as_deref(), Some("Caps Lock was held: 500 ms passed."));
        assert_eq!(t.sent.chips.back().unwrap().note.as_deref(), Some("hold"));
    }

    #[test]
    fn the_test_box_sends_ctrl_and_cmd_swapped_where_a_visit_does() {
        let input = Input { keys: vec![KeyRule::to("capslock", "lctrl")], ..Default::default() };
        let mac = Facts::of(&Scope::Os(os::MACOS), &[]);
        let mut t = Tester::new(input.keymap(&mac).unwrap(), input.tap());
        let t0 = Instant::now();
        t.key(58, true, t0);
        t.key(58, false, t0);
        assert_eq!(t.sent.chips.back().unwrap().text, ctrl_word());
        t.clear();
        t.set_swap(true);
        t.key(58, true, t0);
        t.key(58, false, t0);
        assert_eq!(t.sent.chips.back().unwrap().text, meta_word(), "the Mac gets Cmd");
        // Which computers are swapped: Auto only between a Mac and the rest; a setting overrides it.
        assert!(swap_applies("auto", os::LINUX, os::MACOS) && swap_applies("auto", os::MACOS, os::WINDOWS));
        assert!(!swap_applies("auto", os::LINUX, os::WINDOWS) && !swap_applies("auto", os::MACOS, os::MACOS));
        assert!(swap_applies("on", os::LINUX, os::LINUX) && !swap_applies("off", os::LINUX, os::MACOS));
        let peer = Facts { name: "Mac".into(), id: "ab12-cd34".into(), os: os::MACOS, device: None, swap: true };
        assert!(!peer.with_swap_setting("off").swap);
    }

    #[test]
    fn picker_search_ranks_aliases_and_misspelled_words() {
        let keys = choices(false, os::MACOS);
        for (query, top) in [
            ("off", "none"), ("disable", "none"), ("lock", "lock-pointer"),
            ("mision", "os:overview"), ("mision control", "os:overview"),
            ("volune", "volume down"), ("scrnshot", "os:screenshot-area"), ("desktp", "os:next-desktop"),
            ("swipe", "os:next-desktop"), ("three finger", "os:next-desktop"),
            ("spaces", "os:next-desktop"), ("workspace", "os:next-desktop"), ("expose", "os:app-windows"),
        ] {
            let hits = search(&keys, query);
            assert_eq!(hits.first().map(|choice| choice.value.as_str()), Some(top), "{query}");
        }
        assert!(search(&keys, "lock").iter().any(|choice| choice.value == "lock-all"));
    }

    #[test]
    fn system_choice_availability_and_screenshot_labels_follow_the_scope() {
        for target in [os::WINDOWS, os::LINUX] {
            let choices = choices(false, target);
            let screenshots = search(&choices, "screenshot");
            assert_eq!(screenshots.iter().filter(|c| c.value.starts_with("os:screenshot")).count(), 1);
            let detail = &screenshots[0].detail;
            assert!(detail.contains(if target == os::LINUX { "GNOME 42+" } else { "Snipping Tool" }));
        }
        let mac = choices(false, os::MACOS);
        assert_eq!(mac.iter().filter(|c| c.value.starts_with("os:screenshot")).count(), 2);
        assert!(mac.iter().all(|c| c.value != "os:app-launcher" || !c.mac));
        let windows = choices(false, os::WINDOWS);
        let hidden = windows.iter().find(|c| c.value == "os:hide-app").unwrap();
        assert!(!hidden.available);
        assert_eq!(hidden.detail, "Not on Windows");
        for button in [1, 2] {
            assert!(check_button(button, "os:hide-app", os::WINDOWS).is_err());
            assert!(check_button(button, "os:quit-app", os::UNKNOWN).is_err());
            assert!(check_button(button, "os:hide-app", os::MACOS).is_ok());
        }
        assert!(check_button(4, "os:hide-app", os::WINDOWS).is_ok());
    }

    #[test]
    fn partial_actions_are_pickable_except_for_primary_buttons() {
        let choices = choices(true, os::UNKNOWN);
        for action in ["os:app-windows", "os:app-launcher", "os:hide-app"] {
            let choice = choices.iter().find(|c| c.value == action).unwrap();
            assert!(picker_available(choice, false, os::UNKNOWN));
            assert!(!picker_available(choice, true, os::UNKNOWN));
        }
        let app_windows = choices.iter().find(|c| c.value == "os:app-windows").unwrap();
        assert_eq!(app_windows.detail, "Mac and Linux · nothing on Windows");
        let hide = choices.iter().find(|c| c.value == "os:hide-app").unwrap();
        assert_eq!(hide.disabled_hint.as_deref(), Some("Choose For: Only Macs to use it"));
        assert!(!picker_available(choices.iter().find(|c| c.value == "none").unwrap(), true, os::UNKNOWN));
    }

    #[test]
    fn system_notes_and_labels_follow_the_rule_scope() {
        let action: Output = "os:app-windows".parse().unwrap();
        assert_eq!(out_text_for(&action, os::MACOS), "App Exposé");
        assert_eq!(system_note(&What::To(action.clone()), os::MACOS), None);
        let warning = system_note(&What::To(action), os::UNKNOWN).unwrap();
        assert!(warning.starts_with("App Exposé does nothing on Windows PCs."));
        assert!(warning.contains("For:"));
        assert!(button_rule_note(7, os::MACOS).is_some());
        assert!(button_rule_note(7, os::WINDOWS).is_some());
        assert!(button_rule_note(7, os::LINUX).is_none());
        assert_eq!(key_label(133), "Copy key");
        let list = choices(false, os::UNKNOWN);
        let copy = list.iter().find(|c| c.value == "copy").unwrap();
        assert_eq!((&*copy.label, &*copy.detail), ("Copy key", ""));
    }

    #[test]
    fn system_actions_have_readable_rule_and_keycap_names() {
        assert_eq!(To::One("os:previous-desktop".into()).describe(os::UNKNOWN), "Previous desktop");
        assert_eq!(cap_target(&What::To("os:next-desktop".parse().unwrap()), os::UNKNOWN), "Next desk");
    }

    #[test]
    fn a_paste_with_a_picture_is_still_the_v_key_going_down() {
        let image = std::sync::Arc::new(egui::ColorImage::default());
        assert_eq!(event_key(&egui::Event::PasteImage(image)), Some((47, true, false)));
        assert_eq!(event_key(&egui::Event::Paste("x".into())), Some((47, true, false)));
    }

    #[test]
    fn side_swipes_are_refused_where_autoscroll_takes_the_back_button() {
        let row = |button: &str, enabled: bool| AutoscrollRow { enabled: Some(enabled), speed: None, button: Some(button.into()), scope: Scope::All, app: None };
        let f = Facts::of(&Scope::All, &[]);
        let with = |r: AutoscrollRow| Input { autoscroll: vec![r], ..Default::default() };
        assert!(side_swipe_conflict(&with(row("button4", true)), &f).unwrap().contains("reserved by autoscroll"));
        assert!(side_swipe_conflict(&with(row("4", true)), &f).is_some());
        assert!(side_swipe_conflict(&with(row("button4", false)), &f).is_none(), "off: nothing is shadowed");
        assert!(side_swipe_conflict(&with(row("middle", true)), &f).is_none());
        assert!(side_swipe_conflict(&Input::default(), &f).is_none());
    }

    #[test]
    fn a_button_rule_of_a_broader_scope_is_not_this_scopes_own() {
        let rule = |scope: Scope| ButtonRule { from: "4".into(), to: To::One("esc".into()), scope, app: None };
        let input = Input { buttons: vec![rule(Scope::All)], ..Default::default() };
        assert!(button_has_own_rule(&input, &Scope::All, 4));
        assert!(!button_has_own_rule(&input, &Scope::Peer("Laptop".into()), 4));
        assert!(!button_has_own_rule(&input, &Scope::All, 5));
    }
}
