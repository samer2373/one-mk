//! The window: a top bar, the desk, a panel of facts and switches under it,
//! and a sheet for pairing and for choosing where dropped files go.
//! Everything shown comes from the running daemon; everything changed goes
//! through it.

mod cells;
mod clipboard_history;
mod keys;
mod links;
mod pages;
mod place;
mod shell;
mod desk_cards;
mod typing;
mod updates;
mod settings_controls;
mod palette;
mod wallpaper;

use crate::daemon::{Event, Job, Link};
use crate::motion::{self, tween};
use crate::pairing::{Pairing, Step};
use crate::scene::{self, Cargo, Device, Kind, SPACING, Scene, v};
use crate::style::*;
use crate::rtt;
use crate::glass::Glass;
use crate::view::{Check, Prefs};
use crate::widgets::{self, Look, Pills, a11y_text, button, button_width, label_help, label_help_inline, mix, para, switch_help, text_width};
use egui::accesskit::{Live, Role};
use egui::{Align2, Color32, CornerRadius, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use one_mk_ctl::status::{PeerStatus, Status, on_your, os_name};
use one_mk_proto::{Desktop, Edge, Side, os};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

const TOP_H: f32 = 56.0;
const PANEL_H: f32 = 196.0;
/// Below this height (plus 22 pt for each other computer) the desk shows
/// its computers as named chips: monitors any smaller can't carry names.
const COMPACT_BELOW: f32 = 128.0;
/// Inner padding of the panel's cells.
const PAD: f32 = 20.0;
const GHOST: &str = "\u{0}pairing";
/// Wallpapers for paired computers: low chroma, about equal lightness.
/// Each computer's wallpaper on the desk: a light colour and a deep one,
/// as desktop pictures are (dusk violet, lagoon, ember, ocean); deep
/// enough that white names on them stay readable.
const WALLS: [[u32; 2]; 4] = [[0x7B68E0, 0x191338], [0x2E9F96, 0x08262D], [0xD9714A, 0x34101E], [0x3D7FE6, 0x0A1838]];
const ME_WALL: [u32; 2] = [0x3D7FE6, 0x0A1838];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Desk,
    Layout,
    Files,
    Wallpaper,
    Keys,
    Health,
    Advanced,
}
/// Each tab, and a shorter name for narrow windows. Settings most people
/// never change (Ctrl and Cmd, gaming mode, edge rules) live in Advanced.
const TABS: [(Tab, &str, &str); 7] = [
    (Tab::Desk, "Desk", "Desk"),
    (Tab::Layout, "Desk", "Desk"),
    (Tab::Files, "Clipboard & files", "Files"),
    (Tab::Wallpaper, "Wallpaper", "Wallpaper"),
    (Tab::Keys, "Keys & mouse", "Keys"),
    (Tab::Health, "Health", "Health"),
    (Tab::Advanced, "Settings", "Settings"),
];

impl Tab {
    /// Tabs that show the desk of computers (where they can be dragged
    /// into place).
    fn desk(self) -> bool {
        matches!(self, Tab::Desk | Tab::Files | Tab::Layout)
    }
    /// Tabs that mean something while oneMK is off.
    fn when_off(self) -> bool {
        matches!(self, Tab::Desk | Tab::Files | Tab::Wallpaper | Tab::Keys | Tab::Health | Tab::Advanced)
    }
}
const PLACES: [(Side, &str); 4] = [(Side::Left, "Left"), (Side::Right, "Right"), (Side::Top, "Above"), (Side::Bottom, "Below")];
const SWAPS: [(&str, &str); 3] = [("auto", "Auto"), ("on", "Always swap"), ("off", "Never swap")];
/// One computer's row in the Ctrl and Cmd card.
const SWAP_ROW: f32 = 36.0;

/// A Ctrl and Cmd row `inner_w` wide: one line when the full choices fit
/// beside the name, else the name above and the choices across the row.
pub(crate) fn swap_row_h(inner_w: f32) -> f32 {
    if inner_w * 0.6 >= 264.0 { SWAP_ROW } else { 66.0 }
}

enum Drag {
    /// `place` is set when both computers' monitors are known: then the
    /// drag puts it at an exact place beside this computer's monitors.
    Device { i: usize, start: Pos2, x0: f32, place: Option<Box<Placing>> },
    Orbit,
}

/// Where a computer's (0,0) sits in our pixels, if it was placed there.
type Offset = Option<(i32, i32)>;

/// A computer being dragged to a place beside this one's monitors.
struct Placing {
    fit: scene::Fit,
    mine: Desktop,
    /// Our monitors' free edges, every side.
    edges: Vec<Edge>,
    /// Its monitors' span, in its pixels.
    peer: one_mk_proto::Rect,
    /// That span's top left from the pointer, in our pixels.
    grab: (f64, f64),
    /// Where it goes if let go now: side and offset.
    now: (Side, (i32, i32)),
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
    port: Option<u16>,
}

/// What the sheet does when one of its buttons is chosen.
#[derive(Clone, Debug, PartialEq)]
enum Act {
    Close,
    Decline,
    Confirm,
    Retry,
    Send(String),
    Forget(String, String),
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
    /// What failed, when doing it again is safe: "Try again" does that.
    retry: Option<Job>,
}

/// What a cell of the panel shows. When it changes, the old one fades
/// out and the new one rises in.
#[derive(Clone, PartialEq, Debug)]
enum Cell {
    Head(String, String),
    Inspector(Option<String>),
    /// This computer's switches.
    Controls,
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
    glass: Glass,
    /// A computer has been dragged into place: the drag hint goes.
    dragged_once: bool,
    /// What was behind the glass last frame, and whether the desk moved.
    backdrop_was: u64,
    scene_moving: bool,
    /// Counts status reports, so the glass knows the desk may look different.
    status_seen: u64,
    update_available: Option<String>,
    update_output: Option<String>,
    update_ui: updates::UpdateUi,
    palette: palette::Palette,
    wallpaper_ui: wallpaper::WallpaperUi,
    files_ui: cells::FilesUi,
    link: Link,
    rx: Receiver<Event>,
    /// `None` until the first look at the daemon.
    status: Option<Option<Status>>,
    clipboard_history: Option<Result<Vec<crate::daemon::HistoryItem>, String>>,
    clipboard_reveal: Option<(u64, String)>,
    clipboard_clear_confirm: bool,
    clipboard_reveal_epoch: u64,
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
    /// A computer (id, name) the sheet is asking whether to remove, and since when.
    removing: Option<(String, String, f64)>,
    /// The sheet as last drawn, and when it started to leave.
    sheet_was: Option<(SheetView, f64)>,
    sheet_top: Option<f32>,
    sheet_key: String,
    toast: Option<Toast>,
    latency: HashSet<String>,
    /// Round trips of the connected computers, while a graph is on screen.
    rtts: crate::rtt::Rtts,
    /// Set while a screenshot's scripted history stands in for live samples.
    rtt_frozen: Option<f64>,
    failed_measure: HashSet<String>,
    measuring: Option<String>,
    /// Sides (and offsets) chosen here and not yet reported back by the
    /// daemon, and when.
    pending_side: HashMap<String, (Side, Offset, f64)>,
    /// Layout learning's question (computer id, side) answered here, and
    /// hidden until the daemon stops asking it.
    answered: Option<(String, Side)>,
    /// Computers placed where no screen of theirs touches ours.
    apart: HashSet<String>,
    /// The banner over the desk as drawn this frame (see `refresh_notice`).
    notice: desk_cards::NoticeHold,
    /// Where the toast goes in the desk's band this frame (None: off the desk).
    toast_slot: Option<Rect>,
    /// How tall each short cell's contents were last drawn (cards are
    /// sized to them, not stretched to the tallest).
    cell_h: HashMap<&'static str, f32>,
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
    slots: [Slot; 4],
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
    pending_lock: Option<(bool, f64)>,
    /// Broadcast asked to start (true) or stop, and when: shown so until
    /// the daemon says, or 5 s pass.
    pending_broadcast: Option<(bool, f64)>,
    /// The Keys & mouse page.
    keys: keys::KeysUi,
    /// Text being typed into fields, by field.
    drafts: HashMap<String, String>,
    /// The profile whose options are open, and whether to scroll to it.
    profile_open: Option<String>,
    profile_scroll: bool,
    wifi: Option<one_mk_ctl::wifi::Watch>,
    /// When the tab last changed (the page fades in).
    tab_at: f64,
    fonts: Fallbacks,
    size: egui::Vec2,
    now: f64,
    ctx: egui::Context,
    lockup: egui::TextureHandle,
    /// The macOS permission guide (guide.rs); idle unless started.
    pub guide: crate::guide::Overlay,
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
/// entry is (id, side, half width); this computer is at the origin, and
/// with what stands at an offset it spans `span` across the desk.
fn slots(span: (f32, f32), peers: &[(String, Side, f32)]) -> HashMap<String, (scene::V3, f32)> {
    let mut out = HashMap::new();
    let (mut left, mut right) = span;
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
            Side::Bottom => (v(span.1 + (k - 0.5) * SPACING, 0.0, 40.0), 0.8),
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
            p.circle_filled(at, 3.0, OK);
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
        crate::chrome::install(cc);
        motion::set_reduced(motion::detect());
        let (tx, rx) = std::sync::mpsc::channel();
        let link = Link::new(home, tx, cc.egui_ctx.clone())?;
        link.watch();
        link.first_run();
        let mut scene = Scene::default();
        scene.top_clear = 0.0;
        let mut me = Device::new("me", "This computer", me_kind(), true, ME_WALL);
        me.arrive(v(0.0, 0.0, 0.0), 0.0, 0.0);
        scene.devices.push(me);
        scene.cursor_on = "me".into();
        #[allow(unused_mut)]
        let mut tab = Tab::Desk;
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        match std::env::var("ONE_MK_APP_TAB").as_deref() {
            Ok("files") => tab = Tab::Files,
            Ok("wallpaper") => tab = Tab::Wallpaper,
            Ok("keyboard") => tab = Tab::Advanced,
            Ok("layout") => tab = Tab::Layout,
            Ok("health") => tab = Tab::Health,
            Ok("advanced") => tab = Tab::Advanced,
            Ok("keys") => tab = Tab::Keys,
            _ => {}
        }
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(z) = std::env::var("ONE_MK_APP_ZOOM").ok().and_then(|z| z.parse::<f32>().ok()) {
            cc.egui_ctx.set_zoom_factor(z);
        }
        let slot = || Slot { cell: None, was: None, at: f64::NEG_INFINITY };
        let glass = Glass::default();
        widgets::set_glass(&cc.egui_ctx, glass.clone());
        Ok(Self {
            glass,
            // Dragged once, or the hint already shown in full once: no hint.
            dragged_once: link.dir.join(".window-dragged").exists() || link.dir.join(".window-hinted").exists(),
            backdrop_was: 0,
            scene_moving: false,
            status_seen: 0,
            link,
            rx,
            status: None,
            clipboard_history: None,
            clipboard_reveal: None,
            clipboard_clear_confirm: false,
            clipboard_reveal_epoch: 0,
            autostart: None,
            update_available: None,
            update_output: None,
            update_ui: updates::UpdateUi::default(),
            palette: palette::Palette::default(),
            wallpaper_ui: wallpaper::WallpaperUi::default(),
            files_ui: cells::FilesUi::default(),
            scene,
            tab,
            drag: None,
            drag_side: None,
            menu_for: None,
            pairing: None,
            pending_drop: None,
            removing: None,
            sheet_was: None,
            sheet_top: None,
            sheet_key: String::new(),
            toast: None,
            latency: HashSet::new(),
            rtts: Default::default(),
            rtt_frozen: None,
            failed_measure: HashSet::new(),
            measuring: None,
            pending_side: HashMap::new(),
            answered: None,
            apart: HashSet::new(),
            notice: desk_cards::NoticeHold::default(),
            toast_slot: None,
            cell_h: HashMap::new(),
            pending_swap: HashMap::new(),
            hop: None,
            just_paired: None,
            active: "me".into(),
            busy: vec![],
            selected: None,
            slots: [slot(), slot(), slot(), slot()],
            drop_pos_fresh: false,
            arrowed: false,
            arrow_down: false,
            prefs: Prefs::default(),
            doctor: None,
            copied: None,
            pending_lock: None,
            pending_broadcast: None,
            keys: keys::KeysUi::new(),
            drafts: HashMap::new(),
            profile_open: None,
            profile_scroll: false,
            wifi: None,
            tab_at: f64::NEG_INFINITY,
            fonts: Fallbacks::default(),
            size: vec2(size[0], size[1]),
            now: 0.0,
            ctx: cc.egui_ctx.clone(),
            lockup: lockup_texture(&cc.egui_ctx),
            guide: crate::guide::Overlay::from_env(),
        })
    }

    fn running(&self) -> Option<&Status> {
        self.status.as_ref().and_then(Option::as_ref)
    }

    fn off(&self) -> bool {
        matches!(self.status, Some(None))
    }

    fn toast(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some(Toast { text: text.into(), detail: None, error: false, at: now, life: 3.5, retry: None });
    }

    /// An error says what went wrong and what to do; the raw reason goes
    /// underneath. It stays until dismissed, or for ten seconds.
    fn toast_error(&mut self, text: impl Into<String>, detail: Option<String>, now: f64) {
        let detail = detail.map(|d| crate::view::window_issue(d.trim())).filter(|d| !d.is_empty());
        self.toast = Some(Toast { text: text.into(), detail, error: true, at: now, life: 10.0, retry: None });
    }

    /// Whether `job` can simply be run again from an error toast.
    fn retryable(job: &Job) -> bool {
        matches!(job, Job::Layout { .. } | Job::Swap { .. } | Job::TakeBack | Job::Start | Job::Autostart(_) | Job::Set { .. } | Job::Open)
    }

    /// Runs a failed job again, as the control that asked for it did.
    fn retry(&mut self, job: Job) {
        match job.clone() {
            Job::Layout { id, side, .. } => self.choose_side(&id, side, true),
            Job::Swap { id, name, swap } => {
                self.pending_swap.insert(id.clone(), swap.clone());
                self.busy.push(job);
                self.link.set_swap(&id, &name, &swap);
                self.sync_scene();
            }
            Job::TakeBack => {
                self.busy.push(job);
                self.link.take_back();
            }
            Job::Start => {
                self.busy.push(job);
                self.link.start();
            }
            Job::Autostart(on) => {
                self.busy.push(job);
                self.autostart = Some(on);
                self.link.set_autostart(on);
            }
            Job::Set { what, args } => {
                self.busy.push(job);
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                self.link.set(&what, &args);
            }
            Job::Open => {
                self.busy.push(job);
                self.link.open_downloads();
            }
            _ => {}
        }
    }

    fn handle_events(&mut self, now: f64) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::ClipboardHistory(history) => {
                    self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
                    self.clipboard_reveal = None;
                    if let Ok(items) = &history { self.fonts.cover(&self.ctx, items.iter().map(|item| item.preview.as_str())); }
                    self.clipboard_history = Some(history);
                }
                Event::ClipboardReveal(id, epoch, content) => {
                    if self.clipboard_reveal_epoch == epoch && self.tab == Tab::Files
                        && self.clipboard_history.as_ref().and_then(|items| items.as_ref().ok()).is_some_and(|items| items.iter().any(|item| item.id == id))
                        && let Ok(content) = content {
                        self.fonts.cover(&self.ctx, std::iter::once(content.as_str()));
                        self.clipboard_reveal = Some((id, content));
                    }
                }
                Event::Status(s) => {
                    let s = *s;
                    self.status_seen += 1;
                    // Round trips: what the daemon says now, or nothing once it is gone.
                    match (&s, self.rtt_frozen) {
                        (Some(st), None) => self.rtts.record(now, &st.links),
                        (None, None) => self.rtts = Default::default(),
                        _ => {}
                    }
                    let changed = self.running().map(|status| (status.clipboard_history, status.clipboard_history_revision)) != s.as_ref().map(|status| (status.clipboard_history, status.clipboard_history_revision));
                    if let Some(st) = &s { self.guide.on_status(&st.input, now); }
                    self.status = Some(s);
                    if self.tab == Tab::Files && (changed || self.clipboard_history.is_none()) { self.link.history(); }
                    let names: Vec<String> = self.running().map_or(vec![], |st| std::iter::once(st.name.clone()).chain(st.peers.iter().map(|p| p.name.clone())).collect());
                    self.fonts.cover(&self.ctx, names.iter().map(String::as_str));
                    // A measurement belongs to one connection: once that
                    // computer goes offline (or oneMK stops) it means nothing.
                    let online: Vec<String> = self.running().map_or(vec![], |s| s.peers.iter().filter(|p| p.online).map(|p| p.id.clone()).collect());
                    self.latency.retain(|id| online.contains(id));
                    self.failed_measure.retain(|id| online.contains(id));
                    self.sync_scene();
                }
                Event::Links(links) if self.rtt_frozen.is_none() => self.rtts.record(now, &links),
                Event::Links(_) => {}
                Event::Autostart(on) => self.autostart = Some(on),
                Event::Setup(report) => match report.problems().first() {
                    Some(problem) => self.toast_error(format!("Setup didn't finish. {problem}"), None, now),
                    None if report.changed() => self.toast("oneMK is set up and starts when you log in.", now),
                    None => {}
                },
                Event::UpdateStatus(status) => {
                    self.update_available.clone_from(&status.available);
                    self.update_ui.status = Some(status);
                }
                Event::UpdateOutput(output) => self.update_output = Some(output),
                Event::Latency(l) => {
                    self.measuring = None;
                    self.failed_measure.remove(&l.peer);
                    // Show the measurement where it was taken: one crossing, labelled.
                    self.scene.latency_label = Some("Connection check".into());
                    if self.active == "me" && self.scene.flight.is_none() {
                        self.scene.fly("me", &l.peer, Cargo::Probe);
                    }
                    self.latency.insert(l.peer.clone());
                }
                Event::Done { what, error } => {
                    self.busy.retain(|j| j != &what);
                    self.finished(what, error, now);
                }
                Event::Prefs(p) => {
                    self.prefs = *p;
                    self.keys.check_due = true;
                    // What was being dragged or typed is saved now.
                    if !self.busy.iter().any(|j| matches!(j, Job::Tune { .. })) {
                        self.keys.drafts.clear();
                    }
                }
                Event::Doctor(d) => self.doctor = Some((d, now)),
                Event::KeymapCheck(r) => self.keys.problems = Some(r),
                Event::InputDevices(r) => self.keys.devices = Some(r),
            }
        }
        // Health is open and nothing ran yet: run the checks.
        if self.tab == Tab::Health && self.doctor.is_none() && !self.busy.contains(&Job::Doctor) && self.status.is_some() {
            self.doctor_on_open();
        }
        if self.pending_lock.is_some_and(|(_, t)| now - t > 5.0) {
            self.pending_lock = None;
        }
        if self.pending_broadcast.is_some_and(|(_, t)| now - t > 5.0) {
            self.pending_broadcast = None;
        }
        self.pairing_steps(now);
        // A side the daemon never confirmed: show what it really has.
        let before = self.pending_side.len();
        self.pending_side.retain(|_, (_, _, t)| now - *t < 5.0);
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
                Job::Tune { area, what, .. } => self.tuned(area, &what, None),
                Job::UpdateCheck => self.update_ui.checked = true,
                _ => {}
            }
            return;
        };
        let e = Some(e);
        let again = Self::retryable(&what).then(|| what.clone());
        self.finished_error(what, e, now);
        if let (Some(t), Some(job)) = (self.toast.as_mut(), again)
            && t.error
            && t.at == now
        {
            t.retry = Some(job);
        }
    }

    fn finished_error(&mut self, what: Job, e: Option<String>, now: f64) {
        match what {
            Job::Update => { self.update_output = e; self.update_ui.failed = true; }
            Job::UpdateCheck => self.toast_error("Couldn't check for updates.", e, now),
            Job::UpdateSettings => self.toast_error("Couldn't save update preferences.", e, now),
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
            Job::TakeBack => self.toast_error("Couldn't take back control. Press a key on this computer, then try again.", e, now),
            Job::Autostart(on) => {
                let word = if on { "on" } else { "off" };
                self.toast_error(format!("Couldn't turn {word} start at login."), e, now);
            }
            Job::Start => self.toast_error("oneMK didn't start.", e, now),
            Job::Forget(name) => self.toast_error(format!("Couldn't remove {name}. Try again."), e, now),
            Job::Stop => self.toast_error("Couldn't stop oneMK. If it was started before an update, quit it once from Activity Monitor or Task Manager; after that Stop works.", e, now),
            Job::Open => self.toast_error("Couldn't open the received files folder.", e, now),
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
            Job::Broadcast(on) => {
                self.pending_broadcast = None;
                let what = if on { "start broadcasting to every computer" } else { "stop broadcasting" };
                self.toast_error(format!("Couldn't {what}."), e, now);
            }
            Job::LockAll => self.toast_error("Couldn't lock the screens.", e, now),
            Job::Clip { send, name, .. } => {
                let what = if send { format!("send this clipboard to {name}") } else { format!("get {name}'s clipboard") };
                self.toast_error(format!("Couldn't {what}."), e, now);
            }
            Job::Set { what, args } => {
                // Show what is really saved.
                self.prefs = self.link.prefs();
                if args.first().is_some_and(|arg| arg == "hotkeys") {
                    self.toast_error(settings_controls::shortcut_error(e.as_deref().unwrap_or("")), None, now);
                } else { self.toast_error(format!("Couldn't change {what}."), e, now); }
            }
            Job::Doctor => {}
            // Shown where the change was made, not as a toast.
            Job::Tune { area, what, .. } => self.tuned(area, &what, e),
        }
    }

    fn pairing_steps(&mut self, now: f64) {
        let mut finished: Option<(String, String)> = None;
        if let Some(p) = &mut self.pairing {
            while let Some(step) = p.proc.as_ref().and_then(|c| c.rx.try_recv().ok()) {
                match step {
                    Step::Invitation(link) => { self.ctx.data_mut(|data| data.insert_temp(Id::unique("invitation-link"), link)); }
                    Step::Searching { me, port } => { p.me = me; p.port = port; }
                    Step::Hint => p.hint = true,
                    Step::Retry(why) => {
                        p.retry = Some(why);
                        p.hint = false;
                        p.step = Step::Searching { me: p.me.clone(), port: p.port };
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
            // Pairing is for connecting: bring oneMK up if it isn't.
            if matches!(self.status, Some(None)) && !self.busy.contains(&Job::Start) {
                self.busy.push(Job::Start);
                self.link.start();
                self.toast(format!("Paired with {name}. Starting oneMK…"), now);
            }
            let dir = self.link.dir.clone();
            std::thread::spawn(move || {
                let _ = one_mk_ctl::client::request(&dir, one_mk_ctl::client::Request::Reload, std::time::Duration::from_secs(3));
            });
            self.sync_scene();
        }
        // The success sheet holds a moment, then goes.
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let hold = if std::env::var_os("ONE_MK_APP_CONFIRM").is_some() { 60.0 } else { 1.15 };
        #[cfg(not(any(debug_assertions, feature = "design-evidence")))]
        let hold = 1.15;
        if self.pairing.as_ref().is_some_and(|p| matches!(p.step, Step::Paired { .. }) && now - p.since > hold) {
            self.pairing = None;
        }
    }

    fn side_shown(&self, id: &str, reported: Side) -> Side {
        self.place_shown(id, (reported, None)).0
    }

    /// Side and offset to show: where a drag would put it, or what was
    /// chosen here but not yet confirmed, or what the daemon reports.
    fn place_shown(&self, id: &str, reported: (Side, Option<(i32, i32)>)) -> (Side, Option<(i32, i32)>) {
        if let Some(Drag::Device { i, place: Some(pl), .. }) = &self.drag
            && self.scene.devices.get(*i).is_some_and(|d| d.id == id)
        {
            return (pl.now.0, Some(pl.now.1));
        }
        match &self.drag_side {
            Some((d, side)) if d == id => (*side, None),
            _ => self.pending_side.get(id).map_or(reported, |(s, o, _)| (*s, *o)),
        }
    }

    /// Makes the desk match what the daemon reports (and what was chosen
    /// here but not yet confirmed).
    fn sync_scene(&mut self) {
        let now = self.now;
        let status = self.running().cloned();
        let off = self.off();
        let peers: Vec<_> = status.as_ref().map(|s| s.peers.clone()).unwrap_or_default();
        let offset_of = |id: &str| status.as_ref().and_then(|s| s.offset_of(id));
        // The daemon has caught up with a place or a swap chosen here.
        self.pending_side.retain(|id, (side, offset, _)| peers.iter().any(|p| &p.id == id && (p.side, offset_of(id)) != (*side, *offset)));
        self.pending_swap.retain(|id, swap| peers.iter().any(|p| &p.id == id && &p.swap != swap));
        if let Some(s) = &status
            && self.pending_lock.is_some_and(|(on, _)| on == s.pointer_locked)
        {
            self.pending_lock = None;
        }
        if let Some(s) = &status
            && self.pending_broadcast.is_some_and(|(on, _)| on != s.broadcast.is_empty())
        {
            self.pending_broadcast = None;
        }
        let listed = self.just_paired.as_ref().is_some_and(|(j, _)| peers.iter().any(|p| &p.id == j));
        if listed || self.just_paired.as_ref().is_some_and(|(_, t)| now - t > 6.0) {
            self.just_paired = None;
        }
        // A computer with several monitors stands as a wall of them, laptop or not.
        let kind_of = |o: u8, screens: &[one_mk_proto::Rect]| if o == os::MACOS && screens.len() < 2 { Kind::Laptop } else { Kind::Monitor };
        let me = &mut self.scene.devices[0];
        me.screens = status.as_ref().map(|s| s.screens.clone()).unwrap_or_default();
        me.kind = if me.screens.len() > 1 { Kind::Monitor } else { me_kind() };
        let me_half = scene::half_width(me.kind, &me.screens);
        // A computer with an offset stands there, at our scale; the rest
        // stand in a row on each side, clear of it.
        let mine = (!me.screens.is_empty()).then(|| Desktop::new(me.screens.clone()));
        let (mut placed, mut span) = (HashMap::new(), (-me_half, me_half));
        self.apart.clear();
        let mut layout: Vec<(String, Side, f32)> = Vec::new();
        for p in &peers {
            let kind = kind_of(p.os, &p.screens);
            let (side, offset) = self.place_shown(&p.id, (p.side, offset_of(&p.id)));
            match offset.zip(self.scene.fit(kind, &p.screens)) {
                Some((o, fit)) => {
                    let (pos, half) = (fit.pos(o), scene::half_width(kind, &p.screens) * fit.scale);
                    span = (span.0.min(pos.x - half), span.1.max(pos.x + half));
                    placed.insert(p.id.clone(), (pos, fit.scale));
                    if mine.as_ref().is_some_and(|m| one_mk_proto::touching(m, &Desktop::new(p.screens.clone()), side, o).is_empty()) {
                        self.apart.insert(p.id.clone());
                    }
                }
                None => layout.push((p.id.clone(), side, scene::half_width(kind, &p.screens))),
            }
        }
        if let Some((id, _)) = &self.just_paired
            && let Some(i) = self.scene.index(id)
        {
            let d = &self.scene.devices[i];
            let side = if d.target.x < 0.0 { Side::Left } else { Side::Right };
            layout.push((id.clone(), side, scene::half_width(d.kind, &d.screens) * d.scale));
        }
        if self.scene.index(GHOST).is_some() {
            let right = layout.iter().filter(|(_, s, _)| *s == Side::Right).count();
            let left = layout.iter().filter(|(_, s, _)| *s == Side::Left).count();
            layout.push((GHOST.into(), if right <= left { Side::Right } else { Side::Left }, scene::half_width(Kind::Monitor, &[]) * 0.86));
        }
        let mounted: HashSet<String> = placed.keys().cloned().collect();
        let mut places = slots(span, &layout);
        places.extend(placed);
        let me = &mut self.scene.devices[0];
        if let Some(s) = &status {
            me.name = s.name.clone();
        }
        me.online = !off;
        me.set_subtitle(if off { "oneMK is off" } else if self.tab == Tab::Files { "Here" } else { "This computer" }, now);

        // New computers come in staggered by how far they are from this one.
        let opening = self.scene.opening(now);
        for (n, p) in peers.iter().enumerate() {
            let i = match self.scene.index(&p.id) {
                Some(i) => i,
                None => {
                    let mut d = Device::new(&p.id, &p.name, kind_of(p.os, &p.screens), false, WALLS[n % WALLS.len()]);
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
            d.kind = if p.os == 0 && p.screens.len() < 2 { d.kind } else { kind_of(p.os, &p.screens) };
            d.screens.clone_from(&p.screens);
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
            d.mounted = mounted.contains(&d.id);
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
            Tab::Files => "Drop here".into(),
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
        self.ctx.data_mut(|data| data.remove::<String>(Id::unique("invitation-link")));
        self.pending_drop = None;
        // The new computer joins the desk: show it.
        if !self.tab.desk() {
            self.set_tab(Tab::Desk);
        }
        match Pairing::start(&self.link.daemon, &self.link.home_args(), self.ctx.clone()) {
            Ok(proc) => {
                let me = self.running().map(|s| s.name.clone()).unwrap_or_default();
                let step = Step::Searching { me: me.clone(), port: None };
                self.pairing = Some(PairUi { proc: Some(proc), step, since: now, opened: now, hint: false, retry: None, me, port: None });
            }
            Err(e) => self.toast_error("Couldn't start pairing. Try restarting oneMK.", Some(e.to_string()), now),
        }
    }

    fn sheet_open(&self) -> bool {
        self.pairing.is_some() || self.pending_drop.is_some() || self.removing.is_some()
    }
}

/// An image embedded by scripts/brand-assets.py: width and height (u32
/// LE), then straight RGBA.
pub fn embedded(bytes: &[u8]) -> ([usize; 2], &[u8]) {
    let n = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("header")) as usize;
    ([n(0), n(4)], &bytes[8..])
}

/// The brand lockup (the Crossing mark and the oneMK wordmark), drawn at
/// 2x for sharpness on high-density screens.
pub fn lockup_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let (size, px) = embedded(include_bytes!("../assets/lockup.rgba"));
    ctx.load_texture("lockup", egui::ColorImage::from_rgba_unmultiplied(size, px), egui::TextureOptions::LINEAR)
}

/// Where the ink sits in the lockup picture (its clear space around it),
/// as fractions of its height: left edge, and the centre of the ink.
const LOCKUP_INK_LEFT: f32 = 65.0 / 52.0;
const LOCKUP_INK_MID: f32 = 21.5 / 52.0;

/// The lockup with the left edge of its ink at `left_center`, the ink
/// centred on it vertically, and the picture `height` points tall.
pub fn draw_lockup(p: &Painter, tex: &egui::TextureHandle, left_center: Pos2, height: f32) {
    let [w, h] = tex.size();
    let min = left_center - vec2(LOCKUP_INK_LEFT * height, LOCKUP_INK_MID * height);
    let rect = Rect::from_min_size(min, vec2(height * w as f32 / h as f32, height));
    p.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// How wide the lockup's ink is at `height`.
pub fn lockup_ink_w(height: f32) -> f32 {
    (213.0 - 65.0) / 52.0 * height
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
        self.place_of(id).map(|(side, _)| side)
    }

    /// Its side and offset, as chosen here or reported.
    fn place_of(&self, id: &str) -> Option<(Side, Option<(i32, i32)>)> {
        if let Some((side, offset, _)) = self.pending_side.get(id) {
            return Some((*side, *offset));
        }
        let s = self.running()?;
        s.peers.iter().find(|p| p.id == id).map(|p| (p.side, s.offset_of(id)))
    }

    fn name_of(&self, id: &str) -> String {
        self.running().and_then(|s| s.peers.iter().find(|p| p.id == id)).map_or_else(|| id.to_owned(), |p| p.name.clone())
    }

    /// Anywhere along `side`: drops an offset, even on the same side.
    fn choose_side(&mut self, id: &str, side: Side, hop: bool) {
        self.choose_place(id, side, None, hop);
    }

    fn choose_place(&mut self, id: &str, side: Side, offset: Option<(i32, i32)>, hop: bool) {
        if self.place_of(id) != Some((side, offset)) {
            let name = self.name_of(id);
            self.pending_side.insert(id.to_owned(), (side, offset, self.now));
            self.busy.push(Job::Layout { id: id.into(), name: name.clone(), side });
            self.link.set_side(id, &name, side, offset);
            if hop {
                self.hop = Some(id.to_owned());
            }
        }
        self.sync_scene();
    }

    /// Starts placing computer `i` by dragging it from `start`, when its
    /// monitors and ours are known.
    fn placing(&self, rect: Rect, i: usize, start: Pos2) -> Option<Box<Placing>> {
        let d = &self.scene.devices[i];
        let fit = self.scene.fit(d.kind, &d.screens)?;
        if d.screens.is_empty() || self.scene.devices[0].screens.is_empty() { return None; }
        let mine = Desktop::new(self.scene.devices[0].screens.clone());
        let edges: Vec<Edge> = Side::ALL.into_iter().flat_map(|s| mine.exposed_edges(s)).collect();
        let peer = Desktop::new(d.screens.clone()).bounds();
        let (px, py) = self.scene.wall_px(rect, &fit.map, start)?;
        // Held where it was picked up: from the middle of its screen.
        let (cx, cy) = self.scene.wall_px(rect, &fit.map, self.scene.screen_centre(rect, i))?;
        let grab = (cx - f64::from(peer.w) / 2.0 - px, cy - f64::from(peer.h) / 2.0 - py);
        let now = scene::drop_place(&mine.screens, &edges, peer, (px + grab.0, py + grab.1), fit.reach());
        Some(Box::new(Placing { fit, mine, edges, peer, grab, now }))
    }

    /// The side a dragged computer is on right now.
    fn side_at(&self, i: usize) -> Side {
        if let Some(Drag::Device { place: Some(pl), .. }) = &self.drag {
            return pl.now.0;
        }
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
        if online && !self.latency.contains(&id) && !self.failed_measure.contains(&id) {
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
                self.toast_error("No computer is connected right now. Start oneMK on the other computer, then drop the files again.", None, now)
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
            // Said once: "samer-Victus, this computer", "MacBook Air, macOS, on your left".
            let label = if i == 0 { format!("{}, this computer", d.name) } else { format!("{}, {}, {place}", d.name, d.subtitle) };
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            widgets::targets::note(&label, r.rect);
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

    /// The desk: framed in `rect`, drawn over `all` (behind the glass).
    fn desk(&mut self, ui: &mut Ui, rect: Rect, all: Rect, now: f64) {
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
                    let place = self.placing(rect, i, start);
                    let d = &mut self.scene.devices[i];
                    // Picked up: it lifts a little and its shadow softens.
                    d.lift.go(motion::space(2.5), now, 0.12, motion::out_cubic);
                    d.pop.go(if motion::reduced() { 1.0 } else { 1.02 }, now, 0.12, motion::out_cubic);
                    Some(Drag::Device { i, start, x0: d.pos.x, place })
                }
                _ => Some(Drag::Orbit),
            };
        }
        if resp.dragged() {
            match &mut self.drag {
                Some(Drag::Device { i, start, x0, place }) => {
                    let (i, start, x0) = (*i, *start, *x0);
                    if let Some(pl) = place {
                        // In our monitors' plane, snapping beside the nearest.
                        if let Some((x, y)) = resp.interact_pointer_pos().and_then(|p| self.scene.wall_px(rect, &pl.fit.map, p)) {
                            pl.now = scene::drop_place(&pl.mine.screens, &pl.edges, pl.peer, (x + pl.grab.0, y + pl.grab.1), pl.fit.reach());
                            let d = &mut self.scene.devices[i];
                            d.pos = pl.fit.pos(pl.now.1);
                            d.scale = pl.fit.scale;
                        }
                    } else if let Some(p) = resp.interact_pointer_pos() {
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
            if let Some(Drag::Device { i, place, .. }) = &self.drag
                && *i < self.scene.devices.len()
            {
                let (i, offset) = (*i, place.as_ref().map(|pl| pl.now.1));
                let side = self.side_at(i);
                let d = &mut self.scene.devices[i];
                d.lift.go(0.0, now, motion::space(0.16), motion::out_cubic);
                d.pop.go(1.0, now, 0.16, motion::out_cubic);
                let id = d.id.clone();
                self.drag_side = None;
                if !self.dragged_once {
                    self.dragged_once = true;
                    let _ = std::fs::write(self.drag_hint_file(), b"");
                }
                match offset {
                    Some(o) => self.choose_place(&id, side, Some(o), false),
                    None => self.choose_side(&id, side, false),
                }
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
        // The full name of the computer under the pointer (a narrow screen
        // can't always hold all of it).
        if self.drag.is_none()
            && let Some(d) = under.filter(|i| self.scene.cut_names.borrow().contains(i)).and_then(|i| self.scene.devices.get(i))
        {
            let text = format!("{} · {}", d.name, d.subtitle);
            resp.clone().on_hover_text_at_pointer(text);
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
        self.scene_moving = moving;
        self.scene.fit = Some(rect);
        let painter = ui.painter_at(all);
        self.scene.paint(&painter, all);
        if moving || self.drag.is_some() {
            ui.ctx().request_repaint();
        }
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

    /// The page's title and the quieter words after it.
    fn page_title(&self) -> (String, String) {
        let name = TABS.iter().find(|t| t.0 == self.tab).map_or("", |t| t.1).to_owned();
        let sub = match (self.tab, self.status.as_ref()) {
            (_, None) => String::new(),
            (_, Some(None)) => "oneMK is off".to_owned(),
            (Tab::Desk | Tab::Layout, Some(Some(s))) => match s.peers.len() {
                0 => "nothing paired yet".to_owned(),
                n => format!("{} of {n} connected", s.peers.iter().filter(|p| p.online).count()),
            },
            (Tab::Wallpaper, Some(Some(s))) => match s.wallpaper.as_ref().and_then(|w| Some((w, w.plan.as_ref()?))) {
                Some((_, plan)) if plan.paused => "paused".to_owned(),
                // Counted from what each computer said, as the page says it.
                Some((wallpaper, plan)) => plan.playing_summary(&wallpaper.computers).to_lowercase(),
                None => "one video across all your screens".to_owned(),
            },
            _ => String::new(),
        };
        (name, sub)
    }

    /// Changes whenever what is drawn behind the glass may have changed.
    fn backdrop_sig(&self, full: Rect) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.tab as u8, full.width() as i32, full.height() as i32, self.status_seen, self.scene.hover, self.scene.selected, self.scene.drop_targets, self.scene.empty_slot, self.scene.slot_hover).hash(&mut h);
        // The desk moves behind the glass only on the pages that show it.
        let moving = self.tab.desk() && (self.scene_moving || self.drag.is_some());
        (self.sheet_open(), moving).hash(&mut h);
        if moving {
            // Moving: every frame is new.
            self.now.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// The top bar's button: what it says and does.
    fn action(&self) -> (String, Look, bool, Option<Job>) {
        let controlled = self.running().is_some_and(|s| s.controlled_by.is_some());
        let broadcasting = self.running().is_some_and(|s| !s.broadcast.is_empty());
        let narrow = self.ctx.content_rect().width() < 1100.0;
        let (label, look, enabled, job) = match self.status.as_ref() {
            Some(None) if self.busy.contains(&Job::Start) => ("Starting…", Look::Primary, false, None),
            Some(None) => ("Start oneMK", Look::Primary, true, Some(Job::Start)),
            _ if broadcasting && self.busy.contains(&Job::Broadcast(false)) => ("Stopping…", Look::Primary, false, None),
            _ if broadcasting => (if narrow { "Keyboard stop" } else { "Use keyboard to stop" }, Look::Quiet, false, None),
            _ if controlled && self.busy.contains(&Job::TakeBack) => ("Taking back…", Look::Primary, false, None),
            _ if controlled => ("Take back", Look::Primary, true, Some(Job::TakeBack)),
            _ if self.running().is_some_and(|s| !s.peers.is_empty()) => (if narrow { "Pair another" } else { "Pair another computer" }, Look::Quiet, true, None),
            _ => ("Pair a computer", Look::Primary, true, None),
        };
        // On the desk, Start and Take back are offered where the eye is (in
        // the card, in the banner) in Volt: here they step back to quiet,
        // so one Volt action shows at a time.
        let first_pair = job.is_none() && self.running().is_some_and(|s| s.peers.is_empty());
        let look = if look == Look::Primary && self.tab == Tab::Desk && (matches!(job, Some(Job::Start | Job::TakeBack)) || first_pair) { Look::Quiet } else { look };
        (label.to_owned(), look, enabled, job)
    }

    fn set_tab(&mut self, tab: Tab) {
        if tab != self.tab {
            self.tab = tab;
            if tab == Tab::Files { self.clipboard_history = None; self.link.history(); }
            self.tab_at = self.now;
            if tab == Tab::Health {
                self.doctor_on_open();
            }
            self.sync_scene();
        }
    }

    // ---------- the panel ----------

    fn headline_text(&self) -> (String, String) {
        let Some(s) = self.status.as_ref() else { return ("Looking for oneMK…".into(), String::new()) };
        let Some(s) = s else {
            return ("oneMK isn't running".into(), "Start it to share this computer's mouse, keyboard, clipboard and files.".into());
        };
        if !s.broadcast.is_empty() {
            let title = match &s.broadcast[..] {
                [one] => format!("Broadcasting to {one}"),
                all => format!("Broadcasting to {} computers", all.len()),
            };
            // The title names one; more than three are on the desk above.
            let names = match &s.broadcast[..] {
                [_] => "it".to_owned(),
                [rest @ .., last] if rest.len() < 3 => format!("{} and {last}", rest.join(", ")),
                _ => "all of them".to_owned(),
            };
            return (title, format!("Everything you type, click and scroll here goes to {names}. {}", self.broadcast_stop_hint()));
        }
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
                "Paired computers appear here when oneMK is running on them and they're on the same network.".into(),
            ),
            [p] if self.apart.contains(&p.id) => (
                format!("{} is apart from this computer", p.name),
                format!("No screen of theirs touches one here, so the pointer doesn't cross. Drag it next to a monitor to cross there.{away}"),
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
            _ => [0.0, w * 0.34, w * 0.66, w],
        };
        let cell = |a: usize, b: usize| Rect::from_min_max(pos2(rect.left() + x[a] + PAD, rect.top() + PAD), pos2(rect.left() + x[b] - PAD, rect.bottom() - PAD));
        let (t, s) = self.headline_text();
        match self.tab {
            // Off, there is nothing to inspect: the message takes the room.
            Tab::Desk if self.off() => vec![(Cell::Head(t, s), cell(0, 2)), (Cell::Controls, cell(2, 3))],
            // A small window has room for one card: the chosen computer's
            // (this computer's switches show when it is the one chosen).
            Tab::Desk if w < 760.0 => match self.selected_id() {
                Some(id) if id != "me" => vec![(Cell::Inspector(Some(id)), cell(0, 3))],
                _ => vec![(Cell::Inspector(None), cell(0, 1)), (Cell::Controls, cell(1, 3))],
            },
            Tab::Desk => vec![(Cell::Inspector(self.selected_id()), cell(0, 2)), (Cell::Controls, cell(2, 3))],
            Tab::Layout => {
                let sel = self.selected_id().filter(|id| id != "me");
                vec![(Cell::Arrange(sel.clone()), cell(0, 1)), (Cell::LineUp(sel), cell(1, 2)), (Cell::Profiles, cell(2, 3))]
            }
            Tab::Health | Tab::Advanced | Tab::Keys | Tab::Wallpaper | Tab::Files => vec![],
        }
    }

    fn panel(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        if self.tab == Tab::Files {
            self.files_panel(ui, rect);
            return;
        }
        if self.tab == Tab::Layout && rect.width() < 760.0 {
            self.layout_stacked(ui, rect, now);
            return;
        }
        let cells = self.cells(rect);
        // Each cell on a glass card of its own, the window's gap between.
        for (cell, c) in &cells {
            let mut card = Rect::from_min_max(c.min - vec2(PAD - M / 2.0, PAD), c.max + vec2(PAD - M / 2.0, PAD));
            // A short cell's card ends a padding under what it holds.
            if let Some(h) = cell_key(cell).and_then(|k| self.cell_h.get(k)) {
                card.max.y = card.max.y.min(c.top() + h + PAD + 2.0);
            }
            if card.left() < rect.left() + M {
                card.min.x = rect.left();
            }
            if card.right() > rect.right() - M {
                card.max.x = rect.right();
            }
            card.min.y = card.min.y.max(rect.top());
            card.max.y = card.max.y.min(rect.bottom());
            // Persistent content: a flat quiet group (no gradient, no
            // shadow); only what floats is raised.
            widgets::group(ui, card, R_PANEL);
        }
        let opening = self.scene.opening(now);
        for k in 0..self.slots.len() {
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
        for k in 0..self.slots.len() {
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
            let layer = child.layer_id();
            let start = ui.ctx().graphics(|g| g.get(layer).map_or(0, |l| l.all_entries().len()));
            self.draw_cell(&mut child, &c, r2, now);
            // What it really took (its shapes' extent), for its card next frame.
            if let Some(key) = cell_key(&c) {
                let bottom = ui.ctx().graphics(|g| g.get(layer).map_or(r2.top(), |l| l.all_entries().skip(start).map(|e| e.shape.visual_bounding_rect()).filter(|b| b.is_finite()).fold(r2.top(), |m, b| m.max(b.bottom()))));
                let h = (bottom - r2.top()).clamp(0.0, r2.height());
                if (self.cell_h.get(key).copied().unwrap_or(f32::MAX) - h).abs() > 0.5 && e >= 1.0 {
                    self.cell_h.insert(key, h);
                    ui.ctx().request_repaint();
                }
            }
        }
    }

    /// Layout in a narrow window: its three cards behind one tab row
    /// (Position, Exact place, Profiles), each shown full width and full
    /// height, so every one is a click away and nothing is shortened or cut.
    fn layout_stacked(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let sel = self.selected_id().filter(|id| id != "me");
        let key = Id::unique("layout-tab");
        #[allow(unused_mut)]
        let mut chosen = ui.ctx().data(|d| d.get_temp::<usize>(key)).unwrap_or(0);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(k) = std::env::var("ONE_MK_APP_LAYOUT_TAB").ok().and_then(|v| v.parse::<usize>().ok()) {
            chosen = k.min(2);
        }
        // An open profile's options belong to Profiles: show that tab.
        if self.profile_scroll {
            chosen = 2;
        }
        let height_key = Id::unique(("layout-position-height", &sel, rect.width().to_bits()));
        let panel_height = if chosen == 0 { ui.ctx().data(|data| data.get_temp::<f32>(height_key)).unwrap_or(232.0).min(rect.height()) } else { rect.height() };
        let rect = Rect::from_min_size(rect.min, vec2(rect.width(), panel_height));
        widgets::group(ui, rect, R_PANEL);
        let inner = rect.shrink(PAD);
        let labels = ["Position", "Exact place", "Profiles"];
        let track = Rect::from_min_size(inner.min, vec2(inner.width().min(420.0), 30.0));
        let pills = Pills {
            id: Id::unique("layout-tabs"),
            track,
            cells: Pills::even(track, 3, 2.0),
            labels: &labels,
            names: labels.iter().map(|l| format!("Layout: {l}")).collect(),
            chosen: Some(chosen),
            enabled: vec![true; 3],
            role: Role::Tab,
            font: T_BODY,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            chosen = k;
        }
        ui.ctx().data_mut(|d| d.insert_temp(key, chosen));
        let body = Rect::from_min_max(pos2(inner.left(), track.bottom() + 16.0), inner.max);
        let cell = match chosen {
            0 => Cell::Arrange(sel),
            1 => Cell::LineUp(sel),
            _ => Cell::Profiles,
        };
        let mut child = ui.new_child(UiBuilder::new().max_rect(body).id_salt(("layout-tab-body", chosen)));
        child.set_clip_rect(body.expand(4.0).intersect(ui.clip_rect()));
        let layer = child.layer_id();
        let start = ui.ctx().graphics(|graphics| graphics.get(layer).map_or(0, |list| list.all_entries().len()));
        self.draw_cell(&mut child, &cell, body, now);
        if chosen == 0 {
            let bottom = ui.ctx().graphics(|graphics| graphics.get(layer).map_or(body.top(), |list| list.all_entries().skip(start).map(|entry| entry.shape.visual_bounding_rect()).filter(|bounds| bounds.is_finite()).fold(body.top(), |bottom, bounds| bottom.max(bounds.bottom()))));
            let measured = (bottom + PAD - rect.top()).max(120.0);
            if (measured - panel_height).abs() > 0.5 {
                ui.ctx().data_mut(|data| data.insert_temp(height_key, measured));
                ui.ctx().request_repaint();
            }
        }
    }

    fn draw_cell(&mut self, ui: &mut Ui, cell: &Cell, r: Rect, now: f64) {
        match cell {
            Cell::Head(title, sub) => {
                let th = para(ui, r.left_top(), r.width(), title, semibold(T_TITLE), INK);
                let sh = para(ui, r.left_top() + vec2(0.0, th + 6.0), r.width(), sub, regular(T_BODY), INK2);
                let at = Rect::from_min_size(r.min, vec2(r.width(), th + sh + 6.0));
                a11y_text(ui, Id::unique("headline"), at, Role::Status, &format!("{title}. {sub}"), Some(Live::Polite));
                // Off: the way to turn it on, in the card that says so.
                if self.off() && self.status.is_some() {
                    let busy = self.busy.contains(&Job::Start);
                    let label = if busy { "Starting…" } else { "Start oneMK" };
                    let bw = button_width(ui, "Start oneMK") + 8.0;
                    let b = Rect::from_min_size(pos2(r.left(), at.bottom() + 16.0), vec2(bw, 36.0));
                    if button(ui, b, Id::unique("start-in-card"), label, Look::Primary, !busy && !self.sheet_open(), Some("Start oneMK on this computer")).clicked() {
                        self.busy.push(Job::Start);
                        self.link.start();
                    }
                }
            }
            Cell::Inspector(sel) => self.peer_card(ui, r, sel.clone(), now),
            Cell::Controls => self.controls(ui, r),
            Cell::Arrange(sel) => self.arrange_cell(ui, r, sel.clone()),
            Cell::LineUp(sel) => self.line_up_cell(ui, r, sel.clone()),
            Cell::Profiles => self.profiles_cell(ui, r),
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
            let rh = swap_row_h(c.width());
            let stacked = rh > SWAP_ROW;
            let row = Rect::from_min_size(pos2(c.left(), c.top() + rh * k as f32), vec2(c.width(), rh));
            let p = ui.painter().clone();
            let seg_w = if stacked { row.width() } else { 264.0 };
            let seg = if stacked {
                Rect::from_min_size(pos2(row.left(), row.top() + 28.0), vec2(seg_w, 28.0))
            } else {
                Rect::from_min_size(pos2(row.right() - seg_w, row.center().y - 13.0), vec2(seg_w, 26.0))
            };
            let name_y = if stacked { row.top() + 12.0 } else { row.center().y };
            let os_label = os_name(peer.os);
            let ow = text_width(ui, os_label, regular(T_META));
            let name_room = if stacked { row.width() } else { seg.left() - row.left() - 12.0 };
            let name = fit(&p, &peer.name, &regular(T_BODY), name_room - ow - 8.0);
            let nw = text_width(ui, &name, regular(T_BODY));
            p.text(pos2(row.left(), name_y), Align2::LEFT_CENTER, &name, regular(T_BODY), INK);
            p.text(pos2(row.left() + nw + 8.0, name_y), Align2::LEFT_CENTER, os_label, regular(T_META), INK2);
            if name != peer.name {
                ui.interact(Rect::from_min_size(pos2(row.left(), name_y - 10.0), vec2(nw, 20.0)), Id::unique(("swap-name", &peer.id)), Sense::hover()).on_hover_text(&peer.name);
            }
            let current = self.pending_swap.get(&peer.id).unwrap_or(&peer.swap);
            // The full words always: the row makes room for them.
            let labels: Vec<&str> = SWAPS.iter().map(|(_, l)| *l).collect();
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
        if let Some((id, name, at)) = &self.removing {
            return Some(SheetView {
                title: format!("Remove {name}?"),
                sub: "It stops sharing with this computer. To use it again, pair the two computers again.".into(),
                code: None,
                check: None,
                buttons: vec![("Cancel".into(), Look::Quiet, Act::Close), ("Remove".into(), Look::Primary, Act::Forget(id.clone(), name.clone()))],
                focus: 0,
                appear: *at,
                key: "remove".into(),
            });
        }
        let pair = self.pairing.as_ref()?;
        let quiet = |l: &str, a: Act| (l.to_owned(), Look::Quiet, a);
        let (title, sub, buttons, focus): (String, String, Vec<(String, Look, Act)>, usize) = match &pair.step {
            Step::Searching { .. } | Step::Hint => {
                let me = if pair.me.is_empty() { String::new() } else { format!(" This one shows up there as “{}”.", pair.me) };
                let mut sub = format!("On the other computer, open oneMK and choose Pair a computer.{me}");
                if let Some(why) = &pair.retry {
                    sub += &format!("\n\nThat attempt didn't finish: {why}. Still looking.");
                } else if pair.hint {
                    sub += "\n\nAllow oneMK through the firewall on both computers and keep them on the same network.";
                    if let Some(port) = pair.port {
                        sub += &format!(" This computer's pairing port is {port}.");
                    }
                }
                ("Looking for another computer".into(), sub, vec![quiet("Cancel", Act::Close)], 0)
            }
            Step::Found => ("Found a computer".into(), "Connecting to it…".into(), vec![quiet("Cancel", Act::Close)], 0),
            // A computer named like this one: say so, so the sheet never
            // seems to offer this computer to itself.
            Step::Code { name, .. } if self.running().is_some_and(|s| s.name == *name) => (
                format!("Found another computer named “{name}”"),
                "It has the same name as this one. Check that its screen shows the same code, then confirm.".into(),
                vec![quiet("They don't match", Act::Decline), ("Codes match, pair".into(), Look::Primary, Act::Confirm)],
                0,
            ),
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
                    (None, _) => "Start oneMK to connect.".into(),
                    (_, Some(e)) => format!("Move the pointer off the {e} edge to cross over."),
                    _ => "It joins the desk in a moment.".into(),
                };
                (format!("Paired with {name}"), sub, vec![], 0)
            }
            Step::Retry(_) | Step::Invitation(_) => return None,
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
                    // Closed: the keyboard goes back to where the sheet was
                    // opened from (the sidebar's action).
                    if self.sheet_was.is_some() && widgets::keyboard_focus(ui.ctx()) {
                        ui.memory_mut(|m| m.request_focus(Id::unique("action")));
                    }
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
        let widest = view.buttons.iter().map(|(label, _, _)| button_width(ui, label) + 36.0).fold(440.0f32, f32::max);
        let width = widest.min(desk.width() - 40.0);
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
        let button_labels: Vec<_> = view.buttons.iter().map(|(label, _, _)| ui.painter().layout(label.clone(), regular(T_BODY), INK, inner - 28.0)).collect();
        let button_heights: Vec<f32> = button_labels.iter().map(|label| (label.size().y + 16.0).max(34.0)).collect();
        let btn_h = if view.buttons.is_empty() { 0.0 } else if row { 34.0 } else { button_heights.iter().sum::<f32>() + 8.0 * button_heights.len().saturating_sub(1) as f32 };
        let title_g = ui.painter().layout(view.title.clone(), semibold(T_TITLE), INK, inner - if view.check.is_some() { 26.0 } else { 0.0 });
        let title_h = title_g.size().y.max(22.0);
        let alternatives = leaving.is_none() && self.pairing.as_ref().is_some_and(|pair| matches!(pair.step, Step::Searching { .. } | Step::Hint));
        let height = 18.0 + title_h + 4.0 + sub_g.size().y + code_h + if btn_h > 0.0 { 18.0 + btn_h } else { 0.0 } + 18.0 + if alternatives { 212.0 } else { 0.0 };
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
        p.add(egui::epaint::Shadow { offset: [0, 18], blur: 40, spread: 0, color: Color32::from_black_alpha(115) }.as_shape(rect, CornerRadius::same(R_PANEL)));
        p.rect(rect, CornerRadius::same(R_PANEL), POPOVER, Stroke::new(1.0, Color32::from_white_alpha(16)), StrokeKind::Inside);
        p.hline(rect.x_range().shrink(f32::from(R_PANEL)), rect.top() + 1.0, Stroke::new(1.0, Color32::from_white_alpha(28)));
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
        p.galley(pos2(tx, y), title_g, INK);
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
        if alternatives { self.pairing_controls(&mut child, Rect::from_min_size(pos2(x, y + 12.0), vec2(inner, 200.0)), now); y += 212.0; }
        if view.buttons.is_empty() {
            return;
        }
        y += 18.0;
        let mut clicked = None;
        let mut bx = rect.right() - 18.0 - widths.iter().sum::<f32>() - 8.0 * widths.len().saturating_sub(1) as f32;
        for (k, ((label, look, act), w)) in view.buttons.iter().zip(&widths).enumerate() {
            let r = if row {
                let r = Rect::from_min_size(pos2(bx, y), vec2(*w, 34.0));
                bx += w + 8.0;
                r
            } else {
                Rect::from_min_size(pos2(x, y + button_heights[..k].iter().sum::<f32>() + k as f32 * 8.0), vec2(inner, button_heights[k]))
            };
            let shown = if row { label.as_str() } else { "" };
            let id = Id::unique(("sheet-btn", &view.key, k));
            let resp = button(&mut child, r, id, shown, *look, true, Some(label));
            widgets::a11y(&resp, Role::Button, label, None);
            if !row {
                p.galley(r.center() - button_labels[k].size() / 2.0, button_labels[k].clone(), if *look == Look::Primary { SIGNAL_INK } else { INK });
            }
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
                self.removing = None;
            }
            Act::Forget(id, name) => {
                self.removing = None;
                self.busy.push(Job::Forget(name.clone()));
                self.link.forget(&id, &name);
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
        // On the desk it takes its place in the band (the desk framed
        // below it); on other pages it floats at the top, centred.
        let slot = self.toast_slot.filter(|_| self.tab.desk());
        let w_max = slot.map_or(520.0f32.min(desk.width() - 120.0) + 46.0, |r| r.width());
        let (g, d, h) = desk_cards::toast_lay(ui, t, w_max);
        let rect = match slot {
            Some(r) => r.translate(vec2(0.0, -motion::space(6.0) * (1.0 - a_in))),
            None => {
                let w = g.size().x.max(d.as_ref().map_or(0.0, |d| d.size().x)) + 46.0 + if t.error { desk_cards::toast_action_w(t) } else { 0.0 };
                let y = desk.top() + TOP_H + 10.0 + h / 2.0 - motion::space(8.0) * (1.0 - a_in);
                Rect::from_center_size(pos2(desk.center().x, y), vec2(w, h))
            }
        };
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::unique("toast"));
        let mut child = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(rect));
        child.set_opacity(a);
        let body = if t.error { Rect::from_min_max(rect.min, pos2(rect.right() - desk_cards::toast_action_w(t), rect.bottom())) } else { rect };
        let resp = child.interact(body, Id::unique("toast-body"), Sense::CLICK);
        let p = child.painter().clone();
        p.add(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) }.as_shape(rect, CornerRadius::same(12)));
        p.rect(rect, CornerRadius::same(12), POPOVER, Stroke::new(1.0, Color32::from_white_alpha(16)), StrokeKind::Inside);
        let dot = if t.error { ERROR } else { INK };
        let gh = g.size().y;
        // Errors are one line, centred on the band.
        let gy = if t.error { rect.center().y - g.size().y / 2.0 } else { rect.top() + 10.0 };
        p.circle_filled(pos2(rect.left() + 17.0, gy + 9.0), 3.5, dot);
        if t.error && g.elided {
            resp.clone().on_hover_text(format!("{}{}", t.text, t.detail.as_ref().map_or(String::new(), |d| format!(" {d}"))));
        }
        p.galley(pos2(rect.left() + 30.0, gy), g, INK);
        if let Some(d) = d {
            p.galley(pos2(rect.left() + 30.0, rect.top() + 14.0 + gh), d, INK2);
        }
        // An error says where to look next: Health has the checks and fixes.
        let mut open_health = false;
        let mut try_again = false;
        if t.error {
            let bw = button_width(&child, "Open Health");
            let b = Rect::from_min_size(pos2(rect.right() - 12.0 - bw, rect.center().y - 15.0), vec2(bw, 30.0));
            // What failed, done again: the first thing to try, first in
            // Tab order too.
            if t.retry.is_some() {
                let tw = button_width(&child, "Try again");
                let tb = Rect::from_min_size(pos2(b.left() - 8.0 - tw, b.top()), vec2(tw, 30.0));
                try_again = button(&mut child, tb, Id::unique("toast-retry"), "Try again", Look::Quiet, true, Some("Do it again")).clicked();
            }
            open_health = button(&mut child, b, Id::unique("toast-health"), "Open Health", Look::Quiet, true, Some("Open Health to see what went wrong")).clicked();
        }
        let (role, live) = if t.error { (Role::Alert, Live::Assertive) } else { (Role::Status, Live::Polite) };
        let label = format!("{}{}", t.text, t.detail.as_ref().map_or(String::new(), |d| format!(" {d}")));
        a11y_text(&child, Id::unique("toast-node"), rect, role, &label, Some(live));
        let dismiss = resp.clicked() || (t.error && !self.sheet_open() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if open_health {
            self.set_tab(Tab::Health);
        }
        if try_again && let Some(job) = self.toast.as_mut().and_then(|t| t.retry.take()) {
            self.retry(job);
        }
        let t = self.toast.as_mut().expect("shown above");
        if (dismiss || open_health || try_again) && age < t.life - exit {
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
        if widgets::locked(ui) || self.keys_grab(ui.ctx()) || ui.ctx().data(|data| data.get_temp::<usize>(Id::unique("shortcut-recorder")).is_some() || data.get_temp::<bool>(Id::unique("gesture-recording")).unwrap_or(false)) {
            return;
        }
        let off = self.off();
        // ⌘K (Ctrl+K): the command palette, from anywhere.
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
            if self.palette.open { self.palette.open = false } else { self.open_palette(self.now) }
            return;
        }
        if self.palette.open {
            return;
        }
        // One key per page, as the sidebar shows them: ⌘1 Desk, ⌘2
        // Wallpaper, ⌘3 Clipboard & files, ⌘4 Keys & mouse, ⌘, Settings.
        for (key, tab) in [(Key::Num1, Tab::Desk), (Key::Num2, Tab::Wallpaper), (Key::Num3, Tab::Files), (Key::Num4, Tab::Keys), (Key::Num5, Tab::Health), (Key::Comma, Tab::Advanced)] {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) && (!off || tab.when_off()) {
                self.set_tab(tab);
            }
        }
    }
}

/// The sheet grows from 98% as it arrives.
/// Cells whose card fits what they hold (the others fill their column).
fn cell_key(c: &Cell) -> Option<&'static str> {
    match c {
        Cell::Controls => Some("controls"),
        Cell::Head(..) => Some("head"),
        // Layout's three cards share one height (a row of equals).
        Cell::Inspector(None) => Some("me"),
        Cell::Inspector(Some(_)) => Some("peer"),
        _ => None,
    }
}

fn lerp_scale(t: f32) -> f32 {
    if motion::reduced() { 1.0 } else { 0.98 + 0.02 * t }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        crate::chrome::place_buttons(ui.ctx(), frame);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let ui_started = std::time::Instant::now();
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
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
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
        self.glass.frame();
        crate::chrome::paint_room(ui.painter(), full);
        let f = shell::frame(full);
        let content = f.content;
        let head = Rect::from_min_size(content.min, vec2(content.width(), shell::HEAD_H));
        let with_cards = !matches!(self.tab, Tab::Health | Tab::Advanced | Tab::Keys | Tab::Wallpaper | Tab::Files);
        let cards = Rect::from_min_max(pos2(content.left(), content.bottom() - if with_cards { self.panel_h() } else { 0.0 }), content.max);
        let desk = Rect::from_min_max(pos2(content.left(), head.bottom()), pos2(content.right(), if with_cards { cards.top() - M } else { content.bottom() }));
        // Files let go over a page without the desk: ask where they go.
        if !self.tab.desk() && self.tab != Tab::Wallpaper {
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
        self.refresh_notice(ui, now);
        // Layout learning's question has a strip of its own under the desk.
        let card = self.suggestion_card(ui, desk);
        let scene = card.as_ref().map_or(desk, |c| Rect::from_min_max(desk.min, pos2(desk.right(), c.rect.top() - 6.0)));
        // The computers are framed below a banner over the desk, and below
        // the transfer where a small window puts it at the top.
        let mut frame = scene;
        let mut compact = false;
        if self.tab.desk() && self.tab != Tab::Files {
            // A band at the desk's top holds the banner, the toast and (in a
            // small window) the transfer; the desk is framed below it and
            // above what floats along its bottom, never under either. The
            // band eases open so the desk glides rather than jumps.
            let band = tween(ui.ctx(), Id::unique("desk-band"), self.band_h(ui, scene.width()), motion::space(0.22), motion::out_cubic);
            // The view switch and toolbar row is never drawn over: the
            // monitors start below it (the camera adds its 12-point inset).
            frame.min.y += band + if self.desk_views_shown() { desk_cards::DESK_VIEWS_H } else { 0.0 };
            frame.max.y -= self.desk_foot_h(scene.width());
            // Too little room left for monitors anyone could read: the
            // computers as a row of named chips instead.
            // More computers need more room before their names read.
            let need = COMPACT_BELOW + 22.0 * self.scene.devices.len().saturating_sub(1).min(5) as f32;
            // On Clipboard & files the desk is only where files go: in a
            // narrow window, chips (drop targets with names) always.
            let files_narrow = self.tab == Tab::Files && scene.width() < 780.0;
            compact = (frame.height() < need || files_narrow) && self.running().is_some_and(|s| !s.peers.is_empty()) && matches!(self.tab, Tab::Desk | Tab::Layout | Tab::Files);
        }
        // The sidebar and the toolbar are made before the desk, so Tab meets
        // them first (search, pages, the toolbar left to right, then the
        // computers, then the card); they are painted on a layer above it.
        {
            let over = egui::LayerId::new(egui::Order::Middle, Id::unique("chrome-layer"));
            let mut top = ui.new_child(UiBuilder::new().layer_id(over).max_rect(full).id_salt("chrome"));
            self.sidebar(&mut top, &f, now);
            self.desk_actions(&mut top, head.translate(vec2(0.0, shell::HEAD_H)));
        }
        if self.tab.desk() && self.tab != Tab::Files {
            if compact {
                self.desk_chips(ui, frame);
            } else {
                self.desk(ui, frame, full, now);
            }
        }
        // Behind the glass changed: blur again (else each panel keeps its blur).
        let sig = self.backdrop_sig(full);
        if sig != self.backdrop_was {
            self.backdrop_was = sig;
            self.glass.changed();
        }
        if self.tab.desk() && self.tab != Tab::Files {
            let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
            self.desk_overlay(ui, scene, hovering);
        }
        let body = Rect::from_min_max(pos2(content.left(), head.bottom() + 4.0), content.max);
        match self.tab {
            Tab::Files => {
                let mut child = page(ui, body, "files");
                self.files_panel(&mut child, body);
            }
            Tab::Wallpaper => {
                let mut child = page(ui, body, "wallpaper");
                self.wallpaper_page(&mut child, body);
            }
            Tab::Health => {
                let mut child = page(ui, body, "health");
                self.health(&mut child, body, now);
            }
            Tab::Advanced => {
                let mut child = page(ui, body, "advanced");
                self.advanced(&mut child, body);
            }
            Tab::Keys => {
                let mut child = page(ui, body, "keys");
                self.keys_page(&mut child, body, now);
            }
            _ => {}
        }
        let (title, sub) = self.page_title();
        self.page_head(ui, head, &title, &sub);
        if with_cards {
            self.panel(ui, cards, now);
        }
        if let Some(card) = card {
            self.suggestion_banner(ui, card);
        }
        widgets::set_locked(ui.ctx(), false);
        // The sheet sits over the panel, leaving the desk room to show
        // what is being paired.
        self.sheet(ui, content, now);
        self.toast_view(ui, desk, now);
        self.palette(ui.ctx(), now);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        crate::chrome::paint_fake_buttons(ui.painter());
        self.guide.show(ui.ctx());
        self.update_notice(ui.ctx());
        // Tell the link whether a graph was drawn: it asks the daemon for
        // round trips only then (a minimised window draws nothing).
        let hidden = ui.input(|i| i.viewport().minimized == Some(true));
        self.link.showing.frame(self.graph_wanted() && !hidden);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_BENCH").is_some() {
            // The UI thread's own work this frame (laying out and painting
            // shapes), apart from waiting for the display.
            let ms = ui_started.elapsed().as_secs_f64() * 1000.0;
            ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::Mutex<Vec<f64>>>>(Id::unique("bench-ui")).lock().unwrap().push(ms));
        }
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        debug::tab_walk(ui.ctx(), now);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        debug::shot(ui.ctx(), now);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        widgets::targets::flush();
    }

    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        debug::tab_press(ctx, raw_input);
        debug::point_at(ctx, raw_input);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        crate::chrome::clear_color()
    }
}

/// Development aids for screenshots, in debug builds only:
/// - `ONE_MK_APP_SHOT=file.ppm` saves the window after
///   `ONE_MK_APP_SHOT_AFTER` seconds (default 3) and quits;
/// - `ONE_MK_APP_PAIR=1` starts pairing; `ONE_MK_APP_TAB=files|keyboard`
///   opens a tab; `ONE_MK_APP_SELECT=name` selects a computer (`me`: this one);
/// - `ONE_MK_APP_RTT="ID=4,5,6+2;ID2=..."` fills the latency graphs with a
///   history, one reading a second in ms (`+n`: n packets lost), and stops
///   live samples; the id is the computer's id in the status;
/// - `ONE_MK_APP_DROP=n` drops n files where it's unclear who gets them;
/// - `ONE_MK_APP_DRAG=1` carries the first other computer across this one;
/// - `ONE_MK_APP_FOCUS=1` puts the keyboard on the position control;
/// - `ONE_MK_APP_TOAST=1` shows an error toast; `ONE_MK_APP_AUTOSTART=1`
///   shows start at login as on;
/// - `ONE_MK_APP_CONFIRM=1` says the codes match a second after they
///   show, and keeps the "paired" sheet up.
/// - `ONE_MK_APP_TAB=layout|health|advanced|wallpaper` (and files, keyboard);
///   `ONE_MK_APP_WALLPAPER=poster.png` supplies a fake playing video when
///   combined with `ONE_MK_APP_STATUS` for wallpaper screenshots.
///   `ONE_MK_APP_PROFILE=name` opens a profile's options;
///   `ONE_MK_APP_DOCTOR=file.json` shows that report instead of running
///   `one-mk doctor`; `ONE_MK_APP_ZOOM=2` draws everything twice as big
///   (a high-density screen); `ONE_MK_APP_SIZE=WxH` (main.rs).
/// - `ONE_MK_APP_TAB=keys`: `ONE_MK_APP_KEYS=mouse` shows the mouse;
///   `ONE_MK_APP_KEY_SCOPE=mac|windows|linux|NAME|FINGERPRINT` chooses
///   "For:" (`ONE_MK_APP_KEY_SCOPE_MENU=1` opens its menu;
///   `ONE_MK_APP_INPUT_DEVICES=file.json` lists those devices there);
///   `ONE_MK_APP_KEY=capslock` opens that key (`ONE_MK_APP_KEY_MORE=1`
///   with "hold for another key" open, `ONE_MK_APP_KEY_CAPTURE=1` waiting
///   for a key, `ONE_MK_APP_KEY_PICK=query` with the list open);
///   `ONE_MK_APP_KEY_CHOOSE=esc` makes the opened key that, as a click
///   would; `ONE_MK_APP_BUTTON=4` opens a mouse button; `ONE_MK_APP_KEY_TEST=
///   58d0,58u85` types into the test box (code, down/up, ms).
/// - `ONE_MK_APP_BROADCAST=1` turns broadcast on (or off), as the Desk's
///   switch does.
/// - `ONE_MK_REDUCE_MOTION=1` works in every build (see `motion`).
#[cfg(any(debug_assertions, feature = "design-evidence"))]
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
        // ONE_MK_APP_BENCH=N: N frames as if the desk were dragged (it
        // turns, every glass panel blurs again), then the frame times.
        // ONE_MK_APP_UPDATE=version[:what the update said]: an update waits.
        if let Some(u) = var("ONE_MK_APP_UPDATE") {
            let (v, out) = u.split_once(':').map_or((u.clone(), None), |(v, o)| (v.to_owned(), Some(o.to_owned())));
            app.update_available = Some(v);
            if once(&ctx, "update") {
                app.update_output = out;
            }
        }
        if let Some(status) = var("ONE_MK_APP_UPDATE_STATUS")
            && let Ok(status) = serde_json::from_str(&status)
        {
            app.update_ui.status = Some(status);
        }
        // ONE_MK_APP_MINIMIZE=1: minimised two seconds in (idle measurements).
        if var("ONE_MK_APP_MINIMIZE").is_some() && now > 2.0 && once(&ctx, "minimize") {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        // ONE_MK_APP_PALETTE=query: the command palette, open, with that typed.
        if let Some(q) = var("ONE_MK_APP_PALETTE")
            && now > 0.5
            && once(&ctx, "palette")
        {
            app.open_palette(now);
            app.palette.query = q;
        }
        // ONE_MK_APP_WHY=1: after 5 s, what asked for each frame.
        if var("ONE_MK_APP_WHY").is_some() && now > 5.0 && now < 8.0 {
            let why: Vec<String> = ctx.repaint_causes().iter().map(|c| format!("{c}")).collect();
            eprintln!("frame at {now:.3}: {}", why.join(" | "));
        }
        if var("ONE_MK_APP_BENCH").is_some() && now <= 1.5 {
            ctx.request_repaint_after_secs(0.1);
        }
        if let Some(n) = var("ONE_MK_APP_BENCH").and_then(|v| v.parse::<usize>().ok()) && now > 1.5 {
            let times = ctx.data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::Mutex<Vec<f64>>>>(Id::unique("bench")).clone());
            let mut t = times.lock().unwrap();
            // CPU used by the whole process (all threads), in clock ticks.
            let cpu = || std::fs::read_to_string("/proc/self/stat").ok().and_then(|st| {
                let f: Vec<u64> = st.rsplit(')').next()?.split_whitespace().skip(11).take(2).filter_map(|v| v.parse().ok()).collect();
                Some(f.iter().sum::<u64>())
            });
            if t.is_empty() {
                ctx.data_mut(|d| d.insert_temp(Id::unique("bench-cpu"), cpu().unwrap_or(0)));
            }
            t.push(now);
            if t.len() > n {
                let start = ctx.data(|d| d.get_temp::<u64>(Id::unique("bench-cpu"))).unwrap_or(0);
                let ticks = cpu().unwrap_or(0).saturating_sub(start);
                println!("bench cpu_ms_per_frame={:.2} (all threads)", ticks as f64 * 10.0 / n as f64);
                let ui_times = ctx.data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::Mutex<Vec<f64>>>>(Id::unique("bench-ui")).clone());
                let mut ui_ms = ui_times.lock().unwrap().clone();
                if !ui_ms.is_empty() {
                    let k = ui_ms.len().min(n);
                    ui_ms = ui_ms.split_off(ui_ms.len() - k);
                    ui_ms.sort_by(f64::total_cmp);
                    println!("bench ui_thread_ms_per_frame p50={:.2} p95={:.2} max={:.2} (update + layout + shapes, not GPU or vsync)", ui_ms[k / 2], ui_ms[k * 95 / 100], ui_ms[k - 1]);
                }
                let gaps: Vec<f64> = t.windows(2).map(|w| (w[1] - w[0]) * 1000.0).collect();
                let mut sorted = gaps.clone();
                sorted.sort_by(f64::total_cmp);
                let avg = gaps.iter().sum::<f64>() / gaps.len() as f64;
                println!("bench frames={} avg_ms={avg:.2} p50_ms={:.2} p95_ms={:.2} max_ms={:.2}", gaps.len(), sorted[sorted.len() / 2], sorted[sorted.len() * 95 / 100], sorted[sorted.len() - 1]);
                std::process::exit(0);
            }
            // ONE_MK_APP_BENCH_HOVER=1: frames where nothing behind the
            // glass changes (a hover, a key typed): the kept blur is reused.
            if var("ONE_MK_APP_BENCH_HOVER").is_none() {
                app.scene.yaw_to = (now * 0.7).sin() as f32 * 0.4;
                app.glass.changed();
            }
            ctx.request_repaint();
        }
        let ready = app.running().is_some();
        if app.status.is_some() && var("ONE_MK_APP_PAIR").is_some() && once(&ctx, "pair") {
            app.start_pairing(now);
        }
        // ONE_MK_APP_SHEET_CLOSE=secs: the open sheet is closed then (as
        // Cancel or Esc would), with the keyboard driving.
        if let Some(t) = var("ONE_MK_APP_SHEET_CLOSE").and_then(|v| v.parse::<f64>().ok())
            && now > t
            && app.sheet_open()
            && once(&ctx, "sheet-close")
        {
            ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
            app.sheet_act(Act::Close, now);
        }
        if ready && let Some(state) = var("ONE_MK_APP_PAIR_STATE") && once(&ctx, "pair-state") {
            let step = match state.as_str() {
                "code" => Step::Code { name: "Studio PC".into(), code: "726480".into() },
                "waiting" => Step::Waiting { name: "Studio PC".into() },
                "paired" => Step::Paired { id: "1111-2222".into(), name: "Studio PC".into() },
                "ended" => Step::Ended("The codes did not match. Nothing was saved.".into()),
                _ => Step::Searching { me: "desk".into(), port: Some(45173) },
            };
            app.pairing = Some(PairUi { proc: None, step, since: now, opened: now, hint: state == "hint", retry: None, me: "desk".into(), port: Some(45173) });
        }
        if ready && var("ONE_MK_APP_HISTORY").is_some() {
            app.prefs.history_enabled = true;
            app.clipboard_history = Some(Ok((1..=5).map(|id| crate::daemon::HistoryItem { id, preview: "[hidden]".into(), detail: "text · 96 B · from Studio PC".into(), timestamp: 1793440000 }).collect()));
            if var("ONE_MK_APP_HISTORY").as_deref() == Some("revealed") {
                app.clipboard_reveal = Some((1, "First line — second line with preserved separators and a long Unicode preview…".into()));
            }
        }
        if ready && let Some(script) = var("ONE_MK_APP_RTT") && once(&ctx, "rtt") {
            for part in script.split(';') {
                if let Some((id, series)) = part.split_once('=') {
                    app.rtts.set(id, crate::rtt::scripted(now, series));
                }
            }
            app.rtt_frozen = Some(now);
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
            app.toast_error("Couldn't move Studio PC to the left. The desk went back to where it was.", Some("oneMK didn't answer within 3 seconds".into()), now);
            let peer = app.running().and_then(|s| s.peers.first().map(|p| (p.id.clone(), p.name.clone())));
            if let (Some(t), Some((id, name))) = (app.toast.as_mut(), peer) {
                t.retry = Some(Job::Layout { id, name, side: Side::Left });
            }
        }
        if ready && var("ONE_MK_APP_BROADCAST").is_some() && now > 1.0 && once(&ctx, "broadcast") {
            let on = !app.broadcasting();
            app.ask_broadcast(on);
        }
        if ready && app.tab != Tab::Keys && var("ONE_MK_APP_FOCUS").is_some() && now > 1.0 && once(&ctx, "focus") {
            ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
            if let Some(id) = app.selected_id() {
                let target = Id::unique(("position", &id)).with(1usize);
                ctx.memory_mut(|m| m.request_focus(target));
            }
        }
        if ready && let Some(name) = var("ONE_MK_APP_PROFILE") && once(&ctx, "profile") {
            app.profile_open = Some(name);
            app.profile_scroll = true;
        }
        // The Keys & mouse page.
        if app.tab == Tab::Keys && app.status.is_some() && once(&ctx, "keys") {
            if var("ONE_MK_APP_KEYS").as_deref() == Some("mouse") {
                app.keys.view = keys::View::Mouse;
            }
            if let Some(s) = var("ONE_MK_APP_KEY_SCOPE") {
                app.keys.scope = match s.as_str() {
                    "mac" => crate::tune::Scope::Os(os::MACOS),
                    "windows" => crate::tune::Scope::Os(os::WINDOWS),
                    "linux" => crate::tune::Scope::Os(os::LINUX),
                    // A device's fingerprint.
                    _ if s.contains('/') => crate::tune::Scope::Device(s, Box::default()),
                    _ => crate::tune::Scope::Peer(s),
                };
            }
            if let Some(code) = var("ONE_MK_APP_KEY").and_then(|k| one_mk_proto::keynames::parse_key(&k).ok()) {
                app.select_key(code as u16, None);
                app.keys.more |= var("ONE_MK_APP_KEY_MORE").is_some();
                if var("ONE_MK_APP_KEY_CAPTURE").is_some() {
                    app.keys.capture = Some((keys::Want::To, crate::tune::Capture::default()));
                }
                if let Some(q) = var("ONE_MK_APP_KEY_PICK") {
                    app.keys.picker = Some((keys::Want::To, q));
                }
            }
            if let Some(v) = var("ONE_MK_APP_KEY_CHOOSE").filter(|_| app.keys.selected.is_some()) {
                app.choose(keys::Want::To, v);
            }
            if let Some(n) = var("ONE_MK_APP_BUTTON").and_then(|n| n.parse().ok()) {
                app.keys.button = Some(n);
                if var("ONE_MK_APP_KEY_PICK").is_some() {
                    app.keys.picker = Some((keys::Want::Button, String::new()));
                }
            }
            if var("ONE_MK_APP_FOCUS").is_some() {
                ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
                ctx.memory_mut(|m| m.request_focus(Id::unique(("cap", app.keys.focus_key))));
            }
        }
        if app.tab == Tab::Keys && now > 1.0 && let Some(spec) = var("ONE_MK_APP_KEY_TEST") && once(&ctx, "key-test") {
            app.debug_type(&ctx, &spec);
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
                app.drag = Some(Drag::Device { i, start: Pos2::ZERO, x0: d.pos.x, place: None });
            }
            if let Some(Drag::Device { x0, .. }) = app.drag {
                let t = ((now - 1.5) / 1.5).min(1.0) as f32;
                app.scene.devices[i].pos.x = x0 + t * 150.0;
                app.make_room(i);
            }
        }
    }

    /// `ONE_MK_APP_TABWALK=n` (with `ONE_MK_APP_TARGETS=file`): from one
    /// second in, presses Tab n times (a real key event, as the keyboard
    /// would send), a step every 0.15 s, and prints what has the keyboard
    /// after each: the Tab order, for the record.
    pub fn tab_press(ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(n) = var("ONE_MK_APP_TABWALK").and_then(|v| v.parse::<usize>().ok()) else { return };
        let id = Id::unique("tab-walk");
        let now = raw.time.unwrap_or(0.0);
        let (done, last) = ctx.data(|d| d.get_temp::<(usize, f64)>(id)).unwrap_or((0, 1.0));
        if done > n || now < last + 0.15 {
            return;
        }
        // What the last press reached (focus settles over the frames between).
        if done > 0 {
            let focused = ctx.memory(|m| m.focused()).and_then(|f| ctx.read_response(f));
            let name = ctx.memory(|m| m.focused()).and_then(widgets::targets::name_of).or_else(|| focused.as_ref().and_then(|r| widgets::targets::name_at(r.rect))).unwrap_or_else(|| format!("(nothing named at {:?})", focused.map(|r| r.rect)));
            eprintln!("tab {done}: {name}");
        }
        if done == n {
            ctx.data_mut(|d| d.insert_temp(id, (done + 1, now)));
            return;
        }
        raw.events.push(egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        ctx.data_mut(|d| d.insert_temp(id, (done + 1, now)));
    }

    /// `ONE_MK_APP_POINT_AT=<control name>` (with `ONE_MK_APP_TARGETS`):
    /// from 1.5 s the pointer rests on that control (hover);
    /// `ONE_MK_APP_PRESS=1` also holds the button down on it (pressed).
    pub fn point_at(ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(name) = var("ONE_MK_APP_POINT_AT") else { return };
        if raw.time.unwrap_or(0.0) < 1.5 {
            return;
        }
        // Found once and kept: a frame whose target list is empty (the
        // screenshot's own) must not let the pointer go.
        let key = Id::unique("point-at-rect");
        let Some(r) = widgets::targets::rect_of(&name).or_else(|| ctx.data(|d| d.get_temp::<Rect>(key))) else { return };
        ctx.data_mut(|d| d.insert_temp(key, r));
        let at = r.center();
        raw.events.push(egui::Event::PointerMoved(at));
        let id = Id::unique("point-at-pressed");
        if var("ONE_MK_APP_PRESS").is_some() && !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) && raw.time.unwrap_or(0.0) > 1.8 {
            raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            ctx.data_mut(|d| d.insert_temp(id, true));
        }
    }

    pub fn tab_walk(ctx: &egui::Context, _now: f64) {
        if var("ONE_MK_APP_TABWALK").is_some() {
            ctx.request_repaint_after_secs(0.05);
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
        let s = slots((-29.0, 29.0), &[("a".into(), Side::Left, 15.5), ("b".into(), Side::Right, 29.0), ("c".into(), Side::Left, 29.0), ("d".into(), Side::Top, 29.0)]);
        assert_eq!(s["a"].0.x, -(29.0 + GAP + 15.5));
        assert_eq!(s["c"].0.x, -(29.0 + GAP + 31.0 + GAP + 29.0));
        assert_eq!(s["b"].0.x, 29.0 + GAP + 29.0);
        assert!(s["d"].0.y > 0.0 && s["d"].0.z < 0.0, "above sits raised and behind");
    }

    #[test]
    fn below_stands_beside_this_computer_not_in_front_of_it() {
        let s = slots((-29.0, 29.0), &[("b".into(), Side::Bottom, 29.0)]);
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
