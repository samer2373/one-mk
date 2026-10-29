//! What the newer panels show, worked out without drawing anything: the
//! settings as `settings.toml` has them, `one-mk doctor --json` results,
//! where a computer sits on the layout plan and how a drag along its edge
//! snaps, and transfers in words. Everything here is plain data in, plain
//! data out, so it is tested without a window.

use one_mk_ctl::status::{TransferStatus, human_bytes, human_duration, placement};
use one_mk_proto::{Align, Desktop, Rect, Seam, Side};
use std::collections::HashMap;

// ---------- a small reader for the TOML one-mk writes ----------

/// A value in `settings.toml` or `profiles.toml`, as far as the window cares.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Val>),
    /// Anything else (dates, floats, inline tables): kept as written.
    Other(String),
}

impl Val {
    fn as_bool(&self) -> Option<bool> {
        if let Val::Bool(b) = self { Some(*b) } else { None }
    }
    fn as_int(&self) -> Option<i64> {
        if let Val::Int(n) = self { Some(*n) } else { None }
    }
    fn as_str(&self) -> Option<&str> {
        if let Val::Str(s) = self { Some(s) } else { None }
    }
    /// Strings in a list (a one-word string counts as a list of one).
    fn words(&self) -> Vec<String> {
        match self {
            Val::Str(s) => vec![s.clone()],
            Val::List(l) => l.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => vec![],
        }
    }
}

/// One `key = value` line: under which table (`edge.left`; empty at the
/// top), which repeat of an `[[array]]` table (0 otherwise), and the key.
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
            index = 0;
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
    if let Some(text) = unquote(s) {
        return Val::Str(text);
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Val::List(split_list(inner).iter().map(|v| parse_val(v)).collect());
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
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
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

/// `settings.toml` (and `profiles.toml`), with one-mk's defaults for what
/// isn't there. The same defaults as `crates/one-mk/src/settings.rs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub clip_text: bool,
    pub clip_html: bool,
    pub clip_image: bool,
    pub clip_max_mb: u32,
    pub clip_allow_fetch: bool,
    pub notify_files: bool,
    /// Gaming mode: `off`, `on` or `auto`.
    pub gaming: String,
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
    /// Profile name: when it switches by itself, in words.
    pub auto: HashMap<String, String>,
    /// `settings.toml` couldn't be read (the window shows defaults).
    pub unreadable: bool,
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
        let hotkeys = vec![
            ("Jump to computer N", text("hotkeys", "jump", "ctrl+alt+{n}")),
            ("The next computer", text("hotkeys", "cycle", "ctrl+alt+pagedown")),
            ("Lock the pointer here", text("hotkeys", "lock_pointer", "ctrl+alt+p")),
            ("Back to this computer", text("hotkeys", "home", "ctrl+alt+h")),
        ];
        let hooks = HOOK_EVENTS
            .iter()
            .filter_map(|(key, label)| get("hooks", key).map(|v| (*label, command_words(&v.words()))).filter(|(_, c)| !c.is_empty()))
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
            clip_max_mb: num("clipboard", "max_mb", 25).clamp(1, 4096) as u32,
            clip_allow_fetch: flag("clipboard", "allow_pull", true),
            notify_files: flag("", "notify_files", true),
            gaming: text("", "gaming", "off"),
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
            auto,
            unreadable: false,
        }
    }
}

/// A command's words as one line, quoting the ones with spaces.
fn command_words(words: &[String]) -> String {
    if words.len() == 1 {
        return words[0].clone();
    }
    words
        .iter()
        .map(|w| if w.is_empty() || w.contains(char::is_whitespace) { format!("'{}'", w.replace('\'', "'\\''")) } else { w.clone() })
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
}

/// What `one-mk doctor --json` found, worst first, keeping its order
/// otherwise.
pub fn parse_doctor(text: &str) -> Option<Vec<Check>> {
    let json = Json::parse(text.trim())?;
    let Json::Arr(list) = json.get("checks")? else { return None };
    let mut checks: Vec<Check> = list
        .iter()
        .filter_map(|c| {
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
            Some(Check { id: s("id").unwrap_or_default(), title: s("title")?, verdict, detail: s("detail").unwrap_or_default(), fix: s("fix").filter(|f| !f.is_empty()) })
        })
        .collect();
    checks.sort_by_key(|c| std::cmp::Reverse(c.verdict as u8));
    Some(checks)
}

/// "2 problems, 1 warning" / "Everything checks out".
pub fn doctor_summary(checks: &[Check]) -> String {
    let n = |v: Verdict| checks.iter().filter(|c| c.verdict == v).count();
    let (fail, warn) = (n(Verdict::Fail), n(Verdict::Warn));
    let plural = |k: usize, one: &str, many: &str| if k == 1 { format!("1 {one}") } else { format!("{k} {many}") };
    match (fail, warn) {
        (0, 0) => "Everything checks out".into(),
        (0, w) => plural(w, "warning", "warnings"),
        (f, 0) => plural(f, "problem", "problems"),
        (f, w) => format!("{}, {}", plural(f, "problem", "problems"), plural(w, "warning", "warnings")),
    }
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
    let d = detail.trim();
    let why = if d.is_empty() { String::new() } else { format!(" ({d})") };
    Some(match state {
        "on" => format!("Working{}.", if d.is_empty() { String::new() } else { format!(" {d}") }),
        "starting" => format!("Being set up{why}."),
        "unavailable" => format!("On this computer they work only while the pointer is on another one{why}."),
        _ if !d.is_empty() => format!("Off: {d}."),
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

// ---------- the layout plan ----------

/// Where an edge lines up, as the window offers it: one end, the middle,
/// the other end, or somewhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Start,
    Middle,
    End,
}

/// The three names for where a computer's edge can line up with ours.
pub fn snap_names(side: Side) -> [&'static str; 3] {
    if side.along_x() { ["Left", "Middle", "Right"] } else { ["Top", "Middle", "Bottom"] }
}

/// How long our edge and the neighbour's facing edge are, in pixels.
pub fn edge_lengths(local: Rect, remote: Rect, side: Side) -> (i32, i32) {
    if side.along_x() { (local.w, remote.w) } else { (local.h, remote.h) }
}

/// The offset, in pixels, of the neighbour's edge from the start of ours
/// for an alignment (a stretched one is drawn centred).
pub fn offset_px(align: Align, local_len: i32, remote_len: i32) -> i32 {
    match align {
        Align::Offset(o) => o,
        Align::Center | Align::Stretch => (local_len - remote_len) / 2,
    }
}

/// The alignment for an offset of a snap point.
pub fn snap_align(snap: Snap, local_len: i32, remote_len: i32) -> Align {
    match snap {
        Snap::Start => Align::Offset(0),
        Snap::Middle => Align::Center,
        Snap::End => Align::Offset(local_len - remote_len),
    }
}

/// Which snap point an alignment is, if any.
pub fn snap_of(align: Align, local_len: i32, remote_len: i32) -> Option<Snap> {
    match align {
        Align::Stretch => None,
        Align::Center => Some(Snap::Middle),
        Align::Offset(0) => Some(Snap::Start),
        Align::Offset(o) if o == local_len - remote_len => Some(Snap::End),
        Align::Offset(o) if o == (local_len - remote_len) / 2 => Some(Snap::Middle),
        Align::Offset(_) => None,
    }
}

/// A drag along the edge let go at `raw` pixels: the alignment it means,
/// pulled to the start, the middle or the end when within `pull` pixels.
/// Offsets keep the edges touching by at least one pixel.
pub fn snap_offset(raw: f64, local_len: i32, remote_len: i32, pull: f64) -> Align {
    for snap in [Snap::Start, Snap::Middle, Snap::End] {
        let at = offset_px(snap_align(snap, local_len, remote_len), local_len, remote_len);
        if (raw - f64::from(at)).abs() <= pull {
            return snap_align(snap, local_len, remote_len);
        }
    }
    let lo = 1 - remote_len;
    let hi = local_len - 1;
    Align::Offset((raw.round() as i64).clamp(i64::from(lo.min(hi)), i64::from(hi.max(lo))) as i32)
}

/// "On your left, its top edge 120 px below yours."
pub fn placement_sentence(side: Side, align: Align) -> String {
    let text = placement(side, align);
    let mut c = text.chars();
    let text = c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect());
    format!("{text}.")
}

/// A computer drawn on the plan: its monitors in our coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub screens: Vec<Rect>,
    /// Its monitors aren't known (offline, or an older version): a
    /// stand-in size is drawn.
    pub guessed: bool,
}

/// A monitor size to draw when the real one isn't known.
pub const GUESS: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

/// Where a neighbour's monitors go on the plan, from its side and
/// alignment, in our desktop's coordinates.
pub fn place(local: &[Rect], remote: &[Rect], side: Side, align: Align) -> Placed {
    let guessed = remote.is_empty();
    let remote = if guessed { vec![GUESS] } else { remote.to_vec() };
    let local = if local.is_empty() { vec![GUESS] } else { local.to_vec() };
    let (l, r) = (Desktop::new(local).bounds(), Desktop::new(remote.clone()).bounds());
    let (ll, rl) = edge_lengths(l, r, side);
    let along = offset_px(align, ll, rl);
    let (x, y) = match side {
        Side::Left => (l.x - r.w, l.y + along),
        Side::Right => (l.right(), l.y + along),
        Side::Top => (l.x + along, l.y - r.h),
        Side::Bottom => (l.x + along, l.bottom()),
    };
    let screens = remote.iter().map(|s| Rect::new(s.x - r.x + x, s.y - r.y + y, s.w, s.h)).collect();
    Placed { screens, guessed }
}

/// `[start, end)` ranges along an edge.
pub type Spans = Vec<(f64, f64)>;

/// The parts of our `side` edge that lead to the neighbour (`shared`), and
/// the parts that are walls, as ranges along the edge.
pub fn seam_parts(local: &[Rect], remote: &[Rect], side: Side, align: Align) -> (Spans, Spans) {
    let local = if local.is_empty() { vec![GUESS] } else { local.to_vec() };
    let remote = if remote.is_empty() { vec![GUESS] } else { remote.to_vec() };
    let desk = Desktop::new(local);
    let spans = desk.edge_spans(side);
    let shared = Seam::new(desk, Desktop::new(remote), side, align).shared();
    let mut walls = vec![];
    for (a, b) in spans {
        let mut at = f64::from(a);
        for &(s, e) in shared.iter().filter(|(s, e)| *e > f64::from(a) && *s < f64::from(b)) {
            if s > at {
                walls.push((at, s));
            }
            at = at.max(e);
        }
        if at < f64::from(b) {
            walls.push((at, f64::from(b)));
        }
    }
    (shared, walls)
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
    use super::*;

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
        assert_eq!(p.hotkeys[0].1, "ctrl+alt+{n}", "unset shortcuts are the defaults");
        assert_eq!(p.hooks, vec![("Pointer arrives", "notify-send 'hi there'".to_owned()), ("Pointer leaves", "echo # not a comment".to_owned())]);
        assert_eq!(p.hooks_timeout, 9);
        assert_eq!(p.auto.get("home").map(String::as_str), Some("Wi-Fi “Home Wi-Fi” or 192.168.1.0/24"));
        assert!(!p.auto.contains_key("Café, 2nd floor"));
        // Nothing written yet: one-mk's defaults.
        let d = Prefs::default();
        assert!(d.notify_files && d.clip_html && d.hotkeys_on && d.clip_allow_fetch);
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
    fn chords_read_like_keys() {
        assert_eq!(chord_label("ctrl+alt+{n}", false), "Ctrl+Alt+1…9");
        assert_eq!(chord_label("ctrl+alt+pagedown", false), "Ctrl+Alt+PageDown");
        assert_eq!(chord_label("ctrl+alt+p", true), "Control+Option+P");
        assert_eq!(chord_label("none", false), "Off");
    }

    #[test]
    fn doctor_report_is_read_worst_first() {
        let json = r#"{"ok":false,"summary":{"pass":1,"skip":0,"warn":1,"fail":1},"checks":[
            {"id":"config","title":"Settings","status":"pass","detail":"Found at /tmp/a \"b\".","fix":null},
            {"id":"input","title":"Permissions","status":"warn","detail":"Not allowed yet.","fix":"Run `one-mk run` and choose Allow."},
            {"id":"daemon","title":"Program","status":"fail","detail":"one-mk is not running é😀.","fix":"Start it."}]}"#;
        let checks = parse_doctor(json).unwrap();
        assert_eq!(checks.iter().map(|c| c.verdict).collect::<Vec<_>>(), [Verdict::Fail, Verdict::Warn, Verdict::Pass]);
        assert_eq!(checks[0].detail, "one-mk is not running é😀.");
        assert_eq!(checks[2].detail, "Found at /tmp/a \"b\".");
        assert_eq!(checks[2].fix, None);
        assert_eq!(doctor_summary(&checks), "1 problem, 1 warning");
        assert_eq!(doctor_summary(&checks[2..]), "Everything checks out");
        assert_eq!(fix_command(checks[1].fix.as_ref().unwrap()), ("one-mk run".to_owned(), true));
        assert_eq!(fix_command("Start it."), ("Start it.".to_owned(), false));
        // A flag or a placeholder isn't something to paste; a later command is.
        let fix = "Was it started with `--no-discovery`? Connect with `one-mk run --connect <ip>:24900`.";
        assert!(!fix_command(fix).1);
        assert_eq!(fix_command("Use `--port`, or run `one-mk install`.").0, "one-mk install");
        assert_eq!(hotkeys_note("unavailable", "no portal").as_deref(), Some("On this computer they work only while the pointer is on another one (no portal)."));
        assert_eq!(hotkeys_note("on", "through Windows").as_deref(), Some("Working through Windows."));
        assert_eq!(hotkeys_note("off", ""), None);
        assert_eq!(parse_doctor("one-mk: error"), None);
        assert_eq!(parse_doctor("{\"checks\": [1, }"), None);
    }

    #[test]
    fn offsets_snap_to_the_ends_and_the_middle() {
        // Our edge 1080 long, theirs 800: the end is at 280, the middle at 140.
        assert_eq!(snap_offset(6.0, 1080, 800, 12.0), Align::Offset(0));
        assert_eq!(snap_offset(133.0, 1080, 800, 12.0), Align::Center);
        assert_eq!(snap_offset(290.0, 1080, 800, 12.0), Align::Offset(280));
        assert_eq!(snap_offset(200.4, 1080, 800, 12.0), Align::Offset(200));
        // Never so far that the edges stop touching.
        assert_eq!(snap_offset(5000.0, 1080, 800, 12.0), Align::Offset(1079));
        assert_eq!(snap_offset(-5000.0, 1080, 800, 12.0), Align::Offset(-799));
        assert_eq!(snap_of(Align::Offset(280), 1080, 800), Some(Snap::End));
        assert_eq!(snap_of(Align::Offset(140), 1080, 800), Some(Snap::Middle));
        assert_eq!(snap_of(Align::Offset(7), 1080, 800), None);
        assert_eq!(snap_of(Align::Stretch, 1080, 800), None);
        assert_eq!(snap_names(Side::Left), ["Top", "Middle", "Bottom"]);
        assert_eq!(snap_names(Side::Bottom), ["Left", "Middle", "Right"]);
    }

    #[test]
    fn placement_reads_as_a_sentence() {
        assert_eq!(placement_sentence(Side::Left, Align::Offset(120)), "On your left, its top edge 120 px below yours.");
        assert_eq!(placement_sentence(Side::Top, Align::Center), "Above, centred on yours.");
        assert_eq!(placement_sentence(Side::Right, Align::Stretch), "On your right.");
    }

    #[test]
    fn neighbours_are_placed_against_our_edge() {
        let me = [Rect::new(0, 0, 2560, 1440)];
        let laptop = [Rect::new(0, 0, 1280, 800)];
        let p = place(&me, &laptop, Side::Bottom, Align::Offset(300));
        assert_eq!(p.screens, vec![Rect::new(300, 1440, 1280, 800)]);
        let p = place(&me, &laptop, Side::Left, Align::Center);
        assert_eq!(p.screens, vec![Rect::new(-1280, 320, 1280, 800)]);
        // Two monitors keep their arrangement.
        let dual = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 200, 1280, 1024)];
        let p = place(&me, &dual, Side::Right, Align::Offset(0));
        assert_eq!(p.screens, vec![Rect::new(2560, 0, 1920, 1080), Rect::new(4480, 200, 1280, 1024)]);
        assert!(!p.guessed);
        assert!(place(&me, &[], Side::Top, Align::Stretch).guessed);
    }

    #[test]
    fn the_shared_edge_and_the_walls_split_our_edge() {
        let me = [Rect::new(0, 0, 1920, 1080)];
        let laptop = [Rect::new(0, 0, 1280, 800)];
        let (shared, walls) = seam_parts(&me, &laptop, Side::Bottom, Align::Offset(300));
        assert_eq!(shared, vec![(300.0, 1580.0)]);
        assert_eq!(walls, vec![(0.0, 300.0), (1580.0, 1920.0)]);
        // Stretched, all of the edge leads there.
        let (shared, walls) = seam_parts(&me, &laptop, Side::Bottom, Align::Stretch);
        assert_eq!((shared, walls), (vec![(0.0, 1920.0)], vec![]));
        // Past the end: nothing shared, all wall.
        let (shared, walls) = seam_parts(&me, &laptop, Side::Right, Align::Offset(2000));
        assert!(shared.is_empty());
        assert_eq!(walls, vec![(0.0, 1080.0)]);
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
}
