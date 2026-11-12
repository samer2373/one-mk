//! one-mk: share one mouse and keyboard across your computers.

#[cfg(all(feature = "wallpaper-m1-lab", not(debug_assertions)))]
compile_error!("presentation laboratory features require debug assertions");

macro_rules! print {
    ($($args:tt)*) => {{
        use std::io::Write as _;
        if let Err(error) = write!(std::io::stdout().lock(), $($args)*)
            && error.kind() != std::io::ErrorKind::BrokenPipe {
            panic!("failed printing to stdout: {error}");
        }
    }};
}

macro_rules! println {
    () => { print!("\n") };
    ($($args:tt)*) => { print!("{}\n", format_args!($($args)*)) };
}

mod app;
mod bench;
mod clip;
mod clipctl;
mod history;

fn readable_time(timestamp: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let seconds = now.saturating_sub(timestamp);
    let (count, unit) = if seconds < 60 { (seconds, "second") }
        else if seconds < 3600 { (seconds / 60, "minute") }
        else if seconds < 86400 { (seconds / 3600, "hour") }
        else { (seconds / 86400, "day") };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}
mod control;
mod doctor;
mod wallpaper;
mod pointer_extension;
#[cfg(test)]
mod testdir;
mod live_wallpaper;
mod hooks;
mod hotkeys;
mod inputdevices;
mod install;
mod integrate;
mod invite;
mod learn;
mod link;
mod manage;
mod netwatch;
mod notify;
mod pair;
mod pairing_layout;
mod pointeroverlay;
mod power;
mod profiles;
mod send;
mod drag;
mod settings;
mod settings_tolerant;
mod steer;
mod track;
mod transfer;
mod tuning;
mod update;
mod update_key;
mod typing;
#[cfg(feature = "wallpaper-m1-lab")]
mod wallpaper_import;
#[cfg(all(target_os = "linux", feature = "wallpaper-m1-lab"))]
mod wallpaper_authority;

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
    ///
    /// With another folder, pairing and forgetting leave your file manager's
    /// "Send to" entries alone; received files still go to Downloads (see
    /// `run --downloads`), and install, integrate, pointer-extension and
    /// wallpaper still change this desktop when you run them.
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
    /// Also try this address, e.g. 192.168.1.20:24900 (for networks that block discovery; repeatable).
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
    /// Check for or explicitly install a signed release; never updates in the background.
    Update(update::Options),
    /// Stop the sharing service for this settings folder before restarting it.
    Stop,
    /// Show or switch on file edge drops (experimental; off by default; not on Wayland).
    Drag {
        /// Switch file edge drops on: dropping a file on the screen edge facing another computer sends it there with no confirmation. Experimental: X11, Windows and macOS only, and only X11 has been tried (with scripted drags).
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Disable file edge-drop zones.
        #[arg(long)]
        off: bool,
        /// Width of the edge-drop zone, from 1 to 128 pixels.
        #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u16).range(1..=128))]
        zone: Option<u16>,
    },
    /// Show, or with --clear delete, the unfinished received files kept so an interrupted send can resume.
    Partials {
        /// Delete them all now (any not being received right now). Files older than 7 days are deleted by themselves.
        #[arg(long)]
        clear: bool,
        /// Where received files go, if one-mk is not running and you started it with `run --downloads` (default: Downloads/one-mk).
        #[arg(long, value_name = "DIR")]
        downloads: Option<PathBuf>,
    },
    /// Send files or folders to a paired computer running `one-mk run`.
    Send {
        /// Files or folders to send.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Name or id of the paired computer (needed if there are several).
        #[arg(long, value_name = "DEVICE")]
        to: Option<String>,
        /// Also try this address, e.g. 192.168.1.20:24900 (for networks that block discovery; repeatable).
        #[arg(long, value_name = "IP:PORT")]
        connect: Vec<SocketAddr>,
        /// Also show a desktop notification if sending fails (the file manager's "Send to" entries use this).
        #[arg(long)]
        notify_errors: bool,
    },
    /// Show or change what the clipboard shares: which formats, and how much (applies at once).
    Clipboard {
        #[command(subcommand)]
        history: Option<history::ClipboardCommand>,
        /// Share plain text.
        #[arg(long, value_name = "on|off")]
        text: Option<OnOff>,
        /// Share HTML (rich text).
        #[arg(long, value_name = "on|off")]
        html: Option<OnOff>,
        /// Share images.
        #[arg(long, value_name = "on|off")]
        image: Option<OnOff>,
        /// Largest clipboard to send or accept, in MB (1 to 256, default 25).
        #[arg(long, value_name = "N")]
        max_mb: Option<u32>,
        /// Skip what a password manager marked as sensitive (default on; `clip-send` also refuses it unless --allow-sensitive; not
        /// detectable on every system yet, see the README).
        #[arg(long, value_name = "on|off")]
        skip_sensitive: Option<OnOff>,
        /// Let other computers fetch this clipboard with `one-mk clip-get` (default off).
        #[arg(long, value_name = "on|off")]
        allow_fetch: Option<OnOff>,
    },
    /// Send this computer's clipboard to one paired computer now, without moving the pointer.
    ClipSend {
        /// Name or id of the paired computer (needed if several are connected).
        #[arg(long, value_name = "DEVICE")]
        to: Option<String>,
        /// Send the clipboard even if a password manager marked it sensitive (refused otherwise).
        /// The other computer keeps it out of its history.
        #[arg(long)]
        allow_sensitive: bool,
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
    /// Create a one-time invitation link and QR code.
    Invite(invite::InviteArgs),
    /// Join using an invitation link (or read it from stdin).
    Join(invite::JoinArgs),
    /// Pair with another computer (run it on both).
    Pair {
        /// Pair with the computer at this address instead of searching the
        /// network, e.g. 192.168.1.20:40123 (the other side prints its pairing port).
        #[arg(long, value_name = "IP:PORT")]
        connect: Option<SocketAddr>,
        /// Pair only through direct addresses; do not use multicast discovery.
        #[arg(long)]
        no_discovery: bool,
        /// One event per line for a program to read (the app window uses this).
        #[arg(long, hide = true)]
        machine: bool,
    },
    /// List paired computers.
    Devices,
    /// Show whether one-mk is running, who is connected (with how long each
    /// answer takes, "rtt 4 ms") and who controls whom.
    Status,
    /// Take this computer back from the computer controlling it (on Linux,
    /// where using this computer's own mouse or keyboard can't be noticed;
    /// bind it to a keyboard shortcut).
    Takeback,
    /// Start one-mk automatically when you log in.
    Install,
    /// Stop starting one-mk automatically, turn the wallpaper off (your own
    /// comes back) and remove oneMK's GNOME wallpaper extension.
    Uninstall,
    /// Add "Send to <computer>" to the file manager's menu (Nautilus, Nemo,
    /// Dolphin, Explorer, Finder); it follows the computers you pair or forget.
    Integrate {
        /// Remove the entries instead.
        #[arg(long)]
        remove: bool,
    },
    /// Show or choose what the other computer types for your keys: the same
    /// physical keys (default), or the letters printed on them. Experimental:
    /// letters (characters, auto) only work toward Windows and Mac computers
    /// and have not been tested on real hardware.
    Typing {
        #[arg(long, value_enum, help = "physical: the same keys, so letters follow the other computer's layout (default). characters (experimental): the letters printed on your keys. auto (experimental): characters only when the layouts differ. Letters work only toward Windows and Mac computers")]
        keys: Option<typing::Mode>,
        #[arg(long, value_name = "NAME_OR_ID", help = "Change or show only this paired computer's preference")]
        peer: Option<String>,
    },
    /// Say which side of this computer a paired computer is on, and
    /// optionally exactly where (applies at once if one-mk is running), and
    /// how Ctrl/Cmd are treated.
    Layout {
        /// Name or id of the paired computer (see `one-mk devices`).
        device: String,
        /// Where it is: left, right, top or bottom.
        side: Option<Side>,
        /// Where its (0,0) sits in this computer's screen coordinates, so the
        /// pointer crosses only where the screens touch; `none` crosses
        /// anywhere along the side.
        #[arg(long, value_name = "DX,DY|none", allow_hyphen_values = true, value_parser = manage::offset_arg)]
        offset: Option<String>,
        /// Swap Ctrl and Cmd for keys sent to it, so copy and paste feel the same
        /// between a Mac and a PC (default: auto, which swaps only between a Mac and another computer).
        #[arg(long, value_name = "on|off|auto")]
        swap_ctrl_cmd: Option<Swap>,
    },
    /// Named layouts: save where every computer is, switch between them,
    /// and switch by Wi-Fi network automatically.
    Profile {
        #[command(subcommand)]
        cmd: profiles::ProfileCmd,
    },
    /// Give a paired computer a name of your own here (for example when two
    /// computers call themselves the same). Only this computer shows it; the
    /// other computer keeps its own name.
    Rename {
        /// Current name or id of the paired computer (see `one-mk devices`).
        device: String,
        /// The new name (spaces are fine without quotes).
        #[arg(num_args = 0.., required_unless_present = "reset")]
        name: Vec<String>,
        /// Go back to the name the computer gave itself.
        #[arg(long, conflicts_with = "name")]
        reset: bool,
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
    /// Without an argument, says whether it is locked.
    LockPointer {
        /// Enable, disable or toggle pointer locking.
        #[arg(value_name = "on|off|toggle", value_parser = ["on", "off", "toggle"])]
        state: Option<String>,
    },
    /// Type on every connected computer at once: everything you type, click
    /// and scroll here goes to all of them until you stop it (also with
    /// Ctrl+Alt+B, or Ctrl+Alt+Shift+Esc). Without arguments, says whether
    /// it is on.
    #[command(after_help = "Examples:\n  one-mk broadcast on\n  one-mk broadcast off\n  one-mk broadcast --idle-off 30    stop by itself after 30 minutes without input (0 = never)\n\nSee docs/BROADCAST.md for what is sent where and how it stops.")]
    Broadcast {
        /// Enable, disable or toggle broadcasting; omit to show its state.
        #[arg(value_name = "on|off|toggle", value_parser = ["on", "off", "toggle"])]
        state: Option<String>,
        /// Stop by itself after this many minutes without any key, click,
        /// scroll or movement (0 = never; default 15).
        #[arg(long, value_name = "MINUTES")]
        idle_off: Option<u32>,
    },
    /// Show or change the rules for crossing edges: dead corners, a dwell
    /// time, a key to hold, not leaving a fullscreen window, and what edges
    /// with no computer do (pointer wrap, layout learning).
    Edge(settings::EdgeArgs),
    /// Gaming mode: on another computer, send raw mouse movement (for games)
    /// instead of positions.
    Gaming {
        /// Enable, disable or automatically choose raw mouse movement.
        #[arg(value_enum)]
        mode: Option<settings::Gaming>,
    },
    /// Show or change the keyboard shortcuts: jump to computer N, the next
    /// one, lock the pointer, come back here, broadcast or fetch a clipboard.
    Hotkeys(hotkeys::HotkeyArgs),
    /// Lock this computer's screen and every connected computer's.
    LockAll,
    /// Show or set the window's look: system (follow the computer), light or dark.
    Appearance {
        /// system, light or dark; leave out to see the current setting.
        #[arg(value_enum)]
        to: Option<settings::Appearance>,
    },
    /// Whether locking this computer locks the connected ones too: on or off.
    LockSync {
        /// Enable or disable shared screen locking.
        #[arg(value_name = "on|off", value_parser = settings::parse_on_off, action = clap::ArgAction::Set)]
        state: Option<bool>,
    },
    /// Show or set commands to run when the pointer arrives on or leaves
    /// this computer, or a computer connects or disconnects.
    Hooks(hooks::HooksArgs),
    /// Keep the screensaver and sleep off on computers this one controls,
    /// and put displays to sleep together.
    Power(power::PowerArgs),
    /// Change what keys do on the computers this one controls: remaps,
    /// combos, tap/hold keys, layers, actions (see docs/INPUT.md).
    Keymap {
        #[command(subcommand)]
        cmd: Option<tuning::KeymapCmd>,
    },
    /// Pointer speed on the computers this one controls.
    Pointer(tuning::PointerArgs),
    /// Draw the pointer here while another computer controls this one and
    /// this one shows none (no mouse attached): auto (the default), always
    /// or never. Without a value: show the setting.
    PointerOverlay {
        #[arg(value_name = "auto|always|never", value_parser = ["auto", "always", "never"])]
        mode: Option<String>,
    },
    /// GNOME only: the optional extension that hides this computer's pointer
    /// while the mouse is on another computer (it stays drawn at the edge
    /// otherwise): install, enable, status, uninstall.
    PointerExtension {
        #[command(subcommand)]
        action: pointer_extension::Action,
    },
    /// A video or picture as the wallpaper across your screens, or turn it off.
    ///
    /// Stable: video on X11 desktops, pictures on GNOME. Experimental (not
    /// yet tested on real hardware there): macOS, Windows, layer-shell
    /// Wayland desktops (KDE, Sway, Hyprland), video on GNOME Wayland through
    /// oneMK's GNOME extension, one wallpaper shared across several
    /// computers, and the smaller copy made of large videos.
    Wallpaper(live_wallpaper::Args),
    /// Scrolling on the computers this one controls: direction, natural,
    /// speed, smooth or by notches, modifier keys.
    Scroll(tuning::ScrollArgs),
    /// Click a button, then move away to scroll the other computer continuously.
    Autoscroll(tuning::AutoscrollArgs),
    /// Hold a mouse button and swipe to type a shortcut on the other computer.
    Gestures {
        #[command(subcommand)]
        cmd: Option<tuning::GesturesCmd>,
    },
    /// Remap mouse buttons on the computers this one controls.
    Buttons {
        #[command(subcommand)]
        cmd: Option<tuning::ButtonsCmd>,
    },
    /// List the keyboards and mice this computer has seen, for rules that
    /// apply to one of them (--device on keymap, buttons, pointer and
    /// scroll). Says whether this desktop can tell them apart.
    InputDevices {
        /// Machine-readable output for a program to read.
        #[arg(long)]
        json: bool,
    },
    /// Show this computer's name and id.
    Id,
    /// Find out why computers are not connecting, and how to fix it.
    Doctor {
        /// Machine-readable output for a program to read.
        #[arg(long)]
        json: bool,
        /// Also list the checks that did not apply here.
        #[arg(long)]
        all: bool,
    },
    /// Measure latency and CPU cost.
    Bench {
        /// Number of mouse events to send.
        #[arg(long, default_value_t = 5000, value_parser = clap::value_parser!(u32).range(1..))]
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
    /// This system's real mouse and keyboard, through the best backend it
    /// has (on Linux: the portals, else wlroots, else X11).
    Native,
    /// Linux: X11 (XInput2, XFixes barriers, XTest).
    X11,
    /// Linux: the wlroots Wayland protocols (Sway, Hyprland, labwc ...).
    Wlroots,
    /// No real input; for testing the network side.
    Null,
}

impl From<Backend> for one_mk_input::Kind {
    fn from(b: Backend) -> Self {
        match b {
            Backend::Native => Self::Native,
            Backend::X11 => Self::X11,
            Backend::Wlroots => Self::Wlroots,
            Backend::Null => Self::Null,
        }
    }
}

fn main() -> ExitCode {
    // The macOS pointer window runs in a helper: this program, started by
    // the daemon with this argument (see one_mk_input::cursoroverlay).
    if std::env::args_os().nth(1).is_some_and(|a| a == one_mk_input::cursoroverlay::HELPER_ARG) {
        one_mk_input::cursoroverlay::run_helper();
    }
    // `wallpaper import` and `wallpaper doctor --runtime` belong to the
    // separately installed, sandboxed import package: dispatched before any
    // configuration or identity is touched.
    #[cfg(feature = "wallpaper-m1-lab")]
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    #[cfg(feature = "wallpaper-m1-lab")]
    if raw.first().is_some_and(|a| a == "wallpaper")
        && (raw.get(1).is_some_and(|a| a == "import") || (raw.get(1).is_some_and(|a| a == "doctor") && raw.iter().any(|a| a == "--runtime")))
    {
        return wallpaper_import::run(&raw[1..]);
    }
    let cli = Cli::parse();
    let verbose = cli.verbose;
    let level = match verbose {
        // What happened to the wallpaper is worth keeping in the usual log.
        0 => "warn,mdns_sd=error,zbus=error,one_mk::live_wallpaper=info",
        // arboard says "Time-out hit while reading the clipboard" at info every few
        // seconds while another program holds the selection and doesn't answer.
        1 => "debug,mdns_sd=warn,zbus=info,quinn=info,quinn_proto=info,rustls=info,arboard=warn",
        _ => "trace",
    };
    let log = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
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

    #[cfg(target_os = "macos")]
    let native_daemon = matches!(cli.cmd.as_ref(), None | Some(Cmd::Run(_)));
    #[cfg(target_os = "macos")]
    let result = if native_daemon
    {
        one_mk_input::dropzone::with_main_loop(move || run(cli))
    } else { run(cli) };
    #[cfg(not(target_os = "macos"))]
    let result = run(cli);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // One line; -v shows each cause on its own.
            let text = if verbose > 0 { format!("{e:?}") } else { format!("{e:#}") };
            // "one-mk: one-mk is not running" says it twice.
            let prefix = if text.starts_with("one-mk ") { "" } else { "one-mk: " };
            eprintln!("{prefix}{text}");
            ExitCode::from(e.downcast_ref::<wallpaper::Failure>().map_or(1, |failure| failure.code))
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
        power: None,
    })
}

fn run(cli: Cli) -> Result<()> {
    let verbose = cli.verbose;
    let action = Action::of(cli.run, cli.cmd)?;
    // A home of its own (`--home`, `ONE_MK_HOME`): a test, or a second
    // one-mk. Nothing it does by itself reaches outside that folder: the
    // "Send to" entries follow the default home's computers only.
    let (dir, custom_home) = select_home(cli.home.as_deref(), one_mk_ctl::default_home(), migrate_old_home)?;
    // Files an earlier version made with the process umask become private.
    one_mk_ctl::private::tighten_home(&dir);

    // One thread for networking is plenty and keeps the footprint small;
    // input capture and injection have their own threads.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async move {
        let cmd = match action {
            Action::Run(r) => {
                // 6D-M1 laboratory only: never compiled into release builds.
                #[cfg(all(target_os = "linux", feature = "wallpaper-m1-lab"))]
                let _wallpaper_authority = if wallpaper_authority::requested() {
                    anyhow::ensure!(r.backend == Some(Backend::Null) && r.no_discovery && r.no_notify && r.no_clipboard,
                        "the wallpaper M1 laboratory needs --backend null --no-discovery --no-notify --no-clipboard");
                    let settings = settings::Settings::load(&dir)?;
                    Some(wallpaper_authority::start(&dir, &settings.wallpaper).await?)
                } else { None };
                if r.no_notify {
                    notify::disable();
                }
                return app::run(dir, options(r)?).await;
            }
            Action::Other(cmd) => cmd,
        };
        match cmd {
            Cmd::Update(options) => update::run(&dir, options),
            Cmd::Stop => update::stop(&dir).await,
            Cmd::Drag { on, off, zone } => drag::configure(&dir, on, off, zone).await,
            Cmd::Partials { clear, downloads } => manage::partials(&dir, clear, downloads).await,
            Cmd::Run(_) => unreachable!("handled above"),
            Cmd::Clipboard { history, text, html, image, max_mb, skip_sensitive, allow_fetch } => {
                if let Some(history) = history {
                    if text.is_some() || html.is_some() || image.is_some() || max_mb.is_some() || skip_sensitive.is_some() || allow_fetch.is_some() {
                        anyhow::bail!("Change clipboard settings and manage history in separate commands; see one-mk clipboard --help");
                    }
                    history.run(&dir).await
                }
                else { clipctl::clipboard(&dir, clipctl::ClipChange { text, html, image, max_mb, skip_sensitive, allow_fetch }).await }
            }
            Cmd::ClipSend { to, allow_sensitive } => clipctl::move_clipboard(&dir, to, true, allow_sensitive).await,
            Cmd::ClipGet { from } => clipctl::move_clipboard(&dir, from, false, false).await,
            Cmd::Notify { state } => clipctl::notify(&dir, state).await,
            Cmd::Send { paths, to, connect, notify_errors } => send::run(&dir, paths, to, connect, notify_errors).await,
            Cmd::Pair { connect, machine, no_discovery } => {
                let paired = pair::run(&dir, connect, pair::Out::new(machine), !no_discovery).await;
                integrate::refresh(&dir, custom_home);
                paired
            }
            Cmd::Invite(args) => {
                let paired = invite::run(&dir, args).await;
                integrate::refresh(&dir, custom_home);
                paired
            }
            Cmd::Join(args) => {
                let paired = invite::join(&dir, args).await;
                integrate::refresh(&dir, custom_home);
                paired
            }
            Cmd::Devices => devices(&dir, verbose > 0),
            Cmd::Status => manage::status(&dir).await,
            Cmd::Takeback => manage::takeback(&dir).await,
            Cmd::Install => install::install(&dir, custom_home).await,
            Cmd::Uninstall => install::uninstall(&dir).await,
            Cmd::Integrate { remove } => integrate::run(&dir, custom_home, remove),
            Cmd::Layout { device, side, offset, swap_ctrl_cmd } => {
                manage::layout(&dir, &device, side, offset.as_deref(), swap_ctrl_cmd).await
            }
            Cmd::Typing { keys, peer } => typing::run(&dir, keys, peer.as_deref()).await,
            Cmd::Profile { cmd } => profiles::run(&dir, cmd).await,
            Cmd::Rename { device, name, reset } => {
                let done = manage::rename(&dir, &device, &name.join(" "), reset)?;
                println!("{done}");
                if control::request(&dir, control::Request::Reload).await.is_ok_and(|r| r.is_some()) {
                    println!("Applied right away.");
                }
                integrate::refresh(&dir, custom_home);
                Ok(())
            }
            Cmd::Forget { device } => {
                forget(&dir, &device)?;
                // A running daemon lets go of it at once: its input link
                // and any clipboard or file connection.
                if control::request(&dir, control::Request::Reload).await.is_ok_and(|r| r.is_some()) {
                    println!("one-mk let go of it right away.");
                }
                integrate::refresh(&dir, custom_home);
                Ok(())
            }
            Cmd::Switch { to } => steer::switch(&dir, &to).await,
            Cmd::LockPointer { state: Some(state) } => {
                steer::lock_pointer(&dir, steer::Toggle::parse(&state).context("on, off or toggle")?).await
            }
            Cmd::LockPointer { state: None } => steer::show_pointer_lock(&dir).await,
            Cmd::Broadcast { state, idle_off } => {
                let state = state.map(|s| steer::Toggle::parse(&s).context("on, off or toggle")).transpose()?;
                steer::broadcast(&dir, state, idle_off).await
            }
            Cmd::Edge(args) => settings::edge(&dir, args).await,
            Cmd::Gaming { mode: Some(mode) } => settings::gaming(&dir, mode).await,
            Cmd::Gaming { mode: None } => { println!("Gaming mode: {}", settings::Settings::view(&dir)?.gaming); Ok(()) },
            Cmd::Hotkeys(args) => hotkeys::hotkeys(&dir, args).await,
            Cmd::LockAll => steer::lock_all(&dir).await,
            Cmd::Appearance { to } => settings::appearance(&dir, to),
            Cmd::LockSync { state: Some(state) } => settings::lock_sync(&dir, state).await,
            Cmd::LockSync { state: None } => { println!("Lock sync: {}", if settings::Settings::view(&dir)?.lock_sync { "on" } else { "off" }); Ok(()) },
            Cmd::Hooks(args) => hooks::run_command(&dir, args).await,
            Cmd::Power(args) => power::run_command(&dir, args).await,
            Cmd::Keymap { cmd } => tuning::keymap(&dir, cmd).await,
            Cmd::Pointer(args) => tuning::pointer_command(&dir, args).await,
            Cmd::PointerOverlay { mode } => pointeroverlay::command(&dir, mode.as_deref()).await,
            Cmd::PointerExtension { action } => pointer_extension::command(&action).await,
            Cmd::Wallpaper(args) => live_wallpaper::command(&dir, args).await,
            Cmd::Scroll(args) => tuning::scroll_command(&dir, args).await,
            Cmd::Autoscroll(args) => tuning::autoscroll_command(&dir, args).await,
            Cmd::Gestures { cmd } => tuning::gestures_command(&dir, cmd).await,
            Cmd::Buttons { cmd } => tuning::buttons(&dir, cmd).await,
            Cmd::InputDevices { json } => inputdevices::command(&dir, json).await,
            Cmd::Id => {
                let id = Identity::load_or_create(&dir)?;
                println!("{} ({})", id.name, id.fingerprint.short());
                println!("fingerprint {}", id.fingerprint);
                Ok(())
            }
            Cmd::Doctor { json, all } => doctor::run(&dir, json, all).await,
            Cmd::Bench { count, rate, serve, connect, peer } => {
                bench::run(&dir, bench::Options { count, rate: rate.max(1), serve, connect, peer }).await
            }
        }
    })
}

/// The folder to use (`--home`, `ONE_MK_HOME`, or `default`: where identity
/// and paired devices live, on Windows the local, non-roaming AppData) and
/// whether it is a custom one. `migrate` (moving an older version's files
/// into the default folder) runs only when the default folder is the one
/// used: a custom home never reaches outside itself, not even to look.
fn select_home(given: Option<&Path>, default: Result<PathBuf>, migrate: impl FnOnce(&Path)) -> Result<(PathBuf, bool)> {
    let (dir, custom) = match given {
        // Relative, a folder can't be checked up to the root (and the daemon
        // would see another folder after a `cd`): make it absolute now.
        Some(d) => {
            let dir = std::path::absolute(d).with_context(|| format!("--home {}", d.display()))?;
            let custom = match default {
                Ok(default) => !same_folder(&dir, &default),
                Err(_) => true,
            };
            (dir, custom)
        }
        None => (default?, false),
    };
    if !custom {
        migrate(&dir);
    }
    Ok((dir, custom))
}

/// Whether `a` and `b` are the same folder (both may not exist yet).
fn same_folder(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| std::path::absolute(p).unwrap_or_else(|_| p.to_owned()));
    canonical(a) == canonical(b)
}

/// Moves an older version's files into the default home `dir`, once: on
/// Windows they were kept in the roaming AppData.
fn migrate_old_home(dir: &Path) {
    #[cfg(windows)]
    if let Some(dirs) = directories::ProjectDirs::from("", "", "one-mk") {
        migrate_roaming(dirs.config_dir(), dir);
    }
    #[cfg(not(windows))]
    let _ = dir;
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

pub(crate) fn default_downloads() -> Result<PathBuf> {
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
        let name = match d.alias.as_deref().filter(|a| !a.trim().is_empty()) {
            Some(_) if d.alias.as_deref() != Some(d.name.as_str()) => format!("{} ({})", one_mk_proto::sanitize_name(d.label()), one_mk_proto::sanitize_name(&d.name)),
            _ => one_mk_proto::sanitize_name(d.label()),
        };
        let side = if d.placed_by == one_mk_net::PlacedBy::Pending { "placement needed".into() }
            else { d.offset.map_or_else(|| d.side.clone(), |(x, y)| format!("{} at {x},{y}", d.side)) };
        if verbose {
            println!("{name:<24} {:<9}  {:<6}  {}", d.short_id(), side, d.addrs.join(", "));
        } else {
            println!("{name:<24} {:<9}  {}", d.short_id(), side);
        }
    }
    Ok(())
}

fn forget(dir: &Path, device: &str) -> Result<()> {
    let d = TrustStore::update(dir, |trust| {
        let Some(d) = trust.remove(device) else { return Err(manage::not_found(trust, device)) };
        wallpaper::forget(dir, &d.fingerprint)?;
        Ok(d)
    })?;
    println!("Forgot {}.", one_mk_proto::sanitize_name(d.label()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "1 second ago", never "1 seconds ago".
    #[test]
    fn times_agree_with_their_numbers() {
        let ago = |seconds: u64| readable_time(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - seconds);
        assert_eq!(ago(1), "1 second ago");
        assert_eq!(ago(30), "30 seconds ago");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(3 * 3600), "3 hours ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(2 * 86_400), "2 days ago");
    }

    /// Picking the folder has no side effects unless it is the default
    /// one: `one-mk --home C:\\scratch devices` must never move, copy or
    /// delete the real configuration (the roaming-profile migration).
    #[test]
    fn only_the_default_home_is_migrated() {
        let root = std::env::temp_dir().join(format!("one-mk-select-home-{}", std::process::id()));
        let default = root.join("default");
        let custom = root.join("scratch");
        let migrated = std::cell::RefCell::new(Vec::new());
        let note = |dir: &Path| migrated.borrow_mut().push(dir.to_owned());

        let (dir, is_custom) = select_home(Some(&custom), Ok(default.clone()), note).unwrap();
        assert_eq!((dir, is_custom), (custom.clone(), true));
        assert!(migrated.borrow().is_empty(), "a custom home ran the migration");

        // No default folder known: a given one is custom, and nothing moves.
        let (_, is_custom) = select_home(Some(&custom), Err(anyhow::anyhow!("no home directory")), note).unwrap();
        assert!(is_custom && migrated.borrow().is_empty());

        let (dir, is_custom) = select_home(None, Ok(default.clone()), note).unwrap();
        assert_eq!((dir, is_custom), (default.clone(), false));
        // The default folder named explicitly is the default folder.
        let (dir, is_custom) = select_home(Some(&default), Ok(default.clone()), note).unwrap();
        assert_eq!((dir, is_custom), (default.clone(), false));
        assert_eq!(*migrated.borrow(), vec![default.clone(), default]);

        assert!(select_home(None, Err(anyhow::anyhow!("no home directory")), note).is_err());
        // A relative --home becomes absolute.
        let (dir, _) = select_home(Some(Path::new("rel-home")), Ok(root.join("d")), |_| {}).unwrap();
        assert!(dir.is_absolute() && dir.ends_with("rel-home"));
    }

    #[test]
    fn polished_help_describes_options_and_supported_scopes() {
        for (args, required, forbidden) in [
            (vec!["hotkeys"], "n in braces", "modifiers and "),
            (vec!["drag"], "Width of the edge-drop zone", "__missing__"),
            (vec!["autoscroll"], "Click a button", "--device"),
            (vec!["gestures", "add"], "Minimum swipe distance", "--device"),
            (vec!["gestures", "remove"], "Usage:", "--device"),
            (vec!["gestures", "preset"], "Usage:", "--device"),
            (vec!["wallpaper", "set"], "<FILE>", "--sandbox"),
            (vec!["wallpaper", "off"], "Usage:", "--renderer"),
            (vec!["wallpaper", "status"], "Usage:", "--renderer"),
        ] {
            let error = Cli::try_parse_from(std::iter::once("one-mk").chain(args).chain(["--help"]))
                .err()
                .expect("help should exit before running");
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
            let help = error.to_string();
            assert!(help.contains(required), "{help}");
            assert!(!help.contains(forbidden), "{help}");
        }
    }

    #[test]
    fn wallpaper_help_describes_every_command_and_says_what_is_experimental() {
        let help = |args: &[&str]| Cli::try_parse_from(std::iter::once("one-mk").chain(args.iter().copied()).chain(["--help"])).err().unwrap().to_string();
        let top = help(&["wallpaper"]);
        for line in ["set ", "off ", "status ", "pause ", "play ", "extension "] {
            let row = top.lines().find(|row| row.trim_start().starts_with(line)).unwrap_or_else(|| panic!("{line}: {top}"));
            assert!(row.trim().len() > line.len() + 8, "no description for {line}: {top}");
        }
        assert!(top.contains("Experimental") && top.contains("macOS") && top.contains("Windows") && top.contains("several"), "{top}");
        assert!(help(&["wallpaper", "set"]).contains("A video (MP4"));
        assert!(help(&["wallpaper", "off"]).contains("your own wallpaper back"));
        assert!(help(&["uninstall"]).contains("GNOME wallpaper extension"));
        let install = help(&["wallpaper", "extension", "install"]);
        assert!(install.contains("log out") && install.contains("nothing is"), "{install}");
    }

    #[test]
    fn release_wallpaper_cli_offers_only_consented_extension_setup_not_lab_playback() {
        for action in ["install", "enable", "status", "uninstall"] {
            assert!(Cli::try_parse_from(["one-mk", "wallpaper", "--json", "extension", action]).is_ok());
        }
        for action in ["import", "consent", "doctor"] {
            assert!(Cli::try_parse_from(["one-mk", "wallpaper", action]).is_err());
        }
    }

    #[test]
    fn layout_offset_bounds_and_empty_values() {
        for offset in ["", "10", "99999999,0", "0,-99999999", "2147483648,0", "center", "1,2,3"] {
            assert!(Cli::try_parse_from(["one-mk", "layout", "peer", "--offset", offset]).is_err(), "accepted {offset:?}");
        }
        for offset in ["-1920,0", "16777216,-16777216", "none", "None", "0,0", "-2592, 98"] {
            assert!(Cli::try_parse_from(["one-mk", "layout", "peer", "--offset", offset]).is_ok(), "rejected {offset:?}");
        }
    }

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

    /// What the file manager entries run: file names after `--` are files, whatever they look like.
    #[test]
    fn send_to_entries_parse_odd_file_names() {
        let cli = Cli::try_parse_from(["one-mk", "--home", "/h", "send", "--notify-errors", "--to", "abcd", "--", "-rf", "--to=x", "a b"]).unwrap();
        match cli.cmd {
            Some(Cmd::Send { paths, to, notify_errors, .. }) => {
                assert_eq!(paths, [PathBuf::from("-rf"), PathBuf::from("--to=x"), PathBuf::from("a b")]);
                assert_eq!(to.as_deref(), Some("abcd"));
                assert!(notify_errors);
            }
            _ => panic!("not send"),
        }
        assert!(matches!(Cli::try_parse_from(["one-mk", "integrate", "--remove"]).unwrap().cmd, Some(Cmd::Integrate { remove: true })));
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
