//! Linux backend built on XDG desktop portals and libei, so it works on
//! Wayland (GNOME, KDE) without root, udev rules or the `input` group.
//!
//! - Capture: `org.freedesktop.portal.InputCapture` with pointer barriers on
//!   the edges that lead to peers. Only this side reports `Event::Desktop`.
//! - Injection: `org.freedesktop.portal.RemoteDesktop`. The permission is
//!   remembered with a restore token (`remote-desktop.token`), so the user
//!   is asked only once. InputCapture gets the same where the desktop
//!   supports it (version 2, `input-capture.token`); GNOME 50 doesn't yet,
//!   so there the capture dialog comes up at every start.
//!
//! Nothing may stay stuck when something goes wrong: not the pointer at an
//! edge, not a key or button held down. So:
//! - Each session has a D-Bus connection of its own, dropped with it. The
//!   portal closes every session of a connection that goes away, and the
//!   compositor then ends capture and lets go of injected keys, even if our
//!   own goodbye never arrives (a hung call, a crash, a kill).
//! - Every portal call that should answer at once has a timeout; a portal
//!   that hangs counts as a lost session. Only the calls that may show a
//!   permission dialog wait as long as the user needs.
//! - However a session ends (closed by the desktop, the portal restarting,
//!   the session bus or the libei connection going away, a panic in our
//!   code), capture in progress ends with `Event::Deactivated`, so the
//!   daemon brings the pointer home, keys we hold are let go of, and the
//!   session is re-created with backoff (1 s, 2 s, 4 s ... 30 s; back to
//!   1 s after a session that lasted a minute). Only the user refusing
//!   permission, or the portal not existing at all, stops it for good.
//! - When the screen locks or the computer is about to sleep, capture
//!   ends and held keys are let go of; after waking the screen layout is
//!   read again.
//!
//! `Event::LocalInput` is never sent by this backend: the portals give us
//! no way to see real input on this machine while a peer controls it.
//! InputCapture only delivers input after a barrier activation, and libei
//! senders (RemoteDesktop) never see input at all. Faking it (e.g. from
//! cursor position changes we didn't cause) would misfire, so we don't.
//! Instead the person here takes control back with `one-mk takeback`, and
//! the daemon lets a controller that sent no real input for a minute go.

use crate::common_logic::{Backoff, drain_coalesced};
use crate::{Armed, Backend, CaptureCmd, Event, Inject};
use anyhow::{Context, Result, anyhow, bail};
use ashpd::desktop::input_capture::{
    self, Barrier, Capabilities, InputCapture, ReleaseOptions, SetPointerBarriersOptions,
};
use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, ResponseError, Session};
use ashpd::zbus;
use futures::{FutureExt, StreamExt};
use one_mk_proto::{Desktop, Mods, Pos, Rect, Side};
use reis::ei;
use reis::event::{DeviceCapability, EiEvent};
use std::collections::HashMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// A session that lasted at least this long starts the backoff over.
const STABLE: Duration = Duration::from_secs(60);
/// Portal calls that need no answer from the user get this long. Their
/// answer normally takes milliseconds.
const CALL_TIMEOUT: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 5 });
/// A permission request still open after this has a dialog up.
const DIALOG_AFTER: Duration = Duration::from_millis(if cfg!(test) { 200 } else { 1000 });
/// How long logind waits for us before the computer sleeps (we hold a
/// delay lock): enough to send a few messages to the portal.
const SLEEP_GRACE: Duration = Duration::from_millis(300);

pub(crate) const REMOTE_DESKTOP_TOKEN: &str = "remote-desktop.token";
pub(crate) const INPUT_CAPTURE_TOKEN: &str = "input-capture.token";

pub(crate) async fn open(state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    // The lock watcher's events pass through `watch_power` on their way.
    let (sys_tx, sys_rx) = mpsc::unbounded_channel();
    let (focus, locker) = crate::system::native(&sys_tx);
    drop(sys_tx);
    let (power_tx, power) = watch::channel(Power::default());
    tokio::spawn(watch_power(sys_rx, ev_tx.clone(), power_tx));
    let env = Env { bus: None, unit: Duration::from_secs(1), power };
    let (capture, inject) = start(env, ev_tx, state_dir)?;
    Ok(Backend { name: "portal", events: ev_rx, capture, inject, focus, locker })
}

type Channels = (mpsc::UnboundedSender<CaptureCmd>, mpsc::UnboundedSender<Inject>);

/// Starts both sides; they run until the daemon drops its senders.
fn start(env: Env, ev_tx: mpsc::UnboundedSender<Event>, state_dir: PathBuf) -> Result<Channels> {
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    let (tx, warp, cap_env, cap_dir) = (ev_tx.clone(), inj_tx.clone(), env.clone(), state_dir.clone());
    spawn_thread("one-mk-capture", move || capture(cap_env, tx, cap_rx, warp, cap_dir))?;
    spawn_thread("one-mk-inject", move || inject(env, ev_tx, inj_rx, state_dir))?;
    Ok((cap_tx, inj_tx))
}

/// libei streams are not `Send`, and input should not queue behind network
/// work anyway, so capture and injection each get their own thread. The
/// future is built on that thread, so it never needs to be `Send`.
fn spawn_thread<F, Fut>(name: &str, make: F) -> Result<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    std::thread::Builder::new().name(name.into()).spawn(move || rt.block_on(make()))?;
    Ok(())
}

/// What the sessions need from outside: where the portals are, how fast
/// to retry, and whether the screen is locked or the computer asleep.
/// Tests point it at a fake portal on a private bus.
#[derive(Clone)]
struct Env {
    /// The session bus address; `None` for the user's own.
    bus: Option<String>,
    /// One step of the backoff: a second, except in tests.
    unit: Duration,
    power: watch::Receiver<Power>,
}

impl Env {
    /// A new connection for one session (see the module docs for why).
    async fn connect(&self) -> Result<zbus::Connection> {
        let conn = async {
            match &self.bus {
                None => zbus::Connection::session().await,
                Some(addr) => zbus::connection::Builder::address(addr.as_str())?.build().await,
            }
        };
        timeout("connecting to the session bus", conn).await.context("can't reach the desktop's session bus")
    }
}

/// Things that pause input sharing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Power {
    locked: bool,
    asleep: bool,
}

impl Power {
    fn paused(self) -> bool {
        self.locked || self.asleep
    }
}

/// A call that should answer at once. One that hangs is an error, which
/// ends the session (and so releases whatever it held).
async fn timeout<T, E>(what: &str, f: impl Future<Output = Result<T, E>>) -> Result<T>
where
    E: Into<anyhow::Error>,
{
    match tokio::time::timeout(CALL_TIMEOUT, f).await {
        Ok(r) => r.map_err(Into::into),
        Err(_) => Err(anyhow!("the desktop portal did not answer ({what})")),
    }
}

/// Errors that retrying won't fix: the user said no, or the portal doesn't
/// exist on this desktop. Returns what to tell the user.
fn permanent(e: &anyhow::Error) -> Option<String> {
    e.chain().find_map(|c| match c.downcast_ref::<ashpd::Error>()? {
        ashpd::Error::Response(ResponseError::Cancelled) => {
            Some("you declined the desktop's permission dialog. Restart one-mk to be asked again.".to_owned())
        }
        ashpd::Error::PortalNotFound(name) => {
            Some(format!("this desktop has no {name} portal (`one-mk doctor` tells more)."))
        }
        _ => None,
    })
}

/// The portal answered, and refused: as opposed to not answering at all
/// (then a restore token may well be fine).
fn refused(e: &anyhow::Error) -> bool {
    let answered = |z: &zbus::Error| matches!(z, zbus::Error::MethodError(..) | zbus::Error::FDO(_));
    e.chain().any(|c| match c.downcast_ref::<ashpd::Error>() {
        Some(ashpd::Error::Response(_)) => true,
        Some(ashpd::Error::Portal(ashpd::PortalError::ZBus(z)) | ashpd::Error::Zbus(z)) => answered(z),
        Some(ashpd::Error::Portal(_)) => true,
        _ => false,
    })
}

/// The desktop did not take our restore token; it was deleted, and the
/// next try asks the user afresh.
#[derive(Debug)]
struct TokenRejected;

impl std::fmt::Display for TokenRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the desktop did not accept the saved permission")
    }
}

/// What a session loop does after its session ended with `result`.
#[derive(Debug, PartialEq)]
enum Next {
    /// The daemon is gone, or the failure is permanent.
    Stop,
    Retry(Duration),
}

/// Retry policy and user-facing state of one side.
struct Retry {
    tx: mpsc::UnboundedSender<Event>,
    /// For `Event::Status`.
    part: &'static str,
    /// The feature, for the user, e.g. "Can't share this computer's mouse
    /// with others".
    what: &'static str,
    /// Shown while the permission dialog is open.
    dialog: &'static str,
    backoff: Backoff,
    unit: Duration,
    /// A session was fully set up at least once.
    ever_ready: bool,
    /// The user has been told about the current run of failures.
    reported: bool,
    /// The problem `one-mk status` shows now.
    shown: Option<String>,
}

impl Retry {
    fn new(tx: mpsc::UnboundedSender<Event>, part: &'static str, what: &'static str, dialog: &'static str) -> Self {
        Self {
            tx,
            part,
            what,
            dialog,
            backoff: Backoff::default(),
            unit: Duration::from_secs(1),
            ever_ready: false,
            reported: false,
            shown: None,
        }
    }

    fn show(&mut self, problem: Option<String>) {
        if self.shown != problem {
            self.shown.clone_from(&problem);
            let _ = self.tx.send(Event::Status { part: self.part, problem });
        }
    }

    fn ready(&mut self) {
        self.ever_ready = true;
        self.reported = false;
        self.show(None);
    }

    /// Waits for a request that may show a permission dialog. After a
    /// moment the user is told it waits for them; it never times out.
    async fn asking<T>(&mut self, f: impl Future<Output = T>) -> T {
        tokio::pin!(f);
        tokio::select! {
            r = &mut f => return r,
            _ = tokio::time::sleep(DIALOG_AFTER) => {}
        }
        tracing::info!("{}", self.dialog);
        self.show(Some(self.dialog.to_owned()));
        f.await
    }

    fn after(&mut self, result: Result<()>, ran_for: Duration) -> Next {
        let e = match result {
            Ok(()) => return Next::Stop,
            Err(e) => e,
        };
        if let Some(why) = permanent(&e) {
            tracing::debug!("{}: {e:#}", self.what);
            self.show(Some(format!("{}: {why}", self.what)));
            return Next::Stop;
        }
        if e.downcast_ref::<TokenRejected>().is_some() {
            // The next try has no token, so this can't repeat.
            tracing::warn!("{}: {e:#}; asking again", self.what);
            return Next::Retry(Duration::ZERO);
        }
        if ran_for >= STABLE {
            self.backoff.reset();
        }
        let delay = self.unit * self.backoff.next_delay().as_secs() as u32;
        tracing::info!("{}: {e:#}; trying again in {delay:?}", self.what);
        // A session that was working and just got lost (sleep/wake) is
        // routine; only tell the user once getting it back fails too.
        if !self.reported && (!self.ever_ready || ran_for < STABLE) {
            self.reported = true;
            self.show(Some(format!("{}: {e:#}. Retrying in the background.", self.what)));
        }
        Next::Retry(delay)
    }
}

/// A restore token: the portal's record that the user allowed us, so the
/// next session starts without asking. Each one is good for one use; the
/// portal hands out a new one with every session.
struct TokenFile(PathBuf);

impl TokenFile {
    fn load(&self) -> Option<String> {
        let t = std::fs::read_to_string(&self.0).ok()?;
        let t = t.trim();
        (!t.is_empty()).then(|| t.to_owned())
    }

    /// Saves `token`, or forgets the old one if there is none.
    fn store(&self, token: Option<&str>) {
        let result = match token {
            Some(t) => write_private(&self.0, t),
            None => match std::fs::remove_file(&self.0) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                r => r,
            },
        };
        if let Err(e) = result {
            tracing::warn!("saving the desktop's permission in {}: {e}", self.0.display());
        }
    }
}

/// Writes `path` whole or not at all (a crash leaves the old file), and
/// readable by this user only: the token stands for a permission.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    // A temporary file left over by an older version may be readable.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Starts a session with the restore token in `tokens`, saving the new one.
/// `start` gets the token to restore. If the desktop refuses a session
/// with a token, the token is deleted and `TokenRejected` returned, so the
/// next try asks the user instead.
async fn with_token<T>(
    tokens: &TokenFile,
    retry: &mut Retry,
    start: impl AsyncFnOnce(Option<String>, &mut Retry) -> Result<T>,
    new_token: impl FnOnce(&T) -> Option<&str>,
) -> Result<T> {
    let token = tokens.load();
    let restoring = token.is_some();
    match start(token, retry).await {
        Ok(started) => {
            tokens.store(new_token(&started));
            Ok(started)
        }
        Err(e) if restoring && refused(&e) => {
            tokens.store(None);
            if permanent(&e).is_some() { Err(e) } else { Err(e.context(TokenRejected)) }
        }
        Err(e) => Err(e),
    }
}

/// Closes a session we are done with; this also ends any capture or
/// emulation the compositor still holds for it. Best effort: the session
/// may be gone already (and dropping the connection afterwards closes it
/// anyway).
async fn close<T: ashpd::desktop::SessionPortal>(session: &Session<T>) {
    let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
}

/// Runs a session's main loop, turning a panic in it into an error, so the
/// session is still closed and re-created. (Release builds abort on
/// panic; then the process ending closes the sessions.)
async fn guarded(what: &str, f: impl Future<Output = Result<()>>) -> Result<()> {
    AssertUnwindSafe(f).catch_unwind().await.unwrap_or_else(|_| Err(anyhow!("{what} crashed")))
}

/// Waits for the next change of `power`, forever once its sender is gone.
async fn power_changed(power: &mut watch::Receiver<Power>) -> Power {
    if power.changed().await.is_err() {
        std::future::pending::<()>().await;
    }
    *power.borrow_and_update()
}

/// Sleeps `delay` between sessions, still following the daemon. Returns
/// false once the daemon is gone.
async fn wait_retry<T>(delay: Duration, rx: &mut mpsc::UnboundedReceiver<T>, mut on: impl FnMut(T)) -> bool {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return true,
            m = rx.recv() => match m {
                Some(m) => on(m),
                None => return false,
            },
        }
    }
}

// ------------------------------------------------------------ sleep, lock

/// Keeps `power` up to date: the screen lock from the system watcher's
/// events (passed on to the daemon), and logind's `PrepareForSleep`.
async fn watch_power(
    mut sys: mpsc::UnboundedReceiver<Event>,
    daemon: mpsc::UnboundedSender<Event>,
    power: watch::Sender<Power>,
) {
    let mut sleep = match Sleep::watch().await {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::debug!("not watching for sleep: {e}");
            None
        }
    };
    let mut sys_open = true;
    loop {
        tokio::select! {
            ev = sys.recv(), if sys_open => match ev {
                Some(ev) => {
                    if let Event::ScreenLocked(locked) = ev {
                        power.send_if_modified(|p| std::mem::replace(&mut p.locked, locked) != locked);
                    }
                    if daemon.send(ev).is_err() {
                        return;
                    }
                }
                None => sys_open = false,
            },
            asleep = Sleep::next(&mut sleep) => match asleep {
                Some(true) => {
                    tracing::debug!("going to sleep");
                    power.send_modify(|p| p.asleep = true);
                    // Give both sides a moment to let go, then let logind go on.
                    tokio::time::sleep(SLEEP_GRACE).await;
                    if let Some(s) = sleep.as_mut() {
                        s.inhibitor = None;
                    }
                }
                Some(false) => {
                    tracing::debug!("woke up");
                    power.send_modify(|p| p.asleep = false);
                    if let Some(s) = sleep.as_mut() {
                        s.inhibit().await;
                    }
                }
                None => sleep = None,
            },
            _ = daemon.closed() => return,
        }
    }
}

/// logind's sleep notifications, with a delay lock so they come early
/// enough to act on.
struct Sleep {
    manager: zbus::Proxy<'static>,
    signals: zbus::proxy::SignalStream<'static>,
    inhibitor: Option<zbus::zvariant::OwnedFd>,
}

impl Sleep {
    async fn watch() -> zbus::Result<Sleep> {
        let conn = zbus::Connection::system().await?;
        let manager = zbus::Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await?;
        let signals = manager.receive_signal("PrepareForSleep").await?;
        let mut s = Sleep { manager, signals, inhibitor: None };
        s.inhibit().await;
        Ok(s)
    }

    async fn inhibit(&mut self) {
        let why = "Hand the shared mouse and keyboard back first";
        match self.manager.call("Inhibit", &("sleep", "one-mk", why, "delay")).await {
            Ok(fd) => self.inhibitor = Some(fd),
            Err(e) => tracing::debug!("no sleep delay lock: {e}"),
        }
    }

    /// The next `PrepareForSleep`: `true` before sleeping, `false` after.
    /// `None` when logind went away; pending forever without a watcher.
    async fn next(this: &mut Option<Sleep>) -> Option<bool> {
        let Some(s) = this.as_mut() else { return std::future::pending().await };
        loop {
            let msg = s.signals.next().await?;
            if let Ok(start) = msg.body().deserialize::<bool>() {
                return Some(start);
            }
        }
    }
}

// ---------------------------------------------------------------- capture

/// Capture state that outlives a portal session.
struct CaptureState {
    tx: mpsc::UnboundedSender<Event>,
    sides: Vec<Armed>,
    /// The desktop last reported to the daemon.
    reported: Option<Desktop>,
    next_id: u32,
    /// Our activation id, and the portal's id for the same activation.
    activation: Option<(u32, Option<u32>)>,
    /// This machine's injector, to push the pointer into a barrier.
    warp: mpsc::UnboundedSender<Inject>,
}

impl CaptureState {
    /// Ends the current activation, if any, telling the daemon.
    fn deactivated(&mut self) {
        if let Some((id, _)) = self.activation.take() {
            let _ = self.tx.send(Event::Deactivated { id });
        }
    }

    fn report_desktop(&mut self, desktop: &Desktop) {
        if self.reported.as_ref() != Some(desktop) {
            self.reported = Some(desktop.clone());
            let _ = self.tx.send(Event::Desktop(desktop.clone()));
        }
    }
}

struct Capture<'a> {
    portal: &'a InputCapture,
    session: &'a Session<InputCapture>,
    desktop: Desktop,
    zone_set: u32,
    barriers: HashMap<u32, Side>,
    enabled: bool,
}

impl Capture<'_> {
    async fn refresh_zones(&mut self) -> Result<()> {
        let zones = timeout("screen layout", self.portal.zones(self.session, Default::default())).await?.response()?;
        let screens: Vec<Rect> = zones
            .regions()
            .iter()
            .map(|r| Rect::new(r.x_offset(), r.y_offset(), r.width() as i32, r.height() as i32))
            .collect();
        if screens.is_empty() {
            bail!("the desktop reported no screens");
        }
        self.desktop = Desktop::new(screens);
        self.zone_set = zones.zone_set();
        Ok(())
    }

    /// Barriers are line segments: only the parts of each edge that lead
    /// to a peer get one (`Armed::spans`), so touching the rest never
    /// activates capture.
    async fn apply_barriers(&mut self, sides: &[Armed]) -> Result<()> {
        let mut barriers = Vec::new();
        self.barriers.clear();
        for armed in sides {
            let side = armed.side;
            for (r, first, last) in armed.segments(&self.desktop) {
                let id = side as u32 * 64 + barriers.len() as u32 + 1;
                let (x, y, right, bottom) = (r.x, r.y, r.right(), r.bottom());
                // Barriers lie on the zone edge; right/bottom edges are one
                // past the last pixel, as the portal spec requires.
                let pos = match side {
                    Side::Left => (x, first, x, last),
                    Side::Right => (right, first, right, last),
                    Side::Top => (first, y, last, y),
                    Side::Bottom => (first, bottom, last, bottom),
                };
                barriers.push(Barrier::new(NonZeroU32::new(id).expect("nonzero"), pos));
                self.barriers.insert(id, side);
            }
        }
        let opts = SetPointerBarriersOptions::default();
        let set = self.portal.set_pointer_barriers(self.session, &barriers, self.zone_set, opts);
        let resp = timeout("pointer barriers", set).await?.response()?;
        for failed in resp.failed_barriers() {
            tracing::warn!(barrier = failed.get(), "compositor rejected a pointer barrier");
            self.barriers.remove(&failed.get());
        }

        let want = !self.barriers.is_empty();
        if want && !self.enabled {
            timeout("enabling capture", self.portal.enable(self.session, Default::default())).await?;
        } else if !want && self.enabled {
            timeout("disabling capture", self.portal.disable(self.session, Default::default())).await?;
        }
        self.enabled = want;
        Ok(())
    }

    /// Re-reads the zones and re-applies the barriers. Failures are logged
    /// and otherwise ignored: the old layout stays in use, and a session
    /// that is really gone shows up as a closed libei connection.
    async fn refresh(&mut self, sides: &[Armed]) {
        if let Err(e) = self.refresh_zones().await {
            tracing::warn!("reading the screen layout: {e:#}");
        }
        if let Err(e) = self.apply_barriers(sides).await {
            tracing::warn!("setting pointer barriers: {e:#}");
        }
    }

    /// Hands the pointer back. If the portal refuses, disabling capture
    /// also ends it; only if that fails too is the session given up (which
    /// closes it, and closing always releases).
    async fn release(&mut self, portal_id: Option<u32>, pos: Option<Pos>, sides: &[Armed]) -> Result<()> {
        let opts = ReleaseOptions::default()
            .set_activation_id(portal_id)
            .set_cursor_position(pos.map(|p| (p.x, p.y)));
        let Err(e) = timeout("releasing the pointer", self.portal.release(self.session, opts)).await else {
            return Ok(());
        };
        tracing::warn!("releasing the pointer: {e:#}; disabling capture instead");
        timeout("disabling capture", self.portal.disable(self.session, Default::default()))
            .await
            .context("couldn't release the pointer")?;
        self.enabled = false;
        if let Err(e) = self.apply_barriers(sides).await {
            tracing::warn!("setting pointer barriers: {e:#}");
        }
        Ok(())
    }
}

async fn capture(
    env: Env,
    tx: mpsc::UnboundedSender<Event>,
    mut cmds: mpsc::UnboundedReceiver<CaptureCmd>,
    warp: mpsc::UnboundedSender<Inject>,
    state_dir: PathBuf,
) {
    let mut st = CaptureState { tx: tx.clone(), sides: Vec::new(), reported: None, next_id: 0, activation: None, warp };
    let mut retry = Retry::new(
        tx,
        "capture",
        "Can't share this computer's mouse with others",
        "Waiting for your approval: allow one-mk in the desktop's input capture dialog, \
         so this mouse and keyboard can move to other computers.",
    );
    retry.unit = env.unit;
    let tokens = TokenFile(state_dir.join(INPUT_CAPTURE_TOKEN));
    loop {
        let started = Instant::now();
        let result = capture_session(&env, &mut st, &mut cmds, &mut retry, &tokens).await;
        // However the session ended, so did any capture in progress.
        st.deactivated();
        let Next::Retry(delay) = retry.after(result, started.elapsed()) else { return };
        let follow = wait_retry(delay, &mut cmds, |cmd| match cmd {
            CaptureCmd::SetSides(sides) => st.sides = sides,
            CaptureCmd::Release { .. } => {}
            CaptureCmd::Activate { .. } => {
                let _ = st.tx.send(Event::Error(NO_CAPTURE.into()));
            }
        });
        if !follow.await {
            return;
        }
    }
}

/// One InputCapture session. `Ok` means the daemon is gone.
async fn capture_session(
    env: &Env,
    st: &mut CaptureState,
    cmds: &mut mpsc::UnboundedReceiver<CaptureCmd>,
    retry: &mut Retry,
    tokens: &TokenFile,
) -> Result<()> {
    let conn = env.connect().await?;
    let portal = timeout("InputCapture", InputCapture::with_connection(conn))
        .await
        .context("InputCapture portal not available")?;
    let caps = Capabilities::Keyboard | Capabilities::Pointer;
    let v2 = portal.version() >= 2;
    let session = if v2 {
        timeout("creating a session", portal.create_session2(Default::default())).await?
    } else {
        // Version 1 asks the user here, every time.
        let create = portal.create_session(None, input_capture::CreateSessionOptions::default().set_capabilities(caps));
        retry.asking(create).await.context("permission to capture input was not granted")?.0
    };
    let result = guarded("input capture", async {
        if v2 {
            let start = async |token: Option<String>, retry: &mut Retry| {
                let opts = input_capture::StartOptions::default()
                    .set_capabilities(caps)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(token);
                let request = retry.asking(portal.start(&session, None, opts)).await?;
                request.response().context("permission to capture input was not granted")
            };
            with_token(tokens, retry, start, |r| r.restore_token()).await?;
        }
        run_capture(env, &portal, &session, st, cmds, retry).await
    })
    .await;
    close(&session).await;
    result
}

async fn run_capture(
    env: &Env,
    portal: &InputCapture,
    session: &Session<InputCapture>,
    st: &mut CaptureState,
    cmds: &mut mpsc::UnboundedReceiver<CaptureCmd>,
    retry: &mut Retry,
) -> Result<()> {
    let mut closed = session.receive_closed().await?;
    let mut portal_owner = portal.receive_owner_changed().await?;
    let fd = timeout("connecting to libei", portal.connect_to_eis(session, Default::default())).await?;
    let context = ei::Context::new(UnixStream::from(fd))?;
    let handshake = context.handshake_tokio("one-mk", ei::handshake::ContextType::Receiver);
    let (_conn, mut ei_events) = timeout("libei handshake", handshake).await?;

    let mut activated = portal.receive_activated().await?;
    let mut deactivated = portal.receive_deactivated().await?;
    let mut zones_changed = portal.receive_zones_changed().await?;
    let mut disabled = portal.receive_disabled().await?;

    let mut cap = Capture {
        portal,
        session,
        desktop: Desktop::new(vec![Rect::new(0, 0, 1, 1)]),
        zone_set: 0,
        barriers: HashMap::new(),
        enabled: false,
    };
    // Without zones there is nothing to put barriers on, so this one is
    // fatal (and retried with a new session).
    cap.refresh_zones().await?;
    st.report_desktop(&cap.desktop);
    // A re-created session starts with no barriers.
    if let Err(e) = cap.apply_barriers(&st.sides).await {
        tracing::warn!("setting pointer barriers: {e:#}");
    }
    retry.ready();
    tracing::info!(screens = ?cap.desktop.screens, "input capture ready");

    let mut power = env.power.clone();
    let mut was = *power.borrow_and_update();
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(CaptureCmd::SetSides(sides)) => {
                    st.sides = sides;
                    if let Err(e) = cap.apply_barriers(&st.sides).await {
                        tracing::warn!("setting pointer barriers: {e:#}");
                    }
                }
                Some(CaptureCmd::Release { id, pos }) => {
                    if let Some((ours, portal_id)) = st.activation
                        && ours == id
                    {
                        st.activation = None;
                        cap.release(portal_id, pos, &st.sides).await?;
                    }
                }
                Some(CaptureCmd::Activate { side }) => {
                    if st.activation.is_none() && !was.paused() {
                        push_into_barrier(&cap.desktop, st, side);
                    } else if was.paused() {
                        let _ = st.tx.send(Event::Error(NO_CAPTURE.into()));
                    }
                }
                None => return Ok(()),
            },
            Some(a) = activated.next() => {
                // A new activation supersedes one we never saw end.
                st.deactivated();
                let side = match a.barrier_id() {
                    Some(input_capture::ActivatedBarrier::Barrier(id)) => cap.barriers.get(&id.get()).copied(),
                    _ => None,
                };
                let pos = a.cursor_position().map(|(x, y)| Pos::new(f64::from(x), f64::from(y)));
                match (side, pos) {
                    (Some(side), Some(pos)) if !was.paused() => {
                        st.next_id = st.next_id.wrapping_add(1);
                        st.activation = Some((st.next_id, a.activation_id()));
                        let _ = st.tx.send(Event::Activated { id: st.next_id, side, pos });
                    }
                    // Not one of our barriers, or the screen is locked or
                    // going to sleep: hand the pointer straight back.
                    _ => cap.release(a.activation_id(), None, &st.sides).await?,
                }
            },
            Some(d) = deactivated.next() => {
                // Only end the activation this signal is about; a late signal
                // for an earlier one must not end a newer capture.
                if let Some((_, portal_id)) = st.activation
                    && (d.activation_id().is_none() || d.activation_id() == portal_id)
                {
                    st.deactivated();
                }
            },
            Some(_) = zones_changed.next() => {
                cap.refresh(&st.sides).await;
                st.report_desktop(&cap.desktop);
            },
            Some(_) = disabled.next() => {
                // The compositor dropped our barriers (e.g. monitors changed); set them up again.
                st.deactivated();
                cap.enabled = false;
                cap.refresh(&st.sides).await;
                st.report_desktop(&cap.desktop);
            },
            now = power_changed(&mut power) => {
                if now.paused()
                    && let Some((_, portal_id)) = st.activation
                {
                    tracing::info!("screen locked or going to sleep: the pointer comes home");
                    st.deactivated();
                    cap.release(portal_id, None, &st.sides).await?;
                }
                if was.asleep && !now.asleep {
                    // Monitors may have changed while asleep. A session
                    // that can't say where the screens are is no good.
                    cap.refresh_zones().await.context("after waking up")?;
                    st.report_desktop(&cap.desktop);
                    if let Err(e) = cap.apply_barriers(&st.sides).await {
                        tracing::warn!("setting pointer barriers: {e:#}");
                    }
                }
                was = now;
            },
            _ = closed.next() => bail!("the input capture session was closed"),
            owner = portal_owner.next() => {
                // Its sessions died with it, without a word.
                bail!("the desktop portal {}", if owner.flatten().is_some() { "restarted" } else { "went away" });
            },
            ev = ei_events.next() => {
                let Some(ev) = ev else { bail!("input capture connection closed") };
                let ev = match ev? {
                    EiEvent::SeatAdded(s) => {
                        s.seat.bind_capabilities(
                            DeviceCapability::Pointer | DeviceCapability::PointerAbsolute
                                | DeviceCapability::Keyboard | DeviceCapability::Scroll
                                | DeviceCapability::Button,
                        );
                        context.flush()?;
                        continue;
                    }
                    EiEvent::PointerMotion(m) => Event::Motion { dx: f64::from(m.dx), dy: f64::from(m.dy) },
                    EiEvent::Button(b) => Event::Button { button: b.button, down: b.state == ei::button::ButtonState::Press },
                    EiEvent::ScrollDelta(s) => Event::Scroll { dx: f64::from(s.dx), dy: f64::from(s.dy) },
                    EiEvent::ScrollDiscrete(s) => Event::ScrollDiscrete { dx: s.discrete_dx, dy: s.discrete_dy },
                    EiEvent::ScrollStop(_) => Event::ScrollStop,
                    EiEvent::KeyboardKey(k) => Event::Key { code: k.key, down: k.state == ei::keyboard::KeyState::Press },
                    EiEvent::KeyboardModifiers(m) => Event::Modifiers(Mods::from_xkb(m.depressed | m.latched)),
                    EiEvent::Disconnected(d) => bail!("input capture disconnected: {:?}", d.explanation),
                    _ => continue,
                };
                if st.tx.send(ev).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

const NO_CAPTURE: &str = "Can't move the pointer to another computer right now: input capture is not ready.";

/// `CaptureCmd::Activate`: the InputCapture portal has no call to start
/// capturing, only barriers, so this pushes the pointer into the barrier
/// with our own injector; the compositor then activates capture as if the
/// mouse had done it. Needs a barrier on `side` (the daemon arms it).
fn push_into_barrier(desktop: &Desktop, st: &CaptureState, side: Side) {
    // The middle of the first part of the edge with a barrier.
    let segment = st.sides.iter().filter(|a| a.side == side).flat_map(|a| a.segments(desktop)).next();
    let Some((r, first, last)) = segment else {
        let _ = st.tx.send(Event::Error(NO_CAPTURE.into()));
        return;
    };
    let mid = f64::from(first + last) / 2.0;
    let c = if side.along_x() { Pos::new(mid, r.center().y) } else { Pos::new(r.center().x, mid) };
    let (x, y, right, bottom) = (f64::from(r.x), f64::from(r.y), f64::from(r.right()), f64::from(r.bottom()));
    let (at, dx, dy) = match side {
        Side::Left => (Pos::new(x + 1.0, c.y), -64, 0),
        Side::Right => (Pos::new(right - 2.0, c.y), 64, 0),
        Side::Top => (Pos::new(c.x, y + 1.0), 0, -64),
        Side::Bottom => (Pos::new(c.x, bottom - 2.0), 0, 64),
    };
    let _ = st.warp.send(Inject::MoveTo(at));
    let _ = st.warp.send(Inject::MoveBy { dx, dy });
}

// --------------------------------------------------------------- injection

#[derive(Default)]
struct Devices {
    all: Vec<reis::event::Device>,
    /// Devices we have sent events to since the last frame.
    dirty: Vec<reis::event::Device>,
    /// Regions of the absolute pointer device, if it has any. Positions
    /// outside them are ignored by the compositor, so moves are clamped in.
    regions: Option<Desktop>,
}

impl Devices {
    fn with<T: ei::Interface>(&mut self, f: impl FnOnce(&T)) -> bool {
        let Some(dev) = self.all.iter().find(|d| d.interface::<T>().is_some()) else {
            return false;
        };
        f(&dev.interface::<T>().expect("checked above"));
        if !self.dirty.contains(dev) {
            self.dirty.push(dev.clone());
        }
        true
    }

    fn update_regions(&mut self) {
        self.regions = self.all.iter().find(|d| d.interface::<ei::PointerAbsolute>().is_some()).and_then(|dev| {
            let rects: Vec<Rect> = dev
                .regions()
                .iter()
                .map(|r| Rect::new(r.x as i32, r.y as i32, r.width as i32, r.height as i32))
                .filter(|r| r.w > 0 && r.h > 0)
                .collect();
            (!rects.is_empty()).then(|| Desktop::new(rects))
        });
    }
}

/// Keys and buttons held down through the current session, to let go of
/// when it ends or the computer locks or sleeps. The daemon releases what
/// a peer held too, but that needs a working session and a daemon that
/// still runs; this doesn't.
#[derive(Debug, Default)]
struct Held {
    keys: Vec<u32>,
    buttons: Vec<u32>,
}

impl Held {
    fn update(&mut self, m: &Inject) {
        let (list, code, down) = match *m {
            Inject::Key { code, down } => (&mut self.keys, code, down),
            Inject::Button { button, down } => (&mut self.buttons, button, down),
            _ => return,
        };
        list.retain(|&c| c != code);
        if down {
            list.push(code);
        }
    }

    /// Releases for everything held, last pressed first; forgets them.
    fn release(&mut self) -> Vec<Inject> {
        let buttons = self.buttons.drain(..).rev().map(|button| Inject::Button { button, down: false });
        let keys = self.keys.drain(..).rev().map(|code| Inject::Key { code, down: false });
        buttons.chain(keys).collect()
    }
}

struct Injector {
    context: ei::Context,
    conn: reis::event::Connection,
    devices: Devices,
    sequence: u32,
    last: Option<Pos>,
    warned_relative: bool,
    held: Held,
}

impl Injector {
    /// Sends `batch` as one frame.
    fn send(&mut self, batch: impl IntoIterator<Item = Inject>) {
        for m in batch {
            self.held.update(&m);
            apply(&mut self.devices, m, &mut self.last, &mut self.warned_relative);
        }
        let serial = self.conn.serial();
        let now = monotonic_us();
        for dev in self.devices.dirty.drain(..) {
            dev.device().frame(serial, now);
        }
        if let Err(e) = self.context.flush() {
            tracing::warn!("injecting input: {e}");
        }
    }

    /// Lets go of every key and button we hold down. Best effort: if the
    /// connection is gone, so is what it held.
    fn let_go(&mut self) {
        let up = self.held.release();
        if !up.is_empty() {
            tracing::info!(count = up.len(), "letting go of injected keys and buttons");
            self.send(up);
        }
    }
}

async fn inject(env: Env, tx: mpsc::UnboundedSender<Event>, mut rx: mpsc::UnboundedReceiver<Inject>, state_dir: PathBuf) {
    let mut retry = Retry::new(
        tx,
        "control",
        "Other computers can't control this one",
        "Waiting for your approval: allow one-mk in the desktop's remote desktop dialog, \
         so other computers can control this one.",
    );
    retry.unit = env.unit;
    let tokens = TokenFile(state_dir.join(REMOTE_DESKTOP_TOKEN));
    loop {
        let started = Instant::now();
        let result = inject_session(&env, &mut rx, &tokens, &mut retry).await;
        let Next::Retry(delay) = retry.after(result, started.elapsed()) else { return };
        // Nothing can be injected meanwhile; drop it. What the old session
        // held was let go of with it.
        if !wait_retry(delay, &mut rx, drop).await {
            return;
        }
    }
}

/// What the desktop portals offer, for `one-mk doctor`. Only reads portal
/// properties: it never creates a session, so no permission prompt appears.
pub struct PortalProbe {
    pub remote_desktop: Result<(), String>,
    pub input_capture: Result<(), String>,
}

pub async fn probe_portals() -> PortalProbe {
    let (remote_desktop, input_capture) = tokio::join!(probe_remote_desktop(), probe_input_capture());
    PortalProbe { remote_desktop, input_capture }
}

async fn probe_remote_desktop() -> Result<(), String> {
    let portal = RemoteDesktop::new().await.map_err(|e| e.to_string())?;
    let types = portal.available_device_types().await.map_err(|e| e.to_string())?;
    if types.contains(DeviceType::Keyboard | DeviceType::Pointer) {
        Ok(())
    } else {
        Err("it cannot control a keyboard and pointer".to_owned())
    }
}

async fn probe_input_capture() -> Result<(), String> {
    let portal = InputCapture::new().await.map_err(|e| e.to_string())?;
    // GNOME reports capability bits ashpd does not know and fails to decode
    // them; the portal answered, which is what matters then.
    match portal.supported_capabilities().await {
        Ok(caps) if !caps.contains(Capabilities::Keyboard | Capabilities::Pointer) => {
            Err("it cannot capture a keyboard and pointer".to_owned())
        }
        _ => Ok(()),
    }
}

/// One RemoteDesktop session. `Ok` means the daemon is gone.
async fn inject_session(
    env: &Env,
    rx: &mut mpsc::UnboundedReceiver<Inject>,
    tokens: &TokenFile,
    retry: &mut Retry,
) -> Result<()> {
    let conn = env.connect().await?;
    let portal = timeout("RemoteDesktop", RemoteDesktop::with_connection(conn))
        .await
        .context("RemoteDesktop portal not available")?;
    let session = timeout("creating a session", portal.create_session(Default::default())).await?;
    let result = guarded("input injection", run_inject(env, &portal, &session, rx, tokens, retry)).await;
    close(&session).await;
    result
}

async fn run_inject(
    env: &Env,
    portal: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
    rx: &mut mpsc::UnboundedReceiver<Inject>,
    tokens: &TokenFile,
    retry: &mut Retry,
) -> Result<()> {
    let mut closed = session.receive_closed().await?;
    let mut portal_owner = portal.receive_owner_changed().await?;
    let start = async |token: Option<String>, retry: &mut Retry| {
        let opts = SelectDevicesOptions::default()
            .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token.as_deref());
        timeout("selecting devices", portal.select_devices(session, opts)).await?.response()?;
        let request = retry.asking(portal.start(session, None, Default::default())).await?;
        request.response().context("permission to control this computer was not granted")
    };
    with_token(tokens, retry, start, |s| s.restore_token()).await?;
    let fd = timeout("connecting to libei", portal.connect_to_eis(session, Default::default())).await?;
    let context = ei::Context::new(UnixStream::from(fd))?;
    let handshake = context.handshake_tokio("one-mk", ei::handshake::ContextType::Sender);
    let (conn, mut ei_events) = timeout("libei handshake", handshake).await?;
    retry.ready();
    tracing::info!("input injection ready");

    let mut inj = Injector {
        context,
        conn,
        devices: Devices::default(),
        sequence: 0,
        last: None,
        warned_relative: false,
        held: Held::default(),
    };
    let mut batch = Vec::new();
    let mut power = env.power.clone();
    power.mark_unchanged();

    let result = loop {
        tokio::select! {
            ev = ei_events.next() => {
                let Some(ev) = ev else { break Err(anyhow!("input injection connection closed")) };
                let ev = match ev {
                    Ok(ev) => ev,
                    Err(e) => break Err(e.into()),
                };
                match ev {
                    EiEvent::SeatAdded(s) => {
                        s.seat.bind_capabilities(
                            DeviceCapability::Pointer | DeviceCapability::PointerAbsolute
                                | DeviceCapability::Keyboard | DeviceCapability::Scroll
                                | DeviceCapability::Button,
                        );
                    }
                    EiEvent::DeviceAdded(d) => {
                        inj.devices.all.push(d.device.clone());
                        inj.devices.update_regions();
                    }
                    EiEvent::DeviceResumed(d) => {
                        inj.sequence = inj.sequence.wrapping_add(1);
                        d.device.device().start_emulating(d.serial, inj.sequence);
                    }
                    EiEvent::DeviceRemoved(d) => {
                        inj.devices.all.retain(|x| *x != d.device);
                        inj.devices.dirty.retain(|x| *x != d.device);
                        inj.devices.update_regions();
                    }
                    EiEvent::Disconnected(d) => {
                        break Err(anyhow!("input injection disconnected: {:?}", d.explanation));
                    }
                    _ => {}
                }
                if let Err(e) = inj.context.flush() {
                    break Err(e.into());
                }
            }
            now = power_changed(&mut power) => {
                if now.paused() {
                    inj.let_go();
                }
            }
            _ = closed.next() => break Err(anyhow!("the remote desktop session was closed")),
            owner = portal_owner.next() => {
                let how = if owner.flatten().is_some() { "restarted" } else { "went away" };
                break Err(anyhow!("the desktop portal {how}"));
            }
            msg = rx.recv() => {
                let Some(first) = msg else { break Ok(()) };
                // Apply everything already queued (stale moves dropped),
                // then send one frame.
                drain_coalesced(first, rx, &mut batch);
                inj.send(batch.drain(..));
            }
        }
    };
    inj.let_go();
    result
}

fn apply(devices: &mut Devices, m: Inject, last: &mut Option<Pos>, warned_relative: &mut bool) {
    match m {
        Inject::MoveTo(p) => {
            let target = devices.regions.as_ref().map_or(p, |r| r.clamp(p));
            let abs = devices.with::<ei::PointerAbsolute>(|d| d.motion_absolute(target.x as f32, target.y as f32));
            if !abs {
                // Fallback when the compositor offers only relative motion.
                if !*warned_relative {
                    tracing::warn!("no absolute pointer device; using relative motion");
                    *warned_relative = true;
                }
                let (dx, dy) = match *last {
                    Some(l) => (p.x - l.x, p.y - l.y),
                    None => (0.0, 0.0),
                };
                devices.with::<ei::Pointer>(|d| d.motion_relative(dx as f32, dy as f32));
            }
            *last = Some(p);
        }
        Inject::MoveBy { dx, dy } => {
            devices.with::<ei::Pointer>(|d| d.motion_relative(dx as f32, dy as f32));
            *last = None; // where it ended up is the compositor's business
        }
        Inject::Button { button, down } => {
            let state = if down { ei::button::ButtonState::Press } else { ei::button::ButtonState::Released };
            devices.with::<ei::Button>(|d| d.button(button, state));
        }
        Inject::Scroll { dx, dy } => {
            devices.with::<ei::Scroll>(|d| d.scroll(dx as f32, dy as f32));
        }
        Inject::ScrollDiscrete { dx, dy } => {
            devices.with::<ei::Scroll>(|d| d.scroll_discrete(dx, dy));
        }
        Inject::ScrollStop => {
            devices.with::<ei::Scroll>(|d| d.scroll_stop(1, 1, 0));
        }
        Inject::Key { code, down } => {
            let state = if down { ei::keyboard::KeyState::Press } else { ei::keyboard::KeyState::Released };
            devices.with::<ei::Keyboard>(|d| d.key(code, state));
        }
        // We can't see local input, so there is nothing to gate.
        Inject::Controlled(_) => {}
    }
}

fn monotonic_us() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}

#[cfg(test)]
#[path = "portal_tests.rs"]
mod tests;
