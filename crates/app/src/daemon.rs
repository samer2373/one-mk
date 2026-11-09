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

/// How often the window looks at the daemon while it is in use. A status
/// request is one small message over a local socket.
const POLL: Duration = Duration::from_millis(250);
/// ... while it is open but another window has the focus: a change still
/// shows within half a second.
const POLL_UNFOCUSED: Duration = Duration::from_millis(500);
/// ... while it is minimised or hidden and nobody can see a change. The
/// first frame after it comes back wakes the watcher at once.
const POLL_HIDDEN: Duration = Duration::from_secs(3);
/// While a graph shows, the window is redrawn at least this often (the
/// graph scrolls a little with time) even when no reading changed.
pub const GRAPH_IDLE_REDRAW: Duration = Duration::from_secs(10);
/// ... and a reading that only reads as another number of milliseconds
/// waits this long for its frame (a jittery link, a frame every 2 s).
const GRAPH_VALUE_REDRAW: Duration = Duration::from_secs(2);
/// A graph that has just come on screen is drawn for each of its first
/// readings, so it fills in at once (it says nothing before
/// [`crate::rtt::MIN_SAMPLES`]).
const GRAPH_YOUNG_DRAWS: u32 = crate::rtt::MIN_SAMPLES as u32 + 1;

/// How much attention the window gets, which sets how often it looks at
/// the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Attention {
    /// On screen with the focus (or no way to tell).
    #[default]
    Active,
    /// On screen, another window has the focus.
    Background,
    /// Minimised or hidden.
    Hidden,
}

impl Attention {
    /// From what the windowing system reports about the window.
    pub fn of(minimized: Option<bool>, focused: Option<bool>) -> Self {
        if minimized == Some(true) {
            Attention::Hidden
        } else if focused == Some(false) {
            Attention::Background
        } else {
            Attention::Active
        }
    }

    /// The pause between two looks at the daemon.
    pub fn poll(self) -> Duration {
        match self {
            Attention::Active => POLL,
            Attention::Background => POLL_UNFOCUSED,
            Attention::Hidden => POLL_HIDDEN,
        }
    }
}

/// Whether a graph of the connections is on screen, so the window asks the
/// daemon for round-trip times only then. The window stamps it on every
/// frame it draws one; a window that is minimised or covered stops drawing,
/// the stamp goes stale and nothing more is asked.
#[derive(Clone)]
pub struct Showing {
    /// Milliseconds since `epoch` at the last frame with a graph; 0: none.
    stamp: Arc<AtomicU64>,
    epoch: Instant,
    /// What the window last said about itself; a change wakes the watcher.
    attention: Arc<(std::sync::Mutex<Attention>, std::sync::Condvar)>,
}

/// A stamp older than this says the graph is no longer on screen. A window
/// with a steady graph draws once per [`GRAPH_IDLE_REDRAW`], so the stamp
/// must outlive that.
const SHOWING_FOR: Duration = Duration::from_millis(GRAPH_IDLE_REDRAW.as_millis() as u64 + 3000);
/// How often the connections are sampled while a graph is on screen.
const SAMPLE: Duration = Duration::from_millis((crate::rtt::EVERY * 1000.0) as u64);

impl Showing {
    pub fn new() -> Self {
        Self { stamp: Arc::default(), epoch: Instant::now(), attention: Arc::default() }
    }

    /// The window's state, said on every frame; the watcher is woken the
    /// moment it changes (the window came back: look now).
    pub fn attend(&self, attention: Attention) {
        let (state, wake) = &*self.attention;
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        if *state != attention {
            *state = attention;
            wake.notify_all();
        }
    }

    /// Waits out the pause between two looks, less when the window's state
    /// changes meanwhile.
    fn nap(&self) {
        let (state, wake) = &*self.attention;
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        let seen = *state;
        let _ = wake.wait_timeout_while(state, seen.poll(), |now| *now == seen);
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
        at != 0 && self.ms().saturating_sub(at) < SHOWING_FOR.as_millis() as u64 && !self.hidden()
    }

    /// The window says it is minimised or hidden.
    fn hidden(&self) -> bool {
        *self.attention.0.lock().unwrap_or_else(|e| e.into_inner()) == Attention::Hidden
    }
}

pub enum Event {
    UpdateStatus(one_mk_ctl::update_logic::UpdateStatus),
    UpdateOutput(String),
    /// `None`: one-mk is not running. Boxed: a status is large, and every
    /// other event would otherwise be as large too.
    Status(Box<Option<Status>>),
    /// A round-trip sample of every connected computer, while a graph shows.
    /// `Instant`: when it was taken (a frame may draw it later).
    Links(Vec<(String, one_mk_ctl::status::LinkStats)>, Instant),
    ClipboardHistory(Result<Vec<HistoryItem>, String>),
    ClipboardReveal(u64, u64, Result<String, String>),
    Autostart(bool),
    /// First-run setup ran: what it did.
    Setup(one_mk_ctl::setup::Report),
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
    /// Looking for a newer version now, not waiting for the daily check.
    UpdateCheck,
    UpdateSettings,
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
    pub fn wallpaper(&self, command: one_mk_wallpaper::Command) {
        self.job(Job::Set { what: "wallpaper".into(), args: Vec::new() }, move |link| {
            let json = serde_json::to_string(&command)?;
            let response = client::request(&link.dir, Request::LiveWallpaper(json), Duration::from_secs(660))?
                .ok_or_else(|| anyhow::anyhow!("Start oneMK first"))?;
            anyhow::ensure!(!response.starts_with("error "), "{}", response.trim_start_matches("error ").trim());
            Ok(())
        });
    }
    pub fn pair_with(&self, method: crate::pairing::Method, port: u16) -> std::io::Result<crate::pairing::Pairing> {
        crate::pairing::Pairing::with_method(&self.daemon, &self.home_args(), self.ctx.clone(), method, port)
    }
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
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_STATUS").is_some() {
            self.send(Event::ClipboardHistory(Ok(Vec::new())));
            return;
        }
        let link = self.clone();
        std::thread::spawn(move || {
            let read = || -> Result<Vec<HistoryItem>, String> {
                let response = client::request(&link.dir, Request::ClipHistory { action: client::HistoryAction::List, json: true }, Duration::from_secs(5))
                    .map_err(|_| "Could not read history. Try Refresh when oneMK is running.".to_owned())?
                    .ok_or_else(|| "Start oneMK to see clipboard history.".to_owned())?;
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
        let me = running_executable()?;
        let daemon = install::sibling(&me, "one-mk").unwrap_or_else(|| PathBuf::from("one-mk"));
        let tray = install::sibling(&me, "one-mk-tray");
        Ok(Self { dir, custom_home, daemon, tray, tx, ctx, tuning: Default::default(), showing: Showing::new() })
    }

    /// The settings folder a login entry for this window names: `None`
    /// for the usual one.
    fn login_home(&self) -> Option<PathBuf> { self.custom_home.then(|| install::absolute(&self.dir)) }

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
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(path) = std::env::var_os("ONE_MK_APP_STATUS") {
            // Development aid: a status in its wire format, for screenshots
            // of states that are hard to set up (long names, many computers).
            let mut status = std::fs::read_to_string(path).ok().and_then(|text| Status::decode(&text));
            if let Some(status) = &mut status && let Some(poster) = std::env::var_os("ONE_MK_APP_WALLPAPER") {
                let canvas = one_mk_ctl::wallpaper::canvas(status);
                status.wallpaper = Some(one_mk_wallpaper::Status {
                    plan: Some(one_mk_wallpaper::Plan { hash: "a".repeat(64), title: "Across the horizon.mp4".into(),
                        media: one_mk_wallpaper::Media { width: 3840, height: 1080, duration_ms: 10000, frame_rate: 30 }, canvas,
                        start_ms: 1, fit: Default::default(), computers: std::iter::once(status.id.clone()).chain(status.peers.iter().map(|peer| peer.id.clone())).collect(), pause_on_battery: true, paused: false, picture: false, asset_hash: None }),
                    available: true, poster: Some(poster.into()), message: "Playing on 3 monitors on 2 computers".into(), computers: Default::default(), gnome: None, experimental: None,
                });
                if let Some(wallpaper) = &mut status.wallpaper && let Some(plan) = &wallpaper.plan { wallpaper.message = plan.playing_message(); }
                // ONE_MK_APP_GNOME=1: the GNOME video setup block, for layout checks.
                if std::env::var_os("ONE_MK_APP_GNOME").is_some() && let Some(wallpaper) = &mut status.wallpaper {
                    wallpaper.gnome = Some(one_mk_wallpaper::GnomeSetup { stage: one_mk_wallpaper::GnomeStage::NotInstalled, message: one_mk_wallpaper::GNOME_PLAYBACK_UNAVAILABLE.into(), can_install: true, can_enable: false });
                }
            }
            return Ok(status);
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
            link.send(Event::Autostart(install::is_installed_for(link.login_home().as_deref())));
            link.show_tray();
            // What changed apart from the round-trip times, which change at
            // every look: those go as samples, only while a graph shows.
            let mut last: Option<Option<Status>> = None;
            let mut sampled = Instant::now();
            // What the window was last redrawn with, and when.
            let mut drawn: Vec<(String, one_mk_ctl::status::LinkStats)> = Vec::new();
            let mut redrawn = Instant::now();
            // Readings drawn since the graph came on screen.
            let mut young = 0u32;
            let mut gone = 0;
            let mut stamp = None;
            let opened = std::time::SystemTime::now();
            let mut update_checked = None::<Instant>;
            loop {
                if update_checked.is_none_or(|last| last.elapsed() >= Duration::from_secs(24 * 60 * 60)) {
                    update_checked = Some(Instant::now());
                    link.check_updates();
                }
                // The tray's "Quit oneMK" writes this file: the window goes too.
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
                    // Whether an update waits comes with the update status,
                    // measured against the service's version, not this
                    // window's (a portable update replaces the service).
                    link.update_status();
                }
                let Ok(now) = link.status() else {
                    link.showing.nap();
                    continue;
                };
                gone = if now.is_none() { gone + 1 } else { 0 };
                let first = last.is_none();
                if now.is_none() && gone < 2 && !first {
                    link.showing.nap();
                    continue;
                }
                let same = now.as_ref().map(Status::without_links);
                let visible = link.showing.on_screen();
                if !visible {
                    young = 0;
                }
                if last.as_ref() != Some(&same) {
                    if link.tx.send(Event::Status(Box::new(now))).is_err() {
                        return;
                    }
                    link.ctx.request_repaint();
                    last = Some(same);
                    sampled = Instant::now();
                } else if let Some(s) = now.filter(|s| !s.links.is_empty())
                    && sampled.elapsed() >= SAMPLE
                    && visible
                {
                    // Every reading is kept, but a frame is only asked for
                    // when the picture would change, or now and then so the
                    // graph keeps scrolling: a steady line costs nothing.
                    let draw = young < GRAPH_YOUNG_DRAWS || match crate::rtt::change(&drawn, &s.links) {
                        crate::rtt::Change::Urgent => true,
                        crate::rtt::Change::Value => redrawn.elapsed() >= GRAPH_VALUE_REDRAW,
                        crate::rtt::Change::None => redrawn.elapsed() >= GRAPH_IDLE_REDRAW,
                    };
                    let links = s.links;
                    if draw {
                        drawn.clone_from(&links);
                    }
                    if link.tx.send(Event::Links(links, Instant::now())).is_err() {
                        return;
                    }
                    if draw {
                        redrawn = Instant::now();
                        young = young.saturating_add(1);
                        link.ctx.request_repaint();
                    }
                    sampled = Instant::now();
                }
                link.showing.nap();
            }
        });
    }

    fn prefs_stamp(&self) -> [Option<(std::time::SystemTime, u64)>; 3] {
        let look = |f: &str| std::fs::metadata(self.dir.join(f)).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()));
        [look("settings.toml"), look("profiles.toml"), look("update-state.json")]
    }

    pub fn prefs(&self) -> Prefs {
        let read = |f: &str| std::fs::read_to_string(self.dir.join(f));
        let settings = settings_text(&self.dir.join("settings.toml"));
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

    /// What to start at login. Inside an AppImage the programs sit in a
    /// temporary mount that is gone after the window closes, so it is the
    /// AppImage file itself (which hands its arguments to `one-mk`); the
    /// tray only exists as a real file beside `one-mk`.
    fn login_launch(&self) -> anyhow::Result<Launch> {
        let exe = install::program_path().ok().filter(|p| std::env::var_os("APPIMAGE").is_some() && p.is_file()).unwrap_or_else(|| self.daemon.clone());
        if !exe.is_absolute() {
            anyhow::bail!("The sharing service could not be found next to this app. Try reopening oneMK.");
        }
        Ok(Launch {
            tray: install::sibling(&exe, "one-mk-tray"),
            exe,
            home: self.custom_home.then(|| install::absolute(&self.dir)),
            start_now: !client::is_running(&self.dir),
        })
    }

    /// First launch only: starts one-mk at login, adds the file manager
    /// entries and starts the service, so nobody has to run `one-mk
    /// install`. Never runs again for this settings folder (see
    /// `one_mk_ctl::setup`); the window says what it did.
    pub fn first_run(&self) {
        // Screenshots and demos must not touch the real login items.
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_STATUS").is_some() || std::env::var_os("ONE_MK_APP_NO_SETUP").is_some()
            || std::env::var_os("ONE_MK_APP_GUIDE").is_some() || std::env::var_os("ONE_MK_GUIDE_STAGE").is_some() || std::env::var_os("ONE_MK_APP_SHOT").is_some() {
            return;
        }
        let link = self.clone();
        std::thread::spawn(move || {
            if let Some(report) = one_mk_ctl::setup::run(&link.dir, &link) {
                link.send(Event::Autostart(install::is_installed_for(link.login_home().as_deref())));
                link.send(Event::Setup(report));
            }
        });
    }

    pub fn update(&self) {
        self.install_update(None);
    }

    pub fn move_to_applications(&self) {
        self.install_update(Some("--move-to-applications"));
    }

    pub fn rollback_update(&self) {
        self.install_update(Some("--rollback"));
    }

    fn install_update(&self, action: Option<&'static str>) {
        self.job(Job::Update, move |link| {
            let result = link.stream_update(action);
            link.update_status();
            result
        });
    }

    fn stream_update(&self, action: Option<&str>) -> anyhow::Result<()> {
        use std::io::BufRead;
        let mut command = Command::new(&self.daemon);
        command.arg("update");
        if let Some(action) = action { command.arg(action); }
        command.args(self.home_args()).stdin(Stdio::null()).stdout(Stdio::piped());
        let errors = self.dir.join("update-error.log");
        let error_file = std::fs::File::create(&errors)?;
        command.stderr(Stdio::from(error_file));
        let mut child = proc::hide_window(&mut command).spawn()?;
        let mut restarting = false;
        let mut read_error = None;
        if let Some(output) = child.stdout.take() {
            for line in std::io::BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        restarting |= line.contains("Restarting…");
                        self.send(Event::UpdateOutput(line));
                    }
                    Err(error) => { read_error = Some(error); break; }
                }
            }
        }
        let status = child.wait()?;
        if let Some(error) = read_error { return Err(error.into()); }
        if !status.success() {
            let error = std::fs::read_to_string(&errors).unwrap_or_default();
            anyhow::bail!("{}", if error.trim().is_empty() { "Couldn't finish the update. Choose Download instead" } else { error.trim().trim_start_matches("one-mk: ") });
        }
        if restarting { self.restart_updated(action == Some("--move-to-applications"))?; }
        Ok(())
    }

    fn restart_updated(&self, moved: bool) -> anyhow::Result<()> {
        let _ = self.one_mk(&["stop"])?;
        let mut daemon = self.daemon.clone();
        let mut window = running_executable()?;
        if moved {
            daemon = PathBuf::from("/Applications/one-mk.app/Contents/MacOS/one-mk");
            window = PathBuf::from("/Applications/one-mk.app/Contents/MacOS/one-mk-app");
        }
        let image = std::env::var_os("APPIMAGE").map(PathBuf::from)
            .filter(|_| std::env::var_os("APPDIR").is_some_and(|dir| window.starts_with(dir)));
        let mut args = vec!["run".to_owned()];
        args.extend(self.home_args());
        proc::spawn_detached(image.as_ref().unwrap_or(&daemon), &args, Some(&self.dir.join("daemon.log")))?;
        let deadline = Instant::now() + Duration::from_secs(8);
        while !client::is_running(&self.dir) {
            anyhow::ensure!(Instant::now() < deadline, "The update is installed, but sharing couldn't restart. Open oneMK again");
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut args = Vec::new();
        if image.is_some() { args.push("app".to_owned()); }
        args.extend(self.home_args());
        proc::spawn_detached(image.as_ref().unwrap_or(&window), &args, None)?;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        Ok(())
    }

    pub fn acknowledge_app_start(&self) {
        let link = self.clone();
        std::thread::spawn(move || { let _ = link.one_mk(&["update", "--started-version", env!("CARGO_PKG_VERSION")]); });
    }

    pub fn update_status(&self) {
        match self.one_mk(&["update", "--status-json"]).and_then(|output| Ok(serde_json::from_str(&output)?)) {
            Ok(status) => self.send(Event::UpdateStatus(status)),
            Err(_) => self.send(Event::UpdateOutput("Couldn't read update preferences. Choose Download instead".into())),
        }
    }

    pub fn check_updates(&self) {
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_STATUS").is_some() || std::env::var_os("ONE_MK_APP_NO_SETUP").is_some() { return; }
        let link = self.clone();
        std::thread::spawn(move || {
            let _ = link.one_mk(&["update", "--startup-check"]);
            link.update_status();
        });
    }

    /// "Check for updates": asks the release server now (the daily check
    /// is separate) and refreshes what the window knows.
    pub fn check_updates_now(&self) {
        self.job(Job::UpdateCheck, |link| {
            let result = link.one_mk(&["update", "--check"]);
            link.update_status();
            result.map(drop)
        });
    }

    pub fn update_preference(&self, automatic: bool, enabled: bool) {
        self.job(Job::UpdateSettings, move |link| {
            let result = link.one_mk(&["update", if automatic { "--install-automatically" } else { "--check-on-start" }, if enabled { "on" } else { "off" }]);
            link.update_status();
            result.map(drop)
        });
    }

    pub fn download_update(&self) {
        self.ctx.open_url(egui::OpenUrl::new_tab("https://github.com/samer2373/one-mk/releases/latest"));
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
        self.setting(what, args);
    }

    pub fn setting(&self, what: &str, args: Vec<String>) {
        self.tune(Job::Set { what: what.into(), args: args.clone() }, vec![args], None);
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
        let out = proc::hide_window(&mut cmd).output().map_err(|e| format!("oneMK's service didn't start: {e}"))?;
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
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(path) = std::env::var_os("ONE_MK_APP_INPUT_DEVICES") {
            // Development aid: a list to show instead of the real one.
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            return view::parse_input_devices(&text).ok_or_else(|| "could not read the list".into());
        }
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["input-devices", "--json"]).args(self.home_args()).stdin(Stdio::null());
        let out = proc::hide_window(&mut cmd).output().map_err(|e| format!("oneMK's service didn't start: {e}"))?;
        view::parse_input_devices(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim().trim_start_matches("one-mk: ");
            if err.is_empty() { "oneMK gave no list".to_owned() } else { err.to_owned() }
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
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(path) = std::env::var_os("ONE_MK_APP_DOCTOR") {
            // Development aid: a report to show instead of a real check.
            std::thread::sleep(Duration::from_millis(600));
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            return view::parse_doctor(&text).ok_or_else(|| "could not read the report".into());
        }
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["doctor", "--json"]).args(self.home_args()).stdin(Stdio::null());
        // It says what it found on stdout, also when something failed.
        let out = proc::hide_window(&mut cmd).output().map_err(|e| format!("oneMK's service didn't start: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        view::parse_doctor(&text).ok_or_else(|| {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim().trim_start_matches("one-mk: ");
            if err.is_empty() { "The connection check gave no report. Try again.".to_owned() } else { view::window_issue(err) }
        })
    }

    /// One request to the running daemon; its error if it refused.
    fn ask(&self, req: Request) -> anyhow::Result<()> {
        let timeout = req.reply_timeout();
        let Some(reply) = client::request(&self.dir, req, timeout)? else { anyhow::bail!("oneMK is not running") };
        let reply = reply.trim();
        match reply.strip_prefix("error") {
            Some(e) => anyhow::bail!("{}", e.trim_start_matches([' ', ':'])),
            None => Ok(()),
        }
    }

    /// Puts the menu-bar / tray icon up if it isn't, so the window is never
    /// the only way in. The tray exits by itself when one is already there.
    fn show_tray(&self) {
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
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
            Some(r) if !r.starts_with("ok") => anyhow::bail!("this oneMK is older than the app"),
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
                l.login_launch().and_then(|launch| install::install(&launch))
            } else {
                install::uninstall()
            };
            l.send(Event::Autostart(install::is_installed_for(l.login_home().as_deref())));
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

/// `settings.toml`, refused over the daemon's size limit as the daemon
/// refuses it: the window then says it can't be read rather than showing
/// values the daemon is not using.
fn settings_text(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?.take(one_mk_ctl::MAX_SETTINGS_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > one_mk_ctl::MAX_SETTINGS_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "settings exceeds the 4 MiB limit"));
    }
    Ok(text)
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
    let bad = || "Could not read history. Update oneMK and try Refresh.".to_owned();
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

impl one_mk_ctl::setup::Host for Link {
    /// First-run setup only adds a login entry where there is none: one
    /// for another settings folder (the usual one, seen from a test copy)
    /// is left alone rather than replaced unasked.
    fn login_installed(&self) -> bool {
        install::is_installed()
    }

    fn install_login(&self) -> Result<(), String> {
        let launch = self.login_launch().map_err(|e| format!("{e:#}"))?;
        install::install(&launch).map(drop).map_err(|e| format!("{e:#}"))
    }

    fn integrations_present(&self) -> bool {
        one_mk_ctl::integrate::is_enabled()
    }

    fn install_integrations(&self) -> Result<bool, String> {
        let launch = self.login_launch().map_err(|e| format!("{e:#}"))?;
        let setup = one_mk_ctl::integrate::Setup {
            exe: launch.exe,
            home: launch.home,
            // Pairing refreshes the entries for each computer it adds.
            computers: vec![],
        };
        one_mk_ctl::integrate::install(&setup).map(|o| o.found).map_err(|e| format!("{e:#}"))
    }

    fn running(&self) -> bool {
        client::is_running(&self.dir)
    }

    fn wait_running(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if client::is_running(&self.dir) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    fn start(&self) -> Result<(), String> {
        let mut args = vec!["run".to_owned()];
        args.extend(self.home_args());
        proc::spawn_detached(&self.daemon, &args, Some(&self.dir.join("daemon.log"))).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline && !client::is_running(&self.dir) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if client::is_running(&self.dir) {
            Ok(())
        } else {
            Err(format!("It didn't answer within 4 seconds. Details are in {}.", self.dir.join("daemon.log").display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_or_unfocused_window_looks_at_the_daemon_less_often() {
        assert_eq!(Attention::of(None, None), Attention::Active);
        assert_eq!(Attention::of(Some(false), Some(true)), Attention::Active);
        assert_eq!(Attention::of(Some(false), Some(false)), Attention::Background);
        assert_eq!(Attention::of(Some(true), Some(true)), Attention::Hidden);
        assert_eq!(Attention::of(Some(true), Some(false)), Attention::Hidden);
        assert!(Attention::Active.poll() <= Duration::from_millis(250));
        assert!(Attention::Background.poll() > Attention::Active.poll());
        assert!(Attention::Hidden.poll() >= Duration::from_secs(2));
    }

    #[test]
    fn a_window_that_comes_back_wakes_the_watcher_at_once() {
        let showing = Showing::new();
        showing.attend(Attention::Hidden);
        let watcher = showing.clone();
        let napping = std::thread::spawn(move || {
            let started = Instant::now();
            watcher.nap();
            started.elapsed()
        });
        std::thread::sleep(Duration::from_millis(200));
        showing.attend(Attention::Active);
        let slept = napping.join().unwrap();
        assert!(slept < Duration::from_secs(2), "slept the whole hidden pause: {slept:?}");
    }

    #[test]
    fn a_graph_drawn_on_the_slow_redraw_still_counts_as_on_screen() {
        // The window draws a steady graph once per GRAPH_IDLE_REDRAW: the
        // watcher must not take the quiet between two frames for a hidden one.
        assert!(SHOWING_FOR > GRAPH_IDLE_REDRAW);
        let showing = Showing::new();
        assert!(!showing.on_screen(), "nothing drawn yet");
        showing.frame(true);
        assert!(showing.on_screen());
        showing.attend(Attention::Hidden);
        assert!(!showing.on_screen(), "minimised: no more asking");
        showing.attend(Attention::Active);
        assert!(showing.on_screen());
        showing.frame(false);
        assert!(!showing.on_screen());
    }

    #[test]
    fn the_unfocused_pause_runs_its_full_length_when_nothing_changes() {
        let showing = Showing::new();
        showing.attend(Attention::Background);
        let started = Instant::now();
        showing.nap();
        assert!(started.elapsed() >= Duration::from_millis(450));
    }

    #[test]
    fn settings_over_the_daemons_limit_are_unreadable_here_too() {
        let dir = std::env::temp_dir().join(format!("one-mk-app-oversized-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");
        let mut text = "notify_files = false\n".to_owned();
        text.push_str(&"#".repeat(one_mk_ctl::MAX_SETTINGS_BYTES as usize + 1 - text.len()));
        std::fs::write(&path, &text).unwrap();
        let Err(error) = settings_text(&path) else { panic!("a file over the limit must be refused") };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        text.pop();
        std::fs::write(&path, &text).unwrap();
        assert_eq!(settings_text(&path).unwrap().len() as u64, one_mk_ctl::MAX_SETTINGS_BYTES, "exactly at the limit still reads");
        assert_eq!(settings_text(&dir.join("missing.toml")).unwrap_err().kind(), std::io::ErrorKind::NotFound);
        std::fs::remove_dir_all(dir).unwrap();
    }

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

/// This program's own file. After an update replaced it on disk Linux reports
/// the old file as "<path> (deleted)", which is not a path anything can start:
/// the file now at the same place is the updated program.
fn running_executable() -> std::io::Result<PathBuf> {
    Ok(without_deleted_suffix(std::env::current_exe()?))
}

fn without_deleted_suffix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_suffix(" (deleted)")) {
        Some(original) => PathBuf::from(original),
        None => path,
    }
}

#[cfg(test)]
mod replaced_executable_tests {
    use super::*;

    #[test]
    fn a_program_replaced_by_an_update_is_started_from_the_path_it_had() {
        assert_eq!(without_deleted_suffix(PathBuf::from("/usr/bin/one-mk-app (deleted)")), PathBuf::from("/usr/bin/one-mk-app"));
        assert_eq!(without_deleted_suffix(PathBuf::from("/usr/bin/one-mk-app")), PathBuf::from("/usr/bin/one-mk-app"));
    }
}
