//! Input tuning settings: the key mapper's rules, pointer speed, scrolling
//! and button remapping (`[input]` in `settings.toml`), which of them
//! apply to a given computer, and `one-mk keymap|pointer|scroll|buttons`.
//! The engines are pure and live in `one_mk_proto` (`remap`, `pointer`,
//! `scroll`); the design is in `docs/INPUT-TUNING.md`.
//!
//! Rules are kept as written (strings) and parsed when a visit starts, so
//! a typo in one rule costs that rule, not the whole file; `one-mk keymap
//! check` and `one-mk doctor` report it.

use crate::settings::{self, Settings};
use anyhow::{Result, bail};
use one_mk_net::TrustStore;
use one_mk_proto::keynames::{self, Trigger};
use one_mk_proto::pointer::{self, Accel, PointerTune};
use one_mk_proto::remap::{self, Keymap, Output, RuleSpec, What};
use one_mk_proto::scroll::{self, ModScroll, Natural, ScrollConfig};
use one_mk_proto::os;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
use std::time::Duration;

/// Whether this computer's input backend reports unaccelerated motion.
/// None does yet (see `docs/INPUT-TUNING.md`): acceleration `off` and
/// `curve` then fall back to the system's.
pub const RAW_MOTION: bool = false;

/// Why acceleration `off`/`curve` can't be had here, and what to do instead.
pub fn no_raw_motion_reason() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS reports the pointer's motion after its acceleration, and reading the raw motion (IOHIDManager) isn't done yet. \
         Turn acceleration off in System Settings, Mouse (Pointer acceleration, macOS 14 and later) instead: \
         one-mk sends positions computed here, so that holds on every computer this one controls."
    } else if cfg!(windows) {
        "the Windows backend reads the cursor after Windows' acceleration; raw input (WM_INPUT) isn't read yet. \
         Turn off \"Enhance pointer precision\" (Settings, Mouse, Additional mouse settings, Pointer Options) instead: \
         one-mk sends positions computed here, so that holds on every computer this one controls."
    } else {
        "on Linux the desktop hands one-mk the pointer's motion after its acceleration (libei carries no raw motion), \
         and nothing can divide it back out. Turn acceleration off in the desktop instead (GNOME: Settings, Mouse & \
         Touchpad, Mouse Acceleration off; KDE: Mouse, Acceleration profile Flat): one-mk sends positions computed \
         here, so that holds on every computer this one controls."
    }
}

// ------------------------------------------------------------------ scope

/// Which computers a rule is for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Global,
    Local,
    /// Computers running this OS (one of `os`).
    Os(u8),
    /// One paired computer, by name or id.
    Peer(String),
}

impl Scope {
    pub fn is_global(&self) -> bool {
        *self == Scope::Global
    }

    /// From `--peer` and `--os` (at most one).
    pub fn from_args(peer: Option<&str>, os: Option<&str>) -> Result<Scope> {
        match (peer, os) {
            (Some(_), Some(_)) => bail!("say --peer or --os, not both"),
            (Some(p), None) if p.trim().is_empty() => bail!("--peer needs a computer's name or id"),
            (Some(p), None) => Ok(Scope::Peer(p.trim().to_owned())),
            (None, Some(o)) => Ok(Scope::Os(parse_os(o).map_err(anyhow::Error::msg)?)),
            (None, None) => Ok(Scope::Global),
        }
    }

    /// How specific the scope is for a computer (`None`: not for it):
    /// a computer beats an OS beats everything.
    fn rank(&self, peer: &PeerFacts) -> Option<u8> {
        match self {
            Scope::Local => None,
            Scope::Global => Some(0),
            Scope::Os(o) => (*o == peer.os).then_some(1),
            Scope::Peer(p) => peer_matches(p, peer.name, peer.id).then_some(2),
        }
    }
}

/// Like `TrustStore::find`: the name, or at least 4 characters of the id.
fn peer_matches(query: &str, name: &str, id: &str) -> bool {
    let q = query.replace('-', "").to_ascii_lowercase();
    name.eq_ignore_ascii_case(query) || (q.len() >= 4 && id.starts_with(&q))
}

pub fn parse_os(s: &str) -> Result<u8, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "mac" | "macos" | "osx" => Ok(os::MACOS),
        "windows" | "win" => Ok(os::WINDOWS),
        "linux" => Ok(os::LINUX),
        other => Err(format!("{other:?} is not an OS here: say mac, windows or linux")),
    }
}

fn os_name(o: u8) -> &'static str {
    match o {
        os::MACOS => "mac",
        os::WINDOWS => "windows",
        os::LINUX => "linux",
        _ => "unknown",
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Local => f.write_str("this computer (local)"),
            Scope::Global => f.write_str("every computer"),
            Scope::Os(o) => write!(f, "{} computers", os_name(*o)),
            Scope::Peer(p) => write!(f, "{p}"),
        }
    }
}

impl Serialize for Scope {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Scope::Local => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("local", &true)?;
                m.end()
            }
            Scope::Global => s.serialize_str("global"),
            Scope::Os(o) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("os", os_name(*o))?;
                m.end()
            }
            Scope::Peer(p) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("peer", p)?;
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Scope, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Scope;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(r#""global", { os = "mac|windows|linux" }, { peer = "computer" } or { local = true }"#)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Scope, E> {
                if v.trim().eq_ignore_ascii_case("global") {
                    Ok(Scope::Global)
                } else {
                    Err(E::custom(format!(r#"scope {v:?}: say "global", {{ os = "mac" }} or {{ peer = "name" }}"#)))
                }
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scope, A::Error> {
                let mut out = None;
                while let Some(key) = map.next_key::<String>()? {
                    if out.is_some() {
                        return Err(de::Error::custom("a scope has one entry: os, peer or local"));
                    }
                    if key == "local" {
                        if !map.next_value::<bool>()? {
                            return Err(de::Error::custom("local scope must be true"));
                        }
                        out = Some(Scope::Local);
                        continue;
                    }
                    let value: String = map.next_value()?;
                    out = Some(match key.as_str() {
                        "os" => Scope::Os(parse_os(&value).map_err(de::Error::custom)?),
                        "peer" if value.trim().is_empty() => return Err(de::Error::custom("scope peer is empty")),
                        "peer" => Scope::Peer(value.trim().to_owned()),
                        other => return Err(de::Error::custom(format!("scope {other:?}: say os or peer, or local = true"))),
                    });
                }
                out.ok_or_else(|| de::Error::custom("an empty scope: say os or peer, or local = true"))
            }
        }
        d.deserialize_any(V)
    }
}

// --------------------------------------------------------------- settings

/// `[input]` in `settings.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TuningSettings {
    #[serde(skip_serializing_if = "KeymapSettings::is_empty")]
    pub keymap: KeymapSettings,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pointer: Vec<PointerRule>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scroll: Vec<ScrollRule>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub button: Vec<ButtonRule>,
}

/// `[input.keymap]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapSettings {
    /// How long a tap/hold key waits before it counts as held (default 200).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tap_ms: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rule: Vec<KeyRule>,
}

impl TuningSettings {
    /// Whether any key or button rule runs a command (`run: ...`). Such a
    /// file can start programs, so it is held to the same permission rules
    /// as `[hooks]` (see `Settings::load`).
    pub fn runs_commands(&self) -> bool {
        let run = |s: &str| s.trim_start().to_ascii_lowercase().starts_with("run:");
        let to = |t: &To| match t {
            To::One(s) => run(s),
            To::Many(v) => v.iter().any(|s| run(s)),
        };
        self.keymap.rule.iter().any(|r| {
            r.to.as_ref().is_some_and(to) || r.tap.as_deref().is_some_and(run) || r.hold.as_deref().is_some_and(run)
        }) || self.button.iter().any(|b| to(&b.to))
    }
}

impl KeymapSettings {
    fn is_empty(&self) -> bool {
        self.tap_ms.is_none() && self.rule.is_empty()
    }

    pub fn tap(&self) -> Duration {
        Duration::from_millis(u64::from(self.tap_ms.unwrap_or(200).clamp(remap::MIN_TAP_MS, remap::MAX_TAP_MS)))
    }
}

/// A key or chord, or (an array) a sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
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

    fn describe(&self) -> String {
        match self {
            To::One(s) => s.clone(),
            To::Many(v) => format!("[{}]", v.join(", ")),
        }
    }
}

/// `[[input.keymap.rule]]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyRule {
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<To>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    /// The layer this rule belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Scope::is_global")]
    pub scope: Scope,
    /// Reserved for per-app rules (plan 4D): a rule with it is not used yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

impl KeyRule {
    /// The rule, parsed. Errors say what is wrong with it.
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

    fn describe(&self) -> String {
        let what = match (&self.to, &self.tap, &self.hold) {
            (Some(to), _, _) => format!("-> {}", to.describe()),
            (None, Some(t), Some(h)) => format!("tap {t}, hold {h}"),
            _ => "(incomplete)".into(),
        };
        let layer = self.layer.as_ref().map_or_else(String::new, |l| format!(" in layer {l}"));
        format!("{}{layer} {what}", self.from)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum AccelMode {
    System,
    Off,
    Curve,
}

impl fmt::Display for AccelMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AccelMode::System => "system",
            AccelMode::Off => "off",
            AccelMode::Curve => "curve",
        })
    }
}

/// `[[input.pointer]]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PointerRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceleration: Option<AccelMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel_gain: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel_exp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel_cap: Option<f64>,
    #[serde(default, skip_serializing_if = "Scope::is_global")]
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// Notched mouse wheels.
    Wheel,
    /// Touchpads and other smooth scrolling.
    Touchpad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ScrollMode {
    Auto,
    Smooth,
    Discrete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum NaturalSetting {
    On,
    Off,
    Auto,
}

/// `[[input.scroll]]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrollRule {
    /// Only for this kind of device; both if unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_vertical: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_horizontal: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural: Option<NaturalSetting>,
    /// Both axes; `speed_x`/`speed_y` override it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ScrollMode>,
    /// `shift = "horizontal"`, `alt = "x5"`, `ctrl = "unchanged"`...
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modifiers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Scope::is_global")]
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

/// The modifier groups `modifiers` may name, in `ScrollConfig` order.
const GROUPS: [&str; 4] = ["ctrl", "shift", "alt", "meta"];

/// `shift`, `cmd`... as a group index.
fn modifier_group(name: &str) -> Result<usize, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok(0),
        "shift" => Ok(1),
        "alt" | "option" => Ok(2),
        "meta" | "super" | "win" | "cmd" | "command" => Ok(3),
        other => Err(format!("modifier {other:?}: say ctrl, shift, alt or meta")),
    }
}

/// `horizontal`, `vertical`, `x5`, `unchanged`.
pub fn parse_mod_scroll(s: &str) -> Result<ModScroll, String> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "horizontal" => Ok(ModScroll::Horizontal),
        "vertical" | "vertical-only" => Ok(ModScroll::Vertical),
        "unchanged" | "none" | "normal" => Ok(ModScroll::Unchanged),
        _ => {
            let n = s.strip_prefix('x').or_else(|| s.strip_prefix("faster x")).unwrap_or(&s);
            match n.trim().parse::<f64>() {
                Ok(k) if (0.1..=scroll::MAX_MULTIPLIER).contains(&k) => Ok(ModScroll::Multiply(k)),
                Ok(_) => Err(format!("{s:?}: a multiplier goes from x0.1 to x{}", scroll::MAX_MULTIPLIER)),
                Err(_) => Err(format!("{s:?}: say horizontal, vertical, unchanged or a multiplier like x5")),
            }
        }
    }
}

/// A mouse button: a number (1 left, 2 right, 3 middle, 4 back, 5
/// forward...) or a name, possibly with modifiers (`ctrl+button1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ButtonFrom {
    Number(u32),
    Name(String),
}

impl ButtonFrom {
    fn trigger(&self) -> Result<Trigger, String> {
        let t = match self {
            ButtonFrom::Number(n) => keynames::button_code(*n)
                .map(|c| Trigger::key(c as u16))
                .ok_or_else(|| format!("button {n}: buttons are numbered 1 to {}", keynames::BUTTONS))?,
            ButtonFrom::Name(s) => match s.trim().parse::<u32>() {
                Ok(n) => return ButtonFrom::Number(n).trigger(),
                Err(_) => s.parse::<Trigger>().map_err(|e| e.0)?,
            },
        };
        if !keynames::is_button(u32::from(t.key)) {
            return Err(format!("{} is a key, not a mouse button (key rules go in [[input.keymap.rule]])", keynames::key_name(u32::from(t.key))));
        }
        Ok(t)
    }

    fn describe(&self) -> String {
        match self.trigger() {
            Ok(t) if !t.has_mods() => format!("button {}", keynames::button_number(u32::from(t.key)).unwrap_or(0)),
            Ok(t) => t.to_string(),
            Err(_) => match self {
                ButtonFrom::Number(n) => format!("button {n}"),
                ButtonFrom::Name(s) => s.clone(),
            },
        }
    }
}

/// `[[input.button]]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ButtonRule {
    pub from: ButtonFrom,
    pub to: To,
    #[serde(default, skip_serializing_if = "Scope::is_global")]
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

impl ButtonRule {
    fn spec(&self) -> Result<RuleSpec, String> {
        let from = self.from.trigger()?;
        Ok(RuleSpec { from, layer: None, what: What::To(self.to.parse().map_err(|e| format!("to: {e}"))?) })
    }
}

// ------------------------------------------------------------- resolving

/// The computer input goes to, as far as scopes care.
#[derive(Clone, Copy, Debug)]
pub struct PeerFacts<'a> {
    pub name: &'a str,
    /// Its full fingerprint, in hex.
    pub id: &'a str,
    pub os: u8,
}

/// Everything that applies to one computer.
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub keymap: Keymap,
    pub pointer: PointerTune,
    pub scroll: ScrollConfig,
    /// Rules that were skipped or are odd, for the log.
    pub warnings: Vec<String>,
}

/// A rule's place: `[input.keymap.rule] #3`, for messages.
fn place(table: &str, i: usize) -> String {
    format!("{table} #{}", i + 1)
}

impl TuningSettings {
    pub fn is_empty(&self) -> bool {
        self.keymap.rule.is_empty() && self.pointer.is_empty() && self.scroll.is_empty() && self.button.is_empty()
    }

    /// Key and button rules that apply to `peer`, most specific scope
    /// first, one per key, layer and modifiers; and what was skipped.
    fn specs(&self, peer: &PeerFacts) -> (Vec<RuleSpec>, Vec<String>) {
        self.specs_in(peer, false)
    }

    pub fn local_keymap(&self) -> Result<Keymap> {
        Ok(self.local_compiled()?.map)
    }

    fn local_compiled(&self) -> Result<remap::Compiled> {
        let peer = PeerFacts { name: "", id: "", os: os::local() };
        let (specs, warnings) = self.specs_in(&peer, true);
        if !warnings.is_empty() {
            bail!("{}", warnings.join("; "));
        }
        Ok(Keymap::compile(&specs, remap::Target { os: peer.os, swap: false }, self.keymap.tap())?)
    }

    fn specs_in(&self, peer: &PeerFacts, local: bool) -> (Vec<RuleSpec>, Vec<String>) {
        let mut warnings = Vec::new();
        let mut found: Vec<(u8, usize, RuleSpec, String)> = Vec::new();
        let keys = self.keymap.rule.iter().enumerate().map(|(i, r)| (place("input.keymap.rule", i), &r.scope, &r.app, r.spec()));
        let buttons = self.button.iter().enumerate().map(|(i, r)| (place("input.button", i), &r.scope, &r.app, r.spec()));
        for (n, (at, scope, app, spec)) in keys.chain(buttons.filter(|_| !local)).enumerate() {
            let rank = if local {
                if *scope != Scope::Local { continue; }
                0
            } else {
                let Some(rank) = scope.rank(peer) else { continue };
                rank
            };
            if app.is_some() {
                continue; // per-app rules: plan 4D
            }
            match spec {
                Ok(s) => found.push((rank, n, s, at)),
                Err(e) => warnings.push(format!("{at} skipped: {e}")),
            }
        }
        found.sort_by_key(|(rank, n, ..)| (std::cmp::Reverse(*rank), *n));
        let mut seen: HashMap<(Option<String>, Trigger), u8> = HashMap::new();
        let mut out = Vec::new();
        for (rank, _, spec, at) in found {
            match seen.get(&(spec.layer.clone(), spec.from)) {
                Some(&r) if r == rank => warnings.push(format!("{at}: another rule for {} in the same scope comes first", spec.from)),
                Some(_) => {} // overridden by a more specific scope
                None => {
                    seen.insert((spec.layer.clone(), spec.from), rank);
                    out.push(spec);
                }
            }
        }
        (out, warnings)
    }

    /// What applies to `peer`; `swap` says whether Ctrl and Cmd are
    /// swapped for it (after the mapper).
    pub fn resolve(&self, peer: &PeerFacts, swap: bool) -> Resolved {
        let (specs, mut warnings) = self.specs(peer);
        let target = remap::Target { os: peer.os, swap };
        let keymap = match Keymap::compile(&specs, target, self.keymap.tap()) {
            Ok(c) => {
                warnings.extend(c.warnings);
                c.map
            }
            Err(e) => {
                warnings.push(format!("key rules not used: {e}"));
                Keymap::default()
            }
        };
        let (pointer, w) = self.pointer_for(peer);
        warnings.extend(w);
        let (scroll, w) = self.scroll_for(peer);
        warnings.extend(w);
        Resolved { keymap, pointer, scroll, warnings }
    }

    /// Pointer rules for `peer`, field by field, the most specific last.
    fn pointer_for(&self, peer: &PeerFacts) -> (PointerTune, Vec<String>) {
        let mut rules: Vec<(u8, &PointerRule)> =
            self.pointer.iter().filter(|r| r.app.is_none()).filter_map(|r| Some((r.scope.rank(peer)?, r))).collect();
        rules.sort_by_key(|(rank, _)| *rank);
        let mut p = PointerRule::default();
        for (_, r) in rules {
            p.speed = r.speed.or(p.speed);
            p.acceleration = r.acceleration.or(p.acceleration);
            p.accel_gain = r.accel_gain.or(p.accel_gain);
            p.accel_exp = r.accel_exp.or(p.accel_exp);
            p.accel_cap = r.accel_cap.or(p.accel_cap);
        }
        let accel = match p.acceleration.unwrap_or(AccelMode::System) {
            AccelMode::System => Accel::System,
            AccelMode::Off => Accel::Off,
            AccelMode::Curve => Accel::curve(p.accel_gain.unwrap_or(0.05), p.accel_exp.unwrap_or(1.0), p.accel_cap.unwrap_or(3.0)),
        };
        let mut warnings = Vec::new();
        if accel.needs_raw() && !RAW_MOTION {
            warnings.push(format!("pointer acceleration {} can't be used here; the system's applies: {}", p.acceleration.unwrap_or(AccelMode::System), no_raw_motion_reason()));
        }
        (PointerTune::new(p.speed.unwrap_or(1.0), accel), warnings)
    }

    /// Scroll rules for `peer`: per device kind, field by field, the most
    /// specific (scope, then device) last.
    fn scroll_for(&self, peer: &PeerFacts) -> (ScrollConfig, Vec<String>) {
        let mut warnings = Vec::new();
        let mut rules: Vec<(u8, usize, &ScrollRule)> = self
            .scroll
            .iter()
            .enumerate()
            .filter(|(_, r)| r.app.is_none())
            .filter_map(|(i, r)| Some((r.scope.rank(peer)?, i, r)))
            .collect();
        rules.sort_by_key(|(rank, i, r)| (*rank, r.device.is_some(), *i));
        let params = |device: Device| {
            let mut r = ScrollRule::default();
            for (_, _, x) in rules.iter().filter(|(_, _, x)| x.device.is_none_or(|d| d == device)) {
                r.reverse_vertical = x.reverse_vertical.or(r.reverse_vertical);
                r.reverse_horizontal = x.reverse_horizontal.or(r.reverse_horizontal);
                r.natural = x.natural.or(r.natural);
                // A plain speed resets the axes set less specifically.
                if x.speed.is_some() {
                    (r.speed_x, r.speed_y) = (None, None);
                }
                r.speed = x.speed.or(r.speed);
                r.speed_x = x.speed_x.or(r.speed_x);
                r.speed_y = x.speed_y.or(r.speed_y);
                r.mode = x.mode.or(r.mode);
            }
            let natural = r.natural.map(|n| match n {
                NaturalSetting::On => Natural::On,
                NaturalSetting::Off => Natural::Off,
                NaturalSetting::Auto => Natural::Auto,
            });
            let flip = scroll::natural_flip(natural, os::local(), peer.os);
            let speed = |axis: Option<f64>| {
                pointer::clamp_or(axis.or(r.speed).unwrap_or(1.0), scroll::MIN_SPEED, scroll::MAX_SPEED, 1.0)
            };
            scroll::Params {
                reverse_x: r.reverse_horizontal.unwrap_or(false) != flip,
                reverse_y: r.reverse_vertical.unwrap_or(false) != flip,
                speed_x: speed(r.speed_x),
                speed_y: speed(r.speed_y),
                mode: match r.mode.unwrap_or(ScrollMode::Auto) {
                    ScrollMode::Auto => scroll::Mode::Auto,
                    ScrollMode::Smooth => scroll::Mode::Smooth,
                    ScrollMode::Discrete => scroll::Mode::Discrete,
                },
            }
        };
        let mut modifiers = [ModScroll::Unchanged; 4];
        for (_, i, r) in &rules {
            for (name, b) in &r.modifiers {
                match (modifier_group(name), parse_mod_scroll(b)) {
                    (Ok(g), Ok(m)) => modifiers[g] = m,
                    (Err(e), _) | (_, Err(e)) => warnings.push(format!("{} skipped modifier: {e}", place("input.scroll", *i))),
                }
            }
        }
        let cfg = ScrollConfig { wheel: params(Device::Wheel), smooth: params(Device::Touchpad), modifiers };
        (cfg, warnings)
    }

    /// Every problem, for `one-mk keymap check` and `one-mk doctor`:
    /// rules that can't be read, conflicts, rules nothing can reach,
    /// computers that aren't paired, settings this computer can't do.
    /// `paired` are the paired computers' names and ids.
    pub fn check(&self, paired: &[(String, String)]) -> Vec<String> {
        let mut out = Vec::new();
        // Every scope in use, checked as a computer it would apply to.
        let mut scopes: Vec<&Scope> = Vec::new();
        let all_scopes = self
            .keymap
            .rule
            .iter()
            .map(|r| (&r.scope, &r.app))
            .chain(self.button.iter().map(|r| (&r.scope, &r.app)))
            .chain(self.pointer.iter().map(|r| (&r.scope, &r.app)))
            .chain(self.scroll.iter().map(|r| (&r.scope, &r.app)));
        let mut apps = 0;
        for (scope, app) in all_scopes {
            if app.is_some() {
                apps += 1;
            }
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        if apps > 0 {
            out.push(format!("{apps} rule(s) have `app` set: per-app rules are not supported yet, so they are not used"));
        }
        for (i, r) in self.keymap.rule.iter().enumerate() {
            if let Err(e) = r.spec() {
                out.push(format!("{}: {e}", place("input.keymap.rule", i)));
            }
        }
        for (i, r) in self.button.iter().enumerate() {
            if let Err(e) = r.spec() {
                out.push(format!("{}: {e}", place("input.button", i)));
            }
        }
        for s in &scopes {
            if let Scope::Peer(p) = s
                && !paired.iter().any(|(name, id)| peer_matches(p, name, id))
            {
                out.push(format!("rules for {p:?} never apply: no paired computer is called that"));
            }
        }
        for s in &scopes {
            if **s == Scope::Local {
                if !cfg!(any(windows, target_os = "macos")) {
                    out.push("local rules need Windows or macOS and are ignored on this platform".into());
                } else {
                    out.push("local key rules are inactive: native hook integration is not implemented yet (plan 4C)".into());
                }
                match self.local_compiled() {
                    Ok(compiled) => out.extend(compiled.warnings),
                    Err(error) => out.push(format!("local rules: {error}")),
                }
                continue;
            }
            let facts = match s {
                Scope::Local => unreachable!(),
                Scope::Global => PeerFacts { name: "", id: "", os: os::UNKNOWN },
                Scope::Os(o) => PeerFacts { name: "", id: "", os: *o },
                Scope::Peer(p) => PeerFacts { name: p, id: "", os: os::UNKNOWN },
            };
            let (specs, warnings) = self.specs(&facts);
            out.extend(warnings.into_iter().filter(|w| !w.contains(" skipped: ")));
            let target = remap::Target { os: facts.os, swap: false };
            match Keymap::compile(&specs, target, self.keymap.tap()) {
                Ok(c) => out.extend(c.warnings),
                Err(e) => out.push(format!("for {s}: {e}")),
            }
            out.extend(self.scroll_for(&facts).1);
        }
        if self.button.iter().any(|rule| rule.scope == Scope::Local)
            || self.pointer.iter().any(|rule| rule.scope == Scope::Local)
            || self.scroll.iter().any(|rule| rule.scope == Scope::Local)
        {
            out.push("local scope is reserved for input.keymap.rule; local button, pointer and scroll rules are ignored".into());
        }
        for (i, r) in self.pointer.iter().enumerate() {
            if r.acceleration.is_some_and(|a| a != AccelMode::System) && !RAW_MOTION {
                out.push(format!("{}: acceleration {} can't be used on this computer, the system's applies", place("input.pointer", i), r.acceleration.unwrap()));
            }
            if let Some(s) = r.speed
                && !(pointer::MIN_SPEED..=pointer::MAX_SPEED).contains(&s)
            {
                out.push(format!("{}: speed {s} is outside 0.1 to 10 and is clamped", place("input.pointer", i)));
            }
        }
        for (i, r) in self.scroll.iter().enumerate() {
            for s in [r.speed, r.speed_x, r.speed_y].into_iter().flatten() {
                if !(scroll::MIN_SPEED..=scroll::MAX_SPEED).contains(&s) {
                    out.push(format!("{}: speed {s} is outside 0.1 to 10 and is clamped", place("input.scroll", i)));
                }
            }
        }
        if let Some(ms) = self.keymap.tap_ms
            && !(remap::MIN_TAP_MS..=remap::MAX_TAP_MS).contains(&ms)
        {
            out.push(format!("input.keymap tap_ms {ms} is outside {} to {} and is clamped", remap::MIN_TAP_MS, remap::MAX_TAP_MS));
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| seen.insert(p.clone()));
        out
    }

    /// Summary for `one-mk status`, with a separate local availability line.
    pub fn summary(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        if !self.keymap.rule.is_empty() {
            parts.push(n(self.keymap.rule.len(), "key rule", "key rules"));
        }
        if !self.button.is_empty() {
            parts.push(n(self.button.len(), "button rule", "button rules"));
        }
        if !self.pointer.is_empty() {
            parts.push(n(self.pointer.len(), "pointer setting", "pointer settings"));
        }
        if !self.scroll.is_empty() {
            parts.push(n(self.scroll.len(), "scroll setting", "scroll settings"));
        }
        let local = if cfg!(target_os = "linux") && self.keymap.rule.iter().any(|rule| rule.scope == Scope::Local) {
            "\nlocal key rules are not available on Linux"
        } else if self.keymap.rule.iter().any(|rule| rule.scope == Scope::Local) {
            "\nlocal key rules are inactive: native hook integration is not implemented yet (plan 4C)"
        } else {
            ""
        };
        Some(format!("Input tuning: {} (one-mk keymap, buttons, pointer, scroll).{local}", parts.join(", ")))
    }
}

// ------------------------------------------------------------ command line

/// Checks that `--peer` names a paired computer.
fn check_scope(dir: &Path, scope: &Scope) -> Result<()> {
    if let Scope::Peer(p) = scope
        && TrustStore::load(dir)?.find(p).is_none()
    {
        return Err(crate::manage::unknown_device(p));
    }
    Ok(())
}

fn paired(dir: &Path) -> Vec<(String, String)> {
    TrustStore::load(dir)
        .map(|t| t.devices().iter().map(|d| (d.name.clone(), d.fingerprint.clone())).collect())
        .unwrap_or_default()
}

fn scope_suffix(scope: &Scope) -> String {
    match scope {
        Scope::Global => String::new(),
        s => format!(" (for {s})"),
    }
}

#[derive(clap::Args, Debug, Default, Clone)]
pub struct ScopeArgs {
    /// Only for this paired computer (name or id).
    #[arg(long, value_name = "COMPUTER")]
    pub peer: Option<String>,
    /// Only for computers running this OS: mac, windows or linux.
    #[arg(long, value_name = "OS")]
    pub os: Option<String>,
}

impl ScopeArgs {
    fn selected(&self) -> Result<Scope> {
        Scope::from_args(self.peer.as_deref(), self.os.as_deref())
    }

    fn scope(&self, dir: &Path) -> Result<Scope> {
        let s = self.selected()?;
        check_scope(dir, &s)?;
        Ok(s)
    }
}

#[derive(clap::Args, Debug, Default, Clone)]
pub struct KeyScopeArgs {
    /// Reserve a local keyboard rule (inactive: native hooks are not wired yet).
    #[arg(long, conflicts_with_all = ["peer", "os"])]
    pub local: bool,
    #[command(flatten)]
    pub remote: ScopeArgs,
}

impl KeyScopeArgs {
    fn selected(&self) -> Result<Scope> {
        if self.local {
            if self.remote.peer.is_some() || self.remote.os.is_some() {
                bail!("say --local, --peer or --os, not more than one");
            }
            Ok(Scope::Local)
        } else {
            self.remote.selected()
        }
    }

    fn scope(&self, dir: &Path) -> Result<Scope> {
        let s = self.selected()?;
        check_scope(dir, &s)?;
        Ok(s)
    }
}

/// `one-mk keymap ...`.
#[derive(clap::Subcommand, Debug)]
pub enum KeymapCmd {
    /// Show the key rules.
    List,
    /// Make FROM (a key, or a chord like ctrl+h) type TO: a key, a chord
    /// (ctrl+shift+esc), several (a sequence), `none`, `layer:NAME`,
    /// `back`/`forward`, or an action (`switch next`, `lock-pointer`,
    /// `lock-all`, `scroll down`, `run: COMMAND`). Replaces a rule for the
    /// same key in the same scope and layer.
    Add {
        from: String,
        /// What it becomes; several make a sequence. Leave out with --tap and --hold.
        to: Vec<String>,
        /// Tapped, it types this (with --hold).
        #[arg(long, value_name = "KEY", requires = "hold")]
        tap: Option<String>,
        /// Held (or pressed with another key), this (with --tap).
        #[arg(long, value_name = "KEY", requires = "tap")]
        hold: Option<String>,
        /// The rule only applies while this layer is on.
        #[arg(long, value_name = "NAME")]
        layer: Option<String>,
        /// For --tap/--hold: how long before it counts as held (default 200 ms).
        #[arg(long, value_name = "MS")]
        timeout_ms: Option<u32>,
        #[command(flatten)]
        scope: KeyScopeArgs,
    },
    /// Remove the rule for FROM (in that scope and layer).
    Remove {
        from: String,
        #[arg(long, value_name = "NAME")]
        layer: Option<String>,
        #[command(flatten)]
        scope: KeyScopeArgs,
    },
    /// Remove every key rule.
    Clear,
    /// Check the rules (also the pointer, scroll and button settings).
    Check,
}

fn find_key_rule(rules: &[KeyRule], from: &Trigger, layer: &Option<String>, scope: &Scope) -> Option<usize> {
    let layer = layer.as_ref().map(|l| l.to_ascii_lowercase());
    rules.iter().position(|r| {
        r.scope == *scope
            && r.app.is_none()
            && r.layer.as_ref().map(|l| l.to_ascii_lowercase()) == layer
            && r.from.parse::<Trigger>().is_ok_and(|t| t == *from)
    })
}

fn list_keymap(s: &TuningSettings) {
    if s.keymap.rule.is_empty() {
        println!("No key rules: keys are sent as they are. Add one with `one-mk keymap add capslock esc`.");
        return;
    }
    println!("Key rules (remote unless marked local; local integration is not active yet):");
    for r in &s.keymap.rule {
        let app = if r.app.is_some() { " [app: not used yet]" } else { "" };
        println!("  {}{}{app}", r.describe(), scope_suffix(&r.scope));
    }
    if s.keymap.rule.iter().any(|rule| rule.scope == Scope::Local) {
        println!("Local key rules are inactive: native hook integration is not implemented yet (plan 4C).");
    }
    if let Some(ms) = s.keymap.tap_ms {
        println!("Tap/hold keys decide after {ms} ms.");
    }
}

pub async fn keymap(dir: &Path, cmd: Option<KeymapCmd>) -> Result<()> {
    match cmd.unwrap_or(KeymapCmd::List) {
        KeymapCmd::List => list_keymap(&Settings::load(dir)?.input),
        KeymapCmd::Check => return check_command(dir),
        KeymapCmd::Clear => {
            let n = Settings::update(dir, |s| Ok(std::mem::take(&mut s.input.keymap.rule).len()))?;
            println!("Removed {n} key rule(s).");
            settings::apply(dir).await?;
        }
        KeymapCmd::Add { from, to, tap, hold, layer, timeout_ms, scope } => {
            let scope = scope.scope(dir)?;
            let to = match to.len() {
                0 => None,
                1 => Some(To::One(to[0].clone())),
                _ => Some(To::Many(to)),
            };
            if to.is_some() && tap.is_some() {
                bail!("give TO, or --tap and --hold, not both");
            }
            let rule = KeyRule { from, to, tap, hold, layer, timeout_ms, scope: scope.clone(), app: None };
            let spec = rule.spec().map_err(anyhow::Error::msg)?;
            if keynames::is_button(u32::from(spec.from.key)) {
                bail!("{} is a mouse button: use `one-mk buttons map`", spec.from);
            }
            if let What::TapHold { .. } = spec.what {
                // Bounds and modifiers, checked by building it alone.
                Keymap::compile(std::slice::from_ref(&spec), remap::Target { os: os::UNKNOWN, swap: false }, Duration::from_millis(200))
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            let shown = rule.describe();
            Settings::update(dir, |s| {
                let rules = &mut s.input.keymap.rule;
                match find_key_rule(rules, &spec.from, &rule.layer, &scope) {
                    Some(i) => rules[i] = rule,
                    None => rules.push(rule),
                }
                // The whole keymap must still compile (layers, sizes).
                if scope == Scope::Local {
                    s.input.local_keymap()?;
                }
                let facts = PeerFacts { name: "", id: "", os: os::UNKNOWN };
                let (specs, _) = s.input.specs(&facts);
                Keymap::compile(&specs, remap::Target { os: os::UNKNOWN, swap: false }, s.input.keymap.tap())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(())
            })?;
            println!("{shown}{}.", scope_suffix(&scope));
            settings::apply(dir).await?;
        }
        KeymapCmd::Remove { from, layer, scope } => {
            let scope = scope.selected()?;
            let t: Trigger = from.parse().map_err(|e: keynames::NameError| anyhow::anyhow!("{e}"))?;
            let removed = Settings::update(dir, |s| {
                Ok(find_key_rule(&s.input.keymap.rule, &t, &layer, &scope).map(|i| s.input.keymap.rule.remove(i)))
            })?;
            match removed {
                Some(r) => println!("Removed {}{}.", r.describe(), scope_suffix(&scope)),
                None => bail!("there is no rule for {t}{}{} (see `one-mk keymap list`)", layer.map_or_else(String::new, |l| format!(" in layer {l}")), scope_suffix(&scope)),
            }
            settings::apply(dir).await?;
        }
    }
    Ok(())
}

/// `one-mk keymap check`: every problem, exit status 1 if any.
fn check_command(dir: &Path) -> Result<()> {
    let s = Settings::load(dir)?;
    let problems = s.input.check(&paired(dir));
    if s.input.is_empty() {
        println!("No input tuning is set.");
    }
    for p in &problems {
        println!("{p}");
    }
    if !problems.is_empty() {
        bail!("{} problem(s) with the input tuning in settings.toml", problems.len());
    }
    if !s.input.is_empty() {
        println!("The input tuning is fine.");
    }
    Ok(())
}

/// `one-mk pointer` options.
#[derive(clap::Args, Debug, Default)]
pub struct PointerArgs {
    /// How far the pointer moves on other computers, times this (0.1 to 10).
    #[arg(long, value_name = "X")]
    pub speed: Option<f64>,
    /// system (this computer's acceleration, the default), off or curve.
    /// off and curve need raw mouse motion, which no backend reports yet:
    /// they are refused (turn acceleration off in the OS instead).
    #[arg(long, value_enum, value_name = "MODE")]
    pub accel: Option<AccelMode>,
    /// Forget the pointer settings for this scope.
    #[arg(long)]
    pub reset: bool,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

fn describe_pointer(s: &TuningSettings) {
    if s.pointer.is_empty() {
        println!("Pointer: as on this computer (speed 1, this computer's acceleration).");
    }
    for r in &s.pointer {
        let mut parts = Vec::new();
        if let Some(v) = r.speed {
            parts.push(format!("speed {v}"));
        }
        if let Some(a) = r.acceleration {
            parts.push(format!("acceleration {a}"));
        }
        println!("Pointer{}: {}.", scope_suffix(&r.scope), if parts.is_empty() { "unchanged".into() } else { parts.join(", ") });
    }
}

pub async fn pointer_command(dir: &Path, args: PointerArgs) -> Result<()> {
    if args.speed.is_none() && args.accel.is_none() && !args.reset {
        describe_pointer(&Settings::load(dir)?.input);
        return Ok(());
    }
    let scope = args.scope.scope(dir)?;
    if let Some(v) = args.speed
        && !(v.is_finite() && (pointer::MIN_SPEED..=pointer::MAX_SPEED).contains(&v))
    {
        bail!("--speed {v}: say a number from 0.1 to 10");
    }
    if let Some(a) = args.accel
        && a != AccelMode::System
        && !RAW_MOTION
    {
        bail!("acceleration {a} is not available: {}", no_raw_motion_reason());
    }
    let s = Settings::update(dir, |s| {
        let rules = &mut s.input.pointer;
        let at = rules.iter().position(|r| r.scope == scope && r.app.is_none());
        if args.reset {
            if let Some(i) = at {
                rules.remove(i);
            }
            return Ok(s.input.clone());
        }
        let i = at.unwrap_or_else(|| {
            rules.push(PointerRule { scope: scope.clone(), ..Default::default() });
            rules.len() - 1
        });
        if let Some(v) = args.speed {
            rules[i].speed = (v != 1.0 || !scope.is_global()).then_some(v);
        }
        if let Some(a) = args.accel {
            rules[i].acceleration = (a != AccelMode::System || !scope.is_global()).then_some(a);
        }
        if rules[i] == (PointerRule { scope: scope.clone(), ..Default::default() }) {
            rules.remove(i);
        }
        Ok(s.input.clone())
    })?;
    describe_pointer(&s);
    settings::apply(dir).await
}

/// `one-mk scroll` options.
#[derive(clap::Args, Debug, Default)]
pub struct ScrollArgs {
    /// Flip vertical scrolling.
    #[arg(long, value_name = "on|off", value_parser = settings::parse_on_off)]
    pub reverse_vertical: Option<bool>,
    /// Flip horizontal scrolling.
    #[arg(long, value_name = "on|off", value_parser = settings::parse_on_off)]
    pub reverse_horizontal: Option<bool>,
    /// Scroll this much faster or slower (0.1 to 10).
    #[arg(long, value_name = "X")]
    pub speed: Option<f64>,
    /// auto (as it comes), smooth (in pixels) or discrete (whole notches).
    #[arg(long, value_enum)]
    pub mode: Option<ScrollMode>,
    /// Natural scrolling on the other computers: on, off, or auto (the
    /// other computer's own default: natural on a Mac, traditional elsewhere).
    /// Assumes this computer uses its OS's default direction.
    #[arg(long, value_enum)]
    pub natural: Option<NaturalSetting>,
    /// What holding a modifier does: shift=horizontal, alt=x5,
    /// ctrl=vertical, meta=unchanged (repeatable; unchanged removes it).
    #[arg(long, value_name = "MOD=WHAT")]
    pub modifier: Vec<String>,
    /// Only for wheels or only for touchpads.
    #[arg(long, value_enum)]
    pub device: Option<Device>,
    /// Forget the scroll settings for this scope (and device).
    #[arg(long)]
    pub reset: bool,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

fn describe_scroll(s: &TuningSettings) {
    if s.scroll.is_empty() {
        println!("Scrolling: as on this computer.");
    }
    let on = |b: bool| if b { "on" } else { "off" };
    for r in &s.scroll {
        let mut parts = Vec::new();
        if let Some(v) = r.reverse_vertical {
            parts.push(format!("reverse vertical {}", on(v)));
        }
        if let Some(v) = r.reverse_horizontal {
            parts.push(format!("reverse horizontal {}", on(v)));
        }
        if let Some(v) = r.natural {
            parts.push(format!("natural {v:?}").to_lowercase());
        }
        for (name, v) in [("speed", r.speed), ("speed x", r.speed_x), ("speed y", r.speed_y)] {
            if let Some(v) = v {
                parts.push(format!("{name} {v}"));
            }
        }
        if let Some(m) = r.mode {
            parts.push(format!("mode {m:?}").to_lowercase());
        }
        for (m, b) in &r.modifiers {
            parts.push(format!("{m}: {b}"));
        }
        let device = r.device.map_or("", |d| if d == Device::Wheel { " (wheels)" } else { " (touchpads)" });
        println!("Scrolling{device}{}: {}.", scope_suffix(&r.scope), if parts.is_empty() { "unchanged".into() } else { parts.join(", ") });
    }
}

impl ScrollArgs {
    fn changes_anything(&self) -> bool {
        self.reverse_vertical.is_some()
            || self.reverse_horizontal.is_some()
            || self.speed.is_some()
            || self.mode.is_some()
            || self.natural.is_some()
            || !self.modifier.is_empty()
            || self.reset
    }
}

pub async fn scroll_command(dir: &Path, args: ScrollArgs) -> Result<()> {
    if !args.changes_anything() {
        describe_scroll(&Settings::load(dir)?.input);
        return Ok(());
    }
    let scope = args.scope.scope(dir)?;
    if let Some(v) = args.speed
        && !(v.is_finite() && (scroll::MIN_SPEED..=scroll::MAX_SPEED).contains(&v))
    {
        bail!("--speed {v}: say a number from 0.1 to 10");
    }
    let mut mods = Vec::new();
    for m in &args.modifier {
        let Some((name, what)) = m.split_once('=') else {
            bail!("--modifier {m:?}: say MODIFIER=WHAT, like shift=horizontal or alt=x5");
        };
        let g = modifier_group(name).map_err(anyhow::Error::msg)?;
        let b = parse_mod_scroll(what).map_err(anyhow::Error::msg)?;
        mods.push((GROUPS[g].to_owned(), (b != ModScroll::Unchanged).then(|| what.trim().to_ascii_lowercase())));
    }
    let s = Settings::update(dir, |s| {
        let rules = &mut s.input.scroll;
        let at = rules.iter().position(|r| r.scope == scope && r.device == args.device && r.app.is_none());
        if args.reset {
            if let Some(i) = at {
                rules.remove(i);
            }
            return Ok(s.input.clone());
        }
        let i = at.unwrap_or_else(|| {
            rules.push(ScrollRule { scope: scope.clone(), device: args.device, ..Default::default() });
            rules.len() - 1
        });
        let r = &mut rules[i];
        r.reverse_vertical = args.reverse_vertical.or(r.reverse_vertical);
        r.reverse_horizontal = args.reverse_horizontal.or(r.reverse_horizontal);
        if args.speed.is_some() {
            (r.speed, r.speed_x, r.speed_y) = (args.speed, None, None);
        }
        r.mode = args.mode.or(r.mode);
        r.natural = args.natural.or(r.natural);
        for (name, what) in &mods {
            match what {
                Some(w) => r.modifiers.insert(name.clone(), w.clone()),
                None => r.modifiers.remove(name),
            };
        }
        if *r == (ScrollRule { scope: scope.clone(), device: args.device, ..Default::default() }) {
            rules.remove(i);
        }
        Ok(s.input.clone())
    })?;
    describe_scroll(&s);
    settings::apply(dir).await
}

/// `one-mk buttons ...`.
#[derive(clap::Subcommand, Debug)]
pub enum ButtonsCmd {
    /// Show the button rules.
    List,
    /// Make button N (1 left, 2 right, 3 middle, 4 back, 5 forward, 6 to 8)
    /// do TO: another button (button1), a key or chord, several (a
    /// sequence), back/forward, an action, or none.
    Map {
        button: String,
        #[arg(required = true)]
        to: Vec<String>,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Remove the rule for button N.
    Unmap {
        button: String,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// side-back-forward: buttons 4 and 5 go back and forward (Alt+Left/Right;
    /// Cmd+[ and Cmd+] on a Mac).
    Preset {
        #[arg(value_parser = ["side-back-forward"])]
        name: String,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Remove every button rule.
    Clear,
}

fn list_buttons(s: &TuningSettings) {
    if s.button.is_empty() {
        println!("No button rules: buttons are sent as they are.");
        return;
    }
    println!("Button rules:");
    for r in &s.button {
        println!("  {} -> {}{}", r.from.describe(), r.to.describe(), scope_suffix(&r.scope));
    }
}

fn button_from(s: &str) -> Result<(ButtonFrom, Trigger)> {
    let from = match s.trim().parse::<u32>() {
        Ok(n) => ButtonFrom::Number(n),
        Err(_) => ButtonFrom::Name(s.trim().to_owned()),
    };
    let t = from.trigger().map_err(anyhow::Error::msg)?;
    Ok((from, t))
}

fn set_button(rules: &mut Vec<ButtonRule>, rule: ButtonRule, t: &Trigger) {
    let at = rules.iter().position(|r| r.scope == rule.scope && r.app.is_none() && r.from.trigger().is_ok_and(|x| x == *t));
    match at {
        Some(i) => rules[i] = rule,
        None => rules.push(rule),
    }
}

pub async fn buttons(dir: &Path, cmd: Option<ButtonsCmd>) -> Result<()> {
    match cmd.unwrap_or(ButtonsCmd::List) {
        ButtonsCmd::List => {
            list_buttons(&Settings::load(dir)?.input);
            return Ok(());
        }
        ButtonsCmd::Map { button, to, scope } => {
            let scope = scope.scope(dir)?;
            let (from, t) = button_from(&button)?;
            let to = if to.len() == 1 { To::One(to[0].clone()) } else { To::Many(to) };
            let rule = ButtonRule { from, to, scope, app: None };
            rule.spec().map_err(anyhow::Error::msg)?;
            let line = format!("{} -> {}{}", rule.from.describe(), rule.to.describe(), scope_suffix(&rule.scope));
            Settings::update(dir, |s| {
                set_button(&mut s.input.button, rule, &t);
                Ok(())
            })?;
            println!("{line}.");
        }
        ButtonsCmd::Unmap { button, scope } => {
            let scope = scope.selected()?;
            let (_, t) = button_from(&button)?;
            let removed = Settings::update(dir, |s| {
                let at = s.input.button.iter().position(|r| r.scope == scope && r.from.trigger().is_ok_and(|x| x == t));
                Ok(at.map(|i| s.input.button.remove(i)))
            })?;
            if removed.is_none() {
                bail!("there is no rule for {button}{} (see `one-mk buttons list`)", scope_suffix(&scope));
            }
            println!("Button {button} is sent as it is again{}.", scope_suffix(&scope));
        }
        ButtonsCmd::Preset { name: _, scope } => {
            let scope = scope.scope(dir)?;
            Settings::update(dir, |s| {
                for (n, to) in [(4, "back"), (5, "forward")] {
                    let rule = ButtonRule { from: ButtonFrom::Number(n), to: To::One(to.into()), scope: scope.clone(), app: None };
                    set_button(&mut s.input.button, rule, &Trigger::key(keynames::button_code(n).unwrap() as u16));
                }
                Ok(())
            })?;
            println!("Buttons 4 and 5 go back and forward{} (Alt+Left/Right; Cmd+[ and Cmd+] on a Mac).", scope_suffix(&scope));
        }
        ButtonsCmd::Clear => {
            let n = Settings::update(dir, |s| Ok(std::mem::take(&mut s.input.button).len()))?;
            println!("Removed {n} button rule(s).");
        }
    }
    settings::apply(dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "abcdef0123456789";

    fn facts(os: u8) -> PeerFacts<'static> {
        PeerFacts { name: "Laptop", id: ID, os }
    }

    fn load(toml: &str) -> TuningSettings {
        let s: Settings = toml::from_str(toml).unwrap_or_else(|e| panic!("{e}"));
        s.input
    }

    #[test]
    fn the_documented_example_loads_and_resolves() {
        let t = load(
            r#"
[input.keymap]
tap_ms = 180

[[input.keymap.rule]]
from = "capslock"
tap = "esc"
hold = "lctrl"

[[input.keymap.rule]]
from = "lalt"
to = "lmeta"
scope = { os = "mac" }

[[input.keymap.rule]]
from = "ralt"
to = "layer:nav"

[[input.keymap.rule]]
from = "h"
to = "left"
layer = "nav"

[[input.keymap.rule]]
from = "f2"
to = ["ctrl+a", "ctrl+c"]

[[input.pointer]]
speed = 1.5
scope = { peer = "laptop" }

[[input.scroll]]
natural = "auto"
modifiers = { shift = "horizontal", alt = "x5" }

[[input.scroll]]
device = "wheel"
mode = "discrete"
speed = 2.0
scope = { os = "windows" }

[[input.button]]
from = 4
to = "back"
"#,
        );
        assert_eq!(t.keymap.tap(), Duration::from_millis(180));
        let mac = t.resolve(&facts(os::MACOS), false);
        assert!(mac.warnings.is_empty(), "{:?}", mac.warnings);
        assert_eq!(mac.pointer.speed(), 1.5);
        assert_eq!(mac.scroll.modifiers[1], ModScroll::Horizontal);
        assert_eq!(mac.scroll.modifiers[2], ModScroll::Multiply(5.0));
        let other = PeerFacts { name: "Desk", id: "ffff", os: os::WINDOWS };
        let win = t.resolve(&other, false);
        assert_eq!(win.pointer.speed(), 1.0, "not the laptop");
        assert_eq!(win.scroll.wheel.mode, scroll::Mode::Discrete);
        assert_eq!(win.scroll.wheel.speed_y, 2.0);
        assert_eq!(win.scroll.smooth.mode, scroll::Mode::Auto, "wheel only");
        // Round trip through the file format.
        let back = load(&toml::to_string_pretty(&Settings { input: t.clone(), ..Default::default() }).unwrap());
        assert_eq!(back, t);
    }

    #[test]
    fn scopes_parse_and_print() {
        let t = load(
            r#"
[[input.keymap.rule]]
from = "a"
to = "b"
scope = "global"

[[input.keymap.rule]]
from = "a"
to = "c"
scope = { peer = "Laptop" }
"#,
        );
        assert_eq!(t.keymap.rule[0].scope, Scope::Global);
        assert_eq!(t.keymap.rule[1].scope, Scope::Peer("Laptop".into()));
        for (bad, says) in [
            (r#"scope = "everywhere""#, "say \"global\""),
            (r#"scope = { os = "beos" }"#, "not an OS here"),
            (r#"scope = { planet = "mars" }"#, "say os or peer"),
            (r#"scope = { peer = "" }"#, "peer is empty"),
            (r#"scope = { os = "mac", peer = "x" }"#, "one entry"),
        ] {
            let toml = format!("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\n{bad}\n");
            let e = toml::from_str::<Settings>(&toml).unwrap_err().to_string();
            assert!(e.contains(says), "{bad}: {e}");
        }
        assert_eq!(Scope::from_args(None, Some("Mac")).unwrap(), Scope::Os(os::MACOS));
        assert!(Scope::from_args(Some("x"), Some("mac")).is_err());
    }

    #[test]
    fn local_scope_is_explicit_and_never_reaches_a_peer() {
        let tuning = load("[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"esc\"\nscope = { local = true }\n");
        assert_eq!(tuning.keymap.rule[0].scope, Scope::Local);
        for target in [os::LINUX, os::WINDOWS, os::MACOS] {
            assert!(tuning.resolve(&facts(target), false).keymap.is_empty());
        }
        assert!(!tuning.local_keymap().unwrap().is_empty());
        let encoded = toml::to_string(&Settings { input: tuning.clone(), ..Default::default() }).unwrap();
        assert_eq!(load(&encoded), tuning);
        assert!(toml::from_str::<Settings>("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nscope = { local = false }\n").is_err());
        if os::local() == os::LINUX {
            assert!(tuning.check(&[]).iter().any(|line| line.contains("local rules need Windows or macOS")));
        }
    }

    #[test]
    fn local_cli_scope_is_only_for_keys_and_cannot_select_a_peer() {
        use clap::Parser;

        #[derive(Parser)]
        struct Keys {
            #[command(subcommand)]
            command: KeymapCmd,
        }
        #[derive(Parser)]
        struct Remote {
            #[command(flatten)]
            scope: ScopeArgs,
        }

        let parsed = Keys::try_parse_from(["keymap", "add", "a", "b", "--local"]).unwrap();
        let KeymapCmd::Add { scope, .. } = parsed.command else { panic!("not an add") };
        assert_eq!(scope.selected().unwrap(), Scope::Local);
        let parsed = Keys::try_parse_from(["keymap", "remove", "a", "--local"]).unwrap();
        let KeymapCmd::Remove { scope, .. } = parsed.command else { panic!("not a removal") };
        assert_eq!(scope.selected().unwrap(), Scope::Local);
        for selector in ["--peer", "--os"] {
            assert!(Keys::try_parse_from(["keymap", "add", "a", "b", "--local", selector, "mac"]).is_err());
        }
        assert!(Remote::try_parse_from(["pointer", "--local"]).is_err());
        assert_eq!(Remote::try_parse_from(["pointer", "--os", "mac"]).unwrap().scope.selected().unwrap(), Scope::Os(os::MACOS));
    }

    #[test]
    fn local_validation_keeps_compile_warnings_and_rejects_invalid_rules() {
        let tuning = load("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nlayer = \"nav\"\nscope = { local = true }\n");
        assert!(tuning.check(&[]).iter().any(|line| line.contains("layer \"nav\" is never switched on")));
        let invalid = load("[[input.keymap.rule]]\nfrom = \"not-a-key\"\nto = \"b\"\nscope = { local = true }\n");
        assert!(invalid.local_keymap().is_err());
        let duplicate = load("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nscope = { local = true }\n[[input.keymap.rule]]\nfrom = \"a\"\nto = \"c\"\nscope = { local = true }\n");
        assert!(duplicate.local_keymap().is_err());
        assert!(duplicate.resolve(&facts(os::WINDOWS), false).keymap.is_empty());
        let mixed = "[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nscope = { local = true, os = \"windows\" }\n";
        assert!(toml::from_str::<Settings>(mixed).is_err());
    }

    #[test]
    fn local_status_has_one_explicit_availability_line() {
        let tuning = load("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nscope = { local = true }\n[[input.keymap.rule]]\nfrom = \"c\"\nto = \"d\"\nscope = { local = true }\n");
        let summary = tuning.summary().unwrap();
        assert_eq!(summary.lines().count(), 2);
        if cfg!(target_os = "linux") {
            assert_eq!(summary.lines().nth(1), Some("local key rules are not available on Linux"));
        } else {
            assert!(summary.contains("native hook integration is not implemented yet"));
        }
    }

    #[test]
    fn a_computer_beats_an_os_beats_everything() {
        let rule = |to: &str, scope: Scope| KeyRule { from: "capslock".into(), to: Some(To::One(to.into())), scope, ..Default::default() };
        let t = TuningSettings {
            keymap: KeymapSettings {
                rule: vec![rule("esc", Scope::Global), rule("lctrl", Scope::Peer("laptop".into())), rule("lmeta", Scope::Os(os::MACOS))],
                ..Default::default()
            },
            ..Default::default()
        };
        let first = |p: &PeerFacts| t.specs(p).0[0].what.clone();
        let to = |s: &str| What::To(s.parse().unwrap());
        assert_eq!(first(&facts(os::MACOS)), to("lctrl"));
        assert_eq!(first(&PeerFacts { name: "x", id: "0000", os: os::MACOS }), to("lmeta"));
        assert_eq!(first(&PeerFacts { name: "x", id: "0000", os: os::LINUX }), to("esc"));
        // By id prefix too.
        assert_eq!(first(&PeerFacts { name: "renamed", id: ID, os: os::LINUX }), to("esc"));
        let by_id = TuningSettings {
            keymap: KeymapSettings { rule: vec![rule("lctrl", Scope::Peer("abcd-ef01".into()))], ..Default::default() },
            ..Default::default()
        };
        assert_eq!(by_id.specs(&PeerFacts { name: "renamed", id: ID, os: os::LINUX }).0.len(), 1);
        assert!(t.specs(&facts(os::MACOS)).1.is_empty(), "overrides are not conflicts");
    }

    #[test]
    fn check_reports_conflicts_typos_and_unreachable_rules() {
        let t = load(
            r#"
[[input.keymap.rule]]
from = "capslock"
to = "esc"

[[input.keymap.rule]]
from = "CapsLock"
to = "lctrl"

[[input.keymap.rule]]
from = "capslok"
to = "esc"

[[input.keymap.rule]]
from = "j"
to = "down"
layer = "nav"

[[input.keymap.rule]]
from = "a"
to = "b"
scope = { peer = "nobody" }

[[input.keymap.rule]]
from = "a"
to = "b"
app = "firefox"

[[input.button]]
from = 9
to = "back"

[[input.scroll]]
modifiers = { hyper = "horizontal" }
speed = 20.0

[[input.pointer]]
acceleration = "off"
"#,
        );
        let problems = t.check(&[("Laptop".into(), ID.into())]).join("\n");
        for says in [
            "#2: another rule for capslock in the same scope comes first",
            "input.keymap.rule #3: from: \"capslok\" is not a key name",
            "layer \"nav\" is never switched on",
            "rules for \"nobody\" never apply",
            "1 rule(s) have `app` set",
            "input.button #1: button 9: buttons are numbered 1 to 8",
            "modifier \"hyper\"",
            "speed 20 is outside 0.1 to 10",
            "acceleration off can't be used",
        ] {
            assert!(problems.contains(says), "missing {says:?} in:\n{problems}");
        }
        assert!(TuningSettings::default().check(&[]).is_empty());
        // A broken rule costs only itself.
        let r = t.resolve(&facts(os::LINUX), false);
        assert!(!r.keymap.is_empty());
    }

    #[test]
    fn natural_auto_flips_only_across_mac_and_the_rest() {
        let t = load("[[input.scroll]]\nnatural = \"auto\"\n");
        let flipped = |o| t.resolve(&facts(o), false).scroll.wheel.reverse_y;
        let here_mac = os::local() == os::MACOS;
        assert_eq!(flipped(os::MACOS), !here_mac);
        assert_eq!(flipped(os::WINDOWS), here_mac);
        let both = load("[[input.scroll]]\nnatural = \"on\"\nreverse_vertical = true\n");
        // Two flips cancel out for a computer that isn't a Mac.
        assert_eq!(both.resolve(&facts(os::LINUX), false).scroll.wheel.reverse_y, here_mac);
    }

    #[test]
    fn modifier_scroll_values() {
        assert_eq!(parse_mod_scroll("Horizontal"), Ok(ModScroll::Horizontal));
        assert_eq!(parse_mod_scroll("x5"), Ok(ModScroll::Multiply(5.0)));
        assert_eq!(parse_mod_scroll("faster x5"), Ok(ModScroll::Multiply(5.0)));
        assert_eq!(parse_mod_scroll("0.5"), Ok(ModScroll::Multiply(0.5)));
        assert!(parse_mod_scroll("x500").unwrap_err().contains("from x0.1"));
        assert!(parse_mod_scroll("sideways").unwrap_err().contains("say horizontal"));
    }

    #[test]
    fn button_rules_take_numbers_and_names() {
        assert_eq!(ButtonFrom::Number(4).trigger(), Ok(Trigger::key(0x113)));
        assert_eq!(ButtonFrom::Name("5".into()).trigger(), Ok(Trigger::key(0x114)));
        assert_eq!(ButtonFrom::Name("ctrl+button1".into()).trigger().unwrap().key, 0x110);
        assert!(ButtonFrom::Name("a".into()).trigger().unwrap_err().contains("is a key, not a mouse button"));
        assert_eq!(ButtonFrom::Number(4).describe(), "button 4");
    }

    #[test]
    fn summary_only_when_something_is_set() {
        assert_eq!(TuningSettings::default().summary(), None);
        let t = load("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\n[[input.button]]\nfrom = 4\nto = \"back\"\n");
        assert_eq!(t.summary().unwrap(), "Input tuning: 1 key rule, 1 button rule (one-mk keymap, buttons, pointer, scroll).");
    }
}
