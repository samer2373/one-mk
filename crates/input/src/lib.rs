//! Platform input: capturing the local mouse and keyboard when the pointer
//! leaves the screen, and injecting input received from a peer.
//!
//! Every backend runs on its own task or thread and talks to the daemon
//! over channels, so the daemon is identical on every OS.
//! Key and button codes are Linux evdev codes on all platforms.

mod common_logic;
pub mod hotkeys;
mod null;
pub mod system;
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
pub use system::{DisplaySleeper, FocusProbe, Inhibitor, Power, PowerEvent, PowerHandles, ScreenLocker, fake_power};
#[cfg(target_os = "linux")]
pub use portal::{PortalProbe, probe_portals};

use anyhow::Result;
use one_mk_proto::{Desktop, Mods, Pos, Rect, Side};
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

/// An edge that leads to a connected peer, and which parts of it do.
#[derive(Clone, Debug, PartialEq)]
pub struct Armed {
    pub side: Side,
    /// The stretches of the edge that lead somewhere, as `[start, end)`
    /// along it (y for left and right, x for top and bottom), in logical
    /// pixels; `None` for all of it. With an offset alignment only the
    /// part facing the peer does: touching the rest must not capture (the
    /// daemon would hand the pointer straight back, and the input in
    /// between would be lost), so it is a wall, as beside a real monitor.
    pub spans: Option<Vec<(f64, f64)>>,
}

impl Armed {
    /// All of `side`.
    pub fn whole(side: Side) -> Self {
        Armed { side, spans: None }
    }

    /// Whether `pos` (on or near this edge) is on a part that leads
    /// somewhere.
    pub fn covers(&self, pos: Pos) -> bool {
        let v = Desktop::along(self.side, pos);
        self.spans.as_ref().is_none_or(|s| s.iter().any(|&(a, b)| v >= a && v < b))
    }

    /// The pixels of `desktop`'s outer edge on this side that lead
    /// somewhere, as `(rect, first, last)`: for each monitor on that edge,
    /// the first and last pixel along it (inclusive). What the pointer
    /// barriers of the Linux portal cover.
    pub fn segments<'a>(&'a self, desktop: &'a Desktop) -> impl Iterator<Item = (Rect, i32, i32)> + 'a {
        desktop.outer_edges(self.side).flat_map(move |r| {
            let (lo, hi) = if self.side.along_x() { (r.x, r.right()) } else { (r.y, r.bottom()) };
            let whole = [(f64::from(lo), f64::from(hi))];
            let spans: Vec<(f64, f64)> = self.spans.clone().unwrap_or_else(|| whole.to_vec());
            spans.into_iter().filter_map(move |(a, b)| {
                // Pixels p with a <= p < b, within the monitor.
                let first = (a.ceil() as i32).max(lo);
                let end = (b.ceil() as i32).min(hi);
                (first < end).then_some((*r, first, end - 1))
            })
        })
    }
}

/// The sides in `armed` that lead somewhere at `pos` (no allocation: this
/// runs on every mouse move).
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub(crate) fn open_sides(armed: &[Armed], pos: Pos) -> ([Side; 4], usize) {
    let mut out = [Side::Left; 4];
    let mut n = 0;
    for a in armed.iter().filter(|a| a.covers(pos)).take(4) {
        out[n] = a.side;
        n += 1;
    }
    (out, n)
}

/// From the daemon to the capture side.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureCmd {
    /// Edges that lead to a connected peer.
    SetSides(Vec<Armed>),
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

/// The power functions for a backend of `kind`: the OS's, or fakes that
/// only count (and whose events never come).
pub fn power(kind: Kind) -> Power {
    match kind {
        Kind::Native => system::native_power(),
        Kind::Null => fake_power().0,
    }
}

/// A backend for tests: the returned handles feed fake capture events in
/// and observe injected input.
pub fn null(desktop: Desktop) -> (Backend, NullHandles) {
    null::open(desktop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desk() -> Desktop {
        // A 1920x1080 monitor and a 1280x1024 one to its right, tops aligned.
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 1024)])
    }

    #[test]
    fn whole_edges_cover_everything() {
        let a = Armed::whole(Side::Right);
        assert!(a.covers(Pos::new(3199.0, 0.0)) && a.covers(Pos::new(3199.0, 5000.0)));
        assert_eq!(a.segments(&desk()).collect::<Vec<_>>(), vec![(Rect::new(1920, 0, 1280, 1024), 0, 1023)]);
        let top: Vec<_> = Armed::whole(Side::Top).segments(&desk()).map(|(_, a, b)| (a, b)).collect();
        assert_eq!(top, vec![(0, 1919), (1920, 3199)]);
        // Only the outer boundary: the smaller monitor's bottom is set back.
        let bottom: Vec<_> = Armed::whole(Side::Bottom).segments(&desk()).map(|(_, a, b)| (a, b)).collect();
        assert_eq!(bottom, vec![(0, 1919)]);
    }

    /// Only the part facing the peer captures; the rest is a wall.
    #[test]
    fn spans_limit_the_edge() {
        let a = Armed { side: Side::Right, spans: Some(vec![(300.0, 800.5)]) };
        assert!(!a.covers(Pos::new(3199.0, 299.0)));
        assert!(a.covers(Pos::new(3199.0, 300.0)) && a.covers(Pos::new(3199.0, 800.0)));
        assert!(!a.covers(Pos::new(3199.0, 801.0)));
        assert_eq!(a.segments(&desk()).map(|(_, a, b)| (a, b)).collect::<Vec<_>>(), vec![(300, 800)]);
        // Across two monitors along the top edge; past the end: nothing.
        let b = Armed { side: Side::Top, spans: Some(vec![(1500.0, 2500.0), (9000.0, 9100.0)]) };
        assert_eq!(b.segments(&desk()).map(|(_, a, b)| (a, b)).collect::<Vec<_>>(), vec![(1500, 1919), (1920, 2499)]);
        let none = Armed { side: Side::Left, spans: Some(vec![]) };
        assert_eq!(none.segments(&desk()).count(), 0);
        assert!(!none.covers(Pos::new(0.0, 10.0)));
    }

    #[test]
    fn open_sides_at_a_point() {
        let armed = [Armed { side: Side::Right, spans: Some(vec![(300.0, 800.0)]) }, Armed::whole(Side::Bottom)];
        let (s, n) = open_sides(&armed, Pos::new(3199.0, 1023.0));
        assert_eq!(&s[..n], &[Side::Bottom]);
        let (s, n) = open_sides(&armed, Pos::new(3199.0, 500.0));
        assert_eq!(&s[..n], &[Side::Right, Side::Bottom]);
    }
}
