//! The Keys & mouse tab, worked out without drawing anything: the
//! `[input]` rules as `settings.toml` has them, which of them apply to the
//! computers chosen, rules as sentences, the drawn keyboard and mouse, the
//! quick setups, the `one-mk keymap|pointer|scroll|buttons` commands that
//! make changes, the checks `one-mk keymap add` makes, keys as egui reports
//! them, and the "Test it" box. Names go through `one_mk_proto::keynames`
//! and `remap`, so the window, the command line and `settings.toml` agree.
//! Plain data in, plain data out: tested without a window.

use crate::view::{Entry, Val};
use one_mk_proto::keynames::{self, Chord, Trigger};
use one_mk_proto::os;
use one_mk_proto::remap::{self, Action, Emit, Keymap, Mapper, Nav, Output, RuleSpec, What};
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
}

/// A paired computer, as far as scopes care.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub id: String,
    pub os: u8,
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
    /// `--os mac` / `--peer=NAME`, for the command line.
    pub fn args(&self) -> Vec<String> {
        match self {
            Scope::All => vec![],
            Scope::Os(o) => vec![format!("--os={}", os_word(*o))],
            Scope::Peer(p) => vec![format!("--peer={p}")],
        }
    }

    /// "All computers", "Only Macs", "Only Studio PC".
    pub fn label(&self) -> String {
        match self {
            Scope::All => "All computers".into(),
            Scope::Os(o) => format!("Only {}", os_plural(*o)),
            Scope::Peer(p) => format!("Only {p}"),
        }
    }

    /// How specific the scope is for a computer (`None`: not for it): a
    /// computer beats an OS beats everything, as in `one-mk`.
    fn rank(&self, f: &Facts) -> Option<u8> {
        match self {
            Scope::All => Some(0),
            Scope::Os(o) => (*o == f.os).then_some(1),
            Scope::Peer(p) => peer_matches(p, &f.name, &f.id).then_some(2),
        }
    }

    /// From `scope = ...` (a string or an inline table) or a
    /// `[...scope]` table (`scope.os`, `scope.peer`).
    fn read(get: &dyn Fn(&str) -> Option<Val>) -> Scope {
        let from_pair = |k: &str, v: &Val| match (k, v.as_str()) {
            ("os", Some(o)) => parse_os(o).map(Scope::Os),
            ("peer", Some(p)) if !p.trim().is_empty() => Some(Scope::Peer(p.trim().to_owned())),
            _ => None,
        };
        if let Some(Val::Table(kv)) = get("scope")
            && let Some((k, v)) = kv.first()
        {
            return from_pair(k, v).unwrap_or_default();
        }
        for k in ["os", "peer"] {
            if let Some(v) = get(&format!("scope.{k}")) {
                return from_pair(k, &v).unwrap_or_default();
            }
        }
        Scope::All
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
}

impl Facts {
    /// The computers a choice in "For:" stands for. All computers: only
    /// the rules for every computer.
    pub fn of(scope: &Scope, peers: &[Peer]) -> Facts {
        match scope {
            Scope::All => Facts { name: String::new(), id: String::new(), os: os::UNKNOWN },
            Scope::Os(o) => Facts { name: String::new(), id: String::new(), os: *o },
            Scope::Peer(p) => match peers.iter().find(|x| peer_matches(p, &x.name, &x.id)) {
                Some(x) => Facts { name: x.name.clone(), id: x.id.clone(), os: x.os },
                None => Facts { name: p.clone(), id: String::new(), os: os::UNKNOWN },
            },
        }
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
    fn parse(&self) -> Result<Output, String> {
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
    /// Per-app rules are not used yet (plan 4D).
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

    /// The same key, layer and scope (what `one-mk keymap add` replaces).
    fn same_place(&self, other: &KeyRule) -> bool {
        let layer = |r: &KeyRule| r.layer.as_ref().map(|l| l.to_ascii_lowercase());
        self.scope == other.scope
            && self.app.is_none()
            && other.app.is_none()
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
    pub tap_ms: Option<u32>,
    pub keys: Vec<KeyRule>,
    pub pointer: Vec<PointerRule>,
    pub scroll: Vec<ScrollRule>,
    pub buttons: Vec<ButtonRule>,
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
        let mut input = Input { tap_ms, ..Default::default() };
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
        let specs: Vec<RuleSpec> = self.specs(f).into_iter().map(|(s, _)| s).collect();
        Keymap::compile(&specs, remap::Target { os: f.os, swap: false }, self.tap()).map(|c| c.map).map_err(|e| e.0)
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
    if name.chars().count() == 1 || name.starts_with('f') && name[1..].parse::<u8>().is_ok() {
        return name.to_uppercase();
    }
    title(&name)
}

/// A key's name on its drawn cap: short.
pub fn cap_label(code: u16) -> String {
    let s = match code {
        14 => "Backspace",
        58 => "Caps",
        42 | 54 => "Shift",
        29 | 97 => "Ctrl",
        56 | 100 => if cfg!(target_os = "macos") { "Opt" } else { "Alt" },
        125 | 126 => meta_word(),
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
        96 => "Enter",
        55 => "*",
        74 => "-",
        78 => "+",
        98 => "/",
        83 => ".",
        71..=73 | 75..=77 | 79..=82 => return keynames::key_name(u32::from(code)).trim_start_matches("kp").to_owned(),
        86 => "<>",
        102 => "Home",
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
    match o {
        Output::Chord(c) => chord_text(c),
        Output::Sequence(s) => format!("types {}", s.iter().map(chord_text).collect::<Vec<_>>().join(", then ")),
        Output::None => "does nothing".into(),
        Output::Layer(l) => format!("hold it for the “{l}” keys"),
        Output::Nav(Nav::Back) => "Back (browser)".into(),
        Output::Nav(Nav::Forward) => "Forward (browser)".into(),
        Output::Action(a) => action_text(a),
    }
}

/// The same, as the thing a tap or a hold gives: "Esc", "nothing", "the
/// “nav” keys".
fn out_noun(o: &Output) -> String {
    match o {
        Output::None => "nothing".into(),
        Output::Layer(l) => format!("the “{l}” keys"),
        _ => out_text(o),
    }
}

/// A rule in one sentence: "Caps Lock → Esc when tapped, Ctrl when held";
/// `Err` with the rule as written and why it can't be used.
pub fn sentence(input: &Input, r: &KeyRule) -> Result<String, String> {
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
        What::To(o) => out_text(o),
        What::TapHold { tap, hold, timeout } => {
            let after = timeout.map_or(String::new(), |t| format!(" (after {} ms)", t.as_millis()));
            format!("{} when tapped, {} when held{after}", out_noun(tap), out_noun(hold))
        }
    };
    Ok(format!("{from} → {what}"))
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
pub fn cap_target(what: &What) -> String {
    let short = |o: &Output| match o {
        Output::Chord(c) if c.mods == 0 => cap_label(c.key),
        // A shortcut: its key, marked as one (the whole of it is on hover).
        Output::Chord(c) => format!("+{}", cap_label(c.key)),
        Output::Sequence(_) => "Keys…".into(),
        Output::None => "Off".into(),
        Output::Layer(_) => "Hold".into(),
        Output::Nav(Nav::Back) => "Back".into(),
        Output::Nav(Nav::Forward) => "Fwd".into(),
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
pub fn keyboard_size(keypad: bool) -> (f32, f32) {
    (if keypad { 23.0 } else { 18.5 }, 6.5)
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
    row(0.0, 15.5, &[(99, 1.0), (70, 1.0), (119, 1.0)]);
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
    row(1.5, 15.5, &[(110, 1.0), (102, 1.0), (104, 1.0)]);
    row(2.5, 15.5, &[(111, 1.0), (107, 1.0), (109, 1.0)]);
    row(4.5, 16.5, &[(103, 1.0)]);
    row(5.5, 15.5, &[(105, 1.0), (108, 1.0), (106, 1.0)]);
    if keypad {
        row(1.5, 19.0, &[(69, 1.0), (98, 1.0), (55, 1.0), (74, 1.0)]);
        row(2.5, 19.0, &[(71, 1.0), (72, 1.0), (73, 1.0)]);
        row(3.5, 19.0, &[(75, 1.0), (76, 1.0), (77, 1.0)]);
        row(4.5, 19.0, &[(79, 1.0), (80, 1.0), (81, 1.0)]);
        row(5.5, 19.0, &[(82, 2.0), (83, 1.0)]);
        caps.push(Cap { code: 78, x: 22.0, y: 2.5, w: 1.0, h: 2.0 });
        caps.push(Cap { code: 96, x: 22.0, y: 4.5, w: 1.0, h: 2.0 });
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
    a.push("--".into());
    a.push(r.from.clone());
    if let Some(to) = &r.to {
        a.extend(to.words());
    }
    a
}

/// `one-mk keymap remove`, for the rule's key, layer and scope.
pub fn remove_args(r: &KeyRule) -> Vec<String> {
    let mut a = vec!["keymap".to_owned(), "remove".into()];
    if let Some(l) = &r.layer {
        a.push(format!("--layer={l}"));
    }
    a.extend(r.scope.args());
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
    /// A modifier (`shift`) and what it does (`horizontal`, `unchanged`...).
    Modifier(&'static str, &'static str),
}

fn trim_num(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The commands for a scroll change. Natural scrolling can only be unset
/// by forgetting the rule (`--reset`) and writing the rest of it again.
pub fn scroll_commands(input: &Input, scope: &Scope, device: Option<Device>, change: &ScrollChange) -> Vec<Vec<String>> {
    let base = |a: &mut Vec<String>| {
        if let Some(d) = device {
            a.push(format!("--device={}", d.word()));
        }
        a.extend(scope.args());
    };
    let mut a = vec!["scroll".to_owned()];
    match change {
        ScrollChange::Natural(None) => {
            let Some(r) = input.scroll.iter().find(|r| r.scope == *scope && r.device == device && r.app.is_none()) else { return vec![] };
            if r.natural.is_none() {
                return vec![];
            }
            a.push("--reset".into());
            base(&mut a);
            let mut again = vec!["scroll".to_owned()];
            let on = |b: bool| if b { "on" } else { "off" };
            if let Some(v) = r.reverse_vertical {
                again.push(format!("--reverse-vertical={}", on(v)));
            }
            if let Some(v) = r.reverse_horizontal {
                again.push(format!("--reverse-horizontal={}", on(v)));
            }
            if let Some(s) = r.speed.or(r.speed_y) {
                again.push(format!("--speed={}", trim_num(s)));
            }
            if let Some(m) = &r.mode {
                again.push(format!("--mode={m}"));
            }
            for (m, b) in &r.modifiers {
                again.push(format!("--modifier={m}={b}"));
            }
            if again.len() == 1 {
                return vec![a];
            }
            base(&mut again);
            return vec![a, again];
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

/// Whether `one-mk buttons map N TO` would take it.
pub fn check_button(n: u32, to: &str) -> Result<(), String> {
    ButtonRule { from: n.to_string(), to: To::One(to.into()), scope: Scope::All, app: None }.spec().map(drop)
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
        egui::Event::Paste(_) => Some((47, true, false)),
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
    pub pressed: ChipLog,
    pub sent: ChipLog,
    /// How the last tap/hold key was decided, in a sentence.
    pub note: Option<String>,
    /// A tap/hold key not decided yet, and since when.
    pending: Option<(u16, Instant)>,
}

impl Tester {
    pub fn new(map: Keymap, tap: Duration) -> Tester {
        Tester { mapper: Mapper::new(map), tap, pressed: ChipLog::default(), sent: ChipLog::default(), note: None, pending: None }
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
        if let Some((k, since)) = was
            && !pending_now
        {
            let ms = now.saturating_duration_since(since).as_millis();
            let name = key_label(u32::from(k));
            self.note = Some(if k == code && !down && now.saturating_duration_since(since) < self.tap {
                format!("{name} was tapped: let go after {ms} ms, under {} ms.", self.tap.as_millis())
            } else if k == code && !down {
                format!("{name} was held: {} ms passed.", self.tap.as_millis())
            } else {
                format!("{name} was held: another key went down after {ms} ms.")
            });
            self.pending = None;
        }
        if was.is_none() && pending_now {
            self.pending = Some((code, now));
            self.note = Some(format!("{}: a tap or a hold? Let go within {} ms for a tap.", key_label(u32::from(code)), self.tap.as_millis()));
        }
        let decided = was.is_some() && self.pending.is_none();
        self.emit(out);
        if decided && let Some((k, _)) = was {
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
        if let Some((k, _)) = self.pending
            && self.mapper.deadline().is_none()
        {
            self.note = Some(format!("{} was held: {} ms passed.", key_label(u32::from(k)), self.tap.as_millis()));
            self.pending = None;
            self.emit(out);
            self.sent.note_last("hold".into());
            return;
        }
        self.emit(out);
    }

    /// How far a pending key is towards being held, 0 to 1.
    pub fn progress(&self, now: Instant) -> Option<f32> {
        let (_, since) = self.pending?;
        Some((now.saturating_duration_since(since).as_secs_f32() / self.tap.as_secs_f32()).clamp(0.0, 1.0))
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
                Emit::Key { code, down } => self.sent.key(code as u16, down),
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
        _ => "Its own button",
    }
}

/// What a button can do, offered in the list: (words, as written).
pub const BUTTON_CHOICES: [(&str, &str); 14] = [
    ("Left click", "button1"),
    ("Right click", "button2"),
    ("Middle click", "button3"),
    ("Back (browser)", "back"),
    ("Forward (browser)", "forward"),
    ("Switch to the next computer", "switch next"),
    ("Keep the pointer here (on and off)", "lock-pointer"),
    ("Lock every screen", "lock-all"),
    ("Play/pause", "play/pause"),
    ("Next track", "next track"),
    ("Previous track", "previous track"),
    ("Volume up", "volume up"),
    ("Volume down", "volume down"),
    ("Nothing", "none"),
];

/// What a button does, in words.
pub fn button_text(o: &Output) -> String {
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
        _ => out_text(o),
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
                Some((RuleSpec { what: What::To(o), .. }, _)) => (*n, button_text(o), true),
                _ => (*n, button_default(*n).into(), false),
            }
        })
        .collect()
}

/// Pointer acceleration this computer can do: only the system's, for now.
pub const RAW_MOTION: bool = false;

/// Why Off and Curve can't be chosen here, in one sentence.
pub fn accel_unavailable() -> &'static str {
    if cfg!(target_os = "macos") {
        "Off and Curve need the mouse's raw movement, which one·mk doesn't read on macOS yet: turn off Pointer acceleration in System Settings, Mouse instead."
    } else if cfg!(windows) {
        "Off and Curve need the mouse's raw movement, which one·mk doesn't read on Windows yet: turn off “Enhance pointer precision” in Windows instead."
    } else {
        "Off and Curve need the mouse's raw movement, which Linux desktops don't give one·mk: turn mouse acceleration off in your desktop's settings instead."
    }
}

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
        .filter(|(_, r)| r.app.is_none() && (r.device.is_none() || r.device == device))
        .filter_map(|(i, r)| Some((r.scope.rank(f)?, i, r)))
        .collect();
    rules.sort_by_key(|(rank, i, r)| (*rank, r.device.is_some(), *i));
    let mut s = ScrollShown { reverse_vertical: false, reverse_horizontal: false, natural: None, speed: 1.0, mode: "auto".into(), shift_sideways: false, alt_faster: false };
    for (_, _, r) in rules {
        s.reverse_vertical = r.reverse_vertical.unwrap_or(s.reverse_vertical);
        s.reverse_horizontal = r.reverse_horizontal.unwrap_or(s.reverse_horizontal);
        s.natural = r.natural.clone().or(s.natural);
        s.speed = r.speed.or(r.speed_y).unwrap_or(s.speed).clamp(0.1, 10.0);
        s.mode = r.mode.clone().unwrap_or(s.mode);
        for (m, b) in &r.modifiers {
            match m.as_str() {
                "shift" => s.shift_sideways = b == "horizontal",
                "alt" | "option" => s.alt_faster = b.starts_with('x') && b != "x1",
                _ => {}
            }
        }
    }
    s
}

/// A problem from `one-mk keymap check` in the window's words:
/// `input.keymap.rule #3: ...` becomes `Key rule 3: ...`.
pub fn plain_problem(p: &str) -> String {
    let mut out = p.to_owned();
    for (table, word) in [("input.keymap.rule #", "Key rule "), ("input.button #", "Button rule "), ("input.pointer #", "Pointer setting "), ("input.scroll #", "Scroll setting ")] {
        out = out.replace(table, word);
    }
    title(&out)
}

// ---------- the list to choose from ----------

/// One thing a key or button can become, in the searchable list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    /// As written in a rule (`esc`, `switch next`).
    pub value: String,
    /// Every name it answers to, folded for searching.
    names: Vec<String>,
}

/// Lower case without spaces, `-` and `_`; a name that is only those
/// (`-`) stays as it is.
fn fold(s: &str) -> String {
    let f: String = s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).flat_map(char::to_lowercase).collect();
    if f.is_empty() { s.trim().to_owned() } else { f }
}

/// Everything in the list: what one·mk can do, then every named key
/// (and, for a mouse button, the other buttons and media keys first).
pub fn choices(buttons: bool) -> Vec<Choice> {
    let choice = |label: &str, value: &str, extra: &[&str]| Choice {
        label: label.into(),
        value: value.into(),
        names: std::iter::once(label).chain(std::iter::once(value)).chain(extra.iter().copied()).map(fold).collect(),
    };
    let mut out: Vec<Choice> = if buttons {
        BUTTON_CHOICES.iter().map(|(l, v)| choice(l, v, &[])).collect()
    } else {
        vec![
            choice("Nothing (the key does nothing)", "none", &["off", "disable"]),
            choice("Switch to the next computer", "switch next", &[]),
            choice("Come back to this computer", "switch here", &[]),
            choice("Keep the pointer here (on and off)", "lock-pointer", &["lock"]),
            choice("Lock every screen", "lock-all", &["lock"]),
            choice("Back (browser)", "back", &[]),
            choice("Forward (browser)", "forward", &[]),
        ]
    };
    for (code, name, names) in keynames::all_names() {
        if keynames::is_button(code) || out.iter().any(|c| c.value == name) {
            continue;
        }
        let mut c = choice(&key_label(code), name, names);
        c.names.push(fold(&keynames::key_name(code)));
        out.push(c);
    }
    out
}

/// The choices that match what was typed: by name, alias or label; the
/// ones whose name starts with it first.
pub fn search<'a>(all: &'a [Choice], query: &str) -> Vec<&'a Choice> {
    let q = fold(query.trim());
    if q.is_empty() {
        return all.iter().collect();
    }
    let mut hits: Vec<(u8, &Choice)> = all
        .iter()
        .filter(|c| c.names.iter().any(|n| n.contains(&q)))
        .map(|c| (if c.names.contains(&q) { 0 } else if c.names.iter().any(|n| n.starts_with(&q)) { 1 } else { 2 }, c))
        .collect();
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter().map(|(_, c)| c).collect()
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
        assert_eq!(s(0), "Caps Lock → Esc when tapped, Ctrl when held");
        assert_eq!(s(1), format!("Left Alt → {}", meta_word()));
        assert_eq!(s(2), "Right Alt → hold it for the “nav” keys");
        assert_eq!(s(3), "Right Alt + H → Left");
        assert_eq!(s(4), "F2 → types Ctrl+A, then Ctrl+C");
        assert_eq!(s(5), "F13 → switches to the next computer");
        assert_eq!(sentence(&i, &KeyRule::to("ctrl+h", "backspace")).unwrap(), "Ctrl+H → Backspace");
        assert_eq!(sentence(&i, &KeyRule::to("rctrl", "none")).unwrap(), "Right Ctrl → does nothing");
        assert_eq!(sentence(&i, &KeyRule::to("a", "ctrl+shift+esc")).unwrap(), "A → Ctrl+Shift+Esc");
        let timed = KeyRule { timeout_ms: Some(300), ..KeyRule::tap_hold("space", "space", "layer:nav") };
        assert_eq!(sentence(&i, &timed).unwrap(), "Space → Space when tapped, the “nav” keys when held (after 300 ms)");
        let orphan = KeyRule { layer: Some("fn".into()), ..KeyRule::to("j", "down") };
        assert_eq!(sentence(&i, &orphan).unwrap(), "In “fn”: J → Down");
        assert!(sentence(&i, &KeyRule::to("capslok", "esc")).unwrap_err().contains("\"capslok\" is not a key name"));
        assert_eq!(as_written(&KeyRule::to("capslok", "esc")), "capslok → esc");
        assert_eq!(cap_target(&i.keys[0].spec().unwrap().what), "Esc/Ctrl");
        assert_eq!(cap_target(&What::To("ctrl+shift+esc".parse().unwrap())), "+Esc");
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
        assert_eq!(check_button(4, "back"), Ok(()));
        assert!(check_button(4, "switch").unwrap_err().contains("needs `next`"));
        assert!(check_button(9, "back").unwrap_err().contains("numbered 1 to 8"));
    }

    #[test]
    fn commands_for_buttons_pointer_and_scrolling() {
        assert_eq!(button_args(4, Some("back"), &Scope::All), ["buttons", "map", "--", "4", "back"]);
        assert_eq!(button_args(3, None, &Scope::Os(os::LINUX)), ["buttons", "unmap", "--os=linux", "--", "3"]);
        assert_eq!(pointer_args(Some(1.25), None, &Scope::All), ["pointer", "--speed=1.25"]);
        assert_eq!(pointer_args(None, Some("system"), &Scope::Peer("Studio PC".into())), ["pointer", "--accel=system", "--peer=Studio PC"]);
        let i = input();
        let one = |c: ScrollChange, d: Option<Device>| scroll_commands(&i, &Scope::All, d, &c);
        assert_eq!(one(ScrollChange::ReverseVertical(true), Some(Device::Touchpad)), [["scroll", "--reverse-vertical=on", "--device=touchpad"]]);
        assert_eq!(one(ScrollChange::Modifier("shift", "unchanged"), None), [["scroll", "--modifier=shift=unchanged"]]);
        assert_eq!(one(ScrollChange::Speed(2.0), None), [["scroll", "--speed=2"]]);
        // Back to "as here": the rule is forgotten, and the rest written again.
        assert_eq!(one(ScrollChange::Natural(None), None), [vec!["scroll", "--reset"], vec!["scroll", "--modifier=alt=x5", "--modifier=shift=horizontal"]]);
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
        assert_eq!(button_text(&"button2".parse().unwrap()), "Right click");
        assert_eq!(button_text(&"ctrl+c".parse().unwrap()), "Ctrl+C");
        for (_, to) in BUTTON_CHOICES {
            assert_eq!(check_button(5, to), Ok(()), "{to}");
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
        assert_eq!(plain_problem("input.keymap.rule #3: from: \"capslok\" is not a key name"), "Key rule 3: from: \"capslok\" is not a key name");
        assert_eq!(plain_problem("input.button #1: button 9"), "Button rule 1: button 9");
        assert_eq!(plain_problem("rules for \"x\" never apply"), "Rules for \"x\" never apply");
    }

    #[test]
    fn the_list_finds_keys_by_any_name() {
        let keys = choices(false);
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
        let buttons = choices(true);
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
        assert_eq!(c.key(58, true).map(|c| chord_text(&c)), Some("Ctrl+Caps Lock".into()));
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
        assert_eq!(t.sent.chips.back().unwrap().text, "Ctrl+C");
        assert_eq!(t.pressed.chips.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Caps Lock", "Caps Lock", "C"]);
        // Held past its time: Ctrl, alone.
        t.key(58, true, ms(2000));
        t.tick(ms(2200));
        assert_eq!(t.note.as_deref(), Some("Caps Lock was held: 180 ms passed."));
        assert_eq!(t.deadline(), None);
        t.key(58, false, ms(2300));
        assert_eq!(t.sent.chips.back().unwrap().text, "Ctrl");
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
}
