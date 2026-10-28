//! one-mk: share one mouse and keyboard across your computers.

mod app;
mod bench;
mod clip;
mod control;
mod doctor;
mod install;
mod link;
mod manage;
mod pair;
mod send;
mod transfer;

use anyhow::{Context, Result};
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

#[derive(Args)]
struct RunArgs {
    /// UDP port to listen on.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Also try this address, e.g. 192.168.1.20:24800 (for networks that block discovery; repeatable).
    #[arg(long, value_name = "IP:PORT")]
    connect: Vec<SocketAddr>,
    /// Don't advertise or look for computers on the local network.
    #[arg(long)]
    no_discovery: bool,
    /// Input backend.
    #[arg(long, value_enum, default_value_t = Backend::Native)]
    backend: Backend,
    /// Don't share the clipboard.
    #[arg(long)]
    no_clipboard: bool,
    /// Where received files go (default: Downloads/one-mk).
    #[arg(long, value_name = "DIR")]
    downloads: Option<PathBuf>,
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

#[derive(Clone, Copy, ValueEnum)]
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

fn run(cli: Cli) -> Result<()> {
    let custom_home = cli.home.is_some();
    let dir = match cli.home {
        Some(d) => d,
        None => default_home()?,
    };

    // One thread for networking is plenty and keeps the footprint small;
    // input capture and injection have their own threads.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async move {
        match cli.cmd.unwrap_or(Cmd::Run(cli.run)) {
            Cmd::Run(r) => {
                let opts = app::Options {
                    backend: r.backend.into(),
                    port: r.port,
                    connect: r.connect,
                    discovery: !r.no_discovery,
                    clipboard: (!r.no_clipboard).then(one_mk_clipboard::Clipboard::system),
                    downloads: match r.downloads {
                        Some(d) => d,
                        None => default_downloads()?,
                    },
                };
                app::run(dir, opts).await
            }
            Cmd::Send { paths, to, connect } => send::run(&dir, paths, to, connect).await,
            Cmd::Pair { connect, machine } => pair::run(&dir, connect, pair::Out::new(machine)).await,
            Cmd::Devices => devices(&dir, cli.verbose > 0),
            Cmd::Status => manage::status(&dir).await,
            Cmd::Takeback => manage::takeback(&dir).await,
            Cmd::Install => install::install(&dir, custom_home).await,
            Cmd::Uninstall => install::uninstall(),
            Cmd::Layout { device, side, swap_ctrl_cmd } => manage::layout(&dir, &device, side, swap_ctrl_cmd).await,
            Cmd::Forget { device } => forget(&dir, &device),
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
