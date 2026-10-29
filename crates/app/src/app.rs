//! The window: a top bar, the desk, a panel of facts and switches under it,
//! and a sheet for pairing and for choosing where dropped files go.
//! Everything shown comes from the running daemon; everything changed goes
//! through it.

mod cells;
mod pages;
mod plan;

use crate::daemon::{Event, Job, Latency, Link};
use crate::motion::{self, tween};
use crate::pairing::{Pairing, Step};
use crate::scene::{self, Cargo, Device, Kind, SPACING, Scene, v};
use crate::style::*;
use crate::view::{Check, Prefs};
use crate::widgets::{self, Look, Pills, a11y_text, button, button_width, mix, para, switch, text_width};
use egui::accesskit::{Live, Role};
use egui::{Align2, Color32, CornerRadius, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use one_mk_ctl::status::{PeerStatus, Status, on_your, os_name};
use one_mk_proto::{Side, os};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

const TOP_H: f32 = 56.0;
const PANEL_H: f32 = 196.0;
/// Inner padding of the panel's cells.
const PAD: f32 = 20.0;
const GHOST: &str = "\u{0}pairing";
/// Wallpapers for paired computers: low chroma, about equal lightness.
const WALLS: [[u32; 2]; 4] = [[0x35312C, 0x16140F], [0x283430, 0x101614], [0x2A313B, 0x11151A], [0x3A2E2A, 0x171211]];
const ME_WALL: [u32; 2] = [0x1D2D3C, 0x0D151E];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Desk,
    Layout,
    Files,
    Health,
    Advanced,
}
/// Each tab, and a shorter name for narrow windows. Settings most people
/// never change (Ctrl and Cmd, gaming mode, edge rules) live in Advanced.
const TABS: [(Tab, &str, &str); 5] = [
    (Tab::Desk, "Desk", "Desk"),
    (Tab::Layout, "Layout", "Layout"),
    (Tab::Files, "Clipboard & files", "Files"),
    (Tab::Health, "Health", "Health"),
    (Tab::Advanced, "Advanced", "Advanced"),
];

impl Tab {
    /// Tabs that show the desk of computers.
    fn desk(self) -> bool {
        matches!(self, Tab::Desk | Tab::Files)
    }
    /// Tabs that mean something while one·mk is off.
    fn when_off(self) -> bool {
        matches!(self, Tab::Desk | Tab::Health | Tab::Advanced)
    }
}
const PLACES: [(Side, &str); 4] = [(Side::Left, "Left"), (Side::Right, "Right"), (Side::Top, "Above"), (Side::Bottom, "Below")];
const SWAPS: [(&str, &str); 3] = [("auto", "Auto"), ("on", "Always swap"), ("off", "Never swap")];
/// One computer's row in the Ctrl and Cmd card.
const SWAP_ROW: f32 = 36.0;

enum Drag {
    Device { i: usize, start: Pos2, x0: f32 },
    Orbit,
}

struct PairUi {
    proc: Option<Pairing>,
    step: Step,
    /// When the current step began (for staggered reveals).
    since: f64,
    /// When pairing began: the sheet waits for the desk to make room.
    opened: f64,
    hint: bool,
    retry: Option<String>,
    me: String,
}

/// What the sheet does when one of its buttons is chosen.
#[derive(Clone, Debug, PartialEq)]
enum Act {
    Close,
    Decline,
    Confirm,
    Retry,
    Send(String),
}

/// Everything the sheet shows, so it can be drawn again while it leaves.
#[derive(Clone, PartialEq)]
struct SheetView {
    title: String,
    sub: String,
    code: Option<(String, f64)>,
    /// A tick drawn in from this time: paired.
    check: Option<f64>,
    buttons: Vec<(String, Look, Act)>,
    /// The button the keyboard starts on: never the one that confirms.
    focus: usize,
    /// When it may start to show.
    appear: f64,
    /// Changes when the step changes, to move focus into it again.
    key: String,
}

struct Toast {
    text: String,
    /// The raw error, quieter, under the message.
    detail: Option<String>,
    error: bool,
    at: f64,
    life: f32,
}

/// A measurement, and what it replaced, so the numbers can count over.
struct Measured {
    l: Latency,
    was: (f32, f32),
    at: f64,
}

/// What a cell of the panel shows. When it changes, the old one fades
/// out and the new one rises in.
#[derive(Clone, PartialEq, Debug)]
enum Cell {
    Head(String, String),
    Inspector(Option<String>),
    Clipboard,
    /// This computer's switches.
    Controls,
    FilesHead(String, String),
    FilesSettings,
    Arrange(Option<String>),
    LineUp(Option<String>),
    Profiles,
}

struct Slot {
    cell: Option<(Cell, Rect)>,
    was: Option<(Cell, Rect)>,
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
    /// While dragging: the side the computer would go to if let go now.
    drag_side: Option<(String, Side)>,
    menu_for: Option<usize>,
    pairing: Option<PairUi>,
    /// Files dropped where it wasn't clear which computer they were for, and when.
    pending_drop: Option<(Vec<PathBuf>, f64)>,
    /// The sheet as last drawn, and when it started to leave.
    sheet_was: Option<(SheetView, f64)>,
    sheet_top: Option<f32>,
    sheet_key: String,
    toast: Option<Toast>,
    latency: HashMap<String, Measured>,
    failed_measure: HashSet<String>,
    measuring: Option<String>,
    /// Sides chosen here and not yet reported back by the daemon, and when.
    pending_side: HashMap<String, (Side, f64)>,
    pending_swap: HashMap<String, String>,
    /// A computer that should hop (not slide) to its new place.
    hop: Option<String>,
    /// A computer just paired, and when: shown before the daemon lists it.
    just_paired: Option<(String, f64)>,
    /// The screen that gets the mouse and keyboard right now.
    active: String,
    busy: Vec<Job>,
    /// The computer the panel describes (an id, or "me").
    selected: Option<String>,
    slots: [Slot; 3],
    /// Whether the files were moving over the window (so the pointer
    /// position is real, not left over from before the drag).
    drop_pos_fresh: bool,
    /// An arrow key went down on the frame before (focus moves the frame
    /// after the key, and arrowing onto a choice picks it).
    arrowed: bool,
    arrow_down: bool,
    /// `settings.toml` as it is now.
    prefs: Prefs,
    /// The last `one-mk doctor` report, and when it came.
    doctor: Option<(Result<Vec<Check>, String>, f64)>,
    /// A fix just copied (its row), and when, to say so.
    copied: Option<(usize, f64)>,
    /// Alignments chosen here and not yet reported back, and when.
    pending_align: HashMap<String, (Side, one_mk_proto::Align, f64)>,
    pending_lock: Option<(bool, f64)>,
    plan_drag: Option<plan::PlanDrag>,
    /// The computer on the plan the keyboard is on.
    plan_focus: Option<String>,
    /// An arrow-key move on the plan, applied once the keys rest.
    nudge: Option<(String, f64)>,
    /// Text being typed into fields, by field.
    drafts: HashMap<String, String>,
    /// The profile whose options are open, and whether to scroll to it.
    profile_open: Option<String>,
    profile_scroll: bool,
    /// When the tab last changed (the page fades in).
    tab_at: f64,
    fonts: Fallbacks,
    size: egui::Vec2,
    now: f64,
    ctx: egui::Context,
    lockup: egui::TextureHandle,
}

fn me_kind() -> Kind {
    if cfg!(target_os = "macos") { Kind::Laptop } else { Kind::Monitor }
}

fn me_os() -> u8 {
    if cfg!(target_os = "macos") {
        os::MACOS
    } else if cfg!(windows) {
        os::WINDOWS
    } else {
        os::LINUX
    }
}

fn items(n: usize) -> String {
    if n == 1 { "1 item".to_owned() } else { format!("{n} items") }
}

/// "to the left", "above this computer"…: for sentences about moving one.
fn to_side(side: Side) -> &'static str {
    match side {
        Side::Left => "to the left",
        Side::Right => "to the right",
        Side::Top => "above this computer",
        Side::Bottom => "below this computer",
    }
}

fn edge_word(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
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
            // In front of this computer, but beside it, so it hides
            // neither screen.
            Side::Bottom => (v(me_half + (k - 0.5) * SPACING, 0.0, 40.0), 0.8),
        };
        out.insert(id.clone(), place);
    }
    out
}

// ---------- the panel's painted rows ----------

/// Status by shape, not colour: a filled dot when connected, a ring when
/// not, no dot for this computer. Returns the width it took.
fn chip(ui: &Ui, c: Rect, label: &str, dot: Option<bool>) -> f32 {
    let p = ui.painter();
    let mid = c.top() + 10.0;
    let tw = text_width(ui, label, regular(T_META));
    p.text(pos2(c.right(), mid), Align2::RIGHT_CENTER, label, regular(T_META), INK2);
    let at = pos2(c.right() - tw - 9.0, mid);
    match dot {
        Some(true) => {
            p.circle_filled(at, 3.0, INK);
        }
        Some(false) => {
            p.circle_stroke(at, 2.6, Stroke::new(1.5, OFF));
        }
        None => {}
    }
    tw + if dot.is_some() { 22.0 } else { 10.0 }
}

/// A computer's name on the inspector's first row, shortened to fit.
fn title_row(ui: &Ui, c: Rect, name: &str, room: f32) {
    let font = semibold(T_TITLE);
    let text = fit(ui.painter(), name, &font, c.width() - room);
    ui.painter().text(pos2(c.left(), c.top() + 10.0), Align2::LEFT_CENTER, &text, font, INK);
    if text != name {
        ui.interact(Rect::from_min_size(c.min, vec2(c.width() - room, 20.0)), Id::unique(("title", name)), Sense::hover()).on_hover_text(name);
    }
}

/// A key on the left and its value on the right, centred on `y`.
fn info_row(ui: &Ui, c: Rect, y: f32, key: &str, val: &str, mono_val: bool, alpha: f32) {
    let p = ui.painter();
    p.text(pos2(c.left(), y), Align2::LEFT_CENTER, key, regular(T_BODY), INK2);
    let font = if mono_val { mono(T_META) } else { regular(T_BODY) };
    let val = fit(p, val, &font, c.width() - text_width(ui, key, regular(T_BODY)) - 12.0);
    p.text(pos2(c.right(), y), Align2::RIGHT_CENTER, &val, font, INK.gamma_multiply(alpha));
    let r = Rect::from_min_max(pos2(c.left(), y - 10.0), pos2(c.right(), y + 10.0));
    a11y_text(ui, Id::unique(("row", key)), r, Role::Label, &format!("{key}: {val}"), None);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, home: Option<PathBuf>, size: [f32; 2]) -> anyhow::Result<Self> {
        crate::style::install(&cc.egui_ctx);
        motion::set_reduced(motion::detect());
        let (tx, rx) = std::sync::mpsc::channel();
        let link = Link::new(home, tx, cc.egui_ctx.clone())?;
        link.watch();
        let mut scene = Scene::default();
        scene.top_clear = TOP_H + 12.0;
        let mut me = Device::new("me", "This computer", me_kind(), true, ME_WALL);
        me.arrive(v(0.0, 0.0, 0.0), 0.0, 0.0);
        scene.devices.push(me);
        scene.cursor_on = "me".into();
        #[allow(unused_mut)]
        let mut tab = Tab::Desk;
        #[cfg(debug_assertions)]
        match std::env::var("ONE_MK_APP_TAB").as_deref() {
            Ok("files") => tab = Tab::Files,
            Ok("keyboard") => tab = Tab::Advanced,
            Ok("layout") => tab = Tab::Layout,
            Ok("health") => tab = Tab::Health,
            Ok("advanced") => tab = Tab::Advanced,
            _ => {}
        }
        #[cfg(debug_assertions)]
        if let Some(z) = std::env::var("ONE_MK_APP_ZOOM").ok().and_then(|z| z.parse::<f32>().ok()) {
            cc.egui_ctx.set_zoom_factor(z);
        }
        let slot = || Slot { cell: None, was: None, at: f64::NEG_INFINITY };
        Ok(Self {
            link,
            rx,
            status: None,
            autostart: None,
            scene,
            tab,
            drag: None,
            drag_side: None,
            menu_for: None,
            pairing: None,
            pending_drop: None,
            sheet_was: None,
            sheet_top: None,
            sheet_key: String::new(),
            toast: None,
            latency: HashMap::new(),
            failed_measure: HashSet::new(),
            measuring: None,
            pending_side: HashMap::new(),
            pending_swap: HashMap::new(),
            hop: None,
            just_paired: None,
            active: "me".into(),
            busy: vec![],
            selected: None,
            slots: [slot(), slot(), slot()],
            drop_pos_fresh: false,
            arrowed: false,
            arrow_down: false,
            prefs: Prefs::default(),
            doctor: None,
            copied: None,
            pending_align: HashMap::new(),
            pending_lock: None,
            plan_drag: None,
            plan_focus: None,
            nudge: None,
            drafts: HashMap::new(),
            profile_open: None,
            profile_scroll: false,
            tab_at: f64::NEG_INFINITY,
            fonts: Fallbacks::default(),
            size: vec2(size[0], size[1]),
            now: 0.0,
            ctx: cc.egui_ctx.clone(),
            lockup: lockup_texture(&cc.egui_ctx),
        })
    }

    fn running(&self) -> Option<&Status> {
        self.status.as_ref().and_then(Option::as_ref)
    }

    fn off(&self) -> bool {
        matches!(self.status, Some(None))
    }

    fn toast(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some(Toast { text: text.into(), detail: None, error: false, at: now, life: 3.5 });
    }

    /// An error says what went wrong and what to do; the raw reason goes
    /// underneath. It stays until dismissed, or for ten seconds.
    fn toast_error(&mut self, text: impl Into<String>, detail: Option<String>, now: f64) {
        let detail = detail.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty());
        self.toast = Some(Toast { text: text.into(), detail, error: true, at: now, life: 10.0 });
    }

    fn handle_events(&mut self, now: f64) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Status(s) => {
                    self.status = Some(s);
                    let names: Vec<String> = self.running().map_or(vec![], |st| std::iter::once(st.name.clone()).chain(st.peers.iter().map(|p| p.name.clone())).collect());
                    self.fonts.cover(&self.ctx, names.iter().map(String::as_str));
                    // A measurement belongs to one connection: once that
                    // computer goes offline (or one·mk stops) it means nothing.
                    let online: Vec<String> = self.running().map_or(vec![], |s| s.peers.iter().filter(|p| p.online).map(|p| p.id.clone()).collect());
                    self.latency.retain(|id, _| online.contains(id));
                    self.failed_measure.retain(|id| online.contains(id));
                    self.sync_scene();
                }
                Event::Autostart(on) => self.autostart = Some(on),
                Event::Latency(l) => {
                    self.measuring = None;
                    self.failed_measure.remove(&l.peer);
                    // Show the measurement where it was taken: one crossing, labelled.
                    self.scene.latency_label = Some(format!("{:.2} ms", l.p50_us / 2000.0));
                    if self.active == "me" && self.scene.flight.is_none() {
                        self.scene.fly("me", &l.peer, Cargo::Probe);
                    }
                    let was = self.latency.get(&l.peer).map_or((0.0, 0.0), |m| (m.l.p50_us, m.l.p99_us));
                    self.latency.insert(l.peer.clone(), Measured { l, was, at: now });
                }
                Event::Done { what, error } => {
                    self.busy.retain(|j| j != &what);
                    self.finished(what, error, now);
                }
                Event::Prefs(p) => self.prefs = p,
                Event::Doctor(d) => self.doctor = Some((d, now)),
            }
        }
        // Health is open and nothing ran yet: run the checks.
        if self.tab == Tab::Health && self.doctor.is_none() && !self.busy.contains(&Job::Doctor) && self.status.is_some() {
            self.doctor_on_open();
        }
        self.settle_nudge(now);
        let before = self.pending_align.len();
        self.pending_align.retain(|id, (_, _, t)| now - *t < 5.0 || self.nudge.as_ref().is_some_and(|(n, _)| n == id));
        if self.pending_lock.is_some_and(|(_, t)| now - t > 5.0) {
            self.pending_lock = None;
        }
        if self.pending_align.len() != before {
            self.sync_scene();
        }
        self.pairing_steps(now);
        // A side the daemon never confirmed: show what it really has.
        let before = self.pending_side.len();
        self.pending_side.retain(|_, (_, t)| now - *t < 5.0);
        if self.pending_side.len() != before {
            self.sync_scene();
        }
        self.sync_pairing_scene(now);
    }

    /// A command finished: say how it went, and undo what it promised if it failed.
    fn finished(&mut self, what: Job, error: Option<String>, now: f64) {
        let Some(e) = error else {
            match what {
                Job::Send { to, count } => self.toast(format!("Sent {} to {to}", items(count)), now),
                Job::Clip { send: true, name, .. } => self.toast(format!("Sent this clipboard to {name}"), now),
                Job::Clip { send: false, name, .. } => self.toast(format!("Copied {name}'s clipboard here"), now),
                Job::Profile { verb: "save", name } => self.toast(format!("Saved this layout as “{name}”"), now),
                Job::Profile { verb: "use", name } => self.toast(format!("Using the layout “{name}”"), now),
                Job::Profile { verb: "delete", name } => self.toast(format!("Deleted the layout “{name}”"), now),
                Job::ProfileAuto { name, ssid: Some(ssid) } => self.toast(format!("“{name}” comes on by itself on Wi-Fi “{ssid}”"), now),
                Job::ProfileAuto { name, ssid: None } => self.toast(format!("“{name}” no longer comes on by itself"), now),
                _ => {}
            }
            return;
        };
        let e = Some(e);
        match what {
            Job::Measure(id) => {
                // Not again by itself: it would fail again, forever.
                self.measuring = None;
                self.failed_measure.insert(id);
            }
            Job::Layout { id, name, side } => {
                self.pending_side.remove(&id);
                self.sync_scene();
                self.toast_error(format!("Couldn't move {name} {}. The desk went back to where it was; try again.", to_side(side)), e, now);
            }
            Job::Send { to, count } => {
                self.toast_error(format!("Couldn't send {} to {to}. Check that {to} is online, then drop the files again.", items(count)), e, now);
            }
            Job::Swap { id, name, .. } => {
                self.pending_swap.remove(&id);
                self.sync_scene();
                self.toast_error(format!("Couldn't change Ctrl and Cmd for {name}. Try again."), e, now);
            }
            Job::TakeBack => self.toast_error("Couldn't take back control. Press a key on this computer, or run `one-mk takeback`.", e, now),
            Job::Autostart(on) => {
                let word = if on { "on" } else { "off" };
                self.toast_error(format!("Couldn't turn {word} start at login."), e, now);
            }
            Job::Start => self.toast_error("one·mk didn't start.", e, now),
            Job::Open => self.toast_error("Couldn't open the received files folder.", e, now),
            Job::Align { id, name, .. } => {
                self.pending_align.remove(&id);
                self.sync_scene();
                self.toast_error(format!("Couldn't line up {name}. The plan went back to where it was; try again."), e, now);
            }
            Job::Profile { verb, name } => {
                let what = match verb {
                    "save" => format!("save this layout as “{name}”"),
                    "delete" => format!("delete “{name}”"),
                    _ => format!("switch to “{name}”"),
                };
                self.toast_error(format!("Couldn't {what}."), e, now);
            }
            Job::ProfileAuto { name, .. } => self.toast_error(format!("Couldn't change when “{name}” comes on by itself."), e, now),
            Job::Switch { name, to } => {
                let place = if to == "here" { "here".to_owned() } else { format!("to {name}") };
                self.toast_error(format!("Couldn't move the mouse and keyboard {place}. Check that it's connected, then try again."), e, now);
            }
            Job::LockPointer(on) => {
                self.pending_lock = None;
                let word = if on { "keep the pointer here" } else { "let the pointer cross again" };
                self.toast_error(format!("Couldn't {word}."), e, now);
            }
            Job::LockAll => self.toast_error("Couldn't lock the screens.", e, now),
            Job::Clip { send, name, .. } => {
                let what = if send { format!("send this clipboard to {name}") } else { format!("get {name}'s clipboard") };
                self.toast_error(format!("Couldn't {what}."), e, now);
            }
            Job::Set { what, .. } => {
                // Show what is really saved.
                self.prefs = self.link.prefs();
                self.toast_error(format!("Couldn't change {what}."), e, now);
            }
            Job::Doctor => {}
            Job::EditSettings => self.toast_error("Couldn't open settings.toml.", e, now),
        }
    }

    fn pairing_steps(&mut self, now: f64) {
        let mut finished: Option<(String, String)> = None;
        if let Some(p) = &mut self.pairing {
            while let Some(step) = p.proc.as_ref().and_then(|c| c.rx.try_recv().ok()) {
                match step {
                    Step::Searching { me } => p.me = me,
                    Step::Hint => p.hint = true,
                    Step::Retry(why) => {
                        p.retry = Some(why);
                        p.hint = false;
                        p.step = Step::Searching { me: p.me.clone() };
                        p.since = now;
                    }
                    Step::Paired { id, name } => {
                        finished = Some((id.clone(), name.clone()));
                        p.proc = None;
                        p.step = Step::Paired { id, name };
                        p.since = now;
                    }
                    Step::Ended(why) => {
                        p.proc = None;
                        p.step = Step::Ended(why);
                        p.since = now;
                    }
                    s => {
                        if s == Step::Found {
                            p.hint = false;
                        }
                        p.step = s;
                        p.since = now;
                    }
                }
            }
        }
        if let Some((id, name)) = finished {
            // The outline becomes the new computer; the daemon picks it up
            // on reload. The sheet says it worked, so no toast.
            if let Some(i) = self.scene.index(GHOST) {
                let n = self.running().map_or(0, |s| s.peers.len());
                let d = &mut self.scene.devices[i];
                d.id = id.clone();
                d.name = name.clone();
                d.wall = WALLS[n % WALLS.len()];
                d.alpha.go(1.0, now, motion::fade(0.4), motion::out_cubic);
                d.wire.go(0.0, now, motion::fade(0.4), motion::out_cubic);
            }
            self.scene.code = None;
            self.just_paired = Some((id.clone(), now));
            // Selecting it measures it: the first crossing, labelled, is the payoff.
            self.selected = Some(id);
            if self.running().is_none() {
                self.toast(format!("Paired with {name}. Start one·mk to connect."), now);
            }
            let dir = self.link.dir.clone();
            std::thread::spawn(move || {
                let _ = one_mk_ctl::client::request(&dir, one_mk_ctl::client::Request::Reload, std::time::Duration::from_secs(3));
            });
            self.sync_scene();
        }
        // The success sheet holds a moment, then goes.
        #[cfg(debug_assertions)]
        let hold = if std::env::var_os("ONE_MK_APP_CONFIRM").is_some() { 60.0 } else { 1.15 };
        #[cfg(not(debug_assertions))]
        let hold = 1.15;
        if self.pairing.as_ref().is_some_and(|p| matches!(p.step, Step::Paired { .. }) && now - p.since > hold) {
            self.pairing = None;
        }
    }

    fn side_shown(&self, id: &str, reported: Side) -> Side {
        match &self.drag_side {
            Some((d, side)) if d == id => *side,
            _ => self.pending_side.get(id).map_or(reported, |(s, _)| *s),
        }
    }

    /// Makes the desk match what the daemon reports (and what was chosen
    /// here but not yet confirmed).
    fn sync_scene(&mut self) {
        let now = self.now;
        let status = self.running().cloned();
        let off = self.off();
        let peers: Vec<_> = status.as_ref().map(|s| s.peers.clone()).unwrap_or_default();
        // The daemon has caught up with a side or a swap chosen here.
        self.pending_side.retain(|id, (side, _)| peers.iter().any(|p| &p.id == id && p.side != *side));
        self.pending_swap.retain(|id, swap| peers.iter().any(|p| &p.id == id && &p.swap != swap));
        if let Some(s) = &status {
            // The daemon has caught up with an alignment chosen here.
            self.pending_align.retain(|id, (side, align, _)| {
                let real = s.peers.iter().find(|p| &p.id == id).map(|p| p.side);
                let a = s.layout.place(id).map(|p| p.align);
                real != Some(*side) || a != Some(*align)
            });
            if self.pending_lock.is_some_and(|(on, _)| on == s.pointer_locked) {
                self.pending_lock = None;
            }
        }
        let listed = self.just_paired.as_ref().is_some_and(|(j, _)| peers.iter().any(|p| &p.id == j));
        if listed || self.just_paired.as_ref().is_some_and(|(_, t)| now - t > 6.0) {
            self.just_paired = None;
        }
        let half = |kind: Kind| if kind == Kind::Laptop { 15.5 } else { 29.0 };
        let kind_of = |o: u8| if o == os::MACOS { Kind::Laptop } else { Kind::Monitor };
        let mut layout: Vec<(String, Side, f32)> = peers.iter().map(|p| (p.id.clone(), self.side_shown(&p.id, p.side), half(kind_of(p.os)))).collect();
        if let Some((id, _)) = &self.just_paired
            && let Some(i) = self.scene.index(id)
        {
            let d = &self.scene.devices[i];
            let side = if d.target.x < 0.0 { Side::Left } else { Side::Right };
            layout.push((id.clone(), side, half(d.kind) * d.scale));
        }
        if self.scene.index(GHOST).is_some() {
            let right = layout.iter().filter(|(_, s, _)| *s == Side::Right).count();
            let left = layout.iter().filter(|(_, s, _)| *s == Side::Left).count();
            layout.push((GHOST.into(), if right <= left { Side::Right } else { Side::Left }, half(Kind::Monitor) * 0.86));
        }
        let places = slots(half(me_kind()), &layout);

        let me = &mut self.scene.devices[0];
        if let Some(s) = &status {
            me.name = s.name.clone();
        }
        me.online = !off;
        me.set_subtitle(if off { "one·mk is off" } else { "This computer" }, now);

        // New computers come in staggered by how far they are from this one.
        let opening = self.scene.opening(now);
        for (n, p) in peers.iter().enumerate() {
            let i = match self.scene.index(&p.id) {
                Some(i) => i,
                None => {
                    let mut d = Device::new(&p.id, &p.name, kind_of(p.os), false, WALLS[n % WALLS.len()]);
                    let (pos, _) = places[&p.id];
                    let delay = if opening { 0.1 + 0.06 * (pos.x.abs() / SPACING).round() as f64 } else { 0.0 };
                    d.arrive(pos, now, delay);
                    self.scene.devices.push(d);
                    self.scene.devices.len() - 1
                }
            };
            let sub = self.subtitle_for(p);
            let d = &mut self.scene.devices[i];
            d.name = p.name.clone();
            d.online = p.online;
            d.kind = if p.os == 0 { d.kind } else { kind_of(p.os) };
            d.stay(now);
            d.set_subtitle(&sub, now);
        }
        let just = self.just_paired.as_ref().map(|(j, _)| j.clone());
        let hop = self.hop.take();
        for d in self.scene.devices.iter_mut().skip(1) {
            let keep = d.id == GHOST || peers.iter().any(|p| p.id == d.id) || just.as_deref() == Some(d.id.as_str());
            if !keep {
                d.leave(now);
            }
            if let Some((pos, scale)) = places.get(&d.id) {
                if hop.as_deref() == Some(d.id.as_str()) {
                    d.hop_to(*pos, now);
                } else {
                    d.target = *pos;
                }
                d.scale = *scale;
            }
        }

        // Follow the mouse and keyboard to whichever screen has them.
        let active = match &status {
            Some(Status { controlling: Some(name), .. }) => peers.iter().find(|p| &p.name == name).map_or("me".to_owned(), |p| p.id.clone()),
            _ => "me".to_owned(),
        };
        if active != self.active {
            let from = std::mem::replace(&mut self.active, active.clone());
            self.scene.fly(&from, &active, Cargo::Pointer);
        }
    }

    /// The second line on a computer's screen says what matters in the
    /// open tab: its system, or that files can go there.
    fn subtitle_for(&self, p: &PeerStatus) -> String {
        if !p.online {
            return "Offline".into();
        }
        match self.tab {
            Tab::Files => "Drop files here".into(),
            _ => os_name(p.os).into(),
        }
    }

    /// The pairing outline and the code on its screen.
    fn sync_pairing_scene(&mut self, now: f64) {
        let Some((step, since)) = self.pairing.as_ref().map(|p| (p.step.clone(), p.since)) else {
            if let Some(i) = self.scene.index(GHOST) {
                self.scene.devices[i].leave(now);
            }
            self.scene.code = None;
            return;
        };
        let live = matches!(step, Step::Searching { .. } | Step::Hint | Step::Found | Step::Code { .. } | Step::Waiting { .. });
        if self.scene.index(GHOST).is_none() && live {
            let mut g = Device::new(GHOST, "Looking…", Kind::Monitor, false, WALLS[1]);
            g.alpha.set(0.0);
            // Drawn in once, then still: nothing loops while it waits.
            g.wire = motion::Tween::from_to(0.0, 1.0, now, motion::fade(0.6), motion::in_out_cubic);
            g.scale = 0.86;
            g.set_subtitle("Pairing", now);
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
                    // The sheet is this computer's display of the code; the
                    // outline shows what the other screen displays.
                    if self.scene.code.as_ref().is_none_or(|(_, c, _)| c != code) {
                        self.scene.code = Some((vec![GHOST.into()], code.clone(), since));
                    }
                }
                Step::Ended(_) => {
                    d.leave(now);
                    self.scene.code = None;
                }
                Step::Waiting { .. } | Step::Paired { .. } => {}
                _ => {
                    d.name = "Looking…".into();
                    self.scene.code = None;
                }
            }
        }
    }

    fn start_pairing(&mut self, now: f64) {
        self.pending_drop = None;
        // The new computer joins the desk: show it.
        if !self.tab.desk() {
            self.set_tab(Tab::Desk);
        }
        match Pairing::start(&self.link.daemon, &self.link.home_args(), self.ctx.clone()) {
            Ok(proc) => {
                let me = self.running().map(|s| s.name.clone()).unwrap_or_default();
                let step = Step::Searching { me: me.clone() };
                self.pairing = Some(PairUi { proc: Some(proc), step, since: now, opened: now, hint: false, retry: None, me });
            }
            Err(e) => self.toast_error("Couldn't start pairing. Is one-mk installed next to this app?", Some(e.to_string()), now),
        }
    }

    fn sheet_open(&self) -> bool {
        self.pairing.is_some() || self.pending_drop.is_some()
    }
}

/// An image embedded by scripts/brand-assets.py: width and height (u32
/// LE), then straight RGBA.
pub fn embedded(bytes: &[u8]) -> ([usize; 2], &[u8]) {
    let n = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("header")) as usize;
    ([n(0), n(4)], &bytes[8..])
}

/// The brand lockup (the Crossing mark and the one·mk wordmark), drawn at
/// 2x for sharpness on high-density screens.
pub fn lockup_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let (size, px) = embedded(include_bytes!("../assets/lockup.rgba"));
    ctx.load_texture("lockup", egui::ColorImage::from_rgba_unmultiplied(size, px), egui::TextureOptions::LINEAR)
}

/// The lockup, top left, `height` points tall.
pub fn draw_lockup(p: &Painter, tex: &egui::TextureHandle, left_center: Pos2, height: f32) {
    let [w, h] = tex.size();
    let rect = Rect::from_min_size(left_center - vec2(0.0, height / 2.0), vec2(height * w as f32 / h as f32, height));
    p.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

impl App {
    /// The chosen computer, or the one that matters most right now.
    fn selected_id(&self) -> Option<String> {
        let s = self.running()?;
        if let Some(id) = &self.selected
            && (id == "me" || s.peers.iter().any(|p| &p.id == id))
        {
            return Some(id.clone());
        }
        let busy = s.controlling.as_ref().or(s.controlled_by.as_ref());
        s.peers
            .iter()
            .find(|p| Some(&p.name) == busy)
            .or_else(|| s.peers.iter().find(|p| p.online))
            .or(s.peers.first())
            .map(|p| p.id.clone())
            .or(Some("me".into()))
    }

    fn draggable(&self, i: usize) -> bool {
        let d = &self.scene.devices[i];
        i != 0 && d.id != GHOST && !d.leaving && self.running().is_some_and(|s| s.peers.iter().any(|p| p.id == d.id))
    }

    fn side_of(&self, id: &str) -> Option<Side> {
        self.pending_side.get(id).map(|(s, _)| *s).or_else(|| self.running()?.peers.iter().find(|p| p.id == id).map(|p| p.side))
    }

    fn name_of(&self, id: &str) -> String {
        self.running().and_then(|s| s.peers.iter().find(|p| p.id == id)).map_or_else(|| id.to_owned(), |p| p.name.clone())
    }

    fn choose_side(&mut self, id: &str, side: Side, hop: bool) {
        if self.side_of(id) != Some(side) {
            let name = self.name_of(id);
            self.pending_side.insert(id.to_owned(), (side, self.now));
            self.busy.push(Job::Layout { id: id.into(), name: name.clone(), side });
            self.link.set_side(id, &name, side);
            if hop {
                self.hop = Some(id.to_owned());
            }
        }
        self.sync_scene();
    }

    /// The side a dragged computer is on right now.
    fn side_at(&self, i: usize) -> Side {
        let d = &self.scene.devices[i];
        let dx = d.pos.x - self.scene.devices[0].pos.x;
        let moved = (d.pos.x - d.target.x).abs();
        match self.side_of(&d.id) {
            Some(s @ (Side::Top | Side::Bottom)) if moved < 20.0 => s,
            _ if dx < 0.0 => Side::Left,
            _ => Side::Right,
        }
    }

    /// Chooses a computer for the panel; the first look at one measures it.
    fn select(&mut self, id: String) {
        let online = self.running().is_some_and(|s| s.peers.iter().any(|p| p.id == id && p.online));
        if online && !self.latency.contains_key(&id) && !self.failed_measure.contains(&id) {
            self.measure(&id);
        }
        self.selected = Some(id);
    }

    fn measure(&mut self, id: &str) {
        if self.measuring.is_none() {
            self.measuring = Some(id.to_owned());
            self.busy.push(Job::Measure(id.to_owned()));
            self.link.measure(id);
        }
    }

    fn online_targets(&self) -> Vec<usize> {
        (1..self.scene.devices.len()).filter(|&i| self.draggable(i) && self.scene.devices[i].online).collect()
    }

    /// Files let go over the window. Without a computer under them (or
    /// with a pointer position that can't be trusted), ask which one,
    /// unless only one could take them.
    fn drop_files(&mut self, paths: Vec<PathBuf>, at: Option<Pos2>, now: f64) {
        let online = self.online_targets();
        match at.and_then(|p| self.scene.hit(p)) {
            Some(i) if i != 0 && self.draggable(i) && !self.scene.devices[i].online => {
                let name = self.scene.devices[i].name.clone();
                self.toast_error(format!("{name} is offline. Files can only go to a connected computer."), None, now);
            }
            Some(i) if online.contains(&i) => self.send_to(i, paths),
            _ if online.is_empty() => {
                self.toast_error("No computer is connected right now. Start one·mk on the other computer, then drop the files again.", None, now)
            }
            _ if online.len() == 1 => self.send_to(online[0], paths),
            _ => self.pending_drop = Some((paths, now)),
        }
    }

    fn send_to(&mut self, i: usize, paths: Vec<PathBuf>) {
        let (id, name) = (self.scene.devices[i].id.clone(), self.scene.devices[i].name.clone());
        self.busy.push(Job::Send { to: name.clone(), count: paths.len() });
        self.link.send_files(&id, &name, paths);
        self.scene.fly(&self.active.clone(), &id, Cargo::Parcel);
    }

    // ---------- the desk ----------

    /// Keyboard stops for each computer and the empty slot. They are made
    /// before the desk, so the desk stays on top for the pointer.
    fn desk_keys(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let kb = widgets::keyboard_focus(ui.ctx());
        let enter = ui.input(|inp| inp.key_pressed(Key::Enter) || inp.key_pressed(Key::Space));
        let mut focused = None;
        let mut pick = None;
        let sel = self.selected_id();
        for i in 0..self.scene.devices.len() {
            let selectable = i == 0 || self.draggable(i);
            let Some(b) = self.scene.bounds(i).filter(|_| selectable && self.running().is_some()) else { continue };
            let d = &self.scene.devices[i];
            let id = if i == 0 { "me".to_owned() } else { d.id.clone() };
            let r = ui.interact(b, Id::unique(("device", &id)), widgets::sense(ui, Sense::focusable_noninteractive()));
            let chosen = sel.as_deref() == Some(id.as_str());
            let place = if i == 0 { "this computer".to_owned() } else { self.side_of(&id).map_or(String::new(), |s| on_your(s).to_owned()) };
            let label = format!("{}, {}, {place}", d.name, d.subtitle);
            r.widget_info(move || egui::WidgetInfo::selected(Role::Button, true, chosen, &label));
            if r.has_focus() {
                focused = Some(i);
                if enter {
                    pick = Some(id);
                }
            }
        }
        if let Some(id) = pick {
            self.select(id);
        }
        self.scene.focused = focused.filter(|_| kb);
        self.scene.slot_focus = false;
        if let Some(s) = self.scene.slot_rect(rect) {
            let r = ui.interact(s, Id::unique("slot"), widgets::sense(ui, Sense::focusable_noninteractive()));
            widgets::a11y(&r, Role::Button, "Add a computer: pair another computer", None);
            self.scene.slot_focus = r.has_focus() && kb;
            if r.has_focus() && enter {
                self.start_pairing(now);
            }
        }
    }

    fn desk(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let locked = widgets::locked(ui);
        self.desk_keys(ui, rect, now);
        let resp = ui.interact(rect, Id::unique("desk"), if locked { Sense::hover() } else { Sense::CLICK | Sense::DRAG });
        let pointer = resp.hover_pos().filter(|_| !locked);
        let hovering_files = ui.input(|i| !i.raw.hovered_files.is_empty());
        self.scene.drop_targets = hovering_files;
        if hovering_files {
            self.drop_pos_fresh |= ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        }
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            // Some systems don't move the pointer during a drag from
            // another app; then where it was before means nothing.
            let at = if self.drop_pos_fresh { ui.input(|i| i.pointer.latest_pos()) } else { None };
            self.drop_files(dropped, at, now);
        }
        if !hovering_files {
            self.drop_pos_fresh = false;
        }

        if resp.drag_started() {
            let start = resp.interact_pointer_pos().unwrap_or_default();
            self.drag = match self.scene.hit(start) {
                Some(i) if self.draggable(i) => {
                    let d = &mut self.scene.devices[i];
                    // Picked up: it lifts a little and its shadow softens.
                    d.lift.go(motion::space(2.5), now, 0.12, motion::out_cubic);
                    d.pop.go(if motion::reduced() { 1.0 } else { 1.02 }, now, 0.12, motion::out_cubic);
                    Some(Drag::Device { i, start, x0: d.pos.x })
                }
                _ => Some(Drag::Orbit),
            };
        }
        if resp.dragged() {
            match &self.drag {
                Some(Drag::Device { i, start, x0 }) => {
                    let (i, start, x0) = (*i, *start, *x0);
                    if let Some(p) = resp.interact_pointer_pos() {
                        let ax = self.scene.x_axis(rect, i);
                        let d = p - start;
                        let dx = (d.x * ax.x + d.y * ax.y) / ax.length_sq().max(1e-3);
                        let me_x = self.scene.devices[0].pos.x;
                        self.scene.devices[i].pos.x = (x0 + dx).clamp(me_x - SPACING * 4.0, me_x + SPACING * 4.0);
                    }
                    self.make_room(i);
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
            if let Some(Drag::Device { i, .. }) = self.drag
                && i < self.scene.devices.len()
            {
                let side = self.side_at(i);
                let d = &mut self.scene.devices[i];
                d.lift.go(0.0, now, motion::space(0.16), motion::out_cubic);
                d.pop.go(1.0, now, 0.16, motion::out_cubic);
                let id = d.id.clone();
                self.drag_side = None;
                self.choose_side(&id, side, false);
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
        let slot = self.scene.slot_rect(rect);
        self.scene.slot_hover = self.drag.is_none() && slot.zip(pointer).is_some_and(|(r, p)| r.contains(p));
        if self.scene.slot_hover {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            if self.scene.slot_hover {
                self.start_pairing(now);
            } else if let Some(i) = under.filter(|&i| i == 0 || self.draggable(i)) {
                let id = if i == 0 { "me".to_owned() } else { self.scene.devices[i].id.clone() };
                self.select(id);
            }
        }
        // Double-click the empty desk: the camera goes back to where it rests.
        if resp.double_clicked() && under.is_none() {
            self.scene.reset_camera();
        }
        if resp.secondary_clicked() {
            self.menu_for = under.filter(|&i| self.draggable(i));
        }
        resp.context_menu(|ui| self.place_menu(ui));

        let dragging = match self.drag {
            Some(Drag::Device { i, .. }) => Some(i),
            _ => None,
        };
        self.scene.raise.go(f32::from(self.sheet_open()), now, motion::space(0.38), motion::out_expo);
        self.scene.sheet_top = self.sheet_top;
        self.scene.drop_preview.go(f32::from(self.tab == Tab::Files), now, motion::fade(0.28), motion::out_cubic);
        let several = self.scene.devices.iter().filter(|d| !d.leaving && d.id != GHOST).count() > 1;
        let shown = several && self.tab == Tab::Desk;
        self.scene.selected = match self.selected_id().as_deref().filter(|_| shown) {
            Some("me") => Some(0),
            Some(id) => self.scene.index(id),
            None => None,
        };
        self.scene.empty_slot = !self.sheet_open() && self.running().is_some_and(|s| s.peers.is_empty()) && self.just_paired.is_none();
        let moving = self.scene.step(now, dragging);
        let painter = ui.painter_at(rect);
        self.scene.paint(&painter, rect);
        if moving || self.drag.is_some() {
            ui.ctx().request_repaint();
        }
        self.desk_status(&painter, rect, hovering_files);
    }

    /// While a computer is dragged, the others make room live, so the
    /// result shows before it is let go.
    fn make_room(&mut self, i: usize) {
        let side = (self.scene.devices[i].id.clone(), self.side_at(i));
        if self.drag_side.as_ref() != Some(&side) {
            self.drag_side = Some(side);
            self.sync_scene();
        }
    }

    /// One quiet line at the foot of the desk: files on their way, or
    /// where to let go of the ones being dragged in.
    fn desk_status(&self, painter: &Painter, rect: Rect, hovering_files: bool) {
        let p = pos2(rect.center().x, rect.bottom() - 22.0);
        if hovering_files {
            painter.text(p, Align2::CENTER_CENTER, "Drop on a computer to send", semibold(T_BODY), SIGNAL);
            return;
        }
        // Transfers the daemon reports: the first one, with its bar (the
        // Files tab lists them all below).
        if let Some(t) = self.running().and_then(|s| s.transfers.first()).filter(|_| self.tab != Tab::Files) {
            let (title, detail, frac) = crate::view::transfer_lines(t);
            let more = self.running().map_or(0, |s| s.transfers.len() - 1);
            let text = if more > 0 { format!("{title} · {detail} · and {more} more") } else { format!("{title} · {detail}") };
            let text = fit(painter, &text, &regular(T_META), rect.width() - 80.0);
            painter.text(p - vec2(0.0, 6.0), Align2::CENTER_CENTER, text, regular(T_META), INK2);
            let bar = Rect::from_center_size(p + vec2(0.0, 9.0), vec2(220.0f32.min(rect.width() - 80.0), 3.0));
            painter.rect_filled(bar, CornerRadius::same(2), TRACK_OFF);
            let w = bar.width() * frac.unwrap_or(0.18);
            painter.rect_filled(Rect::from_min_size(bar.min, vec2(w.max(3.0), bar.height())), CornerRadius::same(2), SIGNAL);
            return;
        }
        let sending: Vec<String> = self.busy.iter().filter_map(|j| if let Job::Send { to, count } = j { Some(format!("{} to {to}", items(*count))) } else { None }).collect();
        if !sending.is_empty() {
            let text = fit(painter, &format!("Sending {}…", sending.join(", ")), &regular(T_META), rect.width() - 80.0);
            painter.text(p, Align2::CENTER_CENTER, text, regular(T_META), INK2);
        }
    }

    /// The right-click menu on a computer: where it is.
    fn place_menu(&mut self, ui: &mut Ui) {
        let Some(i) = self.menu_for.filter(|&i| i < self.scene.devices.len()) else {
            ui.close();
            return;
        };
        let (id, name) = (self.scene.devices[i].id.clone(), self.scene.devices[i].name.clone());
        let width = 180.0;
        let p = ui.painter().clone();
        let head = fit(&p, &format!("Where is {name}?"), &regular(T_META), width - 16.0);
        let (r, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
        p.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, head, regular(T_META), INK2);
        let current = self.side_of(&id);
        for (side, label) in [(Side::Left, "On the left"), (Side::Right, "On the right"), (Side::Top, "Above"), (Side::Bottom, "Below")] {
            let (r, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
            let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(resp.hovered()), 0.12, motion::out_cubic);
            p.rect_filled(r, CornerRadius::same(6), mix(Color32::TRANSPARENT, PILL, hover));
            widgets::focus_ring(ui, &resp, r, 6);
            let on = current == Some(side);
            if on {
                widgets::draw_check(&p, pos2(r.left() + 16.0, r.center().y), 10.0, INK, 1.0);
            }
            p.text(pos2(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, label, regular(T_BODY), if on { INK } else { INK_HOVER });
            widgets::a11y(&resp, Role::MenuItemRadio, &format!("{name}: {label}"), Some(on));
            if resp.clicked() {
                self.choose_side(&id, side, true);
                ui.close();
            }
        }
    }

    // ---------- the top bar ----------

    fn top_bar(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let p = ui.painter_at(rect);
        let mut m = egui::Mesh::default();
        let clear = Color32::from_rgba_premultiplied(0, 0, 0, 0);
        for (pt, c) in [(rect.left_top(), GROUND), (rect.right_top(), GROUND), (rect.right_bottom(), clear), (rect.left_bottom(), clear)] {
            m.colored_vertex(pt, c);
        }
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        p.add(egui::Shape::mesh(m));

        draw_lockup(&p, &self.lockup, pos2(rect.left() + 18.0, rect.center().y), 22.0);

        // Tabs. With one·mk off only some have anything to show. In a
        // narrow window the names get shorter, then the padding smaller.
        let off = self.off();
        let action_w = button_width(ui, &self.action().0) + 16.0;
        let lockup_w = 18.0 + 22.0 * self.lockup.size()[0] as f32 / self.lockup.size()[1] as f32 + 16.0;
        let room = rect.width() - 2.0 * lockup_w.max(action_w + 8.0);
        let room_off_centre = rect.width() - lockup_w - action_w - 16.0;
        let mut pick = None;
        for (short, pad) in [(false, 26.0), (true, 26.0), (true, 18.0), (true, 12.0)] {
            let labels: Vec<&str> = TABS.iter().map(|t| if short { t.2 } else { t.1 }).collect();
            let widths: Vec<f32> = labels.iter().map(|l| text_width(ui, l, semibold(T_BODY)) + pad).collect();
            let total = widths.iter().sum::<f32>() + 2.0 * (TABS.len() - 1) as f32 + 6.0;
            pick = Some((labels, widths, total));
            if total <= room_off_centre {
                break;
            }
        }
        let (labels, widths, total) = pick.expect("one choice at least");
        // Centred in the bar when it fits there, else between the logo and the button.
        let cx = if total <= room { rect.center().x } else { rect.left() + lockup_w + (room_off_centre - total) / 2.0 + total / 2.0 };
        let track = Rect::from_center_size(pos2(cx, rect.center().y), vec2(total, 34.0));
        let mut x = track.left() + 3.0;
        let cells: Vec<Rect> = widths
            .iter()
            .map(|w| {
                let r = Rect::from_min_size(pos2(x, track.top() + 3.0), vec2(*w, 28.0));
                x += w + 2.0;
                r
            })
            .collect();
        let tabs = Pills {
            id: Id::unique("tabs"),
            track,
            cells,
            labels: &labels,
            names: TABS.iter().map(|t| t.1.to_owned()).collect(),
            chosen: TABS.iter().position(|t| t.0 == self.tab),
            enabled: TABS.iter().map(|t| !off || t.0.when_off()).collect(),
            role: Role::Tab,
            font: T_BODY,
            radius: 7,
        };
        let cells = tabs.cells.clone();
        if let Some(k) = tabs.show(ui, self.arrowed) {
            self.set_tab(TABS[k].0);
        }
        // A dot on Health when something there needs attention.
        let trouble = self.running().is_some_and(|s| !s.input.is_empty())
            || self.doctor.as_ref().is_some_and(|(d, _)| d.as_ref().is_ok_and(|c| c.iter().any(|c| c.verdict == crate::view::Verdict::Fail)));
        if trouble && let Some(k) = TABS.iter().position(|t| t.0 == Tab::Health) {
            let c = cells[k];
            ui.painter().circle_filled(pos2(c.right() - 7.0, c.top() + 7.0), 3.0, ERROR);
        }

        // The one action that matters most right now. A sheet owns its own
        // way out, so the button steps aside while one is open.
        let (label, look, enabled, job) = self.action();
        let label = label.as_str();
        let shown = tween(ui.ctx(), Id::unique("action-shown"), f32::from(self.status.is_some() && !self.sheet_open()), motion::fade(0.12), motion::linear);
        if shown < 0.01 {
            return;
        }
        let w = button_width(ui, label);
        let r = Rect::from_min_size(pos2(rect.right() - 16.0 - w, rect.center().y - 17.0), vec2(w, 34.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.set_opacity(shown);
        let a11y_label = (label == "Take back").then_some("Take back control of this computer");
        if button(&mut child, r, Id::unique("action"), label, look, enabled && shown > 0.99, a11y_label).clicked() {
            match job {
                Some(Job::Start) => {
                    self.busy.push(Job::Start);
                    self.link.start();
                }
                Some(Job::TakeBack) => {
                    self.busy.push(Job::TakeBack);
                    self.link.take_back();
                }
                _ => self.start_pairing(now),
            }
        }
    }

    /// The top bar's button: what it says and does.
    fn action(&self) -> (String, Look, bool, Option<Job>) {
        let controlled = self.running().is_some_and(|s| s.controlled_by.is_some());
        let narrow = self.ctx.content_rect().width() < 900.0;
        let (label, look, enabled, job) = match self.status.as_ref() {
            Some(None) if self.busy.contains(&Job::Start) => ("Starting…", Look::Primary, false, None),
            Some(None) => ("Start one·mk", Look::Primary, true, Some(Job::Start)),
            _ if controlled && self.busy.contains(&Job::TakeBack) => ("Taking back…", Look::Primary, false, None),
            _ if controlled => ("Take back", Look::Primary, true, Some(Job::TakeBack)),
            _ if self.running().is_some_and(|s| !s.peers.is_empty()) => (if narrow { "Pair another" } else { "Pair another computer" }, Look::Quiet, true, None),
            _ => ("Pair a computer", Look::Primary, true, None),
        };
        (label.to_owned(), look, enabled, job)
    }

    fn set_tab(&mut self, tab: Tab) {
        if tab != self.tab {
            self.tab = tab;
            self.tab_at = self.now;
            self.plan_drag = None;
            if tab == Tab::Health {
                self.doctor_on_open();
            }
            self.sync_scene();
        }
    }

    // ---------- the panel ----------

    fn headline_text(&self) -> (String, String) {
        let Some(s) = self.status.as_ref() else { return ("Looking for one·mk…".into(), String::new()) };
        let Some(s) = s else {
            return ("one·mk isn't running".into(), "Start it to share this computer's mouse, keyboard, clipboard and files.".into());
        };
        if let Some(p) = &s.controlled_by {
            let sub = if cfg!(target_os = "linux") {
                format!("Choose Take back, or stop sending input from {p}. Control comes back by itself after a minute without input from it.")
            } else {
                "Move this computer's mouse or press a key, or choose Take back.".into()
            };
            return (format!("{p} is controlling this computer"), sub);
        }
        if let Some(p) = &s.controlling {
            return (format!("Controlling {p}"), "Move the pointer back across the lit edge to return.".into());
        }
        // The desktop hasn't allowed input sharing yet: that comes first.
        if !s.input.is_empty() && !s.peers.is_empty() {
            return ("Waiting for this computer's permission".into(), s.input.join(" "));
        }
        let online: Vec<_> = s.peers.iter().filter(|p| p.online).collect();
        let away = match s.peers.len() - online.len() {
            0 => String::new(),
            1 => " 1 paired computer is offline.".into(),
            n => format!(" {n} paired computers are offline."),
        };
        match online[..] {
            [] if s.peers.is_empty() => ("No computers paired yet".into(), "Choose Pair a computer, then do the same on the other computer.".into()),
            [] => (
                "Waiting for your computers".into(),
                "Paired computers appear here when one·mk is running on them and they're on the same network.".into(),
            ),
            [p] => {
                let side = self.side_shown(&p.id, p.side);
                (format!("{} is {}", p.name, on_your(side)), format!("Move the pointer off the {} edge to cross over.{away}", edge_word(side)))
            }
            _ => (format!("Connected to {} computers", online.len()), format!("Move the pointer off an edge to cross over. Drag computers to match your desk.{away}")),
        }
    }

    /// What each cell of the panel shows in this state, and where.
    fn cells(&self, rect: Rect) -> Vec<(Cell, Rect)> {
        let w = rect.width();
        let x = match self.tab {
            Tab::Layout => [0.0, w * 0.3, w * 0.6, w],
            Tab::Files => [0.0, w * 0.34, w * 0.63, w],
            _ => [0.0, w * 0.34, w * 0.66, w],
        };
        let cell = |a: usize, b: usize| Rect::from_min_max(pos2(rect.left() + x[a] + PAD, rect.top() + PAD), pos2(rect.left() + x[b] - PAD, rect.bottom() - PAD));
        let (t, s) = self.headline_text();
        match self.tab {
            // Off, there is nothing to inspect: the message takes the room.
            Tab::Desk if self.off() => vec![(Cell::Head(t, s), cell(0, 2)), (Cell::Controls, cell(2, 3))],
            Tab::Desk => vec![(Cell::Head(t, s), cell(0, 1)), (Cell::Inspector(self.selected_id()), cell(1, 2)), (Cell::Controls, cell(2, 3))],
            Tab::Layout => {
                let sel = self.selected_id().filter(|id| id != "me");
                vec![(Cell::Arrange(sel.clone()), cell(0, 1)), (Cell::LineUp(sel), cell(1, 2)), (Cell::Profiles, cell(2, 3))]
            }
            Tab::Health | Tab::Advanced => vec![],
            Tab::Files => {
                let any = self.running().is_some_and(|s| s.peers.iter().any(|p| p.online));
                let sub = if any {
                    "Drag files or folders onto a connected computer on the desk. They arrive in its Downloads/one-mk folder."
                } else {
                    "No computer is connected right now. Files can go to one once it's on and connected."
                };
                vec![(Cell::FilesHead("Drop files on a computer".into(), sub.into()), cell(0, 1)), (Cell::Clipboard, cell(1, 2)), (Cell::FilesSettings, cell(2, 3))]
            }
        }
    }

    fn panel(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, PANEL);
        p.hline(rect.x_range(), rect.top(), Stroke::new(1.0, LINE));
        let cells = self.cells(rect);
        for (_, r) in cells.iter().skip(1) {
            p.vline(r.left() - PAD, rect.top() + 1.0..=rect.bottom(), Stroke::new(1.0, LINE));
        }
        let opening = self.scene.opening(now);
        for k in 0..3 {
            let next = cells.get(k).cloned();
            let slot = &mut self.slots[k];
            if slot.cell.as_ref().map(|c| &c.0) != next.as_ref().map(|c| &c.0) {
                let was = std::mem::replace(&mut slot.cell, next);
                // On first sight nothing fades out; the panel just rises in.
                slot.was = if opening { None } else { was };
                slot.at = now;
            } else if let (Some(c), Some(n)) = (&mut slot.cell, next) {
                c.1 = n.1;
            }
        }
        let out = motion::fade(0.09) as f64;
        // Under a sheet the panel steps back.
        let dim = tween(ui.ctx(), Id::unique("panel-dim"), if self.sheet_open() { 0.3 } else { 1.0 }, motion::fade(0.2), motion::out_cubic);
        for k in 0..3 {
            let (at, was, cell) = (self.slots[k].at, self.slots[k].was.clone(), self.slots[k].cell.clone());
            let t = now - at;
            if let Some((c, r)) = was.filter(|_| t < out) {
                ui.ctx().request_repaint();
                let mut child = ui.new_child(UiBuilder::new().max_rect(r).id_salt(("was", k)));
                child.set_clip_rect(rect);
                child.disable();
                child.set_opacity((1.0 - (t / out) as f32) * dim);
                self.draw_cell(&mut child, &c, r, now);
                continue;
            }
            let Some((c, r)) = cell else { continue };
            // Staggered arrivals are movement too: with reduced motion the
            // cells fade in together, at once.
            let stagger = |secs: f32| f64::from(motion::space(secs));
            let delay = if opening && at < 1.0 { stagger(0.25 + 0.04 * k as f32) } else if self.slots[k].was.is_some() { out + stagger(0.03 * k as f32) } else { 0.0 };
            let e = motion::progress(now, at + delay, motion::fade(if opening { 0.2 } else { 0.18 }), motion::out_cubic);
            if e < 1.0 {
                ui.ctx().request_repaint();
            }
            let r2 = r.translate(vec2(0.0, motion::space(4.0) * (1.0 - e)));
            let mut child = ui.new_child(UiBuilder::new().max_rect(r2).id_salt(("cell", k)));
            child.set_clip_rect(rect);
            child.set_opacity(e * dim);
            self.draw_cell(&mut child, &c, r2, now);
        }
    }

    fn draw_cell(&mut self, ui: &mut Ui, cell: &Cell, r: Rect, now: f64) {
        match cell {
            Cell::Head(title, sub) => {
                let th = para(ui, r.left_top(), r.width(), title, semibold(T_TITLE), INK);
                let sh = para(ui, r.left_top() + vec2(0.0, th + 6.0), r.width(), sub, regular(T_BODY), INK2);
                let at = Rect::from_min_size(r.min, vec2(r.width(), th + sh + 6.0));
                a11y_text(ui, Id::unique("headline"), at, Role::Status, &format!("{title}. {sub}"), Some(Live::Polite));
            }
            Cell::Inspector(sel) => self.inspector(ui, r, sel.clone(), now),
            Cell::Clipboard => self.clipboard_cell(ui, r),
            Cell::Controls => self.controls(ui, r),
            Cell::FilesHead(t, s) => self.files_head(ui, r, t, s),
            Cell::FilesSettings => self.files_settings(ui, r),
            Cell::Arrange(sel) => self.arrange_cell(ui, r, sel.clone()),
            Cell::LineUp(sel) => self.line_up_cell(ui, r, sel.clone()),
            Cell::Profiles => self.profiles_cell(ui, r),
        }
    }

    /// The selected computer: what it is, where it is, how fast it answers.
    fn inspector(&mut self, ui: &mut Ui, c: Rect, sel: Option<String>, now: f64) {
        let Some(status) = self.running().cloned() else { return };
        let Some(peer) = status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() else {
            let room = chip(ui, c, "This computer", None);
            title_row(ui, c, &status.name, room);
            let online = status.peers.iter().filter(|p| p.online).count();
            let paired = match status.peers.len() {
                0 => "None yet".to_owned(),
                n => format!("{n}, {online} connected"),
            };
            // No position here: the others are placed around this one.
            info_row(ui, c, c.top() + 42.0, "ID", &status.id, true, 1.0);
            info_row(ui, c, c.top() + 66.0, "Paired", &paired, false, 1.0);
            info_row(ui, c, c.top() + 90.0, "System", os_name(me_os()), false, 1.0);
            self.quick_actions(ui, c, c.top() + 114.0, None);
            return;
        };
        let mut room = if peer.online { chip(ui, c, "Connected", Some(true)) } else { chip(ui, c, "Offline", Some(false)) };
        // Measure again: a quiet icon button on the title row.
        if peer.online && (self.latency.contains_key(&peer.id) || self.failed_measure.contains(&peer.id)) {
            let busy = self.measuring.is_some();
            let r = Rect::from_min_size(pos2(c.right() - room - 26.0, c.top() - 2.0), vec2(24.0, 24.0));
            let spin = if self.measuring.as_ref() == Some(&peer.id) && !motion::reduced() {
                ui.ctx().request_repaint();
                (now * 5.0) as f32
            } else {
                0.0
            };
            let name = if busy { "Measuring…" } else { "Measure again" };
            if widgets::icon_button(ui, r, Id::unique(("measure", &peer.id)), name, !busy, |pp, rr, col| widgets::draw_refresh(pp, rr, col, spin)).clicked() {
                self.measure(&peer.id);
            }
            room += 32.0;
        }
        title_row(ui, c, &peer.name, room);

        // Where it is: a control, so it can be set without a mouse.
        let side = self.side_shown(&peer.id, peer.side);
        let track = Rect::from_min_size(pos2(c.left(), c.top() + 28.0), vec2(c.width(), 26.0));
        let labels: Vec<&str> = PLACES.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique(("position", &peer.id)),
            track,
            cells: Pills::even(track, 4, 2.0),
            labels: &labels,
            names: PLACES.iter().map(|(_, l)| format!("Position of {}: {l}", peer.name)).collect(),
            chosen: PLACES.iter().position(|(s, _)| *s == side),
            enabled: vec![true; 4],
            role: Role::RadioButton,
            font: if c.width() >= 230.0 { T_BODY } else { T_META },
            radius: 6,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.choose_side(&peer.id, PLACES[k].0, true);
        }

        let (y0, y1) = (c.top() + 70.0, c.top() + 92.0);
        if !peer.online {
            info_row(ui, c, y0, "System", os_name(peer.os), false, 1.0);
            return;
        }
        self.quick_actions(ui, c, c.top() + 116.0, Some(&peer));
        match self.latency.get(&peer.id) {
            Some(m) => {
                // New numbers count over from the old ones, in fixed-width digits.
                let k = motion::progress(now, m.at, motion::fade(0.45), motion::out_cubic);
                if k < 1.0 {
                    ui.ctx().request_repaint();
                }
                let fade_in = if m.was.0 > 0.0 { 1.0 } else { motion::progress(now, m.at, 0.1, motion::linear) };
                let p50 = scene::lerp(m.was.0, m.l.p50_us, k) / 2000.0;
                let p99 = scene::lerp(m.was.1, m.l.p99_us, k) / 2000.0;
                info_row(ui, c, y0, "Response time", &format!("{p50:.2} ms"), true, fade_in);
                info_row(ui, c, y1, "Slowest 1 in 100", &format!("{p99:.2} ms"), true, fade_in);
                let tip = Rect::from_min_max(pos2(c.left(), y0 - 10.0), pos2(c.right(), y0 + 10.0));
                ui.interact(tip, Id::unique("response-tip"), Sense::hover())
                    .on_hover_text("How long a mouse move takes to reach that computer: half the measured round trip.");
            }
            None if self.failed_measure.contains(&peer.id) => info_row(ui, c, y0, "Response time", "Not measured", false, 1.0),
            None => {
                info_row(ui, c, y0, "Response time", "Measuring…", false, 0.6);
                // Measure each computer once, the first time it is shown.
                if self.measuring.is_none() {
                    self.measure(&peer.id);
                }
            }
        }
    }

    /// Ctrl and Cmd for every paired computer, a row each, from the top of
    /// `c` (the Advanced card sizes itself to fit them).
    pub(super) fn swaps(&mut self, ui: &mut Ui, c: Rect) {
        let peers: Vec<_> = self.running().map(|s| s.peers.clone()).unwrap_or_default();
        if peers.is_empty() {
            para(ui, c.left_top(), c.width(), "Pair a computer to choose how Ctrl and Cmd work when you type on it.", regular(T_BODY), INK2);
            return;
        }
        let arrowed = self.arrowed;
        let mut changes = vec![];
        for (k, peer) in peers.iter().enumerate() {
            let row = Rect::from_min_size(pos2(c.left(), c.top() + SWAP_ROW * k as f32), vec2(c.width(), SWAP_ROW));
            let p = ui.painter().clone();
            let seg_w = 264.0f32.min(row.width() * 0.6);
            let seg = Rect::from_min_size(pos2(row.right() - seg_w, row.center().y - 13.0), vec2(seg_w, 26.0));
            let os_label = os_name(peer.os);
            let ow = text_width(ui, os_label, regular(T_META));
            let name = fit(&p, &peer.name, &regular(T_BODY), seg.left() - row.left() - ow - 20.0);
            let nw = text_width(ui, &name, regular(T_BODY));
            p.text(pos2(row.left(), row.center().y), Align2::LEFT_CENTER, &name, regular(T_BODY), INK);
            p.text(pos2(row.left() + nw + 8.0, row.center().y), Align2::LEFT_CENTER, os_label, regular(T_META), INK2);
            if name != peer.name {
                ui.interact(Rect::from_min_max(row.min, pos2(row.left() + nw, row.max.y)), Id::unique(("swap-name", &peer.id)), Sense::hover()).on_hover_text(&peer.name);
            }
            let current = self.pending_swap.get(&peer.id).unwrap_or(&peer.swap);
            // Shorter words when the full ones would be cut.
            let full = seg_w >= 240.0;
            let labels: Vec<&str> = if full { SWAPS.iter().map(|(_, l)| *l).collect() } else { vec!["Auto", "Swap", "No swap"] };
            let pills = Pills {
                id: Id::unique(("swap", &peer.id)),
                track: seg,
                cells: Pills::even(seg, 3, 2.0),
                labels: &labels,
                names: SWAPS.iter().map(|(_, l)| format!("Ctrl and Cmd for {}: {l}", peer.name)).collect(),
                chosen: SWAPS.iter().position(|(v, _)| v == current),
                enabled: vec![true; 3],
                role: Role::RadioButton,
                font: T_META,
                radius: 6,
            };
            if let Some(k) = pills.show(ui, arrowed) {
                changes.push((peer.id.clone(), peer.name.clone(), SWAPS[k].0));
            }
        }
        for (id, name, val) in changes {
            self.pending_swap.insert(id.clone(), val.to_owned());
            self.busy.push(Job::Swap { id: id.clone(), name: name.clone(), swap: val.to_owned() });
            self.link.set_swap(&id, &name, val);
            self.sync_scene();
        }
    }

    // ---------- the sheet ----------

    /// What the sheet shows now: pairing, or where dropped files go.
    fn sheet_view(&self) -> Option<SheetView> {
        if let Some((paths, at)) = &self.pending_drop {
            let mut buttons: Vec<(String, Look, Act)> = vec![("Cancel".into(), Look::Quiet, Act::Close)];
            for i in self.online_targets() {
                let d = &self.scene.devices[i];
                buttons.push((d.name.clone(), Look::Quiet, Act::Send(d.id.clone())));
            }
            return Some(SheetView {
                title: format!("Send {} to which computer?", items(paths.len())),
                sub: "Couldn't tell which computer you dropped them on.".into(),
                code: None,
                check: None,
                buttons,
                focus: 1,
                appear: *at,
                key: "drop".into(),
            });
        }
        let pair = self.pairing.as_ref()?;
        let quiet = |l: &str, a: Act| (l.to_owned(), Look::Quiet, a);
        let (title, sub, buttons, focus): (String, String, Vec<(String, Look, Act)>, usize) = match &pair.step {
            Step::Searching { .. } | Step::Hint => {
                let me = if pair.me.is_empty() { String::new() } else { format!(" This one shows up there as “{}”.", pair.me) };
                let mut sub = format!("On the other computer, open one·mk and choose Pair a computer.{me}");
                if let Some(why) = &pair.retry {
                    sub += &format!("\n\nThat attempt didn't finish: {why}. Still looking.");
                } else if pair.hint {
                    sub += "\n\nNothing found yet. Check that both computers are on the same network (not a guest network) and that the firewall allows one·mk.";
                }
                ("Looking for another computer".into(), sub, vec![quiet("Cancel", Act::Close)], 0)
            }
            Step::Found => ("Found a computer".into(), "Connecting to it…".into(), vec![quiet("Cancel", Act::Close)], 0),
            Step::Code { name, .. } => (
                format!("Found “{name}”"),
                "Check that its screen shows the same code, then confirm.".into(),
                vec![quiet("They don't match", Act::Decline), ("Codes match, pair".into(), Look::Primary, Act::Confirm)],
                0,
            ),
            Step::Waiting { name } => (
                format!("Waiting for {name} to confirm"),
                format!("Confirm the same code on {name}. This stops waiting after 2 minutes."),
                vec![quiet("Cancel", Act::Close)],
                0,
            ),
            Step::Ended(why) => ("Not paired".into(), why.clone(), vec![quiet("Close", Act::Close), ("Try again".into(), Look::Primary, Act::Retry)], 1),
            Step::Paired { name, .. } => {
                let side = self.running().and_then(|s| s.peers.iter().find(|p| &p.name == name)).map(|p| edge_word(p.side));
                let sub = match (self.running(), side) {
                    (None, _) => "Start one·mk to connect.".into(),
                    (_, Some(e)) => format!("Move the pointer off the {e} edge to cross over."),
                    _ => "It joins the desk in a moment.".into(),
                };
                (format!("Paired with {name}"), sub, vec![], 0)
            }
            Step::Retry(_) => return None,
        };
        let code = match &pair.step {
            Step::Code { code, .. } => Some((code.clone(), pair.since)),
            _ => None,
        };
        let check = matches!(pair.step, Step::Paired { .. }).then_some(pair.since);
        let key = format!("{:?}", std::mem::discriminant(&pair.step));
        Some(SheetView { title, sub, code, check, buttons, focus, appear: pair.opened + motion::space(0.4) as f64, key })
    }

    fn sheet(&mut self, ui: &mut Ui, desk: Rect, now: f64) {
        let (view, leaving) = match self.sheet_view() {
            Some(v) => {
                self.sheet_was = Some((v.clone(), now));
                (v, None)
            }
            None => match &self.sheet_was {
                Some((v, t)) if now - t < 0.16 => (v.clone(), Some(*t)),
                _ => {
                    self.sheet_was = None;
                    self.sheet_top = None;
                    self.sheet_key.clear();
                    return;
                }
            },
        };
        if leaving.is_none() && self.keys_on_sheet(ui) {
            return;
        }
        let width = 380.0f32.min(desk.width() - 40.0);
        let x = desk.center().x - width / 2.0 + 18.0;
        let inner = width - 36.0;
        let sub_g = {
            let mut job = egui::text::LayoutJob::simple(view.sub.clone(), regular(T_BODY), INK2, inner);
            job.sections[0].format.line_height = Some((T_BODY * LEADING).round());
            ui.painter().layout_job(job)
        };
        let code_h = if view.code.is_some() { 60.0 } else { 0.0 };
        // Buttons in a row if they fit, else one under another with the
        // way out last.
        let row = view.buttons.iter().map(|(l, _, _)| button_width(ui, l) + 8.0).sum::<f32>() - 8.0 <= inner;
        let mut view = view;
        if !row && view.buttons.first().is_some_and(|b| b.2 == Act::Close) {
            view.buttons.rotate_left(1);
            view.focus = view.focus.saturating_sub(1);
        }
        let widths: Vec<f32> = view.buttons.iter().map(|(l, _, _)| button_width(ui, l).min(inner)).collect();
        let btn_h = if view.buttons.is_empty() { 0.0 } else if row { 34.0 } else { widths.len() as f32 * 42.0 - 8.0 };
        let title_h = 22.0;
        let height = 18.0 + title_h + 4.0 + sub_g.size().y + code_h + if btn_h > 0.0 { 18.0 + btn_h } else { 0.0 } + 18.0;
        let rect = Rect::from_min_size(pos2(x - 18.0, desk.bottom() - height - 20.0), vec2(width, height));
        self.sheet_top = Some(rect.top());

        // In: rises 16 points and grows from 98%; out: fades and drops 8.
        let (o, dy, s) = match leaving {
            None => {
                let t = motion::progress(now, view.appear, motion::space(0.32), motion::out_expo);
                let o = motion::progress(now, view.appear, motion::fade(0.2), motion::linear);
                (o, motion::space(16.0) * (1.0 - t), lerp_scale(t))
            }
            Some(t0) => {
                let t = motion::progress(now, t0, motion::fade(0.16), motion::linear);
                (1.0 - t, motion::space(8.0) * t, 1.0)
            }
        };
        if o < 1.0 || dy > 0.0 {
            ui.ctx().request_repaint();
        }
        let layer = egui::LayerId::new(egui::Order::Foreground, Id::unique("sheet-layer"));
        let c = rect.center();
        ui.ctx().set_transform_layer(layer, egui::emath::TSTransform::new(c.to_vec2() * (1.0 - s) + vec2(0.0, dy), s));
        let mut child = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(rect));
        child.set_opacity(o);
        if leaving.is_some() || o < 0.5 {
            child.disable();
        }
        // The sheet is modal: what is behind it doesn't get its clicks.
        child.interact(rect, Id::unique("sheet-body"), Sense::CLICK | Sense::DRAG);
        let p = child.painter().clone();
        p.add(egui::epaint::Shadow { offset: [0, 18], blur: 40, spread: 0, color: Color32::from_black_alpha(115) }.as_shape(rect, CornerRadius::same(14)));
        p.rect(rect, CornerRadius::same(14), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let mut y = rect.top() + 18.0;
        let mut tx = x;
        if let Some(t0) = view.check {
            // A tick drawn in, then it holds.
            let k = motion::progress(now, t0, motion::fade(0.24), motion::out_cubic);
            widgets::draw_check(&p, pos2(x + 8.0, y + title_h / 2.0), 16.0, INK, k);
            tx += 26.0;
            if k < 1.0 {
                ui.ctx().request_repaint();
            }
        }
        let title = fit(&p, &view.title, &semibold(T_TITLE), inner - (tx - x));
        p.text(pos2(tx, y + title_h / 2.0), Align2::LEFT_CENTER, &title, semibold(T_TITLE), INK);
        y += title_h + 4.0;
        let sub_h = sub_g.size().y;
        p.galley(pos2(x, y), sub_g, INK2);
        y += sub_h;
        a11y_text(&child, Id::unique("sheet-dialog"), rect, Role::Dialog, &format!("{}. {}", view.title, view.sub), Some(Live::Polite));
        if let Some((code, since)) = &view.code {
            // Digits rise in one by one, in the same rhythm as the other screen.
            let font = mono_medium(28.0);
            let adv = text_width(&child, "0", font.clone()) + 3.0;
            let mut cx = x;
            for (i, ch) in code.chars().enumerate() {
                if i == 3 {
                    cx += 12.0;
                }
                let e = scene::digit_t(i, *since, now);
                p.text(pos2(cx, y + 14.0 + motion::space(8.0) * (1.0 - e)), Align2::LEFT_TOP, ch, font.clone(), INK.gamma_multiply(e));
                cx += adv;
            }
            if now - since < 1.2 {
                ui.ctx().request_repaint();
            }
            let digits: Vec<String> = code.chars().map(String::from).collect();
            let spoken = if digits.len() == 6 { format!("{}, {}", digits[..3].join(" "), digits[3..].join(" ")) } else { digits.join(" ") };
            a11y_text(&child, Id::unique("sheet-code"), Rect::from_min_size(pos2(x, y), vec2(inner, code_h)), Role::Label, &format!("Pairing code {spoken}"), None);
            y += code_h;
        }
        if view.buttons.is_empty() {
            return;
        }
        y += 18.0;
        let mut clicked = None;
        let mut bx = rect.right() - 18.0;
        for (k, ((label, look, act), w)) in view.buttons.iter().zip(&widths).enumerate().rev() {
            let r = if row {
                bx -= w;
                let r = Rect::from_min_size(pos2(bx, y), vec2(*w, 34.0));
                bx -= 8.0;
                r
            } else {
                Rect::from_min_size(pos2(x, y + k as f32 * 42.0), vec2(inner, 34.0))
            };
            let shown = fit(&p, label, &semibold(T_BODY), r.width() - 28.0);
            let id = Id::unique(("sheet-btn", &view.key, k));
            let resp = button(&mut child, r, id, &shown, *look, true, Some(label));
            if k == view.focus && self.sheet_key != view.key && o >= 0.5 {
                resp.request_focus();
                self.sheet_key = view.key.clone();
            }
            if resp.clicked() {
                clicked = Some(act.clone());
            }
        }
        if let Some(act) = clicked {
            self.sheet_act(act, now);
        }
    }

    /// Esc on the sheet. Comparing codes, it means "they don't match", so
    /// the other side hears a no; once confirmed, it waits for Cancel.
    /// Returns true when the sheet went away.
    fn keys_on_sheet(&mut self, ui: &Ui) -> bool {
        let act = match self.pairing.as_ref().map(|p| &p.step) {
            Some(Step::Waiting { .. } | Step::Paired { .. }) => return false,
            Some(Step::Code { .. }) => Act::Decline,
            _ => Act::Close,
        };
        if !ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            return false;
        }
        self.sheet_act(act, self.now);
        !self.sheet_open()
    }

    fn sheet_act(&mut self, act: Act, now: f64) {
        match act {
            Act::Close => {
                self.pairing = None;
                self.pending_drop = None;
            }
            Act::Decline => {
                if let Some(pr) = &mut self.pairing
                    && let Some(proc) = &mut pr.proc
                {
                    proc.answer(false);
                }
            }
            Act::Confirm => {
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
            Act::Retry => {
                self.pairing = None;
                self.start_pairing(now);
                // The desk has already made room: no need to wait again.
                if let Some(p) = &mut self.pairing {
                    p.opened = now - 0.4;
                }
            }
            Act::Send(id) => {
                if let (Some((paths, _)), Some(i)) = (self.pending_drop.take(), self.scene.index(&id)) {
                    self.send_to(i, paths);
                }
            }
        }
    }

    // ---------- the toast ----------

    fn toast_view(&mut self, ui: &mut Ui, desk: Rect, now: f64) {
        let Some(t) = &self.toast else { return };
        let age = (now - t.at) as f32;
        let (enter, exit) = (motion::fade(0.22), motion::fade(0.16));
        if age > t.life {
            self.toast = None;
            return;
        }
        let a_in = motion::out_expo(age / enter);
        let a = a_in.min(((t.life - age) / exit).clamp(0.0, 1.0));
        let text_w = 520.0f32.min(desk.width() - 120.0);
        let g = ui.painter().layout(t.text.clone(), regular(T_BODY), INK, text_w);
        let d = t.detail.as_ref().map(|d| ui.painter().layout(d.clone(), regular(T_META), INK2, text_w));
        let h = g.size().y + d.as_ref().map_or(0.0, |d| d.size().y + 4.0);
        let w = g.size().x.max(d.as_ref().map_or(0.0, |d| d.size().x));
        let size = vec2(w + 46.0, h + 20.0);
        let y = desk.top() + TOP_H + 10.0 + size.y / 2.0 - motion::space(8.0) * (1.0 - a_in);
        let rect = Rect::from_center_size(pos2(desk.center().x, y), size);
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::unique("toast"));
        let mut child = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(rect));
        child.set_opacity(a);
        let resp = child.interact(rect, Id::unique("toast-body"), Sense::CLICK);
        let p = child.painter().clone();
        p.add(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) }.as_shape(rect, CornerRadius::same(10)));
        p.rect(rect, CornerRadius::same(10), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let dot = if t.error { ERROR } else { INK };
        let gh = g.size().y;
        p.circle_filled(pos2(rect.left() + 17.0, rect.top() + 10.0 + 9.0), 3.5, dot);
        p.galley(pos2(rect.left() + 30.0, rect.top() + 10.0), g, INK);
        if let Some(d) = d {
            p.galley(pos2(rect.left() + 30.0, rect.top() + 14.0 + gh), d, INK2);
        }
        let (role, live) = if t.error { (Role::Alert, Live::Assertive) } else { (Role::Status, Live::Polite) };
        let label = format!("{}{}", t.text, t.detail.as_ref().map_or(String::new(), |d| format!(" {d}")));
        a11y_text(&child, Id::unique("toast-node"), rect, role, &label, Some(live));
        let dismiss = resp.clicked() || (t.error && !self.sheet_open() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let t = self.toast.as_mut().expect("shown above");
        if dismiss && age < t.life - exit {
            t.life = age + exit;
        } else if resp.hovered() && age > enter && age < t.life - exit {
            // Held while the pointer is on it, so it can be read.
            t.at = now - enter as f64;
        }
        // Frames only while it moves; while it holds, one wake-up to leave.
        let age = (now - t.at) as f32;
        if age < enter || age > t.life - exit {
            ui.ctx().request_repaint();
        } else {
            ui.ctx().request_repaint_after_secs(t.life - exit - age);
        }
    }

    // ---------- keys ----------

    /// Ctrl+1 to 6 switch tabs (Cmd on a Mac).
    fn shortcuts(&mut self, ui: &Ui) {
        if widgets::locked(ui) {
            return;
        }
        let off = self.off();
        // Ctrl+1 to Ctrl+5: one key per tab, however many there are.
        for (key, (tab, _, _)) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6].into_iter().zip(TABS) {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) && (!off || tab.when_off()) {
                self.set_tab(tab);
            }
        }
    }
}

/// The sheet grows from 98% as it arrives.
fn lerp_scale(t: f32) -> f32 {
    if motion::reduced() { 1.0 } else { 0.98 + 0.02 * t }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let now = ui.input(|i| i.time);
        self.now = now;
        if self.size.x > 0.0 {
            // Some desktops (GNOME on Wayland) ignore the size asked for at
            // start; asking again once the window is mapped works.
            if (ui.ctx().content_rect().size() - self.size).length() < 2.0 || now > 1.0 {
                self.size = egui::Vec2::ZERO;
            } else if now > 0.15 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(self.size));
                ui.ctx().request_repaint_after_secs(0.1);
            } else {
                ui.ctx().request_repaint_after_secs(0.05);
            }
        }
        #[cfg(debug_assertions)]
        debug::hooks(self, now);
        widgets::track_focus_mode(ui.ctx());
        self.arrowed = self.arrow_down;
        // Only along a row: Up and Down move between rows without choosing.
        self.arrow_down = ui.input(|i| i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowRight));
        if self.arrow_down {
            // Focus moves at the end of this frame; the choice is made on the next.
            ui.ctx().request_repaint();
        }
        self.handle_events(now);
        if self.off() && !self.tab.when_off() {
            self.set_tab(Tab::Desk);
        }
        widgets::set_locked(ui.ctx(), self.sheet_open());
        self.shortcuts(ui);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, GROUND);
        let panel = Rect::from_min_max(pos2(full.left(), full.bottom() - PANEL_H), full.max);
        let desk = Rect::from_min_max(full.min, pos2(full.right(), panel.top()));
        let top = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
        // Files let go over a page without the desk: ask where they go.
        if !self.tab.desk() {
            let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
            if !dropped.is_empty() {
                self.set_tab(Tab::Files);
                self.drop_files(dropped, None, now);
            }
        }
        // A new page rises in.
        let e = motion::progress(now, self.tab_at, motion::fade(0.18), motion::out_cubic);
        if e < 1.0 {
            ui.ctx().request_repaint();
        }
        let page = |ui: &mut Ui, r: Rect, salt: &str| {
            let mut child = ui.new_child(UiBuilder::new().max_rect(r.translate(vec2(0.0, motion::space(4.0) * (1.0 - e)))).id_salt(salt.to_owned()));
            child.set_clip_rect(r);
            child.set_opacity(e);
            child
        };
        match self.tab {
            t if t.desk() => self.desk(ui, desk, now),
            Tab::Layout => {
                let mut child = page(ui, desk, "plan");
                self.plan(&mut child, desk, now);
            }
            _ => {}
        }
        let body = Rect::from_min_max(pos2(full.left(), top.bottom()), full.max);
        match self.tab {
            Tab::Health => {
                let mut child = page(ui, body, "health");
                self.health(&mut child, body, now);
            }
            Tab::Advanced => {
                let mut child = page(ui, body, "advanced");
                self.advanced(&mut child, body);
            }
            _ => {}
        }
        self.top_bar(ui, top, now);
        if !matches!(self.tab, Tab::Health | Tab::Advanced) {
            self.panel(ui, panel, now);
        }
        widgets::set_locked(ui.ctx(), false);
        // The sheet sits over the panel, leaving the desk room to show
        // what is being paired.
        self.sheet(ui, full, now);
        self.toast_view(ui, desk, now);
        #[cfg(debug_assertions)]
        debug::shot(ui.ctx(), now);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        GROUND.to_normalized_gamma_f32()
    }
}

/// Development aids for screenshots, in debug builds only:
/// - `ONE_MK_APP_SHOT=file.ppm` saves the window after
///   `ONE_MK_APP_SHOT_AFTER` seconds (default 3) and quits;
/// - `ONE_MK_APP_PAIR=1` starts pairing; `ONE_MK_APP_TAB=files|keyboard`
///   opens a tab; `ONE_MK_APP_SELECT=name` selects a computer (`me`: this one);
/// - `ONE_MK_APP_DROP=n` drops n files where it's unclear who gets them;
/// - `ONE_MK_APP_DRAG=1` carries the first other computer across this one;
/// - `ONE_MK_APP_FOCUS=1` puts the keyboard on the position control;
/// - `ONE_MK_APP_TOAST=1` shows an error toast; `ONE_MK_APP_AUTOSTART=1`
///   shows start at login as on;
/// - `ONE_MK_APP_CONFIRM=1` says the codes match a second after they
///   show, and keeps the "paired" sheet up.
/// - `ONE_MK_APP_TAB=layout|health|advanced` (and files, keyboard);
///   `ONE_MK_APP_PLAN_DRAG=px` carries the chosen computer along its edge
///   on the plan; `ONE_MK_APP_PROFILE=name` opens a profile's options;
///   `ONE_MK_APP_DOCTOR=file.json` shows that report instead of running
///   `one-mk doctor`; `ONE_MK_APP_ZOOM=2` draws everything twice as big
///   (a high-density screen); `ONE_MK_APP_SIZE=WxH` (main.rs).
/// - `ONE_MK_REDUCE_MOTION=1` works in every build (see `motion`).
#[cfg(debug_assertions)]
mod debug {
    use super::*;
    use std::io::Write;

    fn var(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn once(ctx: &egui::Context, key: &str) -> bool {
        let id = Id::unique(("debug-once", key));
        let done = ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
        ctx.data_mut(|d| d.insert_temp(id, true));
        !done
    }

    pub fn hooks(app: &mut App, now: f64) {
        let ctx = app.ctx.clone();
        let ready = app.running().is_some();
        if app.status.is_some() && var("ONE_MK_APP_PAIR").is_some() && once(&ctx, "pair") {
            app.start_pairing(now);
        }
        if ready && let Some(name) = var("ONE_MK_APP_SELECT") {
            let id = if name == "me" { Some(name) } else { app.running().and_then(|s| s.peers.iter().find(|p| p.name == name)).map(|p| p.id.clone()) };
            if let Some(id) = id
                && once(&ctx, "select")
            {
                app.select(id);
            }
        }
        if ready && now > 1.0 && let Some(n) = var("ONE_MK_APP_DROP").and_then(|n| n.parse::<usize>().ok()) && once(&ctx, "drop") {
            app.pending_drop = Some(((0..n).map(|i| PathBuf::from(format!("/tmp/file{i}"))).collect(), now));
        }
        if ready && var("ONE_MK_APP_TOAST").is_some() && now > 1.0 && once(&ctx, "toast") {
            app.toast_error("Couldn't move Studio PC to the left. The desk went back to where it was; try again.", Some("one-mk didn't answer within 3 seconds".into()), now);
        }
        if ready && var("ONE_MK_APP_FOCUS").is_some() && now > 1.0 && once(&ctx, "focus") {
            ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
            if let Some(id) = app.selected_id() {
                let target = match app.tab {
                    Tab::Layout => Id::unique(("plan-device", &id)),
                    _ => Id::unique(("position", &id)).with(1usize),
                };
                ctx.memory_mut(|m| m.request_focus(target));
            }
        }
        if ready && let Some(name) = var("ONE_MK_APP_PROFILE") && once(&ctx, "profile") {
            app.profile_open = Some(name);
            app.profile_scroll = true;
        }
        if var("ONE_MK_APP_AUTOSTART").is_some() {
            app.autostart = Some(true);
        }
        if var("ONE_MK_APP_CONFIRM").is_some()
            && app.pairing.as_ref().is_some_and(|p| matches!(p.step, Step::Code { .. }) && now - p.since > 1.2)
        {
            app.sheet_act(Act::Confirm, now);
        }
        // A drag: pick up the first other computer and carry it across.
        if ready && var("ONE_MK_APP_DRAG").is_some() && app.scene.devices.len() > 1 && now > 1.5 {
            let i = 1;
            if once(&ctx, "drag") {
                let d = &mut app.scene.devices[i];
                d.lift.go(2.5, now, 0.12, motion::out_cubic);
                d.pop.go(1.02, now, 0.12, motion::out_cubic);
                app.drag = Some(Drag::Device { i, start: Pos2::ZERO, x0: d.pos.x });
            }
            if let Some(Drag::Device { x0, .. }) = app.drag {
                let t = ((now - 1.5) / 1.5).min(1.0) as f32;
                app.scene.devices[i].pos.x = x0 + t * 150.0;
                app.make_room(i);
            }
        }
    }

    pub fn shot(ctx: &egui::Context, now: f64) {
        let Some(path) = std::env::var_os("ONE_MK_APP_SHOT") else { return };
        let after: f64 = std::env::var("ONE_MK_APP_SHOT_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(3.0);
        let asked = ctx.data(|d| d.get_temp::<bool>(Id::unique("shot-asked")).unwrap_or(false));
        ctx.request_repaint();
        if now < after || asked {
            return;
        }
        ctx.data_mut(|d| d.insert_temp(Id::unique("shot-asked"), true));
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
    fn below_stands_beside_this_computer_not_in_front_of_it() {
        let s = slots(29.0, &[("b".into(), Side::Bottom, 29.0)]);
        assert!(s["b"].0.z > 0.0, "in front");
        assert!(s["b"].0.x.abs() > 20.0, "but to one side");
    }

    #[test]
    fn sentences_name_the_side() {
        assert_eq!(to_side(Side::Left), "to the left");
        assert_eq!(edge_word(Side::Top), "top");
        assert_eq!(items(1), "1 item");
        assert_eq!(items(3), "3 items");
    }
}
