//! wlroots backend: for Wayland compositors without the input portals
//! that speak the wlroots protocols (Sway, Hyprland, labwc, river,
//! Wayfire).
//!
//! - Edges: an invisible layer-shell surface, one pixel thick, along each
//!   edge that leads to a peer (overlay layer, `exclusive_zone` -1, so it
//!   pushes nothing aside). Capture starts when the pointer is on one and
//!   moves on outwards (relative pointer motion, which keeps coming when
//!   the pointer can't move any further), as a pointer barrier would.
//! - Capture: a transparent full-screen layer surface on that output with
//!   exclusive keyboard interactivity takes the keyboard; the pointer is
//!   locked on it (pointer constraints), the cursor hidden, and motion
//!   read from `zwp_relative_pointer_v1` (accelerated, as the pointer here
//!   would have moved). Without pointer constraints every output gets a
//!   capture surface, and the pointer, though it moves, stays on them.
//! - Injection: `zwlr_virtual_pointer_v1` and `zwp_virtual_keyboard_v1`,
//!   the keyboard with the seat's own keymap (so this machine's layout
//!   applies) and modifiers worked out here, since the compositor takes
//!   them from the client.
//! - Monitors: `wl_output` with xdg-output, in logical pixels.
//!
//! Exactly one thread, asleep on the Wayland socket and the daemon's
//! channels: no timers. Nothing stays stuck: every surface, the pointer
//! lock and the virtual devices belong to the connection, and the
//! compositor removes them (and, with them, keys still held: wlroots
//! releases a destroyed keyboard's keys) when the process ends however it
//! ends. While running, capture ends with `Event::Deactivated` when the
//! compositor takes the keyboard or the pointer lock away (a lock screen,
//! another exclusive surface), when the outputs change, the screen locks
//! or the daemon goes away; injected keys and buttons still held are let
//! go of when the daemon exits or the screen locks.
//!
//! `Event::LocalInput` is never sent: a Wayland client sees no input
//! meant for other clients.

use crate::common_logic::{Backoff, drain_coalesced};
use crate::linux_logic::{
    AxisFrame, EdgePush, Held, Notches, OutputGeometry, Pressed, absolute_motion, armed_at, edge_strips, pushes_out,
    segment_point, wlr_missing,
};
use crate::linux_rt::{Socket, forward_lock, lock_changed, spawn_thread};
use crate::xkb::{KeyboardMods, Modifiers};
use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::{Context, Result, anyhow, bail};
use one_mk_proto::{Desktop, Edge, Mods, Pos, Rect, Side};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::pointer_constraints::zv1::client::{zwp_locked_pointer_v1, zwp_pointer_constraints_v1};
use wayland_protocols::wp::relative_pointer::zv1::client::{zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use wayland_protocols_wlr::virtual_pointer::v1::client::{zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1};

use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};

const NO_CAPTURE: &str = "Can't move the pointer to another computer right now: the Wayland compositor is not ready.";
const LAYER_NAMESPACE: &str = "one-mk";
/// Keymap format `xkb_v1` (`wl_keyboard.keymap_format`).
const XKB_V1: u32 = 1;
/// A keymap for a seat without a keyboard; the compositor fills in the
/// includes from its own XKB data.
const FALLBACK_KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
    xkb_geometry { include \"pc(pc105)\" };\n\
};\n";

pub(crate) async fn open(_state_dir: PathBuf) -> Result<Backend> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (sys_tx, sys_rx) = mpsc::unbounded_channel();
    let (focus, locker) = crate::system::native(&sys_tx);
    drop(sys_tx);
    let (locked_tx, locked) = watch::channel(false);
    tokio::spawn(forward_lock(sys_rx, ev_tx.clone(), locked_tx));
    // Fail at once, and clearly, on a compositor that can't do it.
    let first = Session::connect().context("can't use the Wayland compositor's wlroots protocols")?;
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    spawn_thread("one-mk-wlroots", move || run(first, ev_tx, cap_rx, inj_rx, locked))?;
    Ok(Backend { name: "wlroots", events: ev_rx, capture: cap_tx, inject: inj_tx, focus, locker })
}

/// The compositor's global interface names, for choosing a backend.
pub(crate) fn globals() -> Option<Vec<String>> {
    let conn = Connection::connect_to_env().ok()?;
    let (list, _queue) = registry_queue_init::<Probe>(&conn).ok()?;
    Some(list.contents().clone_list().into_iter().map(|g| g.interface).collect())
}

struct Probe;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Runs connections to the compositor until the daemon goes away,
/// reconnecting with backoff when one breaks.
async fn run(
    first: (Session, EventQueue<State>),
    tx: mpsc::UnboundedSender<Event>,
    mut cmds: mpsc::UnboundedReceiver<CaptureCmd>,
    mut inj: mpsc::UnboundedReceiver<Inject>,
    mut locked: watch::Receiver<bool>,
) {
    let mut keep = Keep { edges: Vec::new(), next_id: 0, locked: *locked.borrow_and_update(), reported: None };
    let mut first = Some(first);
    let mut backoff = Backoff::default();
    let mut shown = None::<String>;
    loop {
        let started = Instant::now();
        let result = match first.take().map_or_else(Session::connect, Ok) {
            Ok((session, queue)) => {
                if shown.take().is_some() {
                    for part in ["capture", "control"] {
                        let _ = tx.send(Event::Status { part, problem: None });
                    }
                }
                let mut state = State::new(session, tx.clone(), keep);
                let r = state.run(queue, &mut cmds, &mut inj, &mut locked).await;
                state.end_capture(true);
                keep = state.keep;
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
        tracing::warn!("wlroots input: {e:#}; trying again in {delay:?}");
        let problem = format!("The Wayland compositor connection failed ({e:#}). Retrying in the background.");
        if shown.as_ref() != Some(&problem) {
            for part in ["capture", "control"] {
                let _ = tx.send(Event::Status { part, problem: Some(problem.clone()) });
            }
            shown = Some(problem);
        }
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                cmd = cmds.recv() => match cmd {
                    Some(CaptureCmd::SetEdges(e)) => keep.edges = e,
                    Some(CaptureCmd::Activate { .. }) => { let _ = tx.send(Event::Error(NO_CAPTURE.into())); }
                    Some(CaptureCmd::Release { .. }) => {}
                    None => return,
                },
                m = inj.recv() => if m.is_none() { return },
                l = lock_changed(&mut locked) => keep.locked = l,
            }
        }
    }
}

/// What outlives one connection.
struct Keep {
    edges: Vec<Edge>,
    next_id: u32,
    locked: bool,
    reported: Option<Desktop>,
}

/// The globals of one connection.
struct Session {
    conn: Connection,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    seat: wl_seat::WlSeat,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    pointers: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    keyboards: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    relative: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
    constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
    xdg_outputs: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    globals: GlobalList,
}

impl Session {
    fn connect() -> Result<(Session, EventQueue<State>)> {
        let conn = Connection::connect_to_env().context("no Wayland compositor (WAYLAND_DISPLAY)")?;
        let (globals, queue) = registry_queue_init::<State>(&conn)?;
        let names: Vec<String> = globals.contents().clone_list().into_iter().map(|g| g.interface).collect();
        let missing = wlr_missing(&names);
        if !missing.is_empty() {
            bail!("the compositor lacks {}", missing.join(", "));
        }
        let qh = queue.handle();
        let s = Session {
            compositor: globals.bind(&qh, 4..=6, ())?,
            shm: globals.bind(&qh, 1..=1, ())?,
            seat: globals.bind(&qh, 5..=9, ())?,
            layer_shell: globals.bind(&qh, 1..=4, ())?,
            pointers: globals.bind(&qh, 1..=2, ())?,
            keyboards: globals.bind(&qh, 1..=1, ())?,
            relative: globals.bind(&qh, 1..=1, ())?,
            constraints: globals.bind(&qh, 1..=1, ()).ok(),
            xdg_outputs: globals.bind(&qh, 2..=3, ()).ok(),
            viewporter: globals.bind(&qh, 1..=1, ()).ok(),
            globals,
            conn,
        };
        Ok((s, queue))
    }
}

struct Output {
    /// The registry name.
    name: u32,
    wl: wl_output::WlOutput,
    xdg: Option<zxdg_output_v1::ZxdgOutputV1>,
    geometry: OutputGeometry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Edge(Side),
    Capture,
}

struct Layer {
    id: u32,
    role: Role,
    /// The output's registry name.
    output: u32,
    edge_rect: Option<Rect>,
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    viewport: Option<wp_viewport::WpViewport>,
    buffer: Option<wl_buffer::WlBuffer>,
}

impl Layer {
    fn destroy(self) {
        if let Some(v) = self.viewport {
            v.destroy();
        }
        self.layer.destroy();
        self.surface.destroy();
        if let Some(b) = self.buffer {
            b.destroy();
        }
    }
}

struct Capture {
    id: u32,
    lock: Option<zwp_locked_pointer_v1::ZwpLockedPointerV1>,
}

struct State {
    s: Session,
    tx: mpsc::UnboundedSender<Event>,
    qh: Option<QueueHandle<State>>,
    keep: Keep,
    outputs: Vec<Output>,
    desktop: Option<Desktop>,
    outputs_changed: bool,
    layers: Vec<Layer>,
    next_layer: u32,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    relative: Option<zwp_relative_pointer_v1::ZwpRelativePointerV1>,
    /// The surface under the pointer, with the serial of its enter.
    pointer_on: Option<(wl_surface::WlSurface, u32)>,
    edge: EdgePush,
    capture: Option<Capture>,
    pressed: Pressed,
    axis: AxisFrame,
    /// The seat's keymap, for the virtual keyboard.
    keymap: Option<Vec<u8>>,
    keymap_sent: bool,
    group: u32,
    vpointer: Option<zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1>,
    vkeyboard: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    held: Held,
    /// The virtual keyboard's modifiers, from the keymap it was given.
    mods: Option<KeyboardMods>,
    /// The modifiers last sent to it.
    mods_sent: Option<Modifiers>,
    notches: Notches,
    /// A peer controls this machine: its motion (relative events come
    /// with injected motion too) must not push through our edges.
    controlled: bool,
    started: Instant,
}

impl State {
    fn new(s: Session, tx: mpsc::UnboundedSender<Event>, keep: Keep) -> State {
        State {
            s,
            tx,
            qh: None,
            keep,
            outputs: Vec::new(),
            desktop: None,
            outputs_changed: false,
            layers: Vec::new(),
            next_layer: 0,
            pointer: None,
            keyboard: None,
            relative: None,
            pointer_on: None,
            edge: EdgePush::default(),
            capture: None,
            pressed: Pressed::default(),
            axis: AxisFrame::default(),
            keymap: None,
            keymap_sent: false,
            group: 0,
            vpointer: None,
            vkeyboard: None,
            held: Held::default(),
            mods: None,
            mods_sent: None,
            notches: Notches::default(),
            controlled: false,
            started: Instant::now(),
        }
    }

    fn qh(&self) -> &QueueHandle<State> {
        self.qh.as_ref().expect("set in run")
    }

    fn send(&self, ev: Event) {
        let _ = self.tx.send(ev);
    }

    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    async fn run(
        &mut self,
        mut queue: EventQueue<State>,
        cmds: &mut mpsc::UnboundedReceiver<CaptureCmd>,
        inj: &mut mpsc::UnboundedReceiver<Inject>,
        locked: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        self.qh = Some(queue.handle());
        let qh = queue.handle();
        for g in self.s.globals.contents().clone_list() {
            if g.interface == wl_output::WlOutput::interface().name {
                self.add_output(g.name, g.version, &qh);
            }
        }
        // The outputs' and the seat's first events.
        queue.roundtrip(self)?;
        queue.roundtrip(self)?;
        self.outputs_changed = true;
        self.after_dispatch();
        let fd = self.s.conn.backend().poll_fd().as_raw_fd();
        // SAFETY: the connection, which owns the socket, outlives this.
        let socket = unsafe { Socket::new(fd)? };
        let mut batch = Vec::new();
        loop {
            queue.dispatch_pending(self)?;
            self.after_dispatch();
            queue.flush()?;
            let Some(guard) = queue.prepare_read() else { continue };
            tokio::select! {
                r = socket.readable() => {
                    r?;
                    match guard.read() {
                        Ok(_) => {}
                        Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(anyhow!("the Wayland connection broke: {e}")),
                    }
                }
                cmd = cmds.recv() => {
                    drop(guard);
                    match cmd {
                        Some(CaptureCmd::SetEdges(edges)) => {
                            self.keep.edges = edges;
                            self.set_edges();
                        }
                        Some(CaptureCmd::Release { id, pos }) => {
                            if self.capture.as_ref().is_some_and(|c| c.id == id) {
                                self.end_capture(false);
                                if let Some(p) = pos {
                                    self.move_to(p);
                                }
                            }
                        }
                        Some(CaptureCmd::Activate { side }) => self.activate_cmd(side),
                        None => {
                            self.end_capture(true);
                            self.let_go();
                            queue.flush()?;
                            return Ok(());
                        }
                    }
                }
                m = inj.recv() => {
                    drop(guard);
                    let Some(first) = m else {
                        self.let_go();
                        queue.flush()?;
                        return Ok(());
                    };
                    drain_coalesced(first, inj, &mut batch);
                    for m in batch.drain(..) {
                        self.inject(m);
                    }
                }
                l = lock_changed(locked) => {
                    drop(guard);
                    self.keep.locked = l;
                    if l {
                        tracing::info!("screen locked: the pointer comes home");
                        self.end_capture(true);
                        self.let_go();
                    }
                }
            }
        }
    }

    fn add_output(&mut self, name: u32, version: u32, qh: &QueueHandle<State>) {
        let wl: wl_output::WlOutput = self.s.globals.registry().bind(name, version.min(4), qh, name);
        let xdg = self.s.xdg_outputs.as_ref().map(|m| m.get_xdg_output(&wl, qh, name));
        self.outputs.push(Output { name, wl, xdg, geometry: OutputGeometry { scale: 1, ..Default::default() } });
    }

    fn remove_output(&mut self, name: u32) {
        let Some(i) = self.outputs.iter().position(|o| o.name == name) else { return };
        let o = self.outputs.remove(i);
        if let Some(x) = o.xdg {
            x.destroy();
        }
        if o.wl.version() >= 3 {
            o.wl.release();
        }
        self.outputs_changed = true;
    }

    /// Work left by the event handlers.
    fn after_dispatch(&mut self) {
        if std::mem::take(&mut self.outputs_changed) {
            let rects: Vec<Rect> = self.outputs.iter().filter_map(|o| o.geometry.rect()).collect();
            let desktop = (!rects.is_empty()).then(|| Desktop::new(rects));
            if desktop != self.desktop {
                tracing::info!(screens = ?desktop.as_ref().map(|d| &d.screens), "outputs changed");
                self.desktop = desktop;
                self.end_capture(true);
                if let Some(d) = &self.desktop
                    && self.keep.reported.as_ref() != Some(d)
                {
                    self.keep.reported = Some(d.clone());
                    self.send(Event::Desktop(d.clone()));
                }
                self.set_edges();
            }
        }
    }

    fn output_rect(&self, name: u32) -> Option<Rect> {
        self.outputs.iter().find(|o| o.name == name)?.geometry.rect()
    }

    /// Puts edge surfaces on the armed edge segments (inner exposed edges
    /// too: a layer surface sits on its own output's edge), replacing the
    /// old ones.
    fn set_edges(&mut self) {
        let old: Vec<Layer> = {
            let (edges, rest) = std::mem::take(&mut self.layers).into_iter().partition(|l| matches!(l.role, Role::Edge(_)));
            self.layers = rest;
            edges
        };
        old.into_iter().for_each(Layer::destroy);
        self.edge.leave();
        let Some(desktop) = self.desktop.clone() else { return };
        for (side, output, strip) in edge_strips(&self.keep.edges, &desktop) {
            let Some(o) = self.outputs.iter().find(|o| o.geometry.rect() == Some(output)) else { continue };
            let (anchor, size) = match side {
                Side::Left => (Anchor::Left | Anchor::Top, (1, strip.h as u32)),
                Side::Right => (Anchor::Right | Anchor::Top, (1, strip.h as u32)),
                Side::Top => (Anchor::Top | Anchor::Left, (strip.w as u32, 1)),
                Side::Bottom => (Anchor::Bottom | Anchor::Left, (strip.w as u32, 1)),
            };
            let (name, wl) = (o.name, o.wl.clone());
            self.add_layer(Role::Edge(side), name, &wl, anchor, size, KeyboardInteractivity::None);
            let layer = self.layers.last_mut().expect("added edge");
            layer.edge_rect = Some(strip);
            let (top, left) = if side.along_x() { (0, strip.x - output.x) } else { (strip.y - output.y, 0) };
            layer.layer.set_margin(top, 0, 0, left);
            layer.surface.commit();
        }
    }

    fn add_layer(
        &mut self,
        role: Role,
        output: u32,
        wl: &wl_output::WlOutput,
        anchor: Anchor,
        (w, h): (u32, u32),
        keyboard: KeyboardInteractivity,
    ) {
        let qh = self.qh().clone();
        self.next_layer = self.next_layer.wrapping_add(1);
        let id = self.next_layer;
        let surface = self.s.compositor.create_surface(&qh, ());
        let layer = self.s.layer_shell.get_layer_surface(
            &surface,
            Some(wl),
            zwlr_layer_shell_v1::Layer::Overlay,
            LAYER_NAMESPACE.into(),
            &qh,
            id,
        );
        layer.set_anchor(anchor);
        layer.set_size(w, h);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(keyboard);
        surface.commit();
        let viewport = self.s.viewporter.as_ref().map(|v| v.get_viewport(&surface, &qh, ()));
        self.layers.push(Layer { id, role, output, edge_rect: None, surface, layer, viewport, buffer: None });
    }

    fn configure(&mut self, id: u32, serial: u32, w: u32, h: u32) {
        let qh = self.qh().clone();
        let Some(i) = self.layers.iter().position(|l| l.id == id) else { return };
        let (w, h) = (w.max(1), h.max(1));
        let size = if self.layers[i].viewport.is_some() { (1, 1) } else { (w, h) };
        let buffer = match transparent_buffer(&self.s.shm, size, &qh) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("making a surface buffer: {e:#}");
                return;
            }
        };
        let l = &mut self.layers[i];
        l.layer.ack_configure(serial);
        if let Some(v) = &l.viewport {
            v.set_destination(w as i32, h as i32);
        }
        l.surface.attach(Some(&buffer), 0, 0);
        l.surface.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        l.surface.commit();
        if let Some(old) = l.buffer.replace(buffer) {
            old.destroy();
        }
    }

    fn layer_of(&self, surface: &wl_surface::WlSurface) -> Option<&Layer> {
        self.layers.iter().find(|l| &l.surface == surface)
    }

    /// Where surface-local `(sx, sy)` on an edge surface is on the desktop.
    fn edge_pos(&self, l: &Layer, _side: Side, sx: f64, sy: f64) -> Option<Pos> {
        let strip = l.edge_rect?;
        let pos = Pos::new(f64::from(strip.x) + sx, f64::from(strip.y) + sy);
        self.output_rect(l.output)?.contains(pos).then_some(pos)
    }

    fn activate_cmd(&mut self, side: Side) {
        if self.capture.is_some() {
            return;
        }
        // The middle of the first armed segment of that side, else the
        // middle of the side.
        let first = self.keep.edges.iter().find(|e| e.side == side && e.lo < e.hi).map(segment_point);
        let at = first.or_else(|| self.desktop.as_ref().map(|d| d.point_on_edge(side, 0.5, 0.0)));
        match at {
            Some(at) if !self.keep.locked => {
                // Onto the output that gets the capture surface.
                self.move_to(at);
                self.activate(side, at);
            }
            _ => self.send(Event::Error(NO_CAPTURE.into())),
        }
    }

    /// Starts capturing: surfaces that take the keyboard and the pointer.
    fn activate(&mut self, side: Side, at: Pos) {
        let Some(desktop) = self.desktop.clone() else { return };
        let screen = desktop.screen_at(at).copied().unwrap_or(*desktop.primary());
        let targets: Vec<(u32, wl_output::WlOutput, bool)> = self
            .outputs
            .iter()
            .filter_map(|o| {
                let r = o.geometry.rect()?;
                let main = r == screen;
                // Without pointer constraints the pointer roams: cover every output.
                (main || self.s.constraints.is_none()).then(|| (o.name, o.wl.clone(), main))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        for (name, wl, main) in targets {
            let all = Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right;
            let kb = if main { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
            self.add_layer(Role::Capture, name, &wl, all, (0, 0), kb);
        }
        self.edge.leave();
        self.pressed.clear();
        self.axis = AxisFrame::default();
        self.keep.next_id = self.keep.next_id.wrapping_add(1);
        let id = self.keep.next_id;
        self.capture = Some(Capture { id, lock: None });
        self.send(Event::Activated { id, side, pos: at });
        tracing::debug!(id, %side, ?at, "captured");
    }

    /// Ends capture; `tell` the daemon if it did not ask for it.
    fn end_capture(&mut self, tell: bool) {
        let Some(c) = self.capture.take() else { return };
        if tell {
            self.send(Event::Deactivated { id: c.id });
        }
        if let Some(lock) = c.lock {
            lock.destroy();
        }
        let (caps, rest) = std::mem::take(&mut self.layers).into_iter().partition(|l| l.role == Role::Capture);
        self.layers = rest;
        caps.into_iter().for_each(Layer::destroy);
        self.pressed.clear();
        let _ = self.s.conn.flush();
    }

    // ---------------------------------------------------------- injection

    fn vpointer(&mut self) -> &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1 {
        if self.vpointer.is_none() {
            let qh = self.qh().clone();
            self.vpointer = Some(self.s.pointers.create_virtual_pointer(Some(&self.s.seat), &qh, ()));
        }
        self.vpointer.as_ref().expect("created above")
    }

    fn vkeyboard(&mut self) -> Option<&zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1> {
        if self.vkeyboard.is_none() {
            let qh = self.qh().clone();
            self.vkeyboard = Some(self.s.keyboards.create_virtual_keyboard(&self.s.seat, &qh, ()));
            self.keymap_sent = false;
        }
        if !self.keymap_sent {
            let keymap = self.keymap.clone().unwrap_or_else(|| FALLBACK_KEYMAP.as_bytes().to_vec());
            match keymap_fd(&keymap) {
                Ok((fd, size)) => {
                    self.vkeyboard.as_ref()?.keymap(XKB_V1, fd.as_fd(), size);
                    self.keymap_sent = true;
                    // The compositor keeps the keys held across a new
                    // keymap: so does the state the modifiers come from.
                    let held = self.mods.as_ref().map_or(&[][..], |m| m.held());
                    self.mods = Some(KeyboardMods::new(&keymap, held, self.group));
                    self.mods_sent = None;
                }
                Err(e) => {
                    tracing::warn!("passing the keymap on: {e:#}");
                    return None;
                }
            }
        }
        self.vkeyboard.as_ref()
    }

    fn move_to(&mut self, p: Pos) {
        let Some(d) = self.desktop.clone() else { return };
        let (x, y, w, h) = absolute_motion(&d, p);
        let t = self.time();
        let v = self.vpointer();
        v.motion_absolute(t, x, y, w, h);
        v.frame();
    }

    fn inject(&mut self, m: Inject) {
        self.held.update(&m);
        let t = self.time();
        match m {
            Inject::MoveTo(p) => self.move_to(p),
            Inject::MoveBy { dx, dy } => {
                let v = self.vpointer();
                v.motion(t, f64::from(dx), f64::from(dy));
                v.frame();
            }
            Inject::Button { button, down } => {
                let state = if down { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
                let v = self.vpointer();
                v.button(t, button, state);
                v.frame();
            }
            Inject::Scroll { dx, dy } => {
                let v = self.vpointer();
                v.axis_source(wl_pointer::AxisSource::Finger);
                if dy != 0.0 {
                    v.axis(t, wl_pointer::Axis::VerticalScroll, dy);
                }
                if dx != 0.0 {
                    v.axis(t, wl_pointer::Axis::HorizontalScroll, dx);
                }
                v.frame();
            }
            Inject::ScrollDiscrete { dx, dy } => {
                let (nx, ny) = self.notches.discrete(dx, dy);
                let v = self.vpointer();
                let discrete = v.version() >= 2;
                v.axis_source(wl_pointer::AxisSource::Wheel);
                // 15 per notch, as libinput reports a wheel.
                for (axis, d, n) in
                    [(wl_pointer::Axis::VerticalScroll, dy, ny), (wl_pointer::Axis::HorizontalScroll, dx, nx)]
                {
                    let value = f64::from(d) / 8.0;
                    if d == 0 {
                        continue;
                    }
                    if discrete && n != 0 {
                        v.axis_discrete(t, axis, value, n);
                    } else {
                        v.axis(t, axis, value);
                    }
                }
                v.frame();
            }
            Inject::ScrollStop => {
                self.notches.stop();
                let v = self.vpointer();
                v.axis_source(wl_pointer::AxisSource::Finger);
                v.axis_stop(t, wl_pointer::Axis::VerticalScroll);
                v.axis_stop(t, wl_pointer::Axis::HorizontalScroll);
                v.frame();
            }
            Inject::Key { code, down } => {
                if self.vkeyboard().is_none() {
                    return;
                }
                let (Some(k), Some(mods)) = (self.vkeyboard.as_ref(), self.mods.as_mut()) else { return };
                mods.key(code, down);
                let m = mods.get();
                k.key(t, code, u32::from(down));
                if self.mods_sent != Some(m) {
                    k.modifiers(m.depressed, m.latched, m.locked, m.group);
                    self.mods_sent = Some(m);
                }
            }
            // No local input to watch for, but edges to keep shut.
            Inject::Controlled(c) => self.controlled = c,
        }
    }

    /// Lets go of every injected key and button still held.
    fn let_go(&mut self) {
        if self.held.is_empty() {
            return;
        }
        let up = self.held.release();
        tracing::info!(count = up.len(), "letting go of injected keys and buttons");
        for m in up {
            self.inject(m);
        }
        // Wait for the compositor to have done it: requests still in
        // flight when the connection closes (we may be on the way out)
        // can be lost.
        let _ = self.s.conn.roundtrip();
    }

    // ------------------------------------------------------------ capture

    fn on_pointer(&mut self, ev: wl_pointer::Event) {
        use wl_pointer::Event as P;
        match ev {
            P::Enter { serial, surface, surface_x, surface_y } => {
                self.pointer_on = Some((surface.clone(), serial));
                let Some(l) = self.layer_of(&surface) else { return };
                match l.role {
                    Role::Edge(side) => {
                        if let Some(p) = self.edge_pos(l, side, surface_x, surface_y) {
                            self.edge.enter(side, p);
                        }
                    }
                    Role::Capture => self.on_capture_surface(&surface, serial),
                }
            }
            P::Leave { .. } => {
                self.pointer_on = None;
                self.edge.leave();
            }
            P::Motion { surface_x, surface_y, .. } => {
                let Some((surface, _)) = &self.pointer_on else { return };
                if let Some(l) = self.layer_of(surface)
                    && let Role::Edge(side) = l.role
                    && let Some(p) = self.edge_pos(l, side, surface_x, surface_y)
                {
                    self.edge.moved(p);
                }
            }
            P::Button { button, state, .. } => {
                let down = state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                if self.capture.is_some() && self.pressed.update(button, down) {
                    self.send(Event::Button { button, down });
                }
            }
            P::Axis { axis, value, .. } => {
                if let WEnum::Value(a) = axis {
                    self.axis.value(a as u32, value);
                }
                self.axis_frame_if_unframed();
            }
            P::AxisSource { axis_source } => {
                if axis_source == WEnum::Value(wl_pointer::AxisSource::Wheel) {
                    self.axis.wheel_source();
                }
            }
            P::AxisStop { .. } => {
                self.axis.stop();
                self.axis_frame_if_unframed();
            }
            P::AxisDiscrete { axis: WEnum::Value(a), discrete } => self.axis.discrete(a as u32, discrete),
            P::AxisValue120 { axis: WEnum::Value(a), value120 } => self.axis.value120(a as u32, value120),
            P::Frame => self.axis_frame(),
            _ => {}
        }
    }

    /// Before `wl_pointer` version 5 there are no frames.
    fn axis_frame_if_unframed(&mut self) {
        if self.pointer.as_ref().is_some_and(|p| p.version() < 5) {
            self.axis_frame();
        }
    }

    fn axis_frame(&mut self) {
        let events = self.axis.finish();
        if self.capture.is_some() {
            for ev in events {
                self.send(ev);
            }
        }
    }

    /// The pointer is on a capture surface: hide it and lock it there.
    fn on_capture_surface(&mut self, surface: &wl_surface::WlSurface, serial: u32) {
        if let Some(p) = &self.pointer {
            p.set_cursor(serial, None, 0, 0);
        }
        let qh = self.qh().clone();
        let (Some(c), Some(constraints), Some(pointer)) = (self.capture.as_mut(), &self.s.constraints, &self.pointer)
        else {
            return;
        };
        if c.lock.is_none() {
            let lifetime = zwp_pointer_constraints_v1::Lifetime::Oneshot;
            c.lock = Some(constraints.lock_pointer(surface, pointer, None, lifetime, &qh, ()));
        }
    }

    fn on_relative(&mut self, dx: f64, dy: f64) {
        if self.capture.is_some() {
            if dx != 0.0 || dy != 0.0 {
                self.send(Event::Motion { dx, dy });
            }
            return;
        }
        if self.keep.locked || self.controlled {
            return;
        }
        if let Some((side, at)) = self.edge.push(dx, dy) {
            debug_assert!(pushes_out(side, dx, dy));
            if armed_at(&self.keep.edges, side, at) {
                self.activate(side, at);
            }
        }
    }

    fn on_keyboard(&mut self, ev: wl_keyboard::Event) {
        use wl_keyboard::Event as K;
        match ev {
            K::Keymap { format, fd, size } => {
                if format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    match read_keymap(&fd, size as usize) {
                        Ok(k) => {
                            if self.keymap.as_ref() != Some(&k) {
                                self.keymap = Some(k);
                                self.keymap_sent = false;
                            }
                        }
                        Err(e) => tracing::debug!("reading the seat's keymap: {e}"),
                    }
                }
            }
            K::Leave { surface, .. } => {
                if self.capture.is_some() && self.layer_of(&surface).is_some_and(|l| l.role == Role::Capture) {
                    tracing::info!("the compositor took the keyboard back: the pointer comes home");
                    self.end_capture(true);
                }
            }
            K::Key { key, state, .. } => {
                let down = match state {
                    WEnum::Value(wl_keyboard::KeyState::Pressed) => true,
                    WEnum::Value(wl_keyboard::KeyState::Released) => false,
                    // `repeated` (version 10): senders drop repeats.
                    _ => return,
                };
                if self.capture.is_some() && self.pressed.update(key, down) {
                    self.send(Event::Key { code: key, down });
                }
            }
            K::Modifiers { mods_depressed, mods_latched, group, .. } => {
                self.group = group;
                if let Some(m) = &mut self.mods {
                    m.set_group(group);
                }
                if self.capture.is_some() {
                    self.send(Event::Modifiers(Mods::from_xkb(mods_depressed | mods_latched)));
                }
            }
            _ => {}
        }
    }

    fn on_seat_capabilities(&mut self, caps: wl_seat::Capability) {
        let qh = self.qh().clone();
        if caps.contains(wl_seat::Capability::Pointer) && self.pointer.is_none() {
            let p = self.s.seat.get_pointer(&qh, ());
            self.relative = Some(self.s.relative.get_relative_pointer(&p, &qh, ()));
            self.pointer = Some(p);
        } else if !caps.contains(wl_seat::Capability::Pointer)
            && let Some(p) = self.pointer.take()
        {
            if let Some(r) = self.relative.take() {
                r.destroy();
            }
            if p.version() >= 3 {
                p.release();
            }
            self.end_capture(true);
        }
        if caps.contains(wl_seat::Capability::Keyboard) && self.keyboard.is_none() {
            self.keyboard = Some(self.s.seat.get_keyboard(&qh, ()));
        } else if !caps.contains(wl_seat::Capability::Keyboard)
            && let Some(k) = self.keyboard.take()
            && k.version() >= 3
        {
            k.release();
        }
    }
}

/// A transparent buffer of `(w, h)` pixels, in a memfd of its own.
fn transparent_buffer(shm: &wl_shm::WlShm, (w, h): (u32, u32), qh: &QueueHandle<State>) -> Result<wl_buffer::WlBuffer> {
    let size = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)).context("buffer too large")?;
    let fd = memfd("one-mk-surface")?;
    // A new memfd reads as zeros: transparent ARGB.
    std::fs::File::from(fd.try_clone()?).set_len(size as u64)?;
    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

fn memfd(name: &str) -> Result<OwnedFd> {
    let name = std::ffi::CString::new(name)?;
    // SAFETY: a valid C string; the returned fd is ours.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: just created, owned by nobody else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The keymap text in a memfd, with its size (the terminating NUL included).
fn keymap_fd(keymap: &[u8]) -> Result<(OwnedFd, u32)> {
    use std::io::Write;
    let fd = memfd("one-mk-keymap")?;
    let mut f = std::fs::File::from(fd.try_clone()?);
    f.write_all(keymap)?;
    f.write_all(&[0])?;
    Ok((fd, keymap.len() as u32 + 1))
}

/// Reads a keymap the compositor sent, without the trailing NUL. Reads at
/// offsets: the file may be shared with other clients.
fn read_keymap(fd: &OwnedFd, size: usize) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let f = std::fs::File::from(fd.try_clone()?);
    let mut buf = vec![0; size];
    let mut done = 0;
    while done < size {
        match f.read_at(&mut buf[done..], done as u64)? {
            0 => break,
            n => done += n,
        }
    }
    buf.truncate(done);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    Ok(buf)
}

// ------------------------------------------------------------- dispatch

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } if interface == wl_output::WlOutput::interface().name => {
                state.add_output(name, version, qh);
            }
            wl_registry::Event::GlobalRemove { name } => state.remove_output(name),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = state.outputs.iter_mut().find(|o| o.name == *name) else { return };
        let g = &mut o.geometry;
        match event {
            wl_output::Event::Geometry { x, y, transform, .. } => {
                g.position = Some((x, y));
                g.transform = match transform {
                    WEnum::Value(t) => t as u32,
                    WEnum::Unknown(t) => t,
                };
            }
            wl_output::Event::Mode { flags, width, height, .. } => {
                if let WEnum::Value(f) = flags
                    && f.contains(wl_output::Mode::Current)
                {
                    g.mode = Some((width, height));
                }
            }
            wl_output::Event::Scale { factor } => g.scale = factor,
            wl_output::Event::Done => state.outputs_changed = true,
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = state.outputs.iter_mut().find(|o| o.name == *name) else { return };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => o.geometry.logical_position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => o.geometry.logical_size = Some((width, height)),
            zxdg_output_v1::Event::Done => state.outputs_changed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            state.on_seat_capabilities(caps);
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.on_pointer(event);
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.on_keyboard(event);
    }
}

impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        event: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion { dx, dy, .. } = event {
            state.on_relative(dx, dy);
        }
    }
}

impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for State {
    fn event(
        state: &mut Self,
        lock: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        event: zwp_locked_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A one-shot lock that ends is gone for good: so is the capture.
        if let zwp_locked_pointer_v1::Event::Unlocked = event
            && state.capture.as_ref().and_then(|c| c.lock.as_ref()) == Some(lock)
        {
            tracing::info!("the compositor ended the pointer lock: the pointer comes home");
            state.end_capture(true);
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                state.configure(*id, serial, width, height);
            }
            zwlr_layer_surface_v1::Event::Closed => {
                let Some(i) = state.layers.iter().position(|l| l.id == *id) else { return };
                if state.layers[i].role == Role::Capture {
                    state.end_capture(true);
                } else {
                    state.layers.remove(i).destroy();
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
delegate_noop!(State: ignore zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ignore zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);
delegate_noop!(State: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);
delegate_noop!(State: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);
delegate_noop!(State: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);
delegate_noop!(State: ignore wp_viewporter::WpViewporter);
delegate_noop!(State: ignore wp_viewport::WpViewport);
