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
use one_mk_proto::remap::{self, Keymap, KeymapSet, Output, RuleSpec, What};
use one_mk_proto::scroll::{self, ModScroll, Natural, ScrollConfig};
use one_mk_proto::appmatch::AppPattern;
use one_mk_proto::device::{DeviceRef, Fingerprint, InputDevice};
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
    /// One keyboard or mouse (its fingerprint, as written), on the
    /// computers the inner scope names (`Global`, `Os`, `Peer` or `Local`).
    Device(String, Box<Scope>),
}

/// How much a device rule outranks the same rule without a device: any
/// device rule beats every rule for a computer or OS.
const DEVICE_RANK: u8 = 3;

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

    /// How specific the scope is for a computer and the device input comes
    /// from (`None`: not for them): a device (on a computer, on an OS, on
    /// any) beats a computer beats an OS beats everything. A device rule
    /// never applies to input whose device isn't known.
    fn rank(&self, peer: &PeerFacts) -> Option<u8> {
        match self {
            Scope::Local => None,
            Scope::Global => Some(0),
            Scope::Os(o) => (*o == peer.os).then_some(1),
            Scope::Peer(p) => peer_matches(p, peer.name, peer.id).then_some(2),
            Scope::Device(fp, on) => {
                let device = peer.device?;
                fp.parse::<Fingerprint>().ok()?.matches(device).then_some(())?;
                Some(on.rank(peer)? + DEVICE_RANK)
            }
        }
    }

    /// The computers the scope is for, leaving its device aside.
    pub fn target(&self) -> &Scope {
        match self {
            Scope::Device(_, on) => on,
            s => s,
        }
    }

    /// The device it is for, as written.
    pub fn device(&self) -> Option<&str> {
        match self {
            Scope::Device(fp, _) => Some(fp),
            _ => None,
        }
    }

    /// This scope for `device` only (`None`: for any device).
    pub fn for_device(self, device: Option<String>) -> Scope {
        match device {
            Some(d) => Scope::Device(d, Box::new(self)),
            None => self,
        }
    }
}

/// A device as people know it: its name, from its fingerprint.
pub fn device_name(fp: &str) -> String {
    match fp.parse::<Fingerprint>() {
        Ok(f) if !f.name.is_empty() => f.name,
        Ok(f) => format!("{} {}", f.bus.words(), f.ids()),
        Err(_) => fp.to_owned(),
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
            Scope::Device(fp, on) => match **on {
                Scope::Global => write!(f, "the device \"{}\"", device_name(fp)),
                ref on => write!(f, "the device \"{}\" on {on}", device_name(fp)),
            },
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
            Scope::Device(fp, on) => {
                let mut m = s.serialize_map(None)?;
                m.serialize_entry("device", fp)?;
                match &**on {
                    Scope::Local => m.serialize_entry("local", &true)?,
                    Scope::Os(o) => m.serialize_entry("os", os_name(*o))?,
                    Scope::Peer(p) => m.serialize_entry("peer", p)?,
                    _ => {}
                }
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
                f.write_str(r#""global", { os = "mac|windows|linux" }, { peer = "computer" }, { device = "..." } or { local = true }"#)
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
                let mut device: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "device" {
                        let value: String = map.next_value()?;
                        if device.is_some() {
                            return Err(de::Error::custom("a scope names one device"));
                        }
                        if value.trim().is_empty() {
                            return Err(de::Error::custom("scope device is empty"));
                        }
                        device = Some(value.trim().to_owned());
                        continue;
                    }
                    if out.is_some() {
                        return Err(de::Error::custom("a scope has one entry: os, peer or local (and maybe a device)"));
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
                        other => return Err(de::Error::custom(format!("scope {other:?}: say os, peer or device, or local = true"))),
                    });
                }
                match (out, device) {
                    (out, Some(d)) => Ok(Scope::Device(d, Box::new(out.unwrap_or_default()))),
                    (Some(s), None) => Ok(s),
                    (None, None) => Err(de::Error::custom("an empty scope: say os, peer or device, or local = true")),
                }
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
    pub local_rules: bool,
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
    /// Only while this app has the focus on the computer input goes to
    /// (see `one_mk_proto::appmatch`).
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
    /// The names of the app with the focus there, as it said (empty: it
    /// didn't, or can't tell; then no rule for an app applies).
    pub apps: &'a [String],
    /// The keyboard or mouse input comes from (`None`: not known; then no
    /// rule for a device applies).
    pub device: Option<&'a Fingerprint>,
}

/// How much a rule for an app outranks one without: any scope's rule for
/// the focused app beats every rule that names no app (device rules too).
const APP_RANK: u8 = 2 * DEVICE_RANK;

/// How a rule's `app` bears on `peer`: what it adds to the rule's rank if
/// it applies, `None` if it doesn't, an error if it can't be used.
fn app_rank(app: &Option<String>, peer: &PeerFacts) -> Result<Option<u8>, String> {
    let Some(a) = app else { return Ok(Some(0)) };
    let p: AppPattern = a.parse()?;
    Ok(p.matches(peer.apps).then_some(APP_RANK))
}

/// A rule's whole rank for `peer` (scope, then app), `None` if it doesn't
/// apply. A rule whose `app` can't be used never applies.
fn rank_for(scope: &Scope, app: &Option<String>, peer: &PeerFacts) -> Option<u8> {
    Some(scope.rank(peer)? + app_rank(app, peer).ok()??)
}

/// Whether two rules' `app` say the same (one rule replaces the other).
fn same_app(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
        _ => false,
    }
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

/// What applies to a computer, for input from each device with rules of
/// its own: the base (input from other devices, or not known), and per
/// group of devices the same rules apply to, what applies to them.
#[derive(Clone, Debug, Default)]
pub struct ResolvedSet {
    pub base: Resolved,
    pub devices: Vec<(Vec<DeviceRef>, Resolved)>,
}

/// A rule's place: `[input.keymap.rule] #3`, for messages.
fn place(table: &str, i: usize) -> String {
    format!("{table} #{}", i + 1)
}

impl TuningSettings {
    pub fn is_empty(&self) -> bool {
        !self.local_rules && self.keymap.rule.is_empty() && self.pointer.is_empty() && self.scroll.is_empty() && self.button.is_empty()
    }

    /// Key and button rules that apply to `peer`, most specific scope
    /// first, one per key, layer and modifiers; and what was skipped.
    fn specs(&self, peer: &PeerFacts) -> (Vec<RuleSpec>, Vec<String>) {
        self.specs_in(peer, false)
    }

    pub fn local_keymap(&self) -> Result<Keymap> {
        Ok(self.local_compiled()?.map)
    }

    pub fn local_keymaps(&self, devices: &[InputDevice]) -> Result<KeymapSet> {
        let mut set = KeymapSet::new(self.local_keymap()?);
        for device in devices {
            set.add(&[device.id], self.local_compiled_for(Some(&device.fingerprint))?.map);
        }
        Ok(set)
    }

    fn local_compiled(&self) -> Result<remap::Compiled> {
        self.local_compiled_for(None)
    }

    fn local_compiled_for(&self, device: Option<&Fingerprint>) -> Result<remap::Compiled> {
        let peer = PeerFacts { name: "", id: "", os: os::local(), apps: &[], device };
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
                if *scope.target() != Scope::Local {
                    continue;
                }
                if app.is_some() {
                    warnings.push(format!("{at}: --app is not supported for local rules; the app probe describes a remote computer"));
                    continue; // a local rule can't follow the focused app of another computer
                }
                match scope.device() {
                    Some(fingerprint) => {
                        let Some(device) = peer.device else { continue };
                        match fingerprint.parse::<Fingerprint>() {
                            Ok(fingerprint) if fingerprint.matches(device) => DEVICE_RANK,
                            _ => continue,
                        }
                    }
                    None => 0,
                }
            } else {
                let Some(rank) = scope.rank(peer) else { continue };
                match app_rank(app, peer) {
                    Ok(Some(r)) => rank + r,
                    Ok(None) => continue,
                    Err(e) => {
                        warnings.push(format!("{at} skipped: {e}"));
                        continue;
                    }
                }
            };
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

    /// What applies to `peer` for input from each of `devices` (the ones
    /// connected): devices that have rules of their own (matching, for
    /// that computer and app) get them on top of the rest; devices with
    /// the same such rules share one result.
    pub fn resolve_devices(&self, peer: &PeerFacts, swap: bool, devices: &[InputDevice]) -> ResolvedSet {
        let base = self.resolve(&PeerFacts { device: None, ..*peer }, swap);
        let mut groups: Vec<(Vec<usize>, Vec<DeviceRef>, &Fingerprint)> = Vec::new();
        for d in devices {
            let facts = PeerFacts { device: Some(&d.fingerprint), ..*peer };
            let which: Vec<usize> = self
                .scoped()
                .enumerate()
                .filter(|(_, (_, scope, app))| scope.device().is_some() && rank_for(scope, app, &facts).is_some())
                .map(|(i, _)| i)
                .collect();
            if which.is_empty() {
                continue;
            }
            match groups.iter_mut().find(|g| g.0 == which) {
                Some(g) => g.1.push(d.id),
                None => groups.push((which, vec![d.id], &d.fingerprint)),
            }
        }
        let devices = groups
            .into_iter()
            .map(|(_, ids, fp)| (ids, self.resolve(&PeerFacts { device: Some(fp), ..*peer }, swap)))
            .collect();
        ResolvedSet { base, devices }
    }

    /// Whether any rule is for one device.
    pub fn uses_devices(&self) -> bool {
        self.scoped().any(|(_, scope, _)| scope.device().is_some())
    }

    /// The devices rules name, as written, each once.
    pub fn devices_named(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, scope, _) in self.scoped() {
            if let Some(d) = scope.device()
                && !out.iter().any(|x| x == d)
            {
                out.push(d.to_owned());
            }
        }
        out
    }

    /// Pointer rules for `peer`, field by field, the most specific last.
    fn pointer_for(&self, peer: &PeerFacts) -> (PointerTune, Vec<String>) {
        let mut rules: Vec<(u8, &PointerRule)> =
            self.pointer.iter().filter_map(|r| Some((rank_for(&r.scope, &r.app, peer)?, r))).collect();
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
            .filter_map(|(i, r)| Some((rank_for(&r.scope, &r.app, peer)?, i, r)))
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

    /// Every rule's place, scope and app: key rules, pointer, scroll and
    /// button settings, in that order.
    fn scoped(&self) -> impl Iterator<Item = (String, &Scope, &Option<String>)> {
        let keys = self.keymap.rule.iter().enumerate().map(|(i, r)| (place("input.keymap.rule", i), &r.scope, &r.app));
        let pointer = self.pointer.iter().enumerate().map(|(i, r)| (place("input.pointer", i), &r.scope, &r.app));
        let scroll = self.scroll.iter().enumerate().map(|(i, r)| (place("input.scroll", i), &r.scope, &r.app));
        let buttons = self.button.iter().enumerate().map(|(i, r)| (place("input.button", i), &r.scope, &r.app));
        keys.chain(pointer).chain(scroll).chain(buttons)
    }

    /// Whether any rule names an app.
    pub fn uses_apps(&self) -> bool {
        self.scoped().any(|(_, _, app)| app.is_some())
    }

    /// The apps that rules for `peer` name (usable patterns, as written,
    /// each once). While there are any, the daemon asks `peer` which app
    /// has its focus; otherwise it never does.
    pub fn apps_for(&self, peer: &PeerFacts) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, scope, app) in self.scoped() {
            if let Some(a) = app
                && scope.target().rank(peer).is_some()
                && let Ok(p) = a.parse::<AppPattern>()
                && !out.contains(&p.to_string())
            {
                out.push(p.to_string());
            }
        }
        out
    }

    /// Every problem, for `one-mk keymap check` and `one-mk doctor`:
    /// rules that can't be read, conflicts, rules nothing can reach,
    /// computers that aren't paired, settings this computer can't do.
    /// `paired` are the paired computers' names, ids and OSes.
    pub fn check(&self, paired: &[(String, String, u8)]) -> Vec<String> {
        let mut out = Vec::new();
        // Every scope in use, checked as a computer it would apply to, and
        // every app named plainly, as the focused app there.
        let mut scopes: Vec<&Scope> = Vec::new();
        let mut plain_apps: Vec<String> = Vec::new();
        for (at, scope, app) in self.scoped() {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
            let Some(app) = app else { continue };
            let p = match app.parse::<AppPattern>() {
                Ok(p) => p,
                Err(e) => {
                    out.push(format!("{at}: {e}"));
                    continue;
                }
            };
            let target = match scope.target() {
                Scope::Global | Scope::Local | Scope::Device(..) => None,
                Scope::Os(o) => Some(*o),
                Scope::Peer(name) => paired.iter().find(|(n, id, _)| peer_matches(name, n, id)).map(|(.., o)| *o),
            };
            if let Some(why) = target.and_then(|o| p.never_on(o)) {
                out.push(format!("{at}: {why}"));
            }
            let plain = app.trim();
            if !p.is_negated() && !plain.contains('*') && !plain_apps.iter().any(|a| a.eq_ignore_ascii_case(plain)) {
                plain_apps.push(plain.to_owned());
            }
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
            if let Scope::Peer(p) = s.target()
                && !paired.iter().any(|(name, id, _)| peer_matches(p, name, id))
            {
                out.push(format!("rules for {p:?} never apply: no paired computer is called that"));
            }
            if let Some(d) = s.device()
                && let Err(e) = d.parse::<Fingerprint>()
            {
                out.push(format!("rules for the device {d:?} never apply: {e}"));
            }
        }
        let focused: Vec<Vec<String>> = std::iter::once(Vec::new()).chain(plain_apps.into_iter().map(|a| vec![a])).collect();
        for s in &scopes {
            if *s.target() == Scope::Local {
                if !cfg!(any(windows, target_os = "macos")) {
                    out.push("local rules need Windows or macOS and are ignored on this platform".into());
                } else {
                    out.push("local key rules are inactive: native hook integration is not implemented yet (5H); experimental, untested on real Windows/macOS hardware".into());
                }
                let device = s.device().and_then(|device| device.parse::<Fingerprint>().ok());
                match self.local_compiled_for(device.as_ref()) {
                    Ok(compiled) => out.extend(compiled.warnings),
                    Err(error) => out.push(format!("local rules: {error}")),
                }
                continue;
            }
            let device = s.device().and_then(|d| d.parse::<Fingerprint>().ok());
            if s.device().is_some() && device.is_none() {
                continue; // reported above
            }
            for apps in &focused {
                let mut facts = match s.target() {
                    Scope::Os(o) => PeerFacts { name: "", id: "", os: *o, apps, device: None },
                    Scope::Peer(p) => PeerFacts { name: p, id: "", os: os::UNKNOWN, apps, device: None },
                    _ => PeerFacts { name: "", id: "", os: os::UNKNOWN, apps, device: None },
                };
                facts.device = device.as_ref();
                let (specs, warnings) = self.specs(&facts);
                out.extend(warnings.into_iter().filter(|w| !w.contains(" skipped: ")));
                let target = remap::Target { os: facts.os, swap: false };
                let what = match apps.first() {
                    Some(a) => format!("{s}, in app {a}"),
                    None => s.to_string(),
                };
                match Keymap::compile(&specs, target, self.keymap.tap()) {
                    Ok(c) => out.extend(c.warnings),
                    Err(e) => out.push(format!("for {what}: {e}")),
                }
                out.extend(self.scroll_for(&facts).1);
            }
        }
        if self.button.iter().any(|rule| *rule.scope.target() == Scope::Local)
            || self.pointer.iter().any(|rule| *rule.scope.target() == Scope::Local)
            || self.scroll.iter().any(|rule| *rule.scope.target() == Scope::Local)
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
        if parts.is_empty() {
            parts.push("no rules".into());
        }
        let has_local = self.local_rules || self.keymap.rule.iter().any(|rule| *rule.scope.target() == Scope::Local);
        let local = if cfg!(target_os = "linux") && has_local {
            "\nlocal key rules are not available on Linux"
        } else if has_local {
            "\nlocal key rules are inactive: native hook integration is not implemented yet (5H); experimental, untested on real Windows/macOS hardware"
        } else {
            ""
        };
        Some(format!("Input tuning: {} (one-mk keymap, buttons, pointer, scroll).{local}", parts.join(", ")))
    }
}

// ------------------------------------------------------------ command line

/// Checks that `--peer` names a paired computer.
fn check_scope(dir: &Path, scope: &Scope) -> Result<()> {
    if let Scope::Peer(p) = scope.target()
        && TrustStore::load(dir)?.find(p).is_none()
    {
        return Err(crate::manage::unknown_device(p));
    }
    Ok(())
}

/// After a change to a rule for one device: says so if such rules can't
/// apply on this computer, or aren't tested here.
fn device_note(dir: &Path, scope: &Scope) {
    if *scope.target() == Scope::Local {
        println!("Note: local key rules remain inactive; native integration is incomplete and untested on real Windows/macOS hardware. Local key rules are not available on Linux.");
        return;
    }
    if scope.device().is_none() {
        return;
    }
    let seen = crate::inputdevices::Remembered::load(dir).unwrap_or_default();
    match crate::inputdevices::support(&seen, false) {
        Ok(None) => {}
        Ok(Some(untested)) => println!("Note: per-device rules here are {untested}."),
        Err(why) => println!("Note: {why}, so this rule doesn't apply here. It is kept: it works in an X11 session, on Windows and on macOS."),
    }
}

fn paired(dir: &Path) -> Vec<(String, String, u8)> {
    TrustStore::load(dir)
        .map(|t| t.devices().iter().map(|d| (d.name.clone(), d.fingerprint.clone(), d.os)).collect())
        .unwrap_or_default()
}

/// A rule's `app` in words: "only in app code", "in every app but code".
pub fn app_words(app: &str) -> String {
    let app = app.trim();
    match app.strip_prefix('!') {
        Some(not) => format!("in every app but {}", not.trim_start()),
        None => format!("only in app {app}"),
    }
}

/// Where a rule applies, as a suffix: " (for Laptop, only in app code)".
fn where_suffix(scope: &Scope, app: &Option<String>) -> String {
    match (scope, app) {
        (Scope::Global, None) => String::new(),
        (Scope::Global, Some(a)) => format!(" ({})", app_words(a)),
        (s, None) => format!(" (for {s})"),
        (s, Some(a)) => format!(" (for {s}, {})", app_words(a)),
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
    /// Only while this app has the focus on the other computer: a program
    /// file on Windows (chrome.exe), a bundle id on a Mac
    /// (com.apple.Terminal), a window class on Linux (firefox). `*` matches
    /// anything, `!` in front means every app but this one.
    #[arg(long, value_name = "NAME")]
    pub app: Option<String>,
    /// Only for input from this keyboard or mouse, on this computer: its
    /// name (or a part of it) or fingerprint, as `one-mk input-devices`
    /// lists it. Combines with --peer or --os.
    #[arg(long, value_name = "NAME|FINGERPRINT")]
    pub device: Option<String>,
}

/// The fingerprint `--device` means: one of the devices seen here, or of
/// the devices rules already name (so a rule for a device long gone can
/// still be removed), or a fingerprint written out.
fn device_arg(dir: &Path, query: &str) -> Result<String> {
    let mut known = crate::inputdevices::Remembered::load(dir).unwrap_or_default();
    if let Ok(s) = Settings::load(dir) {
        for fp in s.input.devices_named() {
            if !known.devices.iter().any(|d| d.fingerprint == fp) {
                known.devices.push(crate::inputdevices::Seen { fingerprint: fp, ..Default::default() });
            }
        }
    }
    known.find(query)
}

impl ScopeArgs {
    /// The scope asked for; `--device` is looked up in the devices seen.
    fn selected(&self, dir: &Path) -> Result<Scope> {
        let device = self.device.as_deref().map(|d| device_arg(dir, d)).transpose()?;
        Ok(Scope::from_args(self.peer.as_deref(), self.os.as_deref())?.for_device(device))
    }

    fn scope(&self, dir: &Path) -> Result<Scope> {
        let s = self.selected(dir)?;
        check_scope(dir, &s)?;
        Ok(s)
    }

    /// `--app`, checked; also says when it can't match on the scope's OS.
    fn app(&self) -> Result<Option<String>> {
        let Some(a) = &self.app else { return Ok(None) };
        let p: AppPattern = a.parse().map_err(anyhow::Error::msg)?;
        if let Some(o) = self.os.as_deref().and_then(|o| parse_os(o).ok())
            && let Some(why) = p.never_on(o)
        {
            bail!("{why}");
        }
        Ok(Some(p.to_string()))
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
    fn selected(&self, dir: &Path) -> Result<Scope> {
        if self.local {
            if self.remote.peer.is_some() || self.remote.os.is_some() {
                bail!("say --local, or --peer and --os, not both; --device may be used with either");
            }
            let device = self.remote.device.as_deref().map(|device| device_arg(dir, device)).transpose()?;
            Ok(Scope::Local.for_device(device))
        } else {
            self.remote.selected(dir)
        }
    }

    fn scope(&self, dir: &Path) -> Result<Scope> {
        let s = self.selected(dir)?;
        check_scope(dir, &s)?;
        Ok(s)
    }

    /// `--app`, checked. A local rule (this computer's own keyboard) can't
    /// follow the app focused on another computer, so `--local --app` is refused.
    fn app(&self) -> Result<Option<String>> {
        if self.local && self.remote.app.is_some() {
            bail!("--app is for rules about another computer; a --local rule applies to this computer's own keyboard");
        }
        self.remote.app()
    }
}

/// `one-mk keymap ...`.
#[derive(clap::Subcommand, Debug)]
pub enum KeymapCmd {
    /// Show the key rules.
    List,
    /// Store the experimental local-key preference. Native activation is not
    /// implemented yet; Windows/macOS are untested on real hardware.
    Local {
        #[arg(value_name = "on|off", value_parser = settings::parse_on_off, action = clap::ArgAction::Set)]
        enabled: bool,
    },
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

fn find_key_rule(rules: &[KeyRule], from: &Trigger, layer: &Option<String>, scope: &Scope, app: &Option<String>) -> Option<usize> {
    let layer = layer.as_ref().map(|l| l.to_ascii_lowercase());
    rules.iter().position(|r| {
        r.scope == *scope
            && same_app(&r.app, app)
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
        let state = if *r.scope.target() == Scope::Local {
            if cfg!(target_os = "linux") { " [inactive: local key rules are not available on Linux]" }
            else { " [inactive: native integration incomplete; experimental, untested on real hardware]" }
        } else { "" };
        println!("  {}{}{state}", r.describe(), where_suffix(&r.scope, &r.app));
    }
    if s.keymap.rule.iter().any(|rule| *rule.scope.target() == Scope::Local) {
        println!("Local key rules are inactive: native hook integration is not implemented yet (5H; experimental, untested on real Windows/macOS hardware).");
    }
    if let Some(ms) = s.keymap.tap_ms {
        println!("Tap/hold keys decide after {ms} ms.");
    }
}

pub async fn keymap(dir: &Path, cmd: Option<KeymapCmd>) -> Result<()> {
    match cmd.unwrap_or(KeymapCmd::List) {
        KeymapCmd::List => list_keymap(&Settings::load(dir)?.input),
        KeymapCmd::Local { enabled } => {
            Settings::update(dir, |settings| {
                settings.input.local_rules = enabled;
                Ok(())
            })?;
            println!("Local key preference {}. Rules remain inactive: {}.", if enabled { "on" } else { "off" },
                if cfg!(target_os = "linux") { "local key rules are not available on Linux" }
                else { "native integration incomplete; experimental, untested on real Windows/macOS hardware" });
            settings::apply(dir).await?;
        }
        KeymapCmd::Check => return check_command(dir),
        KeymapCmd::Clear => {
            let n = Settings::update(dir, |s| Ok(std::mem::take(&mut s.input.keymap.rule).len()))?;
            println!("Removed {n} key rule(s).");
            settings::apply(dir).await?;
        }
        KeymapCmd::Add { from, to, tap, hold, layer, timeout_ms, scope } => {
            let app = scope.app()?;
            let scope = scope.scope(dir)?;
            let to = match to.len() {
                0 => None,
                1 => Some(To::One(to[0].clone())),
                _ => Some(To::Many(to)),
            };
            if to.is_some() && tap.is_some() {
                bail!("give TO, or --tap and --hold, not both");
            }
            let rule = KeyRule { from, to, tap, hold, layer, timeout_ms, scope: scope.clone(), app: app.clone() };
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
                match find_key_rule(rules, &spec.from, &rule.layer, &scope, &app) {
                    Some(i) => rules[i] = rule,
                    None => rules.push(rule),
                }
                // The whole keymap must still compile (layers, sizes), also
                // with the rule's app focused.
                if *scope.target() == Scope::Local {
                    let device = scope.device().map(str::parse::<Fingerprint>).transpose().map_err(anyhow::Error::msg)?;
                    s.input.local_compiled_for(device.as_ref())?;
                    s.input.local_keymaps(&[])?;
                }
                let focused: Vec<String> = app.iter().filter(|a| !a.starts_with('!') && !a.contains('*')).cloned().collect();
                let device = scope.device().and_then(|d| d.parse::<Fingerprint>().ok());
                let facts = PeerFacts { name: "", id: "", os: os::UNKNOWN, apps: &focused, device: device.as_ref() };
                let (specs, _) = s.input.specs(&facts);
                Keymap::compile(&specs, remap::Target { os: os::UNKNOWN, swap: false }, s.input.keymap.tap())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(())
            })?;
            println!("{shown}{}.", where_suffix(&scope, &app));
            device_note(dir, &scope);
            settings::apply(dir).await?;
        }
        KeymapCmd::Remove { from, layer, scope } => {
            let app = scope.app()?;
            let scope = scope.selected(dir)?;
            let t: Trigger = from.parse().map_err(|e: keynames::NameError| anyhow::anyhow!("{e}"))?;
            let removed = Settings::update(dir, |s| {
                Ok(find_key_rule(&s.input.keymap.rule, &t, &layer, &scope, &app).map(|i| s.input.keymap.rule.remove(i)))
            })?;
            match removed {
                Some(r) => println!("Removed {}{}.", r.describe(), where_suffix(&r.scope, &r.app)),
                None => bail!("there is no rule for {t}{}{} (see `one-mk keymap list`)", layer.map_or_else(String::new, |l| format!(" in layer {l}")), where_suffix(&scope, &app)),
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
        println!("Pointer{}: {}.", where_suffix(&r.scope, &r.app), if parts.is_empty() { "unchanged".into() } else { parts.join(", ") });
    }
}

pub async fn pointer_command(dir: &Path, args: PointerArgs) -> Result<()> {
    if args.speed.is_none() && args.accel.is_none() && !args.reset {
        describe_pointer(&Settings::load(dir)?.input);
        return Ok(());
    }
    let app = args.scope.app()?;
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
        let at = rules.iter().position(|r| r.scope == scope && same_app(&r.app, &app));
        if args.reset {
            if let Some(i) = at {
                rules.remove(i);
            }
            return Ok(s.input.clone());
        }
        let blank = PointerRule { scope: scope.clone(), app: app.clone(), ..Default::default() };
        let i = at.unwrap_or_else(|| {
            rules.push(blank.clone());
            rules.len() - 1
        });
        // The defaults matter only where they override something.
        let overrides = !scope.is_global() || app.is_some();
        if let Some(v) = args.speed {
            rules[i].speed = (v != 1.0 || overrides).then_some(v);
        }
        if let Some(a) = args.accel {
            rules[i].acceleration = (a != AccelMode::System || overrides).then_some(a);
        }
        if rules[i] == blank {
            rules.remove(i);
        }
        Ok(s.input.clone())
    })?;
    describe_pointer(&s);
    device_note(dir, &scope);
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
    /// Only for wheels or only for touchpads (`--device wheel|touchpad`
    /// still works too).
    #[arg(long, value_enum)]
    pub kind: Option<Device>,
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
        println!("Scrolling{device}{}: {}.", where_suffix(&r.scope, &r.app), if parts.is_empty() { "unchanged".into() } else { parts.join(", ") });
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

pub async fn scroll_command(dir: &Path, mut args: ScrollArgs) -> Result<()> {
    // `--device wheel|touchpad` named the kind of scrolling before devices
    // could be named: it still does.
    if let Some(kind) = args.scope.device.as_deref().and_then(|d| <Device as clap::ValueEnum>::from_str(d.trim(), true).ok()) {
        if args.kind.is_some() {
            bail!("--device {kind:?} is the kind of scrolling, as --kind: say it once", kind = args.scope.device.as_deref().unwrap_or_default());
        }
        args.kind = Some(kind);
        args.scope.device = None;
    }
    if !args.changes_anything() {
        describe_scroll(&Settings::load(dir)?.input);
        return Ok(());
    }
    let app = args.scope.app()?;
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
        let at = rules.iter().position(|r| r.scope == scope && r.device == args.kind && same_app(&r.app, &app));
        if args.reset {
            if let Some(i) = at {
                rules.remove(i);
            }
            return Ok(s.input.clone());
        }
        let blank = ScrollRule { scope: scope.clone(), device: args.kind, app: app.clone(), ..Default::default() };
        let i = at.unwrap_or_else(|| {
            rules.push(blank.clone());
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
        if *r == blank {
            rules.remove(i);
        }
        Ok(s.input.clone())
    })?;
    describe_scroll(&s);
    device_note(dir, &scope);
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
        println!("  {} -> {}{}", r.from.describe(), r.to.describe(), where_suffix(&r.scope, &r.app));
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
    let at = rules.iter().position(|r| r.scope == rule.scope && same_app(&r.app, &rule.app) && r.from.trigger().is_ok_and(|x| x == *t));
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
            let app = scope.app()?;
            let scope = scope.scope(dir)?;
            let (from, t) = button_from(&button)?;
            let to = if to.len() == 1 { To::One(to[0].clone()) } else { To::Many(to) };
            let rule = ButtonRule { from, to, scope, app };
            rule.spec().map_err(anyhow::Error::msg)?;
            let line = format!("{} -> {}{}", rule.from.describe(), rule.to.describe(), where_suffix(&rule.scope, &rule.app));
            let scope = rule.scope.clone();
            Settings::update(dir, |s| {
                set_button(&mut s.input.button, rule, &t);
                Ok(())
            })?;
            println!("{line}.");
            device_note(dir, &scope);
        }
        ButtonsCmd::Unmap { button, scope } => {
            let app = scope.app()?;
            let scope = scope.selected(dir)?;
            let (_, t) = button_from(&button)?;
            let removed = Settings::update(dir, |s| {
                let at = s.input.button.iter().position(|r| {
                    r.scope == scope && same_app(&r.app, &app) && r.from.trigger().is_ok_and(|x| x == t)
                });
                Ok(at.map(|i| s.input.button.remove(i)))
            })?;
            if removed.is_none() {
                bail!("there is no rule for {button}{} (see `one-mk buttons list`)", where_suffix(&scope, &app));
            }
            println!("Button {button} is sent as it is again{}.", where_suffix(&scope, &app));
        }
        ButtonsCmd::Preset { name: _, scope } => {
            let app = scope.app()?;
            let scope = scope.scope(dir)?;
            Settings::update(dir, |s| {
                for (n, to) in [(4, "back"), (5, "forward")] {
                    let rule = ButtonRule { from: ButtonFrom::Number(n), to: To::One(to.into()), scope: scope.clone(), app: app.clone() };
                    set_button(&mut s.input.button, rule, &Trigger::key(keynames::button_code(n).unwrap() as u16));
                }
                Ok(())
            })?;
            println!("Buttons 4 and 5 go back and forward{} (Alt+Left/Right; Cmd+[ and Cmd+] on a Mac).", where_suffix(&scope, &app));
            device_note(dir, &scope);
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
        PeerFacts { name: "Laptop", id: ID, os, apps: &[], device: None }
    }

    /// What the rule for `from` does for `peer`, if there is one.
    fn rule_for(t: &TuningSettings, peer: &PeerFacts, from: &str) -> Option<What> {
        let from: Trigger = from.parse().unwrap();
        t.specs(peer).0.into_iter().find(|s| s.from == from && s.layer.is_none()).map(|s| s.what)
    }

    fn with_app<'a>(os: u8, apps: &'a [String]) -> PeerFacts<'a> {
        PeerFacts { apps, ..facts(os) }
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_owned()).collect()
    }

    const PER_APP: &str = r#"
[[input.keymap.rule]]
from = "ctrl+space"
to = "f13"
app = "code"
scope = { os = "mac" }

[[input.keymap.rule]]
from = "capslock"
to = "esc"

[[input.keymap.rule]]
from = "capslock"
to = "lctrl"
app = "com.microsoft.VSCode"

[[input.keymap.rule]]
from = "f1"
to = "f2"
app = "!code"

[[input.keymap.rule]]
from = "f3"
to = "f4"
app = "co\u0007de"

[[input.pointer]]
speed = 2.0
app = "game*"

[[input.scroll]]
speed = 3.0
app = "chrome"

[[input.button]]
from = 4
to = "back"
app = "firefox"
"#;

    /// A rule with `app` applies only while the computer input goes to
    /// says that app has the focus; it beats the same key's rule without
    /// one; unknown (not said, or can't tell) means it doesn't apply, `!`
    /// rules included.
    #[test]
    fn per_app_rules_apply_only_while_their_app_has_the_focus() {
        let t = load(PER_APP);
        let to = |s: &str| Some(What::To(s.parse().unwrap()));
        let unknown = with_app(os::MACOS, &[]);
        assert_eq!(rule_for(&t, &unknown, "capslock"), to("esc"));
        assert_eq!(rule_for(&t, &unknown, "ctrl+space"), None);
        assert_eq!(rule_for(&t, &unknown, "f1"), None, "a ! rule needs a known app too");
        assert_eq!(rule_for(&t, &unknown, "button4"), None);
        let r = t.resolve(&unknown, false);
        assert_eq!(r.pointer.speed(), 1.0);
        assert_eq!(r.scroll.wheel.speed_y, 1.0);

        let vscode = names(&["com.microsoft.VSCode", "Code"]);
        let mac = with_app(os::MACOS, &vscode);
        assert_eq!(rule_for(&t, &mac, "ctrl+space"), to("f13"));
        assert_eq!(rule_for(&t, &mac, "capslock"), to("lctrl"), "the app's rule beats the general one");
        assert_eq!(rule_for(&t, &mac, "f1"), None);
        assert!(t.specs(&mac).1.iter().all(|w| !w.contains("same scope")), "{:?}", t.specs(&mac).1);

        let code = names(&["code", "Code"]);
        assert_eq!(rule_for(&t, &with_app(os::LINUX, &code), "ctrl+space"), None, "for Macs only");
        let vim = names(&["vim"]);
        assert_eq!(rule_for(&t, &with_app(os::LINUX, &vim), "f1"), to("f2"));
        assert_eq!(rule_for(&t, &with_app(os::LINUX, &vim), "capslock"), to("esc"));

        let game = names(&["GameLauncher.exe", "UnityWndClass"]);
        assert_eq!(t.resolve(&with_app(os::WINDOWS, &game), false).pointer.speed(), 2.0);
        let chrome = names(&["chrome.exe"]);
        assert_eq!(t.resolve(&with_app(os::WINDOWS, &chrome), false).scroll.wheel.speed_y, 3.0);
        let firefox = names(&["firefox", "Firefox"]);
        assert_eq!(rule_for(&t, &with_app(os::LINUX, &firefox), "button4"), to("back"));
        // A pattern that can't be meant costs only its rule.
        let w = t.resolve(&with_app(os::LINUX, &vim), false).warnings.join("\n");
        assert!(w.contains("input.keymap.rule #5 skipped: app: control"), "{w}");
        // Which app patterns could matter to a computer.
        assert_eq!(t.apps_for(&facts(os::MACOS)), ["code", "com.microsoft.VSCode", "!code", "game*", "chrome", "firefox"]);
        assert_eq!(t.apps_for(&facts(os::LINUX)).len(), 5);
        assert!(load("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\n").apps_for(&facts(os::LINUX)).is_empty());
        // Round trip through the file format.
        let back = load(&toml::to_string_pretty(&Settings { input: t.clone(), ..Default::default() }).unwrap());
        assert_eq!(back, t);
    }

    /// Random keys, timer runs and changes of the focused app (to apps
    /// with rules of their own, apps without, and unknown), resolved as the
    /// daemon does and handed to the mapper the way a reload is: the
    /// target never sees a key released that isn't down or pressed twice
    /// (but by a repeat), and nothing is left down once every key is up.
    #[test]
    fn app_changes_never_leave_a_key_stuck() {
        use one_mk_proto::remap::{Emit, Mapper};
        use std::collections::BTreeSet;
        let t = load(
            r#"
[[input.keymap.rule]]
from = "capslock"
tap = "esc"
hold = "lctrl"

[[input.keymap.rule]]
from = "capslock"
to = "lmeta"
app = "code"

[[input.keymap.rule]]
from = "ralt"
to = "layer:nav"

[[input.keymap.rule]]
from = "h"
to = "left"
layer = "nav"

[[input.keymap.rule]]
from = "ralt"
tap = "esc"
hold = "layer:nav"
app = "firefox"

[[input.keymap.rule]]
from = "h"
to = "ctrl+shift+tab"
layer = "nav"
app = "firefox"

[[input.keymap.rule]]
from = "lshift"
tap = "lshift"
hold = "layer:nav"
app = "!code"

[[input.keymap.rule]]
from = "a"
to = ["ctrl+a", "lshift"]
app = "game*"

[[input.button]]
from = 1
to = "button3"
app = "code"
"#,
        );
        let apps: [&[&str]; 5] = [&[], &["code", "Code"], &["firefox"], &["gamelauncher.exe"], &["vim"]];
        let apps: Vec<Vec<String>> = apps.iter().map(|a| names(a)).collect();
        let keymaps: Vec<_> = apps.iter().map(|a| t.resolve(&with_app(os::LINUX, a), false).keymap).collect();
        for (i, k) in keymaps.iter().enumerate() {
            assert!(!k.is_empty(), "{:?}", apps[i]);
        }
        // What the target sees: every up for a key down, a down for a key
        // already down only by a repeat.
        fn wire(down: &mut BTreeSet<u32>, out: Vec<Emit>, repeat: bool, stream: usize) {
            for e in out {
                if let Emit::Key { code, down: d } = e {
                    if d {
                        assert!(down.insert(code) || repeat, "stream {stream}: {code} pressed twice");
                    } else {
                        assert!(down.remove(&code), "stream {stream}: {code} released but not down");
                    }
                }
            }
        }
        let keys = [58, 100, 35, 42, 30, 29, 0x110, 15];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut rng = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for stream in 0..3000 {
            let mut m = Mapper::new(keymaps[0].clone());
            let mut down = BTreeSet::new();
            let mut phys = BTreeSet::new();
            let mut now = std::time::Instant::now();
            for _ in 0..rng(120) + 1 {
                let mut out = Vec::new();
                let mut repeat = false;
                match rng(10) {
                    0..=5 => {
                        let k = keys[rng(keys.len())];
                        let press = rng(2) == 0;
                        repeat = press && phys.contains(&k);
                        if press {
                            phys.insert(k);
                        } else {
                            phys.remove(&k);
                        }
                        m.key(k, press, now, &mut |e| out.push(e));
                    }
                    6 => {
                        now += Duration::from_millis(rng(300) as u64);
                        m.timer(now, &mut |e| out.push(e));
                    }
                    _ => m.set_keymap(keymaps[rng(keymaps.len())].clone(), &mut |e| out.push(e)),
                }
                wire(&mut down, out, repeat, stream);
            }
            for k in std::mem::take(&mut phys) {
                let mut out = Vec::new();
                m.key(k, false, now, &mut |e| out.push(e));
                wire(&mut down, out, false, stream);
            }
            assert!(down.is_empty(), "stream {stream}: left down {down:?}");
            assert!(m.is_idle());
        }
    }

    /// Among rules for an app, the more specific scope wins, as without.
    #[test]
    fn per_app_rules_keep_scope_order() {
        let t = load(
            r#"
[[input.keymap.rule]]
from = "a"
to = "b"
app = "code"

[[input.keymap.rule]]
from = "a"
to = "c"
app = "code"
scope = { peer = "laptop" }

[[input.keymap.rule]]
from = "a"
to = "d"
scope = { peer = "laptop" }
"#,
        );
        let code = names(&["code"]);
        assert_eq!(rule_for(&t, &with_app(os::LINUX, &code), "a"), Some(What::To("c".parse().unwrap())));
        let other = PeerFacts { name: "Desk", id: "ffff", os: os::LINUX, apps: &code, device: None };
        assert_eq!(rule_for(&t, &other, "a"), Some(What::To("b".parse().unwrap())));
        let vim = names(&["vim"]);
        assert_eq!(rule_for(&t, &with_app(os::LINUX, &vim), "a"), Some(What::To("d".parse().unwrap())));
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
        let other = PeerFacts { name: "Desk", id: "ffff", os: os::WINDOWS, apps: &[], device: None };
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
            (r#"scope = { planet = "mars" }"#, "say os, peer or device"),
            (r#"scope = { device = "" }"#, "device is empty"),
            (r#"scope = { device = "usb/0001:0002//k", os = "mac", peer = "x" }"#, "one entry"),
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
    fn local_device_rules_round_trip_override_and_never_reach_peers() {
        let tuning = load(&format!("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\nscope = {{ local = true }}\n[[input.keymap.rule]]\nfrom = \"a\"\nto = \"c\"\nscope = {{ local = true, device = \"{TRAVEL}\" }}\n"));
        let travel = fp(TRAVEL);
        let other = fp(BIG);
        let local = facts(os::local());
        let specs = |device| tuning.specs_in(&PeerFacts { device, ..local }, true).0;
        assert_eq!(specs(None)[0].what, What::To("b".parse().unwrap()));
        assert_eq!(specs(Some(&travel))[0].what, What::To("c".parse().unwrap()));
        assert_eq!(specs(Some(&other))[0].what, What::To("b".parse().unwrap()));
        let connected = InputDevice {
            id: DeviceRef(1), fingerprint: travel.clone(), kinds: one_mk_proto::device::Kinds { keyboard: true, pointer: false },
        };
        let set = tuning.local_keymaps(&[connected]).unwrap();
        assert_ne!(set.for_device(Some(DeviceRef(1))), set.base());
        assert_eq!(set.for_device(Some(DeviceRef(2))), set.base());
        for target in [os::LINUX, os::WINDOWS, os::MACOS] {
            assert!(tuning.resolve(&device(facts(target), &travel), false).keymap.is_empty());
        }
        let encoded = toml::to_string(&Settings { input: tuning.clone(), ..Default::default() }).unwrap();
        assert_eq!(load(&encoded), tuning);
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
        assert_eq!(scope.selected(Path::new("/nonexistent")).unwrap(), Scope::Local);
        let parsed = Keys::try_parse_from(["keymap", "remove", "a", "--local"]).unwrap();
        let KeymapCmd::Remove { scope, .. } = parsed.command else { panic!("not a removal") };
        assert_eq!(scope.selected(Path::new("/nonexistent")).unwrap(), Scope::Local);
        for selector in ["--peer", "--os"] {
            assert!(Keys::try_parse_from(["keymap", "add", "a", "b", "--local", selector, "mac"]).is_err());
        }
        let parsed = Keys::try_parse_from(["keymap", "add", "a", "b", "--local", "--device", TRAVEL]).unwrap();
        let KeymapCmd::Add { scope, .. } = parsed.command else { panic!("not an add") };
        assert_eq!(scope.selected(Path::new("/nonexistent")).unwrap(), Scope::Local.for_device(Some(TRAVEL.into())));
        let parsed = Keys::try_parse_from(["keymap", "add", "a", "b", "--local", "--app", "editor"]).unwrap();
        let KeymapCmd::Add { scope, .. } = parsed.command else { panic!("not an add") };
        assert!(scope.app().is_err());
        assert!(Remote::try_parse_from(["pointer", "--local"]).is_err());
        assert_eq!(Remote::try_parse_from(["pointer", "--os", "mac"]).unwrap().scope.selected(Path::new("/nonexistent")).unwrap(), Scope::Os(os::MACOS));
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
    fn local_preference_upgrade_and_device_validation() {
        assert!(!load("").local_rules);
        let enabled = load("[input]\nlocal_rules = true\n");
        assert!(enabled.local_rules);
        let encoded = toml::to_string(&Settings { input: enabled.clone(), ..Default::default() }).unwrap();
        assert_eq!(load(&encoded), enabled);
        let invalid = load(&format!("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"not-a-key\"\nscope = {{ local = true, device = \"{TRAVEL}\" }}\n"));
        assert!(invalid.check(&[]).iter().any(|line| line.starts_with("local rules:")));
        let app = load("[[input.keymap.rule]]\nfrom = \"a\"\nto = \"b\"\napp = \"editor\"\nscope = { local = true }\n");
        assert!(app.local_keymap().unwrap_err().to_string().contains("--app is not supported"));
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
        assert_eq!(first(&PeerFacts { name: "x", id: "0000", os: os::MACOS, apps: &[], device: None }), to("lmeta"));
        assert_eq!(first(&PeerFacts { name: "x", id: "0000", os: os::LINUX, apps: &[], device: None }), to("esc"));
        // By id prefix too.
        assert_eq!(first(&PeerFacts { name: "renamed", id: ID, os: os::LINUX, apps: &[], device: None }), to("esc"));
        let by_id = TuningSettings {
            keymap: KeymapSettings { rule: vec![rule("lctrl", Scope::Peer("abcd-ef01".into()))], ..Default::default() },
            ..Default::default()
        };
        assert_eq!(by_id.specs(&PeerFacts { name: "renamed", id: ID, os: os::LINUX, apps: &[], device: None }).0.len(), 1);
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
app = "chrome.exe"
scope = { os = "mac" }

[[input.keymap.rule]]
from = "c"
to = "d"
app = "com.apple.Terminal"
scope = { peer = "Laptop" }

[[input.keymap.rule]]
from = "e"
to = "f"
app = ""

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
        let problems = t.check(&[("Laptop".into(), ID.into(), os::WINDOWS)]).join("\n");
        for says in [
            "#2: another rule for capslock in the same scope comes first",
            "input.keymap.rule #3: from: \"capslok\" is not a key name",
            "layer \"nav\" is never switched on",
            "rules for \"nobody\" never apply",
            "input.keymap.rule #6: app \"chrome.exe\" never matches on a Mac",
            "input.keymap.rule #7: app \"com.apple.Terminal\" never matches on Windows",
            "input.keymap.rule #8: app: give a program name",
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
    const TRAVEL: &str = "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard";
    const BIG: &str = "usb/046d:c31c//Logitech USB Keyboard";

    fn fp(s: &str) -> Fingerprint {
        s.parse().unwrap()
    }

    fn device<'a>(peer: PeerFacts<'a>, d: &'a Fingerprint) -> PeerFacts<'a> {
        PeerFacts { device: Some(d), ..peer }
    }

    #[test]
    fn device_scopes_parse_print_and_combine() {
        let t = load(&format!(
            r#"
[[input.keymap.rule]]
from = "capslock"
to = "esc"
scope = {{ device = "{TRAVEL}" }}

[[input.keymap.rule]]
from = "lalt"
to = "lmeta"
scope = {{ device = "{TRAVEL}", peer = "Laptop" }}

[[input.pointer]]
speed = 2.0
scope = {{ os = "mac", device = "{BIG}" }}
"#
        ));
        assert_eq!(t.keymap.rule[0].scope, Scope::Device(TRAVEL.into(), Box::new(Scope::Global)));
        assert_eq!(t.keymap.rule[1].scope, Scope::Device(TRAVEL.into(), Box::new(Scope::Peer("Laptop".into()))));
        assert_eq!(t.pointer[0].scope, Scope::Device(BIG.into(), Box::new(Scope::Os(os::MACOS))));
        assert_eq!(t.keymap.rule[1].scope.to_string(), "the device \"Travel Keyboard\" on Laptop");
        assert_eq!(t.pointer[0].scope.to_string(), "the device \"Logitech USB Keyboard\" on mac computers");
        let back = load(&toml::to_string_pretty(&Settings { input: t.clone(), ..Default::default() }).unwrap());
        assert_eq!(back, t, "round trip");
        assert!(t.uses_devices());
        assert_eq!(t.devices_named(), [TRAVEL, BIG]);
    }

    /// device + computer beats device + OS beats device beats computer
    /// beats OS beats everything; a rule for the focused app beats them all.
    #[test]
    fn device_rules_rank_above_computer_rules() {
        let t = load(&format!(
            r#"
[[input.keymap.rule]]
from = "a"
to = "b"

[[input.keymap.rule]]
from = "a"
to = "c"
scope = {{ os = "linux" }}

[[input.keymap.rule]]
from = "a"
to = "d"
scope = {{ peer = "Laptop" }}

[[input.keymap.rule]]
from = "a"
to = "e"
scope = {{ device = "{TRAVEL}" }}

[[input.keymap.rule]]
from = "a"
to = "f"
scope = {{ device = "{TRAVEL}", os = "linux" }}

[[input.keymap.rule]]
from = "a"
to = "g"
scope = {{ device = "{TRAVEL}", peer = "Laptop" }}

[[input.keymap.rule]]
from = "a"
to = "h"
app = "code"
"#
        ));
        let to = |s: &str| Some(What::To(s.parse().unwrap()));
        let travel = fp(TRAVEL);
        let big = fp(BIG);
        let laptop = facts(os::LINUX);
        let desk = PeerFacts { name: "Desk", id: "", ..facts(os::WINDOWS) };
        assert_eq!(rule_for(&t, &laptop, "a"), to("d"), "unknown device: no device rule");
        assert_eq!(rule_for(&t, &device(laptop, &big), "a"), to("d"), "another device");
        assert_eq!(rule_for(&t, &device(laptop, &travel), "a"), to("g"));
        assert_eq!(rule_for(&t, &device(PeerFacts { name: "Other", id: "", ..facts(os::LINUX) }, &travel), "a"), to("f"));
        assert_eq!(rule_for(&t, &device(desk, &travel), "a"), to("e"));
        assert_eq!(rule_for(&t, &desk, "a"), to("b"));
        let code = names(&["code"]);
        assert_eq!(rule_for(&t, &device(with_app(os::LINUX, &code), &travel), "a"), to("h"), "the app's rule beats a device's");
        assert!(t.specs(&device(laptop, &travel)).1.is_empty(), "no two rules in one scope: {:?}", t.specs(&device(laptop, &travel)).1);
    }

    /// Devices with the same rules share one result; devices without get
    /// the base; a device's pointer and scroll settings stay its own.
    #[test]
    fn resolving_for_the_connected_devices() {
        use one_mk_proto::device::{DeviceRef, Kinds};
        let t = load(&format!(
            r#"
[[input.keymap.rule]]
from = "capslock"
to = "esc"
scope = {{ device = "usb/1234:0001//Twin" }}

[[input.pointer]]
speed = 3.0
scope = {{ device = "{BIG}" }}

[[input.scroll]]
reverse_vertical = true
scope = {{ device = "{BIG}" }}

[[input.keymap.rule]]
from = "a"
to = "b"
"#
        ));
        let d = |id: u32, f: &str| InputDevice { id: DeviceRef(id), fingerprint: fp(f), kinds: Kinds { keyboard: true, pointer: true } };
        let devices = [d(1, "usb/1234:0001//Twin"), d(2, "usb/1234:0001//twin"), d(3, BIG), d(4, TRAVEL)];
        let r = t.resolve_devices(&facts(os::LINUX), false, &devices);
        assert_eq!(r.devices.len(), 2, "the twins share, the travel keyboard has no rules");
        assert_eq!(r.devices[0].0, [DeviceRef(1), DeviceRef(2)]);
        assert_eq!(r.devices[1].0, [DeviceRef(3)]);
        assert_eq!((r.base.pointer.speed(), r.devices[1].1.pointer.speed(), r.devices[0].1.pointer.speed()), (1.0, 3.0, 1.0));
        assert!(r.devices[1].1.scroll.wheel.reverse_y && !r.base.scroll.wheel.reverse_y);
        assert_ne!(r.devices[0].1.keymap, r.base.keymap);
        assert!(t.resolve_devices(&facts(os::LINUX), false, &[d(4, TRAVEL)]).devices.is_empty());
    }

    #[test]
    fn checking_device_rules() {
        let t = load(
            r#"
[[input.keymap.rule]]
from = "a"
to = "b"
scope = { device = "not a device" }

[[input.keymap.rule]]
from = "c"
to = "d"
scope = { device = "usb/0001:0002//Pad", peer = "Nowhere" }

[[input.keymap.rule]]
from = "e"
to = "layer:nav"
scope = { device = "usb/0001:0002//Pad" }
"#,
        );
        let p = t.check(&[]).join("\n");
        assert!(p.contains("rules for the device \"not a device\" never apply: "), "{p}");
        assert!(p.contains("rules for \"Nowhere\" never apply"), "{p}");
        assert!(p.contains("layer \"nav\" has no rules"), "the device's keymap is compiled too: {p}");
    }

}
