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

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Start,
    Layout,
    Swap,
    TakeBack,
    Autostart,
    Send { to: String, count: usize },
    Measure,
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

    fn status(&self) -> Option<Status> {
        client::request(&self.dir, Request::Status, Duration::from_millis(1500)).ok().flatten().and_then(|t| Status::decode(&t))
    }

    /// Watches the daemon until the window closes, reporting only changes.
    pub fn watch(&self) {
        let link = self.clone();
        std::thread::spawn(move || {
            link.send(Event::Autostart(install::is_installed()));
            let mut last: Option<Option<Status>> = None;
            loop {
                let now = link.status();
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
            Ok(())
        });
    }

    pub fn set_side(&self, peer: &str, side: one_mk_proto::Side) {
        let (peer, side) = (peer.to_owned(), side.to_string());
        self.job(Job::Layout, move |l| l.one_mk(&["layout", &peer, &side]).map(drop));
    }

    pub fn set_swap(&self, peer: &str, swap: &str) {
        let (peer, swap) = (peer.to_owned(), swap.to_owned());
        self.job(Job::Swap, move |l| l.one_mk(&["layout", &peer, "--swap-ctrl-cmd", &swap]).map(drop));
    }

    pub fn take_back(&self) {
        self.job(Job::TakeBack, |l| client::request(&l.dir, Request::TakeBack, Duration::from_secs(5)).map(drop));
    }

    pub fn set_autostart(&self, on: bool) {
        self.job(Job::Autostart, move |l| {
            if on {
                if !l.daemon.is_absolute() {
                    anyhow::bail!("cannot find one-mk next to this app; run `one-mk install` instead");
                }
                install::install(&Launch {
                    exe: l.daemon.clone(),
                    tray: l.tray.clone(),
                    home: l.custom_home.then(|| install::absolute(&l.dir)),
                    start_now: !client::is_running(&l.dir),
                })?;
            } else {
                install::uninstall()?;
            }
            l.send(Event::Autostart(install::is_installed()));
            Ok(())
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
        self.job(Job::Measure, move |l| {
            let out = l.one_mk(&["bench", "--peer", &peer, "--count", "400", "--rate", "1000"])?;
            let lat = parse_bench(&out, &peer).ok_or_else(|| anyhow::anyhow!("could not read the measurement"))?;
            l.send(Event::Latency(lat));
            Ok(())
        });
    }

    /// Opens the folder received files go to.
    pub fn open_downloads(&self) {
        let dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.join("one-mk")))
            .unwrap_or_else(|| self.dir.clone());
        let _ = std::fs::create_dir_all(&dir);
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        let _ = proc::spawn_detached(std::path::Path::new(opener), &[dir.display().to_string()], None);
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
