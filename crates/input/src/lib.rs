//! Platform input: capturing the local mouse and keyboard when the pointer
//! leaves the screen, and injecting input received from a peer.
//!
//! Every backend runs on its own task or thread and talks to the daemon
//! over channels, so the daemon is identical on every OS.
//! Key and button codes are Linux evdev codes on all platforms.

mod common_logic;
#[cfg(any(windows, target_os = "macos", test))]
mod devcorr;
pub mod cursoroverlay;
#[cfg(target_os = "macos")]
mod cursoroverlay_macos;
#[cfg(windows)]
mod cursoroverlay_windows;
#[cfg(target_os = "linux")]
mod cursoroverlay_x11;
pub mod localmap;
pub mod dropzone;
pub mod hotkeys;
#[cfg(any(target_os = "linux", test))]
mod linux_devices;
#[cfg(any(target_os = "linux", test))]
mod linux_logic;
#[cfg(target_os = "linux")]
mod linux_rt;
mod null;
pub mod system;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_devices;
#[cfg(any(target_os = "macos", test))]
mod macos_logic;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_devices;
#[cfg(target_os = "linux")]
mod wlroots;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
mod xkb;
#[cfg(any(windows, test))]
mod win_logic;
#[cfg(any(windows, test))]
pub mod windows_service_logic;

pub use cursoroverlay::InjectTx;
pub use null::{NullHandles, fake_keyboard};
pub use system::{DisplaySleeper, FocusProbe, Inhibitor, Power, PowerEvent, PowerHandles, ScreenLocker, fake_power};
#[cfg(target_os = "linux")]
pub use portal::{PortalProbe, probe_portals};
#[cfg(target_os = "linux")]
pub use linux_logic::{LinuxBackend, SessionFacts, choose as choose_linux_backend};

use anyhow::Result;
pub use one_mk_proto::device::{Bus, DeviceRef, Fingerprint, InputDevice, Kinds};
use one_mk_proto::{Desktop, Edge, Mods, Pos, Side};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Why per-device rules can't work with the Wayland backends (portals,
/// wlroots): the compositor merges every keyboard and mouse into one before
/// one-mk sees any input.
pub const MERGED_DEVICES: &str =
    "per-device rules are not available on this desktop: the desktop merges all keyboards and mice before one-mk sees them";

/// The same for the X11 backend on an XWayland server, which has only its
/// own merged virtual devices.
pub const XWAYLAND_DEVICES: &str =
    "per-device rules are not available on this desktop: XWayland merges all keyboards and mice before one-mk sees them";

/// Whether a backend can tell keyboards and mice apart, before it runs:
/// `Ok(note)` where it can (the note says what isn't tested), `Err(why)`
/// where it can't. The running backend has the last word
/// (`Event::DevicesUnavailable`), as X11 does under XWayland.
pub fn device_support(backend: &str) -> Result<Option<&'static str>, &'static str> {
    match backend {
        "x11" => Ok(None),
        "portal" | "wlroots" => Err(MERGED_DEVICES),
        "null" => Err("the null input backend captures no keyboard or mouse"),
        "native" if cfg!(windows) => Ok(Some("untested on real Windows hardware (Raw Input, compile-checked only)")),
        "native" if cfg!(target_os = "macos") => Ok(Some("untested on a real Mac (IOHIDManager, compile-checked only)")),
        _ => Err("this input backend doesn't say which keyboard or mouse input came from"),
    }
}

/// From the backend to the daemon.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// This machine's monitors (initially, and whenever they change).
    Desktop(Desktop),
    /// The pointer hit the `side` edge at `pos`; local input is now
    /// captured and delivered as the events below until released.
    /// `id` identifies this activation; later `Deactivated`/`Release`
    /// carry it so stale ones can be ignored.
    Activated { id: u32, side: Side, pos: Pos },
    /// The input events below say which keyboard or mouse they came from
    /// (`device`, an entry of the latest `Devices`), `None` when the
    /// backend can't tell (see `docs/INPUT-TUNING.md`, "Per-device rules"):
    /// such input gets only the rules that name no device.
    Motion { dx: f64, dy: f64, device: Option<DeviceRef> },
    Button { button: u32, down: bool, device: Option<DeviceRef> },
    Scroll { dx: f64, dy: f64, device: Option<DeviceRef> },
    ScrollDiscrete { dx: i32, dy: i32, device: Option<DeviceRef> },
    ScrollStop,
    Key { code: u32, down: bool, device: Option<DeviceRef> },
    /// The keyboards and mice connected now, sent at start and whenever
    /// one is plugged in, unplugged, falls asleep or wakes. A device
    /// missing from the list is gone: whatever it held is let go of.
    Devices(Vec<InputDevice>),
    /// This backend can't tell keyboards and mice apart (the desktop merges
    /// them before one-mk sees them): every input event's `device` is
    /// `None`. Sent once, at start, by such backends.
    DevicesUnavailable(&'static str),
    /// Capture `id` ended without us asking (session closed, permission
    /// revoked, monitors changed, backend failure).
    Deactivated { id: u32 },
    /// Real (not injected) input happened on this machine while it was
    /// being controlled by a peer, so the person sitting here wants control
    /// back. Only sent while the daemon has said `Inject::Controlled(true)`,
    /// and only by backends that can tell injected input apart (Windows and
    /// macOS; not the Linux portals).
    LocalInput,
    /// The modifier keys held on this machine, where the backend can tell:
    /// Windows and macOS send it just before each `Activated` (their hooks
    /// see every key); Linux whenever libei reports modifiers, which it
    /// does only while input is captured.
    Modifiers(Mods),
    /// While captured: a modifier key held since before the capture went
    /// up. Its release goes to this machine (it went down here), so it is
    /// not input for a peer, but the modifiers held have changed: a
    /// shortcut or a modifier-to-cross must no longer count it. Windows
    /// and macOS (libei reports such changes as `Modifiers`).
    LocalKeyUp { code: u32 },
    /// This machine's screen was locked (`true`) or unlocked, where the OS
    /// tells us: logind's `LockedHint` on Linux, session notifications on
    /// Windows, the screen lock notifications on macOS.
    ScreenLocked(bool),
    /// A keyboard shortcut was pressed (from [`hotkeys::listen`]'s receiver).
    Hotkey(hotkeys::Action),
    /// Whether the system's shortcut service works (from the same receiver).
    Hotkeys(hotkeys::Availability),
    /// A part of the backend failed; the message is shown to the user.
    Error(String),
    /// How a part of the backend is doing, for `one-mk status`: `Some`
    /// says in one line what is wrong or what the user must do (approve
    /// the desktop's permission dialog), `None` that it works (again).
    /// `part` tells the parts apart ("capture", "control"). Sent on changes.
    Status { part: &'static str, problem: Option<String> },
}

/// From the daemon to the capture side.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureCmd {
    /// Edge segments that lead to a connected peer, in the coordinates of
    /// the last `Event::Desktop`; replaces the previous ones. Only these
    /// capture: the rest of an edge is a wall, as beside a real monitor.
    /// A segment may be an inner exposed edge (a monitor's edge with none
    /// of ours beyond it, set back from the bounding box); every backend
    /// honours those like outer ones. `Activate` for a side uses its first
    /// segment where the backend moves the pointer there.
    SetEdges(Vec<Edge>),
    /// End activation `id` and give the pointer back to this machine,
    /// placing it at `pos`. Ignored if `id` is not the current activation.
    Release { id: u32, pos: Option<Pos> },
    /// Start capturing as if the pointer had just hit the `side` edge
    /// (`one-mk switch`): answered with `Event::Activated` for `side`, or
    /// with `Event::Error` if this backend can't. The daemon decides where
    /// the pointer goes, so the reported position doesn't matter.
    Activate { side: Side },
}

/// From the daemon to the injection side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inject {
    MoveTo(Pos),
    /// Relative motion in whole logical pixels (gaming mode): games that
    /// capture the pointer read these as raw movement.
    MoveBy { dx: i32, dy: i32 },
    Button { button: u32, down: bool },
    Scroll { dx: f64, dy: f64 },
    ScrollDiscrete { dx: i32, dy: i32 },
    ScrollStop,
    Key { code: u32, down: bool },
    /// Whether a peer controls this machine from now on. The daemon sends
    /// `true` when a peer takes control and `false` when it lets go (after
    /// releasing what it held). Backends report `Event::LocalInput` only
    /// while it is true. Backends that can't see local input ignore it.
    Controlled(bool),
}

pub struct Backend {
    pub name: &'static str,
    pub events: mpsc::UnboundedReceiver<Event>,
    pub capture: mpsc::UnboundedSender<CaptureCmd>,
    pub inject: mpsc::UnboundedSender<Inject>,
    /// What the focused window is, for the fullscreen guard.
    pub focus: Box<dyn FocusProbe>,
    /// Locks this machine's screen.
    pub locker: Box<dyn ScreenLocker>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The native backend for this OS. On Linux the best one the session
    /// has (see [`choose_linux_backend`]).
    Native,
    /// X11 (Linux).
    X11,
    /// The wlroots Wayland protocols (Linux: Sway, Hyprland, labwc ...).
    Wlroots,
    /// Captures nothing and only logs injected input.
    Null,
}

/// Opens a backend. `state_dir` holds things like permission tokens so the
/// user is asked for permission only once.
pub async fn open(kind: Kind, state_dir: PathBuf) -> Result<Backend> {
    match kind {
        Kind::Null => {
            let desktop = Desktop::new(vec![one_mk_proto::Rect::new(0, 0, 1920, 1080)]);
            let (backend, mut handles) = null::open(desktop.clone());
            tokio::spawn(async move {
                let mut next_id = 0u32;
                loop {
                    tokio::select! {
                        ev = handles.inject.recv() => match ev {
                            Some(ev) => tracing::debug!(?ev, "inject (null backend)"),
                            None => break,
                        },
                        // Pretend to capture when asked, so `one-mk switch`
                        // can be tried without real input.
                        Some(cmd) = handles.capture.recv() => {
                            tracing::debug!(?cmd, "capture (null backend)");
                            if let CaptureCmd::Activate { side } = cmd {
                                next_id = next_id.wrapping_add(1);
                                let pos = desktop.point_on_edge(side, 0.5, 0.0);
                                let _ = handles.events.send(Event::Activated { id: next_id, side, pos });
                            }
                        }
                    }
                }
                drop(handles.events);
            });
            Ok(backend)
        }
        Kind::Native => native(state_dir).await,
        Kind::X11 | Kind::Wlroots => linux(kind, state_dir).await,
    }
}

#[cfg(target_os = "linux")]
async fn native(state_dir: PathBuf) -> Result<Backend> {
    // Outside Wayland the portals don't matter (see `choose`).
    let portals = if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()) {
        let probe = tokio::time::timeout(std::time::Duration::from_secs(3), probe_portals()).await;
        probe.ok().map(|p| p.remote_desktop.is_ok() && p.input_capture.is_ok())
    } else {
        None
    };
    let facts = session_facts(portals).await;
    let (backend, why) = linux_logic::choose(&facts);
    tracing::info!("input backend: {} ({why})", backend.name());
    match backend {
        LinuxBackend::Portal => portal::open(state_dir).await,
        LinuxBackend::Wlroots => wlroots::open(state_dir).await,
        LinuxBackend::X11 => x11::open(state_dir).await,
    }
}

#[cfg(target_os = "linux")]
async fn linux(kind: Kind, state_dir: PathBuf) -> Result<Backend> {
    if kind == Kind::X11 { x11::open(state_dir).await } else { wlroots::open(state_dir).await }
}

#[cfg(not(target_os = "linux"))]
async fn linux(_kind: Kind, _state_dir: PathBuf) -> Result<Backend> {
    anyhow::bail!("the x11 and wlroots input backends are for Linux")
}

/// What this session offers for picking a Linux backend; `portals` is
/// what [`probe_portals`] found, if asked. Asks the Wayland compositor
/// (if any) for its globals; never takes more than about two seconds.
#[cfg(target_os = "linux")]
pub async fn session_facts(portals: Option<bool>) -> SessionFacts {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty());
    let wayland_globals = if wayland {
        let probe = tokio::task::spawn_blocking(wlroots::globals);
        tokio::time::timeout(std::time::Duration::from_secs(2), probe).await.ok().and_then(Result::ok).flatten()
    } else {
        None
    };
    let x11 = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
    SessionFacts { portals, wayland, wayland_globals, x11 }
}

#[cfg(windows)]
async fn native(state_dir: PathBuf) -> Result<Backend> {
    windows::open(state_dir).await
}

#[cfg(target_os = "macos")]
async fn native(state_dir: PathBuf) -> Result<Backend> {
    macos::open(state_dir).await
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn native(_state_dir: PathBuf) -> Result<Backend> {
    anyhow::bail!("no native input backend for this OS")
}

/// The power functions for a backend of `kind`: the OS's, or fakes that
/// only count (and whose events never come).
pub fn power(kind: Kind) -> Power {
    match kind {
        Kind::Native | Kind::X11 | Kind::Wlroots => system::native_power(),
        Kind::Null => fake_power().0,
    }
}

/// A backend for tests: the returned handles feed fake capture events in
/// and observe injected input.
pub fn null(desktop: Desktop) -> (Backend, NullHandles) {
    null::open(desktop)
}
