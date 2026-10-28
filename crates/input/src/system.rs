//! What the daemon needs from the OS besides input: what the focused window
//! is (the fullscreen guard, gaming mode `auto`) and locking the screen
//! (`one-mk lock-all`, lock sync). Each sits behind a trait so the daemon's
//! decisions are tested with fakes; only the calls here are per OS.
//!
//! Noticing that the screen locked is the input backends' job (they already
//! watch the session) and arrives as `Event::ScreenLocked`.
//!
//! | | focused window | lock | lock noticed |
//! |---|---|---|---|
//! | Linux | X11/XWayland windows only; Wayland-native ones are unknown | logind `Session.Lock` | logind `LockedHint` |
//! | Windows | foreground window vs. its monitor | `LockWorkStation` | session notifications |
//! | macOS | not yet (unknown) | `CGSession -suspend`, else display sleep | screen lock notifications |

use crate::Event;
use one_mk_proto::Focus;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub trait FocusProbe: Send {
    /// The focused window, or `None` if this system can't tell (then the
    /// fullscreen guard never blocks).
    fn focus(&mut self) -> Option<Focus>;
}

pub trait ScreenLocker: Send {
    /// Asks the OS to lock this computer's screen. Returns at once; the
    /// lock itself is reported later as `Event::ScreenLocked(true)`.
    fn lock(&mut self) -> anyhow::Result<()>;
}

/// The probe and locker for this OS. On Linux this also starts watching
/// logind for lock changes, reported on `events`.
pub(crate) fn native(events: &mpsc::UnboundedSender<Event>) -> (Box<dyn FocusProbe>, Box<dyn ScreenLocker>) {
    #[cfg(target_os = "linux")]
    {
        linux::watch_lock(events.clone());
        (Box::new(linux::X11Focus::default()), Box::new(linux::Logind))
    }
    #[cfg(windows)]
    {
        let _ = events;
        (Box::new(windows::Foreground), Box::new(windows::Workstation))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = events;
        (Box::new(Unknown), Box::new(macos::Suspend))
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = events;
        (Box::new(Unknown), Box::new(Unsupported))
    }
}

/// A system that can't tell what has the focus.
pub struct Unknown;

impl FocusProbe for Unknown {
    fn focus(&mut self) -> Option<Focus> {
        None
    }
}

#[allow(dead_code)] // only on OSes without a locker
struct Unsupported;

impl ScreenLocker for Unsupported {
    fn lock(&mut self) -> anyhow::Result<()> {
        anyhow::bail!("locking the screen is not supported on this system")
    }
}

/// For tests and `--backend null`: reports whatever focus it is given,
/// and counts how often it was asked.
pub struct FakeFocus {
    pub focus: Arc<Mutex<Option<Focus>>>,
    pub probes: Arc<AtomicUsize>,
}

impl FocusProbe for FakeFocus {
    fn focus(&mut self) -> Option<Focus> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        self.focus.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// For tests and `--backend null`: counts lock requests and reports the
/// screen locked at once, as an OS would a moment later. Nothing is locked.
pub struct FakeLocker {
    pub events: mpsc::UnboundedSender<Event>,
    pub count: Arc<AtomicUsize>,
}

impl ScreenLocker for FakeLocker {
    fn lock(&mut self) -> anyhow::Result<()> {
        self.count.fetch_add(1, Ordering::SeqCst);
        tracing::info!("screen lock requested (null backend: nothing is locked)");
        let _ = self.events.send(Event::ScreenLocked(true));
        Ok(())
    }
}
