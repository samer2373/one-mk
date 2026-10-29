//! Everything the tray does, through the same channels the command line
//! uses: the control socket, the `one-mk` program and the login settings.

use crate::model::{Action, Snapshot};
use one_mk_ctl::client::{self, Request, SwitchTo, Toggle};
use one_mk_ctl::install::{self, Launch};
use one_mk_ctl::proc;
use one_mk_ctl::status::Status;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

pub struct Ctx {
    /// Where settings live, and the daemon's socket.
    pub dir: PathBuf,
    /// The user chose the folder (`--home` or `ONE_MK_HOME`): pass it on.
    pub custom_home: bool,
    /// The `one-mk` program.
    pub daemon: PathBuf,
    /// This program.
    pub tray: PathBuf,
}

impl Ctx {
    pub fn new(home: Option<PathBuf>) -> anyhow::Result<Self> {
        let custom_home = home.is_some();
        let dir = match home {
            Some(h) => h,
            None => one_mk_ctl::default_home()?,
        };
        let tray = std::env::current_exe()?;
        let daemon = install::sibling(&tray, "one-mk").unwrap_or_else(|| PathBuf::from("one-mk"));
        Ok(Self { dir, custom_home, daemon, tray })
    }

    fn home_args(&self) -> Vec<String> {
        if self.custom_home { vec!["--home".into(), install::absolute(&self.dir).display().to_string()] } else { vec![] }
    }

    /// A fresh look at the daemon. Never blocks for long.
    pub fn snapshot(&self) -> Snapshot {
        let status = client::request(&self.dir, Request::Status, Duration::from_millis(1500))
            .ok()
            .flatten()
            .and_then(|t| Status::decode(&t));
        Snapshot { status, autostart: install::is_installed() }
    }

    pub fn start_daemon(&self) -> std::io::Result<()> {
        if client::is_running(&self.dir) {
            return Ok(());
        }
        let mut args = vec!["run".to_owned()];
        args.extend(self.home_args());
        proc::spawn_detached(&self.daemon, &args, Some(&self.dir.join("daemon.log")))?;
        // Give it a moment to open its control channel, so the menu is right.
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline && !client::is_running(&self.dir) {
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    /// Opens the settings window (`one-mk-app`, installed next to the tray).
    fn open_app(&self) -> std::io::Result<()> {
        let app = install::sibling(&self.tray, "one-mk-app").unwrap_or_else(|| PathBuf::from("one-mk-app"));
        proc::spawn_detached(&app, &self.home_args(), None)
    }

    /// Does what a menu item says. Failures go to stderr (there is no
    /// window to show them in); the menu is rebuilt afterwards either way.
    pub fn perform(&self, action: &Action) {
        let result: anyhow::Result<()> = match action {
            Action::OpenApp => self.open_app().map_err(Into::into),
            Action::StartDaemon => self.start_daemon().map_err(Into::into),
            Action::TakeBack => client::request(&self.dir, Request::TakeBack, Duration::from_secs(5)).map(drop),
            Action::SetSide { peer, side } => self.set_side(peer, &side.to_string()),
            Action::Switch(to) => {
                let to = if to == "here" { SwitchTo::Here } else { SwitchTo::Device(to.clone()) };
                self.ask(Request::Switch(to))
            }
            Action::LockPointer(on) => self.ask(Request::LockPointer(if *on { Toggle::On } else { Toggle::Off })),
            Action::LockAll => self.ask(Request::LockAll),
            Action::ClipSend(to) => self.ask(Request::ClipSend(Some(to.clone()))),
            Action::ToggleAutostart => self.toggle_autostart(),
            Action::Quit => Ok(()),
        };
        if let Err(e) = result {
            eprintln!("one-mk-tray: {e:#}");
        }
    }

    /// One request to the daemon; its refusal as an error.
    fn ask(&self, req: Request) -> anyhow::Result<()> {
        let timeout = req.reply_timeout();
        match client::request(&self.dir, req, timeout)? {
            None => anyhow::bail!("one-mk is not running"),
            Some(r) if r.starts_with("error") => anyhow::bail!("{}", r.trim_start_matches("error").trim()),
            Some(_) => Ok(()),
        }
    }

    fn set_side(&self, peer: &str, side: &str) -> anyhow::Result<()> {
        // `one-mk layout` edits the device list and tells the daemon to reload it.
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["layout", peer, side]).args(self.home_args());
        let out = proc::hide_window(&mut cmd).output()?;
        if !out.status.success() {
            anyhow::bail!("one-mk layout failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    fn toggle_autostart(&self) -> anyhow::Result<()> {
        if install::is_installed() {
            install::uninstall()?;
        } else {
            if !self.daemon.is_absolute() {
                anyhow::bail!("cannot find one-mk next to the tray; run `one-mk install` instead");
            }
            let running = client::is_running(&self.dir);
            install::install(&Launch {
                exe: self.daemon.clone(),
                tray: Some(self.tray.clone()),
                home: self.custom_home.then(|| install::absolute(&self.dir)),
                // Do not start a second copy of a daemon that is already up.
                start_now: !running,
            })?;
        }
        Ok(())
    }
}
