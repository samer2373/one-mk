//! one-mk: share one mouse and keyboard across your computers.

mod app;
mod bench;
mod clip;
mod link;
mod pair;
mod send;
mod transfer;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use one_mk_net::{Identity, TrustStore};
use one_mk_proto::{DEFAULT_PORT, Side};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "one-mk", version, about = "Share one mouse and keyboard across your computers")]
struct Cli {
    /// Where settings and keys live (default: your config folder).
    #[arg(long, env = "ONE_MK_HOME", global = true)]
    home: Option<PathBuf>,
    /// More logging (-v debug, -vv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the sharing service (the default).
    Run {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Also try these addresses (for networks that block discovery).
        #[arg(long)]
        connect: Vec<SocketAddr>,
        /// Don't advertise or look for computers on the local network.
        #[arg(long)]
        no_discovery: bool,
        #[arg(long, value_enum, default_value_t = BackendArg::Native)]
        backend: BackendArg,
        /// Don't share the clipboard.
        #[arg(long)]
        no_clipboard: bool,
        /// Where received files go (default: Downloads/one-mk).
        #[arg(long, value_name = "DIR")]
        downloads: Option<PathBuf>,
    },
    /// Send files or folders to a paired computer running `one-mk run`.
    Send {
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Name or id of the paired computer (needed if there are several).
        #[arg(long, value_name = "DEVICE")]
        to: Option<String>,
        /// Also try these addresses (for networks that block discovery).
        #[arg(long)]
        connect: Vec<SocketAddr>,
    },
    /// Pair with another computer (run it on both).
    Pair {
        /// Pair with this address instead of searching the network.
        #[arg(long)]
        connect: Option<SocketAddr>,
    },
    /// List paired computers.
    Devices,
    /// Say which side of this computer a paired computer is on.
    Layout {
        /// Name or id of the paired computer.
        device: String,
        side: Side,
    },
    /// Unpair a computer.
    Forget { device: String },
    /// Show this computer's name and id.
    Id,
    /// Measure latency and CPU cost.
    Bench {
        #[arg(long, default_value_t = 5000)]
        count: u32,
        /// Events per second (gaming mice report 1000-8000).
        #[arg(long, default_value_t = 1000)]
        rate: u32,
        /// Echo for a bench on another machine.
        #[arg(long, value_name = "PORT", num_args = 0..=1, default_missing_value = "24901")]
        serve: Option<u16>,
        /// Measure against `one-mk bench --serve` on another machine.
        #[arg(long)]
        connect: Option<SocketAddr>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Native,
    /// No real input; for testing the network side.
    Null,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "info,mdns_sd=warn,zbus=warn,quinn=warn,quinn_proto=warn",
        1 => "debug,mdns_sd=warn,zbus=info,quinn=info,quinn_proto=info,rustls=info",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()),
        )
        .with_target(false)
        .init();

    let dir = match cli.home {
        Some(d) => d,
        None => directories::ProjectDirs::from("", "", "one-mk")
            .context("no home directory")?
            .config_dir()
            .to_path_buf(),
    };

    // One thread for networking is plenty and keeps the footprint small;
    // input capture and injection have their own threads.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async move {
        match cli.cmd.unwrap_or(Cmd::Run {
            port: DEFAULT_PORT,
            connect: Vec::new(),
            no_discovery: false,
            backend: BackendArg::Native,
            no_clipboard: false,
            downloads: None,
        }) {
            Cmd::Run { port, connect, no_discovery, backend, no_clipboard, downloads } => {
                let backend = match backend {
                    BackendArg::Native => one_mk_input::Kind::Native,
                    BackendArg::Null => one_mk_input::Kind::Null,
                };
                let opts = app::Options {
                    backend,
                    port,
                    connect,
                    discovery: !no_discovery,
                    clipboard: (!no_clipboard).then(one_mk_clipboard::Clipboard::system),
                    downloads: match downloads {
                        Some(d) => d,
                        None => default_downloads()?,
                    },
                };
                app::run(dir, opts).await
            }
            Cmd::Send { paths, to, connect } => send::run(&dir, paths, to, connect).await,
            Cmd::Pair { connect } => pair::run(&dir, connect).await,
            Cmd::Devices => devices(&dir),
            Cmd::Layout { device, side } => layout(&dir, &device, side),
            Cmd::Forget { device } => forget(&dir, &device),
            Cmd::Id => {
                let id = Identity::load_or_create(&dir)?;
                println!("{} ({})", id.name, id.fingerprint.short());
                println!("fingerprint {}", id.fingerprint);
                Ok(())
            }
            Cmd::Bench { count, rate, serve, connect } => {
                bench::run(bench::Options { count, rate: rate.max(1), serve, connect }).await
            }
        }
    })
}

fn default_downloads() -> Result<PathBuf> {
    let dirs = directories::UserDirs::new().context("no home directory")?;
    let downloads = dirs.download_dir().map_or_else(|| dirs.home_dir().join("Downloads"), |d| d.to_path_buf());
    Ok(downloads.join("one-mk"))
}

fn devices(dir: &std::path::Path) -> Result<()> {
    let trust = TrustStore::load(dir)?;
    if trust.devices().is_empty() {
        println!("No paired computers. Run `one-mk pair` on this and the other computer.");
    }
    for d in trust.devices() {
        println!("{:<24} {}  {:<6}  {}", d.name, d.short_id(), d.side, d.addrs.first().map_or("", String::as_str));
    }
    Ok(())
}

fn layout(dir: &std::path::Path, device: &str, side: Side) -> Result<()> {
    let mut trust = TrustStore::load(dir)?;
    let fp = trust.find(device).with_context(|| format!("no paired computer called {device:?}"))?.fingerprint()
        .context("corrupt fingerprint")?;
    let dev = trust.get_mut(&fp).expect("found above");
    dev.side = side.to_string();
    let name = dev.name.clone();
    trust.save()?;
    println!("{name} is now to the {side} of this computer.");
    println!("On {name}, this computer should be set to the {}. Restart `one-mk run` to apply.", side.opposite());
    Ok(())
}

fn forget(dir: &std::path::Path, device: &str) -> Result<()> {
    let mut trust = TrustStore::load(dir)?;
    let d = trust.remove(device).with_context(|| format!("no paired computer called {device:?}"))?;
    trust.save()?;
    println!("Forgot {}.", d.name);
    Ok(())
}
