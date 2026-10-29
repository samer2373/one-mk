//! The window's link to one-mk: it watches the daemon over the local
//! control channel and changes things through the `one-mk` program, the
//! same way the command line and the tray do. Every slow step runs on its
//! own thread and reports back as an [`Event`], waking the window.

use one_mk_ctl::client::{self, Request};
use one_mk_ctl::install::{self, Launch};
use one_mk_ctl::proc;
use one_mk_ctl::status::Status;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// How often the window looks at the daemon while it is open. A status
/// request is one small message over a local socket.
const POLL: Duration = Duration::from_millis(250);

pub enum Event {
    /// `None`: one-mk is not running.
    Status(Option<Status>),
    Autostart(bool),
    /// A command finished: what it was for, and its error if it failed.
    Done { what: Job, error: Option<String> },
    Latency(Latency),
}

/// A command the window started, with what it needs to report on it.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
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
}

impl Link {
    pub fn new(home: Option<PathBuf>, tx: Sender<Event>, ctx: egui::Context) -> anyhow::Result<Self> {
        let custom_home = home.is_some();
        let dir = match home {
            Some(h) => h,
            None => one_mk_ctl::default_home()?,
        };
        let me = std::env::current_exe()?;
        let daemon = install::sibling(&me, "one-mk").unwrap_or_else(|| PathBuf::from("one-mk"));
        let tray = install::sibling(&me, "one-mk-tray");
        Ok(Self { dir, custom_home, daemon, tray, tx, ctx })
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
            let mut last: Option<Option<Status>> = None;
            let mut gone = 0;
            let opened = std::time::SystemTime::now();
            loop {
                // The tray's "Quit one·mk" writes this file: the window goes too.
                let quit = std::fs::metadata(link.dir.join(client::QUIT_FILE)).and_then(|m| m.modified());
                if quit.is_ok_and(|t| t > opened) {
                    link.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                if last.as_ref() != Some(&now) {
                    if link.tx.send(Event::Status(now.clone())).is_err() {
                        return;
                    }
                    link.ctx.request_repaint();
                    last = Some(now);
                }
                std::thread::sleep(POLL);
            }
        });
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
        let mut cmd = Command::new(&self.daemon);
        cmd.args(args).args(self.home_args()).stdin(Stdio::null());
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

    pub fn set_side(&self, peer: &str, name: &str, side: one_mk_proto::Side) {
        let (id, word) = (peer.to_owned(), side.to_string());
        self.job(Job::Layout { id: peer.into(), name: name.into(), side }, move |l| l.one_mk(&["layout", &id, &word]).map(drop));
    }

    pub fn set_swap(&self, peer: &str, name: &str, swap: &str) {
        let (id, value) = (peer.to_owned(), swap.to_owned());
        let what = Job::Swap { id: peer.into(), name: name.into(), swap: swap.into() };
        self.job(what, move |l| l.one_mk(&["layout", &id, "--swap-ctrl-cmd", &value]).map(drop));
    }

    /// Puts the menu-bar / tray icon up if it isn't, so the window is never
    /// the only way in. The tray exits by itself when one is already there.
    fn show_tray(&self) {
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
        self.job(Job::Stop, |l| client::request(&l.dir, Request::Stop, Duration::from_secs(5)).map(drop));
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
