//! What the daemon needs from the OS besides input: what the focused window
//! is (the fullscreen guard, gaming mode `auto`) and locking the screen
//! (`one-mk lock-all`, lock sync). Each sits behind a trait so the daemon's
//! decisions are tested with fakes; only the calls here are per OS.
//!
//! Noticing that the screen locked is the input backends' job (they already
//! watch the session) and arrives as `Event::ScreenLocked`.
//!
//! Gaming mode `auto` looks at the focus again shortly after input that may
//! move it, and every few seconds while a fullscreen window has it. Windows
//! also says when another window comes to the front ([`FocusProbe::watch`]).
//! Linux has no such signal for X11 focus short of an X event loop; macOS
//! doesn't probe the focus at all yet, so there is nothing to watch (an
//! `NSWorkspace` activation observer would come with that probe).
//!
//! | | focused window | lock | lock noticed |
//! |---|---|---|---|
//! | Linux | X11/XWayland windows only; Wayland-native ones are unknown | logind `Session.Lock` | logind `LockedHint` |
//! | Windows | foreground window: borderless, covering its monitor | `LockWorkStation` | session notifications |
//! | macOS | not yet (unknown) | `CGSession -suspend`, else display sleep | screen lock notifications |
//!
//! Power (see [`Power`]): keeping the screensaver and idle sleep off while a
//! peer controls this computer, putting the display to sleep when a peer's
//! display slept, and noticing that this one did.
//!
//! | | keep awake | display to sleep | display or suspend noticed |
//! |---|---|---|---|
//! | Linux | `org.freedesktop.ScreenSaver.Inhibit` and a logind `idle` inhibitor | GNOME's `PowerSaveMode` (Mutter), else `xset dpms force off` | Mutter `PowerSaveMode`; logind `PrepareForSleep` |
//! | Windows | `SetThreadExecutionState(ES_DISPLAY_REQUIRED)` from a thread of its own | `SC_MONITORPOWER` | `GUID_CONSOLE_DISPLAY_STATE`, `PBT_APMSUSPEND` |
//! | macOS | a `caffeinate -d -i` child | `pmset displaysleepnow` | not noticed (no public notification without polling) |

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

    /// Calls `notify` (from another thread) whenever another window comes
    /// to the front, until called with `None`. Only where the OS says so
    /// without polling: Windows (`EVENT_SYSTEM_FOREGROUND`); elsewhere
    /// nothing, and the caller looks again after input and every few
    /// seconds.
    fn watch(&mut self, _notify: Option<Box<dyn Fn() + Send>>) {}
}

pub trait ScreenLocker: Send {
    /// Asks the OS to lock this computer's screen. Returns at once; the
    /// lock itself is reported later as `Event::ScreenLocked(true)`.
    fn lock(&mut self) -> anyhow::Result<()>;
}

/// Keeps the screensaver, display blanking and idle sleep off.
pub trait Inhibitor: Send {
    /// Starts keeping the screen awake, for `reason` (shown by the OS in
    /// its list of inhibitors where it has one). Returns at once; the OS
    /// goes back to normal when the returned guard is dropped (or this
    /// process ends: nothing outlives it).
    fn inhibit(&mut self, reason: &str) -> anyhow::Result<Box<dyn Send>>;
}

pub trait DisplaySleeper: Send {
    /// Asks the OS to turn this computer's display off. Returns at once.
    fn sleep_display(&mut self) -> anyhow::Result<()>;
}

/// What the OS tells us about this computer's power state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerEvent {
    /// The display went to sleep or the computer is suspending (`true`);
    /// or it woke.
    DisplayAsleep(bool),
}

/// The power side of the OS, apart from the input backend.
pub struct Power {
    pub inhibitor: Box<dyn Inhibitor>,
    pub sleeper: Box<dyn DisplaySleeper>,
    pub events: mpsc::UnboundedReceiver<PowerEvent>,
}

/// The power functions of this OS; on Linux this also starts watching for
/// the display sleeping and the computer suspending.
pub fn native_power() -> Power {
    let (tx, events) = mpsc::unbounded_channel();
    #[cfg(target_os = "linux")]
    {
        linux::watch_display(tx);
        Power { inhibitor: Box::new(linux::KeepAwake), sleeper: Box::new(linux::Dpms), events }
    }
    #[cfg(windows)]
    {
        windows::watch_display(tx);
        Power { inhibitor: Box::new(windows::KeepAwake), sleeper: Box::new(windows::MonitorPower), events }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = tx;
        Power { inhibitor: Box::new(macos::Caffeinate), sleeper: Box::new(macos::DisplaySleepNow), events }
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = tx;
        Power { inhibitor: Box::new(Unsupported), sleeper: Box::new(Unsupported), events }
    }
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
        (Box::new(windows::Foreground::default()), Box::new(windows::Workstation))
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

impl Inhibitor for Unsupported {
    fn inhibit(&mut self, _: &str) -> anyhow::Result<Box<dyn Send>> {
        anyhow::bail!("keeping the screen awake is not supported on this system")
    }
}

impl DisplaySleeper for Unsupported {
    fn sleep_display(&mut self) -> anyhow::Result<()> {
        anyhow::bail!("putting the display to sleep is not supported on this system")
    }
}

impl ScreenLocker for Unsupported {
    fn lock(&mut self) -> anyhow::Result<()> {
        anyhow::bail!("locking the screen is not supported on this system")
    }
}

/// What [`FocusProbe::watch`] was given, for a test to call.
pub type FocusNotify = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// For tests and `--backend null`: reports whatever focus it is given,
/// and counts how often it was asked.
pub struct FakeFocus {
    pub focus: Arc<Mutex<Option<Focus>>>,
    pub probes: Arc<AtomicUsize>,
    pub notify: FocusNotify,
}

impl FocusProbe for FakeFocus {
    fn focus(&mut self) -> Option<Focus> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        self.focus.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn watch(&mut self, notify: Option<Box<dyn Fn() + Send>>) {
        *self.notify.lock().unwrap_or_else(|e| e.into_inner()) = notify;
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

/// For tests and `--backend null`: counts inhibits, and the ones still held.
pub struct FakeInhibitor {
    pub held: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
}

struct FakeHold(Arc<AtomicUsize>);

impl Drop for FakeHold {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Inhibitor for FakeInhibitor {
    fn inhibit(&mut self, reason: &str) -> anyhow::Result<Box<dyn Send>> {
        tracing::info!(reason, "keeping the screen awake (null backend: nothing is inhibited)");
        self.total.fetch_add(1, Ordering::SeqCst);
        self.held.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeHold(self.held.clone())))
    }
}

/// For tests and `--backend null`: counts display sleeps and reports the
/// display asleep at once, as an OS would a moment later. Nothing sleeps.
pub struct FakeSleeper {
    pub events: mpsc::UnboundedSender<PowerEvent>,
    pub count: Arc<AtomicUsize>,
}

impl DisplaySleeper for FakeSleeper {
    fn sleep_display(&mut self) -> anyhow::Result<()> {
        self.count.fetch_add(1, Ordering::SeqCst);
        tracing::info!("display sleep requested (null backend: nothing sleeps)");
        let _ = self.events.send(PowerEvent::DisplayAsleep(true));
        Ok(())
    }
}

/// Test-side ends of [`fake_power`].
pub struct PowerHandles {
    /// Report the display going to sleep or waking.
    pub events: mpsc::UnboundedSender<PowerEvent>,
    /// Inhibits held now, and in total.
    pub held: Arc<AtomicUsize>,
    pub inhibits: Arc<AtomicUsize>,
    /// How often the display was asked to sleep.
    pub sleeps: Arc<AtomicUsize>,
}

/// Power functions that do nothing but count, for tests and `--backend null`.
pub fn fake_power() -> (Power, PowerHandles) {
    let (tx, events) = mpsc::unbounded_channel();
    let (held, inhibits, sleeps) = Default::default();
    let power = Power {
        inhibitor: Box::new(FakeInhibitor { held: Arc::clone(&held), total: Arc::clone(&inhibits) }),
        sleeper: Box::new(FakeSleeper { events: tx.clone(), count: Arc::clone(&sleeps) }),
        events,
    };
    (power, PowerHandles { events: tx, held, inhibits, sleeps })
}
