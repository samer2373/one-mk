//! Platform input: capturing the local mouse and keyboard when the pointer
//! leaves the screen, and injecting input received from a peer.
//!
//! Every backend runs on its own task or thread and talks to the daemon
//! over channels, so the daemon is identical on every OS.
//! Key and button codes are Linux evdev codes on all platforms.

mod common_logic;
mod null;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_logic;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))]
mod win_logic;

pub use null::NullHandles;

use anyhow::Result;
use one_mk_proto::{Desktop, Pos, Side};
use std::path::PathBuf;
use tokio::sync::mpsc;

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
    Motion { dx: f64, dy: f64 },
    Button { button: u32, down: bool },
    Scroll { dx: f64, dy: f64 },
    ScrollDiscrete { dx: i32, dy: i32 },
    ScrollStop,
    Key { code: u32, down: bool },
    /// Capture `id` ended without us asking (session closed, permission
    /// revoked, monitors changed, backend failure).
    Deactivated { id: u32 },
    /// Real (not injected) input happened on this machine while it was
    /// being controlled by a peer, so the person sitting here wants control
    /// back. Only sent by backends that can tell injected input apart.
    LocalInput,
    /// A part of the backend failed; the message is shown to the user.
    Error(String),
}

/// From the daemon to the capture side.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureCmd {
    /// Edges that lead to a connected peer.
    SetSides(Vec<Side>),
    /// End activation `id` and give the pointer back to this machine,
    /// placing it at `pos`. Ignored if `id` is not the current activation.
    Release { id: u32, pos: Option<Pos> },
}

/// From the daemon to the injection side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inject {
    MoveTo(Pos),
    Button { button: u32, down: bool },
    Scroll { dx: f64, dy: f64 },
    ScrollDiscrete { dx: i32, dy: i32 },
    ScrollStop,
    Key { code: u32, down: bool },
}

pub struct Backend {
    pub events: mpsc::UnboundedReceiver<Event>,
    pub capture: mpsc::UnboundedSender<CaptureCmd>,
    pub inject: mpsc::UnboundedSender<Inject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The native backend for this OS.
    Native,
    /// Captures nothing and only logs injected input.
    Null,
}

/// Opens a backend. `state_dir` holds things like permission tokens so the
/// user is asked for permission only once.
pub async fn open(kind: Kind, state_dir: PathBuf) -> Result<Backend> {
    match kind {
        Kind::Null => {
            let (backend, mut handles) = null::open(Desktop::new(vec![one_mk_proto::Rect::new(0, 0, 1920, 1080)]));
            tokio::spawn(async move {
                while let Some(ev) = handles.inject.recv().await {
                    tracing::debug!(?ev, "inject (null backend)");
                }
                drop(handles.events);
            });
            Ok(backend)
        }
        Kind::Native => native(state_dir).await,
    }
}

#[cfg(target_os = "linux")]
async fn native(state_dir: PathBuf) -> Result<Backend> {
    portal::open(state_dir).await
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

/// A backend for tests: the returned handles feed fake capture events in
/// and observe injected input.
pub fn null(desktop: Desktop) -> (Backend, NullHandles) {
    null::open(desktop)
}
