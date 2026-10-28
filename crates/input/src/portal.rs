//! Linux backend built on XDG desktop portals and libei, so it works on
//! Wayland (GNOME, KDE) without root, udev rules or the `input` group.
//!
//! - Capture: `org.freedesktop.portal.InputCapture` with pointer barriers on
//!   the edges that lead to peers. Only this side reports `Event::Desktop`.
//! - Injection: `org.freedesktop.portal.RemoteDesktop`. The permission is
//!   remembered with a restore token, so the user is asked only once.
//! - Both portal sessions can be lost (sleep/wake, a dock replug, the portal
//!   or compositor restarting). Each side then ends any capture in progress
//!   and re-creates its session with backoff (1 s, 2 s, 5 s ... 30 s). Only
//!   the user refusing permission, or the portal not existing at all, stops
//!   it for good.
//!
//! `Event::LocalInput` is never sent by this backend: the portals give us
//! no way to see real input on this machine while a peer controls it.
//! InputCapture only delivers input after a barrier activation, and libei
//! senders (RemoteDesktop) never see input at all. Faking it (e.g. from
//! cursor position changes we didn't cause) would misfire, so we don't.
//! Instead the person here takes control back with `one-mk takeback`, and
//! the daemon lets a controller that sent no real input for a minute go.

use crate::common_logic::{Backoff, drain_coalesced};
use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::{Context, Result, bail};
use ashpd::desktop::input_capture::{
    self, Barrier, Capabilities, InputCapture, ReleaseOptions, SetPointerBarriersOptions,
};
use ashpd::desktop::remote_desktop::{self, DeviceType, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, ResponseError, Session};
use futures::StreamExt;
use one_mk_proto::{Desktop, Mods, Pos, Rect, Side};
use reis::ei;
use reis::event::{DeviceCapability, EiEvent};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// A session that lasted at least this long starts the backoff over.
const STABLE: Duration = Duration::from_secs(60);

pub(crate) async fn open(state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();

    let (focus, locker) = crate::system::native(&ev_tx);
    let (tx, warp) = (ev_tx.clone(), inj_tx.clone());
    spawn_thread("one-mk-capture", move || capture(tx, cap_rx, warp))?;
    spawn_thread("one-mk-inject", move || inject(ev_tx, inj_rx, state_dir))?;
    Ok(Backend { events: ev_rx, capture: cap_tx, inject: inj_tx, focus, locker })
}

/// libei streams are not `Send`, and input should not queue behind network
/// work anyway, so capture and injection each get their own thread. The
/// future is built on that thread, so it never needs to be `Send`.
fn spawn_thread<F, Fut>(name: &str, make: F) -> Result<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    std::thread::Builder::new().name(name.into()).spawn(move || rt.block_on(make()))?;
    Ok(())
}

/// Errors that retrying won't fix: the user said no, or the portal doesn't
/// exist on this desktop.
fn is_permanent(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        matches!(
            c.downcast_ref::<ashpd::Error>(),
            Some(ashpd::Error::Response(ResponseError::Cancelled) | ashpd::Error::PortalNotFound(_))
        )
    })
}

/// What a session loop does after its session ended with `result`.
enum Next {
    /// The daemon is gone, or the failure is permanent.
    Stop,
    Retry(Duration),
}

/// Shared retry policy of both sides. `what` describes the feature for the
/// user, e.g. "Can't share this computer's mouse with others".
struct Retry {
    what: &'static str,
    backoff: Backoff,
    /// A session was fully set up at least once.
    ever_ready: bool,
    /// The user has been told about the current run of failures.
    reported: bool,
}

impl Retry {
    fn new(what: &'static str) -> Self {
        Self { what, backoff: Backoff::default(), ever_ready: false, reported: false }
    }

    fn ready(&mut self) {
        self.ever_ready = true;
        self.reported = false;
    }

    fn after(&mut self, result: Result<()>, ran_for: Duration, tx: &mpsc::UnboundedSender<Event>) -> Next {
        let e = match result {
            Ok(()) => return Next::Stop,
            Err(e) => e,
        };
        if is_permanent(&e) {
            tracing::warn!("{}: {e:#}", self.what);
            let _ = tx.send(Event::Error(format!("{}: {e:#}", self.what)));
            return Next::Stop;
        }
        if ran_for >= STABLE {
            self.backoff.reset();
        }
        let delay = self.backoff.next_delay();
        tracing::warn!("{}: {e:#}; trying again in {delay:?}", self.what);
        // A session that was working and just got lost (sleep/wake) is
        // routine; only tell the user once getting it back fails too.
        if !self.reported && (!self.ever_ready || ran_for < STABLE) {
            self.reported = true;
            let _ = tx.send(Event::Error(format!("{}: {e:#}. Retrying in the background.", self.what)));
        }
        Next::Retry(delay)
    }
}

/// Closes a session we are done with; this also ends any capture or
/// emulation the compositor still holds for it. Best effort: the session
/// may be gone already.
async fn close<T: ashpd::desktop::SessionPortal>(session: &Session<T>) {
    let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
}

// ---------------------------------------------------------------- capture

/// Capture state that outlives a portal session.
struct CaptureState {
    tx: mpsc::UnboundedSender<Event>,
    sides: Vec<Side>,
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
        let zones = self
            .portal
            .zones(self.session, input_capture::GetZonesOptions::default())
            .await?
            .response()?;
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

    async fn apply_barriers(&mut self, sides: &[Side]) -> Result<()> {
        let mut barriers = Vec::new();
        self.barriers.clear();
        for &side in sides {
            for r in self.desktop.outer_edges(side) {
                let id = side as u32 * 64 + barriers.len() as u32 + 1;
                let (x, y, right, bottom) = (r.x, r.y, r.right(), r.bottom());
                // Barriers lie on the zone edge; right/bottom edges are one
                // past the last pixel, as the portal spec requires.
                let pos = match side {
                    Side::Left => (x, y, x, bottom - 1),
                    Side::Right => (right, y, right, bottom - 1),
                    Side::Top => (x, y, right - 1, y),
                    Side::Bottom => (x, bottom, right - 1, bottom),
                };
                barriers.push(Barrier::new(NonZeroU32::new(id).expect("nonzero"), pos));
                self.barriers.insert(id, side);
            }
        }
        let resp = self
            .portal
            .set_pointer_barriers(self.session, &barriers, self.zone_set, SetPointerBarriersOptions::default())
            .await?
            .response()?;
        for failed in resp.failed_barriers() {
            tracing::warn!(barrier = failed.get(), "compositor rejected a pointer barrier");
            self.barriers.remove(&failed.get());
        }

        let want = !self.barriers.is_empty();
        if want && !self.enabled {
            self.portal.enable(self.session, input_capture::EnableOptions::default()).await?;
        } else if !want && self.enabled {
            self.portal.disable(self.session, input_capture::DisableOptions::default()).await?;
        }
        self.enabled = want;
        Ok(())
    }

    /// Re-reads the zones and re-applies the barriers. Failures are logged
    /// and otherwise ignored: the old layout stays in use, and a session
    /// that is really gone shows up as a closed libei connection.
    async fn refresh(&mut self, sides: &[Side]) {
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
    async fn release(&mut self, portal_id: Option<u32>, pos: Option<Pos>, sides: &[Side]) -> Result<()> {
        let opts = ReleaseOptions::default()
            .set_activation_id(portal_id)
            .set_cursor_position(pos.map(|p| (p.x, p.y)));
        let Err(e) = self.portal.release(self.session, opts).await else { return Ok(()) };
        tracing::warn!("releasing the pointer: {e:#}; disabling capture instead");
        self.portal
            .disable(self.session, input_capture::DisableOptions::default())
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
    tx: mpsc::UnboundedSender<Event>,
    mut cmds: mpsc::UnboundedReceiver<CaptureCmd>,
    warp: mpsc::UnboundedSender<Inject>,
) {
    let mut st = CaptureState { tx, sides: Vec::new(), reported: None, next_id: 0, activation: None, warp };
    let mut retry = Retry::new("Can't share this computer's mouse with others");
    loop {
        let started = Instant::now();
        let result = capture_session(&mut st, &mut cmds, &mut retry).await;
        // However the session ended, so did any capture in progress.
        st.deactivated();
        match retry.after(result, started.elapsed(), &st.tx) {
            Next::Stop => return,
            Next::Retry(delay) => {
                // Keep following the daemon while waiting.
                let sleep = tokio::time::sleep(delay);
                tokio::pin!(sleep);
                loop {
                    tokio::select! {
                        _ = &mut sleep => break,
                        cmd = cmds.recv() => match cmd {
                            Some(CaptureCmd::SetSides(sides)) => st.sides = sides,
                            Some(CaptureCmd::Release { .. }) => {}
                            Some(CaptureCmd::Activate { .. }) => {
                                let _ = st.tx.send(Event::Error(NO_CAPTURE.into()));
                            }
                            None => return,
                        },
                    }
                }
            }
        }
    }
}

/// One InputCapture session. `Ok` means the daemon is gone.
async fn capture_session(
    st: &mut CaptureState,
    cmds: &mut mpsc::UnboundedReceiver<CaptureCmd>,
    retry: &mut Retry,
) -> Result<()> {
    let portal = InputCapture::new().await.context("InputCapture portal not available")?;
    let (session, _caps) = portal
        .create_session(
            None,
            input_capture::CreateSessionOptions::default()
                .set_capabilities(Capabilities::Keyboard | Capabilities::Pointer),
        )
        .await
        .context("permission to capture input was not granted")?;
    let result = run_capture(&portal, &session, st, cmds, retry).await;
    close(&session).await;
    result
}

async fn run_capture(
    portal: &InputCapture,
    session: &Session<InputCapture>,
    st: &mut CaptureState,
    cmds: &mut mpsc::UnboundedReceiver<CaptureCmd>,
    retry: &mut Retry,
) -> Result<()> {
    let mut closed = session.receive_closed().await?;
    let fd = portal.connect_to_eis(session, input_capture::ConnectToEISOptions::default()).await?;
    let context = ei::Context::new(UnixStream::from(fd))?;
    let (_conn, mut ei_events) = context
        .handshake_tokio("one-mk", ei::handshake::ContextType::Receiver)
        .await?;

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
                    if st.activation.is_none() {
                        push_into_barrier(&cap.desktop, st, side);
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
                    (Some(side), Some(pos)) => {
                        st.next_id = st.next_id.wrapping_add(1);
                        st.activation = Some((st.next_id, a.activation_id()));
                        let _ = st.tx.send(Event::Activated { id: st.next_id, side, pos });
                    }
                    // Not one of our barriers; hand the pointer straight back.
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
            _ = closed.next() => bail!("the input capture session was closed"),
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
    let Some(r) = desktop.outer_edges(side).next().filter(|_| st.sides.contains(&side)) else {
        let _ = st.tx.send(Event::Error(NO_CAPTURE.into()));
        return;
    };
    let c = r.center();
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

async fn inject(tx: mpsc::UnboundedSender<Event>, mut rx: mpsc::UnboundedReceiver<Inject>, state_dir: PathBuf) {
    let mut retry = Retry::new("Other computers can't control this one");
    loop {
        let started = Instant::now();
        let result = inject_session(&mut rx, &state_dir, &mut retry).await;
        match retry.after(result, started.elapsed(), &tx) {
            Next::Stop => return,
            Next::Retry(delay) => {
                // Nothing can be injected meanwhile; drop it. Keys held by
                // the old session were released by the compositor with it.
                let sleep = tokio::time::sleep(delay);
                tokio::pin!(sleep);
                loop {
                    tokio::select! {
                        _ = &mut sleep => break,
                        m = rx.recv() => if m.is_none() { return },
                    }
                }
            }
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
async fn inject_session(rx: &mut mpsc::UnboundedReceiver<Inject>, state_dir: &Path, retry: &mut Retry) -> Result<()> {
    let portal = RemoteDesktop::new().await.context("RemoteDesktop portal not available")?;
    let session = portal.create_session(Default::default()).await?;
    let result = run_inject(&portal, &session, rx, state_dir, retry).await;
    close(&session).await;
    result
}

async fn run_inject(
    portal: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
    rx: &mut mpsc::UnboundedReceiver<Inject>,
    state_dir: &Path,
    retry: &mut Retry,
) -> Result<()> {
    let mut closed = session.receive_closed().await?;
    let token_path = state_dir.join("remote-desktop.token");
    let token = std::fs::read_to_string(&token_path).ok();
    portal
        .select_devices(
            session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await?
        .response()?;
    let selected = portal
        .start(session, None, remote_desktop::StartOptions::default())
        .await?
        .response()
        .context("permission to control this computer was not granted")?;
    if let Some(t) = selected.restore_token() {
        std::fs::create_dir_all(state_dir)?;
        std::fs::write(&token_path, t)?;
    }
    let fd = portal.connect_to_eis(session, remote_desktop::ConnectToEISOptions::default()).await?;
    let context = ei::Context::new(UnixStream::from(fd))?;
    let (conn, mut ei_events) = context
        .handshake_tokio("one-mk", ei::handshake::ContextType::Sender)
        .await?;
    retry.ready();
    tracing::info!("input injection ready");

    let mut devices = Devices::default();
    let mut sequence = 0u32;
    let mut last: Option<Pos> = None;
    let mut warned_relative = false;
    let mut batch = Vec::new();

    loop {
        tokio::select! {
            ev = ei_events.next() => {
                let Some(ev) = ev else { bail!("input injection connection closed") };
                match ev? {
                    EiEvent::SeatAdded(s) => {
                        s.seat.bind_capabilities(
                            DeviceCapability::Pointer | DeviceCapability::PointerAbsolute
                                | DeviceCapability::Keyboard | DeviceCapability::Scroll
                                | DeviceCapability::Button,
                        );
                    }
                    EiEvent::DeviceAdded(d) => {
                        devices.all.push(d.device.clone());
                        devices.update_regions();
                    }
                    EiEvent::DeviceResumed(d) => {
                        sequence = sequence.wrapping_add(1);
                        d.device.device().start_emulating(d.serial, sequence);
                    }
                    EiEvent::DeviceRemoved(d) => {
                        devices.all.retain(|x| *x != d.device);
                        devices.dirty.retain(|x| *x != d.device);
                        devices.update_regions();
                    }
                    EiEvent::Disconnected(d) => bail!("input injection disconnected: {:?}", d.explanation),
                    _ => {}
                }
                context.flush()?;
            }
            _ = closed.next() => bail!("the remote desktop session was closed"),
            msg = rx.recv() => {
                let Some(first) = msg else { return Ok(()) };
                // Apply everything already queued (stale moves dropped),
                // then send one frame.
                drain_coalesced(first, rx, &mut batch);
                for m in batch.drain(..) {
                    apply(&mut devices, m, &mut last, &mut warned_relative);
                }
                let serial = conn.serial();
                let now = monotonic_us();
                for dev in devices.dirty.drain(..) {
                    dev.device().frame(serial, now);
                }
                if let Err(e) = context.flush() {
                    tracing::warn!("injecting input: {e}");
                }
            }
        }
    }
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
