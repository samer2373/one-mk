//! Linux backend built on XDG desktop portals and libei, so it works on
//! Wayland (GNOME, KDE) without root, udev rules or the `input` group.
//!
//! - Capture: `org.freedesktop.portal.InputCapture` with pointer barriers on
//!   the edges that lead to peers.
//! - Injection: `org.freedesktop.portal.RemoteDesktop`. The permission is
//!   remembered with a restore token, so the user is asked only once.

use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::{Context, Result, bail};
use ashpd::desktop::input_capture::{
    self, Barrier, Capabilities, InputCapture, ReleaseOptions, SetPointerBarriersOptions,
};
use ashpd::desktop::remote_desktop::{self, DeviceType, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, Session};
use futures::StreamExt;
use one_mk_proto::{Desktop, Pos, Rect, Side};
use reis::ei;
use reis::event::{DeviceCapability, EiEvent};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub(crate) async fn open(state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();

    let tx = ev_tx.clone();
    spawn_thread("one-mk-capture", move || async move {
        if let Err(e) = capture(tx.clone(), cap_rx).await {
            tracing::warn!("input capture unavailable: {e:#}");
            let _ = tx.send(Event::Error(format!("Can't share this computer's mouse with others: {e:#}")));
        }
    })?;
    spawn_thread("one-mk-inject", move || async move {
        if let Err(e) = inject(ev_tx.clone(), inj_rx, state_dir).await {
            tracing::warn!("input injection unavailable: {e:#}");
            let _ = ev_tx.send(Event::Error(format!("Other computers can't control this one: {e:#}")));
        }
    })?;
    Ok(Backend { events: ev_rx, capture: cap_tx, inject: inj_tx })
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

// ---------------------------------------------------------------- capture

struct Capture<'a> {
    portal: &'a InputCapture,
    session: Session<InputCapture>,
    desktop: Desktop,
    zone_set: u32,
    sides: Vec<Side>,
    barriers: HashMap<u32, Side>,
    enabled: bool,
}

impl Capture<'_> {
    async fn refresh_zones(&mut self) -> Result<()> {
        let zones = self
            .portal
            .zones(&self.session, input_capture::GetZonesOptions::default())
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

    async fn apply_barriers(&mut self) -> Result<()> {
        let mut barriers = Vec::new();
        self.barriers.clear();
        for &side in &self.sides {
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
            .set_pointer_barriers(&self.session, &barriers, self.zone_set, SetPointerBarriersOptions::default())
            .await?
            .response()?;
        for failed in resp.failed_barriers() {
            tracing::warn!(barrier = failed.get(), "compositor rejected a pointer barrier");
            self.barriers.remove(&failed.get());
        }

        let want = !self.barriers.is_empty();
        if want && !self.enabled {
            self.portal.enable(&self.session, input_capture::EnableOptions::default()).await?;
        } else if !want && self.enabled {
            self.portal.disable(&self.session, input_capture::DisableOptions::default()).await?;
        }
        self.enabled = want;
        Ok(())
    }
}

async fn capture(tx: mpsc::UnboundedSender<Event>, mut cmds: mpsc::UnboundedReceiver<CaptureCmd>) -> Result<()> {
    let portal = InputCapture::new().await.context("InputCapture portal not available")?;
    let (session, _caps) = portal
        .create_session(
            None,
            input_capture::CreateSessionOptions::default()
                .set_capabilities(Capabilities::Keyboard | Capabilities::Pointer),
        )
        .await
        .context("permission to capture input was not granted")?;
    let fd = portal.connect_to_eis(&session, input_capture::ConnectToEISOptions::default()).await?;
    let context = ei::Context::new(UnixStream::from(fd))?;
    let (_conn, mut ei_events) = context
        .handshake_tokio("one-mk", ei::handshake::ContextType::Receiver)
        .await?;

    let mut activated = portal.receive_activated().await?;
    let mut deactivated = portal.receive_deactivated().await?;
    let mut zones_changed = portal.receive_zones_changed().await?;
    let mut disabled = portal.receive_disabled().await?;

    let mut cap = Capture {
        portal: &portal,
        session,
        desktop: Desktop::new(vec![Rect::new(0, 0, 1, 1)]),
        zone_set: 0,
        sides: Vec::new(),
        barriers: HashMap::new(),
        enabled: false,
    };
    cap.refresh_zones().await?;
    let _ = tx.send(Event::Desktop(cap.desktop.clone()));
    tracing::info!(screens = ?cap.desktop.screens, "input capture ready");

    // Our activation id, and the portal's id for the same activation.
    let mut next_id = 0u32;
    let mut activation: Option<(u32, Option<u32>)> = None;
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(CaptureCmd::SetSides(sides)) => {
                    cap.sides = sides;
                    cap.apply_barriers().await?;
                }
                Some(CaptureCmd::Release { id, pos }) => {
                    if activation.is_some_and(|(ours, _)| ours == id) {
                        let portal_id = activation.take().and_then(|(_, p)| p);
                        let opts = ReleaseOptions::default()
                            .set_activation_id(portal_id)
                            .set_cursor_position(pos.map(|p| (p.x, p.y)));
                        cap.portal.release(&cap.session, opts).await?;
                    }
                }
                None => return Ok(()),
            },
            Some(a) = activated.next() => {
                next_id = next_id.wrapping_add(1);
                activation = Some((next_id, a.activation_id()));
                let side = match a.barrier_id() {
                    Some(input_capture::ActivatedBarrier::Barrier(id)) => cap.barriers.get(&id.get()).copied(),
                    _ => None,
                };
                let pos = a.cursor_position().map(|(x, y)| Pos::new(f64::from(x), f64::from(y)));
                match (side, pos) {
                    (Some(side), Some(pos)) => { let _ = tx.send(Event::Activated { id: next_id, side, pos }); }
                    _ => {
                        // Not one of our barriers; hand the pointer straight back.
                        let portal_id = activation.take().and_then(|(_, p)| p);
                        let opts = ReleaseOptions::default().set_activation_id(portal_id);
                        cap.portal.release(&cap.session, opts).await?;
                    }
                }
            },
            Some(d) = deactivated.next() => {
                // Only end the activation this signal is about; a late signal
                // for an earlier one must not end a newer capture.
                if let Some((ours, portal_id)) = activation
                    && (d.activation_id().is_none() || d.activation_id() == portal_id)
                {
                    activation = None;
                    let _ = tx.send(Event::Deactivated { id: ours });
                }
            },
            Some(_) = zones_changed.next() => {
                cap.refresh_zones().await?;
                cap.apply_barriers().await?;
                let _ = tx.send(Event::Desktop(cap.desktop.clone()));
            },
            Some(_) = disabled.next() => {
                // The compositor dropped our barriers (e.g. monitors changed); set them up again.
                if let Some((ours, _)) = activation.take() {
                    let _ = tx.send(Event::Deactivated { id: ours });
                }
                cap.enabled = false;
                cap.refresh_zones().await?;
                cap.apply_barriers().await?;
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
                    EiEvent::Disconnected(d) => bail!("input capture disconnected: {:?}", d.explanation),
                    _ => continue,
                };
                if tx.send(ev).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

// --------------------------------------------------------------- injection

#[derive(Default)]
struct Devices {
    all: Vec<reis::event::Device>,
    /// Devices we have sent events to since the last frame.
    dirty: Vec<reis::event::Device>,
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

    fn absolute_regions(&self) -> Option<Vec<Rect>> {
        let dev = self.all.iter().find(|d| d.interface::<ei::PointerAbsolute>().is_some())?;
        let rects: Vec<Rect> = dev
            .regions()
            .iter()
            .map(|r| Rect::new(r.x as i32, r.y as i32, r.width as i32, r.height as i32))
            .collect();
        (!rects.is_empty()).then_some(rects)
    }
}

async fn inject(tx: mpsc::UnboundedSender<Event>, mut rx: mpsc::UnboundedReceiver<Inject>, state_dir: PathBuf) -> Result<()> {
    let portal = RemoteDesktop::new().await.context("RemoteDesktop portal not available")?;
    let session = portal.create_session(Default::default()).await?;
    let token_path = state_dir.join("remote-desktop.token");
    let token = std::fs::read_to_string(&token_path).ok();
    portal
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await?
        .response()?;
    let selected = portal
        .start(&session, None, remote_desktop::StartOptions::default())
        .await?
        .response()
        .context("permission to control this computer was not granted")?;
    if let Some(t) = selected.restore_token() {
        std::fs::create_dir_all(&state_dir)?;
        std::fs::write(&token_path, t)?;
    }
    let fd = portal.connect_to_eis(&session, remote_desktop::ConnectToEISOptions::default()).await?;
    let context = ei::Context::new(UnixStream::from(fd))?;
    let (conn, mut ei_events) = context
        .handshake_tokio("one-mk", ei::handshake::ContextType::Sender)
        .await?;
    tracing::info!("input injection ready");

    let mut devices = Devices::default();
    let mut sequence = 0u32;
    let mut last: Option<Pos> = None;
    let mut warned_relative = false;

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
                        if let Some(screens) = devices.absolute_regions() {
                            let _ = tx.send(Event::Desktop(Desktop::new(screens)));
                        }
                    }
                    EiEvent::DeviceResumed(d) => {
                        sequence = sequence.wrapping_add(1);
                        d.device.device().start_emulating(d.serial, sequence);
                    }
                    EiEvent::DeviceRemoved(d) => devices.all.retain(|x| *x != d.device),
                    EiEvent::Disconnected(d) => bail!("input injection disconnected: {:?}", d.explanation),
                    _ => {}
                }
                context.flush()?;
            }
            msg = rx.recv() => {
                let Some(first) = msg else { return Ok(()) };
                // Apply everything already queued, then send one frame.
                let mut next = Some(first);
                while let Some(m) = next {
                    apply(&mut devices, m, &mut last, &mut warned_relative);
                    next = rx.try_recv().ok();
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
            let abs = devices.with::<ei::PointerAbsolute>(|d| d.motion_absolute(p.x as f32, p.y as f32));
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
    }
}

fn monotonic_us() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}
