//! The window's link to one-mk: it watches the daemon over the local
//! control channel and changes things through the `one-mk` program, the
//! same way the command line and the tray do. Every slow step runs on its
//! own thread and reports back as an [`Event`], waking the window.

use crate::view::{self, Check, Prefs};
use one_mk_ctl::client::{self, Request, SwitchTo, Toggle};
use one_mk_ctl::install::{self, Launch};
use one_mk_ctl::proc;
use one_mk_ctl::status::Status;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the window looks at the daemon while it is open. A status
/// request is one small message over a local socket.
const POLL: Duration = Duration::from_millis(250);

/// Whether a graph of the connections is on screen, so the window asks the
/// daemon for round-trip times only then. The window stamps it on every
/// frame it draws one; a window that is minimised or covered stops drawing,
/// the stamp goes stale and nothing more is asked.
#[derive(Clone)]
pub struct Showing {
    /// Milliseconds since `epoch` at the last frame with a graph; 0: none.
    stamp: Arc<AtomicU64>,
    epoch: Instant,
}

/// A stamp older than this says the graph is no longer on screen.
const SHOWING_FOR: Duration = Duration::from_millis(3000);
/// How often the connections are sampled while a graph is on screen.
const SAMPLE: Duration = Duration::from_millis((crate::rtt::EVERY * 1000.0) as u64);

impl Showing {
    pub fn new() -> Self {
        Self { stamp: Arc::default(), epoch: Instant::now() }
    }

    fn ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    /// A frame was drawn with (`true`) or without a graph.
    pub fn frame(&self, graph: bool) {
        self.stamp.store(if graph { self.ms() } else { 0 }, Ordering::Relaxed);
    }

    fn on_screen(&self) -> bool {
        let at = self.stamp.load(Ordering::Relaxed);
        at != 0 && self.ms().saturating_sub(at) < SHOWING_FOR.as_millis() as u64
    }
}

pub enum Event {
    UpdateAvailable(Option<String>),
    UpdateOutput(String),
    /// `None`: one-mk is not running. Boxed: a status is large, and every
    /// other event would otherwise be as large too.
    Status(Box<Option<Status>>),
    /// A round-trip sample of every connected computer, while a graph shows.
    Links(Vec<(String, one_mk_ctl::status::LinkStats)>),
    ClipboardHistory(Result<Vec<HistoryItem>, String>),
    ClipboardReveal(u64, u64, Result<String, String>),
    Autostart(bool),
    /// A command finished: what it was for, and its error if it failed.
    Done { what: Job, error: Option<String> },
    Latency(Latency),
    /// `settings.toml` or `profiles.toml` as they are now.
    Prefs(Box<Prefs>),
    /// `one-mk doctor --json`: the checks, or why they couldn't be run.
    Doctor(Result<Vec<Check>, String>),
    /// `one-mk keymap check`: the problems with the input tuning (none:
    /// fine), or why it couldn't be run.
    KeymapCheck(Result<Vec<String>, String>),
    /// `one-mk input-devices --json`: the keyboards and mice seen here, or
    /// why they couldn't be listed.
    InputDevices(Result<crate::tune::InputDevices, String>),
}

#[derive(Clone, Debug)]
pub struct HistoryItem {
    pub id: u64,
    pub preview: String,
    pub detail: String,
    pub timestamp: u64,
}

/// A command the window started, with what it needs to report on it.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Update,
    Start,
    Layout { id: String, name: String, side: one_mk_proto::Side },
    Swap { id: String, name: String, swap: String },
    TakeBack,
    Stop,
    Forget(String),
    /// Turning start at login on (true) or off.
    Autostart(bool),
    Send { to: String, count: usize },
    /// Measuring the computer with this id.
    Measure(String),
    Open,
    /// `one-mk profile <verb> <name>`.
    Profile { verb: &'static str, name: String },
    /// Switching automatically to a profile on a Wi-Fi network (`None`: off).
    ProfileAuto { name: String, ssid: Option<String> },
    /// The mouse and keyboard to a computer (an id, or `here`).
    Switch { to: String, name: String },
    LockPointer(bool),
    /// Starting (true) or stopping broadcast to every connected computer.
    Broadcast(bool),
    LockAll,
    /// Sending this clipboard to a computer (`send`), or fetching its.
    Clip { send: bool, id: String, name: String },
    /// A setting changed with `one-mk <args>`; `what` names it in a sentence.
    Set { what: String, args: Vec<String> },
    Doctor,
    EditSettings,
    /// Input tuning changed with `one-mk keymap|pointer|scroll|buttons`;
    /// `what` names it, `area` is where its error shows, `n` tells two
    /// changes apart.
    Tune { what: String, area: &'static str, n: u64 },
}

/// Work for the input tuning worker, which runs one command at a time.
enum Tuning {
    /// Commands run in order, stopping at the first that fails. A later
    /// one with the same `key` (a slider moving) replaces it while queued.
    Run { job: Job, cmds: Vec<Vec<String>>, key: Option<String> },
    Check,
    Devices,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Latency {
    pub peer: String,
    pub p50_us: f32,
    pub p99_us: f32,
    pub lost: u32,
    pub count: u32,
}

#[derive(Clone)]
pub struct Link {
    pub dir: PathBuf,
    custom_home: bool,
    pub daemon: PathBuf,
    tray: Option<PathBuf>,
    tx: Sender<Event>,
    ctx: egui::Context,
    /// The input tuning worker, started on first use.
    tuning: std::sync::Arc<std::sync::Mutex<Option<Sender<Tuning>>>>,
    pub showing: Showing,
}

impl Link {
    pub fn history_reveal(&self, id: u64, epoch: u64) {
        let link = self.clone();
        std::thread::spawn(move || {
            let content = client::request(&link.dir, Request::ClipHistory { action: client::HistoryAction::Show(id), json: false }, Duration::from_secs(5))
                .map_err(|_| "Could not reveal this item. Try Refresh.".to_owned())
                .and_then(|reply| reply.and_then(|reply| reply.strip_prefix("ok\n").map(|body| history_preview(body.trim_end_matches('\n')))).ok_or_else(|| "This item is no longer available.".to_owned()));
            link.send(Event::ClipboardReveal(id, epoch, content));
        });
    }

    pub fn history(&self) {
        #[cfg(debug_assertions)]
        if std::env::var_os("ONE_MK_APP_STATUS").is_some() {
            self.send(Event::ClipboardHistory(Ok(Vec::new())));
            return;
        }
        let link = self.clone();
        std::thread::spawn(move || {
            let read = || -> Result<Vec<HistoryItem>, String> {
                let response = client::request(&link.dir, Request::ClipHistory { action: client::HistoryAction::List, json: true }, Duration::from_secs(5))
                    .map_err(|_| "Could not read history. Try Refresh when one·mk is running.".to_owned())?
                    .ok_or_else(|| "Start one·mk to see clipboard history.".to_owned())?;
                let body = response.strip_prefix("ok\n").ok_or_else(|| response.trim_start_matches("error ").trim().to_owned())?;
                parse_history(body)
            };
            link.send(Event::ClipboardHistory(read()));
        });
    }

    pub fn history_change(&self, id: Option<u64>) {
        self.job(Job::Set { what: "clipboard history".into(), args: Vec::new() }, move |link| {
            link.ask(Request::ClipHistory { action: id.map_or(client::HistoryAction::Clear, client::HistoryAction::Copy), json: false })?;
            link.history();
            Ok(())
        });
    }
    pub fn new(home: Option<PathBuf>, tx: Sender<Event>, ctx: egui::Context) -> anyhow::Result<Self> {
        let custom_home = home.is_some();
        let dir = match home {
            Some(h) => h,
            None => one_mk_ctl::default_home()?,
        };
        let me = std::env::current_exe()?;
        let daemon = install::sibling(&me, "one-mk").unwrap_or_else(|| PathBuf::from("one-mk"));
        let tray = install::sibling(&me, "one-mk-tray");
        Ok(Self { dir, custom_home, daemon, tray, tx, ctx, tuning: Default::default(), showing: Showing::new() })
    }

    pub fn home_args(&self) -> Vec<String> {
        if self.custom_home { vec!["--home".into(), install::absolute(&self.dir).display().to_string()] } else { vec![] }
    }

    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }

    /// `Err` when the daemon could not be asked (busy, a timeout): that
    /// says nothing about whether it runs.
    fn status(&self) -> Result<Option<Status>, ()> {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("ONE_MK_APP_STATUS") {
            // Development aid: a status in its wire format, for screenshots
            // of states that are hard to set up (long names, many computers).
            return Ok(std::fs::read_to_string(path).ok().and_then(|t| Status::decode(&t)));
        }
        match client::request(&self.dir, Request::Status, Duration::from_millis(1500)) {
            Ok(text) => Ok(text.and_then(|t| Status::decode(&t))),
            Err(_) => Err(()),
        }
    }

    /// Watches the daemon until the window closes, reporting only changes.
    /// A daemon that stops answering once is not yet "not running": that
    /// takes two looks in a row, so a busy moment doesn't flip the window.
    pub fn watch(&self) {
        let link = self.clone();
        std::thread::spawn(move || {
            link.send(Event::Autostart(install::is_installed()));
            link.show_tray();
            // What changed apart from the round-trip times, which change at
            // every look: those go as samples, only while a graph shows.
            let mut last: Option<Option<Status>> = None;
            let mut sampled = Instant::now();
            let mut gone = 0;
            let mut stamp = None;
            let opened = std::time::SystemTime::now();
            loop {
                // The tray's "Quit one·mk" writes this file: the window goes too.
                let quit = std::fs::metadata(link.dir.join(client::QUIT_FILE)).and_then(|m| m.modified());
                if quit.is_ok_and(|t| t > opened) {
                    link.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // Settings live in files, not in the status: look at them
                // whenever they change (a stat per look).
                let now = link.prefs_stamp();
                if stamp != Some(now) {
                    stamp = Some(now);
                    link.send(Event::Prefs(Box::new(link.prefs())));
                    link.send(Event::UpdateAvailable(one_mk_ctl::update_logic::cached_version(&link.dir, env!("CARGO_PKG_VERSION"))));
                }
                let Ok(now) = link.status() else {
                    std::thread::sleep(POLL);
                    continue;
                };
                gone = if now.is_none() { gone + 1 } else { 0 };
                let first = last.is_none();
                if now.is_none() && gone < 2 && !first {
                    std::thread::sleep(POLL);
                    continue;
                }
                let same = now.as_ref().map(Status::without_links);
                if last.as_ref() != Some(&same) {
                    if link.tx.send(Event::Status(Box::new(now))).is_err() {
                        return;
                    }
                    link.ctx.request_repaint();
                    last = Some(same);
                    sampled = Instant::now();
                } else if let Some(s) = now.filter(|s| !s.links.is_empty())
                    && sampled.elapsed() >= SAMPLE
                    && link.showing.on_screen()
                {
                    if link.tx.send(Event::Links(s.links)).is_err() {
                        return;
                    }
                    link.ctx.request_repaint();
                    sampled = Instant::now();
                }
                std::thread::sleep(POLL);
            }
        });
    }

    fn prefs_stamp(&self) -> [Option<(std::time::SystemTime, u64)>; 3] {
        let look = |f: &str| std::fs::metadata(self.dir.join(f)).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()));
        [look("settings.toml"), look("profiles.toml"), look("update-state.json")]
    }

    pub fn prefs(&self) -> Prefs {
        let read = |f: &str| std::fs::read_to_string(self.dir.join(f));
        let settings = read("settings.toml");
        let mut p = Prefs::read(settings.as_deref().unwrap_or(""), &read("profiles.toml").unwrap_or_default());
        p.unreadable = settings.as_ref().is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound);
        p
    }

    /// Runs `f` on its own thread and reports how it went.
    fn job(&self, what: Job, f: impl FnOnce(&Link) -> anyhow::Result<()> + Send + 'static) {
        let link = self.clone();
        std::thread::spawn(move || {
            let error = f(&link).err().map(|e| format!("{e:#}"));
            link.send(Event::Done { what, error });
        });
    }

    /// `one-mk <args>`, with the settings folder; its error output on failure.
    fn one_mk(&self, args: &[&str]) -> anyhow::Result<String> {
        let home = self.home_args();
        self.run_one_mk(&args.iter().copied().chain(home.iter().map(String::as_str)).collect::<Vec<_>>())
    }

    /// `one-mk <args>` exactly as given.
    fn run_one_mk(&self, args: &[&str]) -> anyhow::Result<String> {
        let mut cmd = Command::new(&self.daemon);
        cmd.args(args).stdin(Stdio::null());
        let out = proc::hide_window(&mut cmd).output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("{}", err.trim().trim_start_matches("one-mk: "));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn start(&self) {
        self.job(Job::Start, |l| {
            if client::is_running(&l.dir) {
                return Ok(());
            }
            let mut args = vec!["run".to_owned()];
            args.extend(l.home_args());
            proc::spawn_detached(&l.daemon, &args, Some(&l.dir.join("daemon.log")))?;
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline && !client::is_running(&l.dir) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if !client::is_running(&l.dir) {
                anyhow::bail!("It didn't answer within 4 seconds. Details are in {}.", l.dir.join("daemon.log").display());
            }
            Ok(())
        });
    }

    pub fn update(&self) {
        self.job(Job::Update, |link| {
            let output = link.one_mk(&["update"])?;
            link.send(Event::UpdateOutput(output));
            Ok(())
        });
    }

    /// Moves a peer to `side`: with an offset, to that exact place; without,
    /// anywhere along it.
    pub fn set_side(&self, peer: &str, name: &str, side: one_mk_proto::Side, offset: Option<(i32, i32)>) {
        let (id, word) = (peer.to_owned(), side.to_string());
        let at = offset.map_or_else(|| "--offset=none".to_owned(), |(x, y)| format!("--offset={x},{y}"));
        self.job(Job::Layout { id: peer.into(), name: name.into(), side }, move |l| l.one_mk(&["layout", &id, &word, &at]).map(drop));
    }

    pub fn set_swap(&self, peer: &str, name: &str, swap: &str) {
        let (id, value) = (peer.to_owned(), swap.to_owned());
        let what = Job::Swap { id: peer.into(), name: name.into(), swap: swap.into() };
        self.job(what, move |l| l.one_mk(&["layout", &id, "--swap-ctrl-cmd", &value]).map(drop));
    }

    /// `one-mk profile use|save|delete <name>`.
    pub fn profile(&self, verb: &'static str, name: &str) {
        let n = name.to_owned();
        self.job(Job::Profile { verb, name: name.into() }, move |l| l.one_mk(&["profile", verb, &n]).map(drop));
    }

    pub fn profile_auto(&self, name: &str, ssid: Option<String>) {
        let n = name.to_owned();
        let what = Job::ProfileAuto { name: name.into(), ssid: ssid.clone() };
        self.job(what, move |l| match &ssid {
            Some(s) => l.one_mk(&["profile", "auto", &n, "--ssid", s]).map(drop),
            None => l.one_mk(&["profile", "auto", &n, "--off"]).map(drop),
        });
    }

    /// Moves the mouse and keyboard to `to` (a peer's id, or `here`).
    pub fn switch(&self, to: &str, name: &str) {
        let target = if to == "here" { SwitchTo::Here } else { SwitchTo::Device(to.to_owned()) };
        self.job(Job::Switch { to: to.into(), name: name.into() }, move |l| l.ask(Request::Switch(target)));
    }

    pub fn lock_pointer(&self, on: bool) {
        let t = if on { Toggle::On } else { Toggle::Off };
        self.job(Job::LockPointer(on), move |l| l.ask(Request::LockPointer(t)));
    }

    pub fn broadcast(&self, on: bool) {
        let t = if on { Toggle::On } else { Toggle::Off };
        self.job(Job::Broadcast(on), move |l| l.ask(Request::Broadcast(t)));
    }

    pub fn lock_all(&self) {
        self.job(Job::LockAll, |l| l.ask(Request::LockAll));
    }

    pub fn clip(&self, send: bool, peer: &str, name: &str) {
        let id = peer.to_owned();
        let req = if send { Request::ClipSend(Some(id)) } else { Request::ClipGet(Some(id)) };
        self.job(Job::Clip { send, id: peer.into(), name: name.into() }, move |l| l.ask(req));
    }

    /// Changes a setting with `one-mk <args>`; `what` names it for an error.
    pub fn set(&self, what: &str, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let job = Job::Set { what: what.into(), args: args.clone() };
        self.job(job, move |l| l.one_mk(&args.iter().map(String::as_str).collect::<Vec<_>>()).map(drop));
    }

    /// Changes input tuning: `cmds` are `one-mk` commands, run in order
    /// after everything asked before (one command at a time, so two
    /// changes to `settings.toml` never race). A change with the same
    /// `key` that is still waiting is dropped for this one.
    pub fn tune(&self, job: Job, cmds: Vec<Vec<String>>, key: Option<String>) {
        self.tuning_send(Tuning::Run { job, cmds, key });
    }

    /// Runs `one-mk keymap check` after whatever tuning is queued.
    pub fn keymap_check(&self) {
        self.tuning_send(Tuning::Check);
    }

    /// Lists the keyboards and mice seen here (`one-mk input-devices`).
    pub fn input_devices(&self) {
        self.tuning_send(Tuning::Devices);
    }

    fn tuning_send(&self, t: Tuning) {
        let mut slot = self.tuning.lock().unwrap_or_else(|e| e.into_inner());
        let t = match slot.as_ref() {
            Some(tx) => match tx.send(t) {
                Ok(()) => return,
                Err(e) => e.0,
            },
            None => t,
        };
        let (tx, rx) = std::sync::mpsc::channel::<Tuning>();
        let _ = tx.send(t);
        *slot = Some(tx);
        let link = self.clone();
        // Blocked on the channel while idle; it ends with the window.
        std::thread::spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut queue = vec![first];
                queue.extend(rx.try_iter());
                // A newer change to the same thing wins.
                let mut keep = vec![true; queue.len()];
                for (i, t) in queue.iter().enumerate() {
                    if let Tuning::Run { key: Some(k), .. } = t {
                        keep[i] = !queue[i + 1..].iter().any(|u| matches!(u, Tuning::Run { key: Some(l), .. } if l == k));
                    }
                }
                for (t, keep) in queue.into_iter().zip(keep) {
                    match t {
                        Tuning::Run { job, .. } if !keep => link.send(Event::Done { what: job, error: None }),
                        Tuning::Run { job, cmds, .. } => {
                            // The settings folder goes first: these end their options with `--`.
                            let error = cmds
                                .iter()
                                .find_map(|c| link.run_one_mk(&link.home_args().iter().chain(c).map(String::as_str).collect::<Vec<_>>()).err())
                                .map(|e| format!("{e:#}"));
                            link.send(Event::Done { what: job, error });
                        }
                        Tuning::Check => link.send(Event::KeymapCheck(link.run_keymap_check())),
                        Tuning::Devices => link.send(Event::InputDevices(link.run_input_devices())),
                    }
                }
            }
        });
    }

    fn run_keymap_check(&self) -> Result<Vec<String>, String> {
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["keymap", "check"]).args(self.home_args()).stdin(Stdio::null());
        let out = proc::hide_window(&mut cmd).output().map_err(|e| format!("one-mk didn't start: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        if out.status.success() {
            return Ok(vec![]);
        }
        // Problems are on stdout, one a line; stderr says how many.
        let problems: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("No input tuning")).map(str::to_owned).collect();
        if problems.is_empty() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(err.trim().trim_start_matches("one-mk: ").to_owned());
        }
        Ok(problems)
    }

    fn run_input_devices(&self) -> Result<crate::tune::InputDevices, String> {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("ONE_MK_APP_INPUT_DEVICES") {
            // Development aid: a list to show instead of the real one.
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            return view::parse_input_devices(&text).ok_or_else(|| "could not read the list".into());
        }
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["input-devices", "--json"]).args(self.home_args()).stdin(Stdio::null());
        let out = proc::hide_window(&mut cmd).output().map_err(|e| format!("one-mk didn't start: {e}"))?;
        view::parse_input_devices(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim().trim_start_matches("one-mk: ");
            if err.is_empty() { "one-mk gave no list".to_owned() } else { err.to_owned() }
        })
    }

    /// Runs `one-mk doctor --json` and reports the checks.
    pub fn doctor(&self) {
        self.job(Job::Doctor, |l| {
            let result = l.run_doctor();
            l.send(Event::Doctor(result));
            Ok(())
        });
    }

    fn run_doctor(&self) -> Result<Vec<Check>, String> {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("ONE_MK_APP_DOCTOR") {
            // Development aid: a report to show instead of a real check.
            std::thread::sleep(Duration::from_millis(600));
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            return view::parse_doctor(&text).ok_or_else(|| "could not read the report".into());
        }
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["doctor", "--json"]).args(self.home_args()).stdin(Stdio::null());
        // It says what it found on stdout, also when something failed.
        let out = proc::hide_window(&mut cmd).output().map_err(|e| format!("one-mk didn't start: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        view::parse_doctor(&text).ok_or_else(|| {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim().trim_start_matches("one-mk: ");
            if err.is_empty() { "one-mk doctor gave no report".to_owned() } else { err.to_owned() }
        })
    }

    /// Opens `settings.toml` in the system's editor (made first, private,
    /// if there is none yet).
    pub fn edit_settings(&self) {
        self.job(Job::EditSettings, |l| {
            let path = l.dir.join("settings.toml");
            if !path.exists() {
                std::fs::create_dir_all(&l.dir)?;
                let mut o = std::fs::OpenOptions::new();
                o.write(true).create_new(true);
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
                drop(o.open(&path));
            }
            let (opener, args): (&str, Vec<String>) = if cfg!(target_os = "macos") {
                ("open", vec!["-t".into(), path.display().to_string()])
            } else if cfg!(windows) {
                ("notepad", vec![path.display().to_string()])
            } else {
                ("xdg-open", vec![path.display().to_string()])
            };
            proc::spawn_detached(std::path::Path::new(opener), &args, None)
                .map_err(|e| anyhow::anyhow!("{opener} didn't start ({e}). The file is {}", path.display()))
        });
    }

    /// One request to the running daemon; its error if it refused.
    fn ask(&self, req: Request) -> anyhow::Result<()> {
        let timeout = req.reply_timeout();
        let Some(reply) = client::request(&self.dir, req, timeout)? else { anyhow::bail!("one-mk is not running") };
        let reply = reply.trim();
        match reply.strip_prefix("error") {
            Some(e) => anyhow::bail!("{}", e.trim_start_matches([' ', ':'])),
            None => Ok(()),
        }
    }

    /// Puts the menu-bar / tray icon up if it isn't, so the window is never
    /// the only way in. The tray exits by itself when one is already there.
    fn show_tray(&self) {
        #[cfg(debug_assertions)]
        if std::env::var_os("ONE_MK_APP_STATUS").is_some() { return; }
        if let Some(tray) = &self.tray {
            let _ = proc::spawn_detached(tray, &self.home_args(), None);
        }
    }

    /// Forgets a paired computer; a running daemon lets go of it at once.
    pub fn forget(&self, id: &str, name: &str) {
        let id = id.to_owned();
        self.job(Job::Forget(name.to_owned()), move |l| {
            l.one_mk(&["forget", &id])?;
            client::request(&l.dir, Request::Reload, Duration::from_secs(5)).map(drop)
        });
    }

    pub fn stop(&self) {
        self.job(Job::Stop, |l| match client::request(&l.dir, Request::Stop, Duration::from_secs(5))? {
            // One started before an update answers "unknown request".
            Some(r) if !r.starts_with("ok") => anyhow::bail!("this one·mk is older than the app"),
            _ => Ok(()),
        });
    }

    pub fn take_back(&self) {
        self.job(Job::TakeBack, |l| client::request(&l.dir, Request::TakeBack, Duration::from_secs(5)).map(drop));
    }

    /// Either way it ends, the switch is told what is really installed.
    pub fn set_autostart(&self, on: bool) {
        self.job(Job::Autostart(on), move |l| {
            let done = if on {
                if l.daemon.is_absolute() {
                    install::install(&Launch {
                        exe: l.daemon.clone(),
                        tray: l.tray.clone(),
                        home: l.custom_home.then(|| install::absolute(&l.dir)),
                        start_now: !client::is_running(&l.dir),
                    })
                } else {
                    Err(anyhow::anyhow!("one-mk isn't next to this app. Run `one-mk install` instead"))
                }
            } else {
                install::uninstall()
            };
            l.send(Event::Autostart(install::is_installed()));
            done.map(drop)
        });
    }

    pub fn send_files(&self, peer: &str, name: &str, paths: Vec<PathBuf>) {
        let peer = peer.to_owned();
        let what = Job::Send { to: name.to_owned(), count: paths.len() };
        self.job(what, move |l| {
            let mut args = vec!["send".to_owned(), "--to".into(), peer];
            args.extend(paths.iter().map(|p| p.display().to_string()));
            l.one_mk(&args.iter().map(String::as_str).collect::<Vec<_>>()).map(drop)
        });
    }

    /// Measures the round trip to a connected computer with a short burst
    /// of probe messages (a fraction of a second; nothing moves on screen).
    pub fn measure(&self, peer: &str) {
        let peer = peer.to_owned();
        self.job(Job::Measure(peer.clone()), move |l| {
            let out = l.one_mk(&["bench", "--peer", &peer, "--count", "400", "--rate", "1000"])?;
            let lat = parse_bench(&out, &peer).ok_or_else(|| anyhow::anyhow!("could not read the measurement"))?;
            l.send(Event::Latency(lat));
            Ok(())
        });
    }

    /// Opens the folder received files go to.
    pub fn open_downloads(&self) {
        self.job(Job::Open, |l| {
            let dir = directories::UserDirs::new()
                .and_then(|u| u.download_dir().map(|d| d.join("one-mk")))
                .unwrap_or_else(|| l.dir.clone());
            std::fs::create_dir_all(&dir)?;
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else if cfg!(windows) {
                "explorer"
            } else {
                "xdg-open"
            };
            proc::spawn_detached(std::path::Path::new(opener), &[dir.display().to_string()], None)
                .map_err(|e| anyhow::anyhow!("{opener} didn't start ({e}). The folder is {}", dir.display()))?;
            Ok(())
        });
    }
}

fn history_preview(body: &str) -> String {
    let normalized: String = body.chars().map(|ch| if ch.is_whitespace() { ' ' } else { ch })
        .filter(|ch| !ch.is_control()).collect();
    let mut chars = normalized.chars();
    let mut preview: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() { preview.push('…'); }
    preview
}

fn parse_history(body: &str) -> Result<Vec<HistoryItem>, String> {
    #[derive(serde::Deserialize)]
    struct Metadata {
        id: u64,
        kind: String,
        preview: String,
        size: u64,
        timestamp: u64,
        origin: String,
    }
    let bad = || "Could not read history. Update one·mk and try Refresh.".to_owned();
    if body.len() > 1000 * (2 * 80 * 4 + 128) { return Err(bad()); }
    let entries: Vec<Metadata> = serde_json::from_str(body).map_err(|_| bad())?;
    if entries.len() > 1000 { return Err(bad()); }
    entries.into_iter().map(|entry| {
        let Metadata { id, kind, preview, size, timestamp, origin } = entry;
        if id == 0 || id == u64::MAX || !matches!(kind.as_str(), "text" | "html" | "image")
            || preview != "[hidden; use Show]" || origin.chars().count() > 85 || origin.chars().any(char::is_control) { return Err(bad()); }
        let size = if size >= 1 << 20 { format!("{:.1} MB", size as f64 / f64::from(1 << 20)) }
            else if size >= 1 << 10 { format!("{} KB", size.div_ceil(1 << 10)) }
            else { format!("{size} B") };
        Ok(HistoryItem { id, preview, detail: format!("{kind} · {size} · from {origin}"), timestamp })
    }).collect()
}

/// Reads `one-mk bench` output: "round trip  p50 312 µs  p99 520 µs ..."
/// and "lost  0 of 400".
pub fn parse_bench(out: &str, peer: &str) -> Option<Latency> {
    let num_after = |line: &str, key: &str| -> Option<f32> {
        let rest = &line[line.find(key)? + key.len()..];
        rest.split_whitespace().next()?.parse().ok()
    };
    let rtt = out.lines().find(|l| l.trim_start().starts_with("round trip"))?;
    let lost = out.lines().find(|l| l.trim_start().starts_with("lost"))?;
    let mut words = lost.split_whitespace().skip(1);
    let lost_n = words.next()?.parse().ok()?;
    let count = words.nth(1)?.parse().ok()?;
    Some(Latency { peer: peer.into(), p50_us: num_after(rtt, "p50")?, p99_us: num_after(rtt, "p99 ")?, lost: lost_n, count })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_accepts_the_daemons_stable_json_metadata() {
        let body = r#"[{"id":7,"kind":"text","preview":"[hidden; use Show]","size":12,"timestamp":123,"origin":"desk"}]"#;
        let parsed = parse_history(body).unwrap();
        assert_eq!(parsed[0].id, 7);
        assert_eq!(parsed[0].timestamp, 123);
        assert_eq!(parsed[0].preview, "[hidden; use Show]");
        assert!(parse_history("[]\n").unwrap().is_empty());
    }

    #[test]
    fn history_preview_preserves_separators_and_marks_truncation() {
        assert_eq!(history_preview("multi\nline\ttext"), "multi line text");
        assert_eq!(history_preview(&"a".repeat(80)), "a".repeat(80));
        assert_eq!(history_preview(&"é".repeat(81)), format!("{}…", "é".repeat(80)));
        assert_eq!(history_preview("hello\u{1b}world"), "helloworld");
    }

    #[test]
    fn history_metadata_and_hostile_generated_lines() {
        let row = format!(r#"{{"id":7,"kind":"text","preview":"[hidden; use Show]","size":12,"timestamp":123,"origin":"{}"}}"#, "🦀".repeat(80));
        let largest = format!("[{}]", vec![row.as_str(); 1000].join(","));
        assert_eq!(parse_history(&largest).unwrap().len(), 1000);
        let valid = format!("[{row}]");
        let parsed = parse_history(&valid).unwrap();
        assert_eq!(parsed[0].id, 7);
        assert_eq!(parsed[0].preview, "[hidden; use Show]");
        assert!(parse_history(&format!("[{}]", vec![row.as_str(); 1001].join(","))).is_err());
        assert!(parse_history(&valid.replace("[hidden; use Show]", "unexpected secret")).is_err());
        assert!(parse_history(&valid.replace(&"🦀".repeat(80), "hostile\\u001b[2J")).is_err());
        for (cut, _) in valid.char_indices().skip(1) {
            let _ = parse_history(&valid[..cut]);
        }
        let mut state = 7u32;
        for length in 0..10000usize {
            let generated: String = (0..length % 200).map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                char::from_u32(state % 0x110000).unwrap_or('\u{fffd}')
            }).collect();
            if let Ok(items) = parse_history(&generated) { assert!(items.len() <= 1000); }
        }
    }

    #[test]
    fn reads_bench_output() {
        let out = "one-mk bench (peer mac, 400 events at 1000 Hz)\n\
                   \x20 protocol cost   34 ns per mouse event (move, encode, decode, filter)\n\
                   \x20 round trip      p50 312 µs   p99 520 µs   p99.9 610 µs   max 700 µs\n\
                   \x20 one-way         ~156 µs typical (half the round trip)\n\
                   \x20 lost            2 of 400\n";
        assert_eq!(
            parse_bench(out, "mac"),
            Some(Latency { peer: "mac".into(), p50_us: 312.0, p99_us: 520.0, lost: 2, count: 400 })
        );
        assert_eq!(parse_bench("garbage", "mac"), None);
    }
}
