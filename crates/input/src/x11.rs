//! X11 backend: for X11 sessions (Xfce, MATE, Cinnamon, i3, Openbox, KDE
//! or GNOME on X11) where the Wayland portals don't apply.
//!
//! - Edges: an XFixes pointer barrier on each edge that leads to a peer
//!   (XFixes 5, XInput 2.3: X.Org 1.14 and later). The X server reports
//!   the pointer pushing against one as `XI_BarrierHit`; nothing runs
//!   while the pointer is elsewhere. Without barriers, an invisible
//!   one-pixel input-only window along the edge reports the pointer
//!   entering it instead.
//! - Capture: `XGrabPointer` (confined to a 1x1 input-only window, with a
//!   blank cursor) and `XGrabKeyboard`, so no application sees the input;
//!   the input itself comes from XInput2 raw events, which X delivers
//!   regardless of grabs. Raw events are selected only while capturing
//!   (and, to notice local input, while controlled), so an idle pointer
//!   costs nothing.
//! - Injection: XTest. Key codes are evdev + 8, the X keycodes of the
//!   evdev and libinput drivers, so the layout set on this machine
//!   applies. XTest has no smooth scrolling: scrolling becomes wheel
//!   clicks (buttons 4-7), with remainders carried over.
//! - Monitors: RandR 1.5 monitors, in X pixels. X11 has no logical
//!   pixels: the scale is 1, and a HiDPI setup (`Xft.dpi`, `GDK_SCALE`)
//!   only makes applications draw larger, so the desktop is reported in
//!   physical pixels, as the pointer moves in them.
//!
//! Nothing may stay stuck. A grab belongs to the X connection, so it ends
//! with the process however that dies (even `kill -9`); while running,
//! capture ends (with `Event::Deactivated`) when the daemon goes away,
//! the screen locks, the monitors change or the connection breaks.
//! XTest key presses, on the other hand, outlive the client that made
//! them, so injected keys and buttons still held are let go of when the
//! daemon exits or the screen locks, after a lost connection on the next
//! one, and after a crash (whatever the XTest devices hold) at the next
//! start.

use crate::common_logic::{Backoff, LocalInputGate, drain_coalesced, now_ms};
use crate::linux_logic::{
    Held, Notches, Pressed, ScrollAxis, XButton, absolute_delta, barrier_line, barrier_open_directions, edge_strip,
    evdev_to_x_button, evdev_to_xkeycode, fp1616, fp3232, notch_buttons, set_bits, x_button, xkeycode_to_evdev,
};
use crate::linux_rt::{Socket, forward_lock, lock_changed, spawn_thread};
use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::{Context, Result, bail};
use one_mk_proto::{Desktop, Mods, Pos, Rect, Side};
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

const NO_CAPTURE: &str = "Can't move the pointer to another computer right now: another program holds the mouse or keyboard.";

pub(crate) async fn open(_state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (sys_tx, sys_rx) = mpsc::unbounded_channel();
    let (focus, locker) = crate::system::native(&sys_tx);
    drop(sys_tx);
    let (locked_tx, locked) = watch::channel(false);
    tokio::spawn(forward_lock(sys_rx, ev_tx.clone(), locked_tx));
    // Fail at once, and clearly, without an X server.
    let first = X::connect(&Target::default()).context("can't use the X server (is DISPLAY right?)")?;
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    spawn_thread("one-mk-x11", move || run(Some(first), Target::default(), ev_tx, cap_rx, inj_rx, locked))?;
    Ok(Backend { events: ev_rx, capture: cap_tx, inject: inj_tx, focus, locker })
}

/// Which X server, and (for tests) whether pointer barriers may be used.
#[derive(Clone, Debug)]
struct Target {
    display: Option<String>,
    barriers: bool,
}

impl Default for Target {
    fn default() -> Self {
        Self { display: None, barriers: true }
    }
}

/// State that outlives one X connection.
struct Shared {
    tx: mpsc::UnboundedSender<Event>,
    sides: Vec<Side>,
    next_id: u32,
    /// Injected keys and buttons held down.
    held: Held,
    notches: Notches,
    controlled: bool,
    gate: LocalInputGate,
    locked: bool,
    reported: Option<Desktop>,
    shown: Option<String>,
}

impl Shared {
    fn send(&self, ev: Event) {
        let _ = self.tx.send(ev);
    }

    fn report_desktop(&mut self, d: &Desktop) {
        if self.reported.as_ref() != Some(d) {
            self.reported = Some(d.clone());
            self.send(Event::Desktop(d.clone()));
        }
    }

    fn show(&mut self, problem: Option<String>) {
        if self.shown != problem {
            self.shown.clone_from(&problem);
            for part in ["capture", "control"] {
                self.send(Event::Status { part, problem: problem.clone() });
            }
        }
    }
}

/// Runs connections to the X server until the daemon goes away,
/// reconnecting with backoff when one breaks.
async fn run(
    mut first: Option<X>,
    target: Target,
    tx: mpsc::UnboundedSender<Event>,
    mut cmds: mpsc::UnboundedReceiver<CaptureCmd>,
    mut inj: mpsc::UnboundedReceiver<Inject>,
    mut locked: watch::Receiver<bool>,
) {
    let mut st = Shared {
        tx,
        sides: Vec::new(),
        next_id: 0,
        held: Held::default(),
        notches: Notches::default(),
        controlled: false,
        gate: LocalInputGate::default(),
        locked: *locked.borrow_and_update(),
        reported: None,
        shown: None,
    };
    let mut backoff = Backoff::default();
    loop {
        let started = Instant::now();
        let result = match first.take().map_or_else(|| X::connect(&target), Ok) {
            Ok(mut x) => {
                let r = x.session(&mut st, &mut cmds, &mut inj, &mut locked).await;
                x.close(&mut st);
                r
            }
            Err(e) => Err(e),
        };
        let e = match result {
            Ok(()) => return,
            Err(e) => e,
        };
        if started.elapsed() >= Duration::from_secs(60) {
            backoff.reset();
        }
        let delay = backoff.next_delay();
        tracing::warn!("X11 input: {e:#}; trying again in {delay:?}");
        st.show(Some(format!("The X server connection failed ({e:#}). Retrying in the background.")));
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                cmd = cmds.recv() => match cmd {
                    Some(CaptureCmd::SetSides(s)) => st.sides = s.iter().map(|a| a.side).collect(),
                    Some(CaptureCmd::Activate { .. }) => st.send(Event::Error(NO_CAPTURE.into())),
                    Some(CaptureCmd::Release { .. }) => {}
                    None => return,
                },
                m = inj.recv() => match m {
                    // Held keys are let go of on the next connection.
                    Some(m) => st.held.update(&m),
                    None => return,
                },
                l = lock_changed(&mut locked) => st.locked = l,
            }
        }
    }
}

/// One valuator of a device: which number, and for absolute devices
/// (tablets, VM pointers) the range and the last value seen.
#[derive(Debug)]
struct Axis {
    number: u16,
    absolute: bool,
    min: f64,
    max: f64,
    last: Option<f64>,
}

/// What raw events from one device mean.
#[derive(Debug, Default)]
struct Axes {
    x: Option<Axis>,
    y: Option<Axis>,
    /// Scroll valuators: number, vertical, increment per notch.
    scroll: Vec<(u16, bool, f64, ScrollAxis)>,
}

struct Grab {
    id: u32,
    window: xproto::Window,
}

enum Edges {
    /// XFixes barriers.
    Barriers(Vec<(xfixes::Barrier, Side)>),
    /// Input-only windows; `ignore` is one the daemon put the pointer
    /// into (on release), which must not capture again until it leaves.
    Windows { windows: Vec<(xproto::Window, Side)>, ignore: Option<xproto::Window> },
}

struct X {
    conn: RustConnection,
    root: xproto::Window,
    /// XFixes 5 and XInput 2.3.
    barriers: bool,
    randr: bool,
    xtest_devices: Vec<u16>,
    blank: xproto::Cursor,
    desktop: Desktop,
    edges: Edges,
    grab: Option<Grab>,
    /// The XInput event mask selected on the root window.
    selected: u32,
    devices: HashMap<u16, Axes>,
    pressed: Pressed,
    /// A barrier hit sequence whose capture failed: not tried again until
    /// the pointer leaves the barrier and comes back.
    failed_hit: Option<u32>,
}

const RAW_CAPTURE: u32 = (1 << 13) | (1 << 14) | (1 << 15) | (1 << 16) | (1 << 17);
const RAW_LOCAL: u32 = (1 << 13) | (1 << 15) | (1 << 17);
const BARRIER_HIT: u32 = 1 << 25;
const BARRIER_LEAVE: u32 = 1 << 26;
const ALL_MASTER_DEVICES: u16 = 1;

impl X {
    fn connect(target: &Target) -> Result<X> {
        let (conn, screen) = x11rb::connect(target.display.as_deref())?;
        let root = conn.setup().roots.get(screen).context("no such X screen")?.root;
        // All version requests go out before waiting for the first answer.
        let xtest = conn.xtest_get_version(2, 2);
        let xi = conn.xinput_xi_query_version(2, 3);
        let fixes = conn.xfixes_query_version(5, 0);
        let rr = conn.randr_query_version(1, 5);
        xtest.ok().and_then(|c| c.reply().ok()).context("the X server has no XTest extension")?;
        let xi = xi.ok().and_then(|c| c.reply().ok()).context("the X server has no XInput 2 extension")?;
        if (xi.major_version, xi.minor_version) < (2, 1) {
            bail!("the X server's XInput is {}.{}; one-mk needs 2.1", xi.major_version, xi.minor_version);
        }
        let fixes = fixes.ok().and_then(|c| c.reply().ok());
        let rr = rr.ok().and_then(|c| c.reply().ok());
        let barriers = target.barriers
            && (xi.major_version, xi.minor_version) >= (2, 3)
            && fixes.is_some_and(|f| f.major_version >= 5);
        let randr = rr.is_some_and(|r| (r.major_version, r.minor_version) >= (1, 5));

        let xtest_devices = conn
            .xinput_xi_query_device(0u16)?
            .reply()?
            .infos
            .iter()
            .filter(|d| d.name.windows(5).any(|w| w == b"XTEST"))
            .map(|d| d.deviceid)
            .collect();

        // An invisible cursor for while the pointer is away.
        let pixmap = conn.generate_id()?;
        let blank = conn.generate_id()?;
        conn.create_pixmap(1, pixmap, root, 1, 1)?;
        conn.create_cursor(blank, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)?;
        conn.free_pixmap(pixmap)?;
        if randr {
            conn.randr_select_input(root, randr::NotifyMask::SCREEN_CHANGE | randr::NotifyMask::CRTC_CHANGE)?;
        }
        let mut x = X {
            conn,
            root,
            barriers,
            randr,
            xtest_devices,
            blank,
            desktop: Desktop::new(vec![Rect::new(0, 0, 1, 1)]),
            edges: if barriers { Edges::Barriers(Vec::new()) } else { Edges::Windows { windows: Vec::new(), ignore: None } },
            grab: None,
            selected: u32::MAX,
            devices: HashMap::new(),
            pressed: Pressed::default(),
            failed_hit: None,
        };
        x.desktop = x.read_desktop()?;
        x.select(false);
        x.conn.flush()?;
        tracing::info!(barriers, randr, screens = ?x.desktop.screens, "X11 input ready");
        Ok(x)
    }

    fn read_desktop(&self) -> Result<Desktop> {
        if self.randr {
            let monitors = self.conn.randr_get_monitors(self.root, true)?.reply()?.monitors;
            let rects: Vec<Rect> = monitors
                .iter()
                .filter(|m| m.width > 0 && m.height > 0)
                .map(|m| Rect::new(m.x.into(), m.y.into(), m.width.into(), m.height.into()))
                .collect();
            if !rects.is_empty() {
                return Ok(Desktop::new(rects));
            }
        }
        let g = self.conn.get_geometry(self.root)?.reply()?;
        Ok(Desktop::new(vec![Rect::new(0, 0, g.width.into(), g.height.into())]))
    }

    /// Selects the XInput events needed now on the root window.
    fn select(&mut self, controlled: bool) {
        let mut mask = if self.barriers { BARRIER_HIT | BARRIER_LEAVE } else { 0 };
        if self.grab.is_some() {
            mask |= RAW_CAPTURE;
        } else if controlled {
            mask |= RAW_LOCAL;
        }
        if mask == self.selected {
            return;
        }
        self.selected = mask;
        let m = xinput::EventMask { deviceid: ALL_MASTER_DEVICES, mask: vec![mask.into()] };
        if let Err(e) = self.conn.xinput_xi_select_events(self.root, &[m]) {
            tracing::debug!("selecting XInput events: {e}");
        }
    }

    /// Puts barriers (or edge windows) on the outer edges of `sides`.
    fn set_edges(&mut self, sides: &[Side]) -> Result<()> {
        let c = &self.conn;
        match &mut self.edges {
            Edges::Barriers(old) => {
                for (b, _) in old.drain(..) {
                    c.xfixes_delete_pointer_barrier(b)?;
                }
                for &side in sides {
                    for r in self.desktop.outer_edges(side) {
                        let (x1, y1, x2, y2) = barrier_line(side, r);
                        let id = c.generate_id()?;
                        let dirs = barrier_open_directions(side).into();
                        let xy = |v: i32| v.clamp(0, i32::from(u16::MAX)) as u16;
                        c.xfixes_create_pointer_barrier(id, self.root, xy(x1), xy(y1), xy(x2), xy(y2), dirs, &[])?;
                        old.push((id, side));
                    }
                }
            }
            Edges::Windows { windows, ignore } => {
                for (w, _) in windows.drain(..) {
                    c.destroy_window(w)?;
                }
                *ignore = None;
                for &side in sides {
                    for r in self.desktop.outer_edges(side) {
                        let s = edge_strip(side, r);
                        let w = c.generate_id()?;
                        let aux = xproto::CreateWindowAux::new()
                            .override_redirect(1)
                            .event_mask(xproto::EventMask::ENTER_WINDOW | xproto::EventMask::LEAVE_WINDOW);
                        c.create_window(
                            0,
                            w,
                            self.root,
                            s.x as i16,
                            s.y as i16,
                            s.w as u16,
                            s.h as u16,
                            0,
                            xproto::WindowClass::INPUT_ONLY,
                            0,
                            &aux,
                        )?;
                        c.map_window(w)?;
                        windows.push((w, side));
                    }
                }
            }
        }
        // A pointer resting on an edge when it gets a window is not
        // pushing at it (mapping the window reports it entering).
        if matches!(&self.edges, Edges::Windows { windows, .. } if !windows.is_empty()) {
            let p = self.conn.query_pointer(self.root)?.reply()?;
            self.ignore_strip_at(Pos::new(p.root_x.into(), p.root_y.into()));
        }
        Ok(())
    }

    /// Ignores the edge window at `p` until the pointer leaves it.
    fn ignore_strip_at(&mut self, p: Pos) {
        let under = match &self.edges {
            Edges::Windows { windows, .. } => windows.iter().find(|(w, _)| self.strip_contains(*w, p)).map(|(w, _)| *w),
            Edges::Barriers(_) => None,
        };
        if let Edges::Windows { ignore, .. } = &mut self.edges {
            *ignore = under;
        }
    }

    /// Takes the pointer and keyboard. False if another program has
    /// either grabbed (a menu is open, a game, a screen locker).
    fn grab(&mut self, st: &mut Shared, side: Side, at: Pos) -> Result<bool> {
        let c = &self.conn;
        let window = c.generate_id()?;
        let aux = xproto::CreateWindowAux::new().override_redirect(1);
        let (x, y) = (at.x.round() as i16, at.y.round() as i16);
        c.create_window(0, window, self.root, x, y, 1, 1, 0, xproto::WindowClass::INPUT_ONLY, 0, &aux)?;
        c.map_window(window)?;
        let async_ = xproto::GrabMode::ASYNC;
        let pointer = c.grab_pointer(false, window, xproto::EventMask::NO_EVENT, async_, async_, window, self.blank, CURRENT_TIME)?;
        let keyboard = c.grab_keyboard(false, window, CURRENT_TIME, async_, async_)?;
        let state = c.query_pointer(self.root)?;
        let (pointer, keyboard) = (pointer.reply()?.status, keyboard.reply()?.status);
        let ok = xproto::GrabStatus::SUCCESS;
        if pointer != ok || keyboard != ok {
            tracing::debug!(?pointer, ?keyboard, "could not grab the pointer and keyboard");
            c.ungrab_pointer(CURRENT_TIME)?;
            c.ungrab_keyboard(CURRENT_TIME)?;
            c.destroy_window(window)?;
            c.flush()?;
            return Ok(false);
        }
        let mask = u16::from(state.reply()?.mask);
        st.next_id = st.next_id.wrapping_add(1);
        self.grab = Some(Grab { id: st.next_id, window });
        self.devices.clear();
        self.pressed.clear();
        self.select(st.controlled);
        self.conn.flush()?;
        st.send(Event::Modifiers(Mods::from_xkb(u32::from(mask))));
        st.send(Event::Activated { id: st.next_id, side, pos: at });
        tracing::debug!(id = st.next_id, %side, ?at, "captured");
        Ok(true)
    }

    /// Lets go of the pointer and keyboard, if held.
    fn ungrab(&mut self, st: &Shared) -> Result<()> {
        let Some(g) = self.grab.take() else { return Ok(()) };
        self.conn.ungrab_keyboard(CURRENT_TIME)?;
        self.conn.ungrab_pointer(CURRENT_TIME)?;
        self.conn.destroy_window(g.window)?;
        self.select(st.controlled);
        Ok(())
    }

    /// Capture ended without the daemon asking.
    fn deactivate(&mut self, st: &Shared) -> Result<()> {
        if let Some(g) = &self.grab {
            st.send(Event::Deactivated { id: g.id });
        }
        self.ungrab(st)
    }

    fn release(&mut self, st: &Shared, id: u32, pos: Option<Pos>) -> Result<()> {
        if self.grab.as_ref().is_none_or(|g| g.id != id) {
            return Ok(());
        }
        self.ungrab(st)?;
        if let Some(p) = pos {
            let p = self.desktop.clamp(p);
            self.conn.warp_pointer(NONE, self.root, 0, 0, 0, 0, p.x.round() as i16, p.y.round() as i16)?;
            self.ignore_strip_at(p);
        }
        Ok(())
    }

    fn strip_contains(&self, w: xproto::Window, p: Pos) -> bool {
        let Edges::Windows { windows, .. } = &self.edges else { return false };
        let Some(&(_, side)) = windows.iter().find(|(x, _)| *x == w) else { return false };
        self.desktop.outer_edges(side).map(|r| edge_strip(side, r)).any(|s| s.contains(p))
    }

    async fn session(
        &mut self,
        st: &mut Shared,
        cmds: &mut mpsc::UnboundedReceiver<CaptureCmd>,
        inj: &mut mpsc::UnboundedReceiver<Inject>,
        locked: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        // SAFETY: the connection, which owns the socket, outlives this.
        let socket = unsafe { Socket::new(self.conn.stream().as_raw_fd())? };
        st.report_desktop(&self.desktop.clone());
        self.set_edges(&st.sides.clone())?;
        // Keys held through a connection that broke are still down, and so
        // are those of an injector that died (XTest keeps them pressed).
        self.let_go(st)?;
        self.release_leftovers()?;
        self.select(st.controlled);
        st.show(None);
        let mut batch = Vec::new();
        loop {
            while let Some(ev) = self.conn.poll_for_event()? {
                self.on_x_event(st, ev)?;
            }
            self.conn.flush()?;
            tokio::select! {
                r = socket.readable() => r?,
                cmd = cmds.recv() => match cmd {
                    Some(CaptureCmd::SetSides(sides)) => {
                        st.sides = sides.iter().map(|a| a.side).collect();
                        self.set_edges(&st.sides.clone())?;
                    }
                    Some(CaptureCmd::Release { id, pos }) => self.release(st, id, pos)?,
                    Some(CaptureCmd::Activate { side }) => {
                        if self.grab.is_none() {
                            let p = self.conn.query_pointer(self.root)?.reply()?;
                            let at = Pos::new(p.root_x.into(), p.root_y.into());
                            if st.locked || !self.grab(st, side, at)? {
                                st.send(Event::Error(NO_CAPTURE.into()));
                            }
                        }
                    }
                    None => {
                        self.deactivate(st)?;
                        self.let_go(st)?;
                        return Ok(());
                    }
                },
                m = inj.recv() => {
                    let Some(first) = m else {
                        self.let_go(st)?;
                        return Ok(());
                    };
                    drain_coalesced(first, inj, &mut batch);
                    for m in batch.drain(..) {
                        self.inject(st, m)?;
                    }
                }
                l = lock_changed(locked) => {
                    st.locked = l;
                    if l {
                        tracing::info!("screen locked: the pointer comes home");
                        self.deactivate(st)?;
                        self.let_go(st)?;
                    }
                }
            }
        }
    }

    /// Cleans up a connection that ends: capture in progress ends.
    fn close(&mut self, st: &mut Shared) {
        if let Some(g) = self.grab.take() {
            st.send(Event::Deactivated { id: g.id });
        }
        let _ = self.conn.flush();
    }

    fn on_x_event(&mut self, st: &mut Shared, ev: x11rb::protocol::Event) -> Result<()> {
        use x11rb::protocol::Event as E;
        match ev {
            E::XinputBarrierHit(e) => {
                if self.grab.is_some() || st.locked || st.controlled || self.failed_hit == Some(e.eventid) {
                    return Ok(());
                }
                let Edges::Barriers(barriers) = &self.edges else { return Ok(()) };
                let Some(&(_, side)) = barriers.iter().find(|(b, _)| *b == e.barrier) else { return Ok(()) };
                let at = Pos::new(fp1616(e.root_x), fp1616(e.root_y));
                if !self.grab(st, side, at)? {
                    self.failed_hit = Some(e.eventid);
                }
            }
            E::XinputBarrierLeave(_) => self.failed_hit = None,
            E::EnterNotify(e) => {
                let Edges::Windows { windows, ignore } = &self.edges else { return Ok(()) };
                if self.grab.is_some() || st.locked || st.controlled || *ignore == Some(e.event) {
                    return Ok(());
                }
                if let Some(&(_, side)) = windows.iter().find(|(w, _)| *w == e.event) {
                    self.grab(st, side, Pos::new(e.root_x.into(), e.root_y.into()))?;
                }
            }
            E::LeaveNotify(e) => {
                if let Edges::Windows { ignore, .. } = &mut self.edges
                    && *ignore == Some(e.event)
                {
                    *ignore = None;
                }
            }
            E::RandrScreenChangeNotify(_) | E::RandrNotify(_) => {
                let desktop = self.read_desktop()?;
                if desktop != self.desktop {
                    tracing::info!(screens = ?desktop.screens, "monitors changed");
                    self.desktop = desktop;
                    self.deactivate(st)?;
                    st.report_desktop(&self.desktop.clone());
                    self.set_edges(&st.sides.clone())?;
                }
            }
            E::XinputRawKeyPress(e) => self.raw_key(st, e.sourceid, e.detail, true),
            E::XinputRawKeyRelease(e) => self.raw_key(st, e.sourceid, e.detail, false),
            E::XinputRawButtonPress(e) => {
                let emulated = u32::from(e.flags) & u32::from(xinput::PointerEventFlags::POINTER_EMULATED) != 0;
                self.raw_button(st, e.sourceid, e.detail, true, emulated)?;
            }
            E::XinputRawButtonRelease(e) => {
                let emulated = u32::from(e.flags) & u32::from(xinput::PointerEventFlags::POINTER_EMULATED) != 0;
                self.raw_button(st, e.sourceid, e.detail, false, emulated)?;
            }
            E::XinputRawMotion(e) => self.raw_motion(st, &e)?,
            E::Error(e) => tracing::debug!("X error: {e:?}"),
            _ => {}
        }
        Ok(())
    }

    /// Real input on this machine while a peer controls it.
    fn local(&mut self, st: &mut Shared, source: u16) -> bool {
        if self.grab.is_some() {
            return false;
        }
        if !self.xtest_devices.contains(&source) && st.gate.check(now_ms(), st.controlled) {
            st.send(Event::LocalInput);
        }
        true
    }

    fn raw_key(&mut self, st: &mut Shared, source: u16, keycode: u32, down: bool) {
        if self.local(st, source) {
            return;
        }
        if let Some(code) = xkeycode_to_evdev(keycode)
            && self.pressed.update(code, down)
        {
            st.send(Event::Key { code, down });
        }
    }

    fn raw_button(&mut self, st: &mut Shared, source: u16, button: u32, down: bool, emulated: bool) -> Result<()> {
        if self.local(st, source) || emulated {
            return Ok(());
        }
        match x_button(button) {
            Some(XButton::Button(b)) => {
                if self.pressed.update(b, down) {
                    st.send(Event::Button { button: b, down });
                }
            }
            // Devices with scroll valuators report scrolling through
            // those (smoothly); their wheel buttons are emulated.
            Some(XButton::Scroll(dx, dy)) if down && self.axes(source)?.scroll.is_empty() => {
                st.send(Event::ScrollDiscrete { dx, dy });
            }
            _ => {}
        }
        Ok(())
    }

    fn raw_motion(&mut self, st: &mut Shared, e: &xinput::RawMotionEvent) -> Result<()> {
        if self.local(st, e.sourceid) {
            return Ok(());
        }
        let bounds = self.desktop.bounds();
        let axes = self.axes(e.sourceid)?;
        let (mut dx, mut dy, mut sx, mut sy) = (0.0, 0.0, 0, 0);
        let numbers = e
            .valuator_mask
            .iter()
            .enumerate()
            .flat_map(|(i, word)| (0..32).filter(move |b| word & (1 << b) != 0).map(move |b| (i * 32 + b) as u16));
        for (n, v) in numbers.zip(&e.axisvalues) {
            let v = fp3232(v.integral, v.frac);
            let motion = |a: &mut Option<Axis>, extent: i32| -> Option<f64> {
                let a = a.as_mut().filter(|a| a.number == n)?;
                if !a.absolute {
                    return Some(v);
                }
                let d = a.last.map_or(0.0, |last| absolute_delta(last, v, a.min, a.max, extent));
                a.last = Some(v);
                Some(d)
            };
            if let Some(d) = motion(&mut axes.x, bounds.w) {
                dx += d;
            } else if let Some(d) = motion(&mut axes.y, bounds.h) {
                dy += d;
            } else if let Some((_, vertical, inc, acc)) = axes.scroll.iter_mut().find(|s| s.0 == n) {
                let d = acc.push(v, *inc);
                if *vertical { sy += d } else { sx += d }
            }
        }
        if dx != 0.0 || dy != 0.0 {
            st.send(Event::Motion { dx, dy });
        }
        if sx != 0 || sy != 0 {
            st.send(Event::ScrollDiscrete { dx: sx, dy: sy });
        }
        Ok(())
    }

    /// The valuators of device `id`, asked once per capture.
    fn axes(&mut self, id: u16) -> Result<&mut Axes> {
        if !self.devices.contains_key(&id) {
            let mut axes = Axes::default();
            let reply = self.conn.xinput_xi_query_device(id)?.reply();
            for class in reply.iter().flat_map(|r| &r.infos).filter(|d| d.deviceid == id).flat_map(|d| &d.classes) {
                match &class.data {
                    xinput::DeviceClassData::Valuator(v) if v.number < 2 => {
                        let a = Axis {
                            number: v.number,
                            absolute: v.mode == xinput::ValuatorMode::ABSOLUTE,
                            min: fp3232(v.min.integral, v.min.frac),
                            max: fp3232(v.max.integral, v.max.frac),
                            last: None,
                        };
                        if v.number == 0 { axes.x = Some(a) } else { axes.y = Some(a) }
                    }
                    xinput::DeviceClassData::Scroll(s) => axes.scroll.push((
                        s.number,
                        s.scroll_type == xinput::ScrollType::VERTICAL,
                        fp3232(s.increment.integral, s.increment.frac),
                        ScrollAxis::default(),
                    )),
                    _ => {}
                }
            }
            self.devices.insert(id, axes);
        }
        Ok(self.devices.get_mut(&id).expect("inserted above"))
    }

    fn fake(&self, type_: u8, detail: u8, root: xproto::Window, x: i16, y: i16) -> Result<()> {
        self.conn.xtest_fake_input(type_, detail, CURRENT_TIME, root, x, y, 0)?;
        Ok(())
    }

    fn inject(&mut self, st: &mut Shared, m: Inject) -> Result<()> {
        st.held.update(&m);
        match m {
            Inject::MoveTo(p) => {
                let p = self.desktop.clamp(p);
                self.fake(xproto::MOTION_NOTIFY_EVENT, 0, self.root, p.x.round() as i16, p.y.round() as i16)?;
            }
            Inject::MoveBy { dx, dy } => {
                let c = |v: i32| v.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
                self.fake(xproto::MOTION_NOTIFY_EVENT, 1, NONE, c(dx), c(dy))?;
            }
            Inject::Button { button, down } => match evdev_to_x_button(button) {
                Some(b) => self.fake(if down { 4 } else { 5 }, b, NONE, 0, 0)?,
                None => tracing::debug!(button, "no X11 equivalent for mouse button"),
            },
            Inject::Scroll { dx, dy } => {
                let (x, y) = st.notches.smooth(dx, dy);
                self.click_notches(x, y)?;
            }
            Inject::ScrollDiscrete { dx, dy } => {
                let (x, y) = st.notches.discrete(dx, dy);
                self.click_notches(x, y)?;
            }
            Inject::ScrollStop => st.notches.stop(),
            Inject::Key { code, down } => match evdev_to_xkeycode(code) {
                Some(k) => self.fake(if down { 2 } else { 3 }, k, NONE, 0, 0)?,
                None => tracing::debug!(code, "key code beyond X11's range"),
            },
            Inject::Controlled(c) => {
                st.controlled = c;
                self.select(c);
            }
        }
        Ok(())
    }

    /// Lets go of the keys and buttons the XTest devices hold: what an
    /// earlier run (or another XTest client) pressed and never released,
    /// because it was killed. Nobody holds a key through XTest on purpose
    /// for long, so whatever is down when we start is stuck.
    fn release_leftovers(&self) -> Result<()> {
        let mut released = 0;
        for &id in &self.xtest_devices {
            let Ok(Ok(state)) = self.conn.xinput_query_device_state(id as u8).map(|c| c.reply()) else { continue };
            for class in state.classes {
                let (type_, held) = match class.data {
                    xinput::InputStateData::Key(k) => (3, set_bits(&k.keys)),
                    xinput::InputStateData::Button(b) => (5, set_bits(&b.buttons)),
                    _ => continue,
                };
                for n in held.into_iter().filter_map(|n| u8::try_from(n).ok()).filter(|&n| n > 0) {
                    self.fake(type_, n, NONE, 0, 0)?;
                    released += 1;
                }
            }
        }
        if released > 0 {
            tracing::warn!(released, "let go of keys and buttons a killed program had held down through XTest");
        }
        Ok(())
    }

    fn click_notches(&self, dx: i32, dy: i32) -> Result<()> {
        for b in notch_buttons(dx, dy) {
            self.fake(4, b, NONE, 0, 0)?;
            self.fake(5, b, NONE, 0, 0)?;
        }
        Ok(())
    }

    /// Lets go of every injected key and button still held.
    fn let_go(&mut self, st: &mut Shared) -> Result<()> {
        if st.held.is_empty() {
            return Ok(());
        }
        let up = st.held.release();
        tracing::info!(count = up.len(), "letting go of injected keys and buttons");
        for m in up {
            self.inject(st, m)?;
        }
        // Wait for the server to have done it: requests still in flight
        // when the connection closes (we may be on the way out) can be lost.
        self.conn.get_input_focus()?.reply()?;
        Ok(())
    }
}

/// For tests: the backend against the X server at `display`.
#[cfg(test)]
fn open_on(display: &str) -> Result<(Backend, watch::Sender<bool>)> {
    open_target(Target { display: Some(display.to_owned()), barriers: true })
}

#[cfg(test)]
fn open_target(target: Target) -> Result<(Backend, watch::Sender<bool>)> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (locked_tx, locked) = watch::channel(false);
    let first = X::connect(&target)?;
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    spawn_thread("one-mk-x11", move || run(Some(first), target, ev_tx, cap_rx, inj_rx, locked))?;
    let focus = Box::new(crate::system::Unknown);
    let locker = Box::new(NoLock);
    Ok((Backend { events: ev_rx, capture: cap_tx, inject: inj_tx, focus, locker }, locked_tx))
}

#[cfg(test)]
struct NoLock;

#[cfg(test)]
impl crate::ScreenLocker for NoLock {
    fn lock(&mut self) -> Result<()> {
        Err(anyhow::anyhow!("not in tests"))
    }
}

#[cfg(test)]
#[path = "x11_tests.rs"]
mod tests;
