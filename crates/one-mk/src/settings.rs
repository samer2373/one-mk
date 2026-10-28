//! Settings that are not about one paired computer: the edge rules, gaming
//! mode and lock sync. They live in `settings.toml` next to `devices.toml`,
//! are changed with `one-mk edge`, `one-mk gaming` and `one-mk lock-sync`,
//! and a running daemon applies them at once (like `one-mk layout`).

use crate::control::{self, Request};
use anyhow::{Result, bail};
use one_mk_proto::{EdgeGuard, Modifier, Side};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

const FILE: &str = "settings.toml";
/// Bounds for the edge rules; larger values are almost surely typos.
pub const MAX_CORNER: u32 = 500;
pub const MAX_DWELL_MS: u32 = 5000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub edge: EdgeSettings,
    pub gaming: Gaming,
    /// Locking any computer locks the others.
    pub lock_sync: bool,
}

/// Rules for crossing edges: for every edge, and optionally overridden for
/// one (`[edge.left]` etc.).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeSettings {
    #[serde(flatten)]
    pub all: Rules,
    /// Programs that may always be left, fullscreen or not.
    pub fullscreen_allow: Vec<String>,
    /// Programs that are never left while they have the focus.
    pub fullscreen_block: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Rules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Rules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<Rules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Rules>,
}

/// Edge rules; anything unset falls back to the rules for every edge (and
/// those to off).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dwell_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fullscreen_guard: Option<bool>,
}

impl Rules {
    fn is_empty(&self) -> bool {
        *self == Rules::default()
    }

    fn modifier(&self) -> Option<Modifier> {
        // Written by `one-mk edge`, which checks it; a bad hand edit is off.
        self.modifier.as_deref().and_then(|m| m.parse().ok())
    }

    /// In words, for `one-mk status`; empty if nothing is set. With
    /// `overrides` (one edge's rules), a rule turned off is said too.
    fn describe(&self, overrides: bool) -> String {
        let mut parts = Vec::new();
        match self.corner {
            Some(0) if overrides => parts.push("no dead corners".to_owned()),
            Some(n) if n > 0 => parts.push(format!("dead corners {n} px")),
            _ => {}
        }
        match self.dwell_ms {
            Some(0) if overrides => parts.push("no dwell".to_owned()),
            Some(ms) if ms > 0 => parts.push(format!("dwell {ms} ms")),
            _ => {}
        }
        match self.modifier() {
            Some(Modifier::None) if overrides => parts.push("no key to hold".to_owned()),
            Some(m) if m != Modifier::None => parts.push(format!("hold {m}")),
            _ => {}
        }
        match self.fullscreen_guard {
            Some(true) => parts.push("fullscreen guard".into()),
            Some(false) if overrides => parts.push("no fullscreen guard".into()),
            _ => {}
        }
        parts.join(", ")
    }
}

impl EdgeSettings {
    pub fn side(&self, side: Side) -> Option<&Rules> {
        match side {
            Side::Left => self.left.as_ref(),
            Side::Right => self.right.as_ref(),
            Side::Top => self.top.as_ref(),
            Side::Bottom => self.bottom.as_ref(),
        }
    }

    fn side_mut(&mut self, side: Side) -> &mut Option<Rules> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
            Side::Top => &mut self.top,
            Side::Bottom => &mut self.bottom,
        }
    }

    /// The rules in force for crossing `side`.
    pub fn guard(&self, side: Side) -> EdgeGuard {
        let one = self.side(side);
        let pick = |f: fn(&Rules) -> Option<u32>| one.and_then(f).or_else(|| f(&self.all)).unwrap_or(0);
        EdgeGuard {
            corner: pick(|r| r.corner).min(MAX_CORNER),
            dwell_ms: pick(|r| r.dwell_ms).min(MAX_DWELL_MS),
            modifier: one.and_then(Rules::modifier).or_else(|| self.all.modifier()).unwrap_or_default(),
            fullscreen: one.and_then(|r| r.fullscreen_guard).or(self.all.fullscreen_guard).unwrap_or(false),
        }
    }

    /// In words, for `one-mk status`; empty if no rule is on.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        let all = self.all.describe(false);
        if !all.is_empty() {
            parts.push(all);
        }
        for side in Side::ALL {
            if let Some(d) = self.side(side).map(|r| r.describe(true)).filter(|d| !d.is_empty()) {
                parts.push(format!("{side} edge: {d}"));
            }
        }
        let uses_guard = self.all.fullscreen_guard == Some(true)
            || Side::ALL.iter().any(|&s| self.side(s).is_some_and(|r| r.fullscreen_guard == Some(true)));
        if uses_guard && !self.fullscreen_allow.is_empty() {
            parts.push(format!("fullscreen allowed for {}", self.fullscreen_allow.join(" ")));
        }
        if uses_guard && !self.fullscreen_block.is_empty() {
            parts.push(format!("never leaving {}", self.fullscreen_block.join(" ")));
        }
        // Status lines are tab-separated.
        parts.join("; ").replace(['\t', '\n'], " ")
    }
}

/// Relative mouse movement for games.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Gaming {
    /// Absolute positions, like a normal mouse (the default).
    #[default]
    Off,
    /// Relative movement whenever the pointer is on another computer.
    On,
    /// Relative movement while the other computer has a fullscreen window
    /// in front (where it can tell: Windows, and X11 windows on Linux).
    Auto,
}

impl fmt::Display for Gaming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Gaming::Off => "off",
            Gaming::On => "on",
            Gaming::Auto => "auto",
        })
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

impl Settings {
    /// The saved settings; defaults if there are none. A file that can't be
    /// read is reported and ignored, so a bad hand edit never stops one-mk.
    pub fn load(dir: &Path) -> Settings {
        match std::fs::read_to_string(path(dir)) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("ignoring {}: {e}", path(dir).display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = path(dir).with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path(dir))?;
        Ok(())
    }
}

/// `one-mk edge` options.
#[derive(clap::Args, Debug, Default)]
pub struct EdgeArgs {
    /// Change only this edge (left, right, top or bottom); without it,
    /// every edge.
    #[arg(long)]
    pub side: Option<Side>,
    /// Ignore crossings within this many pixels of a screen corner (0: off).
    #[arg(long, value_name = "PX")]
    pub corner: Option<u32>,
    /// The pointer must stay at the edge this long before it crosses (0: off).
    #[arg(long, value_name = "MS")]
    pub dwell: Option<u32>,
    /// Only cross while holding this key: none, shift, ctrl or alt.
    #[arg(long, value_name = "KEY")]
    pub modifier: Option<Modifier>,
    /// Don't cross while a fullscreen window has the focus here: on or off.
    #[arg(long, value_name = "on|off", value_parser = parse_on_off)]
    pub fullscreen_guard: Option<bool>,
    /// A program that may always be left, even fullscreen (repeatable).
    #[arg(long, value_name = "APP")]
    pub fullscreen_allow: Vec<String>,
    /// A program that is never left while it has the focus (repeatable).
    #[arg(long, value_name = "APP")]
    pub fullscreen_block: Vec<String>,
    /// Remove a program from both lists (repeatable).
    #[arg(long, value_name = "APP")]
    pub fullscreen_forget: Vec<String>,
    /// Turn every rule off (for one edge with --side).
    #[arg(long)]
    pub reset: bool,
}

pub fn parse_on_off(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        _ => Err(format!("expected on or off, not {s:?}")),
    }
}

impl EdgeArgs {
    fn changes_anything(&self) -> bool {
        self.reset
            || self.corner.is_some()
            || self.dwell.is_some()
            || self.modifier.is_some()
            || self.fullscreen_guard.is_some()
            || !self.fullscreen_allow.is_empty()
            || !self.fullscreen_block.is_empty()
            || !self.fullscreen_forget.is_empty()
    }

    /// Applies these options to `s`.
    pub fn apply(&self, s: &mut EdgeSettings) -> Result<()> {
        if let Some(n) = self.corner.filter(|&n| n > MAX_CORNER) {
            bail!("--corner {n} is too large (at most {MAX_CORNER} px)");
        }
        if let Some(ms) = self.dwell.filter(|&ms| ms > MAX_DWELL_MS) {
            bail!("--dwell {ms} is too long (at most {MAX_DWELL_MS} ms)");
        }
        let lists = !self.fullscreen_allow.is_empty() || !self.fullscreen_block.is_empty();
        if self.side.is_some() && lists {
            bail!("the fullscreen program lists are for every edge; leave out --side");
        }
        let rules = match self.side {
            Some(side) => s.side_mut(side).get_or_insert_with(Rules::default),
            None => &mut s.all,
        };
        if self.reset {
            *rules = Rules::default();
        }
        if let Some(n) = self.corner {
            rules.corner = Some(n);
        }
        if let Some(ms) = self.dwell {
            rules.dwell_ms = Some(ms);
        }
        if let Some(m) = self.modifier {
            rules.modifier = Some(m.to_string());
        }
        if let Some(on) = self.fullscreen_guard {
            rules.fullscreen_guard = Some(on);
        }
        if let Some(side) = self.side
            && s.side(side).is_some_and(Rules::is_empty)
        {
            *s.side_mut(side) = None;
        }
        let add = |list: &mut Vec<String>, apps: &[String]| {
            for a in apps.iter().map(|a| a.trim()).filter(|a| !a.is_empty()) {
                if !list.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                    list.push(a.to_owned());
                }
            }
        };
        add(&mut s.fullscreen_allow, &self.fullscreen_allow);
        add(&mut s.fullscreen_block, &self.fullscreen_block);
        for forget in &self.fullscreen_forget {
            s.fullscreen_allow.retain(|a| !a.eq_ignore_ascii_case(forget));
            s.fullscreen_block.retain(|a| !a.eq_ignore_ascii_case(forget));
        }
        if self.reset && self.side.is_none() {
            *s = EdgeSettings::default();
        }
        Ok(())
    }
}

/// Saves `settings` and asks a running daemon to apply them.
async fn save_and_apply(dir: &Path, settings: &Settings) -> Result<()> {
    settings.save(dir)?;
    if control::request(dir, Request::Reload).await?.is_some() {
        println!("Applied right away.");
    } else {
        println!("one-mk is not running; this takes effect the next time it starts.");
    }
    Ok(())
}

/// `one-mk edge [...]`: shows the edge rules, or changes them.
pub async fn edge(dir: &Path, args: EdgeArgs) -> Result<()> {
    let mut settings = Settings::load(dir);
    if !args.changes_anything() {
        let d = settings.edge.describe();
        if d.is_empty() {
            println!("No edge rules: the pointer crosses as soon as it reaches an edge.");
        } else {
            println!("Edges: {d}.");
        }
        if settings.edge.guard(Side::Left).fullscreen && cfg!(target_os = "linux") {
            println!("{}", FULLSCREEN_LINUX);
        }
        return Ok(());
    }
    args.apply(&mut settings.edge)?;
    let d = settings.edge.describe();
    if d.is_empty() {
        println!("No edge rules: the pointer crosses as soon as it reaches an edge.");
    } else {
        println!("Edges: {d}.");
    }
    if args.fullscreen_guard == Some(true) && cfg!(target_os = "linux") {
        println!("{}", FULLSCREEN_LINUX);
    }
    save_and_apply(dir, &settings).await
}

const FULLSCREEN_LINUX: &str = "Note: on Wayland only programs running through XWayland (most games) can be seen as fullscreen; \
                                 for others the guard lets the pointer cross.";

/// `one-mk gaming on|off|auto`.
pub async fn gaming(dir: &Path, mode: Gaming) -> Result<()> {
    let mut settings = Settings::load(dir);
    settings.gaming = mode;
    match mode {
        Gaming::Off => println!("Gaming mode is off: the pointer moves by position, and edges lead back."),
        Gaming::On => println!(
            "Gaming mode is on: on another computer the mouse sends raw movement and doesn't stop at its screen edges. \
             Come back with `one-mk switch here` (or Ctrl+Alt+Shift+Esc)."
        ),
        Gaming::Auto => println!(
            "Gaming mode is automatic: raw movement while the other computer shows a fullscreen window \
             (where it can tell). Come back with `one-mk switch here` (or Ctrl+Alt+Shift+Esc)."
        ),
    }
    save_and_apply(dir, &settings).await
}

/// `one-mk lock-sync on|off`.
pub async fn lock_sync(dir: &Path, on: bool) -> Result<()> {
    let mut settings = Settings::load(dir);
    settings.lock_sync = on;
    if on {
        println!("Lock sync is on: locking this computer locks the connected ones (if they run one-mk too).");
    } else {
        println!("Lock sync is off. (`one-mk lock-all` still locks every computer.)");
    }
    save_and_apply(dir, &settings).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn per_edge_rules_fall_back_to_every_edge() {
        let mut s = EdgeSettings::default();
        EdgeArgs { corner: Some(20), modifier: Some(Modifier::Shift), ..EdgeArgs::default() }.apply(&mut s).unwrap();
        EdgeArgs { side: Some(Side::Left), dwell: Some(300), modifier: Some(Modifier::None), ..EdgeArgs::default() }
            .apply(&mut s)
            .unwrap();
        assert_eq!(s.guard(Side::Right), EdgeGuard { corner: 20, dwell_ms: 0, modifier: Modifier::Shift, fullscreen: false });
        assert_eq!(s.guard(Side::Left), EdgeGuard { corner: 20, dwell_ms: 300, modifier: Modifier::None, fullscreen: false });
        assert_eq!(s.describe(), "dead corners 20 px, hold shift; left edge: dwell 300 ms, no key to hold");

        // Resetting one edge drops its overrides only.
        EdgeArgs { side: Some(Side::Left), reset: true, ..EdgeArgs::default() }.apply(&mut s).unwrap();
        assert_eq!(s.left, None);
        assert_eq!(s.guard(Side::Left).modifier, Modifier::Shift);
        // Resetting everything.
        EdgeArgs { reset: true, ..EdgeArgs::default() }.apply(&mut s).unwrap();
        assert_eq!(s, EdgeSettings::default());
        assert_eq!(s.describe(), "");
    }

    #[test]
    fn edge_options_are_checked() {
        let mut s = EdgeSettings::default();
        assert!(EdgeArgs { corner: Some(MAX_CORNER + 1), ..EdgeArgs::default() }.apply(&mut s).is_err());
        assert!(EdgeArgs { dwell: Some(60_000), ..EdgeArgs::default() }.apply(&mut s).is_err());
        let lists = EdgeArgs { side: Some(Side::Top), fullscreen_allow: vec!["vlc".into()], ..EdgeArgs::default() };
        assert!(lists.apply(&mut s).is_err());
        assert_eq!(s, EdgeSettings::default(), "nothing changed by refused options");
        assert!(!EdgeArgs::default().changes_anything());
        assert_eq!(parse_on_off("ON"), Ok(true));
        assert!(parse_on_off("maybe").is_err());
    }

    #[test]
    fn fullscreen_lists() {
        let mut s = EdgeSettings::default();
        let args = EdgeArgs {
            fullscreen_guard: Some(true),
            fullscreen_allow: vec!["vlc".into(), "VLC".into(), " ".into()],
            fullscreen_block: vec!["doom".into()],
            ..EdgeArgs::default()
        };
        args.apply(&mut s).unwrap();
        assert_eq!(s.fullscreen_allow, vec!["vlc"]);
        assert_eq!(s.describe(), "fullscreen guard; fullscreen allowed for vlc; never leaving doom");
        EdgeArgs { fullscreen_forget: vec!["DOOM".into()], ..EdgeArgs::default() }.apply(&mut s).unwrap();
        assert!(s.fullscreen_block.is_empty());
        assert!(s.guard(Side::Bottom).fullscreen);
    }

    /// clap checks the command definitions only when asked (or when the
    /// broken subcommand runs): a positional `bool` once panicked here.
    #[test]
    fn command_line_is_well_formed() {
        use clap::{CommandFactory, Parser};
        crate::Cli::command().debug_assert();
        for args in [
            &["one-mk", "lock-sync", "on"][..],
            &["one-mk", "lock-pointer", "toggle"],
            &["one-mk", "switch", "next"],
            &["one-mk", "gaming", "auto"],
            &["one-mk", "edge", "--side", "left", "--dwell", "300", "--modifier", "ctrl", "--fullscreen-guard", "off"],
            &["one-mk", "run", "--start-locked"],
        ] {
            assert!(crate::Cli::try_parse_from(args).is_ok(), "{args:?}");
        }
        assert!(crate::Cli::try_parse_from(["one-mk", "lock-sync", "maybe"]).is_err());
    }

    #[test]
    fn saved_and_loaded() {
        let dir = temp_dir("roundtrip");
        assert_eq!(Settings::load(&dir), Settings::default(), "no file");
        let mut s = Settings { gaming: Gaming::Auto, lock_sync: true, ..Settings::default() };
        EdgeArgs { corner: Some(8), side: Some(Side::Top), ..EdgeArgs::default() }.apply(&mut s.edge).unwrap();
        EdgeArgs { dwell: Some(250), fullscreen_guard: Some(true), ..EdgeArgs::default() }.apply(&mut s.edge).unwrap();
        s.save(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("gaming = \"auto\"") && text.contains("[edge.top]"), "{text}");
        assert_eq!(Settings::load(&dir), s);
        // A broken file is ignored, not fatal.
        std::fs::write(dir.join(FILE), "gaming = 7\n[[[").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
        // A hand-written bad modifier means none.
        std::fs::write(dir.join(FILE), "[edge]\nmodifier = \"hyper\"\ncorner = 99999\n").unwrap();
        let s = Settings::load(&dir);
        assert_eq!(s.edge.guard(Side::Left), EdgeGuard { corner: MAX_CORNER, ..EdgeGuard::default() });
        let _ = std::fs::remove_dir_all(dir);
    }
}
