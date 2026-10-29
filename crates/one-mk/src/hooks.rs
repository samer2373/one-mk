//! Commands to run when the pointer arrives on or leaves this computer, and
//! when a peer connects or disconnects (`[hooks]` in `settings.toml`, set
//! with `one-mk hooks`).
//!
//! ```toml
//! [hooks]
//! on_enter = ["/usr/bin/notify-send", "one-mk", "the mouse is here"]
//! on_leave = "ddcutil setvcp 60 0x0f"        # a string is split like a shell would, minus expansion
//! timeout_secs = 5
//! ```
//!
//! *Enter* means the pointer is now on this computer: a peer took control
//! of it, or our own pointer came back from a peer. *Leave* is the reverse.
//! Each command gets the environment variables `ONE_MK_EVENT` (`enter`,
//! `leave`, `peer_connect`, `peer_disconnect`), `ONE_MK_PEER` (the other
//! computer's name), `ONE_MK_PEER_ID` and `ONE_MK_SIDE` (the side of this
//! computer's screen the other one is on).
//!
//! A hook runs without a shell unless `shell = true`: an array is the
//! program and its arguments; a string is split into words (single and
//! double quotes and backslashes, nothing is expanded). It is started and
//! reaped by its own task, so the daemon never waits for it, killed after
//! `timeout_secs` (5 by default), run at most once per 500 ms per event
//! (per peer for connection events), with independent serial pointer/peer lanes. Anything that
//! can write `settings.toml` can run programs as you, so a file others can
//! write is reported (see [`unsafe_permissions`]).

use anyhow::{Result, bail};
use one_mk_net::Fingerprint;
use one_mk_proto::Side;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::Instant;

/// One hook runs at most this often; faster triggers are coalesced.
pub const MIN_INTERVAL: Duration = Duration::from_millis(500);
/// Key actions (`run:`) running at once; more are dropped.
pub const MAX_RUNNING: usize = 8;
/// Default and largest time a hook may run before it is killed.
pub const DEFAULT_TIMEOUT_SECS: u64 = 5;
pub const MAX_TIMEOUT_SECS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    Enter,
    Leave,
    PeerConnect,
    PeerDisconnect,
}

impl Event {
    pub const ALL: [Event; 4] = [Event::Enter, Event::Leave, Event::PeerConnect, Event::PeerDisconnect];

    /// The value of `ONE_MK_EVENT`.
    pub fn as_str(self) -> &'static str {
        match self {
            Event::Enter => "enter",
            Event::Leave => "leave",
            Event::PeerConnect => "peer_connect",
            Event::PeerDisconnect => "peer_disconnect",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A command as written in `settings.toml`: an argv array, or a string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Command {
    Argv(Vec<String>),
    Line(String),
}

impl Command {
    /// The program and its arguments. A string is split into words (or, with
    /// `shell`, handed to the system shell as one script).
    pub fn argv(&self, shell: bool) -> Result<Vec<String>> {
        let bytes = match self {
            Command::Line(line) => line.len(),
            Command::Argv(words) => words.iter().fold(0usize, |size, word| size.saturating_add(word.len()).saturating_add(1)),
        };
        if bytes > 4096 {
            bail!("a hook command must be at most 4096 bytes");
        }
        let argv = match self {
            Command::Argv(v) => v.clone(),
            Command::Line(s) if shell => shell_argv(s),
            Command::Line(s) => split(s)?,
        };
        if argv.first().is_none_or(|p| p.is_empty()) {
            bail!("the command is empty");
        }
        Ok(argv)
    }

    /// How to show it.
    #[cfg(test)]
    pub fn describe(&self) -> String {
        match self {
            Command::Argv(v) => v.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "),
            Command::Line(s) => s.clone(),
        }
    }
}

#[cfg(windows)]
fn shell_argv(script: &str) -> Vec<String> {
    vec!["cmd".into(), "/C".into(), script.into()]
}

#[cfg(not(windows))]
fn shell_argv(script: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

/// Words for display and for `one-mk hooks` to store: quotes only where
/// [`split`] needs them to read the same word back.
fn quote(word: &str) -> String {
    if !word.is_empty() && !word.contains(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '\\')) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

fn legacy_shell_word(word: &str) -> String {
    #[cfg(windows)]
    {
        if word.is_empty() || word.contains(|ch: char| ch.is_whitespace() || ch == '"') {
            format!("\"{}\"", word.replace('"', "\"\""))
        } else {
            word.to_owned()
        }
    }
    #[cfg(not(windows))]
    {
        if !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte)) {
            word.to_owned()
        } else {
            format!("'{}'", word.replace('\'', r"'\''"))
        }
    }
}

/// Splits a command line into words like a POSIX shell would, without
/// expanding anything (no `$VAR`, `~`, globs, pipes or redirections: those
/// characters are ordinary). Words are separated by whitespace; `'...'`
/// keeps everything literally; inside `"..."` a backslash escapes `"` and
/// `\` and is otherwise literal; outside quotes a backslash escapes the next
/// character. (For Windows paths, quote them with single quotes or use an
/// array.)
pub fn split(line: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => bail!("a ' is not closed"),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => word.push(e),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => bail!("a \" is not closed"),
                        },
                        Some(c) => word.push(c),
                        None => bail!("a \" is not closed"),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(e) => word.push(e),
                    None => bail!("the command ends with a \\"),
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// `[hooks]` in `settings.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hooks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_enter: Option<Command>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_leave: Option<Command>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_peer_connect: Option<Command>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_peer_disconnect: Option<Command>,
    /// Seconds a hook may run (default 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Run a string through the system shell (`sh -c` / `cmd /C`). Off by
    /// default: nothing is then expanded or interpreted.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shell: bool,
    #[serde(skip)]
    pub trust_permissions: bool,
    #[serde(skip)]
    pub blocked: Option<String>,
}

impl Hooks {
    pub fn get(&self, ev: Event) -> Option<&Command> {
        match ev {
            Event::Enter => self.on_enter.as_ref(),
            Event::Leave => self.on_leave.as_ref(),
            Event::PeerConnect => self.on_peer_connect.as_ref(),
            Event::PeerDisconnect => self.on_peer_disconnect.as_ref(),
        }
    }

    pub fn slot(&mut self, ev: Event) -> &mut Option<Command> {
        match ev {
            Event::Enter => &mut self.on_enter,
            Event::Leave => &mut self.on_leave,
            Event::PeerConnect => &mut self.on_peer_connect,
            Event::PeerDisconnect => &mut self.on_peer_disconnect,
        }
    }

    pub fn any(&self) -> bool {
        self.blocked.is_none() && Event::ALL.iter().any(|&e| self.get(e).is_some())
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).clamp(1, MAX_TIMEOUT_SECS))
    }

    /// Every problem with the commands (an unparsable string, an empty one).
    pub fn problems(&self) -> Vec<String> {
        Event::ALL
            .iter()
            .filter_map(|&e| {
                let cmd = self.get(e)?;
                cmd.argv(self.shell).err().map(|err| format!("hooks.on_{}: {err}", e.as_str()))
            })
            .collect()
    }

    pub fn describe(&self) -> String {
        if let Some(reason) = &self.blocked {
            return format!("Hooks REFUSED: {reason}. Fix permissions or explicitly use `one-mk hooks --trust-permissions`.");
        }
        let mut out = String::new();
        for e in Event::ALL {
            if let Some(c) = self.get(e) {
                let actual = c.argv(self.shell).map(|words| words.iter().map(|word| quote(word)).collect::<Vec<_>>().join(" "));
                out += &format!("  on_{}: {}\n", e.as_str(), actual.unwrap_or_else(|error| format!("INVALID: {error}")));
            }
        }
        if out.is_empty() {
            return "No hooks are set.".into();
        }
        let shell = if self.shell { ", run through the shell" } else { "" };
        format!("Hooks (timeout {} s{shell}):\n{}", self.timeout().as_secs(), out.trim_end())
    }
}

/// The environment a hook gets, besides the one it inherits.
pub fn env(ev: Event, peer: &PeerInfo) -> [(&'static str, String); 4] {
    [
        ("ONE_MK_EVENT", ev.as_str().to_owned()),
        ("ONE_MK_PEER", peer.name.clone()),
        ("ONE_MK_PEER_ID", peer.id.clone()),
        ("ONE_MK_SIDE", peer.side.to_string()),
    ]
}

fn shell_value(value: &str) -> String {
    value.chars().map(|ch| if ch.is_alphabetic() || ch.is_ascii_digit() || matches!(ch, ' ' | '.' | '_' | '-') { ch } else { '_' }).collect()
}

/// Whom a hook is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    pub name: String,
    pub id: String,
    /// Which side of this computer's screen the peer is on.
    pub side: Side,
}

/// No more than one run per event per [`MIN_INTERVAL`].
#[derive(Debug, Default)]
pub struct RateLimit {
    last: [Option<Instant>; 4],
}

impl RateLimit {
    /// Whether `ev` may run now; if so, counts it.
    pub fn allow(&mut self, ev: Event, now: Instant) -> bool {
        let slot = &mut self.last[ev.index()];
        if slot.is_some_and(|t| now.saturating_duration_since(t) < MIN_INTERVAL) {
            return false;
        }
        *slot = Some(now);
        true
    }
}

/// Why `settings.toml` should not be trusted to hold commands: it can be
/// written by others (Unix mode bits), so whoever can write it runs
/// programs as this user. `None` when it is fine, missing, or the OS has no
/// such bits.
pub fn unsafe_permissions(path: &Path) -> Option<String> {
    let resolved = std::fs::canonicalize(path).ok()?;
    let file = std::fs::File::open(&resolved).ok()?;
    opened_permissions(path, &resolved, &file)
}

pub fn read_settings(path: &Path) -> std::io::Result<(String, Option<String>)> {
    use std::io::Read;
    let resolved = std::fs::canonicalize(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(&resolved)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("settings must be a regular file"));
    }
    let reason = opened_permissions(path, &resolved, &file);
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok((text, reason))
}

fn opened_permissions(path: &Path, resolved: &Path, file: &std::fs::File) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let md = match file.metadata() { Ok(md) => md, Err(error) => return Some(error.to_string()) };
        let uid = unsafe { libc::geteuid() };
        let config = path.parent()?.to_owned();
        let canonical_config = match std::fs::canonicalize(&config) { Ok(dir) => dir, Err(error) => return Some(error.to_string()) };
        let target_parent = resolved.parent()?;
        let mut checks = vec![(resolved.to_owned(), md)];
        for start in [config.as_path(), canonical_config.as_path(), target_parent] {
            for target in start.ancestors() {
                let metadata = match std::fs::metadata(target) { Ok(md) => md, Err(error) => return Some(error.to_string()) };
                checks.push((target.to_owned(), metadata));
            }
        }
        for (target, metadata) in checks {
            if metadata.uid() != uid && !(metadata.is_dir() && metadata.uid() == 0) {
                return Some(format!("{} is owned by uid {}, not current uid {uid}", target.display(), metadata.uid()));
            }
            if !(metadata.is_dir() && metadata.mode() & 0o1000 != 0)
                && let Some(msg) = writable_by_others(metadata.mode()) {
                return Some(format!("{} is {msg}", target.display()));
            }
        }
        None
    }
    #[cfg(not(unix))]
    {
        let _ = (path, resolved, file);
        None
    }
}

const TRUST_FILE: &str = "hooks-trust-permissions";

pub fn permissions_trusted(dir: &Path) -> bool {
    use std::io::Read;
    let path = dir.join(TRUST_FILE);
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let Ok(file) = options.open(path) else { return false };
    let Ok(metadata) = file.metadata() else { return false };
    if !metadata.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 { return false; }
    }
    let mut text = String::new();
    file.take(64).read_to_string(&mut text).is_ok() && text == "explicitly trust hook permissions\n"
}

fn trust_permissions(dir: &Path) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    if permissions_trusted(dir) { return Ok(()); }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(dir.join(TRUST_FILE))?.write_all(b"explicitly trust hook permissions\n")?;
    Ok(())
}

/// What is wrong with a file of mode `mode`, if it is group or world writable.
#[cfg(any(unix, test))]
pub fn writable_by_others(mode: u32) -> Option<&'static str> {
    match mode & 0o022 {
        0 => None,
        m if m & 0o002 != 0 => Some("writable by everyone"),
        _ => Some("writable by its group"),
    }
}

/// Reports each distinct hook configuration or permission problem once.
pub fn warn_if_unsafe(dir: &Path, hooks: &Hooks) {
    if !Event::ALL.iter().any(|&event| hooks.get(event).is_some()) {
        return;
    }
    static WARNED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    let mut problems = hooks.problems();
    if let Some(msg) = unsafe_permissions(&dir.join("settings.toml")) {
        problems.push(format!("{msg}: hooks {}", if hooks.trust_permissions { "explicitly trusted despite unsafe permissions" } else { "REFUSED; fix permissions or use `one-mk hooks --trust-permissions`" }));
    }
    let mut warned = WARNED.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for problem in problems {
        if warned.insert(problem.clone()) {
            tracing::warn!("{problem}");
        }
    }
}

struct Pending {
    command: Option<Command>,
    generation: u64,
    event: Event,
    argv: Vec<String>,
    env: [(&'static str, String); 4],
    shell: bool,
    timeout: Duration,
    due: Instant,
    sequence: u64,
}

#[derive(Default)]
struct Queue {
    configuration: Option<Hooks>,
    generation: u64,
    limit: RateLimit,
    pending: HashMap<(Event, String), Pending>,
    peer_last: HashMap<(Event, String), Instant>,
    sequence: u64,
    active: bool,
}

impl Queue {
    fn cancel_opposite(&mut self, event: Event, peer: &str) {
        let opposite = match event {
            Event::Enter => Some(Event::Leave),
            Event::Leave => Some(Event::Enter),
            Event::PeerConnect => Some(Event::PeerDisconnect),
            Event::PeerDisconnect => Some(Event::PeerConnect),
        };
        if let Some(opposite) = opposite {
            self.pending.retain(|(event, id), _| *event != opposite || (!id.is_empty() && id != peer));
        }
    }

    fn insert(&mut self, mut pending: Pending, now: Instant) {
        pending.generation = self.generation;
        let event = pending.event;
        self.cancel_opposite(event, &pending.env[2].1);
        self.sequence = self.sequence.wrapping_add(1);
        pending.sequence = self.sequence;
        let key = (event, if matches!(event, Event::PeerConnect | Event::PeerDisconnect) { pending.env[2].1.clone() } else { String::new() });
        self.peer_last.retain(|_, last| now.saturating_duration_since(*last) < MIN_INTERVAL);
        let last = if key.1.is_empty() { self.limit.last[event.index()] } else { self.peer_last.get(&key).copied() };
        pending.due = last.map_or(now, |last| (last + MIN_INTERVAL).max(now));
        if !key.1.is_empty() {
            for queued in self.pending.values().filter(|queued| queued.env[2].1 == key.1) {
                pending.due = pending.due.max(queued.due);
            }
        }
        if self.pending.len() < 256 || self.pending.contains_key(&key) { self.pending.insert(key, pending); }
    }

    fn next(&self) -> Option<(Event, String)> {
        self.pending.iter().min_by_key(|(_, pending)| (pending.due, pending.sequence)).map(|(key, _)| key.clone())
    }

    fn configure(&mut self, cfg: &Hooks) {
        if self.configuration.as_ref() == Some(cfg) { return; }
        self.generation = self.generation.wrapping_add(1);
        self.configuration = Some(cfg.clone());
        self.pending.retain(|_, pending| {
            pending.generation = self.generation;
            if pending.command.as_ref() != cfg.get(pending.event) { return false; }
            cfg.get(pending.event).is_some_and(|command| command.argv(cfg.shell).is_ok_and(|argv| argv == pending.argv)
                && (cfg.shell && matches!(command, Command::Line(_))) == pending.shell && cfg.timeout() == pending.timeout)
        });
    }

    fn take_ready(&mut self, now: Instant) -> Option<Pending> {
        self.pending.retain(|_, pending| pending.generation == self.generation);
        let index = self.next()?;
        if self.pending.get(&index)?.due > now {
            return None;
        }
        let pending = self.pending.remove(&index)?;
        if index.1.is_empty() { self.limit.allow(pending.event, now); }
        else { self.peer_last.insert(index, now); }
        Some(pending)
    }
}

/// A connected peer as the engine last saw it.
struct Known {
    info: PeerInfo,
    /// The `sync` that last saw it connected.
    seen: u64,
}

/// Notices changes in what the daemon is doing and runs the hooks for them.
/// It compares snapshots, so nothing in the daemon has to know about it,
/// and it does nothing at all while no hook is set.
#[derive(Default)]
pub struct Engine {
    /// Whether the fields below describe the daemon, or a hook was just set.
    primed: bool,
    pass: u64,
    /// The peer our pointer is on (once it accepted), and the one controlling us.
    away: Option<Fingerprint>,
    owner: Option<Fingerprint>,
    known: HashMap<Fingerprint, Known>,
    queue: Arc<std::sync::Mutex<Queue>>,
    peer_queue: Arc<std::sync::Mutex<Queue>>,
    wake: Arc<tokio::sync::Notify>,
    peer_wake: Arc<tokio::sync::Notify>,
    workers: Vec<tokio::task::JoinHandle<()>>,
    running: Arc<AtomicUsize>,
    actions_running: Arc<AtomicUsize>,
    /// When each key action's command last ran (see [`Engine::run_action`]).
    actions: HashMap<String, Instant>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        for worker in &self.workers {
            worker.abort();
        }
    }
}

impl Engine {
    /// Runs the hooks for whatever changed since the last call. `peers` are
    /// all connected peers: name and which side of this computer each is on.
    pub fn sync<'a>(
        &mut self,
        cfg: &Hooks,
        away: Option<Fingerprint>,
        owner: Option<Fingerprint>,
        peers: impl Iterator<Item = (Fingerprint, &'a str, Side)>,
    ) {
        if !cfg.any() {
            if self.primed {
                // What runs (and key actions' limits) outlive the hooks.
                let (running, actions_running, actions) = (self.running.clone(), self.actions_running.clone(), std::mem::take(&mut self.actions));
                let mut fresh = Engine::default();
                (fresh.running, fresh.actions_running, fresh.actions) = (running, actions_running, actions);
                *self = fresh;
            }
            return;
        }
        for queue in [&self.queue, &self.peer_queue] {
            queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).configure(cfg);
        }
        for (ev, info) in self.diff(away, owner, peers) {
            self.fire(cfg, ev, &info);
        }
    }

    /// The events since the last call. The first call after hooks were set
    /// only takes note: nothing has changed yet.
    fn diff<'a>(
        &mut self,
        away: Option<Fingerprint>,
        owner: Option<Fingerprint>,
        peers: impl Iterator<Item = (Fingerprint, &'a str, Side)>,
    ) -> Vec<(Event, PeerInfo)> {
        let quiet = !std::mem::replace(&mut self.primed, true);
        let mut out = Vec::new();
        self.pass += 1;
        for (fp, name, side) in peers {
            match self.known.get_mut(&fp) {
                Some(k) => {
                    k.seen = self.pass;
                    k.info.side = side;
                    if k.info.name != name {
                        k.info.name = name.to_owned();
                    }
                }
                None => {
                    let info = PeerInfo { name: name.to_owned(), id: fp.short(), side };
                    if !quiet {
                        out.push((Event::PeerConnect, info.clone()));
                    }
                    self.known.insert(fp, Known { info, seen: self.pass });
                }
            }
        }
        // The pointer going away or coming back, and a peer taking control
        // or letting go, are looked up before a peer that just
        // disconnected is forgotten. Moving from one peer to another is not
        // coming back.
        let info = |known: &HashMap<Fingerprint, Known>, fp: Fingerprint| known.get(&fp).map(|k| k.info.clone());
        if !quiet {
            match (self.away, away) {
                (None, Some(fp)) => out.extend(info(&self.known, fp).map(|i| (Event::Leave, i))),
                (Some(fp), None) => out.extend(info(&self.known, fp).map(|i| (Event::Enter, i))),
                _ => {}
            }
            match (self.owner, owner) {
                (None, Some(fp)) => out.extend(info(&self.known, fp).map(|i| (Event::Enter, i))),
                (Some(fp), None) => out.extend(info(&self.known, fp).map(|i| (Event::Leave, i))),
                _ => {}
            }
        }
        self.away = away;
        self.owner = owner;
        let pass = self.pass;
        if self.known.values().any(|k| k.seen != pass) {
            self.known.retain(|_, k| {
                let here = k.seen == pass;
                if !here && !quiet {
                    out.push((Event::PeerDisconnect, k.info.clone()));
                }
                here
            });
        }
        out
    }

    fn fire(&mut self, cfg: &Hooks, ev: Event, peer: &PeerInfo) {
        let (lane, wake) = if matches!(ev, Event::Enter | Event::Leave) { (&self.queue, &self.wake) } else { (&self.peer_queue, &self.peer_wake) };
        lane.lock().unwrap_or_else(std::sync::PoisonError::into_inner).cancel_opposite(ev, &peer.id);
        wake.notify_one();
        let Some(cmd) = cfg.get(ev) else { return };
        let argv = match cmd.argv(cfg.shell) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(event = ev.as_str(), "hook not run: {e}");
                return;
            }
        };
        let mut environment = env(ev, peer);
        if cfg.shell && matches!(cmd, Command::Line(_)) {
            for (_, value) in &mut environment[1..] {
                *value = shell_value(value);
            }
        }
        let now = Instant::now();
        let mut queue = lane.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.insert(Pending { command: Some(cmd.clone()), generation: 0, event: ev, argv, env: environment, shell: cfg.shell && matches!(cmd, Command::Line(_)), timeout: cfg.timeout(), due: now, sequence: 0 }, now);
        wake.notify_one();
        if queue.active {
            return;
        }
        queue.active = true;
        drop(queue);
        let (queue, wake, running) = (lane.clone(), wake.clone(), self.running.clone());
        self.workers.retain(|worker| !worker.is_finished());
        self.workers.push(tokio::spawn(async move {
            loop {
                let (ready, due) = {
                    let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let ready = queue.take_ready(Instant::now());
                    let due = queue.next().and_then(|index| queue.pending.get(&index).map(|pending| pending.due));
                    if ready.is_none() && due.is_none() {
                        queue.active = false;
                        return;
                    }
                    (ready, due)
                };
                if let Some(pending) = ready {
                    if running.fetch_add(1, Ordering::SeqCst) >= MAX_RUNNING {
                        running.fetch_sub(1, Ordering::SeqCst);
                        tracing::warn!("hook dropped: global running cap reached");
                        continue;
                    }
                    let _running = Running(running.clone());
                    run(pending.event.as_str(), &pending.argv, &pending.env, pending.timeout, pending.shell).await;
                } else if let Some(due) = due {
                    tokio::select! {
                        _ = tokio::time::sleep_until(due) => {},
                        _ = wake.notified() => {},
                    }
                }
            }
        }));
    }

    /// Runs `cmd` for a key mapped to `run: ...` (see `tuning.rs`), the way
    /// hooks run: split into words without a shell (whatever `shell` says:
    /// a key repeats too easily for that), killed after `timeout_secs`, at
    /// most once per [`MIN_INTERVAL`] per command and [`MAX_RUNNING`] at a
    /// time independently of hooks, and refused like hooks are when `settings.toml`
    /// is writable by others. `peer` is the computer the keys go to (name,
    /// short id), for `ONE_MK_PEER` and `ONE_MK_PEER_ID`.
    pub fn run_action(&mut self, cfg: &Hooks, cmd: &str, peer: Option<(String, String)>) {
        if let Some(reason) = &cfg.blocked {
            tracing::warn!("key action `run: {cmd}` refused: {reason}");
            return;
        }
        let now = Instant::now();
        if self.actions.get(cmd).is_some_and(|t| now.saturating_duration_since(*t) < MIN_INTERVAL) {
            tracing::debug!("key action dropped: it ran less than {MIN_INTERVAL:?} ago");
            return;
        }
        if self.actions.len() >= 64 {
            self.actions.clear();
        }
        self.actions.insert(cmd.to_owned(), now);
        let argv = match Command::Line(cmd.to_owned()).argv(false) {
            Ok(a) => a,
            Err(e) => return tracing::warn!("key action `run: {cmd}` not run: {e}"),
        };
        let (name, id) = peer.unwrap_or_default();
        let env = vec![("ONE_MK_EVENT", "key".to_owned()), ("ONE_MK_PEER", name), ("ONE_MK_PEER_ID", id)];
        self.spawn_now("key", argv, env, cfg.timeout());
    }

    /// Starts `argv` on its own task right away (key actions are not queued
    /// with the enter/leave hooks), unless [`MAX_RUNNING`] already run.
    fn spawn_now(&self, label: &'static str, argv: Vec<String>, env: Vec<(&'static str, String)>, timeout: Duration) {
        if self.actions_running.fetch_add(1, Ordering::SeqCst) >= MAX_RUNNING {
            self.actions_running.fetch_sub(1, Ordering::SeqCst);
            tracing::warn!(event = label, "hook dropped: {MAX_RUNNING} are still running");
            return;
        }
        let running = Running(self.actions_running.clone());
        tokio::spawn(async move {
            let _running = running;
            run(label, &argv, &env, timeout, false).await;
        });
    }
}

/// Counts a running hook for as long as it lives.
struct Running(Arc<AtomicUsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct ProcessGroup(Option<u32>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.and_then(|pid| i32::try_from(pid).ok()) {
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }
}

/// Runs one hook to its end or the timeout, and reaps it.
async fn run(label: &'static str, argv: &[String], env: &[(&'static str, String)], timeout: Duration, shell: bool) {
    #[cfg(windows)]
    let sanitized;
    #[cfg(windows)]
    let env = if Path::new(&argv[0]).extension().and_then(|extension| extension.to_str()).is_some_and(|extension| ["bat", "cmd", "com"].iter().any(|candidate| extension.eq_ignore_ascii_case(candidate))) {
        sanitized = env.iter().map(|(key, value)| (*key, if *key == "ONE_MK_EVENT" { value.clone() } else { shell_value(value) })).collect::<Vec<_>>();
        sanitized.as_slice()
    } else { env };
    let mut cmd = tokio::process::Command::new(&argv[0]);
    #[cfg(windows)]
    if shell {
        use std::os::windows::process::CommandExt;
        cmd.arg("/C").as_std_mut().raw_arg(&argv[2]);
    } else {
        cmd.args(&argv[1..]);
    }
    #[cfg(not(windows))]
    {
        let _ = shell;
        cmd.args(&argv[1..]);
    }
    cmd.envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0); // its own group, so a timeout can take its children too
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(event = label, "hook {:?} did not start: {e}", argv[0]);
            return;
        }
    };
    let pid = child.id();
    let mut group = ProcessGroup(pid);
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) if status.success() => tracing::debug!(event = label, "hook finished"),
        Ok(Ok(status)) => tracing::warn!(event = label, "hook {:?} failed: {status}", argv[0]),
        Ok(Err(e)) => tracing::warn!(event = label, "hook {:?}: {e}", argv[0]),
        Err(_) => {
            tracing::warn!(event = label, "hook {:?} killed after {} s", argv[0], timeout.as_secs());
            #[cfg(unix)]
            if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
                // SAFETY: plain syscall; the group was created for this child
                // and nothing else uses its number while we hold the child.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
            let _ = pid;
            let _ = child.kill().await; // and reaps it
        }
    }
    group.0 = None;
}

// ------------------------------------------------------------ command line

/// `one-mk hooks`: shows the hooks, or changes them. An empty command
/// (`--on-enter ""`) removes that hook.
#[derive(clap::Args, Debug, Default)]
pub struct HooksArgs {
    /// Run when the pointer arrives on this computer.
    #[arg(long, value_name = "CMD")]
    pub on_enter: Option<String>,
    /// Run when the pointer leaves this computer.
    #[arg(long, value_name = "CMD")]
    pub on_leave: Option<String>,
    /// Run when a computer connects.
    #[arg(long, value_name = "CMD")]
    pub on_peer_connect: Option<String>,
    /// Run when a computer disconnects.
    #[arg(long, value_name = "CMD")]
    pub on_peer_disconnect: Option<String>,
    /// Seconds a command may run before it is killed (1-60, default 5).
    #[arg(long, value_name = "SECS")]
    pub timeout: Option<u64>,
    /// Run commands through the system shell (`on`), which expands and
    /// interprets them, or without (`off`, the default).
    #[arg(long, value_name = "on|off", value_parser = crate::settings::parse_on_off, action = clap::ArgAction::Set)]
    pub shell: Option<bool>,
    /// Explicitly allow hooks despite unsafe Unix ownership or permissions.
    #[arg(long, conflicts_with = "off")]
    pub trust_permissions: bool,
    /// Remove every hook.
    #[arg(long)]
    pub off: bool,
}

impl HooksArgs {
    fn changes_anything(&self) -> bool {
        self.off
            || self.trust_permissions
            || self.timeout.is_some()
            || self.shell.is_some()
            || [&self.on_enter, &self.on_leave, &self.on_peer_connect, &self.on_peer_disconnect].iter().any(|c| c.is_some())
    }

    /// Applies the options to `h`, retaining command text for runtime mode changes.
    pub fn apply(&self, h: &mut Hooks) -> Result<()> {
        if self.off {
            *h = Hooks::default();
        }
        if let Some(t) = self.timeout {
            if !(1..=MAX_TIMEOUT_SECS).contains(&t) {
                bail!("--timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds");
            }
            h.timeout_secs = (t != DEFAULT_TIMEOUT_SECS).then_some(t);
        }
        if let Some(on) = self.shell {
            #[cfg(windows)]
            if on {
                for event in Event::ALL {
                    if let Some(Command::Argv(words)) = h.get(event)
                        && words.iter().any(|word| word.contains(['^', '&', '|', '<', '>', '%', '!', '"', '\r', '\n', '(', ')']) || word.ends_with('\\')) {
                        bail!("cannot safely convert Windows argv to cmd script: {words:?}; keep --shell off or explicitly provide the exact shell script");
                    }
                }
            }
            h.shell = on;
            if on {
                for event in Event::ALL {
                    if let Some(Command::Argv(words)) = h.get(event) {
                        let line = words.iter().map(|word| legacy_shell_word(word)).collect::<Vec<_>>().join(" ");
                        *h.slot(event) = Some(Command::Line(line));
                    }
                }
            }
        }
        h.trust_permissions |= self.trust_permissions;
        h.blocked = None;
        let given = [
            (Event::Enter, &self.on_enter),
            (Event::Leave, &self.on_leave),
            (Event::PeerConnect, &self.on_peer_connect),
            (Event::PeerDisconnect, &self.on_peer_disconnect),
        ];
        for (ev, text) in given {
            let Some(text) = text else { continue };
            *h.slot(ev) = if text.trim().is_empty() {
                None
            } else {
                Some(Command::Line(text.clone()))
            };
        }
        let problems = h.problems();
        if !problems.is_empty() {
            bail!("{}", problems.join("; "));
        }
        Ok(())
    }
}

pub const SECURITY_NOTE: &str = "Hooks run with your rights, from settings.toml: keep that file writable by you only.";

pub async fn run_command(dir: &Path, args: HooksArgs) -> Result<()> {
    use crate::settings::Settings;
    if args.trust_permissions {
        trust_permissions(dir)?;
    }
    if args.off {
        match std::fs::remove_file(dir.join(TRUST_FILE)) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    let hooks = if args.changes_anything() {
        Settings::update(dir, |s| {
            args.apply(&mut s.hooks)?;
            Ok(s.hooks.clone())
        })?
    } else {
        Settings::load(dir)?.hooks
    };
    let hooks = Settings::load(dir).map(|settings| settings.hooks).unwrap_or(hooks);
    println!("{}", hooks.describe());
    if hooks.any() {
        println!("{SECURITY_NOTE}");
        if let Some(msg) = unsafe_permissions(&dir.join("settings.toml")) {
            println!("Warning: {msg}; running hooks because you explicitly trusted these permissions.");
        }
    }
    if args.changes_anything() {
        if crate::control::request(dir, crate::control::Request::Reload).await?.is_some() {
            println!("Applied right away.");
        } else {
            println!("one-mk is not running; this takes effect the next time it starts.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn symlink_target_directory_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let base = std::env::temp_dir().join(format!("one-mk-target-permissions-{}", std::process::id()));
        for (case, relative, writable) in [
            ("ancestor", "..", true), ("sibling", "../dotfiles", true),
            ("child", "dotfiles", true), ("safe", "../dotfiles", false),
        ] {
            let root = base.join(case);
            let config = root.join("one-mk");
            let parent = config.join(relative);
            std::fs::create_dir_all(&parent).unwrap();
            std::fs::create_dir_all(&config).unwrap();
            for directory in [&base, &root, &config, &parent] {
                std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let target = parent.join("target.toml");
            std::fs::write(&target, "").unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
            let path = config.join("settings.toml");
            let _ = std::fs::remove_file(&path);
            symlink(&target, &path).unwrap();
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(if writable { 0o2775 } else { 0o755 })).unwrap();
            assert_eq!(unsafe_permissions(&path).is_some(), writable, "{case}");
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn newest_peer_transition_cancels_the_opposite() {
        let mut queue = Queue::default();
        let peer = PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left };
        let pending = |event| Pending { command: None, generation: 0, event, argv: vec!["echo".into()], env: env(event, &peer), shell: false, timeout: Duration::from_secs(1), due: Instant::now(), sequence: 0 };
        queue.insert(pending(Event::PeerConnect), Instant::now());
        assert_eq!(queue.take_ready(Instant::now()).unwrap().event, Event::PeerConnect);
        tokio::time::advance(Duration::from_millis(100)).await;
        queue.insert(pending(Event::PeerDisconnect), Instant::now());
        tokio::time::advance(Duration::from_millis(100)).await;
        queue.insert(pending(Event::PeerConnect), Instant::now());
        tokio::time::advance(Duration::from_millis(100)).await;
        queue.insert(pending(Event::PeerDisconnect), Instant::now());
        tokio::time::advance(MIN_INTERVAL).await;
        assert_eq!(queue.take_ready(Instant::now()).unwrap().event, Event::PeerDisconnect);
        assert!(queue.next().is_none(), "stale connect must not run after the final disconnect");
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn saturated_actions_do_not_drop_hooks() {
        let mut engine = Engine::default();
        let cfg = Hooks { on_enter: Some(Command::Argv(vec!["/bin/sleep".into(), "30".into()])), ..Hooks::default() };
        for index in 0..MAX_RUNNING {
            engine.run_action(&cfg, &format!("/bin/sh -c 'sleep 30' unused{index}"), None);
        }
        engine.fire(&cfg, Event::Enter, &PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left });
        for _ in 0..32 { tokio::task::yield_now().await; }
        assert_eq!(engine.running.load(Ordering::SeqCst), 1, "hook budget is independent of actions");
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn pointer_and_peer_hooks_have_independent_serial_lanes() {
        let mut engine = Engine::default();
        let slow = Some(Command::Argv(vec!["/bin/sleep".into(), "30".into()]));
        let cfg = Hooks { on_enter: slow.clone(), on_peer_connect: slow, ..Hooks::default() };
        let peer = PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left };
        engine.fire(&cfg, Event::PeerConnect, &peer);
        engine.fire(&cfg, Event::Enter, &peer);
        for _ in 0..32 { tokio::task::yield_now().await; }
        assert_eq!(engine.running.load(Ordering::SeqCst), 2);
        drop(engine);
        tokio::task::yield_now().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reset_kills_hook_descendants() {
        let dir = std::env::temp_dir().join(format!("one-mk-hook-reset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let started = dir.join("started");
        let late = dir.join("late");
        let argv = vec!["/bin/sh".into(), "-c".into(), format!("(sleep .3; touch {}) & touch {}; wait", late.display(), started.display())];
        let cfg = Hooks { on_enter: Some(Command::Argv(argv)), timeout_secs: Some(30), ..Hooks::default() };
        let mut engine = Engine::default();
        engine.sync(&cfg, None, None, std::iter::empty());
        engine.fire(&cfg, Event::Enter, &PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left });
        for _ in 0..100 {
            if started.exists() { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(started.exists());
        engine.sync(&Hooks::default(), None, None, std::iter::empty());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!late.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn config_changes_discard_pending_work() {
        let mut engine = Engine::default();
        let mut cfg = Hooks { on_enter: Some(Command::Argv(vec!["echo".into(), "old".into()])), ..Hooks::default() };
        engine.sync(&cfg, None, None, std::iter::empty());
        let peer = PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left };
        engine.fire(&cfg, Event::Enter, &peer);
        cfg.on_enter = Some(Command::Argv(vec!["echo".into(), "new".into()]));
        engine.sync(&cfg, None, None, std::iter::empty());
        assert!(engine.queue.lock().unwrap().next().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn shell_conversion_preserves_hostile_argv() {
        let alphabet = ["", "a", "&", ";", "|", "$HOME", ">", "<", "*", "?", "(", ")", "`id`", "#", "~", "'", "\"", "\\", " ", "\n", "é"];
        for first in alphabet {
            for second in alphabet {
                let word = format!("{first}{second}");
                let mut hooks = Hooks { on_enter: Some(Command::Argv(vec!["printf".into(), "%s\\000".into(), word.clone()])), ..Hooks::default() };
                HooksArgs { shell: Some(true), ..Default::default() }.apply(&mut hooks).unwrap();
                let Some(Command::Line(script)) = hooks.on_enter else { panic!("shell conversion must store a script") };
                let output = std::process::Command::new("/bin/sh").args(["-c", &script]).output().unwrap();
                assert!(output.status.success(), "{word:?}");
                assert_eq!(output.stdout, [word.as_bytes(), &[0]].concat(), "{word:?}");
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn different_peers_are_not_coalesced() {
        let mut queue = Queue::default();
        for id in ["first", "second"] {
            let peer = PeerInfo { name: id.into(), id: id.into(), side: Side::Left };
            queue.insert(Pending { command: None, generation: 0, event: Event::PeerConnect, argv: vec!["echo".into()], env: env(Event::PeerConnect, &peer), shell: false, timeout: Duration::from_secs(1), due: Instant::now(), sequence: 0 }, Instant::now());
        }
        assert!(queue.take_ready(Instant::now()).is_some());
        assert!(queue.take_ready(Instant::now()).is_some());
    }
    use super::*;

    #[test]
    fn shell_flips_reinterpret_existing_commands() {
        let mut hooks = Hooks::default();
        HooksArgs { on_enter: Some("echo hello | cat".into()), ..Default::default() }.apply(&mut hooks).unwrap();
        HooksArgs { shell: Some(true), ..Default::default() }.apply(&mut hooks).unwrap();
        assert_eq!(hooks.on_enter.as_ref().unwrap().argv(true).unwrap(), shell_argv("echo hello | cat"));
        HooksArgs { shell: Some(false), ..Default::default() }.apply(&mut hooks).unwrap();
        assert_eq!(hooks.on_enter.unwrap().argv(false).unwrap(), ["echo", "hello", "|", "cat"]);
        let mut legacy = Hooks { on_enter: Some(Command::Argv(vec!["notify".into(), "hi there".into()])), ..Hooks::default() };
        HooksArgs { shell: Some(true), ..Default::default() }.apply(&mut legacy).unwrap();
        let script = if cfg!(windows) { "notify \"hi there\"" } else { "notify 'hi there'" };
        assert_eq!(legacy.on_enter.unwrap().argv(true).unwrap(), shell_argv(script));
    }

    #[test]
    fn oversized_hook_commands_are_rejected() {
        assert!(Command::Line("x".repeat(4097)).argv(false).is_err());
    }

    #[test]
    fn shell_environment_has_no_interpreter_metacharacters() {
        for (name, expected) in [("& calc", "_ calc"), ("%PATH%", "_PATH_"), ("$(id)", "__id_"), ("`id`", "_id_"), ("a\nb", "a_b"), ("\"; rm", "__ rm")] {
            assert_eq!(shell_value(name), expected);
        }
        assert_eq!(shell_value("مكتب 日本-é._- 12"), "مكتب 日本-é._- 12");
        let long = "&a".repeat(10_000);
        assert_eq!(shell_value(&long), "_a".repeat(10_000));
        let peer = PeerInfo { name: "& calc".into(), id: "%PATH%".into(), side: Side::Left };
        assert_eq!(env(Event::Enter, &peer)[1].1, "& calc", "argv hooks keep the exact peer name");
    }

    #[tokio::test(start_paused = true)]
    async fn coalescing_preserves_the_newest_transition() {
        let mut queue = Queue::default();
        let peer = PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left };
        let pending = |event| Pending { command: None, generation: 0, event, argv: vec!["echo".into()], env: env(event, &peer), shell: false, timeout: Duration::from_secs(1), due: Instant::now(), sequence: 0 };
        queue.insert(pending(Event::Leave), Instant::now());
        assert_eq!(queue.take_ready(Instant::now()).unwrap().event, Event::Leave);
        tokio::time::advance(Duration::from_millis(200)).await;
        queue.insert(pending(Event::Enter), Instant::now());
        assert_eq!(queue.take_ready(Instant::now()).unwrap().event, Event::Enter);
        tokio::time::advance(Duration::from_millis(200)).await;
        queue.insert(pending(Event::Leave), Instant::now());
        assert!(queue.take_ready(Instant::now()).is_none());
        tokio::time::advance(Duration::from_millis(100)).await;
        assert_eq!(queue.take_ready(Instant::now()).unwrap().event, Event::Leave);
        assert!(queue.next().is_none());
        queue.insert(pending(Event::Leave), Instant::now());
        queue.insert(pending(Event::Enter), Instant::now());
        tokio::time::advance(MIN_INTERVAL).await;
        assert_eq!(queue.take_ready(Instant::now()).unwrap().event, Event::Enter);
        assert!(queue.next().is_none(), "an older pending leave cannot fire after the final enter");
    }

    #[test]
    fn a_transition_without_a_hook_cancels_stale_opposite_work() {
        let mut engine = Engine::default();
        let peer = PeerInfo { name: "peer".into(), id: "id".into(), side: Side::Left };
        engine.queue.lock().unwrap().insert(Pending {
            command: None, generation: 0,
            event: Event::Leave, argv: vec!["echo".into()], env: env(Event::Leave, &peer),
            shell: false, timeout: Duration::from_secs(1), due: Instant::now(), sequence: 0,
        }, Instant::now());
        engine.fire(&Hooks::default(), Event::Enter, &peer);
        assert!(engine.queue.lock().unwrap().next().is_none());
    }

    fn words(s: &str) -> Vec<String> {
        split(s).unwrap()
    }

    #[test]
    fn splits_like_a_shell_without_expanding() {
        assert_eq!(words("a b  c"), ["a", "b", "c"]);
        assert_eq!(words("  lead and trail  "), ["lead", "and", "trail"]);
        assert_eq!(words(r#"notify-send "one mk" 'it is here'"#), ["notify-send", "one mk", "it is here"]);
        assert_eq!(words(r#"say "a \"quoted\" word" 'back\slash'"#), ["say", r#"a "quoted" word"#, r"back\slash"]);
        assert_eq!(words(r"a\ b c\\d"), ["a b", r"c\d"]);
        assert_eq!(words(r#"a"b c"d"#), ["ab cd"]); // adjacent pieces make one word
        assert_eq!(words("''"), [""]); // an empty argument is still one
        assert_eq!(words(""), Vec::<String>::new());
        // Nothing is expanded or interpreted.
        assert_eq!(words("echo $HOME ~ *.txt | cat > out; rm -rf `x` $(y)"), [
            "echo", "$HOME", "~", "*.txt", "|", "cat", ">", "out;", "rm", "-rf", "`x`", "$(y)"
        ]);
        assert_eq!(words(r#""C:\dir\x""#), [r"C:\dir\x"]);
        // Unfinished quotes are errors, not guesses.
        for bad in ["a 'b", "a \"b", "a \\"] {
            assert!(split(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn quoting_round_trips_through_split() {
        for w in ["plain", "two words", "it's", r#"say "hi""#, r"back\slash", "", "$HOME", "tab\there"] {
            assert_eq!(words(&quote(w)), [w], "{w:?}");
        }
        let cmd = Command::Argv(vec!["a b".into(), "c".into(), "it's".into()]);
        assert_eq!(words(&cmd.describe()), ["a b", "c", "it's"]);
    }

    #[test]
    fn commands_read_from_toml_as_arrays_or_strings() {
        let h: Hooks = toml::from_str(
            r#"
            on_enter = ["/bin/echo", "hello world"]
            on_leave = "notify-send 'bye now'"
            timeout_secs = 3
            "#,
        )
        .unwrap();
        assert_eq!(h.get(Event::Enter).unwrap().argv(false).unwrap(), ["/bin/echo", "hello world"]);
        assert_eq!(h.get(Event::Leave).unwrap().argv(false).unwrap(), ["notify-send", "bye now"]);
        assert_eq!(h.get(Event::PeerConnect), None);
        assert_eq!(h.timeout(), Duration::from_secs(3));
        assert!(h.any() && h.problems().is_empty());
        // A string goes to the shell as one script when asked to.
        let argv = Command::Line("a | b".into()).argv(true).unwrap();
        assert_eq!(argv.len(), 3);
        assert_eq!(argv[2], "a | b");
        // An array is never shell-run.
        assert_eq!(Command::Argv(vec!["a".into(), "| b".into()]).argv(true).unwrap(), ["a", "| b"]);
        // Written back, it reads the same.
        let again: Hooks = toml::from_str(&toml::to_string_pretty(&h).unwrap()).unwrap();
        assert_eq!(again, h);
        assert_eq!(toml::to_string_pretty(&Hooks::default()).unwrap().trim(), "");
    }

    #[test]
    fn bad_commands_are_reported() {
        let h = Hooks {
            on_enter: Some(Command::Line("echo 'oops".into())),
            on_leave: Some(Command::Argv(vec![])),
            on_peer_connect: Some(Command::Line("   ".into())),
            ..Hooks::default()
        };
        assert_eq!(h.problems().len(), 3);
        assert!(Hooks::default().problems().is_empty());
    }

    #[test]
    fn timeout_is_bounded() {
        let at = |t| Hooks { timeout_secs: t, ..Hooks::default() }.timeout().as_secs();
        assert_eq!((at(None), at(Some(0)), at(Some(10)), at(Some(100_000))), (5, 1, 10, 60));
    }

    #[test]
    fn each_event_runs_at_most_every_half_second() {
        let mut r = RateLimit::default();
        let t = Instant::now();
        assert!(r.allow(Event::Enter, t));
        assert!(!r.allow(Event::Enter, t + Duration::from_millis(499)));
        assert!(r.allow(Event::Leave, t + Duration::from_millis(1)), "events don't share a limit");
        // A dropped run doesn't push the next one back.
        assert!(r.allow(Event::Enter, t + Duration::from_millis(500)));
        assert!(!r.allow(Event::Enter, t + Duration::from_millis(999)));
        assert!(r.allow(Event::Enter, t + Duration::from_millis(1000)));
    }

    #[test]
    fn environment_names_the_event_and_the_peer() {
        let p = PeerInfo { name: "laptop".into(), id: "ab12".into(), side: Side::Left };
        let e = env(Event::PeerDisconnect, &p);
        assert_eq!(
            e.map(|(k, v)| format!("{k}={v}")),
            ["ONE_MK_EVENT=peer_disconnect", "ONE_MK_PEER=laptop", "ONE_MK_PEER_ID=ab12", "ONE_MK_SIDE=left"]
        );
        assert_eq!(Event::ALL.map(Event::as_str), ["enter", "leave", "peer_connect", "peer_disconnect"]);
    }

    #[test]
    fn group_and_world_writable_files_are_reported() {
        assert_eq!(writable_by_others(0o600), None);
        assert_eq!(writable_by_others(0o644), None);
        assert_eq!(writable_by_others(0o664), Some("writable by its group"));
        assert_eq!(writable_by_others(0o666), Some("writable by everyone"));
        assert_eq!(writable_by_others(0o602), Some("writable by everyone"));
    }

    #[cfg(unix)]
    #[test]
    fn a_real_file_is_checked() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("one-mk-hooks-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The folder counts too, and a umask of 002 makes new ones group-writable.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let f = dir.join("settings.toml");
        std::fs::write(&f, "").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(unsafe_permissions(&f), None);
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o660)).unwrap();
        assert!(unsafe_permissions(&f).unwrap().contains("group"));
        assert_eq!(unsafe_permissions(&dir.join("missing.toml")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_directory_blocks_hooks_and_only_explicit_trust_overrides_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("one-mk-hooks-directory-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770)).unwrap();
        let mut settings = crate::settings::Settings::default();
        settings.hooks.on_enter = Some(Command::Line("echo test".into()));
        settings.save(&dir).unwrap();
        let loaded = crate::settings::Settings::load(&dir).unwrap();
        assert!(!loaded.hooks.any());
        assert!(loaded.hooks.describe().contains("REFUSED"));
        let path = dir.join("settings.toml");
        std::fs::write(&path, std::fs::read_to_string(&path).unwrap().replace("[hooks]\n", "[hooks]\ntrust_permissions = true\n")).unwrap();
        assert!(!crate::settings::Settings::load(&dir).unwrap().hooks.any(), "untrusted TOML cannot authorize itself");
        trust_permissions(&dir).unwrap();
        assert!(crate::settings::Settings::load(&dir).unwrap().hooks.any());
        std::fs::set_permissions(dir.join(TRUST_FILE), std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(!crate::settings::Settings::load(&dir).unwrap().hooks.any());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn command_line_changes() {
        let mut h = Hooks::default();
        let args = HooksArgs {
            on_enter: Some("notify-send 'one mk' here".into()),
            on_leave: Some("/bin/true".into()),
            timeout: Some(8),
            ..HooksArgs::default()
        };
        args.apply(&mut h).unwrap();
        assert_eq!(h.on_enter, Some(Command::Line("notify-send 'one mk' here".into())));
        assert_eq!(h.timeout_secs, Some(8));
        // An empty command removes one hook, leaves the rest.
        HooksArgs { on_leave: Some(String::new()), ..HooksArgs::default() }.apply(&mut h).unwrap();
        assert_eq!((h.on_leave.is_none(), h.on_enter.is_some()), (true, true));
        // The shell keeps strings whole.
        HooksArgs { shell: Some(true), on_leave: Some("a | b".into()), ..HooksArgs::default() }.apply(&mut h).unwrap();
        assert_eq!(h.on_leave, Some(Command::Line("a | b".into())));
        // Mistakes change nothing that was valid before and are reported.
        assert!(HooksArgs { timeout: Some(0), ..HooksArgs::default() }.apply(&mut h).is_err());
        assert!(HooksArgs { shell: Some(false), on_peer_connect: Some("echo 'x".into()), ..HooksArgs::default() }.apply(&mut Hooks::default()).is_err());
        HooksArgs { off: true, ..HooksArgs::default() }.apply(&mut h).unwrap();
        assert_eq!(h, Hooks::default());
    }

    fn fp(n: u8) -> Fingerprint {
        Fingerprint::of(&[n])
    }

    /// The events `Engine::diff` reports as the daemon's state moves.
    #[test]
    fn engine_reports_enter_leave_and_peers_coming_and_going() {
        let (a, b) = (fp(1), fp(2));
        let mut e = Engine::default();
        let step = |e: &mut Engine, away, owner, peers: &[(Fingerprint, &str, Side)]| -> Vec<(Event, String)> {
            e.diff(away, owner, peers.iter().copied()).into_iter().map(|(ev, i)| (ev, i.name)).collect()
        };
        let both = [(a, "alpha", Side::Left), (b, "beta", Side::Right)];
        // Whatever is true when hooks get set is not an event.
        assert_eq!(step(&mut e, Some(a), None, &both), []);
        assert_eq!(step(&mut e, Some(a), None, &both), []);
        // The pointer comes back from alpha.
        assert_eq!(step(&mut e, None, None, &both), [(Event::Enter, "alpha".into())]);
        // It goes to beta, then straight on to alpha: still away.
        assert_eq!(step(&mut e, Some(b), None, &both), [(Event::Leave, "beta".into())]);
        assert_eq!(step(&mut e, Some(a), None, &both), []);
        assert_eq!(step(&mut e, None, None, &both), [(Event::Enter, "alpha".into())]);
        // A peer takes control, then lets go.
        assert_eq!(step(&mut e, None, Some(b), &both), [(Event::Enter, "beta".into())]);
        assert_eq!(step(&mut e, None, None, &both), [(Event::Leave, "beta".into())]);
        // A peer disconnects while controlling: it leaves, then is gone.
        assert_eq!(step(&mut e, None, Some(b), &both), [(Event::Enter, "beta".into())]);
        assert_eq!(
            step(&mut e, None, None, &both[..1]),
            [(Event::Leave, "beta".into()), (Event::PeerDisconnect, "beta".into())]
        );
        // And comes back.
        assert_eq!(step(&mut e, None, None, &both), [(Event::PeerConnect, "beta".into())]);
        assert_eq!(step(&mut e, None, None, &both), []);
    }

    #[tokio::test]
    async fn nothing_is_tracked_while_no_hook_is_set() {
        let mut e = Engine::default();
        let off = Hooks::default();
        e.sync(&off, Some(fp(1)), None, [(fp(1), "x", Side::Left)].into_iter());
        assert!(!e.primed && e.known.is_empty());
        // Setting a hook later takes note of the state without firing.
        let on = Hooks { on_enter: Some(Command::Argv(vec!["/bin/true".into()])), ..Hooks::default() };
        e.sync(&on, Some(fp(1)), None, [(fp(1), "x", Side::Left)].into_iter());
        assert!(e.primed);
        assert_eq!(e.running.load(Ordering::SeqCst), 0);
        e.sync(&off, None, None, std::iter::empty());
        assert!(!e.primed);
    }

    #[tokio::test]
    async fn a_hook_runs_and_is_killed_at_the_timeout() {
        if !cfg!(unix) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("one-mk-hooks-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out");
        let script = format!("echo \"$ONE_MK_EVENT $ONE_MK_PEER $ONE_MK_SIDE\" >> {}", out.display());
        let argv = vec!["/bin/sh".to_owned(), "-c".to_owned(), script];
        let p = PeerInfo { name: "other pc".into(), id: "x".into(), side: Side::Right };
        run(Event::Enter.as_str(), &argv, &env(Event::Enter, &p), Duration::from_secs(5), false).await;
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "enter other pc right\n");

        // A hook that outlives its timeout is killed, with what it started.
        let marker = dir.join("late");
        let slow = vec!["/bin/sh".to_owned(), "-c".to_owned(), format!("sleep 30 & sleep 30; touch {}", marker.display())];
        let t = Instant::now();
        run(Event::Leave.as_str(), &slow, &env(Event::Leave, &p), Duration::from_millis(200), false).await;
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(!marker.exists());
        // A program that doesn't exist is just reported.
        run(Event::Leave.as_str(), &["/nonexistent/program".to_owned()], &[], Duration::from_secs(1), false).await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
