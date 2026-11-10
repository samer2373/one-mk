//! What the newer panels show, worked out without drawing anything: the
//! settings as `settings.toml` has them, `one-mk doctor --json` results,
//! where a computer sits on the layout plan and how a drag along its edge
//! snaps, and transfers in words. Everything here is plain data in, plain
//! data out, so it is tested without a window.

use one_mk_ctl::status::{TransferStatus, human_bytes, human_duration};
use std::collections::HashMap;

// ---------- a small reader for the TOML one-mk writes ----------

/// A value in `settings.toml` or `profiles.toml`, as far as the window cares.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Val>),
    /// `{ os = "mac" }`, keys in the order written.
    Table(Vec<(String, Val)>),
    /// Anything else (dates): kept as written.
    Other(String),
}

impl Val {
    pub fn as_bool(&self) -> Option<bool> {
        if let Val::Bool(b) = self { Some(*b) } else { None }
    }
    pub fn as_int(&self) -> Option<i64> {
        if let Val::Int(n) = self { Some(*n) } else { None }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Val::Str(s) = self { Some(s) } else { None }
    }
    /// A number, whole or not.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Val::Float(x) => Some(*x),
            Val::Int(n) => Some(*n as f64),
            _ => None,
        }
    }
    /// Strings in a list (a one-word string counts as a list of one).
    pub fn words(&self) -> Vec<String> {
        match self {
            Val::Str(s) => vec![s.clone()],
            Val::List(l) => l.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => vec![],
        }
    }
}

/// One `key = value` line: under which table (`edge.left`; empty at the
/// top), which repeat of an `[[array]]` table (0 otherwise), and the key.
/// A table inside an array's repeat (`[input.keymap.rule.scope]` after
/// `[[input.keymap.rule]]`) has the index of that repeat.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub table: String,
    pub index: usize,
    pub key: String,
    pub val: Val,
}

/// Reads the simple TOML that one-mk writes (tables, arrays of tables,
/// strings, numbers, booleans and lists, also across lines). Lines it
/// doesn't understand are skipped: the window only shows these values,
/// and `one-mk` remains the judge of what the file means.
pub fn read_toml(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let (mut table, mut index) = (String::new(), 0usize);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let line = strip_comment(line);
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(name) = t.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
            table = name.trim().to_owned();
            let n = counts.entry(table.clone()).or_default();
            index = *n;
            *n += 1;
            continue;
        }
        if let Some(name) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            table = name.trim().to_owned();
            // Inside the latest repeat of the longest array it belongs to.
            index = counts
                .iter()
                .filter(|(a, _)| table.strip_prefix(a.as_str()).is_some_and(|rest| rest.starts_with('.')))
                .max_by_key(|(a, _)| a.len())
                .map_or(0, |(_, n)| n - 1);
            continue;
        }
        let Some((key, rest)) = t.split_once('=') else { continue };
        let mut value = rest.trim().to_owned();
        // A list may go on over several lines.
        while value.starts_with('[') && !balanced(&value) {
            let Some(next) = lines.next() else { break };
            value.push(' ');
            value.push_str(strip_comment(next).trim());
        }
        let key = unquote(key.trim()).unwrap_or_else(|| key.trim().to_owned());
        out.push(Entry { table: table.clone(), index, key, val: parse_val(&value) });
    }
    out
}

/// The line without a `#` comment (one outside a string).
fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (Some('"'), '\\') if !escaped => {
                escaped = true;
                continue;
            }
            (Some(q), c) if c == q && !escaped => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') => return &line[..i],
            _ => {}
        }
        escaped = false;
    }
    line
}

/// Whether every `[` outside a string is closed.
fn balanced(s: &str) -> bool {
    let (mut depth, mut quote, mut escaped) = (0i32, None, false);
    for c in s.chars() {
        match (quote, c) {
            (Some('"'), '\\') if !escaped => {
                escaped = true;
                continue;
            }
            (Some(q), c) if c == q && !escaped => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            _ => {}
        }
        escaped = false;
    }
    depth <= 0
}

fn unquote(s: &str) -> Option<String> {
    if let Some(inner) = s.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return Some(inner.to_owned());
    }
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'u' => out.push(char::from_u32(u32::from_str_radix(&it.by_ref().take(4).collect::<String>(), 16).ok()?)?),
            'U' => out.push(char::from_u32(u32::from_str_radix(&it.by_ref().take(8).collect::<String>(), 16).ok()?)?),
            c => out.push(c),
        }
    }
    Some(out)
}

fn parse_val(s: &str) -> Val {
    let s = s.trim();
    match s {
        "true" => return Val::Bool(true),
        "false" => return Val::Bool(false),
        _ => {}
    }
    if let Ok(n) = s.replace('_', "").parse::<i64>() {
        return Val::Int(n);
    }
    let numeric = s.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+');
    if numeric && let Ok(x) = s.replace('_', "").parse::<f64>() {
        return Val::Float(x);
    }
    if let Some(text) = unquote(s) {
        return Val::Str(text);
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Val::List(split_list(inner).iter().map(|v| parse_val(v)).collect());
    }
    if let Some(inner) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        let pairs = split_list(inner)
            .iter()
            .filter_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                let k = k.trim();
                Some((unquote(k).unwrap_or_else(|| k.to_owned()), parse_val(v)))
            })
            .collect();
        return Val::Table(pairs);
    }
    Val::Other(s.to_owned())
}

/// The items of a list, split at commas outside strings and inner lists.
fn split_list(s: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let (mut depth, mut quote, mut escaped) = (0i32, None, false);
    for c in s.chars() {
        match (quote, c) {
            (Some('"'), '\\') if !escaped => {
                escaped = true;
                cur.push(c);
                continue;
            }
            (Some(q), c) if c == q && !escaped => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '[' | '{') => depth += 1,
            (None, ']' | '}') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        escaped = false;
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).collect()
}

// ---------- the settings, as the window shows them ----------

/// The largest clipboard one-mk will carry, in MB: the same ceiling as
/// `one-mk clipboard --max-mb` (`CEILING_BYTES` in the clipboard crate).
pub const CLIP_MAX_MB: u32 = 256;

/// Whether this desktop can have file edge drops at all (Windows, macOS, or
/// Linux on X11; not Wayland). The same rule as `one_mk_input::dropzone::supported`.
pub fn drop_zones_available() -> bool {
    cfg!(any(windows, target_os = "macos")) || (cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some())
}

/// `settings.toml` (and `profiles.toml`), with one-mk's defaults for what
/// isn't there. The same defaults as `crates/one-mk/src/settings.rs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub clip_text: bool,
    pub clip_html: bool,
    pub clip_image: bool,
    pub clip_max_mb: u32,
    pub clip_allow_fetch: bool,
    pub clip_skip_sensitive: bool,
    pub drop_zones: bool,
    pub broadcast_idle: u32,
    pub pointer_draw: String,
    pub edge_wrap: String,
    pub edge_learn: String,
    pub edge_sides: Vec<(String, EdgePrefs)>,
    pub history_persist: bool,
    pub history_enabled: bool,
    pub notify_files: bool,
    /// `[window] appearance`: System (the default), Light or Dark.
    pub appearance: crate::style::Appearance,
    /// Gaming mode: `off`, `on` or `auto`.
    pub gaming: String,
    pub typing: &'static str,
    pub lock_sync: bool,
    /// The edge rules for every edge.
    pub edge_corner: u32,
    pub edge_dwell_ms: u32,
    /// `none`, `shift`, `ctrl` or `alt`.
    pub edge_modifier: String,
    pub edge_fullscreen: bool,
    pub block_screensaver: bool,
    pub sleep_sync: bool,
    pub hotkeys_on: bool,
    /// (what it does, chord as written) in the order `one-mk hotkeys` lists them.
    pub hotkeys: Vec<(&'static str, String)>,
    /// (when it runs, the command in words).
    pub hooks: Vec<(&'static str, String)>,
    pub hooks_timeout: u64,
    pub hooks_shell: bool,
    pub hook_options: HashMap<String, (String, u64, bool)>,
    /// Profile name: when it switches by itself, in words.
    pub auto: HashMap<String, String>,
    /// `[input]`: keys, buttons, pointer and scrolling.
    pub input: crate::tune::Input,
    /// `settings.toml` couldn't be read (the window shows defaults).
    pub unreadable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgePrefs {
    pub corner: u32,
    pub dwell: u32,
    pub modifier: String,
    pub fullscreen: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs::read("", "")
    }
}

pub const HOOK_EVENTS: [(&str, &str); 4] =
    [("on_enter", "Pointer arrives"), ("on_leave", "Pointer leaves"), ("on_peer_connect", "A computer connects"), ("on_peer_disconnect", "A computer disconnects")];

impl Prefs {
    pub fn read(settings: &str, profiles: &str) -> Prefs {
        let e = read_toml(settings);
        let get = |table: &str, key: &str| e.iter().find(|x| x.table == table && x.key == key).map(|x| &x.val);
        let flag = |table: &str, key: &str, def: bool| get(table, key).and_then(Val::as_bool).unwrap_or(def);
        let num = |table: &str, key: &str, def: i64| get(table, key).and_then(Val::as_int).unwrap_or(def);
        let text = |table: &str, key: &str, def: &str| get(table, key).and_then(Val::as_str).unwrap_or(def).to_owned();
        let hotkeys = one_mk_proto::shortcuts::defaults(cfg!(windows)).into_iter()
            .map(|shortcut| (shortcut.label, text("hotkeys", shortcut.key, shortcut.default))).collect();
        let hooks = HOOK_EVENTS
            .iter()
            .filter_map(|(key, label)| get("hooks", key).map(|value| (*label, match value { Val::Str(line) => line.clone(), _ => command_words(&value.words()) })).filter(|(_, command)| !command.is_empty()))
            .collect();
        let mut auto = HashMap::new();
        let p = read_toml(profiles);
        for i in 0.. {
            let mine: Vec<&Entry> = p.iter().filter(|x| x.table == "profile" && x.index == i).collect();
            if mine.is_empty() {
                break;
            }
            let field = |k: &str| mine.iter().find(|x| x.key == k).map(|x| x.val.words()).unwrap_or_default();
            let Some(name) = field("name").pop() else { continue };
            let rules = auto_rules(&field("ssid"), &field("network"));
            if !rules.is_empty() {
                auto.insert(name, rules);
            }
        }
        Prefs {
            clip_text: flag("clipboard", "text", true),
            clip_html: flag("clipboard", "html", true),
            clip_image: flag("clipboard", "image", true),
            clip_max_mb: num("clipboard", "max_mb", 25).clamp(1, i64::from(CLIP_MAX_MB)) as u32,
            // Off unless the file says so: oneMK writes `allow_pull` only when it is on, and
            // a file from before `settings_version = 2` has its old `true` switched off.
            clip_allow_fetch: num("", "settings_version", 1) >= 2 && flag("clipboard", "allow_pull", false),
            clip_skip_sensitive: flag("clipboard", "skip_sensitive", true),
            // Opt-in: on only when the file says so, with the current settings version (older files held a default written out by whichever session saved first).
            drop_zones: num("", "settings_version", 1) >= 3 && flag("drag", "enabled", false),
            broadcast_idle: num("broadcast", "idle_off_minutes", 15).clamp(0, 1440) as u32,
            pointer_draw: text("pointer", "draw_when_no_mouse", "auto"),
            edge_wrap: get("edge", "wrap").and_then(Val::as_bool).map(|on| if on { "on" } else { "off" }.to_owned()).unwrap_or_else(|| text("edge", "wrap", "off")),
            edge_learn: text("edge", "learn", "suggest"),
            edge_sides: ["left", "right", "top", "bottom"].into_iter().map(|side| {
                let table = format!("edge.{side}");
                (side.to_owned(), EdgePrefs {
                    corner: num(&table, "corner", num("edge", "corner", 0)).max(0) as u32,
                    dwell: num(&table, "dwell_ms", num("edge", "dwell_ms", 0)).max(0) as u32,
                    modifier: text(&table, "modifier", &text("edge", "modifier", "none")),
                    fullscreen: flag(&table, "fullscreen_guard", flag("edge", "fullscreen_guard", false)),
                })
            }).collect(),
            history_persist: flag("clipboard", "history_persist", false),
            history_enabled: num("clipboard", "history_max", 0) > 0 && num("clipboard", "history_max_mb", 16) > 0,
            notify_files: flag("", "notify_files", true),
            appearance: crate::style::Appearance::parse(&text("window", "appearance", "system")),
            gaming: text("", "gaming", "off"),
            typing: match text("typing", "keys", "physical").as_str() {
                "characters" => "characters",
                "auto" => "auto",
                _ => "physical",
            },
            lock_sync: flag("", "lock_sync", false),
            edge_corner: num("edge", "corner", 0).max(0) as u32,
            edge_dwell_ms: num("edge", "dwell_ms", 0).max(0) as u32,
            edge_modifier: text("edge", "modifier", "none"),
            edge_fullscreen: flag("edge", "fullscreen_guard", false),
            block_screensaver: flag("power", "block_screensaver_on_peers", false),
            sleep_sync: flag("power", "sleep_sync", false),
            hotkeys_on: flag("hotkeys", "enabled", true),
            hotkeys,
            hooks,
            hooks_timeout: num("hooks", "timeout_secs", 5).max(1) as u64,
            hooks_shell: flag("hooks", "shell", false),
            hook_options: ["enter", "leave", "peer_connect", "peer_disconnect"].into_iter().map(|event| {
                let table = format!("hooks.options.{event}");
                let shell = !matches!(get("hooks", &format!("on_{event}")), Some(Val::List(_))) && flag(&table, "shell", flag("hooks", "shell", false));
                (event.into(), (text(&table, "computer", ""), num(&table, "timeout_secs", num("hooks", "timeout_secs", 5)).clamp(1, 60) as u64, shell))
            }).collect(),
            auto,
            input: crate::tune::Input::read(&e),
            unreadable: false,
        }
    }
}

/// A command's words as one line, quoting the ones with spaces.
fn command_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| format!("'{}'", word.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// "Wi-Fi “Home” or 192.168.1.0/24", as `one-mk profile list` says it.
fn auto_rules(ssids: &[String], networks: &[String]) -> String {
    let mut parts: Vec<String> = ssids.iter().map(|s| format!("Wi-Fi “{s}”")).collect();
    parts.extend(networks.iter().map(|n| if n.contains('/') { n.clone() } else { format!("router {n}") }));
    parts.join(" or ")
}

/// `ctrl+alt+pagedown` as people write it: `Ctrl+Alt+PageDown`
/// (`Control+Option+…` on a Mac). `{n}` stands for 1 to 9; `none` is off.
pub fn chord_label(chord: &str, mac: bool) -> String {
    let chord = chord.trim();
    if chord.is_empty() || chord.eq_ignore_ascii_case("none") {
        return "Off".into();
    }
    chord
        .split('+')
        .map(|part| {
            let p = part.trim().to_ascii_lowercase();
            match p.as_str() {
                "ctrl" | "control" => (if mac { "Control" } else { "Ctrl" }).to_owned(),
                "alt" | "option" => (if mac { "Option" } else { "Alt" }).to_owned(),
                "shift" => "Shift".into(),
                "super" | "meta" | "win" | "cmd" | "command" => (if mac { "Cmd" } else { "Super" }).to_owned(),
                "{n}" => "1…9".into(),
                "pagedown" => "PageDown".into(),
                "pageup" => "PageUp".into(),
                "esc" | "escape" => "Esc".into(),
                _ => {
                    let mut c = p.chars();
                    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

// ---------- doctor ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Skip,
    Warn,
    Fail,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// doctor's own name for it (`daemon`, `pairing`, …).
    pub id: String,
    pub title: String,
    pub verdict: Verdict,
    pub detail: String,
    pub fix: Option<String>,
    pub action: Option<String>,
}

/// An error from `one-mk` or the service as the window shows it: what
/// happened, in its own words, without the steps meant for a terminal
/// (commands, `settings.toml`). Only when nothing but such steps is left
/// is it replaced by a pointer to the window's own options.
pub fn window_issue(text: &str) -> String {
    const COMMANDS: [&str; 16] = ["one-mk run", "one-mk install", "one-mk pair", "one-mk hooks", "one-mk edge", "one-mk send", "one-mk keymap", "one-mk clipboard", "one-mk hotkeys", "one-mk takeback", "one-mk gestures", "one-mk invite", "one-mk join", "one-mk doctor", "sudo ", "chmod "];
    let terminal = |part: &str| part.contains('`') || part.contains("settings.toml") || COMMANDS.iter().any(|command| part.contains(command));
    if text.contains("settings.toml") && (text.contains("Permission denied") || text.contains("Read-only file system")) {
        return "oneMK isn't allowed to save its settings here (permission denied).".into();
    }
    // A step in brackets goes ("… (`one-mk clipboard` there)"), then each
    // clause that is one ("…; on Mac run: one-mk clipboard --allow-fetch on").
    let mut plain = text.to_owned();
    while let Some(open) = plain.find(" (") {
        let Some(close) = plain[open..].find(')').map(|c| open + c) else { break };
        if !terminal(&plain[open..=close]) {
            break;
        }
        plain.replace_range(open..=close, "");
    }
    let kept: Vec<&str> = plain.split("; ").map(str::trim).filter(|part| !part.is_empty() && !terminal(part)).collect();
    if kept.is_empty() || (kept.len() == 1 && kept[0].len() < 12) {
        return if text.contains("settings.toml") { "Check the options in Settings, then try again.".into() } else { "Review the options in this window, or restart oneMK and try again.".into() };
    }
    if kept.len() == 1 && kept[0] == text {
        return text.replace("one-mk", "oneMK");
    }
    let mut said = kept.join("; ").trim_end_matches([':', ',', ' ']).to_owned();
    if !said.ends_with(['.', '!', '?']) {
        said.push('.');
    }
    said.replace("one-mk", "oneMK")
}

/// What `one-mk doctor --json` found, worst first, keeping its order
/// otherwise.
pub fn parse_doctor(text: &str) -> Option<Vec<Check>> {
    let json = Json::parse(text.trim())?;
    let Json::Arr(list) = json.get("checks")? else { return None };
    let mut checks: Vec<Check> = list
        .iter()
        .filter_map(|c| {
            if matches!(c.get("visible"), Some(Json::Bool(false))) { return None; }
            let s = |k: &str| match c.get(k) {
                Some(Json::Str(s)) => Some(s.clone()),
                _ => None,
            };
            let verdict = match s("status")?.as_str() {
                "pass" => Verdict::Pass,
                "skip" => Verdict::Skip,
                "warn" => Verdict::Warn,
                _ => Verdict::Fail,
            };
            let check = Check { id: s("id").unwrap_or_default(), title: s("title")?, verdict, detail: s("detail").unwrap_or_default(), fix: s("fix").filter(|f| !f.is_empty()), action: s("action") };
            if c.get("visible").is_some() { Some(check) } else { legacy_check(check) }
        })
        .collect();
    checks.sort_by_key(|c| std::cmp::Reverse(c.verdict as u8));
    Some(checks)
}

/// The UDP port a doctor line names ("UDP port 24900", "24900/udp", "UDP 24900").
fn udp_port(text: &str) -> Option<u16> {
    let words: Vec<&str> = text.split(|c: char| !c.is_ascii_alphanumeric() && c != '/').filter(|w| !w.is_empty()).collect();
    for (k, w) in words.iter().enumerate() {
        if let Some(n) = w.strip_suffix("/udp").and_then(|n| n.parse().ok()) {
            return Some(n);
        }
        if w.eq_ignore_ascii_case("udp") {
            let next = words.get(k + 1).filter(|n| n.eq_ignore_ascii_case("port")).map_or(k + 1, |_| k + 2);
            if let Some(n) = words.get(next).and_then(|n| n.parse().ok()) {
                return Some(n);
            }
        }
    }
    None
}

fn legacy_check(mut check: Check) -> Option<Check> {
    let good = check.verdict == Verdict::Pass;
    let detail = check.detail.as_str();
    let (title, plain, fix) = match check.id.as_str() {
        "typing" => {
            // The daemon's summary ("physical (same keys; …)") as a sentence.
            let d = detail.to_lowercase();
            check.detail = if d.starts_with("physical") {
                "Keys are sent by position; each computer applies its own keyboard layout.".into()
            } else if d.starts_with("characters") {
                "Letters are sent as you type them, whatever the other computer's keyboard layout.".into()
            } else if d.is_empty() {
                check.detail.clone()
            } else {
                let mut c = check.detail.chars();
                c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
            };
            return Some(check);
        }
        "updates" | "local-key-rules" | "input-tuning" => return Some(check),
        "wallpaper" if check.detail != "not shipped" => return Some(check),
        "wallpaper" | "wallpaper-import-sandbox" | "drag" => return None,
        "daemon" if detail.contains("not running") => {
            check.verdict = Verdict::Skip;
            check.action = Some("Start".into());
            ("Sharing", "Sharing is stopped.", Some("Start sharing."))
        }
        "daemon" => ("Sharing", if good { "Sharing is on." } else { "Sharing is not responding." }, None),
        "pairing" if detail.contains("not paired") => {
            check.verdict = Verdict::Skip;
            check.action = Some("Pair".into());
            ("Paired computers", "No computers paired yet.", Some("Pair a computer to start sharing."))
        }
        "pairing" => ("Paired computers", if good { "Your paired computers are saved." } else { "Your paired computers could not be read." }, None),
        "config" => {
            if detail.contains("no settings folder") { check.verdict = Verdict::Pass; }
            ("Settings", if check.verdict == Verdict::Pass { "Your settings are saved automatically." } else { "Some saved settings could not be read." }, None)
        }
        "identity" => {
            if check.verdict != Verdict::Fail { check.verdict = Verdict::Pass; }
            ("Secure identity", "A secure identity is created automatically when sharing starts.", None)
        }
        "port" if !good => {
            // Say which port and, when doctor knows it, which program.
            let port = udp_port(detail);
            let who = detail.split('"').nth(1).map(str::to_owned);
            check.title = "Network connection".into();
            check.detail = match (&port, &who) {
                (Some(p), Some(w)) => format!("UDP port {p} is already used by {w}."),
                (Some(p), None) => format!("UDP port {p} is already in use."),
                _ => "Sharing cannot use its network connection.".into(),
            };
            check.fix = Some(match who {
                Some(w) => format!("Quit {w}, then choose Check again. Or use another port on both computers."),
                None => "Quit the program using it, then choose Check again. Or use another port on both computers.".into(),
            });
            return Some(check);
        }
        "port" => ("Network connection", "Ready for sharing.", None),
        "mdns" => ("Find nearby computers", if good { "Nearby computers can be found on this network." } else { "Nearby computers could not be found." }, if good { None } else { Some("Connect both computers to the same home or office network.") }),
        "input" | "portals" => {
            if check.verdict == Verdict::Skip { return None; }
            if detail.contains("Accessibility") || detail.contains("Input Monitoring") { check.action = Some("OpenPermissionGuide".into()); }
            ("Permissions", if good { "Your desktop allows sharing." } else { "Your desktop has not allowed sharing yet." }, if good { None } else { Some("Allow mouse and keyboard sharing in your desktop's permission settings.") })
        }
        "firewall" if !good => {
            // The port and the firewall by name, when doctor says them.
            let port = udp_port(detail).or_else(|| check.fix.as_deref().and_then(udp_port));
            let which = ["ufw", "firewalld", "Windows Defender Firewall", "macOS firewall"].into_iter().find(|f| detail.contains(f) || check.fix.as_deref().is_some_and(|x| x.contains(f)));
            check.title = "Firewall".into();
            check.detail = match (&port, which) {
                (Some(p), Some(f)) => format!("{f} is on and blocks UDP port {p}, so other computers can't reach this one."),
                (Some(p), None) => format!("The firewall blocks UDP port {p}, so other computers can't reach this one."),
                _ => "The firewall may be blocking sharing connections.".into(),
            };
            check.fix = Some(match port {
                Some(p) => format!("Allow incoming UDP on port {p} for oneMK in the firewall's settings, then choose Check again."),
                None => "Allow incoming connections for oneMK in the firewall's settings, then choose Check again.".into(),
            });
            return Some(check);
        }
        "firewall" => ("Firewall", "The firewall allows sharing connections.", None),
        "hotkeys" => ("Keyboard shortcuts", if good { "Keyboard shortcuts are ready." } else { "Keyboard shortcuts only work while the pointer is on another computer." }, None),
        "broadcast" => ("Share with several computers", if good { "Sharing with several computers at once is off." } else { "Your typing and clicks go to several computers at once." }, if good { None } else { Some("Press Ctrl+Alt+Shift+Esc to stop sharing with several computers.") }),
        "clipboard" => ("Clipboard sharing", "Your clipboard sharing preferences are saved.", None),
        "clipboard-history" => ("Clipboard history", "Saved clipboard history could not be read.", None),
        "hooks" => ("Automatic actions", "An automatic action was blocked for your safety.", None),
        "per-device-rules" => ("Individual keyboards and mice", if good { "Your keyboards and mice can have different preferences." } else { "Your desktop treats keyboards and mice as one device." }, None),
        "pointer-overlay" => ("Pointer visibility", if good { "The pointer can be shown when another computer controls this screen." } else { "This desktop cannot show an extra pointer." }, None),
        "edge-push" => {
            if detail.contains("Not yet tested") { check.verdict = Verdict::Pass; }
            ("Screen edges", "Your screen-edge preferences are saved.", None)
        }
        "network" => ("Wi-Fi or wired network", if good { "Connected to a home or office network." } else { "Connect both computers to the same home or office network." }, None),
        "link" => ("Connection quality", if good { "Your connected computers respond quickly." } else { "The connection is slow or unreliable." }, None),
        id if id.starts_with("peer:") => ("Paired computer", if good { "Connected." } else { "Not connected right now." }, if good { None } else { Some("Make sure sharing is on on both computers and they are on the same network.") }),
        _ => return None,
    };
    if check.verdict == Verdict::Skip && check.action.is_none() { return None; }
    check.title = title.to_owned();
    check.detail = plain.to_owned();
    check.fix = fix.map(str::to_owned);
    Some(check)
}

/// `one-mk input-devices --json`: the keyboards and mice seen here, and
/// whether this computer can tell them apart.
pub fn parse_input_devices(text: &str) -> Option<crate::tune::InputDevices> {
    let json = Json::parse(text.trim())?;
    let s = |j: &Json, k: &str| match j.get(k) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let b = |j: &Json, k: &str| matches!(j.get(k), Some(Json::Bool(true)));
    let Json::Arr(list) = json.get("devices")? else { return None };
    let devices = list
        .iter()
        .filter_map(|d| {
            Some(crate::tune::SeenDevice {
                fingerprint: s(d, "fingerprint")?,
                name: s(d, "name")?,
                bus: s(d, "bus").unwrap_or_default(),
                connection: s(d, "connection").unwrap_or_default(),
                keyboard: b(d, "keyboard"),
                pointer: b(d, "pointer"),
                connected: b(d, "connected"),
            })
        })
        .collect();
    Some(crate::tune::InputDevices { available: b(&json, "available"), note: s(&json, "note"), devices })
}

/// The command in a fix (`one-mk …` in backticks), which a Copy button
/// copies; without one, the whole fix (a backticked flag or a placeholder
/// alone is not a command to paste).
pub fn fix_command(fix: &str) -> (String, bool) {
    let cmd = fix
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .find(|c| (c.starts_with("one-mk ") || c.starts_with("sudo ") || c.starts_with("chmod ")) && !c.contains('<'));
    match cmd {
        Some(c) => (c.to_owned(), true),
        None => (fix.replace('`', ""), false),
    }
}

/// The keyboard shortcuts' state in a short sentence (`None`: nothing to say).
pub fn hotkeys_note(state: &str, detail: &str) -> Option<String> {
    Some(match state {
        "on" => "Ready. You can change shortcuts in Settings.".into(),
        "starting" => "Being set up.".into(),
        "unavailable" => "Shortcuts work only while the pointer is on another computer. You can also switch computers from the window.".into(),
        _ if !detail.is_empty() => "Off. You can change shortcuts in Settings.".into(),
        _ => return None,
    })
}

/// Just enough JSON for doctor's report.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn parse(s: &str) -> Option<Json> {
        let mut p = JsonParser { s: s.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let ok = self.s.get(self.i) == Some(&c);
        self.i += usize::from(ok);
        ok
    }
    fn word(&mut self, w: &str, v: Json) -> Option<Json> {
        self.s[self.i..].starts_with(w.as_bytes()).then(|| {
            self.i += w.len();
            v
        })
    }
    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.s.get(self.i)? {
            b'n' => self.word("null", Json::Null),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' => {
                self.i += 1;
                let mut items = vec![];
                if self.eat(b']') {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b']') {
                        return Some(Json::Arr(items));
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut kv = vec![];
                if self.eat(b'}') {
                    return Some(Json::Obj(kv));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    kv.push((k, self.value()?));
                    if self.eat(b'}') {
                        return Some(Json::Obj(kv));
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || b"+-.eE".contains(c)) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok().map(Json::Num)
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = vec![];
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = |p: &mut Self| -> Option<u32> {
                                let h = std::str::from_utf8(p.s.get(p.i..p.i + 4)?).ok()?;
                                p.i += 4;
                                u32::from_str_radix(h, 16).ok()
                            };
                            let hi = hex(self)?;
                            let code = if (0xD800..0xDC00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = hex(self)?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.checked_sub(0xDC00)?)
                            } else {
                                hi
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        other => other as char,
                    };
                    let mut buf = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
    }
}

// ---------- transfers ----------

/// A transfer as two lines and a bar: "Sending 3 files to desk" and
/// "45% · 12.3 MB of 27.1 MB · 8.1 MB/s · 1 s left", and how full the bar
/// is (`None` while the size isn't known).
pub fn transfer_lines(t: &TransferStatus) -> (String, String, Option<f32>) {
    let what = match t.files {
        0 => "files".to_owned(),
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    };
    let title = if t.sending { format!("Sending {what} to {}", t.peer) } else { format!("Receiving {what} from {}", t.peer) };
    let mut parts = vec![];
    match t.percent() {
        Some(p) => {
            parts.push(format!("{p}%"));
            parts.push(format!("{} of {}", human_bytes(t.done), human_bytes(t.total)));
        }
        None => parts.push(human_bytes(t.done)),
    }
    if t.rate > 0 {
        parts.push(format!("{}/s", human_bytes(t.rate)));
        if t.total > t.done {
            parts.push(format!("{} left", human_duration((t.total - t.done) / t.rate)));
        }
    } else {
        parts.push("stalled".into());
    }
    let frac = (t.total > 0).then(|| (t.done.min(t.total) as f64 / t.total as f64) as f32);
    (title, parts.join(" · "), frac)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcut_preferences_share_every_daemon_default() {
        let defaults = super::Prefs::read("", "");
        for (actual, shortcut) in defaults.hotkeys.iter().zip(one_mk_proto::shortcuts::defaults(cfg!(windows))) {
            assert_eq!(*actual, (shortcut.label, shortcut.default.to_owned()));
        }
        assert_eq!(defaults.hotkeys.len(), 6);
        let custom = super::Prefs::read("[hotkeys]\nbroadcast = 'none'\nfetch_clipboard = 'ctrl+alt+f'", "");
        assert_eq!(custom.hotkeys[4].1, "none");
        assert_eq!(custom.hotkeys[5].1, "ctrl+alt+f");
    }
    use super::*;

    #[test]
    fn hook_editor_preserves_literal_words_and_existing_scripts() {
        let prefs = Prefs::read("[hooks]\non_enter = [\"program with spaces\", \"it's\", \"$HOME\", \"\"]\non_leave = \"echo $HOME\"\nshell = true", "");
        assert_eq!(prefs.hooks[0].1, "'program with spaces' 'it'\\''s' '$HOME' ''");
        assert_eq!(prefs.hooks[1].1, "echo $HOME");
        assert!(!prefs.hook_options["enter"].2, "argument arrays run without a shell, even with the old shell setting on");
        assert!(prefs.hook_options["leave"].2);
        assert_eq!(Prefs::read("[hooks]\non_enter = ['program with spaces']", "").hooks[0].1, "'program with spaces'");
    }

    #[test]
    fn firewall_and_port_rows_name_the_port() {
        assert_eq!(udp_port("ufw is on and blocks UDP 24900."), Some(24900));
        assert_eq!(udp_port("Allow it: `sudo ufw allow 24910/udp comment one-mk`."), Some(24910));
        assert_eq!(udp_port("UDP port 24900 is already used by \"barrier\"."), Some(24900));
        assert_eq!(udp_port("Blocked."), None);
        let fw = legacy_check(Check { id: "firewall".into(), title: String::new(), detail: "ufw is on and blocks UDP 24900.".into(), fix: Some("Allow it: `sudo ufw allow 24900/udp`.".into()), verdict: Verdict::Fail, action: None }).unwrap();
        assert!(fw.detail.contains("ufw") && fw.detail.contains("24900"), "{}", fw.detail);
        assert!(fw.fix.as_deref().is_some_and(|f| f.starts_with("Allow") && !f.contains('`')));
        let port = legacy_check(Check { id: "port".into(), title: String::new(), detail: "UDP port 24900 is already used by \"barrier\".".into(), fix: None, verdict: Verdict::Fail, action: None }).unwrap();
        assert_eq!(port.detail, "UDP port 24900 is already used by barrier.");
    }

    #[test]
    fn window_diagnostics_never_offer_terminal_steps() {
        for text in ["Run one-mk invite", "Use one-mk takeback", "Edit settings.toml", "Try `one-mk hotkeys`", "sudo example"] {
            let shown = window_issue(text);
            assert!(!shown.contains("one-mk ") && !shown.contains("settings.toml") && !shown.contains('`'), "{shown}");
        }
    }

    /// What went wrong is kept; only the terminal steps go.
    #[test]
    fn window_diagnostics_keep_the_reason() {
        assert_eq!(window_issue("MacBook Air does not allow fetching its clipboard; on MacBook Air run: one-mk clipboard --allow-fetch on"), "MacBook Air does not allow fetching its clipboard.");
        assert_eq!(window_issue("its clipboard is empty, or it does not share it (`one-mk clipboard` there)"), "its clipboard is empty, or it does not share it.");
        assert_eq!(window_issue("saving /home/x/.config/one-mk/settings.toml: Permission denied (os error 13)"), "oneMK isn't allowed to save its settings here (permission denied).");
        assert_eq!(window_issue("Studio PC is offline"), "Studio PC is offline");
        assert_eq!(window_issue("Edit settings.toml"), "Check the options in Settings, then try again.");
        assert_eq!(window_issue("Try `one-mk hotkeys`"), "Review the options in this window, or restart oneMK and try again.");
        assert_eq!(window_issue("one-mk is busy (try again)"), "oneMK is busy (try again)");
    }

    #[test]
    fn desktop_settings_read_nested_overrides_and_defaults() {
        let defaults = Prefs::read("", "");
        assert!(defaults.clip_skip_sensitive && !defaults.history_persist && !defaults.drop_zones, "file edge drops are opt-in");
        assert!(!Prefs::read("settings_version = 2\n[drag]\nenabled=true\n", "").drop_zones, "an old file's true was a frozen default, not a choice");
        assert_eq!(CLIP_MAX_MB, 256, "the window and one-mk clipboard --max-mb share one ceiling");
        assert_eq!((defaults.pointer_draw.as_str(), defaults.edge_wrap.as_str(), defaults.edge_learn.as_str(), defaults.broadcast_idle), ("auto", "off", "suggest", 15));
        let settings = "settings_version = 3\n[clipboard]\nskip_sensitive=false\nhistory_persist=true\nhistory_max=20\n[drag]\nenabled=true\n[broadcast]\nidle_off_minutes=7\n[pointer]\ndraw_when_no_mouse='always'\n[edge]\nwrap='all'\nlearn='suggest'\ncorner=12\ndwell_ms=100\nmodifier='ctrl'\n[edge.left]\ndwell_ms=250\n[hooks]\non_enter='echo hi'\ntimeout_secs=9\nshell=false\n[hooks.options.enter]\ncomputer='abcdef01'\ntimeout_secs=12\nshell=true\n";
        let prefs = Prefs::read(settings, "");
        assert!(!prefs.clip_skip_sensitive && prefs.history_persist && prefs.drop_zones);
        assert_eq!(prefs.broadcast_idle, 7);
        assert_eq!((prefs.pointer_draw.as_str(), prefs.edge_wrap.as_str(), prefs.edge_learn.as_str()), ("always", "all", "suggest"));
        let left = &prefs.edge_sides.iter().find(|(side, _)| side == "left").unwrap().1;
        assert_eq!((left.corner, left.dwell, left.modifier.as_str()), (12, 250, "ctrl"));
        assert_eq!(prefs.hook_options["enter"], ("abcdef01".into(), 12, true));
        assert_eq!(prefs.hook_options["leave"], (String::new(), 9, false));
        assert_eq!(Prefs::read("[edge]\nwrap=true", "").edge_wrap, "on");
    }

    #[test]
    fn typing_preferences_default_and_parse_only_known_modes() {
        for (settings, expected) in [
            ("", "physical"),
            ("[typing]\nkeys = 'physical'", "physical"),
            ("[typing]\nkeys = 'characters'", "characters"),
            ("[typing]\nkeys = 'auto'", "auto"),
            ("[typing]\nkeys = 'unknown'", "physical"),
        ] {
            assert_eq!(Prefs::read(settings, "").typing, expected);
        }
    }

    const SETTINGS: &str = r#"notify_files = false
gaming = "auto"
lock_sync = true

[clipboard]
text = true
html = false # rich text off
image = true
max_mb = 10

[edge]
corner = 20
dwell_ms = 300
modifier = "shift"
fullscreen_guard = true
fullscreen_allow = []

[edge.left]
dwell_ms = 0

[hooks]
on_enter = [
    "notify-send",
    "hi there",
]
on_leave = "echo # not a comment"
timeout_secs = 9

[power]
sleep_sync = true

[hotkeys]
enabled = false
cycle = "ctrl+alt+n"
"#;

    const PROFILES: &str = r#"active = "Café, 2nd floor"

[[profile]]
name = "home"
ssid = ["Home Wi-Fi"]
network = ["192.168.1.0/24"]

[[profile.device]]
name = "Studio PC"
side = "left"

[[profile]]
name = "Café, 2nd floor"
"#;

    #[test]
    fn reads_what_one_mk_writes() {
        let p = Prefs::read(SETTINGS, PROFILES);
        assert!(!p.notify_files && p.clip_text && !p.clip_html && p.clip_image);
        assert_eq!((p.gaming.as_str(), p.lock_sync), ("auto", true));
        assert_eq!(p.clip_max_mb, 10);
        assert_eq!((p.edge_corner, p.edge_dwell_ms, p.edge_modifier.as_str(), p.edge_fullscreen), (20, 300, "shift", true));
        assert!(p.sleep_sync && !p.block_screensaver && !p.hotkeys_on);
        assert_eq!(p.hotkeys[1], ("The next computer", "ctrl+alt+n".to_owned()));
        assert_eq!(p.hotkeys[0].1, if cfg!(windows) { "ctrl+alt+shift+{n}" } else { "ctrl+alt+{n}" }, "unset shortcuts are the defaults");
        assert_eq!(p.hooks, vec![("Pointer arrives", "'notify-send' 'hi there'".to_owned()), ("Pointer leaves", "echo # not a comment".to_owned())]);
        assert_eq!(p.hooks_timeout, 9);
        assert_eq!(p.auto.get("home").map(String::as_str), Some("Wi-Fi “Home Wi-Fi” or 192.168.1.0/24"));
        assert!(!p.auto.contains_key("Café, 2nd floor"));
        // Nothing written yet: one-mk's defaults.
        let d = Prefs::default();
        assert!(d.notify_files && d.clip_html && d.hotkeys_on && !d.clip_allow_fetch);
        // Fetching is on only with the current settings version and an explicit true.
        let on = |s: &str| Prefs::read(s, "").clip_allow_fetch;
        assert!(on("settings_version = 2\n[clipboard]\nallow_pull = true\n"));
        assert!(!on("settings_version = 2\n[clipboard]\ntext = true\n"), "absent means off");
        assert!(!on("[clipboard]\nallow_pull = true\n"), "an old file's true is switched off");
        assert_eq!((d.clip_max_mb, d.edge_modifier.as_str(), d.hooks.len(), d.gaming.as_str()), (25, "none", 0, "off"));
    }

    #[test]
    fn toml_reader_handles_strings_and_lists() {
        let e = read_toml("a = \"x\\\"y\\u00e9\"\nb = 'lit\\n'\n[t]\nc = [[1, 2], [3]]\nd = 1_000\nweird line\n");
        assert_eq!(e[0].val, Val::Str("x\"yé".into()));
        assert_eq!(e[1].val, Val::Str("lit\\n".into()));
        assert_eq!((e[2].table.as_str(), &e[2].val), ("t", &Val::List(vec![Val::List(vec![Val::Int(1), Val::Int(2)]), Val::List(vec![Val::Int(3)])])));
        assert_eq!(e[3].val, Val::Int(1000));
        assert_eq!(e.len(), 4, "a line that isn't key = value is skipped");
    }

    #[test]
    fn toml_reader_handles_inline_tables_floats_and_tables_in_repeats() {
        let e = read_toml(
            "[[r]]\nscope = { os = \"mac\", n = [1, 2] }\nspeed = 1.5\n[r.scope]\nos = \"linux\"\n[[r]]\nx = -0.25\n[r.scope]\npeer = \"a, b\"\n[other]\ny = 1e3\n",
        );
        let table = Val::Table(vec![("os".into(), Val::Str("mac".into())), ("n".into(), Val::List(vec![Val::Int(1), Val::Int(2)]))]);
        assert_eq!((e[0].table.as_str(), e[0].index, &e[0].val), ("r", 0, &table));
        assert_eq!(e[1].val, Val::Float(1.5));
        assert_eq!((e[2].table.as_str(), e[2].index), ("r.scope", 0), "a table inside the first repeat");
        assert_eq!((e[3].table.as_str(), e[3].index, e[3].val.as_f64()), ("r", 1, Some(-0.25)));
        assert_eq!((e[4].table.as_str(), e[4].index, e[4].val.as_str()), ("r.scope", 1, Some("a, b")));
        assert_eq!((e[5].table.as_str(), e[5].index, e[5].val.as_f64()), ("other", 0, Some(1000.0)));
    }

    #[test]
    fn chords_read_like_keys() {
        assert_eq!(chord_label("ctrl+alt+{n}", false), "Ctrl+Alt+1…9");
        assert_eq!(chord_label("ctrl+alt+pagedown", false), "Ctrl+Alt+PageDown");
        assert_eq!(chord_label("ctrl+alt+p", true), "Control+Option+P");
        assert_eq!(chord_label("none", false), "Off");
    }

    #[test]
    fn doctor_report_is_read_worst_first() {
        let json = r#"{"ok":false,"summary":{"pass":1,"skip":0,"warn":1,"fail":1},"checks":[
            {"id":"config","title":"Settings","status":"pass","detail":"Saved for é😀.","fix":null,"visible":true},
            {"id":"input","title":"Permissions","status":"warn","detail":"Not allowed yet.","fix":"Choose Allow.","visible":true},
            {"id":"daemon","title":"Sharing","status":"fail","detail":"Sharing is not responding.","fix":"Try again.","visible":true}]}"#;
        let checks = parse_doctor(json).unwrap();
        assert_eq!(checks.iter().map(|c| c.verdict).collect::<Vec<_>>(), [Verdict::Fail, Verdict::Warn, Verdict::Pass]);
        assert_eq!(checks[0].detail, "Sharing is not responding.");
        assert_eq!(checks[2].detail, "Saved for é😀.");
        assert_eq!(checks[2].fix, None);
        assert_eq!(fix_command(checks[1].fix.as_ref().unwrap()), ("Choose Allow.".to_owned(), false));
        assert_eq!(fix_command("Start it."), ("Start it.".to_owned(), false));
        // A flag or a placeholder isn't something to paste; a later command is.
        let fix = "Was it started with `--no-discovery`? Connect with `one-mk run --connect <ip>:24900`.";
        assert!(!fix_command(fix).1);
        assert_eq!(hotkeys_note("unavailable", "no portal").as_deref(), Some("Shortcuts work only while the pointer is on another computer. You can also switch computers from the window."));
        assert_eq!(hotkeys_note("on", "through Windows").as_deref(), Some("Ready. You can change shortcuts in Settings."));
        assert_eq!(hotkeys_note("off", ""), None);
        assert_eq!(parse_doctor("one-mk: error"), None);
        assert_eq!(parse_doctor("{\"checks\": [1, }"), None);
    }

    #[test]
    fn doctor_actions_are_optional_and_hidden_rows_are_ignored() {
        let checks = parse_doctor(r#"{"checks":[
            {"id":"daemon","title":"Sharing","status":"skip","detail":"Sharing is stopped.","action":"Start","visible":true},
            {"id":"pairing","title":"Paired computers","status":"skip","detail":"No computers paired yet.","action":"Pair","visible":true},
            {"id":"input","title":"Permissions","status":"warn","detail":"Allow Input Monitoring.","action":"OpenPermissionGuide","visible":true},
            {"id":"firewall","title":"Firewall","status":"fail","detail":"Blocked.","action":"OpenFirewallSettings","visible":true},
            {"id":"wallpaper","title":"Wallpaper","status":"fail","detail":"not shipped","visible":false},
            {"id":"config","title":"Settings","status":"pass","detail":"Ready.","visible":true}
        ]}"#).unwrap();
        assert_eq!(checks.len(), 5);
        for action in ["Start", "Pair", "OpenPermissionGuide", "OpenFirewallSettings"] {
            assert!(checks.iter().any(|check| check.action.as_deref() == Some(action)));
        }
        assert_eq!(checks.last().unwrap().action, None);
    }

    #[test]
    fn older_doctor_reports_are_plain_and_normal_states_stay_neutral() {
        let checks = parse_doctor(r#"{"checks":[
            {"id":"daemon","title":"Program","status":"fail","detail":"one-mk is not running on this computer.","fix":"one-mk run"},
            {"id":"pairing","title":"Paired computers","status":"fail","detail":"This computer is not paired with any other.","fix":"one-mk pair"},
            {"id":"config","title":"Settings","status":"warn","detail":"There is no settings folder yet (settings.toml)."},
            {"id":"identity","title":"Identity","status":"warn","detail":"This computer has no identity yet."},
            {"id":"wallpaper-import-sandbox","title":"Wallpaper import sandbox","status":"skip","detail":"not shipped"},
            {"id":"clipboard-history","title":"Clipboard history","status":"skip","detail":"compile-checked"},
            {"id":"broadcast","title":"Broadcast","status":"pass","detail":"Broadcast has not been tried on this system yet (untested)."},
            {"id":"edge-push","title":"Edges with no computer","status":"warn","detail":"Not yet tested on macOS.","fix":"one-mk edge"}
        ]}"#).unwrap();
        assert_eq!(checks.len(), 6);
        assert!(checks.iter().any(|check| check.action.as_deref() == Some("Start") && check.verdict == Verdict::Skip));
        assert!(checks.iter().any(|check| check.action.as_deref() == Some("Pair") && check.verdict == Verdict::Skip));
        for check in checks {
            let text = format!("{} {} {}", check.title, check.detail, check.fix.unwrap_or_default());
            for forbidden in ["one-mk ", "settings.toml", "`", "untested", "compile", "sandbox", "MacApp"] {
                assert!(!text.contains(forbidden), "{text}");
            }
        }
    }

    #[test]
    fn transfers_read_as_two_lines_and_a_bar() {
        let t = TransferStatus { id: 1, sending: false, peer: "desk".into(), files: 3, done: 12_300_000, total: 27_100_000, rate: 8_100_000 };
        let (title, detail, frac) = transfer_lines(&t);
        assert_eq!(title, "Receiving 3 files from desk");
        assert_eq!(detail, "45% · 12.3 MB of 27.1 MB · 8.1 MB/s · 1 s left");
        assert!((frac.unwrap() - 0.4538).abs() < 0.001);
        let t = TransferStatus { sending: true, files: 0, total: 0, done: 2_000_000, rate: 0, ..t };
        assert_eq!(transfer_lines(&t), ("Sending files to desk".into(), "2.0 MB · stalled".into(), None));
    }
    #[test]
    fn input_devices_are_read() {
        let text = r#"{"available":true,"note":null,"running":true,"problem":null,"devices":[{"fingerprint":"usb/046d:c077//Mouse \"M\"","name":"Mouse \"M\"","bus":"usb","connection":"USB","ids":"046d:c077","serial":null,"keyboard":false,"pointer":true,"connected":true,"first_seen":1,"last_seen":2,"twins":false},{"name":"no fingerprint"}]}"#;
        let d = parse_input_devices(text).unwrap();
        assert!(d.available && d.note.is_none());
        assert_eq!(d.devices.len(), 1, "an entry without a fingerprint is skipped");
        assert_eq!((d.devices[0].name.as_str(), d.devices[0].pointer, d.devices[0].connected), ("Mouse \"M\"", true, true));
        let off = parse_input_devices(r#"{"available":false,"note":"merged","devices":[]}"#).unwrap();
        assert_eq!((off.available, off.note.as_deref()), (false, Some("merged")));
        assert_eq!(parse_input_devices("one-mk: error"), None);
        assert_eq!(parse_input_devices(r#"{"available":true}"#), None);
    }

}
