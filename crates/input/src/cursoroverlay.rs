//! A pointer drawn by one-mk where the OS draws none (plan 3A).
//!
//! A computer with no mouse attached often shows no pointer, even while a
//! peer moves it (see `docs/NO-MOUSE-POINTER.md`). Where the decision in
//! [`one_mk_proto::overlay`] says so, a small always-on-top, click-through
//! window with an arrow follows the injected position.
//!
//! Nothing exists while it isn't drawn: no window, no thread, no timer.
//! The overlay is created when a peer takes control (or later, once the
//! decision says draw) and destroyed when control ends, when the setting
//! says never, or when a real mouse appears (the OS pointer is used then).
//! Everything is driven by the daemon's injected events: [`InjectTx`]
//! forwards them to the backend and shows the controller the few that
//! matter (`Controlled`, `MoveTo`, `MoveBy`). The probe of the OS pointer
//! runs at most every [`PROBE_EVERY`], and only while controlled and
//! moving.

use crate::Inject;
use one_mk_proto::Pos;
use one_mk_proto::overlay::{Decider, Facts, wants};
pub use one_mk_proto::overlay::{DrawMode, OsCursor};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How often, at most, the OS pointer and the attached devices are asked
/// about while a peer moves the pointer.
pub const PROBE_EVERY: Duration = Duration::from_millis(500);

/// The first argument that makes `one-mk` the macOS pointer window helper
/// (see `cursoroverlay_macos.rs`) instead of anything else.
pub const HELPER_ARG: &str = "--one-mk-pointer-overlay-helper";

/// Runs the pointer window helper and exits. Only macOS has one; elsewhere
/// it exits with an error at once.
pub fn run_helper() -> ! {
    #[cfg(target_os = "macos")]
    crate::cursoroverlay_macos::helper_main();
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("one-mk: the pointer helper is only for macOS");
        std::process::exit(2)
    }
}

/// What a platform can tell about its pointer right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// A mouse or touchpad is attached; `None` if the OS can't tell.
    pub pointing_device: Option<bool>,
    pub os_cursor: OsCursor,
}

/// A drawn pointer. Dropping it removes the window (and ends its thread,
/// where there is one).
pub trait Overlay: Send {
    /// Put the arrow's tip at `pos` (the daemon's logical pixels).
    fn move_to(&mut self, pos: Pos);
    /// A real pointing device appeared since it was created; the overlay
    /// has hidden itself already, and is dropped at the next event.
    fn device_appeared(&self) -> bool {
        false
    }
}

/// One OS's way of drawing a pointer.
pub trait Platform: Send {
    fn probe(&mut self) -> Probe;
    /// Asks the OS to show its own pointer (macOS: `CGDisplayShowCursor`);
    /// `true` if it now does for sure. Tried once when control starts, before
    /// drawing one of our own in `auto` mode.
    fn reveal(&mut self) -> bool {
        false
    }
    /// Creates the overlay with its tip at `at`.
    fn create(&mut self, at: Pos) -> anyhow::Result<Box<dyn Overlay>>;
}

/// Whether drawing a pointer is possible where the daemon runs with this
/// backend, for `one-mk doctor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// The OS may hide its pointer, and we can draw one.
    Available,
    /// The OS always draws its pointer (X11); `always` still draws ours.
    NotNeeded,
    /// No way to draw one here (Wayland: only the compositor places
    /// surfaces at a pointer position; it draws the virtual pointer).
    NotAvailable,
    /// Captures and injects nothing (`--backend null`).
    NoInput,
}

/// What drawing a pointer means with the backend called `backend` (as in
/// `one-mk status`) on this OS.
pub fn support(backend: &str) -> Support {
    match backend {
        "null" => Support::NoInput,
        "x11" => Support::NotNeeded,
        "portal" | "wlroots" => Support::NotAvailable,
        _ if cfg!(any(windows, target_os = "macos")) => Support::Available,
        _ => Support::NotAvailable,
    }
}

/// The platform for a running backend, if it can draw a pointer.
pub fn platform(backend: &str) -> Option<Box<dyn Platform>> {
    match (backend, support(backend)) {
        #[cfg(windows)]
        ("native", Support::Available) => Some(Box::new(crate::cursoroverlay_windows::Windows)),
        #[cfg(target_os = "macos")]
        ("native", Support::Available) => Some(Box::new(crate::cursoroverlay_macos::Mac)),
        #[cfg(target_os = "linux")]
        ("x11", _) => Some(Box::new(crate::cursoroverlay_x11::X11::from_env())),
        _ => None,
    }
}

/// Decides, from the injected events, whether the pointer is drawn, and
/// creates, moves and drops the overlay.
pub struct Controller {
    platform: Option<Box<dyn Platform>>,
    mode: DrawMode,
    decider: Decider,
    controlled: bool,
    relative: bool,
    pos: Option<Pos>,
    probe: Probe,
    probed_at: Option<Instant>,
    /// Creating failed during this control session: don't retry on every
    /// move (and say so once).
    failed: bool,
    /// The overlay saw a mouse arrive during this control session: the OS
    /// pointer is used until it ends, whatever later probes say.
    device_seen: bool,
    overlay: Option<Box<dyn Overlay>>,
    active: Arc<AtomicBool>,
}

impl Controller {
    /// `platform` is `None` where nothing can be drawn.
    pub fn new(platform: Option<Box<dyn Platform>>, mode: DrawMode) -> Self {
        Self {
            platform,
            mode,
            decider: Decider::default(),
            controlled: false,
            relative: false,
            pos: None,
            probe: Probe::default(),
            probed_at: None,
            failed: false,
            device_seen: false,
            overlay: None,
            active: Arc::default(),
        }
    }

    pub fn set_mode(&mut self, mode: DrawMode, now: Instant) {
        self.mode = mode;
        self.sync(now);
    }

    /// Whether the pointer is drawn right now.
    pub fn is_active(&self) -> bool {
        self.overlay.is_some()
    }

    /// An injected event, just after it went to the backend.
    pub fn on_inject(&mut self, i: &Inject, now: Instant) {
        let mut moved = false;
        match *i {
            Inject::Controlled(c) if c != self.controlled => {
                self.controlled = c;
                self.relative = false;
                self.pos = None;
                self.failed = false;
                self.device_seen = false;
                if c {
                    self.start(now);
                }
            }
            Inject::MoveTo(p) => {
                self.pos = Some(p);
                self.relative = false;
                moved = true;
            }
            Inject::MoveBy { .. } => self.relative = true,
            _ => return,
        }
        if self.controlled {
            if self.probed_at.is_none_or(|t| now.saturating_duration_since(t) >= PROBE_EVERY) {
                self.probe_now(now);
            }
            if self.overlay.as_ref().is_some_and(|o| o.device_appeared()) {
                self.device_seen = true;
            }
        }
        let existed = self.overlay.is_some();
        self.sync(now);
        if let (true, true, Some(o), Some(p)) = (moved, existed, self.overlay.as_mut(), self.pos) {
            o.move_to(p);
        }
    }

    /// Control starts: ask the OS, and in `auto` mode try to make it show
    /// its own pointer before drawing ours.
    fn start(&mut self, now: Instant) {
        self.probe_now(now);
        let facts = self.facts();
        if self.mode == DrawMode::Auto
            && wants(self.mode, &Facts { relative: false, ..facts })
            && self.platform.as_mut().is_some_and(|p| p.reveal())
        {
            self.probe_now(now);
        }
    }

    fn probe_now(&mut self, now: Instant) {
        if let Some(p) = self.platform.as_mut() {
            self.probe = p.probe();
        }
        self.probed_at = Some(now);
    }

    fn facts(&self) -> Facts {
        Facts {
            controlled: self.controlled,
            relative: self.relative,
            pointing_device: if self.device_seen { Some(true) } else { self.probe.pointing_device },
            os_cursor: self.probe.os_cursor,
            can_draw: self.platform.is_some() && !self.failed,
        }
    }

    fn sync(&mut self, now: Instant) {
        let on = self.decider.update(self.mode, &self.facts(), now);
        match (on, self.overlay.is_some(), self.pos) {
            (true, false, Some(at)) => {
                let Some(platform) = self.platform.as_mut() else { return };
                match platform.create(at) {
                    Ok(o) => {
                        tracing::info!("drawing the pointer (no mouse here, or `pointer-overlay always`)");
                        self.overlay = Some(o);
                    }
                    Err(e) => {
                        tracing::warn!("can't draw the pointer: {e:#}");
                        self.failed = true;
                    }
                }
            }
            (false, true, _) => {
                tracing::debug!("pointer overlay removed");
                self.overlay = None;
            }
            _ => {}
        }
        self.active.store(self.overlay.is_some(), Ordering::Relaxed);
    }
}

/// The daemon's sender of injected input: forwards to the backend and, where
/// a pointer may be drawn, shows the [`Controller`] what was sent.
#[derive(Clone)]
pub struct InjectTx {
    tx: mpsc::UnboundedSender<Inject>,
    overlay: Option<Arc<Mutex<Controller>>>,
    active: Arc<AtomicBool>,
}

impl InjectTx {
    /// With a pointer overlay for `platform` (none if `None`).
    pub fn new(tx: mpsc::UnboundedSender<Inject>, platform: Option<Box<dyn Platform>>, mode: DrawMode) -> Self {
        match platform {
            Some(p) => {
                let c = Controller::new(Some(p), mode);
                let active = c.active.clone();
                Self { tx, overlay: Some(Arc::new(Mutex::new(c))), active }
            }
            None => Self::plain(tx),
        }
    }

    /// Without a pointer overlay.
    pub fn plain(tx: mpsc::UnboundedSender<Inject>) -> Self {
        Self { tx, overlay: None, active: Arc::default() }
    }

    pub fn send(&self, i: Inject) -> Result<(), mpsc::error::SendError<Inject>> {
        let sent = self.tx.send(i);
        if let Some(c) = &self.overlay
            && matches!(i, Inject::Controlled(_) | Inject::MoveTo(_) | Inject::MoveBy { .. })
            && let Ok(mut c) = c.lock()
        {
            c.on_inject(&i, Instant::now());
        }
        sent
    }

    /// The `[pointer] draw_when_no_mouse` setting changed.
    pub fn set_overlay_mode(&self, mode: DrawMode) {
        if let Some(Ok(mut c)) = self.overlay.as_ref().map(|c| c.lock()) {
            c.set_mode(mode, Instant::now());
        }
    }

    /// Whether one-mk draws the pointer right now (for `one-mk status`).
    pub fn overlay_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------- the arrow

/// The arrow at 1x, 12 x 19 pixels, tip at the top left: `#` outline,
/// `o` fill, `.` transparent. The shape of the classic pointer, so it reads
/// as "the pointer" on every OS.
const ARROW: [&str; 19] = [
    "#...........",
    "##..........",
    "#o#.........",
    "#oo#........",
    "#ooo#.......",
    "#oooo#......",
    "#ooooo#.....",
    "#oooooo#....",
    "#ooooooo#...",
    "#oooooooo#..",
    "#ooooooooo#.",
    "#oooooo#####",
    "#ooo#oo#....",
    "#oo##oo#....",
    "#o#..#oo#...",
    "##...#oo#...",
    "#.....#oo#..",
    "......#oo#..",
    ".......##...",
];

/// One pixel of the arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(windows, target_os = "linux", test)), allow(dead_code))]
pub(crate) enum Px {
    Clear,
    Outline,
    Fill,
}

/// The arrow drawn at `scale` (1, 2 or 3; anything else is clamped), as
/// rows of pixels; the tip (the hot spot) is the top-left pixel.
#[cfg_attr(not(any(windows, target_os = "linux", test)), allow(dead_code))]
pub(crate) fn arrow(scale: u32) -> Vec<Vec<Px>> {
    let s = scale.clamp(1, 3) as usize;
    let mut rows = Vec::with_capacity(ARROW.len() * s);
    for line in ARROW {
        let row: Vec<Px> = line
            .bytes()
            .flat_map(|b| {
                let px = match b {
                    b'#' => Px::Outline,
                    b'o' => Px::Fill,
                    _ => Px::Clear,
                };
                std::iter::repeat_n(px, s)
            })
            .collect();
        rows.extend(std::iter::repeat_n(row, s));
    }
    rows
}

/// The drawing scale for a display scale factor (1.0 = 96 DPI).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn arrow_scale(factor: f64) -> u32 {
    if factor.is_finite() { (factor.round() as u32).clamp(1, 3) } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// What the fake platform did, shared with the test.
    #[derive(Default)]
    struct Log {
        created: AtomicUsize,
        dropped: AtomicUsize,
        probes: AtomicUsize,
        reveals: AtomicUsize,
        moves: Mutex<Vec<Pos>>,
        probe: Mutex<Probe>,
        reveal_works: AtomicBool,
        fail_create: AtomicBool,
        device_appeared: AtomicBool,
    }

    impl Log {
        fn live(&self) -> usize {
            self.created.load(Ordering::SeqCst) - self.dropped.load(Ordering::SeqCst)
        }
        fn set(&self, device: Option<bool>, os_cursor: OsCursor) {
            *self.probe.lock().unwrap() = Probe { pointing_device: device, os_cursor };
        }
    }

    struct Fake(Arc<Log>);
    struct FakeOverlay(Arc<Log>);

    impl Platform for Fake {
        fn probe(&mut self) -> Probe {
            self.0.probes.fetch_add(1, Ordering::SeqCst);
            *self.0.probe.lock().unwrap()
        }
        fn reveal(&mut self) -> bool {
            self.0.reveals.fetch_add(1, Ordering::SeqCst);
            let works = self.0.reveal_works.load(Ordering::SeqCst);
            if works {
                self.0.probe.lock().unwrap().os_cursor = OsCursor::Shown;
            }
            works
        }
        fn create(&mut self, at: Pos) -> anyhow::Result<Box<dyn Overlay>> {
            if self.0.fail_create.load(Ordering::SeqCst) {
                anyhow::bail!("no display");
            }
            self.0.created.fetch_add(1, Ordering::SeqCst);
            self.0.moves.lock().unwrap().push(at);
            Ok(Box::new(FakeOverlay(self.0.clone())))
        }
    }

    impl Overlay for FakeOverlay {
        fn move_to(&mut self, pos: Pos) {
            self.0.moves.lock().unwrap().push(pos);
        }
        fn device_appeared(&self) -> bool {
            self.0.device_appeared.load(Ordering::SeqCst)
        }
    }

    impl Drop for FakeOverlay {
        fn drop(&mut self) {
            self.0.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn setup(mode: DrawMode) -> (InjectTx, mpsc::UnboundedReceiver<Inject>, Arc<Log>) {
        let log = Arc::new(Log::default());
        log.set(Some(false), OsCursor::Hidden);
        let (tx, rx) = mpsc::unbounded_channel();
        (InjectTx::new(tx, Some(Box::new(Fake(log.clone()))), mode), rx, log)
    }

    fn p(x: f64, y: f64) -> Pos {
        Pos::new(x, y)
    }

    /// Created when a peer takes control, follows the moves, gone when
    /// control ends; every event still reaches the backend in order.
    #[test]
    fn created_on_control_destroyed_after() {
        let (tx, mut rx, log) = setup(DrawMode::Auto);
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        assert_eq!((log.live(), log.probes.load(Ordering::SeqCst)), (0, 0), "not controlled: nothing");
        tx.send(Inject::Controlled(true)).unwrap();
        assert_eq!(log.live(), 0, "no position yet");
        tx.send(Inject::MoveTo(p(10.0, 20.0))).unwrap();
        assert_eq!(log.live(), 1);
        assert!(tx.overlay_active());
        tx.send(Inject::Key { code: 30, down: true }).unwrap();
        tx.send(Inject::MoveTo(p(11.0, 21.0))).unwrap();
        assert_eq!(*log.moves.lock().unwrap(), [p(10.0, 20.0), p(11.0, 21.0)]);
        tx.send(Inject::Controlled(false)).unwrap();
        assert_eq!(log.live(), 0);
        assert!(!tx.overlay_active());
        let sent: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(sent.len(), 6, "{sent:?}");
        assert_eq!(sent[3], Inject::Key { code: 30, down: true });
    }

    /// Many control sessions leave nothing behind.
    #[test]
    fn no_leak_over_many_cycles() {
        let (tx, _rx, log) = setup(DrawMode::Auto);
        for i in 0..200 {
            tx.send(Inject::Controlled(true)).unwrap();
            tx.send(Inject::MoveTo(p(f64::from(i), 5.0))).unwrap();
            tx.send(Inject::MoveTo(p(f64::from(i) + 1.0, 5.0))).unwrap();
            assert_eq!(log.live(), 1);
            tx.send(Inject::Controlled(false)).unwrap();
            assert_eq!(log.live(), 0);
        }
        assert_eq!(log.created.load(Ordering::SeqCst), 200);
        // A repeated `Controlled(true)` doesn't make a second one.
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(2.0, 1.0))).unwrap();
        assert_eq!(log.created.load(Ordering::SeqCst), 201);
        drop(tx);
        assert_eq!(log.live(), 0, "dropped with the daemon's sender");
    }

    /// With a mouse and a visible OS pointer, auto draws nothing and the
    /// probe runs at most every `PROBE_EVERY`.
    #[test]
    fn auto_with_a_mouse_draws_nothing() {
        let log = Arc::new(Log::default());
        log.set(Some(true), OsCursor::Shown);
        let mut c = Controller::new(Some(Box::new(Fake(log.clone()))), DrawMode::Auto);
        let t = Instant::now();
        c.on_inject(&Inject::Controlled(true), t);
        for ms in 0..1000 {
            c.on_inject(&Inject::MoveTo(p(ms as f64, 0.0)), t + Duration::from_millis(ms));
        }
        assert_eq!(log.created.load(Ordering::SeqCst), 0);
        assert_eq!(log.probes.load(Ordering::SeqCst), 2, "at the start, then once more after 500 ms");
        assert_eq!(log.reveals.load(Ordering::SeqCst), 0, "nothing to reveal");
        // `always` draws anyway, at once.
        c.set_mode(DrawMode::Always, t + Duration::from_secs(1));
        assert!(c.is_active());
        c.set_mode(DrawMode::Never, t + Duration::from_secs(1));
        assert!(!c.is_active());
    }

    /// A mouse plugged in while drawn: the overlay goes at the next event.
    #[test]
    fn a_real_mouse_ends_it() {
        let (tx, _rx, log) = setup(DrawMode::Auto);
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        assert_eq!(log.live(), 1);
        log.device_appeared.store(true, Ordering::SeqCst);
        tx.send(Inject::MoveTo(p(2.0, 1.0))).unwrap();
        assert_eq!(log.live(), 0);
        // Even if the probe hasn't caught up yet, it stays gone.
        std::thread::sleep(PROBE_EVERY);
        tx.send(Inject::MoveTo(p(3.0, 1.0))).unwrap();
        assert_eq!(log.live(), 0);
        log.device_appeared.store(false, Ordering::SeqCst);
        tx.send(Inject::Controlled(false)).unwrap();
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(3.0, 1.0))).unwrap();
        assert_eq!(log.live(), 1, "the next session asks again");
    }

    /// When the OS can be made to show its own pointer, ours isn't drawn.
    #[test]
    fn reveal_first() {
        let (tx, _rx, log) = setup(DrawMode::Auto);
        log.reveal_works.store(true, Ordering::SeqCst);
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        assert_eq!(log.reveals.load(Ordering::SeqCst), 1);
        assert_eq!(log.created.load(Ordering::SeqCst), 0);
    }

    /// Gaming mode (raw motion) hides it; positions bring it back.
    #[test]
    fn relative_motion_hides_it() {
        let (tx, _rx, log) = setup(DrawMode::Always);
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        tx.send(Inject::MoveBy { dx: 3, dy: 0 }).unwrap();
        assert_eq!(log.live(), 0);
        tx.send(Inject::MoveTo(p(5.0, 1.0))).unwrap();
        assert_eq!(log.live(), 1);
        assert_eq!(log.created.load(Ordering::SeqCst), 2);
    }

    /// A failure to create is tried once per control session, not on
    /// every move.
    #[test]
    fn create_failure_is_not_retried_every_move() {
        let (tx, _rx, log) = setup(DrawMode::Always);
        log.fail_create.store(true, Ordering::SeqCst);
        tx.send(Inject::Controlled(true)).unwrap();
        for i in 0..10 {
            tx.send(Inject::MoveTo(p(f64::from(i), 1.0))).unwrap();
        }
        assert!(!tx.overlay_active());
        log.fail_create.store(false, Ordering::SeqCst);
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        assert_eq!(log.live(), 0, "not this session");
        tx.send(Inject::Controlled(false)).unwrap();
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        assert_eq!(log.live(), 1, "the next one tries again");
    }

    /// No platform (Wayland, null): a plain forwarder.
    #[test]
    fn without_a_platform_it_only_forwards() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let tx = InjectTx::new(tx, None, DrawMode::Always);
        tx.send(Inject::Controlled(true)).unwrap();
        tx.send(Inject::MoveTo(p(1.0, 1.0))).unwrap();
        tx.set_overlay_mode(DrawMode::Always);
        assert!(!tx.overlay_active());
        assert_eq!(rx.try_recv().unwrap(), Inject::Controlled(true));
    }

    #[test]
    fn arrow_shapes() {
        for s in 1..=3 {
            let a = arrow(s);
            assert_eq!((a.len(), a[0].len()), (19 * s as usize, 12 * s as usize));
            assert_eq!(a[0][0], Px::Outline, "the tip is the hot spot");
            assert!(a.iter().all(|r| r.len() == a[0].len()));
        }
        assert_eq!(arrow(9).len(), arrow(3).len());
        assert_eq!((arrow_scale(1.0), arrow_scale(1.25), arrow_scale(1.5), arrow_scale(2.0), arrow_scale(4.0)), (1, 1, 2, 2, 3));
        assert_eq!(arrow_scale(f64::NAN), 1);
    }

    #[test]
    fn support_by_backend() {
        assert_eq!(support("null"), Support::NoInput);
        assert_eq!(support("x11"), Support::NotNeeded);
        assert_eq!(support("portal"), Support::NotAvailable);
        assert_eq!(support("wlroots"), Support::NotAvailable);
        let native = if cfg!(any(windows, target_os = "macos")) { Support::Available } else { Support::NotAvailable };
        assert_eq!(support("native"), native);
        assert!(platform("portal").is_none() && platform("null").is_none());
    }
}
