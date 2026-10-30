//! `one-mk doctor`: finds out why computers aren't connecting and says what
//! to do about it, in plain language.
//!
//! The decisions are pure functions from plain facts to a [`Check`], so they
//! are unit-tested with fake inputs. The probes at the bottom gather the
//! facts: they are read-only (no firewall rule, config file or permission is
//! ever changed), need no root, run concurrently with short timeouts, and
//! only run when the command is invoked.

use crate::control::{self, Request};
use anyhow::Result;
use one_mk_net::{Discovery, Found, Identity, TrustStore, is_virtual_interface};
use one_mk_proto::{DEFAULT_PORT, sanitize_name};
use std::net::{Ipv4Addr, UdpSocket};
use std::path::Path;
use std::time::Duration;

use crate::manage::Status as DaemonStatus;
use crate::settings::Settings;
use one_mk_input::cursoroverlay::{DrawMode, Support};

/// How the daemon is asked, and how long mDNS gets to answer.
const DAEMON_TIMEOUT: Duration = Duration::from_millis(1500);
const MDNS_WINDOW: Duration = Duration::from_millis(1500);
const PORTAL_TIMEOUT: Duration = Duration::from_millis(2500);
const COMMAND_TIMEOUT: Duration = Duration::from_millis(1500);
const MDNS_PORT: u16 = 5353;
const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

// ---------------------------------------------------------------- model --

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Pass,
    /// Not applicable here (another OS), or not something a probe can know.
    Skip,
    Warn,
    Fail,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Skip => "skip",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Self::Pass => "[ ok ]",
            Self::Skip => "[skip]",
            Self::Warn => "[warn]",
            Self::Fail => "[FAIL]",
        }
    }
}

/// What a check found: how bad it is, what was seen, and the fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

impl Outcome {
    fn new(status: Status, detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self { status, detail: detail.into(), fix: fix.map(str::to_owned) }
    }
    fn pass(detail: impl Into<String>) -> Self {
        Self::new(Status::Pass, detail, None)
    }
    fn skip(detail: impl Into<String>) -> Self {
        Self::new(Status::Skip, detail, None)
    }
    fn warn(detail: impl Into<String>, fix: &str) -> Self {
        Self::new(Status::Warn, detail, Some(fix))
    }
    fn fail(detail: impl Into<String>, fix: &str) -> Self {
        Self::new(Status::Fail, detail, Some(fix))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Stable name for programs (the window will read `--json`).
    pub id: String,
    pub title: String,
    pub outcome: Outcome,
}

fn check(id: &str, title: &str, outcome: Outcome) -> Check {
    Check { id: id.to_owned(), title: title.to_owned(), outcome }
}

// ---------------------------------------------------------------- facts --

/// The daemon, as the control channel answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Daemon {
    Running(Box<DaemonStatus>),
    NotRunning,
    /// Something is there but did not answer sensibly.
    Broken(String),
}

impl Daemon {
    /// Reads the reply of `request(dir, Request::Status)`.
    fn from_reply(reply: Result<Option<String>, String>) -> Self {
        match reply {
            Ok(None) => Self::NotRunning,
            Ok(Some(text)) => match DaemonStatus::decode(&text) {
                Some(s) => Self::Running(Box::new(s)),
                None => Self::Broken("it gave an answer we could not read".into()),
            },
            Err(e) => Self::Broken(e),
        }
    }

    fn status(&self) -> Option<&DaemonStatus> {
        match self {
            Self::Running(s) => Some(s.as_ref()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paired {
    pub id: String,
    pub name: String,
    pub addrs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub dir_exists: bool,
    pub has_identity: bool,
    /// The paired computers, or why `devices.toml` could not be read.
    pub paired: Result<Vec<Paired>, String>,
    /// Whether `settings.toml` (if there is one) can be read.
    pub settings: Result<(), String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bind {
    Free,
    InUse,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct PortFacts {
    pub port: u16,
    pub bind: Bind,
    /// Which program holds it, when we could find out.
    pub holder: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MdnsFacts {
    pub join: Result<(), String>,
    /// What browsing for one-mk computers saw, or why it could not start.
    pub seen: Result<Vec<Seen>, String>,
}

/// A one-mk computer seen on the network (short id as in `one-mk devices`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub id: String,
    pub addrs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Firewall {
    /// `Some(true)`: ufw is switched on. `None`: could not tell.
    pub ufw_active: Option<bool>,
    /// `/etc/ufw/user.rules`, when this user may read it (usually not).
    pub ufw_rules: Option<String>,
    pub firewalld: Option<Firewalld>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Firewalld {
    pub ports: String,
    pub services: String,
}

#[derive(Clone, Debug)]
pub struct Portals {
    pub wayland: bool,
    /// `None`: the portals did not answer in time.
    pub remote_desktop: Option<Result<(), String>>,
    pub input_capture: Option<Result<(), String>>,
    pub token: bool,
    /// The running backend when known, otherwise what `native` picks.
    pub backend: Option<(String, String)>,
}

impl Portals {
    fn use_running_backend(&mut self, backend: one_mk_ctl::status::InputBackend) {
        if !backend.as_str().is_empty() {
            self.backend = Some((backend.as_str().to_owned(), "reported by the running daemon".into()));
        }
    }
}

/// An address on an up network interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addr {
    pub iface: String,
    pub ip: Ipv4Addr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    Mac,
    Other,
}

const fn this_os() -> Os {
    if cfg!(target_os = "linux") {
        Os::Linux
    } else if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Other
    }
}

// ------------------------------------------------------------ decisions --

pub fn daemon_check(d: &Daemon) -> Check {
    let out = match d {
        Daemon::Running(s) => Outcome::pass(format!("one-mk is running on {} (listening on UDP {}).", s.name, s.port)),
        Daemon::NotRunning => Outcome::fail(
            "one-mk is not running on this computer.",
            "Start it with `one-mk run`, or run `one-mk install` so it starts when you log in.",
        ),
        Daemon::Broken(why) => Outcome::fail(
            format!("one-mk seems to be running but does not answer ({why})."),
            "Stop it (end the one-mk process) and start it again with `one-mk run`.",
        ),
    };
    check("daemon", "Program", out)
}

pub fn config_checks(c: &Config, dir: &Path) -> Vec<Check> {
    let folder = if let (true, Err(e)) = (c.dir_exists, &c.settings) {
        Outcome::warn(
            format!(
                "Found at {}, but settings.toml cannot be read ({e}). one-mk ignores it: a running one-mk keeps the \
                 settings it has, and a new start uses the defaults.",
                dir.display()
            ),
            "Fix the line it points to, or delete `settings.toml` in the settings folder (everything goes back to the defaults).",
        )
    } else if c.dir_exists {
        Outcome::pass(format!("Found at {}.", dir.display()))
    } else {
        Outcome::warn(
            format!("There is no settings folder yet ({}).", dir.display()),
            "This computer has not been set up: run `one-mk pair` on it and on the other computer.",
        )
    };
    let identity = match (c.dir_exists, c.has_identity) {
        (_, true) => Outcome::pass("This computer has its identity (key and certificate)."),
        (false, false) => Outcome::skip("No identity yet; it is created the first time one-mk starts."),
        (true, false) => Outcome::warn(
            "This computer has no identity yet.",
            "It is created automatically the first time you run `one-mk run` or `one-mk pair`.",
        ),
    };
    let pairing = match &c.paired {
        Ok(p) if p.is_empty() => Outcome::fail(
            "This computer is not paired with any other.",
            "Run `one-mk pair` on both computers and confirm the same code on each.",
        ),
        Ok(p) => Outcome::pass(format!("Paired with {} computer{}.", p.len(), if p.len() == 1 { "" } else { "s" })),
        Err(e) => Outcome::fail(
            format!("The list of paired computers could not be read ({e})."),
            "Fix or delete `devices.toml` in the settings folder, then pair again with `one-mk pair`.",
        ),
    };
    vec![
        check("config", "Settings", folder),
        check("identity", "Identity", identity),
        check("pairing", "Paired computers", pairing),
    ]
}

pub fn port_check(d: &Daemon, p: &PortFacts) -> Check {
    let port = p.port;
    let out = match (d.status().is_some(), &p.bind) {
        (true, Bind::InUse) => Outcome::pass(format!("UDP port {port} is open, held by one-mk itself.")),
        (true, Bind::Free) => Outcome::fail(
            format!("one-mk is running but is not listening on UDP port {port}."),
            "Restart one-mk (`one-mk run`) and watch for an error about the port.",
        ),
        (false, Bind::Free) => Outcome::pass(format!("UDP port {port} is free for one-mk.")),
        (false, Bind::InUse) => {
            let who = p.holder.as_deref().map_or("another program".to_owned(), |h| format!("{h:?}"));
            Outcome::fail(
                format!("UDP port {port} is already used by {who}."),
                &format!(
                    "Close that program, or use another port on both computers: `one-mk run --port {}`.",
                    port.wrapping_add(10).max(1024)
                ),
            )
        }
        (_, Bind::Error(e)) => Outcome::warn(
            format!("Could not test UDP port {port} ({e})."),
            "Not necessarily a problem; if connecting fails, check that nothing else uses this port.",
        ),
    };
    check("port", "Port", out)
}

/// The port one-mk listens on here (the other computers are assumed to use the same).
fn port_of(d: &Daemon) -> u16 {
    d.status().map_or(DEFAULT_PORT, |s| s.port)
}

pub fn mdns_check(d: &Daemon, m: &MdnsFacts) -> Check {
    let title = "Discovery";
    let port = port_of(d);
    let blocked = format!(
        "Check that no firewall or VPN blocks UDP 5353 (mDNS) and that you are not on a guest Wi-Fi. \
         Without discovery, connect directly: `one-mk run --connect <ip>:{port}`."
    );
    let blocked = blocked.as_str();
    if let Err(e) = &m.join {
        let out =
            Outcome::fail(format!("This computer cannot join the mDNS group {MDNS_GROUP}:{MDNS_PORT} ({e})."), blocked);
        return check("mdns", title, out);
    }
    let seen = match &m.seen {
        Ok(s) => s,
        Err(e) => {
            let out = Outcome::fail(format!("Looking for computers failed ({e})."), blocked);
            return check("mdns", title, out);
        }
    };
    let out = match d {
        Daemon::Running(s) if seen.iter().any(|f| f.id == s.id) => {
            Outcome::pass("Discovery works: this computer's own announcement came back over the network.")
        }
        Daemon::Running(_) => Outcome::warn(
            "one-mk is running but its announcement was not seen on the network.",
            &format!(
                "Was it started with `--no-discovery`? Otherwise something drops mDNS (UDP 5353): a firewall, VPN or guest Wi-Fi. \
                 Connecting with `--connect <ip>:{port}` still works."
            ),
        ),
        _ => Outcome::pass(format!(
            "This computer can use mDNS; {} one-mk computer{} announced {}.",
            seen.len(),
            if seen.len() == 1 { "" } else { "s" },
            if seen.len() == 1 { "itself" } else { "themselves" },
        )),
    };
    check("mdns", title, out)
}

/// Whether `list` (like `24900/udp 8000-9000/tcp`) allows UDP `port`.
fn port_listed(list: &str, port: u16) -> bool {
    list.split_whitespace().any(|item| {
        let Some((range, proto)) = item.split_once('/') else { return false };
        if proto != "udp" {
            return false;
        }
        match range.split_once('-') {
            Some((a, b)) => matches!((a.parse::<u16>(), b.parse::<u16>()), (Ok(a), Ok(b)) if a <= port && port <= b),
            None => range.parse() == Ok(port),
        }
    })
}

/// Whether ufw's `user.rules` accepts UDP `port` (single, or a range like `24000:25000`).
fn ufw_allows(rules: &str, port: u16) -> bool {
    rules.lines().any(|l| {
        let l = l.trim();
        if !l.starts_with("-A ufw-user-input") || !l.ends_with("-j ACCEPT") || !l.contains("-p udp") {
            return false;
        }
        let mut words = l.split_whitespace();
        while let Some(w) = words.next() {
            if w == "--dport" || w == "--dports" {
                return words.next().is_some_and(|spec| {
                    spec.split(',').any(|s| match s.split_once(':') {
                        Some((a, b)) => {
                            matches!((a.parse::<u16>(), b.parse::<u16>()), (Ok(a), Ok(b)) if a <= port && port <= b)
                        }
                        None => s.parse() == Ok(port),
                    })
                });
            }
        }
        false
    })
}

pub fn firewall_check(os: Os, f: &Firewall, port: u16) -> Check {
    let title = "Firewall";
    let out = match os {
        Os::Windows => Outcome::skip(
            "Not checked on Windows. If Windows Defender Firewall asked about one-mk, choose Allow (at least on Private networks); \
             otherwise allow one-mk.exe under Windows Security > Firewall & network protection > Allow an app.",
        ),
        Os::Mac => Outcome::skip(
            "Not checked on macOS. If it asked \"accept incoming network connections?\", choose Allow; \
             otherwise allow one-mk under System Settings > Network > Firewall > Options.",
        ),
        Os::Other => Outcome::skip("Not checked on this system."),
        Os::Linux => linux_firewall(f, port),
    };
    check("firewall", title, out)
}

fn linux_firewall(f: &Firewall, port: u16) -> Outcome {
    let mut active = false;
    let mut blocked: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    if f.ufw_active == Some(true) {
        active = true;
        // ufw's own rules already let mDNS multicast in; only our port matters.
        match &f.ufw_rules {
            Some(r) if ufw_allows(r, port) => {}
            Some(_) => blocked.push(format!("ufw blocks UDP {port}: `sudo ufw allow {port}/udp`")),
            None => unknown.push(format!(
                "ufw is on and its rules need root to read: if connecting fails, run `sudo ufw allow {port}/udp`"
            )),
        }
    }
    if let Some(fw) = &f.firewalld {
        active = true;
        let mdns = fw.services.split_whitespace().any(|s| s == "mdns") || port_listed(&fw.ports, MDNS_PORT);
        let mut missing = Vec::new();
        let mut fix = Vec::new();
        if !port_listed(&fw.ports, port) {
            missing.push(format!("UDP {port}"));
            fix.push(format!("`sudo firewall-cmd --permanent --add-port={port}/udp`"));
        }
        if !mdns {
            missing.push(format!("mDNS (UDP {MDNS_PORT})"));
            fix.push("`sudo firewall-cmd --permanent --add-service=mdns`".to_owned());
        }
        if !missing.is_empty() {
            blocked.push(format!(
                "firewalld does not allow {} in its default zone: {} then `sudo firewall-cmd --reload`",
                missing.join(" and "),
                fix.join(" and "),
            ));
        }
    }
    if !blocked.is_empty() {
        return Outcome::new(Status::Fail, "The firewall blocks one-mk.", Some(&blocked.join("; ")));
    }
    if !unknown.is_empty() {
        return Outcome::new(
            Status::Warn,
            "Could not tell whether the firewall allows one-mk.",
            Some(&unknown.join("; ")),
        );
    }
    if active {
        Outcome::pass(format!("The firewall allows UDP {port} and mDNS."))
    } else if f.ufw_active.is_none() {
        Outcome::warn(
            "Could not tell whether a firewall is on (ufw and firewalld could not be queried without root).",
            &format!("If connecting fails, allow UDP {port} and UDP {MDNS_PORT} (mDNS) in your firewall."),
        )
    } else {
        Outcome::pass(
            "No ufw or firewalld firewall is on. (Other filters, such as a router's, cannot be checked from here.)",
        )
    }
}

/// `running`: what the running daemon's input backend reports (waiting
/// for the permission dialog, permission declined, retrying).
pub fn portal_check(os: Os, p: Option<&Portals>, running: &[String]) -> Check {
    let title = "Permissions";
    let out = match (os, p) {
        (Os::Linux, Some(p)) => linux_portals(p),
        (Os::Mac, _) => Outcome::skip(
            "Not checked on macOS. Allow one-mk under System Settings > Privacy & Security > Accessibility and Input Monitoring.",
        ),
        _ => Outcome::skip("Nothing to check on this system."),
    };
    let out = match running {
        [] => out,
        _ if out.status == Status::Fail => out,
        lines => Outcome::warn(
            format!("The running one-mk says: {}", lines.join(" ")),
            "Do as it says; `one-mk status` shows it until it is resolved.",
        ),
    };
    check("input", title, out)
}

fn linux_portals(p: &Portals) -> Outcome {
    match p.backend.as_ref().map(|(name, why)| (name.as_str(), why)) {
        Some(("x11", why)) => {
            return Outcome::pass(format!("one-mk uses its X11 backend: {why}. X11 asks for no permission."));
        }
        Some(("wlroots", why)) => {
            return Outcome::pass(format!(
                "one-mk uses its wlroots backend: {why}. The compositor asks for no permission."
            ));
        }
        _ => {}
    }
    let mut out = linux_portal_permissions(p);
    if let Some((_, why)) = &p.backend {
        out.detail = format!("one-mk uses the desktop portals ({why}). {}", out.detail);
    }
    out
}

fn linux_portal_permissions(p: &Portals) -> Outcome {
    const INSTALL: &str = "Use GNOME 45+ or KDE Plasma 6.1+ on Wayland, with xdg-desktop-portal and your desktop's portal \
                           (xdg-desktop-portal-gnome or -kde) installed, then log in again. A wlroots compositor (Sway, \
                           Hyprland, labwc) or an X11 session works too: one-mk drives those directly.";
    match &p.remote_desktop {
        None => {
            return Outcome::warn(
                "The desktop portal did not answer in time.",
                "Log out and in again; if it persists, restart xdg-desktop-portal.",
            );
        }
        Some(Err(e)) => {
            return Outcome::fail(
                format!("This desktop cannot let one-mk control the mouse and keyboard ({e})."),
                INSTALL,
            );
        }
        Some(Ok(())) => (),
    }
    match &p.input_capture {
        None => {
            return Outcome::warn(
                "The desktop portal did not answer in time.",
                "Log out and in again; if it persists, restart xdg-desktop-portal.",
            );
        }
        Some(Err(e)) => {
            return Outcome::fail(
                format!("This desktop cannot let one-mk take the mouse and keyboard to other computers ({e})."),
                INSTALL,
            );
        }
        Some(Ok(())) => (),
    }
    if !p.wayland {
        return Outcome::warn(
            "This is not a Wayland session; one-mk's Linux support is built for Wayland.",
            "Log in with the Wayland session (the gear menu on the login screen).",
        );
    }
    if p.token {
        Outcome::pass("The desktop offers the needed portals and you already allowed one-mk.")
    } else {
        Outcome::warn(
            "The desktop offers the needed portals, but you have not allowed one-mk yet.",
            "Run `one-mk run` and choose Allow in the dialog your desktop shows. (If you dismissed it, run it again.)",
        )
    }
}

/// Keyboard shortcuts, as the running daemon reports them; with none
/// running, whether the desktop offers them (`portal`: Linux only).
pub fn hotkey_check(d: &Daemon, portal: Option<&Result<u32, String>>) -> Check {
    let mut fallback = one_mk_ctl::status::HOTKEYS_FALLBACK.to_owned() + ".";
    fallback[..1].make_ascii_uppercase();
    let away = "They work only while the pointer is on another computer";
    let out = match d.status() {
        Some(s) => match s.hotkeys.as_str() {
            "on" => Outcome::pass(format!("They work {}.", s.hotkeys_detail)),
            "unavailable" => Outcome::warn(format!("{away}: {}.", s.hotkeys_detail), &fallback),
            "starting" => Outcome::skip(format!("Being set up: {}.", s.hotkeys_detail)),
            "off" if s.hotkeys_detail.is_empty() => Outcome::skip("Turned off (`one-mk hotkeys --on` turns them on)."),
            "off" => Outcome::warn(
                format!("They are off: {}.", s.hotkeys_detail),
                "Fix them with `one-mk hotkeys`, or the [hotkeys] table in settings.toml.",
            ),
            _ => Outcome::skip("This one-mk is too old to report them."),
        },
        None => match portal {
            Some(Ok(_)) => Outcome::pass("The desktop offers global shortcuts (the GlobalShortcuts portal)."),
            Some(Err(e)) => Outcome::warn(format!("{away}: {e}."), &fallback),
            None => Outcome::skip("Checked while one-mk runs."),
        },
    };
    check("hotkeys", "Keyboard shortcuts", out)
}

/// Broadcast (`one-mk broadcast`): on right now (said, so typing that goes
/// everywhere is never a mystery), its shortcut and when it stops by
/// itself, and where it has not been tried yet.
pub fn broadcast_check(d: &Daemon, settings: Option<&crate::settings::BroadcastSettings>, os: Os) -> Check {
    let idle = settings.map_or_else(|| "by itself after 15 minutes without input".to_owned(), |b| b.describe_idle_off());
    // Tried for real: Linux with the null and X11 backends.
    let untested = match (os, d.status().map(|s| s.backend.as_str())) {
        (Os::Windows | Os::Mac, _) => Some("Broadcast has not been tried on this system yet (untested)."),
        (_, Some("portal" | "wlroots")) => Some("Broadcast has not been tried with this input backend yet (untested)."),
        _ => None,
    };
    let note = untested.map_or_else(String::new, |n| format!(" {n}"));
    let out = match d.status() {
        Some(s) if !s.broadcast.is_empty() => Outcome::warn(
            format!("On: everything typed, clicked and scrolled here goes to {} ({}). It stops {idle}.{note}", s.broadcast.len(), s.broadcast.join(", ")),
            "If you didn't mean it, stop it: `one-mk broadcast off`, its shortcut, or Ctrl+Alt+Shift+Esc.",
        ),
        Some(s) if s.broadcast_hotkey.is_empty() => {
            Outcome::pass(format!("Off. No shortcut starts it (`one-mk hotkeys --broadcast CHORD` sets one); `one-mk broadcast on` does. It stops {idle}.{note}"))
        }
        Some(s) => Outcome::pass(format!("Off. {} or `one-mk broadcast on` starts it; it stops {idle}.{note}", s.broadcast_hotkey)),
        None => Outcome::skip(format!("Checked while one-mk runs.{note}")),
    };
    check("broadcast", "Broadcast", out)
}

/// `[input]` in settings.toml, read from the file (`None`: it can't be read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// What is set, in a line; `None` when nothing is.
    pub summary: Option<String>,
    /// Rules that can't work, conflict or never apply.
    pub problems: Vec<String>,
    /// What to know about rules that do (where rules for apps can't see
    /// the app).
    pub notes: Vec<String>,
}

/// Why rules for apps don't apply to Wayland windows on Linux.
pub const WAYLAND_APPS: &str = "Rules for apps see only X11 (XWayland) windows on a Linux computer: GNOME and KDE don't tell \
a background program which Wayland window has the focus, so in those windows the rules for apps don't apply";

/// Input tuning: skipped when nothing is set; the problems
/// `one-mk keymap check` would print, if any.
pub fn tuning_check(t: Option<&Tuning>) -> Check {
    let out = match t {
        None => Outcome::skip("settings.toml can't be read (see the settings check)."),
        Some(Tuning { summary: None, .. }) => Outcome::skip("No key, button, pointer or scroll rules are set."),
        Some(Tuning { summary: Some(s), problems, notes }) if problems.is_empty() => {
            Outcome::pass(notes.iter().fold(s.clone(), |all, n| format!("{all} {n}.")))
        }
        Some(Tuning { problems, notes, .. }) => Outcome::warn(
            format!("{}.", problems.iter().chain(notes).cloned().collect::<Vec<_>>().join("; ")),
            "See `one-mk keymap check`, and fix or remove those rules (`one-mk keymap`, `buttons`, `pointer`, `scroll`, or [input] in settings.toml).",
        ),
    };
    check("input-tuning", "Input tuning", out)
}

/// What decides the per-device rules check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRules {
    /// `Ok(untested note)` where this computer can tell devices apart,
    /// `Err(why not)` where it can't.
    pub support: Result<Option<String>, String>,
    /// Devices seen, and how many of them are connected now.
    pub seen: usize,
    pub connected: usize,
    /// Rules for one device are set.
    pub rules: bool,
}

/// Per-device rules (5I): pass where devices can be told apart (saying
/// what is untested), skip with the reason where the desktop merges them,
/// warn when such rules are set there.
pub fn device_rules_check(d: &DeviceRules) -> Check {
    let out = match &d.support {
        Ok(note) => {
            let mut detail = format!(
                "This computer can tell its keyboards and mice apart: {} seen, {} connected now (`one-mk input-devices`).",
                d.seen, d.connected
            );
            if let Some(n) = note {
                detail.push_str(&format!(" Note: {n}."));
            }
            Outcome::pass(detail)
        }
        Err(why) if d.rules => Outcome::warn(
            format!("Rules for one keyboard or mouse are set, but {why}, so they never apply here."),
            "They work on X11 sessions, Windows and macOS. Remove them with `one-mk keymap remove ... --device ...` (and buttons, pointer, scroll), or leave them for another session.",
        ),
        Err(why) => Outcome::skip(format!("{}.", crate::inputdevices::sentence(why))),
    };
    check("per-device-rules", "Per-device rules", out)
}

fn probe_device_rules(dir: &Path, running: bool) -> DeviceRules {
    let r = crate::inputdevices::Remembered::load(dir).unwrap_or_default();
    let support = crate::inputdevices::support(&r, running);
    let rules = Settings::load(dir).map(|s| s.input.uses_devices()).unwrap_or(false);
    let connected = if running { r.devices.iter().filter(|x| x.connected).count() } else { 0 };
    DeviceRules { support, seen: r.devices.len(), connected, rules }
}

/// The pointer one-mk draws where a computer has no mouse (plan 3A):
/// available, not needed or not available here.
pub fn pointer_overlay_check(support: Support, mode: DrawMode) -> Check {
    let detail = crate::pointeroverlay::describe(support, mode);
    let out = match (support, mode) {
        (Support::NotAvailable, DrawMode::Always) => {
            Outcome::warn(detail, "`one-mk pointer-overlay always` has no effect here; `one-mk pointer-overlay auto` is the default.")
        }
        (Support::Available | Support::NotNeeded, _) => Outcome::pass(detail),
        _ => Outcome::skip(detail),
    };
    check("pointer-overlay", "Pointer without a mouse", out)
}

/// Pointer wrap and layout learning (`[edge] wrap`, `learn`): what is on,
/// whether it was tried with this input backend (`backend` as `one-mk
/// status` names it; empty when one-mk isn't running), and the question
/// learning is asking, if any.
pub fn edge_push_check(edge: &crate::settings::EdgeSettings, backend: &str, question: Option<String>) -> Check {
    use crate::settings::{Learn, Wrap};
    let id = "edge-push";
    let title = "Edges with no computer";
    if edge.wrap == Wrap::Off && edge.learn == Learn::Off {
        return check(id, title, Outcome::skip("Pointer wrap and layout learning are off."));
    }
    let wrap = match edge.wrap {
        Wrap::Off => "Pointer wrap is off",
        Wrap::On => "The pointer wraps round at left and right edges with no computer",
        Wrap::All => "The pointer wraps round at every edge with no computer",
    };
    let learn = match edge.learn {
        Learn::Off => "layout learning is off",
        Learn::Suggest => "pushing toward a computer that isn't placed yet suggests where it is",
        Learn::Auto => "pushing toward a computer that isn't placed yet places it",
    };
    let mut detail = format!("{wrap}; {learn}.");
    if let Some(q) = question {
        detail += &format!(" Asking now: {q}");
    }
    let acts = edge.wrap != Wrap::Off || edge.learn == Learn::Auto;
    let out = match edge_push_tested(backend) {
        Some(true) | None => Outcome::pass(detail),
        Some(false) if acts => Outcome::warn(
            format!("{detail} Not yet tested on {}.", backend_words(backend)),
            "If the pointer misbehaves at an edge with no computer, turn it off with `one-mk edge --wrap off --learn suggest`, and report it.",
        ),
        Some(false) => Outcome::pass(format!("{detail} Not yet tested on {}.", backend_words(backend))),
    };
    check(id, title, out)
}

/// Where `backend` (as `one-mk status` names it) runs, in words.
pub fn backend_words(backend: &str) -> &str {
    match backend {
        "portal" => "the Wayland desktop portals (GNOME, KDE)",
        "wlroots" => "wlroots desktops (Sway, Hyprland)",
        "native" if cfg!(windows) => "Windows",
        "native" if cfg!(target_os = "macos") => "macOS",
        other => other,
    }
}

/// Whether wrap and learning were tried with `backend` (`None`: unknown,
/// one-mk isn't running). Only X11 was, on a private X server; the null
/// backend captures nothing.
pub fn edge_push_tested(backend: &str) -> Option<bool> {
    match backend {
        "" => None,
        "x11" | "null" => Some(true),
        _ => Some(false),
    }
}

fn probe_tuning(dir: &Path) -> Option<Tuning> {
    let input = Settings::load(dir).ok()?.input;
    let paired = TrustStore::load(dir)
        .map(|t| t.devices().iter().map(|d| (d.name.clone(), d.fingerprint.clone(), d.os)).collect::<Vec<_>>())
        .unwrap_or_default();
    let linux = paired.iter().any(|(.., o)| *o == one_mk_proto::os::LINUX);
    let notes = if input.uses_apps() && linux { vec![WAYLAND_APPS.to_owned()] } else { Vec::new() };
    Some(Tuning { summary: input.summary(), problems: input.check(&paired), notes })
}

pub fn peer_checks(d: &Daemon, paired: &[Paired], seen: &[Seen]) -> Vec<Check> {
    let port = port_of(d);
    paired
        .iter()
        .map(|p| {
            let name = sanitize_name(&p.name);
            let id = format!("peer:{}", p.id);
            let title = format!("Computer \"{name}\"");
            let online = d.status().is_some_and(|s| s.peers.iter().any(|x| x.id == p.id && x.online));
            let found = seen.iter().find(|s| s.id == p.id);
            let out = match (online, found, d.status().is_some()) {
                (true, ..) => Outcome::pass("Connected."),
                (false, Some(f), true) => Outcome::warn(
                    format!("It is on the network ({}) but not connected to this computer.", f.addrs.join(", ")),
                    &format!(
                        "On \"{name}\" run `one-mk doctor` (its firewall may block one-mk's UDP port); \
                         or wait a few seconds: connecting is automatic. You can also try `one-mk run --connect {}`.",
                        f.addrs.first().map_or(format!("<ip>:{port}"), |a| a.clone()),
                    ),
                ),
                (false, Some(f), false) => Outcome::warn(
                    format!("It is on the network ({}), but one-mk is not running here.", f.addrs.join(", ")),
                    "Start one-mk here with `one-mk run`.",
                ),
                (false, None, running) => Outcome::warn(
                    if running {
                        "It is not connected and was not found on the network.".to_owned()
                    } else {
                        "It was not found on the network.".to_owned()
                    },
                    &format!(
                        "Is \"{name}\" switched on, running one-mk and on the same network (not a guest Wi-Fi)? {}",
                        match p.addrs.first() {
                            Some(a) => format!("It was last at {a}: try `one-mk run --connect {a}`."),
                            None => format!("Connect directly with `one-mk run --connect <ip>:{port}`."),
                        }
                    ),
                ),
            };
            Check { id, title, outcome: out }
        })
        .collect()
}

/// Private IPv4 ranges (RFC 1918): what a home or office LAN uses.
pub fn network_check(addrs: &[Addr]) -> Check {
    let title = "Network";
    let out = if let Some(a) = addrs.iter().find(|a| a.ip.is_private()) {
        Outcome::pass(format!("Connected to a private network ({} on {}).", a.ip, a.iface))
    } else if addrs.is_empty() {
        Outcome::fail(
            "This computer has no IPv4 address on any active network.",
            "Connect to your Wi-Fi or plug in a network cable.",
        )
    } else {
        let list: Vec<String> = addrs.iter().map(|a| format!("{} on {}", a.ip, a.iface)).collect();
        Outcome::warn(
            format!(
                "No private (home or office) network address, only {}. This is typical of a guest or public network.",
                list.join(", ")
            ),
            "Join the same normal Wi-Fi or cable network on both computers (not a guest network); \
             on a VPN or 169.254.x.x address, check the router's DHCP.",
        )
    };
    check("network", title, out)
}

/// How the connections to the connected computers are doing right now,
/// from what QUIC measures (the window draws the same over time).
pub fn connection_check(d: &Daemon) -> Check {
    let title = "Connection quality";
    let Some(status) = d.status() else {
        return check("link", title, Outcome::skip("one-mk is not running, so there is no connection to measure."));
    };
    let live: Vec<(&str, one_mk_ctl::status::LinkStats)> =
        status.peers.iter().filter(|p| p.online).filter_map(|p| Some((p.name.as_str(), status.link_of(&p.id)?))).collect();
    if live.is_empty() {
        return check("link", title, Outcome::skip("No computer is connected right now."));
    }
    let shown = |us: u32| crate::manage::LinkStats { rtt_us: us, ..Default::default() }.render().replacen("rtt ", "", 1);
    let slow: Vec<String> = live
        .iter()
        .filter_map(|(name, l)| {
            // A fifty-th of the packets lost, once enough have been sent to say.
            let lossy = l.sent >= 200 && l.lost * 50 >= l.sent;
            match (l.rtt_us > one_mk_ctl::status::LinkStats::STALL_US, lossy) {
                (true, true) => Some(format!("{name} answers in {} and lost {} of {} packets", shown(l.rtt_us), l.lost, l.sent)),
                (true, false) => Some(format!("{name} answers in {}", shown(l.rtt_us))),
                (false, true) => Some(format!("{name} lost {} of {} packets", l.lost, l.sent)),
                (false, false) => None,
            }
        })
        .collect();
    let out = if slow.is_empty() {
        let list: Vec<String> = live.iter().map(|(name, l)| format!("{name} {}", shown(l.rtt_us))).collect();
        Outcome::pass(format!("Answers come back fast: {}.", list.join(", ")))
    } else {
        Outcome::warn(
            format!("{}. Mouse moves may stall (under 40 ms is smooth).", slow.join("; ")),
            "Move closer to the Wi-Fi router, use the 5 GHz band or a network cable, and pause big downloads on this network. The Health page in the window shows how it changes.",
        )
    };
    check("link", title, out)
}

// ------------------------------------------------------------- renderer --

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub pass: usize,
    pub skip: usize,
    pub warn: usize,
    pub fail: usize,
}

pub fn summarize(checks: &[Check]) -> Summary {
    let mut s = Summary::default();
    for c in checks {
        match c.outcome.status {
            Status::Pass => s.pass += 1,
            Status::Skip => s.skip += 1,
            Status::Warn => s.warn += 1,
            Status::Fail => s.fail += 1,
        }
    }
    s
}

pub fn render_text(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        out += &format!("{} {}: {}\n", c.outcome.status.tag(), c.title, c.outcome.detail);
        if let Some(fix) = &c.outcome.fix {
            out += &format!("       Fix: {fix}\n");
        }
    }
    let s = summarize(checks);
    out += &match (s.fail, s.warn) {
        (0, 0) => "\nEverything looks fine. If computers still do not connect, run `one-mk run -v` and read the log.\n"
            .to_owned(),
        (0, w) => format!("\nNo problems found, but {w} thing{} worth a look.\n", if w == 1 { " is" } else { "s are" }),
        (f, _) => format!("\n{f} problem{} found. Fix the [FAIL] lines first.\n", if f == 1 { "" } else { "s" }),
    };
    out
}

pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `{"ok":bool,"summary":{...},"checks":[{"id","title","status","detail","fix"}]}`
/// where status is `pass`, `skip`, `warn` or `fail` and `fix` may be null.
pub fn render_json(checks: &[Check]) -> String {
    let s = summarize(checks);
    let items: Vec<String> = checks
        .iter()
        .map(|c| {
            format!(
                "{{\"id\":{},\"title\":{},\"status\":\"{}\",\"detail\":{},\"fix\":{}}}",
                json_str(&c.id),
                json_str(&c.title),
                c.outcome.status.word(),
                json_str(&c.outcome.detail),
                c.outcome.fix.as_deref().map_or("null".to_owned(), json_str),
            )
        })
        .collect();
    format!(
        "{{\"ok\":{},\"summary\":{{\"pass\":{},\"skip\":{},\"warn\":{},\"fail\":{}}},\"checks\":[{}]}}\n",
        s.fail == 0,
        s.pass,
        s.skip,
        s.warn,
        s.fail,
        items.join(","),
    )
}

// --------------------------------------------------------------- probes --

/// `one-mk doctor`. Exits with status 1 if any check fails.
pub async fn run(dir: &Path, json: bool) -> Result<()> {
    let checks = diagnose(dir).await;
    let text = if json { render_json(&checks) } else { render_text(&checks) };
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
    if summarize(&checks).fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

pub async fn diagnose(dir: &Path) -> Vec<Check> {
    let os = this_os();
    // The daemon and the config tell the other probes what to look for.
    let (daemon, config) = tokio::join!(
        probe_daemon(dir),
        blocking(
            {
                let dir = dir.to_owned();
                move || probe_config(&dir)
            },
            Config { dir_exists: false, has_identity: false, paired: Err("probe failed".into()), settings: Ok(()) },
        )
    );
    let paired = config.paired.clone().unwrap_or_default();
    let port = port_of(&daemon);

    let non_portal = daemon.status().filter(|status| !status.backend.as_str().is_empty() && status.backend.as_str() != "portal");

    // Whose announcements we wait for: this computer's own, and every
    // paired computer that is not connected.
    let mut wanted: Vec<String> = Vec::new();
    if let Some(s) = daemon.status() {
        wanted.push(s.id.clone());
    }
    wanted.extend(
        paired
            .iter()
            .filter(|p| !daemon.status().is_some_and(|s| s.peers.iter().any(|x| x.id == p.id && x.online)))
            .map(|p| p.id.clone()),
    );

    let (port_facts, mdns, firewall, mut portals, addrs, shortcuts) = tokio::join!(
        blocking(move || probe_port(port), PortFacts { port, bind: Bind::Error("probe failed".into()), holder: None },),
        probe_mdns(wanted),
        probe_firewall(os),
        async { if non_portal.is_some() { None } else { probe_portals(os, dir.to_owned()).await } },
        blocking(probe_addrs, Vec::new()),
        probe_shortcuts(os),
    );
    if let (Some(status), Some(portals)) = (daemon.status(), portals.as_mut()) {
        portals.use_running_backend(status.backend);
    }

    let mut checks = vec![daemon_check(&daemon)];
    checks.push(check("drag", "File edge drops", Outcome::skip(one_mk_input::dropzone::availability())));
    checks.extend(config_checks(&config, dir));
    checks.push(port_check(&daemon, &port_facts));
    checks.push(mdns_check(&daemon, &mdns));
    checks.push(firewall_check(os, &firewall, port));
    checks.push(match non_portal {
        Some(status) => check("portals", "Permissions", Outcome::skip(format!("this daemon uses the {} backend", status.backend.as_str()))),
        None => portal_check(os, portals.as_ref(), daemon.status().map_or(&[], |s| &s.input)),
    });
    if let Ok(settings) = crate::settings::Settings::load(dir) {
        if let Some(reason) = &settings.hooks.blocked {
            checks.push(check("hooks", "Hooks", Outcome::fail(format!("Hooks REFUSED: {reason}"), "Fix settings.toml ownership and permissions and remove group/world write access from its folder; alternatively explicitly use `one-mk hooks --trust-permissions`.")));
        }
        checks.push(check("clipboard", "Clipboard", Outcome::pass(settings.clipboard.summary())));
    }
    checks.push(hotkey_check(&daemon, shortcuts.as_ref()));
    checks.push(broadcast_check(&daemon, crate::settings::Settings::load(dir).ok().map(|s| s.broadcast).as_ref(), os));
    checks.push(tuning_check(probe_tuning(dir).as_ref()));
    checks.push(device_rules_check(&probe_device_rules(dir, daemon.status().is_some())));
    checks.push(pointer_overlay_check(
        crate::pointeroverlay::support_here(daemon.status().map(|s| s.backend.as_str())),
        crate::settings::Settings::load(dir).map(|s| s.pointer.draw_when_no_mouse).unwrap_or_default(),
    ));
    if let Ok(settings) = crate::settings::Settings::load(dir) {
        let status = daemon.status();
        let question = status.and_then(|s| s.suggestion.as_ref()).map(|g| one_mk_ctl::status::is_it_there(&g.name, g.side));
        checks.push(edge_push_check(&settings.edge, status.map_or("", |s| s.backend.as_str()), question));
    }
    checks.push(network_check(&addrs));
    checks.push(connection_check(&daemon));
    let seen = mdns.seen.as_deref().unwrap_or_default();
    checks.extend(peer_checks(&daemon, &paired, seen));
    checks
}

/// Runs a blocking probe off the async thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static, fallback: T) -> T {
    tokio::task::spawn_blocking(f).await.unwrap_or(fallback)
}

async fn probe_daemon(dir: &Path) -> Daemon {
    let reply = match tokio::time::timeout(DAEMON_TIMEOUT, control::request(dir, Request::Status)).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("it did not answer in time".to_owned()),
    };
    Daemon::from_reply(reply)
}

fn probe_config(dir: &Path) -> Config {
    let paired = TrustStore::load(dir).map_err(|e| format!("{e:#}")).map(|t| {
        t.devices().iter().map(|d| Paired { id: d.short_id(), name: d.name.clone(), addrs: d.addrs.clone() }).collect()
    });
    Config {
        dir_exists: dir.is_dir(),
        has_identity: dir.join("identity.key.der").is_file() && dir.join("identity.cert.der").is_file(),
        paired,
        settings: Settings::load(dir).map(|_| ()).map_err(|e| one_line(&format!("{e:#}"))),
    }
}

/// A parse error (several lines, quoting the offending text) as one line:
/// where it is and what is wrong.
fn one_line(error: &str) -> String {
    // `toml` draws `1 | text` and `  | ^^^` around the message.
    let quoted = |l: &str| l.split_once('|').is_some_and(|(n, _)| n.trim().is_empty() || n.trim().parse::<u32>().is_ok());
    error.lines().map(str::trim).filter(|l| !l.is_empty() && !quoted(l)).collect::<Vec<_>>().join("; ")
}

fn probe_port(port: u16) -> PortFacts {
    let mut bind = Bind::Free;
    // The daemon binds dual-stack, so either address family collides with it.
    for addr in [format!("0.0.0.0:{port}"), format!("[::]:{port}")] {
        match UdpSocket::bind(&addr) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                bind = Bind::InUse;
                break;
            }
            // No IPv6 on this machine is fine.
            Err(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable || addr.starts_with('[') => {}
            Err(e) => {
                bind = Bind::Error(e.to_string());
                break;
            }
        }
    }
    let holder = if bind == Bind::InUse { udp_holder(port) } else { None };
    PortFacts { port, bind, holder }
}

/// The program holding a UDP port, found via /proc (Linux, own processes only).
fn udp_holder(port: u16) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let inode = ["/proc/net/udp", "/proc/net/udp6"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .find_map(|t| udp_inode_for_port(&t, port))?;
    let want = format!("socket:[{inode}]");
    for pid in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(fds) = std::fs::read_dir(pid.path().join("fd")) else {
            continue;
        };
        if fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|l| l.to_string_lossy() == want)) {
            return std::fs::read_to_string(pid.path().join("comm")).ok().map(|c| c.trim().to_owned());
        }
    }
    None
}

/// The socket inode in a `/proc/net/udp` table for a local `port`.
fn udp_inode_for_port(table: &str, port: u16) -> Option<u64> {
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let local_port = u16::from_str_radix(f.get(1)?.rsplit_once(':')?.1, 16).ok()?;
        (local_port == port).then(|| f.get(9)?.parse().ok()).flatten()
    })
}

async fn probe_mdns(wanted: Vec<String>) -> MdnsFacts {
    let join = blocking(
        || -> Result<(), String> {
            let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
            s.join_multicast_v4(&MDNS_GROUP, &Ipv4Addr::UNSPECIFIED).map_err(|e| e.to_string())
        },
        Err("probe failed".to_owned()),
    );
    // A throwaway identity: browsing never advertises, and this way our own
    // daemon's announcement is not filtered out as "ourselves".
    let seen = async {
        let id = Identity::ephemeral().map_err(|e| format!("{e:#}"))?;
        let (_discovery, mut rx) = Discovery::browse(&id).map_err(|e| format!("{e:#}"))?;
        let mut seen: Vec<Seen> = Vec::new();
        let done = |seen: &[Seen]| wanted.iter().all(|w| seen.iter().any(|s| &s.id == w));
        let _ = tokio::time::timeout(MDNS_WINDOW, async {
            while !done(&seen) {
                let Some(Found { fingerprint, addrs, .. }) = rx.recv().await else {
                    break;
                };
                if addrs.is_empty() {
                    continue; // nothing to connect to yet; a later answer has addresses
                }
                let id = fingerprint.short();
                let addrs = addrs.iter().map(|a| a.to_string());
                match seen.iter_mut().find(|s| s.id == id) {
                    Some(s) => s.addrs.extend(addrs),
                    None => seen.push(Seen { id, addrs: addrs.collect() }),
                }
            }
        })
        .await;
        Ok::<_, String>(seen)
    };
    let (join, seen) = tokio::join!(join, seen);
    MdnsFacts { join, seen }
}

async fn probe_firewall(os: Os) -> Firewall {
    if os != Os::Linux {
        return Firewall::default();
    }
    blocking(
        || {
            let ufw_active = match std::fs::read_to_string("/etc/ufw/ufw.conf") {
                Ok(c) => Some(c.lines().any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes"))),
                // No such file: ufw is not installed, so it is not on. Anything
                // else (permissions): ask systemd, which anyone may.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
                Err(_) => run_with_timeout("systemctl", &["is-active", "ufw"]).map(|o| o.trim() == "active"),
            };
            let ufw_rules =
                (ufw_active == Some(true)).then(|| std::fs::read_to_string("/etc/ufw/user.rules").ok()).flatten();
            let firewalld = run_with_timeout("firewall-cmd", &["--state"])
                .filter(|s| s.trim() == "running")
                .and_then(|_| run_with_timeout("firewall-cmd", &["--list-all"]))
                .map(|all| Firewalld { ports: field(&all, "ports:"), services: field(&all, "services:") });
            Firewall { ufw_active, ufw_rules, firewalld }
        },
        Firewall::default(),
    )
    .await
}

/// The value of a `key: value` line in `firewall-cmd --list-all` output.
fn field(all: &str, key: &str) -> String {
    all.lines().find_map(|l| l.trim().strip_prefix(key)).unwrap_or_default().trim().to_owned()
}

/// Output of a command, or `None` if it is missing, fails or takes too long.
fn run_with_timeout(program: &str, args: &[&str]) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(st) if st.success() => break,
            Some(_) => return None,
            None if start.elapsed() > COMMAND_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

#[cfg(target_os = "linux")]
async fn probe_portals(os: Os, dir: std::path::PathBuf) -> Option<Portals> {
    if os != Os::Linux {
        return None;
    }
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland = session == "wayland" || (session.is_empty() && std::env::var_os("WAYLAND_DISPLAY").is_some());
    let probe = tokio::time::timeout(PORTAL_TIMEOUT, one_mk_input::probe_portals()).await;
    let (remote_desktop, input_capture) = match probe {
        Ok(p) => (Some(p.remote_desktop), Some(p.input_capture)),
        Err(_) => (None, None),
    };
    let portals_ok = match (&remote_desktop, &input_capture) {
        (Some(rd), Some(ic)) => Some(rd.is_ok() && ic.is_ok()),
        _ => None,
    };
    let (backend, why) = one_mk_input::choose_linux_backend(&one_mk_input::session_facts(portals_ok).await);
    let token = dir.join("remote-desktop.token").is_file();
    Some(Portals { wayland, remote_desktop, input_capture, token, backend: Some((backend.name().to_owned(), why)) })
}

#[cfg(target_os = "linux")]
async fn probe_shortcuts(os: Os) -> Option<Result<u32, String>> {
    if os != Os::Linux {
        return None;
    }
    let probe = tokio::time::timeout(PORTAL_TIMEOUT, one_mk_input::hotkeys::probe()).await;
    Some(probe.unwrap_or_else(|_| Err("the desktop portal did not answer in time".into())))
}

#[cfg(not(target_os = "linux"))]
async fn probe_shortcuts(_os: Os) -> Option<Result<u32, String>> {
    None
}

#[cfg(not(target_os = "linux"))]
async fn probe_portals(_os: Os, _dir: std::path::PathBuf) -> Option<Portals> {
    None
}

fn probe_addrs() -> Vec<Addr> {
    let all = if_addrs::get_if_addrs().unwrap_or_default();
    usable_addrs(all.iter().filter(|i| i.is_oper_up()).filter_map(|i| match &i.addr {
        if_addrs::IfAddr::V4(a) => Some((i.name.as_str(), a.ip)),
        _ => None,
    }))
}

/// Addresses that could reach another computer: not loopback, not a
/// container or VM bridge.
fn usable_addrs<'a>(all: impl Iterator<Item = (&'a str, Ipv4Addr)>) -> Vec<Addr> {
    all.filter(|(name, ip)| !ip.is_loopback() && !ip.is_unspecified() && !is_virtual_interface(name))
        .map(|(name, ip)| Addr { iface: name.to_owned(), ip })
        .collect()
}

// ---------------------------------------------------------------- tests --

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::Side;

    fn status(id: &str, port: u16, peers: &[(&str, bool)]) -> DaemonStatus {
        DaemonStatus {
            name: "desk".into(),
            id: id.into(),
            port,
            controlling: None,
            controlled_by: None,
            peers: peers
                .iter()
                .map(|(pid, online)| crate::manage::PeerStatus {
                    id: (*pid).into(),
                    name: "laptop".into(),
                    side: Side::Right,
                    os: 0,
                    online: *online,
                    swap: "auto".into(),
                    screens: vec![],
                })
                .collect(),
            ..DaemonStatus::default()
        }
    }

    fn connected(links: &[(&str, u32, u64, u64)]) -> Daemon {
        let mut s = status("aaaa-aaaa", 24900, &links.iter().map(|l| (l.0, true)).collect::<Vec<_>>());
        s.links = links.iter().map(|(id, rtt_us, lost, sent)| ((*id).into(), one_mk_ctl::status::LinkStats { rtt_us: *rtt_us, lost: *lost, sent: *sent })).collect();
        Daemon::Running(Box::new(s))
    }

    #[test]
    fn connection_quality_reads_what_quic_measured() {
        let c = connection_check(&Daemon::NotRunning);
        assert_eq!(st(&c), Status::Skip);
        assert_eq!(st(&connection_check(&running())), Status::Skip);
        let c = connection_check(&connected(&[("1111-1111", 4_200, 0, 5000)]));
        assert_eq!(st(&c), Status::Pass);
        assert!(c.outcome.detail.contains("laptop 4 ms"), "{}", c.outcome.detail);
        // Late answers, loss, or both warn, and say which computer.
        let c = connection_check(&connected(&[("1111-1111", 85_000, 0, 5000)]));
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.detail.contains("laptop answers in 85 ms") && c.outcome.fix.unwrap().contains("5 GHz"), "{}", c.outcome.detail);
        let c = connection_check(&connected(&[("1111-1111", 4_000, 150, 5000)]));
        assert!(c.outcome.detail.contains("lost 150 of 5000 packets"), "{}", c.outcome.detail);
        let c = connection_check(&connected(&[("1111-1111", 90_000, 150, 5000)]));
        assert!(c.outcome.detail.contains("answers in 90 ms and lost 150"), "{}", c.outcome.detail);
        // 40 ms is the edge, and a handful of packets in the first seconds says nothing.
        assert_eq!(st(&connection_check(&connected(&[("1111-1111", 40_000, 3, 100)]))), Status::Pass);
        assert_eq!(st(&connection_check(&connected(&[("1111-1111", 40_001, 0, 100)]))), Status::Warn);
        // A computer that is not connected has no link to judge.
        let mut s = status("aaaa-aaaa", 24900, &[("1111-1111", false)]);
        s.links = vec![("1111-1111".into(), one_mk_ctl::status::LinkStats { rtt_us: 900_000, lost: 9, sent: 10 })];
        assert_eq!(st(&connection_check(&Daemon::Running(Box::new(s)))), Status::Skip);
    }

    fn running() -> Daemon {
        Daemon::Running(Box::new(status("aaaa-aaaa", 24900, &[])))
    }

    fn st(c: &Check) -> Status {
        c.outcome.status
    }

    #[test]
    fn edge_push_states() {
        use crate::settings::{EdgeSettings, Learn, Wrap};
        let edge = |wrap, learn| EdgeSettings { wrap, learn, ..EdgeSettings::default() };
        assert_eq!(st(&edge_push_check(&edge(Wrap::Off, Learn::Off), "x11", None)), Status::Skip);
        let default = edge_push_check(&EdgeSettings::default(), "portal", Some("Is B on your left?".into()));
        assert_eq!(st(&default), Status::Pass, "suggesting only hands the pointer back");
        assert!(default.outcome.detail.contains("Not yet tested on the Wayland desktop portals"), "{}", default.outcome.detail);
        assert!(default.outcome.detail.contains("Asking now: Is B on your left?"));
        let wrap = edge_push_check(&edge(Wrap::On, Learn::Suggest), "wlroots", None);
        assert_eq!(st(&wrap), Status::Warn);
        assert!(wrap.outcome.fix.as_deref().is_some_and(|f| f.contains("one-mk edge --wrap off")));
        assert_eq!(st(&edge_push_check(&edge(Wrap::All, Learn::Auto), "x11", None)), Status::Pass);
        assert_eq!(st(&edge_push_check(&edge(Wrap::Off, Learn::Auto), "native", None)), Status::Warn);
        assert_eq!(st(&edge_push_check(&edge(Wrap::On, Learn::Auto), "", None)), Status::Pass, "not running: nothing to say");
    }

    #[test]
    fn daemon_states() {
        assert_eq!(st(&daemon_check(&running())), Status::Pass);
        let down = daemon_check(&Daemon::NotRunning);
        assert_eq!(st(&down), Status::Fail);
        assert!(down.outcome.fix.unwrap().contains("one-mk run"));
        assert_eq!(st(&daemon_check(&Daemon::Broken("x".into()))), Status::Fail);
    }

    #[test]
    fn daemon_from_reply() {
        assert_eq!(Daemon::from_reply(Ok(None)), Daemon::NotRunning);
        assert!(matches!(Daemon::from_reply(Err("boom".into())), Daemon::Broken(_)));
        assert!(matches!(Daemon::from_reply(Ok(Some("garbage".into()))), Daemon::Broken(_)));
        let s = status("aaaa-aaaa", 24900, &[("bbbb-bbbb", true)]);
        assert_eq!(Daemon::from_reply(Ok(Some(s.encode()))), Daemon::Running(Box::new(s)));
    }

    fn cfg(dir: bool, id: bool, paired: Result<Vec<Paired>, String>) -> Vec<Check> {
        config_checks(&Config { dir_exists: dir, has_identity: id, paired, settings: Ok(()) }, Path::new("/x/one-mk"))
    }

    fn paired(id: &str) -> Paired {
        Paired { id: id.into(), name: "laptop".into(), addrs: vec![] }
    }

    #[test]
    fn config_fresh_machine_is_not_paired() {
        let c = cfg(false, false, Ok(vec![]));
        assert_eq!(c.iter().map(st).collect::<Vec<_>>(), [Status::Warn, Status::Skip, Status::Fail]);
        assert!(c[2].outcome.fix.as_ref().unwrap().contains("one-mk pair"));
    }

    #[test]
    fn config_identity_without_peers() {
        let c = cfg(true, true, Ok(vec![]));
        assert_eq!(c.iter().map(st).collect::<Vec<_>>(), [Status::Pass, Status::Pass, Status::Fail]);
        let c = cfg(true, false, Ok(vec![]));
        assert_eq!(st(&c[1]), Status::Warn);
    }

    #[test]
    fn a_settings_file_that_does_not_parse_is_reported() {
        let c = |settings| {
            let cfg = Config { dir_exists: true, has_identity: true, paired: Ok(vec![]), settings };
            config_checks(&cfg, Path::new("/x/one-mk")).remove(0)
        };
        assert_eq!(st(&c(Ok(()))), Status::Pass);
        let bad = c(Err("reading /x/one-mk/settings.toml: TOML parse error at line 1, column 13; unclosed array".into()));
        assert_eq!(st(&bad), Status::Warn);
        assert!(bad.outcome.detail.contains("TOML parse error at line 1"), "{}", bad.outcome.detail);
        let fix = bad.outcome.fix.unwrap();
        assert!(fix.contains("delete") && fix.contains("settings.toml"), "{fix}");
    }

    /// The real probe: a corrupt file in a real folder.
    #[test]
    fn probing_a_corrupt_settings_file() {
        let dir = std::env::temp_dir().join(format!("one-mk-doctor-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(probe_config(&dir).settings, Ok(()), "no file is fine");
        std::fs::write(dir.join("settings.toml"), "[clipboard]\nmax_mb = 5\n").unwrap();
        assert_eq!(probe_config(&dir).settings, Ok(()));
        std::fs::write(dir.join("settings.toml"), "gaming = 7\n[[[").unwrap();
        let e = probe_config(&dir).settings.unwrap_err();
        assert!(e.contains("settings.toml") && e.contains("line 2") && !e.contains('\n'), "{e}");
        assert_eq!(st(&config_checks(&probe_config(&dir), &dir)[0]), Status::Warn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_paired_and_corrupt() {
        let c = cfg(true, true, Ok(vec![paired("bbbb-bbbb"), paired("cccc-cccc")]));
        assert_eq!(st(&c[2]), Status::Pass);
        assert!(c[2].outcome.detail.contains("2 computers"));
        assert_eq!(st(&cfg(true, true, Err("bad toml".into()))[2]), Status::Fail);
    }

    fn pf(bind: Bind, holder: Option<&str>) -> PortFacts {
        PortFacts { port: 24900, bind, holder: holder.map(str::to_owned) }
    }

    #[test]
    fn port_decisions() {
        assert_eq!(st(&port_check(&running(), &pf(Bind::InUse, None))), Status::Pass);
        assert_eq!(st(&port_check(&running(), &pf(Bind::Free, None))), Status::Fail);
        assert_eq!(st(&port_check(&Daemon::NotRunning, &pf(Bind::Free, None))), Status::Pass);
        let other = port_check(&Daemon::NotRunning, &pf(Bind::InUse, Some("deskflow")));
        assert_eq!(st(&other), Status::Fail);
        assert!(other.outcome.detail.contains("deskflow"));
        assert!(other.outcome.fix.unwrap().contains("--port"));
        assert_eq!(st(&port_check(&Daemon::NotRunning, &pf(Bind::InUse, None))), Status::Fail);
        assert_eq!(st(&port_check(&running(), &pf(Bind::Error("denied".into()), None))), Status::Warn);
    }

    #[test]
    fn port_holder_from_proc_table() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops\n\
                     1234: 00000000:6144 00000000:0000 07 00000000:00000000 00:00000000 00000000  1000        0 987654 2 0000000000000000 0\n\
                     1235: 0100007F:0035 00000000:0000 07 00000000:00000000 00:00000000 00000000   101        0 111 2 0000000000000000 0\n";
        assert_eq!(udp_inode_for_port(table, 24900), Some(987654));
        assert_eq!(udp_inode_for_port(table, 53), Some(111));
        assert_eq!(udp_inode_for_port(table, 80), None);
    }

    fn mdns(join: Result<(), &str>, seen: Result<Vec<Seen>, &str>) -> MdnsFacts {
        MdnsFacts { join: join.map_err(str::to_owned), seen: seen.map_err(str::to_owned) }
    }

    fn seen(id: &str) -> Seen {
        Seen { id: id.into(), addrs: vec!["192.168.1.5:24900".into()] }
    }

    #[test]
    fn mdns_decisions() {
        let m = mdns(Err("no route"), Ok(vec![]));
        assert_eq!(st(&mdns_check(&running(), &m)), Status::Fail);
        let m = mdns(Ok(()), Err("no socket"));
        assert_eq!(st(&mdns_check(&running(), &m)), Status::Fail);
        // Daemon up and its own announcement seen.
        let m = mdns(Ok(()), Ok(vec![seen("aaaa-aaaa")]));
        assert_eq!(st(&mdns_check(&running(), &m)), Status::Pass);
        // Daemon up, nothing seen: no discovery or blocked multicast.
        let m = mdns(Ok(()), Ok(vec![seen("bbbb-bbbb")]));
        let c = mdns_check(&running(), &m);
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.fix.unwrap().contains("--no-discovery"));
        // No daemon: multicast working is enough.
        let m = mdns(Ok(()), Ok(vec![]));
        assert_eq!(st(&mdns_check(&Daemon::NotRunning, &m)), Status::Pass);
    }

    #[test]
    fn port_lists() {
        assert!(port_listed("24900/udp", 24900));
        assert!(port_listed("80/tcp 24000-25000/udp", 24900));
        assert!(!port_listed("24900/tcp", 24900));
        assert!(!port_listed("24901/udp 5353/udp", 24900));
        assert!(!port_listed("", 24900));
        assert!(!port_listed("junk 1-x/udp", 24900));
    }

    #[test]
    fn ufw_rule_parsing() {
        let rules = "-A ufw-user-input -p tcp --dport 22 -j ACCEPT\n\
                     -A ufw-user-input -p udp --dport 24900 -j ACCEPT\n";
        assert!(ufw_allows(rules, 24900));
        assert!(!ufw_allows(rules, 22));
        assert!(!ufw_allows("-A ufw-user-input -p udp --dport 24900 -j DROP\n", 24900));
        assert!(ufw_allows("-A ufw-user-input -p udp -m multiport --dports 5000:25000,30000 -j ACCEPT\n", 24900));
        assert!(!ufw_allows("", 24900));
    }

    fn fw(ufw: Option<bool>, rules: Option<&str>, firewalld: Option<(&str, &str)>) -> Firewall {
        Firewall {
            ufw_active: ufw,
            ufw_rules: rules.map(str::to_owned),
            firewalld: firewalld.map(|(p, s)| Firewalld { ports: p.into(), services: s.into() }),
        }
    }

    #[test]
    fn firewall_none_active_passes() {
        assert_eq!(st(&firewall_check(Os::Linux, &fw(Some(false), None, None), 24900)), Status::Pass);
    }

    #[test]
    fn firewall_unknown_degrades_to_warn() {
        let c = firewall_check(Os::Linux, &fw(None, None, None), 24900);
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.fix.unwrap().contains("24900"));
        // ufw on but rules unreadable (no root): "couldn't tell", never a failure.
        let c = firewall_check(Os::Linux, &fw(Some(true), None, None), 24900);
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.fix.unwrap().contains("sudo ufw allow 24900/udp"));
    }

    #[test]
    fn firewall_ufw_rules_known() {
        let allow = "-A ufw-user-input -p udp --dport 24900 -j ACCEPT\n";
        assert_eq!(st(&firewall_check(Os::Linux, &fw(Some(true), Some(allow), None), 24900)), Status::Pass);
        let c = firewall_check(
            Os::Linux,
            &fw(Some(true), Some("-A ufw-user-input -p tcp --dport 22 -j ACCEPT\n"), None),
            24900,
        );
        assert_eq!(st(&c), Status::Fail);
        assert!(c.outcome.fix.unwrap().contains("ufw allow 24900/udp"));
    }

    #[test]
    fn firewall_firewalld() {
        let ok = firewall_check(Os::Linux, &fw(Some(false), None, Some(("24900/udp", "ssh mdns"))), 24900);
        assert_eq!(st(&ok), Status::Pass);
        let no_port = firewall_check(Os::Linux, &fw(Some(false), None, Some(("", "ssh mdns"))), 24900);
        assert_eq!(st(&no_port), Status::Fail);
        let fix = no_port.outcome.fix.unwrap();
        assert!(fix.contains("--add-port=24900/udp") && !fix.contains("mdns"));
        let no_mdns = firewall_check(Os::Linux, &fw(Some(false), None, Some(("1025-65535/udp", "ssh"))), 24900);
        assert_eq!(st(&no_mdns), Status::Pass); // 5353 is inside the range
        let nothing = firewall_check(Os::Linux, &fw(Some(false), None, Some(("", ""))), 24900);
        let fix = nothing.outcome.fix.unwrap();
        assert!(fix.contains("--add-port=24900/udp") && fix.contains("--add-service=mdns"));
    }

    #[test]
    fn firewall_other_os_is_a_hint() {
        for os in [Os::Windows, Os::Mac, Os::Other] {
            let c = firewall_check(os, &Firewall::default(), 24900);
            assert_eq!(st(&c), Status::Skip);
            assert!(c.outcome.fix.is_none());
        }
        assert!(firewall_check(Os::Windows, &Firewall::default(), 24900).outcome.detail.contains("Windows"));
    }

    #[test]
    fn firewalld_list_all_fields() {
        let all = "public (default, active)\n  target: default\n  services: dhcpv6-client ssh\n  ports: 24900/udp 80/tcp\n  protocols:\n";
        assert_eq!(field(all, "ports:"), "24900/udp 80/tcp");
        assert_eq!(field(all, "services:"), "dhcpv6-client ssh");
        assert_eq!(field(all, "missing:"), "");
    }

    fn portals(rd: Option<Result<(), &str>>, ic: Option<Result<(), &str>>, wayland: bool, token: bool) -> Portals {
        Portals {
            wayland,
            remote_desktop: rd.map(|r| r.map_err(str::to_owned)),
            input_capture: ic.map(|r| r.map_err(str::to_owned)),
            token,
            backend: None,
        }
    }

    #[test]
    fn running_backend_overrides_session_inference() {
        for name in ["null", "native", "portal", "x11", "wlroots"] {
            let mut facts = portals(None, None, true, false);
            facts.backend = Some(("x11".into(), "inferred".into()));
            facts.use_running_backend(name.into());
            assert_eq!(facts.backend.as_ref().unwrap().0, name);
            assert!(facts.backend.as_ref().unwrap().1.contains("running daemon"));
        }
    }

    #[test]
    fn hotkey_decisions() {
        let with = |state: &str, detail: &str| {
            let mut s = status("aaaa-aaaa", 24900, &[]);
            (s.hotkeys, s.hotkeys_detail) = (state.into(), detail.into());
            hotkey_check(&Daemon::Running(Box::new(s)), None)
        };
        assert_eq!(st(&with("on", "through Windows")), Status::Pass);
        let c = with("unavailable", "this desktop has no GlobalShortcuts portal");
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.fix.as_deref().unwrap().starts_with("Bind `one-mk switch next`"), "{c:?}");
        assert_eq!(st(&with("off", "")), Status::Skip);
        assert_eq!(st(&with("off", "settings.toml: \"hyper+x\" is not a shortcut")), Status::Warn);
        assert_eq!(st(&with("", "")), Status::Skip, "an older daemon");
        // Not running: the desktop is asked (Linux), or nothing is said.
        assert_eq!(st(&hotkey_check(&Daemon::NotRunning, Some(&Ok(1)))), Status::Pass);
        assert_eq!(st(&hotkey_check(&Daemon::NotRunning, Some(&Err("no portal".into())))), Status::Warn);
        assert_eq!(st(&hotkey_check(&Daemon::NotRunning, None)), Status::Skip);
    }

    #[test]
    fn broadcast_decisions() {
        let base = |f: &dyn Fn(&mut DaemonStatus)| {
            let mut s = status("aaaa-aaaa", 24900, &[]);
            f(&mut s);
            Daemon::Running(Box::new(s))
        };
        let b = crate::settings::BroadcastSettings::default();
        let off = broadcast_check(&base(&|s| s.broadcast_hotkey = "Ctrl+Alt+B".into()), Some(&b), Os::Linux);
        assert_eq!(st(&off), Status::Pass);
        assert!(off.outcome.detail.contains("Ctrl+Alt+B or `one-mk broadcast on` starts it; it stops by itself after 15 minutes"), "{off:?}");
        let none = broadcast_check(&base(&|_| {}), Some(&b), Os::Linux);
        assert!(none.outcome.detail.contains("No shortcut starts it"), "{none:?}");
        let on = broadcast_check(&base(&|s| s.broadcast = vec!["bee".into(), "sea".into()]), Some(&b), Os::Linux);
        assert_eq!(st(&on), Status::Warn);
        assert!(on.outcome.detail.contains("goes to 2 (bee, sea)") && on.outcome.fix.as_deref().unwrap().contains("broadcast off"), "{on:?}");
        // Untested where it has not been tried.
        for os in [Os::Windows, Os::Mac] {
            assert!(broadcast_check(&base(&|_| {}), Some(&b), os).outcome.detail.contains("untested"));
        }
        let portal = broadcast_check(&base(&|s| s.backend = "portal".into()), Some(&b), Os::Linux);
        assert!(portal.outcome.detail.contains("untested"), "{portal:?}");
        assert!(!off.outcome.detail.contains("untested"));
        assert_eq!(st(&broadcast_check(&Daemon::NotRunning, None, Os::Linux)), Status::Skip);
    }

    #[test]
    fn portal_decisions() {
        let ok = Some(Ok(()));
        let c = portal_check(Os::Linux, Some(&portals(ok, ok, true, true)), &[]);
        assert_eq!(st(&c), Status::Pass);
        let c = portal_check(Os::Linux, Some(&portals(ok, ok, true, false)), &[]);
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.fix.unwrap().contains("one-mk run"));
        let c = portal_check(Os::Linux, Some(&portals(Some(Err("not found")), ok, true, true)), &[]);
        assert_eq!(st(&c), Status::Fail);
        let c = portal_check(Os::Linux, Some(&portals(ok, Some(Err("no")), true, true)), &[]);
        assert_eq!(st(&c), Status::Fail);
        let c = portal_check(Os::Linux, Some(&portals(None, None, true, true)), &[]);
        assert_eq!(st(&c), Status::Warn);
        let c = portal_check(Os::Linux, Some(&portals(ok, ok, false, true)), &[]);
        assert_eq!(st(&c), Status::Warn);
        assert_eq!(st(&portal_check(Os::Windows, None, &[])), Status::Skip);
        assert_eq!(st(&portal_check(Os::Mac, None, &[])), Status::Skip);
        // What the running daemon reports comes first, unless the desktop
        // can't do it at all.
        let waiting = ["Waiting for your approval: allow one-mk in the desktop's dialog.".to_owned()];
        let c = portal_check(Os::Linux, Some(&portals(ok, ok, true, true)), &waiting);
        assert_eq!(st(&c), Status::Warn);
        assert!(c.outcome.detail.contains("Waiting for your approval"), "{}", c.outcome.detail);
        let c = portal_check(Os::Linux, Some(&portals(Some(Err("not found")), ok, true, true)), &waiting);
        assert_eq!(st(&c), Status::Fail);
        // Which backend one-mk picks, and why.
        let with = |name: &str, rd: Option<Result<(), &str>>| {
            let mut p = portals(rd, rd, false, false);
            p.backend = Some((name.into(), "the reason".into()));
            portal_check(Os::Linux, Some(&p), &[])
        };
        let c = with("x11", Some(Err("not found")));
        assert_eq!(st(&c), Status::Pass, "X11 needs no portal");
        assert!(c.outcome.detail.contains("X11 backend: the reason"), "{}", c.outcome.detail);
        let c = with("wlroots", None);
        assert_eq!(st(&c), Status::Pass);
        assert!(c.outcome.detail.contains("wlroots backend: the reason"), "{}", c.outcome.detail);
        let c = with("portal", Some(Err("not found")));
        assert_eq!(st(&c), Status::Fail);
        assert!(c.outcome.detail.starts_with("one-mk uses the desktop portals (the reason)."), "{}", c.outcome.detail);
        assert!(c.outcome.fix.unwrap().contains("Sway"));
    }

    #[test]
    fn peer_decisions() {
        let paired = vec![paired("bbbb-bbbb")];
        let up = Daemon::Running(Box::new(status("aaaa-aaaa", 24900, &[("bbbb-bbbb", true)])));
        let c = peer_checks(&up, &paired, &[]);
        assert_eq!(st(&c[0]), Status::Pass);
        assert_eq!(c[0].id, "peer:bbbb-bbbb");

        let down = Daemon::Running(Box::new(status("aaaa-aaaa", 24900, &[("bbbb-bbbb", false)])));
        let c = peer_checks(&down, &paired, &[seen("bbbb-bbbb")]);
        assert_eq!(st(&c[0]), Status::Warn);
        assert!(c[0].outcome.detail.contains("192.168.1.5:24900"));
        assert!(c[0].outcome.fix.as_ref().unwrap().contains("firewall"));

        let c = peer_checks(&down, &paired, &[]);
        assert_eq!(st(&c[0]), Status::Warn);
        assert!(c[0].outcome.detail.contains("not found"));

        let c = peer_checks(&Daemon::NotRunning, &paired, &[seen("bbbb-bbbb")]);
        assert!(c[0].outcome.fix.as_ref().unwrap().contains("one-mk run"));

        let mut with_addr = paired.clone();
        with_addr[0].addrs = vec!["10.0.0.7:24900".into()];
        let c = peer_checks(&down, &with_addr, &[]);
        assert!(c[0].outcome.fix.as_ref().unwrap().contains("--connect 10.0.0.7:24900"));
        assert!(peer_checks(&down, &[], &[]).is_empty());
    }

    /// Advice to connect directly names the port one-mk really uses.
    #[test]
    fn connect_advice_uses_the_real_port() {
        let on = |port| Daemon::Running(Box::new(status("aaaa-aaaa", port, &[("bbbb-bbbb", false)])));
        let fixes = |d: &Daemon| {
            let mut all = vec![mdns_check(d, &mdns(Ok(()), Ok(vec![seen("bbbb-bbbb")]))), mdns_check(d, &mdns(Err("no"), Ok(vec![])))];
            all.extend(peer_checks(d, &[paired("bbbb-bbbb")], &[]));
            let fixes: Vec<_> = all.into_iter().filter_map(|c| c.outcome.fix).collect();
            assert!(fixes.len() >= 2);
            fixes
        };
        for fix in fixes(&on(24955)) {
            assert!(!fix.contains("24900"), "{fix}");
            assert!(fix.contains("<ip>:24955"), "{fix}");
        }
        for fix in fixes(&Daemon::NotRunning) {
            assert!(fix.contains("<ip>:24900"), "{fix}");
        }
    }

    fn addr(iface: &str, ip: [u8; 4]) -> Addr {
        Addr { iface: iface.into(), ip: Ipv4Addr::from(ip) }
    }

    #[test]
    fn network_decisions() {
        let c = network_check(&[addr("wlan0", [192, 168, 1, 20])]);
        assert_eq!(st(&c), Status::Pass);
        assert_eq!(st(&network_check(&[addr("eth0", [10, 1, 2, 3])])), Status::Pass);
        assert_eq!(st(&network_check(&[addr("eth0", [172, 16, 0, 9])])), Status::Pass);
        let guest = network_check(&[addr("wlan0", [100, 64, 3, 3])]);
        assert_eq!(st(&guest), Status::Warn);
        assert!(guest.outcome.detail.contains("100.64.3.3"));
        assert_eq!(st(&network_check(&[addr("wlan0", [169, 254, 1, 1])])), Status::Warn);
        assert_eq!(st(&network_check(&[])), Status::Fail);
    }

    #[test]
    fn usable_addrs_skip_loopback_and_bridges() {
        let all = [
            ("lo", Ipv4Addr::LOCALHOST),
            ("docker0", Ipv4Addr::new(172, 17, 0, 1)),
            ("wlan0", Ipv4Addr::new(192, 168, 1, 4)),
        ];
        assert_eq!(usable_addrs(all.into_iter()), [addr("wlan0", [192, 168, 1, 4])]);
    }

    #[test]
    fn render_text_lists_fixes_and_summary() {
        let checks = vec![
            check("a", "Fine", Outcome::pass("all good")),
            check("b", "Broken", Outcome::fail("it broke", "turn it on")),
        ];
        let t = render_text(&checks);
        assert!(t.contains("[ ok ] Fine: all good"));
        assert!(t.contains("[FAIL] Broken: it broke\n       Fix: turn it on\n"));
        assert!(t.contains("1 problem found"));
        let fine = render_text(&checks[..1]);
        assert!(fine.contains("Everything looks fine"));
        let warn = render_text(&[check("w", "W", Outcome::warn("hm", "look"))]);
        assert!(warn.contains("No problems found, but 1 thing is worth a look"));
    }

    #[test]
    fn render_json_escapes_and_reports() {
        let checks = vec![
            check("a", "Say \"hi\"", Outcome::pass("line\nbreak\\ \u{1}")),
            check("b", "B", Outcome::fail("d", "fix it")),
        ];
        let j = render_json(&checks);
        assert!(j.starts_with("{\"ok\":false,\"summary\":{\"pass\":1,\"skip\":0,\"warn\":0,\"fail\":1},\"checks\":["));
        assert!(j.contains(r#""title":"Say \"hi\"""#));
        assert!(j.contains(r#""detail":"line\nbreak\\ \u0001""#));
        assert!(j.contains(r#""status":"pass","detail":"line\nbreak\\ \u0001","fix":null"#));
        assert!(j.contains(r#""status":"fail","detail":"d","fix":"fix it""#));
        assert!(render_json(&checks[..1]).starts_with("{\"ok\":true"));
    }

    #[test]
    fn summary_counts() {
        let checks = vec![
            check("a", "", Outcome::pass("")),
            check("b", "", Outcome::skip("")),
            check("c", "", Outcome::warn("", "")),
            check("d", "", Outcome::fail("", "")),
        ];
        assert_eq!(summarize(&checks), Summary { pass: 1, skip: 1, warn: 1, fail: 1 });
    }

    #[test]
    fn input_tuning() {
        let t = |summary: Option<&str>, problems: &[&str]| Tuning {
            summary: summary.map(str::to_owned),
            problems: problems.iter().map(|p| p.to_string()).collect(),
            notes: Vec::new(),
        };
        assert_eq!(st(&tuning_check(None)), Status::Skip);
        assert_eq!(st(&tuning_check(Some(&t(None, &[])))), Status::Skip);
        let ok = tuning_check(Some(&t(Some("Input tuning: 1 key rule."), &[])));
        assert_eq!((st(&ok), ok.id.as_str()), (Status::Pass, "input-tuning"));
        let bad = tuning_check(Some(&t(Some("x"), &["a: conflict", "b: typo"])));
        assert_eq!(st(&bad), Status::Warn);
        assert_eq!(bad.outcome.detail, "a: conflict; b: typo.");
        // From a real settings folder.
        let dir = std::env::temp_dir().join(format!("one-mk-doctor-tuning-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.toml"), "[[input.keymap.rule]]\nfrom = \"capslok\"\nto = \"esc\"\n").unwrap();
        let found = probe_tuning(&dir).unwrap();
        assert!(found.problems[0].contains("\"capslok\" is not a key name"), "{found:?}");
        assert!(found.notes.is_empty());
        // Rules for apps, and a Linux computer paired: where they can't see.
        let with_app = Tuning { notes: vec![WAYLAND_APPS.into()], ..t(Some("Input tuning: 1 key rule."), &[]) };
        let noted = tuning_check(Some(&with_app));
        assert_eq!(st(&noted), Status::Pass);
        assert!(noted.outcome.detail.contains("Wayland window has the focus"), "{}", noted.outcome.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn per_device_rules_check() {
        let d = |support: Result<Option<String>, String>, rules| DeviceRules { support, seen: 3, connected: 2, rules };
        let ok = device_rules_check(&d(Ok(None), false));
        assert_eq!((st(&ok), ok.id.as_str()), (Status::Pass, "per-device-rules"));
        assert!(ok.outcome.detail.contains("3 seen, 2 connected now"), "{:?}", ok.outcome.detail);
        let untested = device_rules_check(&d(Ok(Some("untested on real Windows hardware".into())), true));
        assert_eq!(st(&untested), Status::Pass);
        assert!(untested.outcome.detail.contains("untested on real Windows hardware"));
        let merged = one_mk_input::MERGED_DEVICES.to_owned();
        let skip = device_rules_check(&d(Err(merged.clone()), false));
        assert_eq!(st(&skip), Status::Skip);
        assert!(skip.outcome.detail.starts_with("Per-device rules are not available on this desktop: the desktop merges all keyboards and mice"), "{}", skip.outcome.detail);
        let warn = device_rules_check(&d(Err(merged), true));
        assert_eq!(st(&warn), Status::Warn);
        assert!(warn.outcome.fix.as_deref().unwrap_or_default().contains("--device"));
    }

}
