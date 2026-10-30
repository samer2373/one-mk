//! Settings that are not about one paired computer: what the clipboard
//! shares, whether received files raise a notification, the edge rules,
//! gaming mode, lock sync, keyboard shortcuts and the drawn pointer. They
//! live in `settings.toml` next to `devices.toml`; the command line (`one-mk
//! clipboard`, `notify`, `edge`, `gaming`, `lock-sync`, `hotkeys`,
//! `pointer-overlay`) edits the
//! file and a running daemon applies it at once (like `one-mk layout`).

use crate::control::{self, Request};
use anyhow::{Context, Result, bail};
use one_mk_clipboard::{CEILING_BYTES, DEFAULT_MAX_BYTES, Filter};
use one_mk_proto::{EdgeGuard, Modifier, Side};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const FILE: &str = "settings.toml";
/// Held while settings are changed (see [`Settings::update`]).
const LOCK_FILE: &str = "settings.toml.lock";
/// Bounds for the edge rules; larger values are almost surely typos.
pub const MAX_CORNER: u32 = 500;
pub const MAX_DWELL_MS: u32 = 5000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub drag: crate::drag::Settings,
    #[serde(default)]
    pub settings_version: u32,
    pub clipboard: ClipSettings,
    /// Show a desktop notification when files arrive.
    pub notify_files: bool,
    pub edge: EdgeSettings,
    pub gaming: Gaming,
    /// Locking any computer locks the others.
    pub lock_sync: bool,
    /// Commands to run when the pointer arrives or leaves (see `hooks.rs`).
    pub hooks: crate::hooks::Hooks,
    /// Screensaver and sleep sync (see `power.rs`).
    pub power: crate::power::PowerSettings,
    pub hotkeys: crate::hotkeys::HotkeySettings,
    /// Key mapper, pointer, scroll and button tuning (see `tuning.rs`).
    #[serde(skip_serializing_if = "crate::tuning::TuningSettings::is_empty")]
    pub input: crate::tuning::TuningSettings,
    /// Drawing the pointer on a computer with no mouse (see
    /// `pointeroverlay.rs`).
    pub pointer: crate::pointeroverlay::PointerSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            drag: Default::default(),
            settings_version: 2,
            clipboard: ClipSettings::default(),
            notify_files: true,
            edge: EdgeSettings::default(),
            gaming: Gaming::default(),
            lock_sync: false,
            hooks: crate::hooks::Hooks::default(),
            power: crate::power::PowerSettings::default(),
            hotkeys: Default::default(),
            input: Default::default(),
            pointer: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipSettings {
    /// Share plain text.
    pub text: bool,
    /// Share HTML (rich text).
    pub html: bool,
    /// Share images.
    pub image: bool,
    /// Largest clipboard to send or accept, in MB (1 MB = 2^20 bytes).
    pub max_mb: u32,
    /// Don't share what a password manager marked as sensitive.
    pub skip_sensitive: bool,
    /// Let a paired computer fetch this clipboard (`one-mk clip-get` there).
    #[serde(skip_serializing_if = "is_false")]
    pub allow_pull: bool,
    #[serde(skip)]
    pub fetch_migrated: bool,
    #[serde(skip)]
    pub fetch_notice: Option<PathBuf>,
}

fn is_false(value: &bool) -> bool { !*value }

impl Default for ClipSettings {
    fn default() -> Self {
        Self {
            text: true,
            html: true,
            image: true,
            max_mb: (DEFAULT_MAX_BYTES >> 20) as u32,
            skip_sensitive: true,
            allow_pull: false,
            fetch_migrated: false,
            fetch_notice: None,
        }
    }
}

impl ClipSettings {
    /// The most `max_mb` can be set to.
    pub const MAX_MB_LIMIT: u32 = (CEILING_BYTES >> 20) as u32;

    pub fn filter(&self) -> Filter {
        Filter {
            text: self.text,
            html: self.html,
            image: self.image,
            max_bytes: (self.max_mb.clamp(1, Self::MAX_MB_LIMIT) as usize) << 20,
            skip_concealed: self.skip_sensitive,
        }
    }

    pub fn summary(&self) -> String {
        let on = |b: bool| if b { "on" } else { "off" };
        let summary = format!(
            "text {}, html {}, images {}, size limit {} MB, skip password managers {}, others may fetch it {}",
            on(self.text),
            on(self.html),
            on(self.image),
            self.max_mb.clamp(1, Self::MAX_MB_LIMIT),
            on(self.skip_sensitive),
            on(self.allow_pull)
        );
        let show_notice = self.fetch_migrated && !self.allow_pull && self.fetch_notice.as_ref().is_none_or(|path| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(_) => true,
                Err(error) => error.kind() != std::io::ErrorKind::AlreadyExists,
            }
        });
        if show_notice {
            format!("{summary}; clipboard fetching was switched off by this update; turn it on with one-mk clipboard --allow-fetch on")
        } else { summary }
    }
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
    /// Where no computer sits, the pointer comes back in at the opposite
    /// edge (`docs/EDGE-PUSH.md`).
    #[serde(skip_serializing_if = "is_default")]
    pub wrap: Wrap,
    /// Pushing the pointer twice against an edge where no computer sits
    /// suggests (or sets) where a computer is.
    #[serde(skip_serializing_if = "is_default")]
    pub learn: Learn,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Pointer wrap (`[edge] wrap`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Wrap {
    /// An edge where no computer sits stops the pointer (the default).
    #[default]
    Off,
    /// The left and right edges where no computer sits lead round to the opposite edge.
    On,
    /// Every edge where no computer sits does, the top and bottom too.
    All,
}

impl Wrap {
    /// The edges that wrap.
    pub fn sides(self) -> &'static [Side] {
        match self {
            Wrap::Off => &[],
            Wrap::On => &[Side::Left, Side::Right],
            Wrap::All => &Side::ALL,
        }
    }
}

/// `off`, `on` or `all`; a hand-written `true` or `false` means on or off.
impl<'de> Deserialize<'de> for Wrap {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Word(String),
        }
        match Raw::deserialize(d)? {
            Raw::Bool(on) => Ok(if on { Wrap::On } else { Wrap::Off }),
            Raw::Word(w) => match w.to_ascii_lowercase().as_str() {
                "off" => Ok(Wrap::Off),
                "on" => Ok(Wrap::On),
                "all" => Ok(Wrap::All),
                _ => Err(serde::de::Error::custom(format!("wrap must be off, on or all, not {w:?}"))),
            },
        }
    }
}

/// Layout learning (`[edge] learn`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Learn {
    /// Pushing toward a computer does nothing.
    Off,
    /// Say where a computer seems to be, and how to place it there (the default).
    #[default]
    Suggest,
    /// Place it there (beside you on the left or right; above or below is
    /// only suggested), and say how to undo that.
    Auto,
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

    /// What edges with no computer do, in a sentence (for `one-mk edge`).
    pub fn no_computer(&self) -> String {
        let wrap = match self.wrap {
            Wrap::Off => "the pointer stops there",
            Wrap::On => "the pointer wraps round to the opposite edge on the left and right",
            Wrap::All => "the pointer wraps round to the opposite edge",
        };
        let learn = match self.learn {
            Learn::Off => "pushing toward a computer does nothing",
            Learn::Suggest => "pushing twice toward a computer that isn't placed yet suggests where it is",
            Learn::Auto => "pushing twice toward a computer that isn't placed yet places it there",
        };
        format!("Where no computer sits, {wrap}, and {learn} (`--wrap`, `--learn`).")
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
        match self.wrap {
            Wrap::Off => {}
            Wrap::On => parts.push("the pointer wraps round at left and right edges with no computer".into()),
            Wrap::All => parts.push("the pointer wraps round at every edge with no computer".into()),
        }
        match self.learn {
            Learn::Suggest => {}
            Learn::Off => parts.push("no layout learning".into()),
            Learn::Auto => parts.push("layout learning places computers".into()),
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
    /// Reads `settings.toml`; a missing file means the defaults. A file that
    /// can't be read is an error, so the command line never overwrites a
    /// hand edit it didn't understand.
    pub fn load(dir: &Path) -> Result<Settings> {
        let path = path(dir);
        match crate::hooks::read_settings(&path) {
            Ok((s, unsafe_reason)) => {
                let mut settings: Settings = toml::from_str(&s).with_context(|| format!("reading {}", path.display()))?;
                if settings.settings_version < 2 && settings.clipboard.allow_pull {
                    settings.clipboard.allow_pull = false;
                    settings.clipboard.fetch_migrated = true;
                    settings.clipboard.fetch_notice = Some(dir.join(".clipboard-fetch-update-noticed"));
                }
                settings.settings_version = 2;
                settings.hooks.trust_permissions = crate::hooks::permissions_trusted(dir);
                // Hooks and key actions (`run: ...`) both start programs.
                if (settings.hooks.any() || settings.input.runs_commands()) && !settings.hooks.trust_permissions {
                    settings.hooks.blocked = unsafe_reason;
                }
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// The saved settings, or the defaults if there are none. A file that
    /// can't be read is reported and ignored, so a bad hand edit never
    /// stops one-mk.
    pub fn load_or_default(dir: &Path) -> Settings {
        Self::load(dir).unwrap_or_else(|e| {
            tracing::warn!("ignoring {}: {e:#}", path(dir).display());
            Settings::default()
        })
    }

    /// Writes the settings to `settings.toml` through a file of its own that
    /// is then renamed over it, so a reader never sees half a file and two
    /// writers never share one. To change settings use [`Settings::update`]:
    /// this alone would lose a change made by someone else in between.
    pub fn save(&self, dir: &Path) -> Result<()> {
        static N: AtomicU32 = AtomicU32::new(0);
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let write = || -> Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
            drop(file);
            std::fs::rename(&tmp, path(dir))?;
            Ok(())
        };
        write().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Changes the saved settings: waits for its turn (an exclusive lock on a
    /// file next to `settings.toml`, so `one-mk edge` and `one-mk gaming`
    /// run at the same moment both land), loads what is there (a file that
    /// can't be read is an error, as with [`Settings::load`]), applies
    /// `change` and saves. If `change` fails nothing is written.
    pub fn update<R>(dir: &Path, change: impl FnOnce(&mut Settings) -> Result<R>) -> Result<R> {
        std::fs::create_dir_all(dir)?;
        let lock_path = dir.join(LOCK_FILE);
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        lock.lock().with_context(|| format!("locking {}", lock_path.display()))?;
        // Unlocked when `lock` is dropped (also if this fails or panics).
        let mut settings = Settings::load(dir)?;
        let out = change(&mut settings)?;
        settings.save(dir)?;
        Ok(out)
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
    /// Turn every rule off (for one edge with --side); for every edge,
    /// also --wrap and --learn go back to off and suggest.
    #[arg(long)]
    pub reset: bool,
    /// Where no computer sits, bring the pointer in at the opposite edge:
    /// off, on (left and right edges) or all (every edge). For every edge.
    #[arg(long, value_enum, value_name = "off|on|all")]
    pub wrap: Option<Wrap>,
    /// Pushing the pointer twice toward a computer that isn't placed yet:
    /// off, suggest (say where it seems to be) or auto (place it). For
    /// every edge.
    #[arg(long, value_enum, value_name = "off|suggest|auto")]
    pub learn: Option<Learn>,
    /// Hide the layout suggestion shown now (in `one-mk status` and the window).
    #[arg(long)]
    pub dismiss: bool,
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
            || self.wrap.is_some()
            || self.learn.is_some()
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
        if self.side.is_some() && (self.wrap.is_some() || self.learn.is_some()) {
            bail!("--wrap and --learn are for every edge; leave out --side");
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
        if let Some(wrap) = self.wrap {
            s.wrap = wrap;
        }
        if let Some(learn) = self.learn {
            s.learn = learn;
        }
        Ok(())
    }
}

/// Asks a running daemon to apply the saved settings.
pub(crate) async fn apply(dir: &Path) -> Result<()> {
    if control::request(dir, Request::Reload).await?.is_some() {
        println!("Applied right away.");
    } else {
        println!("one-mk is not running; this takes effect the next time it starts.");
    }
    Ok(())
}

/// `one-mk edge [...]`: shows the edge rules, or changes them.
pub async fn edge(dir: &Path, args: EdgeArgs) -> Result<()> {
    let describe = |e: &EdgeSettings| {
        let d = e.describe();
        if d.is_empty() {
            println!("No edge rules: the pointer crosses as soon as it reaches an edge.");
        } else {
            println!("Edges: {d}.");
        }
        println!("{}", e.no_computer());
    };
    if args.dismiss && !args.changes_anything() {
        return dismiss(dir).await;
    }
    if !args.changes_anything() {
        let settings = Settings::load(dir)?;
        describe(&settings.edge);
        if settings.edge.guard(Side::Left).fullscreen && cfg!(target_os = "linux") {
            println!("{}", FULLSCREEN_LINUX);
        }
        return Ok(());
    }
    let edge = Settings::update(dir, |s| {
        args.apply(&mut s.edge)?;
        Ok(s.edge.clone())
    })?;
    describe(&edge);
    if args.fullscreen_guard == Some(true) && cfg!(target_os = "linux") {
        println!("{}", FULLSCREEN_LINUX);
    }
    if args.wrap == Some(Wrap::All) && edge.all.dwell_ms.unwrap_or(0) == 0 {
        println!("Tip: with the top and bottom edges wrapping, a short dwell (`one-mk edge --dwell 300`) keeps menus and the taskbar easy to reach.");
    }
    apply(dir).await?;
    if args.dismiss {
        dismiss(dir).await?;
    }
    Ok(())
}

/// `one-mk edge --dismiss`: hides the layout suggestion.
async fn dismiss(dir: &Path) -> Result<()> {
    match control::request(dir, Request::DismissSuggestion).await?.as_deref() {
        None => println!("one-mk is not running, so there is no layout suggestion to hide."),
        Some("ok\n") => println!("The layout suggestion is hidden. one-mk won't suggest that computer again for 10 minutes."),
        Some("none\n") => println!("There is no layout suggestion to hide."),
        Some(other) => bail!("unexpected answer from one-mk: {other:?}"),
    }
    Ok(())
}

const FULLSCREEN_LINUX: &str = "Note: on Wayland only programs running through XWayland (most games) can be seen as fullscreen; \
                                 for others the guard lets the pointer cross.";

/// `one-mk gaming on|off|auto`.
pub async fn gaming(dir: &Path, mode: Gaming) -> Result<()> {
    Settings::update(dir, |s| {
        s.gaming = mode;
        Ok(())
    })?;
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
    apply(dir).await
}

/// `one-mk lock-sync on|off`.
pub async fn lock_sync(dir: &Path, on: bool) -> Result<()> {
    Settings::update(dir, |s| {
        s.lock_sync = on;
        Ok(())
    })?;
    if on {
        println!("Lock sync is on: locking this computer locks the connected ones (if they run one-mk too).");
    } else {
        println!("Lock sync is off. (`one-mk lock-all` still locks every computer.)");
    }
    apply(dir).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_clipboard_fetch_is_migrated_but_explicit_v2_is_kept() {
        let dir = temp_dir("clipboard-version");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "[clipboard]\nallow_pull = true\n").unwrap();
        let settings = Settings::load(&dir).unwrap();
        assert!(!settings.clipboard.allow_pull);
        assert!(settings.clipboard.summary().contains("clipboard fetching was switched off by this update; turn it on with one-mk clipboard --allow-fetch on"));
        assert!(!Settings::load(&dir).unwrap().clipboard.summary().contains("switched off"));
        std::fs::write(dir.join(FILE), "settings_version = 2\n[clipboard]\nallow_pull = true\n").unwrap();
        assert!(Settings::load(&dir).unwrap().clipboard.allow_pull);
        assert!(!toml::to_string(&ClipSettings::default()).unwrap().contains("allow_pull"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_directory_must_be_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = temp_dir("symlink-settings");
        let target = dir.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o777)).unwrap();
        let file = target.join(FILE);
        std::fs::write(&file, "[hooks]\non_enter = ['echo']\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&file, dir.join(FILE)).unwrap();
        assert!(Settings::load(&dir).unwrap().hooks.blocked.is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn external_symlink_target_ancestors_must_be_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = temp_dir("external-symlink-settings");
        let home = dir.join("home");
        let shared = dir.join("shared");
        let target = shared.join("private");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        for folder in [&dir, &home, &target] { std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).unwrap(); }
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        let file = target.join(FILE);
        std::fs::write(&file, "[hooks]\non_enter = ['echo']\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&file, home.join(FILE)).unwrap();
        assert!(Settings::load(&home).unwrap().hooks.blocked.is_some());
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(Settings::load(&home).unwrap().hooks.blocked.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn settings_replacement_keeps_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private-save");
        Settings::default().save(&dir).unwrap();
        std::fs::set_permissions(path(&dir), std::fs::Permissions::from_mode(0o600)).unwrap();
        Settings::default().save(&dir).unwrap();
        assert_eq!(std::fs::metadata(path(&dir)).unwrap().permissions().mode() & 0o777, 0o600);
    }
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

    /// `[edge] wrap` and `learn`: the defaults are not written, a file
    /// from before them still loads, and hand edits are read kindly where
    /// they can be and refused clearly where they can't.
    #[test]
    fn wrap_and_learn_settings() {
        let dir = temp_dir("wrap-learn");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "[edge]\ncorner = 20\n[edge.left]\ndwell_ms = 300\n").unwrap();
        let old = Settings::load(&dir).unwrap();
        assert_eq!((old.edge.wrap, old.edge.learn, old.edge.guard(Side::Left).dwell_ms), (Wrap::Off, Learn::Suggest, 300));
        assert!(!toml::to_string(&Settings::default()).unwrap().contains("wrap"), "defaults stay out of the file");
        assert!(!toml::to_string(&Settings::default()).unwrap().contains("learn"));
        for (text, wrap) in [("true", Wrap::On), ("false", Wrap::Off), ("'ON'", Wrap::On), ("'all'", Wrap::All), ("'off'", Wrap::Off)] {
            std::fs::write(dir.join(FILE), format!("[edge]\nwrap = {text}\nlearn = 'auto'\n")).unwrap();
            let s = Settings::load(&dir).unwrap();
            assert_eq!((s.edge.wrap, s.edge.learn), (wrap, Learn::Auto), "{text}");
            // Round trip through the file.
            s.save(&dir).unwrap();
            assert_eq!(Settings::load(&dir).unwrap().edge, s.edge);
        }
        for bad in ["wrap = 'sideways'", "wrap = 3", "learn = 'maybe'", "learn = true"] {
            std::fs::write(dir.join(FILE), format!("[edge]\n{bad}\n")).unwrap();
            let e = format!("{:#}", Settings::load(&dir).unwrap_err());
            assert!(e.contains("settings.toml"), "{bad}: {e}");
        }
        std::fs::remove_dir_all(dir).unwrap();

        let mut s = EdgeSettings::default();
        EdgeArgs { wrap: Some(Wrap::On), learn: Some(Learn::Off), ..EdgeArgs::default() }.apply(&mut s).unwrap();
        assert_eq!((s.wrap, s.learn), (Wrap::On, Learn::Off));
        assert_eq!(s.describe(), "the pointer wraps round at left and right edges with no computer; no layout learning");
        assert!(EdgeArgs { side: Some(Side::Left), wrap: Some(Wrap::All), ..EdgeArgs::default() }.apply(&mut s).is_err());
        assert_eq!(s.wrap, Wrap::On, "nothing changed by a refused option");
        EdgeArgs { reset: true, ..EdgeArgs::default() }.apply(&mut s).unwrap();
        assert_eq!((s.wrap, s.learn), (Wrap::Off, Learn::Suggest));
        assert!(EdgeArgs { learn: Some(Learn::Auto), ..EdgeArgs::default() }.changes_anything());
        assert!(!EdgeArgs { dismiss: true, ..EdgeArgs::default() }.changes_anything());
        assert_eq!(Wrap::On.sides(), [Side::Left, Side::Right]);
        assert!(Wrap::Off.sides().is_empty());
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
        assert_eq!(Settings::load(&dir).unwrap(), Settings::default(), "no file");
        let mut s = Settings { gaming: Gaming::Auto, lock_sync: true, ..Settings::default() };
        EdgeArgs { corner: Some(8), side: Some(Side::Top), ..EdgeArgs::default() }.apply(&mut s.edge).unwrap();
        EdgeArgs { dwell: Some(250), fullscreen_guard: Some(true), ..EdgeArgs::default() }.apply(&mut s.edge).unwrap();
        s.save(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("gaming = \"auto\"") && text.contains("[edge.top]"), "{text}");
        assert_eq!(Settings::load(&dir).unwrap(), s);
        // A broken file is ignored, not fatal.
        std::fs::write(dir.join(FILE), "gaming = 7\n[[[").unwrap();
        assert_eq!(Settings::load_or_default(&dir), Settings::default());
        // A hand-written bad modifier means none.
        std::fs::write(dir.join(FILE), "[edge]\nmodifier = \"hyper\"\ncorner = 99999\n").unwrap();
        let s = Settings::load_or_default(&dir);
        assert_eq!(s.edge.guard(Side::Left), EdgeGuard { corner: MAX_CORNER, ..EdgeGuard::default() });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clipboard_defaults_share_everything_up_to_25_mb() {
        let s = Settings::default();
        assert!(!s.clipboard.allow_pull, "fetching must be opt-in");
        assert!(s.clipboard.text && s.clipboard.html && s.clipboard.image && s.notify_files);
        assert_eq!(s.clipboard.filter(), Filter::default());
        assert_eq!(s.clipboard.max_mb, 25);
    }

    #[test]
    fn explicit_clipboard_fetch_opt_in_survives_the_new_default() {
        let settings: Settings = toml::from_str("[clipboard]\nallow_pull = true\n").unwrap();
        assert!(settings.clipboard.allow_pull);
    }

    #[test]
    fn clipboard_settings_roundtrip_next_to_the_edge_rules() {
        let dir = temp_dir("clip-round");
        let mut s = Settings {
            clipboard: ClipSettings { image: false, max_mb: 3, allow_pull: false, ..Default::default() },
            notify_files: false,
            gaming: Gaming::On,
            ..Settings::default()
        };
        EdgeArgs { corner: Some(8), ..EdgeArgs::default() }.apply(&mut s.edge).unwrap();
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir).unwrap(), s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_and_future_files_still_load() {
        let dir = temp_dir("partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "[clipboard]\nmax_mb = 5\nsome_future_key = 1\n").unwrap();
        let s = Settings::load(&dir).unwrap();
        assert_eq!(s.clipboard.max_mb, 5);
        assert!(s.clipboard.text && s.notify_files, "what the file doesn't say keeps its default");
        std::fs::write(dir.join(FILE), "clipboard = [").unwrap();
        assert!(Settings::load(&dir).is_err(), "a broken file is reported, not silently reset");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A key action that runs a command is held to the same permission
    /// rules as `[hooks]`: a settings file others can write must not be able
    /// to start programs, even when no hook is set.
    #[cfg(unix)]
    #[test]
    fn run_key_actions_are_refused_when_the_file_is_writable_by_others() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("run-actions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = dir.join(FILE);
        let write = |text: &str, mode: u32| {
            std::fs::write(&file, text).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        let run_rule = "[[input.keymap.rule]]\nfrom = \"f13\"\nto = \"run: touch /tmp/x\"\n";
        let plain_rule = "[[input.keymap.rule]]\nfrom = \"f13\"\nto = \"esc\"\n";
        let button_run = "[[input.button]]\nfrom = 8\nto = [\"run: touch /tmp/x\"]\n";

        write(run_rule, 0o660);
        assert!(Settings::load(&dir).unwrap().hooks.blocked.is_some(), "run: in a group-writable file");
        write(button_run, 0o660);
        assert!(Settings::load(&dir).unwrap().hooks.blocked.is_some(), "run: on a button");
        write(run_rule, 0o600);
        assert!(Settings::load(&dir).unwrap().hooks.blocked.is_none(), "a private file is fine");
        write(plain_rule, 0o660);
        assert!(Settings::load(&dir).unwrap().hooks.blocked.is_none(), "no command, nothing to refuse");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_clipboard_size_limit_is_kept_sane() {
        let f = |mb| ClipSettings { max_mb: mb, ..Default::default() }.filter().max_bytes();
        assert_eq!(f(0), 1 << 20, "never zero");
        assert_eq!(f(25), 25 << 20);
        assert_eq!(f(u32::MAX), CEILING_BYTES);
    }

    /// Commands run at the same moment (a script, or a window and a
    /// terminal) must all land, whichever field they change.
    #[test]
    fn concurrent_updates_all_survive() {
        let dir = temp_dir("concurrent");
        const THREADS: usize = 8;
        const EACH: usize = 6;
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for i in 0..EACH {
                        Settings::update(&dir, |s| {
                            s.edge.fullscreen_allow.push(format!("app-{t}-{i}"));
                            match t {
                                0 => s.clipboard.max_mb = 3,
                                1 => s.clipboard.image = false,
                                2 => s.gaming = Gaming::Auto,
                                3 => s.lock_sync = true,
                                4 => s.notify_files = false,
                                5 => s.edge.all.corner = Some(7),
                                6 => s.edge.all.dwell_ms = Some(250),
                                _ => s.clipboard.allow_pull = false,
                            }
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        let s = Settings::load(&dir).unwrap();
        assert_eq!(s.edge.fullscreen_allow.len(), THREADS * EACH, "an update was lost: {:?}", s.edge.fullscreen_allow);
        assert_eq!(s.clipboard.max_mb, 3);
        assert!(!s.clipboard.image && !s.notify_files && !s.clipboard.allow_pull);
        assert!(s.lock_sync);
        assert_eq!((s.gaming, s.edge.all.corner, s.edge.all.dwell_ms), (Gaming::Auto, Some(7), Some(250)));
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(leftovers.iter().all(|n| n == FILE || n == LOCK_FILE), "stray files: {leftovers:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_update_or_a_bad_file_changes_nothing() {
        let dir = temp_dir("failed-update");
        Settings::update(&dir, |s| {
            s.lock_sync = true;
            Ok(())
        })
        .unwrap();
        let before = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let refused = Settings::update(&dir, |s| -> Result<()> {
            s.lock_sync = false;
            bail!("no")
        });
        assert!(refused.is_err());
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), before);
        // A hand edit that doesn't parse is not overwritten by a command line change.
        std::fs::write(dir.join(FILE), "clipboard = [").unwrap();
        let bad = Settings::update(&dir, |s| {
            s.lock_sync = true;
            Ok(())
        });
        assert!(bad.is_err());
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), "clipboard = [");
        let _ = std::fs::remove_dir_all(dir);
    }
}
