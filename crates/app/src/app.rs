//! The window: a top bar, the desk, a panel of facts and switches under it,
//! and the pairing sheet. Everything shown comes from the running daemon;
//! everything changed goes through it.

use crate::daemon::{Event, Job, Latency, Link};
use crate::pairing::{Pairing, Step};
use crate::scene::{self, Cargo, Device, Kind, SIGNAL, SPACING, Scene, v};
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use one_mk_ctl::status::{Status, on_your, os_name, takeback_hint};
use one_mk_proto::{Side, os};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

pub const GROUND: Color32 = Color32::from_rgb(0x0F, 0x12, 0x16);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x17, 0x1C);
pub const RAISED: Color32 = Color32::from_rgb(0x17, 0x1B, 0x21);
pub const LINE: Color32 = Color32::from_rgb(0x25, 0x2B, 0x34);
pub const INK: Color32 = Color32::from_rgb(0xED, 0xEF, 0xF2);
pub const INK2: Color32 = Color32::from_rgb(0x9D, 0xA6, 0xB1);
pub const SIGNAL_INK: Color32 = Color32::from_rgb(0x1E, 0x15, 0x05);
const TOP_H: f32 = 56.0;
const PANEL_H: f32 = 132.0;
const GHOST: &str = "\u{0}pairing";
const WALLS: [[u32; 2]; 4] = [[0x3D3127, 0x1A1511], [0x22302A, 0x0F1613], [0x2E2638, 0x141019], [0x3A2A2A, 0x170F0F]];

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Desk,
    Files,
    Keyboard,
}
const TABS: [(Tab, &str); 3] = [(Tab::Desk, "Desk"), (Tab::Files, "Clipboard & files"), (Tab::Keyboard, "Keyboard")];

enum Drag {
    Device { i: usize, start: Pos2, x0: f32 },
    Orbit,
}

struct PairUi {
    proc: Option<Pairing>,
    step: Step,
    /// When the current step began (for staggered reveals).
    since: f64,
    hint: bool,
    retry: Option<String>,
    me: String,
}

struct Toast {
    text: String,
    error: bool,
    at: f64,
}

pub struct App {
    link: Link,
    rx: Receiver<Event>,
    /// `None` until the first look at the daemon.
    status: Option<Option<Status>>,
    autostart: Option<bool>,
    scene: Scene,
    tab: Tab,
    drag: Option<Drag>,
    menu_for: Option<usize>,
    pairing: Option<PairUi>,
    toast: Option<Toast>,
    latency: HashMap<String, Latency>,
    measuring: Option<String>,
    /// Sides chosen here and not yet reported back by the daemon.
    pending_side: HashMap<String, Side>,
    /// The screen that gets the mouse and keyboard right now.
    active: String,
    headline: (String, f64),
    busy: Vec<Job>,
    ctx: egui::Context,
}

fn me_kind() -> Kind {
    if cfg!(target_os = "macos") { Kind::Laptop } else { Kind::Monitor }
}

/// Room between neighbouring devices on the desk, in centimetres: enough
/// for the crossing to read as an arc.
const GAP: f32 = 26.0;

/// Where each computer stands, from the sides set in the layout. Each
/// entry is (id, side, half width); this computer is at the origin.
fn slots(me_half: f32, peers: &[(String, Side, f32)]) -> HashMap<String, (scene::V3, f32)> {
    let mut out = HashMap::new();
    let (mut left, mut right) = (-me_half, me_half);
    let mut count: HashMap<Side, usize> = HashMap::new();
    for (id, side, half) in peers {
        let n = count.entry(*side).or_default();
        *n += 1;
        let k = *n as f32;
        let place = match side {
            Side::Left => {
                left -= GAP + half;
                let x = left;
                left -= half;
                (v(x, 0.0, 0.0), 1.0)
            }
            Side::Right => {
                right += GAP + half;
                let x = right;
                right += half;
                (v(x, 0.0, 0.0), 1.0)
            }
            Side::Top => (v((k - 1.0) * SPACING, 34.0 + (k - 1.0) * 6.0, -26.0), 0.92),
            Side::Bottom => (v((k - 1.0) * SPACING, 0.0, 30.0), 0.8),
        };
        out.insert(id.clone(), place);
    }
    out
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, home: Option<PathBuf>) -> anyhow::Result<Self> {
        crate::style::install(&cc.egui_ctx);
        let (tx, rx) = std::sync::mpsc::channel();
        let link = Link::new(home, tx, cc.egui_ctx.clone())?;
        link.watch();
        let mut scene = Scene::default();
        let mut me = Device::new("me", "This computer", me_kind(), true, [0x1D2D3C, 0x0D151E]);
        me.alpha = 0.0;
        scene.devices.push(me);
        scene.cursor_on = "me".into();
        Ok(Self {
            link,
            rx,
            status: None,
            autostart: None,
            scene,
            tab: Tab::Desk,
            drag: None,
            menu_for: None,
            pairing: None,
            toast: None,
            latency: HashMap::new(),
            measuring: None,
            pending_side: HashMap::new(),
            active: "me".into(),
            headline: (String::new(), 0.0),
            busy: vec![],
            ctx: cc.egui_ctx.clone(),
        })
    }

    fn running(&self) -> Option<&Status> {
        self.status.as_ref().and_then(Option::as_ref)
    }

    fn toast(&mut self, text: impl Into<String>, error: bool, now: f64) {
        self.toast = Some(Toast { text: text.into(), error, at: now });
    }

    fn handle_events(&mut self, now: f64) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Status(s) => {
                    self.status = Some(s);
                    self.sync_scene();
                }
                Event::Autostart(on) => self.autostart = Some(on),
                Event::Latency(l) => {
                    self.measuring = None;
                    // Show the measurement where it was taken: one crossing, labelled.
                    self.scene.latency_label = Some(format!("{:.2} ms", l.p50_us / 2000.0));
                    if self.active == "me" && self.scene.flight.is_none() {
                        self.scene.fly("me", &l.peer, Cargo::Probe);
                    }
                    self.latency.insert(l.peer.clone(), l);
                }
                Event::Done { what, error } => {
                    self.busy.retain(|j| j != &what);
                    match (&what, error) {
                        (Job::Measure, Some(_)) => self.measuring = None,
                        (Job::Layout, Some(e)) => {
                            self.pending_side.clear();
                            self.sync_scene();
                            self.toast(format!("Could not change the layout: {e}"), true, now);
                        }
                        (Job::Send { to, count }, None) => {
                            let what = if *count == 1 { "1 item".to_owned() } else { format!("{count} items") };
                            self.toast(format!("Sent {what} to {to}"), false, now);
                        }
                        (_, Some(e)) => self.toast(e, true, now),
                        _ => {}
                    }
                }
            }
        }
        let mut finished: Option<(String, String)> = None;
        if let Some(p) = &mut self.pairing {
            while let Some(step) = p.proc.as_ref().and_then(|c| c.rx.try_recv().ok()) {
                match step {
                    Step::Searching { me } => p.me = me,
                    Step::Hint => p.hint = true,
                    Step::Retry(why) => {
                        p.retry = Some(why);
                        p.step = Step::Searching { me: p.me.clone() };
                        p.since = now;
                    }
                    Step::Paired { id, name } => finished = Some((id, name)),
                    Step::Ended(why) => {
                        p.proc = None;
                        p.step = Step::Ended(why);
                        p.since = now;
                    }
                    s => {
                        p.step = s;
                        p.since = now;
                    }
                }
            }
        }
        if let Some((id, name)) = finished {
            // The ghost becomes the new computer; the daemon picks it up on reload.
            if let Some(i) = self.scene.index(GHOST) {
                let d = &mut self.scene.devices[i];
                d.id = id;
                d.name = name.clone();
                d.alpha_to = 1.0;
            }
            self.scene.code = None;
            self.pairing = None;
            self.toast(format!("Paired with {name}"), false, now);
            let dir = self.link.dir.clone();
            std::thread::spawn(move || {
                let _ = one_mk_ctl::client::request(&dir, one_mk_ctl::client::Request::Reload, std::time::Duration::from_secs(3));
            });
            self.sync_scene();
        }
        self.sync_pairing_scene(now);
    }

    /// Makes the desk match what the daemon reports.
    fn sync_scene(&mut self) {
        let status = self.running().cloned();
        let peers: Vec<_> = status.as_ref().map(|s| s.peers.clone()).unwrap_or_default();
        // The daemon has caught up with a side chosen here.
        self.pending_side.retain(|id, side| peers.iter().any(|p| &p.id == id && p.side != *side));
        let half = |kind: Kind| if kind == Kind::Laptop { 15.5 } else { 29.0 };
        let mut layout: Vec<(String, Side, f32)> = peers
            .iter()
            .map(|p| {
                let kind = if p.os == os::MACOS { Kind::Laptop } else { Kind::Monitor };
                (p.id.clone(), *self.pending_side.get(&p.id).unwrap_or(&p.side), half(kind))
            })
            .collect();
        if self.scene.index(GHOST).is_some() {
            let right = layout.iter().filter(|(_, s, _)| *s == Side::Right).count();
            let left = layout.iter().filter(|(_, s, _)| *s == Side::Left).count();
            layout.push((GHOST.into(), if right <= left { Side::Right } else { Side::Left }, half(Kind::Monitor) * 0.86));
        }
        let places = slots(half(me_kind()), &layout);

        let me = &mut self.scene.devices[0];
        me.alpha_to = 1.0;
        me.wire_to = 1.0;
        if let Some(s) = &status {
            me.name = s.name.clone();
        }
        for (n, p) in peers.iter().enumerate() {
            let i = match self.scene.index(&p.id) {
                Some(i) => i,
                None => {
                    let kind = if p.os == os::MACOS { Kind::Laptop } else { Kind::Monitor };
                    let mut d = Device::new(&p.id, &p.name, kind, false, WALLS[n % WALLS.len()]);
                    let (pos, _) = places[&p.id];
                    d.pos = pos + v(0.0, 0.0, -8.0);
                    d.alpha = 0.0;
                    d.wire = 0.0;
                    self.scene.devices.push(d);
                    self.scene.devices.len() - 1
                }
            };
            let d = &mut self.scene.devices[i];
            d.name = p.name.clone();
            d.online = p.online;
            d.kind = if p.os == os::MACOS { Kind::Laptop } else if p.os == 0 { d.kind } else { Kind::Monitor };
            d.leaving = false;
            d.alpha_to = 1.0;
            d.wire_to = 1.0;
        }
        for d in self.scene.devices.iter_mut().skip(1) {
            if d.id != GHOST && !peers.iter().any(|p| p.id == d.id) {
                d.leaving = true;
                d.alpha_to = 0.0;
                d.wire_to = 0.0;
            }
            if let Some((pos, scale)) = places.get(&d.id) {
                d.target = *pos;
                d.scale = *scale;
            }
        }

        // Follow the mouse and keyboard to whichever screen has them.
        let active = match &status {
            Some(Status { controlling: Some(name), .. }) => {
                peers.iter().find(|p| &p.name == name).map_or("me".to_owned(), |p| p.id.clone())
            }
            _ => "me".to_owned(),
        };
        if active != self.active {
            let from = std::mem::replace(&mut self.active, active.clone());
            self.scene.fly(&from, &active, Cargo::Pointer);
        }
    }

    /// The pairing ghost and the code on screens.
    fn sync_pairing_scene(&mut self, now: f64) {
        let Some(step) = self.pairing.as_ref().map(|p| (p.step.clone(), p.since)) else {
            if let Some(i) = self.scene.index(GHOST) {
                let d = &mut self.scene.devices[i];
                d.leaving = true;
                d.alpha_to = 0.0;
                d.wire_to = 0.0;
            }
            self.scene.code = None;
            return;
        };
        let (step, since) = step;
        if self.scene.index(GHOST).is_none() && !matches!(step, Step::Ended(_)) {
            let mut g = Device::new(GHOST, "Looking…", Kind::Monitor, false, WALLS[1]);
            g.alpha = 0.0;
            g.alpha_to = 0.0;
            g.wire = 0.0;
            g.wire_to = 1.0;
            g.scale = 0.86;
            self.scene.devices.push(g);
            self.sync_scene();
            let i = self.scene.index(GHOST).unwrap();
            let t = self.scene.devices[i].target;
            self.scene.devices[i].pos = t;
        }
        if let Some(i) = self.scene.index(GHOST) {
            let d = &mut self.scene.devices[i];
            match &step {
                Step::Code { name, code } => {
                    d.name = name.clone();
                    if self.scene.code.as_ref().is_none_or(|(_, c, _)| c != code) {
                        self.scene.code = Some((vec!["me".into(), GHOST.into()], code.clone(), since));
                    }
                }
                Step::Ended(_) => {
                    d.leaving = true;
                    d.wire_to = 0.0;
                    self.scene.code = None;
                }
                Step::Waiting { .. } => {}
                _ => {
                    d.name = "Looking…".into();
                    self.scene.code = None;
                    // A slow breathing outline while searching.
                    d.wire_to = 0.55 + 0.45 * ((now * 1.6).sin() as f32 * 0.5 + 0.5);
                }
            }
        }
    }

    fn start_pairing(&mut self, now: f64) {
        match Pairing::start(&self.link.daemon, &self.link.home_args(), self.link_ctx()) {
            Ok(proc) => {
                let me = self.running().map(|s| s.name.clone()).unwrap_or_default();
                self.pairing = Some(PairUi { proc: Some(proc), step: Step::Searching { me: me.clone() }, since: now, hint: false, retry: None, me });
            }
            Err(e) => self.toast(format!("Could not start pairing: {e}"), true, now),
        }
    }

    fn link_ctx(&self) -> egui::Context {
        self.ctx.clone()
    }
}

// ---------- drawing helpers ----------

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// Overshoots a little and settles: for switches and toggles.
fn back_out(t: f32) -> f32 {
    let (c1, t) = (1.55, t.clamp(0.0, 1.0) - 1.0);
    1.0 + (c1 + 1.0) * t * t * t + c1 * t * t
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.into(), font, INK).size().x
}

fn button_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, semibold(13.0)) + 28.0
}

/// A pill button that presses in a little.
fn button(ui: &mut Ui, rect: Rect, id: Id, label: &str, primary: bool) -> egui::Response {
    let r = ui.interact(rect, id, Sense::click());
    let ctx = ui.ctx().clone();
    let press = ctx.animate_bool_with_time(id.with("press"), r.is_pointer_button_down_on(), 0.1);
    let hover = ctx.animate_bool_with_time(id.with("hover"), r.hovered(), 0.15);
    let rr = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.05 * press));
    let (bg, lit, fg) = if primary {
        (SIGNAL, Color32::from_rgb(0xFF, 0xC2, 0x66), SIGNAL_INK)
    } else {
        (Color32::from_rgb(0x22, 0x28, 0x31), Color32::from_rgb(0x2A, 0x31, 0x3B), INK)
    };
    let p = ui.painter();
    p.rect_filled(rr, CornerRadius::same(9), mix(bg, lit, hover));
    if r.has_focus() {
        p.rect_stroke(rr.expand(2.0), CornerRadius::same(11), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
    }
    p.text(rr.center(), Align2::CENTER_CENTER, label, semibold(13.0 * (1.0 - 0.05 * press)), fg);
    if r.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r
}

/// A text link: quiet until hovered.
fn link(ui: &mut Ui, pos: Pos2, id: Id, label: &str) -> egui::Response {
    let w = text_width(ui, label, regular(12.5));
    let rect = Rect::from_min_size(pos, vec2(w, 18.0));
    let r = ui.interact(rect, id, Sense::click());
    let col = if r.hovered() { SIGNAL } else { INK };
    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, regular(12.5), col);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, mix(SIGNAL, Color32::TRANSPARENT, 0.5)));
    }
    r
}

/// A labelled switch across `row`; true when clicked.
fn switch(ui: &mut Ui, row: Rect, id: Id, label: &str, on: bool, enabled: bool) -> bool {
    let r = ui.interact(row, id, if enabled { Sense::click() } else { Sense::hover() });
    let t = ui.ctx().animate_bool_with_time(id.with("knob"), on, 0.42);
    let e = if on { back_out(t) } else { 1.0 - back_out(1.0 - t) };
    let p = ui.painter();
    p.text(row.left_center(), Align2::LEFT_CENTER, label, regular(13.0), if enabled { INK } else { INK2 });
    let track = Rect::from_min_size(pos2(row.right() - 34.0, row.center().y - 10.0), vec2(34.0, 20.0));
    p.rect_filled(track, CornerRadius::same(10), mix(Color32::from_rgb(0x2A, 0x30, 0x3A), SIGNAL, t));
    let knob = pos2(track.left() + 10.0 + 14.0 * e, track.center().y);
    p.circle_filled(knob, 8.0, mix(Color32::from_rgb(0xC9, 0xCE, 0xD6), SIGNAL_INK, t));
    if r.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    enabled && r.clicked()
}

/// Wrapped text; returns its height.
fn para(ui: &Ui, pos: Pos2, width: f32, text: &str, font: FontId, col: Color32) -> f32 {
    let g = ui.painter().layout(text.into(), font, col, width);
    let h = g.size().y;
    ui.painter().galley(pos, g, col);
    h
}

impl App {
    fn draggable(&self, i: usize) -> bool {
        let d = &self.scene.devices[i];
        i != 0 && d.id != GHOST && !d.leaving
    }

    fn side_of(&self, id: &str) -> Option<Side> {
        self.pending_side.get(id).copied().or_else(|| self.running()?.peers.iter().find(|p| p.id == id).map(|p| p.side))
    }

    fn choose_side(&mut self, id: &str, side: Side) {
        if self.side_of(id) != Some(side) {
            self.pending_side.insert(id.to_owned(), side);
            self.busy.push(Job::Layout);
            self.link.set_side(id, side);
        }
        self.sync_scene();
    }

    /// A computer was let go after dragging: it goes to the side it is on.
    fn dropped_device(&mut self, i: usize) {
        let d = &self.scene.devices[i];
        let id = d.id.clone();
        let dx = d.pos.x - self.scene.devices[0].pos.x;
        let moved = (d.pos.x - d.target.x).abs();
        let current = self.side_of(&id);
        let side = match current {
            Some(Side::Top | Side::Bottom) if moved < 20.0 => current.unwrap(),
            _ if dx < 0.0 => Side::Left,
            _ => Side::Right,
        };
        self.choose_side(&id, side);
    }

    fn measure(&mut self, id: &str) {
        if self.measuring.is_none() {
            self.measuring = Some(id.to_owned());
            self.link.measure(id);
        }
    }

    fn drop_files(&mut self, paths: Vec<PathBuf>, at: Option<Pos2>, now: f64) {
        let online: Vec<usize> = (1..self.scene.devices.len()).filter(|&i| self.draggable(i) && self.scene.devices[i].online).collect();
        let target = at.and_then(|p| self.scene.hit(p)).filter(|i| online.contains(i)).or(if online.len() == 1 { Some(online[0]) } else { None });
        let Some(i) = target else {
            let msg = if online.is_empty() { "Connect a computer first, then drop files on it." } else { "Drop the files onto the computer you want to send them to." };
            self.toast(msg, true, now);
            return;
        };
        let (id, name) = (self.scene.devices[i].id.clone(), self.scene.devices[i].name.clone());
        let n = paths.len();
        self.busy.push(Job::Send { to: name.clone(), count: n });
        self.link.send_files(&id, &name, paths);
        self.scene.fly(&self.active.clone(), &id, Cargo::Parcel);
        let what = if n == 1 { "1 item".to_owned() } else { format!("{n} items") };
        self.toast(format!("Sending {what} to {name}…"), false, now);
    }

    // ---------- the desk ----------

    fn desk(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let resp = ui.interact(rect, Id::unique("desk"), Sense::click_and_drag());
        let pointer = resp.hover_pos();
        let hovering_files = ui.input(|i| !i.raw.hovered_files.is_empty());
        self.scene.drop_targets = hovering_files;
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            let at = ui.input(|i| i.pointer.latest_pos());
            self.drop_files(dropped, at, now);
        }

        if resp.drag_started() {
            let start = resp.interact_pointer_pos().unwrap_or_default();
            self.drag = match self.scene.hit(start) {
                Some(i) if self.draggable(i) => Some(Drag::Device { i, start, x0: self.scene.devices[i].pos.x }),
                _ => Some(Drag::Orbit),
            };
        }
        if resp.dragged() {
            match &self.drag {
                Some(Drag::Device { i, start, x0 }) => {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let ax = self.scene.x_axis(rect, *i);
                        let d = p - *start;
                        let dx = (d.x * ax.x + d.y * ax.y) / ax.length_sq().max(1e-3);
                        let me_x = self.scene.devices[0].pos.x;
                        self.scene.devices[*i].pos.x = (x0 + dx).clamp(me_x - SPACING * 3.0, me_x + SPACING * 3.0);
                    }
                }
                Some(Drag::Orbit) => {
                    let d = resp.drag_delta();
                    self.scene.yaw_to = (self.scene.yaw_to - d.x * 0.006).clamp(-0.75, 0.75);
                    self.scene.pitch_to = (self.scene.pitch_to + d.y * 0.004).clamp(0.2, 0.78);
                }
                None => {}
            }
        }
        if resp.drag_stopped() {
            if let Some(Drag::Device { i, .. }) = self.drag {
                self.dropped_device(i);
            }
            self.drag = None;
        }

        let under = pointer.and_then(|p| self.scene.hit(p));
        self.scene.hover = match self.drag {
            Some(Drag::Device { i, .. }) => Some(i),
            Some(Drag::Orbit) => None,
            None => under.filter(|&i| self.draggable(i)),
        };
        match self.drag {
            Some(_) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            None if self.scene.hover.is_some() => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            None => {}
        }
        if resp.clicked()
            && let Some(i) = under.filter(|&i| self.draggable(i) && self.scene.devices[i].online)
        {
            let id = self.scene.devices[i].id.clone();
            self.measure(&id);
        }
        if resp.secondary_clicked() {
            self.menu_for = under.filter(|&i| self.draggable(i));
        }
        resp.context_menu(|ui| {
            let Some(i) = self.menu_for.filter(|&i| i < self.scene.devices.len()) else {
                ui.close();
                return;
            };
            let (id, name) = (self.scene.devices[i].id.clone(), self.scene.devices[i].name.clone());
            ui.label(egui::RichText::new(format!("Where is {name}?")).color(INK2).size(12.0));
            let current = self.side_of(&id);
            for (side, label) in [(Side::Left, "On the left"), (Side::Right, "On the right"), (Side::Top, "Above"), (Side::Bottom, "Below")] {
                if ui.radio(current == Some(side), label).clicked() {
                    self.choose_side(&id, side);
                    ui.close();
                }
            }
        });

        let dragging = match self.drag {
            Some(Drag::Device { i, .. }) => Some(i),
            _ => None,
        };
        self.scene.raise_to = if self.pairing.is_some() { 1.0 } else { 0.0 };
        let moving = self.scene.step(now, dragging);
        let painter = ui.painter_at(rect);
        self.scene.paint(&painter, rect, now);
        if moving || self.drag.is_some() || self.pairing.is_some() {
            ui.ctx().request_repaint();
        }
        if hovering_files {
            let p = pos2(rect.center().x, rect.bottom() - 26.0);
            painter.text(p, Align2::CENTER_CENTER, "Drop on a computer to send", semibold(14.0), SIGNAL);
        }
    }

    // ---------- the top bar ----------

    fn top_bar(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let p = ui.painter_at(rect);
        let mut m = egui::Mesh::default();
        let fade = Color32::from_rgba_premultiplied(0, 0, 0, 0);
        for (pt, c) in [(rect.left_top(), GROUND), (rect.right_top(), GROUND), (rect.right_bottom(), fade), (rect.left_bottom(), fade)] {
            m.colored_vertex(pt, c);
        }
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        p.add(egui::Shape::mesh(m));

        // The mark: this screen, and the next one lit.
        let o = pos2(rect.left() + 16.0, rect.center().y - 11.0);
        let s = 22.0 / 24.0;
        p.rect_stroke(
            Rect::from_min_size(o + vec2(2.5, 5.0) * s, vec2(11.0, 8.0) * s),
            CornerRadius::same(2),
            Stroke::new(1.8, INK),
            StrokeKind::Middle,
        );
        p.rect_filled(Rect::from_min_size(o + vec2(10.5, 11.0) * s, vec2(11.0, 8.0) * s), CornerRadius::same(2), SIGNAL);
        p.text(pos2(o.x + 30.0, rect.center().y), Align2::LEFT_CENTER, "one·mk", semibold(16.0), INK);

        // Tabs, with an indicator that slides between them.
        let widths: Vec<f32> = TABS.iter().map(|(_, l)| text_width(ui, l, regular(13.0)) + 24.0).collect();
        let total = widths.iter().sum::<f32>() + 2.0 * (TABS.len() - 1) as f32 + 6.0;
        let bar = Rect::from_center_size(rect.center(), vec2(total, 34.0));
        p.rect(bar, CornerRadius::same(10), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let mut x = bar.left() + 3.0;
        let mut target = (0.0, 0.0);
        let mut cells = vec![];
        for ((tab, _), w) in TABS.iter().zip(&widths) {
            let r = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(*w, 28.0));
            if *tab == self.tab {
                target = (r.left(), r.width());
            }
            cells.push(r);
            x += w + 2.0;
        }
        let ctx = ui.ctx().clone();
        let ix = ctx.animate_value_with_time(Id::unique("tab-x"), target.0, 0.32);
        let iw = ctx.animate_value_with_time(Id::unique("tab-w"), target.1, 0.32);
        p.rect_filled(Rect::from_min_size(pos2(ix, bar.top() + 3.0), vec2(iw, 28.0)), CornerRadius::same(7), Color32::from_rgb(0x26, 0x2C, 0x36));
        for (((tab, label), r), i) in TABS.iter().zip(&cells).zip(0..) {
            let resp = ui.interact(*r, Id::unique(("tab", i)), Sense::click());
            if resp.clicked() {
                self.tab = *tab;
                self.headline.1 = now;
            }
            let on = ctx.animate_bool_with_time(Id::unique(("tab-on", i)), *tab == self.tab, 0.25);
            let col = mix(if resp.hovered() { Color32::from_rgb(0xC4, 0xCA, 0xD2) } else { INK2 }, INK, on);
            p.text(r.center(), Align2::CENTER_CENTER, *label, regular(13.0), col);
            if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }

        // The one action that matters most right now.
        let (label, primary) = match (self.status.as_ref(), &self.pairing) {
            (Some(None), _) => ("Start one·mk", true),
            (_, Some(_)) => ("Cancel pairing", false),
            _ if self.running().is_some_and(|s| !s.peers.is_empty()) => ("Pair another", true),
            _ => ("Pair a device", true),
        };
        let w = button_width(ui, label);
        let r = Rect::from_min_size(pos2(rect.right() - 16.0 - w, rect.center().y - 16.0), vec2(w, 32.0));
        if self.status.is_some() && button(ui, r, Id::unique("action"), label, primary).clicked() {
            match (self.status.as_ref(), self.pairing.is_some()) {
                (Some(None), _) => {
                    if !self.busy.contains(&Job::Start) {
                        self.busy.push(Job::Start);
                        self.link.start();
                    }
                }
                (_, true) => self.pairing = None,
                _ => self.start_pairing(now),
            }
        }
    }

    // ---------- the panel ----------

    fn headline_text(&self) -> (String, String) {
        let Some(s) = self.status.as_ref() else { return ("Looking for one·mk…".into(), String::new()) };
        let Some(s) = s else {
            return ("one·mk isn't running".into(), "Start it to share this computer's mouse, keyboard, clipboard and files.".into());
        };
        let arrange = "Drag a computer to where it really is. Right-click it for above or below.";
        if let Some(p) = &s.controlled_by {
            return (format!("{p} is controlling this computer"), takeback_hint().replace("`one-mk takeback`", "Take back"));
        }
        if let Some(p) = &s.controlling {
            return (format!("Controlling {p}"), "Move the pointer back across the lit edge to return.".into());
        }
        let online: Vec<_> = s.peers.iter().filter(|p| p.online).collect();
        match online[..] {
            [] if s.peers.is_empty() => ("No computers paired yet".into(), "Choose Pair a device here and on the other computer.".into()),
            [] => (
                "Waiting for your computers".into(),
                "Paired computers appear here when one·mk is running on them and they're on the same network.".into(),
            ),
            [p] => (format!("{} is {}", p.name, on_your(*self.pending_side.get(&p.id).unwrap_or(&p.side))), arrange.into()),
            _ => (format!("Connected to {} computers", online.len()), arrange.into()),
        }
    }

    fn panel(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, PANEL);
        p.hline(rect.x_range(), rect.top(), Stroke::new(1.0, LINE));
        let w = rect.width();
        let cols = [0.0, w * 0.4, w * 0.7, w];
        // The keyboard tab uses the last two cells as one.
        let seps = if self.tab == Tab::Keyboard { &cols[1..2] } else { &cols[1..3] };
        for x in seps {
            p.vline(rect.left() + x, rect.y_range(), Stroke::new(1.0, LINE));
        }
        let cell = |k: usize| Rect::from_min_max(pos2(rect.left() + cols[k] + 18.0, rect.top() + 16.0), pos2(rect.left() + cols[k + 1] - 18.0, rect.bottom() - 14.0));

        // Content slides in a few pixels when it changes.
        let (title, sub) = match self.tab {
            Tab::Desk => self.headline_text(),
            Tab::Files => (
                "Drop files on a computer".into(),
                "Drag files or folders from this computer onto another one on the desk. They arrive in Downloads/one-mk there.".into(),
            ),
            Tab::Keyboard => (
                "Ctrl and Cmd".into(),
                "Swap them for keys sent to another computer, so shortcuts like copy and paste use the same keys everywhere. Auto swaps only between a Mac and a PC.".into(),
            ),
        };
        if self.headline.0 != title {
            self.headline = (title.clone(), now);
        }
        let k = scene::ease_out(((now - self.headline.1) / 0.35) as f32);
        if k < 1.0 {
            ui.ctx().request_repaint();
        }
        let c1 = cell(0);
        let alpha = |c: Color32| c.gamma_multiply(k);
        p.text(c1.left_top() + vec2(0.0, 6.0 * (1.0 - k)), Align2::LEFT_TOP, &title, semibold(14.5), alpha(INK));
        let h = para(ui, c1.left_top() + vec2(0.0, 24.0 + 6.0 * (1.0 - k)), c1.width(), &sub, regular(12.5), alpha(INK2));

        match self.tab {
            Tab::Desk => {
                // Starting one·mk is the top bar's button; taking back lives here.
                if self.running().is_some_and(|s| s.controlled_by.is_some()) {
                    let label = "Take back";
                    let r = Rect::from_min_size(pos2(c1.left(), (c1.top() + 30.0 + h).min(c1.bottom() - 30.0)), vec2(button_width(ui, label), 30.0));
                    if button(ui, r, Id::unique("panel-action"), label, true).clicked() {
                        self.link.take_back();
                    }
                }
                self.metrics(ui, cell(1));
                self.switches(ui, cell(2));
            }
            Tab::Files => {
                let c = cell(1);
                p.text(c.left_top(), Align2::LEFT_TOP, "Clipboard", semibold(13.5), INK);
                para(ui, c.left_top() + vec2(0.0, 22.0), c.width(), "Copy on one computer, paste on another. It follows the pointer.", regular(12.5), INK2);
                let c = cell(2);
                p.text(c.left_top(), Align2::LEFT_TOP, "Received files", semibold(13.5), INK);
                if link(ui, c.left_top() + vec2(0.0, 24.0), Id::unique("open-dl"), "Open the folder").clicked() {
                    self.link.open_downloads();
                }
                if let Some(Job::Send { to, count }) = self.busy.iter().find(|j| matches!(j, Job::Send { .. })) {
                    let what = if *count == 1 { "1 item".to_owned() } else { format!("{count} items") };
                    p.text(c.left_top() + vec2(0.0, 52.0), Align2::LEFT_TOP, format!("Sending {what} to {to}…"), regular(12.5), SIGNAL);
                }
            }
            Tab::Keyboard => self.swaps(ui, Rect::from_min_max(cell(1).min, cell(2).max)),
        }
    }

    fn metrics(&mut self, ui: &mut Ui, c: Rect) {
        let p = ui.painter();
        let target = self.running().and_then(|s| {
            let online: Vec<_> = s.peers.iter().filter(|p| p.online).collect();
            online.iter().find(|p| Some(&p.id) == self.measuring.as_ref() || self.latency.contains_key(&p.id)).or(online.first()).map(|p| (p.id.clone(), p.name.clone()))
        });
        let Some((id, name)) = target else {
            p.text(c.left_top(), Align2::LEFT_TOP, "Connection", semibold(13.5), INK);
            para(ui, c.left_top() + vec2(0.0, 22.0), c.width(), "Speed shows here once a computer is connected.", regular(12.5), INK2);
            return;
        };
        // Measure each computer once when it first shows up.
        if !self.latency.contains_key(&id) && self.measuring.is_none() && !self.busy.contains(&Job::Measure) {
            self.busy.push(Job::Measure);
            self.measure(&id);
        }
        p.text(c.left_top(), Align2::LEFT_TOP, format!("To {name}"), semibold(13.5), INK);
        let row = |i: usize, k: &str, val: String| {
            let y = c.top() + 26.0 + i as f32 * 20.0;
            p.text(pos2(c.left(), y), Align2::LEFT_TOP, k, regular(12.5), INK2);
            p.text(pos2(c.right(), y), Align2::RIGHT_TOP, val, mono(12.5), INK);
        };
        match self.latency.get(&id) {
            Some(l) => {
                row(0, "Handoff, typical", format!("{:.2} ms", l.p50_us / 2000.0));
                row(1, "Slowest 1%", format!("{:.2} ms", l.p99_us / 2000.0));
                row(2, "Lost", format!("{} of {}", l.lost, l.count));
            }
            None => row(0, "Measuring…", String::new()),
        }
        if self.latency.contains_key(&id)
            && self.measuring.is_none()
            && link(ui, pos2(c.left(), c.bottom() - 16.0), Id::unique("measure"), "Measure again").clicked()
        {
            self.measure(&id);
        }
    }

    fn switches(&mut self, ui: &mut Ui, c: Rect) {
        let on = self.autostart.unwrap_or(false);
        let busy = self.busy.contains(&Job::Autostart);
        let row = Rect::from_min_size(c.left_top(), vec2(c.width(), 24.0));
        if switch(ui, row, Id::unique("autostart"), "Start at login", on, self.autostart.is_some() && !busy) {
            self.busy.push(Job::Autostart);
            self.autostart = Some(!on);
            self.link.set_autostart(!on);
        }
        if link(ui, c.left_top() + vec2(0.0, 40.0), Id::unique("open-dl2"), "Open received files").clicked() {
            self.link.open_downloads();
        }
    }

    fn swaps(&mut self, ui: &mut Ui, c: Rect) {
        let peers: Vec<_> = self.running().map(|s| s.peers.clone()).unwrap_or_default();
        if peers.is_empty() {
            ui.painter().text(c.left_top(), Align2::LEFT_TOP, "Pair a computer to choose this for it.", regular(12.5), INK2);
            return;
        }
        for (i, peer) in peers.iter().take(4).enumerate() {
            let y = c.top() + i as f32 * 26.0;
            let p = ui.painter();
            p.text(pos2(c.left(), y + 11.0), Align2::LEFT_CENTER, &peer.name, regular(13.0), INK);
            let nw = text_width(ui, &peer.name, regular(13.0));
            ui.painter().text(pos2(c.left() + nw + 8.0, y + 11.0), Align2::LEFT_CENTER, os_name(peer.os), regular(12.0), INK2);
            let opts = [("auto", "Auto"), ("on", "Swap"), ("off", "Don't")];
            let seg_w = 58.0;
            let bar = Rect::from_min_size(pos2(c.right() - seg_w * 3.0 - 6.0, y), vec2(seg_w * 3.0 + 6.0, 22.0));
            ui.painter().rect(bar, CornerRadius::same(7), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
            let sel = opts.iter().position(|(v, _)| *v == peer.swap).unwrap_or(0);
            let ctx = ui.ctx().clone();
            let x = ctx.animate_value_with_time(Id::unique(("swap-x", &peer.id)), bar.left() + 3.0 + sel as f32 * seg_w, 0.3);
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(seg_w, 18.0)), CornerRadius::same(5), Color32::from_rgb(0x26, 0x2C, 0x36));
            for (k, (val, label)) in opts.iter().enumerate() {
                let r = Rect::from_min_size(pos2(bar.left() + 3.0 + k as f32 * seg_w, bar.top() + 2.0), vec2(seg_w, 18.0));
                let resp = ui.interact(r, Id::unique(("swap", &peer.id, k)), Sense::click());
                let col = if k == sel { INK } else if resp.hovered() { Color32::from_rgb(0xC4, 0xCA, 0xD2) } else { INK2 };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, regular(12.0), col);
                if resp.clicked() && k != sel {
                    self.link.set_swap(&peer.id, val);
                }
            }
        }
    }

    // ---------- the pairing sheet ----------

    fn sheet(&mut self, ui: &mut Ui, desk: Rect, now: f64) {
        let open = ui.ctx().animate_bool_with_time(Id::unique("sheet"), self.pairing.is_some(), 0.3);
        let Some(pair) = &self.pairing else { return };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pairing = None;
            return;
        }
        let width = 380.0;
        let inner = width - 36.0;
        let (title, sub): (String, String) = match &pair.step {
            Step::Searching { .. } | Step::Hint => {
                let me = if pair.me.is_empty() { String::new() } else { format!(" This one appears as “{}”.", pair.me) };
                let mut sub = format!("On the other computer, open one·mk and choose Pair a device.{me}");
                if let Some(why) = &pair.retry {
                    sub += &format!("\n\nThe last try didn't finish ({why}). Still looking.");
                } else if pair.hint {
                    sub += "\n\nNothing yet. Both computers need to be on the same network, and a firewall must allow one·mk.";
                }
                ("Looking for another computer".into(), sub)
            }
            Step::Found => ("Found a computer".into(), "Agreeing on a code…".into()),
            Step::Code { name, .. } => (format!("Found “{name}”"), "Check that its screen shows the same code, then confirm.".into()),
            Step::Waiting { name } => (format!("Waiting for {name}"), "Confirm the code on the other computer too.".into()),
            Step::Ended(why) => ("Not paired".into(), why.clone()),
            Step::Paired { .. } | Step::Retry(_) => (String::new(), String::new()),
        };
        let sub_g = ui.painter().layout(sub.clone(), regular(12.5), INK2, inner);
        let code_h = if matches!(pair.step, Step::Code { .. }) { 56.0 } else { 0.0 };
        let height = 16.0 + 22.0 + sub_g.size().y + code_h + 14.0 + 32.0 + 16.0;
        let lift = 16.0 * (1.0 - scene::ease_out(open));
        let rect = Rect::from_min_size(pos2(desk.center().x - width / 2.0, desk.bottom() - height - 20.0 + lift), vec2(width, height));
        let p = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Foreground, Id::unique("sheet-layer")));
        let a = open;
        p.add(egui::epaint::Shadow { offset: [0, 18], blur: 40, spread: 0, color: Color32::from_black_alpha((115.0 * a) as u8) }.as_shape(rect, CornerRadius::same(14)));
        p.rect(rect, CornerRadius::same(14), RAISED.gamma_multiply(a), Stroke::new(1.0, LINE.gamma_multiply(a)), StrokeKind::Inside);
        let mut y = rect.top() + 16.0;
        let x = rect.left() + 18.0;
        p.text(pos2(x, y), Align2::LEFT_TOP, &title, semibold(15.0), INK.gamma_multiply(a));
        y += 22.0;
        p.galley(pos2(x, y), sub_g.clone(), INK2.gamma_multiply(a));
        y += sub_g.size().y;
        if let Step::Code { code, .. } = &pair.step {
            // Digits arrive one by one: easy to compare in the same rhythm on both screens.
            let mut cx = x;
            for (i, ch) in code.chars().enumerate() {
                if i == 3 {
                    cx += 14.0;
                }
                let t = (((now - pair.since - 0.3 - i as f64 * 0.07) / 0.45) as f32).clamp(0.0, 1.0);
                let e = scene::ease_out(t);
                p.text(pos2(cx, y + 12.0 + 10.0 * (1.0 - e)), Align2::LEFT_TOP, ch.to_string(), mono(30.0), INK.gamma_multiply(e * a));
                cx += 24.0;
            }
            if now - pair.since < 1.5 {
                ui.ctx().request_repaint();
            }
            y += code_h;
        }
        y += 14.0;
        // Buttons, right-aligned.
        let buttons: Vec<(&str, bool, u8)> = match &pair.step {
            Step::Code { .. } => vec![("They don't match", false, 1), ("Codes match, pair", true, 2)],
            Step::Ended(_) => vec![("Close", false, 0), ("Try again", true, 3)],
            _ => vec![("Cancel", false, 0)],
        };
        let mut bx = rect.right() - 18.0;
        let mut clicked = None;
        let child = &mut ui.new_child(egui::UiBuilder::new().layer_id(p.layer_id()).max_rect(rect));
        for (label, primary, what) in buttons.iter().rev() {
            let bw = button_width(child, label);
            bx -= bw;
            let r = Rect::from_min_size(pos2(bx, y), vec2(bw, 32.0));
            if button(child, r, Id::unique(("sheet-btn", *what)), label, *primary).clicked() {
                clicked = Some(*what);
            }
            bx -= 8.0;
        }
        match clicked {
            Some(0) => self.pairing = None,
            Some(1) => {
                if let Some(pr) = &mut self.pairing
                    && let Some(proc) = &mut pr.proc
                {
                    proc.answer(false);
                }
            }
            Some(2) => {
                if let Some(pr) = &mut self.pairing {
                    if let Some(proc) = &mut pr.proc {
                        proc.answer(true);
                    }
                    if let Step::Code { name, .. } = &pr.step {
                        pr.step = Step::Waiting { name: name.clone() };
                        pr.since = now;
                    }
                }
            }
            Some(3) => {
                self.pairing = None;
                self.start_pairing(now);
            }
            _ => {}
        }
    }

    fn toast_view(&mut self, ui: &mut Ui, desk: Rect, now: f64) {
        let Some(t) = &self.toast else { return };
        let age = (now - t.at) as f32;
        let life = if t.error { 6.0 } else { 3.5 };
        if age > life {
            self.toast = None;
            return;
        }
        ui.ctx().request_repaint();
        let a = scene::ease_out(age / 0.25).min(((life - age) / 0.4).clamp(0.0, 1.0));
        let g = ui.painter().layout(t.text.clone(), regular(13.0), INK, 520.0);
        let size = g.size() + vec2(40.0, 18.0);
        let rect = Rect::from_center_size(pos2(desk.center().x, desk.top() + TOP_H + 14.0 + size.y / 2.0 - 8.0 * (1.0 - a)), size);
        let p = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Tooltip, Id::unique("toast")));
        p.rect(rect, CornerRadius::same(10), RAISED.gamma_multiply(a), Stroke::new(1.0, LINE.gamma_multiply(a)), StrokeKind::Inside);
        let dot = if t.error { Color32::from_rgb(0xFF, 0x6B, 0x5E) } else { SIGNAL };
        p.circle_filled(pos2(rect.left() + 16.0, rect.center().y), 3.5, dot.gamma_multiply(a));
        p.galley(pos2(rect.left() + 28.0, rect.top() + 9.0), g, INK.gamma_multiply(a));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let now = ui.input(|i| i.time);
        #[cfg(debug_assertions)]
        if self.pairing.is_none() && now < 0.5 && self.status.is_some() && std::env::var_os("ONE_MK_APP_PAIR").is_some() {
            self.start_pairing(now);
        }
        #[cfg(debug_assertions)]
        match std::env::var("ONE_MK_APP_TAB").as_deref() {
            Ok("files") => self.tab = Tab::Files,
            Ok("keyboard") => self.tab = Tab::Keyboard,
            _ => {}
        }
        self.handle_events(now);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, GROUND);
        let panel = Rect::from_min_max(pos2(full.left(), full.bottom() - PANEL_H), full.max);
        let desk = Rect::from_min_max(full.min, pos2(full.right(), panel.top()));
        let top = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
        self.desk(ui, desk, now);
        self.top_bar(ui, top, now);
        self.panel(ui, panel, now);
        self.sheet(ui, desk, now);
        self.toast_view(ui, desk, now);
        #[cfg(debug_assertions)]
        shot::maybe(ui.ctx(), now);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        GROUND.to_normalized_gamma_f32()
    }
}

/// Development aid: `ONE_MK_APP_SHOT=file.ppm` saves the window after
/// `ONE_MK_APP_SHOT_AFTER` seconds (default 3) and quits.
#[cfg(debug_assertions)]
mod shot {
    use std::io::Write;

    pub fn maybe(ctx: &egui::Context, now: f64) {
        let Some(path) = std::env::var_os("ONE_MK_APP_SHOT") else { return };
        let after: f64 = std::env::var("ONE_MK_APP_SHOT_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(3.0);
        let asked = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("shot-asked")).unwrap_or(false));
        ctx.request_repaint();
        if now < after || asked {
            return;
        }
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("shot-asked"), true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let mut f = std::fs::File::create(&path).expect("shot file");
            let _ = writeln!(f, "P6 {} {} 255", img.size[0], img.size[1]);
            let rgb: Vec<u8> = img.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b()]).collect();
            let _ = f.write_all(&rgb);
            std::process::exit(0);
        })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_become_places() {
        let s = slots(29.0, &[("a".into(), Side::Left, 15.5), ("b".into(), Side::Right, 29.0), ("c".into(), Side::Left, 29.0), ("d".into(), Side::Top, 29.0)]);
        assert_eq!(s["a"].0.x, -(29.0 + GAP + 15.5));
        assert_eq!(s["c"].0.x, -(29.0 + GAP + 31.0 + GAP + 29.0));
        assert_eq!(s["b"].0.x, 29.0 + GAP + 29.0);
        assert!(s["d"].0.y > 0.0 && s["d"].0.z < 0.0, "above sits raised and behind");
    }

    #[test]
    fn switches_overshoot_then_settle() {
        assert_eq!(back_out(0.0), 0.0);
        assert!((back_out(1.0) - 1.0).abs() < 1e-6);
        assert!((0..100).map(|i| back_out(i as f32 / 100.0)).any(|x| x > 1.0));
    }
}
