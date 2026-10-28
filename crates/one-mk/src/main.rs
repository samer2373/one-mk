//! one-mk: share one mouse and keyboard across your computers.

mod app;
mod bench;
mod clip;
mod clipctl;
mod control;
mod doctor;
mod install;
mod link;
mod manage;
mod notify;
mod pair;
mod send;
mod settings;
mod steer;
mod track;
mod transfer;

use anyhow::{Context, Result};
use clipctl::OnOff;
use clap::{Args, Parser, Subcommand, ValueEnum};
use one_mk_net::{Identity, Swap, TrustStore};
use one_mk_proto::{DEFAULT_PORT, Side};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "one-mk",
    version,
    about = "Share one mouse and keyboard across your computers"
)]
struct Cli {
    /// Where settings and keys live (default: your config folder).
    #[arg(long, env = "ONE_MK_HOME", global = true)]
    home: Option<PathBuf>,
    /// More detail: show full errors and debug logs (-vv for trace logs).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    /// Options for `run`, which is what plain `one-mk` does.
    #[command(flatten)]
    run: RunArgs,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

/// Options for `run`. They are accepted before the subcommand
/// (`one-mk --backend null run`), after it (`one-mk run --backend null`) and,
/// with no subcommand at all, on their own; given in both places, the ones
/// after `run` win. Nothing has a clap default, so "not given" can be told
/// from "given" when the two are merged (see [`RunArgs::over`]).
#[derive(Args, Debug, Default, PartialEq)]
struct RunArgs {
    /// UDP port to listen on (default: 24900).
    #[arg(long)]
    port: Option<u16>,
    /// Also try this address, e.g. 192.168.1.20:24800 (for networks that block discovery; repeatable).
    #[arg(long, value_name = "IP:PORT")]
    connect: Vec<SocketAddr>,
    /// Don't advertise or look for computers on the local network.
    #[arg(long)]
    no_discovery: bool,
    /// Input backend (default: native).
    #[arg(long, value_enum)]
    backend: Option<Backend>,
    /// Don't share the clipboard.
    #[arg(long)]
    no_clipboard: bool,
    /// Where received files go (default: Downloads/one-mk).
    #[arg(long, value_name = "DIR")]
    downloads: Option<PathBuf>,
    /// Start with the pointer locked to this computer (see `lock-pointer`).
    #[arg(long)]
    start_locked: bool,
    /// Don't show a notification when files arrive (for this run; `one-mk notify off` is permanent).
    #[arg(long)]
    no_notify: bool,
}

impl RunArgs {
    /// `self` (given after `run`) on top of `outer` (given before it): what
    /// `self` leaves out comes from `outer`, and repeatable options add up.
    fn over(self, mut outer: RunArgs) -> RunArgs {
        outer.connect.extend(self.connect);
        RunArgs {
            port: self.port.or(outer.port),
            connect: outer.connect,
            no_discovery: self.no_discovery || outer.no_discovery,
            backend: self.backend.or(outer.backend),
            no_clipboard: self.no_clipboard || outer.no_clipboard,
            downloads: self.downloads.or(outer.downloads),
            start_locked: self.start_locked || outer.start_locked,
            no_notify: self.no_notify || outer.no_notify,
        }
    }
}

/// What the command line asks for.
enum Action {
    /// `one-mk run`, which is what plain `one-mk` does.
    Run(RunArgs),
    Other(Cmd),
}

impl Action {
    /// `outer` is what came before the subcommand. Options for `run` in front
    /// of another command are an error, not ignored: a `--backend null` that
    /// silently did nothing would be a nasty surprise.
    fn of(outer: RunArgs, cmd: Option<Cmd>) -> Result<Action> {
        match cmd {
            None => Ok(Action::Run(outer)),
            Some(Cmd::Run(r)) => Ok(Action::Run(r.over(outer))),
            Some(other) if outer == RunArgs::default() => Ok(Action::Other(other)),
            Some(_) => anyhow::bail!("those options are for `one-mk run`; put them after it (see `one-mk run --help`)"),
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the sharing service (the default).
    Run(RunArgs),
    /// Send files or folders to a paired computer running `one-mk run`.
    Send {
        /// Files or folders to send.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Name or id of the paired computer (needed if there are several).
        #[arg(long, value_name = "DEVICE")]
        to: Option<String>,
        /// Also try this address, e.g. 192.168.1.20:24800 (for networks that block discovery; repeatable).
        #[arg(long, value_name = "IP:PORT")]
        connect: Vec<SocketAddr>,
    },
    /// Show or change what the clipboard shares: which formats, and how much (applies at once).
    Clipboard {
        /// Share plain text.
        #[arg(long, value_name = "on|off")]
        text: Option<OnOff>,
        /// Share HTML (rich text).
        #[arg(long, value_name = "on|off")]
        html: Option<OnOff>,
        /// Share images.
        #[arg(long, value_name = "on|off")]
        image: Option<OnOff>,
        /// Largest clipboard to send or accept, in MB (default 25).
        #[arg(long, value_name = "N")]
        max_mb: Option<u32>,
        /// Skip what a password manager marked as sensitive (default on; not
        /// detectable on every system yet, see the README).
        #[arg(long, value_name = "on|off")]
        skip_sensitive: Option<OnOff>,
        /// Let other computers fetch this clipboard with `one-mk clip-get` (default on).
        #[arg(long, value_name = "on|off")]
        allow_fetch: Option<OnOff>,
    },
    /// Send this computer's clipboard to one paired computer now, without moving the pointer.
    ClipSend {
        /// Name or id of the paired computer (needed if several are connected).
        #[arg(long, value_name = "DEVICE")]
        to: Option<String>,
    },
    /// Copy a paired computer's clipboard to this one now.
    ClipGet {
        /// Name or id of the paired computer (needed if several are connected).
        #[arg(long, value_name = "DEVICE")]
        from: Option<String>,
    },
    /// Show or set whether a notification appears when files arrive.
    Notify {
        /// on or off; leave out to see the current setting.
        state: Option<OnOff>,
    },
    /// Pair with another computer (run it on both).
    Pair {
        /// Pair with the computer at this address instead of searching the
        /// network, e.g. 192.168.1.20:40123 (the other side prints its pairing port).
        #[arg(long, value_name = "IP:PORT")]
        connect: Option<SocketAddr>,
        /// One event per line for a program to read (the app window uses this).
        #[arg(long, hide = true)]
        machine: bool,
    },
    /// List paired computers.
    Devices,
    /// Show whether one-mk is running, who is connected and who controls whom.
    Status,
    /// Take this computer back from the computer controlling it (on Linux,
    /// where using this computer's own mouse or keyboard can't be noticed;
    /// bind it to a keyboard shortcut).
    Takeback,
    /// Start one-mk automatically when you log in.
    Install,
    /// Stop starting one-mk automatically.
    Uninstall,
    /// Say which side of this computer a paired computer is on (applies at
    /// once if one-mk is running) and how Ctrl/Cmd are treated.
    Layout {
        /// Name or id of the paired computer (see `one-mk devices`).
        device: String,
        /// Where it is: left, right, top or bottom.
        side: Option<Side>,
        /// Swap Ctrl and Cmd for keys sent to it, so copy and paste feel the same
        /// between a Mac and a PC (default: auto, which swaps only between a Mac and another computer).
        #[arg(long, value_name = "on|off|auto")]
        swap_ctrl_cmd: Option<Swap>,
    },
    /// Unpair a computer.
    Forget {
        /// Name or id of the paired computer (see `one-mk devices`).
        device: String,
    },
    /// Move the mouse and keyboard to another computer without crossing an
    /// edge: a paired computer's name or id, `here`, or `next` (round and round).
    Switch {
        /// Name or id, `here` or `next`.
        #[arg(value_name = "COMPUTER|here|next")]
        to: String,
    },
    /// Keep the pointer on this computer: edges lead nowhere while on.
    LockPointer {
        #[arg(value_name = "on|off|toggle", value_parser = ["on", "off", "toggle"])]
        state: String,
    },
    /// Show or change the rules for crossing edges: dead corners, a dwell
    /// time, a key to hold, and not leaving a fullscreen window.
    Edge(settings::EdgeArgs),
    /// Gaming mode: on another computer, send raw mouse movement (for games)
    /// instead of positions.
    Gaming {
        #[arg(value_enum)]
        mode: settings::Gaming,
    },
    /// Lock this computer's screen and every connected computer's.
    LockAll,
    /// Whether locking this computer locks the connected ones too: on or off.
    LockSync {
        #[arg(value_name = "on|off", value_parser = settings::parse_on_off, action = clap::ArgAction::Set)]
        state: bool,
    },
    /// Show this computer's name and id.
    Id,
    /// Find out why computers are not connecting, and how to fix it.
    Doctor {
        /// Machine-readable output for a program to read.
        #[arg(long)]
        json: bool,
    },
    /// Measure latency and CPU cost.
    Bench {
        /// Number of mouse events to send.
        #[arg(long, default_value_t = 5000)]
        count: u32,
        /// Events per second (gaming mice report 1000-8000).
        #[arg(long, default_value_t = 1000)]
        rate: u32,
        /// Echo for a bench on another machine.
        #[arg(long, value_name = "PORT", num_args = 0..=1, default_missing_value = "24901")]
        serve: Option<u16>,
        /// Measure against `one-mk bench --serve` on another machine, e.g. 192.168.1.20:24901.
        #[arg(long, value_name = "IP:PORT")]
        connect: Option<SocketAddr>,
        /// Measure against the running `one-mk run` on a paired computer (name or id).
        #[arg(long, value_name = "DEVICE", conflicts_with_all = ["serve", "connect"])]
        peer: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Backend {
    /// This system's real mouse and keyboard.
    Native,
    /// No real input; for testing the network side.
    Null,
}

impl From<Backend> for one_mk_input::Kind {
    fn from(b: Backend) -> Self {
        match b {
            Backend::Native => Self::Native,
            Backend::Null => Self::Null,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let verbose = cli.verbose;
    let level = match verbose {
        0 => "warn,mdns_sd=error,zbus=error",
        1 => "debug,mdns_sd=warn,zbus=info,quinn=info,quinn_proto=info,rustls=info",
        _ => "trace",
    };
    let log = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()),
        )
        .with_target(false);
    // Plain lines unless the user asked for details.
    if verbose == 0 {
        log.without_time().init();
    } else {
        log.init();
    }

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if verbose > 0 {
                eprintln!("one-mk: {e:?}");
            } else {
                // One line; -v shows each cause on its own.
                eprintln!("one-mk: {e:#}");
            }
            ExitCode::FAILURE
        }
    }
}

/// What `run` is started with.
fn options(r: RunArgs) -> Result<app::Options> {
    Ok(app::Options {
        backend: r.backend.unwrap_or(Backend::Native).into(),
        port: r.port.unwrap_or(DEFAULT_PORT),
        connect: r.connect,
        discovery: !r.no_discovery,
        clipboard: (!r.no_clipboard).then(one_mk_clipboard::Clipboard::system),
        downloads: match r.downloads {
            Some(d) => d,
            None => default_downloads()?,
        },
        start_locked: r.start_locked,
    })
}

fn run(cli: Cli) -> Result<()> {
    let custom_home = cli.home.is_some();
    let verbose = cli.verbose;
    let action = Action::of(cli.run, cli.cmd)?;
    let dir = match cli.home {
        Some(d) => d,
        None => default_home()?,
    };

    // One thread for networking is plenty and keeps the footprint small;
    // input capture and injection have their own threads.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async move {
        let cmd = match action {
            Action::Run(r) => {
                if r.no_notify {
                    notify::disable();
                }
                return app::run(dir, options(r)?).await;
            }
            Action::Other(cmd) => cmd,
        };
        match cmd {
            Cmd::Run(_) => unreachable!("handled above"),
            Cmd::Clipboard { text, html, image, max_mb, skip_sensitive, allow_fetch } => {
                clipctl::clipboard(&dir, clipctl::ClipChange { text, html, image, max_mb, skip_sensitive, allow_fetch }).await
            }
            Cmd::ClipSend { to } => clipctl::move_clipboard(&dir, to, true).await,
            Cmd::ClipGet { from } => clipctl::move_clipboard(&dir, from, false).await,
            Cmd::Notify { state } => clipctl::notify(&dir, state).await,
            Cmd::Send { paths, to, connect } => send::run(&dir, paths, to, connect).await,
            Cmd::Pair { connect, machine } => pair::run(&dir, connect, pair::Out::new(machine)).await,
            Cmd::Devices => devices(&dir, verbose > 0),
            Cmd::Status => manage::status(&dir).await,
            Cmd::Takeback => manage::takeback(&dir).await,
            Cmd::Install => install::install(&dir, custom_home).await,
            Cmd::Uninstall => install::uninstall(),
            Cmd::Layout { device, side, swap_ctrl_cmd } => manage::layout(&dir, &device, side, swap_ctrl_cmd).await,
            Cmd::Forget { device } => forget(&dir, &device),
            Cmd::Switch { to } => steer::switch(&dir, &to).await,
            Cmd::LockPointer { state } => {
                steer::lock_pointer(&dir, steer::Toggle::parse(&state).context("on, off or toggle")?).await
            }
            Cmd::Edge(args) => settings::edge(&dir, args).await,
            Cmd::Gaming { mode } => settings::gaming(&dir, mode).await,
            Cmd::LockAll => steer::lock_all(&dir).await,
            Cmd::LockSync { state } => settings::lock_sync(&dir, state).await,
            Cmd::Id => {
                let id = Identity::load_or_create(&dir)?;
                println!("{} ({})", id.name, id.fingerprint.short());
                println!("fingerprint {}", id.fingerprint);
                Ok(())
            }
            Cmd::Doctor { json } => doctor::run(&dir, json).await,
            Cmd::Bench { count, rate, serve, connect, peer } => {
                bench::run(&dir, bench::Options { count, rate: rate.max(1), serve, connect, peer }).await
            }
        }
    })
}

/// Where identity and paired devices live. On Windows this is the local
/// (non-roaming) AppData: the identity belongs to this one computer, and a
/// roaming profile would give every computer the same one.
fn default_home() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "one-mk").context("no home directory")?;
    let dir = dirs.config_local_dir().to_path_buf();
    #[cfg(windows)]
    migrate_roaming(dirs.config_dir(), &dir);
    Ok(dir)
}

/// Earlier versions kept their settings in the roaming AppData; move them
/// to the local one the first time.
#[cfg(windows)]
fn migrate_roaming(old: &Path, new: &Path) {
    if old == new || new.join("identity.key.der").exists() || !old.join("identity.key.der").exists() {
        return;
    }
    let moved = new.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(old, new));
    if let Err(e) = moved {
        // Different volumes or a partial new folder: copy file by file.
        tracing::debug!("moving {} failed ({e}); copying", old.display());
        let copy = || -> std::io::Result<()> {
            std::fs::create_dir_all(new)?;
            for entry in std::fs::read_dir(old)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    std::fs::copy(entry.path(), new.join(entry.file_name()))?;
                }
            }
            std::fs::remove_dir_all(old)
        };
        if let Err(e) = copy() {
            tracing::warn!("could not move settings from {} to {}: {e}", old.display(), new.display());
            return;
        }
    }
    tracing::info!("moved settings from {} to {}", old.display(), new.display());
}

fn default_downloads() -> Result<PathBuf> {
    let dirs = directories::UserDirs::new().context("no home directory")?;
    let downloads = dirs.download_dir().map_or_else(|| dirs.home_dir().join("Downloads"), |d| d.to_path_buf());
    Ok(downloads.join("one-mk"))
}

fn devices(dir: &Path, verbose: bool) -> Result<()> {
    let trust = TrustStore::load(dir)?;
    if trust.devices().is_empty() {
        println!("No paired computers. Run `one-mk pair` on this and the other computer.");
        return Ok(());
    }
    if verbose {
        println!("{:<24} {:<9}  {:<6}  ADDRESS", "NAME", "ID", "SIDE");
    } else {
        println!("{:<24} {:<9}  SIDE", "NAME", "ID");
    }
    for d in trust.devices() {
        let name = one_mk_proto::sanitize_name(&d.name);
        if verbose {
            println!("{name:<24} {:<9}  {:<6}  {}", d.short_id(), d.side, d.addrs.join(", "));
        } else {
            println!("{name:<24} {:<9}  {}", d.short_id(), d.side);
        }
    }
    Ok(())
}

fn forget(dir: &Path, device: &str) -> Result<()> {
    let mut trust = TrustStore::load(dir)?;
    let d = trust.remove(device).ok_or_else(|| manage::unknown_device(device))?;
    trust.save()?;
    println!("Forgot {}.", d.name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> RunArgs {
        let cli = Cli::try_parse_from(std::iter::once("one-mk").chain(args.iter().copied())).unwrap();
        match Action::of(cli.run, cli.cmd).unwrap() {
            Action::Run(r) => r,
            Action::Other(_) => panic!("not run: {args:?}"),
        }
    }

    /// What the options are in the end, for the settings that must not be lost.
    fn effective(args: &[&str]) -> (one_mk_input::Kind, u16, bool, bool, Vec<SocketAddr>) {
        let r = run_args(args);
        let clipboard = !r.no_clipboard;
        // Not the real clipboard: that would start a thread reading the desktop's.
        let o = options(RunArgs { no_clipboard: true, ..r }).unwrap();
        (o.backend, o.port, o.discovery, clipboard, o.connect)
    }

    #[test]
    fn run_options_mean_the_same_before_and_after_run() {
        let c: SocketAddr = "127.0.0.1:24901".parse().unwrap();
        let want = (one_mk_input::Kind::Null, 24901, false, false, vec![c]);
        for args in [
            &["--backend", "null", "--port", "24901", "--no-discovery", "--no-clipboard", "--connect", "127.0.0.1:24901", "-v", "run"][..],
            &["-v", "run", "--backend", "null", "--port", "24901", "--no-discovery", "--no-clipboard", "--connect", "127.0.0.1:24901"],
            &["--backend", "null", "--no-discovery", "-v", "run", "--port", "24901", "--no-clipboard", "--connect", "127.0.0.1:24901"],
            // No `run` at all is the same as `run`.
            &["--backend", "null", "--port", "24901", "--no-discovery", "--no-clipboard", "--connect", "127.0.0.1:24901"],
        ] {
            assert_eq!(effective(args), want, "{args:?}");
        }
        // Left out, the defaults.
        for args in [&["run"][..], &[]] {
            let d = effective(args);
            assert_eq!((d.0, d.1, d.2, d.3), (one_mk_input::Kind::Native, DEFAULT_PORT, true, true), "{args:?}");
        }
    }

    #[test]
    fn options_after_run_win_and_lists_add_up() {
        let r = run_args(&["--port", "1", "--connect", "10.0.0.1:1", "run", "--port", "2", "--connect", "10.0.0.2:2"]);
        assert_eq!(r.port, Some(2));
        assert_eq!(r.connect.len(), 2);
        let r = run_args(&["--backend", "null", "run", "--backend", "native"]);
        assert_eq!(r.backend, Some(Backend::Native));
    }

    #[test]
    fn run_options_before_another_command_are_refused() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from(args).unwrap();
            Action::of(cli.run, cli.cmd).map(|_| ())
        };
        assert!(parse(&["one-mk", "--backend", "null", "devices"]).is_err());
        assert!(parse(&["one-mk", "--no-discovery", "status"]).is_err());
        assert!(parse(&["one-mk", "--home", "/tmp/x", "-v", "devices"]).is_ok(), "global options are fine");
        assert!(parse(&["one-mk", "send", "--connect", "127.0.0.1:1", "f"]).is_ok(), "send has its own --connect");
    }
}
