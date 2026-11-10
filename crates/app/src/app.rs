//! The window: a top bar, the desk, a panel of facts and switches under it,
//! and a sheet for pairing and for choosing where dropped files go.
//! Everything shown comes from the running daemon; everything changed goes
//! through it.

mod cells;
mod clipboard_history;
mod keys;
mod links;
mod page_ui;
mod pages;
mod place;
mod preview;
mod shell;
mod desk_cards;
mod desk_panel;
mod settings_page;
mod updates;
mod settings_controls;
mod palette;
mod wallpaper;

use crate::daemon::{self, Event, Job, Link};
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

/// Below this height (plus 22 pt for each other computer) the desk shows
/// its computers as named chips: monitors any smaller can't carry names.
const COMPACT_BELOW: f32 = 128.0;
const GHOST: &str = "\u{0}pairing";

fn truncated_name_target(focused: Option<usize>, hovered: Option<usize>, truncated: &std::collections::HashSet<usize>) -> Option<(usize, bool)> {
    focused.filter(|index| truncated.contains(index)).map(|index| (index, true))
        .or_else(|| hovered.filter(|index| truncated.contains(index)).map(|index| (index, false)))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Desk,
    Arrange,
    Files,
    Wallpaper,
    Keys,
    Health,
    Settings,
}
/// Each tab, and a shorter name for narrow windows. Settings most people
/// never change (Ctrl and Cmd, gaming mode, edge rules) live in Settings.
/// Arrange is a page of its own in the top bar (shell::NAV gives the order).
const TABS: [(Tab, &str, &str); 7] = [
    (Tab::Desk, "Desk", "Desk"),
    (Tab::Arrange, "Arrange", "Arrange"),
    (Tab::Files, "Clipboard & files", "Files"),
    (Tab::Wallpaper, "Wallpaper", "Wallpaper"),
    (Tab::Keys, "Keys & mouse", "Keys"),
    (Tab::Health, "Health", "Health"),
    (Tab::Settings, "Settings", "Settings"),
];

impl Tab {
    /// Tabs that take files let go over them themselves: the Desk (on a
    /// computer) and Clipboard & files (its own drop area).
    fn desk(self) -> bool {
        matches!(self, Tab::Desk | Tab::Files)
    }
    /// Tabs that mean something while oneMK is off.
    fn when_off(self) -> bool {
        matches!(self, Tab::Desk | Tab::Files | Tab::Wallpaper | Tab::Keys | Tab::Health | Tab::Settings)
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
    /// Delete this saved layout (after the sheet asked).
    DeleteProfile(String),
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
    /// One more thing it offers to do about it.
    act: Option<ToastAct>,
    /// It offers "Open Health" (the checks and their fixes).
    health: bool,
}

/// What an error toast offers besides trying again.
#[derive(Clone, Debug, PartialEq)]
enum ToastAct {
    /// Opens oneMK's log: the service's own account of what went wrong.
    ShowLog,
    /// Moves the computer with this id beside the nearest edge of ours.
    Snap(String),
}

impl ToastAct {
    fn label(&self) -> &'static str {
        match self {
            ToastAct::ShowLog => "Show log",
            ToastAct::Snap(_) => "Snap to an edge",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            ToastAct::ShowLog => "Open oneMK's log",
            ToastAct::Snap(_) => "Move it beside the nearest edge of your screens, so the pointer can cross",
        }
    }
}

pub struct App {
    glass: Glass,
    /// Light or dark, following Settings → Appearance (and the system).
    themer: crate::style::Themer,
    /// The Desk's panel and toolbar (desk_panel.rs).
    desk_ui: desk_panel::DeskUi,
    /// Where the Desk fits its computers this frame (handoff 5.1's scale).
    desk_stage: Option<Rect>,
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
    /// A saved layout the sheet is asking whether to delete, and since when.
    deleting_profile: Option<(String, f64)>,
    /// Settings: the category showing, and the row the palette took us to.
    settings_cat: settings_page::Cat,
    settings_jump: Option<settings_page::Jump>,
    /// Health: the list of passed checks is open.
    health_passed_open: bool,
    /// The sheet as last drawn, and when it started to leave.
    sheet_was: Option<(SheetView, f64)>,
    /// Where the sheet is this frame (the toast keeps clear of it).
    sheet_rect: Option<Rect>,
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
    /// A computer being placed on Arrange's plan, by the pointer or the
    /// arrow keys (place.rs).
    plan: Option<place::PlanDrag>,
    /// Arrange's "Place freely": let go anywhere, even apart.
    plan_free: bool,
    /// The banner over the desk as drawn this frame (see `refresh_notice`).
    notice: desk_cards::NoticeHold,
    /// Whether the window had the focus last frame (see `pointer_away`).
    was_focused: bool,
    /// While another computer is controlled: whether the pointer was seen
    /// here all the same (oneMK couldn't hold it, or it came back unannounced).
    pointer_here: bool,
    /// When control of another computer began (input time), so a move
    /// queued just before the pointer left isn't taken for it being here.
    controlling_since: Option<f64>,
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
    /// Settings → Appearance as chosen here, kept until `settings.toml`
    /// says the same (or for the whole session if it can't be saved).
    appearance_choice: crate::style::AppearanceChoice,
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

fn separate_crowded_slots(places: &mut HashMap<String, (scene::V3, f32)>, widths: &[(String, f32)], mine: f32, mounted: &HashSet<String>) {
    // Exact placements describe touching screen edges. Moving those for
    // presentation makes a dropped computer spring away from its neighbour.
    let mut ordered: Vec<_> = widths.iter().filter(|(id, _)| !mounted.contains(id)).cloned().collect();
    ordered.sort_by(|first, second| places[&first.0].0.x.total_cmp(&places[&second.0].0.x).then(first.0.cmp(&second.0)));
    for left in [true, false] {
        let mut edge = mine;
        let mut group: Vec<_> = ordered.iter().filter(|(id, _)| (places[id].0.x < 0.0) == left).cloned().collect();
        if left { group.reverse(); }
        for (id, half) in group {
            let (position, scale) = places.get_mut(&id).expect("paired slot");
            let half = half * *scale;
            let distance = position.x.abs().max(edge + 32.0 + half);
            position.x = if left { -distance } else { distance };
            edge = distance + half;
        }
    }
}

// ---------- the panel's painted rows ----------

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
            Ok("keyboard") => tab = Tab::Settings,
            Ok("arrange" | "layout") => tab = Tab::Arrange,
            Ok("health") => tab = Tab::Health,
            Ok("settings" | "advanced") => tab = Tab::Settings,
            Ok("keys") => tab = Tab::Keys,
            _ => {}
        }
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(z) = std::env::var("ONE_MK_APP_ZOOM").ok().and_then(|z| z.parse::<f32>().ok()) {
            cc.egui_ctx.set_zoom_factor(z);
        }
        let glass = Glass::default();
        widgets::set_glass(&cc.egui_ctx, glass.clone());
        Ok(Self {
            glass,
            themer: crate::style::Themer::default(),
            desk_ui: desk_panel::DeskUi::default(),
            desk_stage: None,
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
            deleting_profile: None,
            settings_cat: settings_page::Cat::General,
            settings_jump: None,
            health_passed_open: false,
            sheet_was: None,
            sheet_rect: None,
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
            plan: None,
            plan_free: false,
            notice: desk_cards::NoticeHold::default(),
            was_focused: true,
            pointer_here: false,
            controlling_since: None,
            pending_swap: HashMap::new(),
            hop: None,
            just_paired: None,
            active: "me".into(),
            busy: vec![],
            selected: None,
            drop_pos_fresh: false,
            arrowed: false,
            arrow_down: false,
            prefs: Prefs::default(),
            doctor: None,
            copied: None,
            pending_lock: None,
            appearance_choice: Default::default(),
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
        self.toast = Some(Toast { text: text.into(), detail: None, error: false, at: now, life: 3.5, retry: None, act: None, health: false });
    }

    /// An error says what went wrong and what to do; the raw reason goes
    /// underneath. It stays until dismissed, or for ten seconds.
    fn toast_error(&mut self, text: impl Into<String>, detail: Option<String>, now: f64) {
        let detail = detail.map(|d| crate::view::window_issue(d.trim())).filter(|d| !d.is_empty());
        self.toast = Some(Toast { text: text.into(), detail, error: true, at: now, life: 10.0, retry: None, act: None, health: true });
    }

    /// The service didn't start: why, in plain words, with "Try again" and
    /// its log one click away.
    fn start_failed(&mut self, why: Option<String>, now: f64) {
        self.toast_error("oneMK couldn't start.", why, now);
        if let Some(t) = self.toast.as_mut() {
            t.retry = Some(Job::Start);
            t.act = Some(ToastAct::ShowLog);
        }
    }

    /// A computer placed apart from ours on purpose: say what that means,
    /// and offer to put it where the pointer can cross.
    fn placed_apart(&mut self, id: &str, now: f64) {
        let name = self.name_of(id);
        self.toast_error(format!("{name} is placed apart from your screens."), Some("The pointer can't cross to it until their edges touch.".into()), now);
        if let Some(t) = self.toast.as_mut() {
            t.health = false;
            t.act = Some(ToastAct::Snap(id.to_owned()));
        }
    }

    /// Whether `job` can simply be run again from an error toast.
    fn retryable(job: &Job) -> bool {
        matches!(job, Job::Layout { .. } | Job::Swap { .. } | Job::TakeBack | Job::Start | Job::Autostart(_) | Job::Set { .. } | Job::Appearance(_) | Job::Open)
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
            Job::Appearance(_) => self.set_appearance(self.appearance()),
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
                // At the moment it was taken, not the frame that got to it.
                Event::Links(links, taken) if self.rtt_frozen.is_none() => self.rtts.record(now - taken.elapsed().as_secs_f64(), &links),
                Event::Links(..) => {}
                Event::Autostart(on) => self.autostart = Some(on),
                Event::Setup(report) => match report.problems().first() {
                    // Not running matters most, and has its own way out.
                    _ if report.service.failed().is_some() => self.start_failed(report.service.failed().map(str::to_owned), now),
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
                    self.appearance_choice.read(p.appearance);
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
            Job::Start => self.start_failed(e, now),
            Job::ShowLog => self.toast_error("Couldn't open the log.", e, now),
            Job::Forget(name) => self.toast_error(format!("Couldn't remove {name}. Try again."), e, now),
            Job::Stop => self.toast_error("Couldn't stop oneMK. If it was started before an update, quit it once from Activity Monitor or Task Manager; after that Stop works.", e, now),
            Job::Open => self.toast_error("Couldn't open the received files folder.", e, now),
            Job::Profile { verb, name } => {
                // A name that couldn't be saved stays in its field to be fixed.
                if verb == "save" {
                    let draft = self.drafts.entry("profile-new".to_owned()).or_default();
                    if draft.trim().is_empty() {
                        draft.clone_from(&name);
                    }
                }
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
            // Kept on screen for this session all the same.
            Job::Appearance(_) => self.toast_error("Couldn't save the appearance. This window keeps it until it closes.", e, now),
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
        let monitor_count = me.screens.len().max(1) + peers.iter().map(|peer| peer.screens.len().max(1)).sum::<usize>();
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
        let widths: Vec<_> = peers.iter().map(|peer| (peer.id.clone(), scene::half_width(kind_of(peer.os, &peer.screens), &peer.screens))).collect();
        if monitor_count >= 5 { separate_crowded_slots(&mut places, &widths, me_half, &mounted); }
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
            // It forgot this computer: waiting for it would be in vain.
            let removed = self.status.as_ref().and_then(Option::as_ref).is_some_and(|s| s.removed_us(&p.id));
            return if removed { "Removed you" } else { "Offline" }.into();
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
        self.pairing.is_some() || self.pending_drop.is_some() || self.removing.is_some() || self.deleting_profile.is_some()
    }
}

/// Whether the pointer has left the window without the window being told:
/// it is on another computer (`controlling`: oneMK holds it there, and the
/// window hears no leave) unless it has been seen moving or clicking here
/// since (`here`: then it is here, and its clicks must count), or the window
/// just lost the focus to something in front of it. Moving over the window
/// again brings hover back.
fn pointer_away(controlling: bool, here: bool, was_focused: bool, focused: bool) -> bool {
    (controlling && !here) || (was_focused && !focused)
}

/// A raw image embedded in the app (the format scripts/make-icons.py
/// writes): width and height (u32 LE), then straight RGBA.
pub fn embedded(bytes: &[u8]) -> ([usize; 2], &[u8]) {
    let n = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("header")) as usize;
    ([n(0), n(4)], &bytes[8..])
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

    /// Whether the computer with this id is being dragged on the desk.
    fn dragging(&self, id: &str) -> bool {
        matches!(&self.drag, Some(Drag::Device { i, .. }) if self.scene.devices.get(*i).is_some_and(|d| d.id == id))
    }

    /// A computer placed apart, moved beside the nearest edge of ours (or
    /// anywhere along its side, when no edge has room); it hops there.
    fn snap(&mut self, id: &str) {
        let Some((side, offset)) = self.place_of(id) else { return };
        let Some(i) = self.scene.index(id) else { return };
        let (mine, theirs) = (&self.scene.devices[0].screens, &self.scene.devices[i].screens);
        let known = !mine.is_empty() && !theirs.is_empty();
        let place = offset.filter(|_| known).and_then(|o| scene::snapped(&Desktop::new(mine.clone()), &Desktop::new(theirs.clone()), (side, o)));
        match place {
            Some((side, o)) => self.choose_place(id, side, Some(o), true),
            None => self.choose_side(id, side, true),
        }
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
                let i = *i;
                let side = self.side_at(i);
                // Where the screens touch, or beside the nearest edge of
                // ours; never apart by accident.
                let theirs = &self.scene.devices[i].screens;
                let landing = match place.as_ref().filter(|_| !theirs.is_empty()) {
                    Some(pl) => scene::landing(&pl.mine, &Desktop::new(theirs.clone()), (side, pl.now.1), false),
                    None => scene::Landing::Along(side),
                };
                let d = &mut self.scene.devices[i];
                d.lift.go(0.0, now, motion::space(0.16), motion::out_cubic);
                d.pop.go(1.0, now, 0.16, motion::out_cubic);
                let id = d.id.clone();
                self.drag_side = None;
                // Snapped: it glides from where it was let go to its place.
                match landing {
                    scene::Landing::Beside(side, o) => self.choose_place(&id, side, Some(o), false),
                    scene::Landing::Apart(side, o) => {
                        self.choose_place(&id, side, Some(o), false);
                        self.placed_apart(&id, now);
                    }
                    scene::Landing::Along(side) => self.choose_side(&id, side, false),
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
            && let Some((index, focused)) = truncated_name_target(self.scene.focused, under, &self.scene.cut_names.borrow())
            && let Some(d) = self.scene.devices.get(index)
        {
            let text = format!("{} · {}", d.name, d.subtitle);
            if focused { resp.clone().show_tooltip_text(text); } else { resp.clone().on_hover_text_at_pointer(text); }
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
        // The sheet is centred over the scrim (handoff 5.8): the desk stays
        // where it is behind it, rather than making room below.
        self.scene.raise.go(0.0, now, motion::space(0.38), motion::out_expo);
        self.scene.sheet_top = None;
        self.scene.drop_preview.go(f32::from(self.tab == Tab::Files), now, motion::fade(0.28), motion::out_cubic);
        let several = self.scene.devices.iter().filter(|d| !d.leaving && d.id != GHOST).count() > 1;
        let shown = several && self.tab == Tab::Desk;
        self.scene.selected = match self.selected_id().as_deref().filter(|_| shown) {
            Some("me") => Some(0),
            Some(id) => self.scene.index(id),
            None => None,
        };
        self.scene.empty_slot = false;
        let moving = self.scene.step(now, dragging);
        self.scene_moving = moving;
        // On the Desk the computers fit the handoff's box; elsewhere the area.
        self.scene.fit = Some(self.desk_stage.filter(|_| self.tab == Tab::Desk).unwrap_or(rect));
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
        let head = fit(ui.painter(), &format!("Where is {name}?"), &regular(T_META), width - 20.0);
        widgets::menu_heading(ui, width, &head);
        let current = self.side_of(&id);
        for (side, label) in [(Side::Left, "On the left"), (Side::Right, "On the right"), (Side::Top, "Above"), (Side::Bottom, "Below")] {
            let full = format!("{name}: {label}");
            if widgets::menu_item(ui, width, widgets::MenuItem::choice(label, &full, current == Some(side))).clicked() {
                self.choose_side(&id, side, true);
                ui.close();
            }
        }
    }

    // ---------- the top bar ----------

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
        // Shorter forms for a narrow bar: shell::action_label.
        let (label, look, enabled, job) = match self.status.as_ref() {
            Some(None) if self.busy.contains(&Job::Start) => ("Starting…", Look::Primary, false, None),
            Some(None) => ("Start oneMK", Look::Primary, true, Some(Job::Start)),
            _ if broadcasting && self.busy.contains(&Job::Broadcast(false)) => ("Stopping…", Look::Primary, false, None),
            _ if broadcasting => ("Use keyboard to stop", Look::Quiet, false, None),
            _ if controlled && self.busy.contains(&Job::TakeBack) => ("Taking back…", Look::Primary, false, None),
            _ if controlled => ("Take back", Look::Primary, true, Some(Job::TakeBack)),
            _ if self.running().is_some_and(|s| !s.peers.is_empty()) => ("Pair another computer", Look::Primary, true, None),
            _ => ("Pair a computer", Look::Primary, true, None),
        };
        // On the desk, Start, Take back and the first pairing are offered
        // where the eye is (in the card, in the banner) in cobalt: here they
        // step back to quiet, so one cobalt action shows at a time.
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
            let connected = s.peers.iter().filter(|peer| peer.online && s.broadcast.contains(&peer.name)).count();
            let title = format!("Broadcasting · {connected} of {} connected", s.broadcast.len());
            // The title names one; more than three are on the desk above.
            let names = match &s.broadcast[..] {
                [_] => "it".to_owned(),
                [rest @ .., last] if rest.len() < 3 => format!("{} and {last}", rest.join(", ")),
                _ if connected == 4 => "the four connected computers".to_owned(),
                _ => format!("the {connected} connected computers"),
            };
            return (title, format!("Everything you type, click and scroll here goes to {names}. {}", self.broadcast_stop_hint()));
        }
        if let Some(p) = &s.controlled_by {
            let sub = if cfg!(target_os = "linux") {
                format!("Choose Take back, or stop sending input from {p}. Control comes back by itself after a while without input from it.")
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
        if online.is_empty() && let Some(p) = s.peers.iter().find(|p| s.removed_us(&p.id)) {
            return (format!("{} removed this computer", p.name), "It no longer trusts this computer, so it refuses to connect. Remove it here too, or pair the two computers again.".into());
        }
        match online[..] {
            [] if s.peers.is_empty() => ("No computers paired yet".into(), "Choose Pair a computer, then do the same on the other computer.".into()),
            [] => (
                "Waiting for your computers".into(),
                "Paired computers appear here when oneMK is running on them and they're on the same network.".into(),
            ),
            // Carried away from our screens on the Computers view: it won't stay there.
            [p] if self.apart.contains(&p.id) && self.tab == Tab::Desk && self.dragging(&p.id) => (
                format!("Let go to place {}", p.name),
                format!("It moves beside the nearest edge of your screens, where the pointer crosses.{away}"),
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

    /// Ctrl and Cmd for every paired computer, a row each, from the top of
    /// `c` (the Settings card sizes itself to fit them).
    pub(super) fn swaps(&mut self, ui: &mut Ui, c: Rect) {
        let peers: Vec<_> = self.running().map(|s| s.peers.clone()).unwrap_or_default();
        if peers.is_empty() {
            para(ui, c.left_top(), c.width(), "Pair a computer to choose how Ctrl and Cmd work when you type on it.", regular(T_BODY), pal().ink2);
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
            p.text(pos2(row.left(), name_y), Align2::LEFT_CENTER, &name, regular(T_BODY), pal().ink);
            p.text(pos2(row.left() + nw + 8.0, name_y), Align2::LEFT_CENTER, os_label, regular(T_META), pal().ink2);
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
                focus: 0,
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
                buttons: vec![("Cancel".into(), Look::Quiet, Act::Close), ("Remove".into(), Look::Destructive, Act::Forget(id.clone(), name.clone()))],
                focus: 0,
                appear: *at,
                key: "remove".into(),
            });
        }
        if let Some((name, at)) = &self.deleting_profile {
            // Deleting a saved layout can't be undone: ask first. Cancel is
            // where the keyboard starts, never the button that deletes.
            return Some(SheetView {
                title: format!("Delete the layout “{name}”?"),
                sub: "It is removed from your saved layouts. The computers on the desk stay as they are now.".into(),
                code: None,
                check: None,
                buttons: vec![("Cancel".into(), Look::Quiet, Act::Close), ("Delete".into(), Look::Destructive, Act::DeleteProfile(name.clone()))],
                focus: 0,
                appear: *at,
                key: "delete-profile".into(),
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

    /// The sheet (handoff 5.8): pairing, where dropped files go, and every
    /// confirmation, one way: centred in the window over the dimmed window
    /// (the scrim), `surface`, corner 16, padding 24, at least 440 wide. It
    /// holds the keyboard while it is up (Tab goes round its own controls,
    /// Esc is its way out, a click on the scrim too where that is safe), and
    /// a body taller than the window scrolls inside it.
    fn sheet(&mut self, ui: &mut Ui, now: f64) {
        let (view, leaving) = match self.sheet_view() {
            Some(v) => {
                self.sheet_was = Some((v.clone(), now));
                (v, None)
            }
            None => match &self.sheet_was {
                Some((v, t)) if now - t < 0.16 => (v.clone(), Some(*t)),
                _ => {
                    // Closed: the keyboard goes back to where the sheet was
                    // opened from (the top bar's action).
                    if self.sheet_was.is_some() && widgets::keyboard_focus(ui.ctx()) {
                        ui.memory_mut(|m| m.request_focus(Id::unique("action")));
                    }
                    self.sheet_was = None;
                    self.sheet_rect = None;
                    self.sheet_key.clear();
                    return;
                }
            },
        };
        if leaving.is_none() && self.keys_on_sheet(ui) {
            return;
        }
        let ctx = ui.ctx().clone();
        let window = ctx.content_rect();
        let pad = SHEET_PAD;
        let widest = view.buttons.iter().map(|(label, _, _)| button_width(ui, label) + 2.0 * pad).fold(0.0f32, f32::max);
        let width = sheet_width(window.width(), widest);
        let inner = width - 2.0 * pad;
        let sub_g = {
            let mut job = egui::text::LayoutJob::simple(view.sub.clone(), regular(T_BODY), pal().ink2, inner);
            job.sections[0].format.line_height = Some((T_BODY * LEADING).round());
            ui.painter().layout_job(job)
        };
        // Buttons in a row if they fit, else one under another with the
        // way out last.
        let row = view.buttons.iter().map(|(l, _, _)| button_width(ui, l) + 8.0).sum::<f32>() - 8.0 <= inner;
        let mut view = view;
        if !row && view.buttons.first().is_some_and(|b| b.2 == Act::Close) {
            view.buttons.rotate_left(1);
            view.focus = view.focus.saturating_sub(1);
        }
        let widths: Vec<f32> = view.buttons.iter().map(|(l, _, _)| button_width(ui, l).min(inner)).collect();
        let button_labels: Vec<_> = view.buttons.iter().map(|(label, look, _)| ui.painter().layout(label.clone(), widgets::button_font(*look), pal().ink, inner - 28.0)).collect();
        let button_heights: Vec<f32> = button_labels.iter().map(|label| (label.size().y + 18.0).max(SHEET_BUTTON_H)).collect();
        let btn_h = if view.buttons.is_empty() { 0.0 } else if row { SHEET_BUTTON_H } else { button_heights.iter().sum::<f32>() + 8.0 * button_heights.len().saturating_sub(1) as f32 };
        let title_g = ui.painter().layout(view.title.clone(), semibold(T_TITLE), pal().ink, inner - if view.check.is_some() { 26.0 } else { 0.0 });
        let title_h = title_g.size().y.max(22.0);
        let alternatives = leaving.is_none() && self.pairing.as_ref().is_some_and(|pair| matches!(pair.step, Step::Searching { .. } | Step::Hint));
        let alt_h = if alternatives { settings_controls::pairing_controls_h(ui, inner) } else { 0.0 };
        let body = SheetBody { title: title_h, sub: sub_g.size().y, code: view.code.is_some(), alternatives: alt_h };
        let fit = sheet_fit(window, width, body.height(), btn_h);
        let rect = fit.rect;
        self.sheet_rect = Some(rect);

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
            ctx.request_repaint();
        }
        // The scrim: the window dimmed (light: black 20 %, dark: 50 %). It is
        // egui's modal layer, so nothing behind it takes the pointer or the
        // keyboard (Tab stays in the sheet), and a click on it is a "no"
        // where that is safe.
        let (scrim_area, sheet_area) = sheet_areas(window, rect);
        let layer = sheet_area.layer();
        hold_keyboard(&ctx, scrim_area.layer(), layer);
        let scrim_clicked = scrim_area
            .show(&ctx, |ui| {
                ui.painter().rect_filled(window, 0.0, crate::style::scrim().gamma_multiply(o));
                // Clicks, never the keyboard's focus.
                ui.allocate_rect(window, Sense::CLICK).clicked()
            })
            .inner
            && leaving.is_none()
            && o >= 0.5;
        let c = rect.center();
        ctx.set_transform_layer(layer, egui::emath::TSTransform::new(c.to_vec2() * (1.0 - s) + vec2(0.0, dy), s));
        let mut clicked = None;
        sheet_area.show(&ctx, |child| {
            child.set_opacity(o);
            if leaving.is_some() || o < 0.5 {
                child.disable();
            }
            // The sheet takes its own clicks: none reach the scrim.
            let _ = child.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
            let p = child.painter().clone();
            for shape in widgets::overlay_shapes(rect, R_SHEET) {
                p.add(shape);
            }
            a11y_text(child, Id::unique("sheet-dialog"), rect, Role::Dialog, &format!("{}. {}", view.title, view.sub), Some(Live::Polite));
            let body_rect = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(inner, fit.body));
            // What the sheet says, from `at` down (in a list that scrolls
            // when the window is too short for all of it).
            let draw_body = |this: &mut Self, ui: &mut Ui, at: Pos2| {
                let p = ui.painter().clone();
                let mut y = at.y;
                let mut tx = at.x;
                if let Some(t0) = view.check {
                    // A tick drawn in, then it holds.
                    let k = motion::progress(now, t0, motion::fade(0.24), motion::out_cubic);
                    widgets::draw_check(&p, pos2(at.x + 8.0, y + title_h / 2.0), 16.0, pal().ink, k);
                    tx += 26.0;
                    if k < 1.0 {
                        ui.ctx().request_repaint();
                    }
                }
                p.galley(pos2(tx, y), title_g.clone(), pal().ink);
                y += title_h + SheetBody::GAP_SUB;
                p.galley(pos2(at.x, y), sub_g.clone(), pal().ink2);
                y += sub_g.size().y;
                if let Some((code, _)) = &view.code {
                    y += SheetBody::GAP;
                    let block = Rect::from_min_size(pos2(at.x, y), vec2(inner, SheetBody::CODE_H));
                    pairing_code(ui, block, code);
                    y += SheetBody::CODE_H;
                }
                if alternatives {
                    y += SheetBody::GAP;
                    this.pairing_controls(ui, Rect::from_min_size(pos2(at.x, y), vec2(inner, alt_h)), now);
                }
            };
            if fit.scrolls {
                let mut list = child.new_child(UiBuilder::new().max_rect(body_rect));
                egui::ScrollArea::vertical().id_salt("sheet-body").max_height(fit.body).auto_shrink([false, false]).show(&mut list, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(inner, body.height()), Sense::hover());
                    draw_body(self, ui, r.min);
                });
            } else {
                draw_body(self, child, body_rect.min);
            }
            if view.buttons.is_empty() {
                return;
            }
            let y = rect.bottom() - pad - btn_h;
            let mut bx = rect.right() - pad - widths.iter().sum::<f32>() - 8.0 * widths.len().saturating_sub(1) as f32;
            for (k, ((label, look, act), w)) in view.buttons.iter().zip(&widths).enumerate() {
                let r = if row {
                    let r = Rect::from_min_size(pos2(bx, y), vec2(*w, SHEET_BUTTON_H));
                    bx += w + 8.0;
                    r
                } else {
                    Rect::from_min_size(pos2(rect.left() + pad, y + button_heights[..k].iter().sum::<f32>() + k as f32 * 8.0), vec2(inner, button_heights[k]))
                };
                let shown = if row { label.as_str() } else { "" };
                let id = Id::unique(("sheet-btn", &view.key, k));
                let resp = button(child, r, id, shown, *look, true, Some(label));
                if !row {
                    p.galley(r.center() - button_labels[k].size() / 2.0, button_labels[k].clone(), if matches!(*look, Look::Primary | Look::Destructive) { pal().on_accent } else { pal().ink });
                }
                // The keyboard starts on the safe button (never the one
                // that confirms), each time the sheet's step changes.
                if k == view.focus && self.sheet_key != view.key && o >= 0.5 {
                    resp.request_focus();
                    self.sheet_key = view.key.clone();
                }
                if resp.clicked() {
                    clicked = Some(act.clone());
                }
            }
        });
        if clicked.is_none() && scrim_clicked {
            clicked = self.scrim_act();
        }
        if let Some(act) = clicked {
            self.sheet_act(act, now);
        }
    }

    /// What a click on the scrim means: Cancel, except while two computers
    /// compare codes, wait for each other or have just paired (a stray click
    /// must not say "no" to the other computer).
    fn scrim_act(&self) -> Option<Act> {
        let confirm = self.pending_drop.is_some() || self.removing.is_some() || self.deleting_profile.is_some();
        match self.pairing.as_ref().map(|p| &p.step) {
            Some(Step::Code { .. } | Step::Waiting { .. } | Step::Paired { .. }) if !confirm => None,
            _ => Some(Act::Close),
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
            // Only the sheet showing goes (in `sheet_view`'s order): a
            // confirmation over pairing leaves the pairing as it was.
            Act::Close => {
                if self.pending_drop.take().is_none() && self.removing.take().is_none() && self.deleting_profile.take().is_none() {
                    self.pairing = None;
                }
            }
            Act::DeleteProfile(name) => {
                self.deleting_profile = None;
                self.profile_open = None;
                self.busy.push(Job::Profile { verb: "delete", name: name.clone() });
                self.link.profile("delete", &name);
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

    /// The toast: bottom centre over the page, the banner's look.
    fn toast_view(&mut self, ui: &mut Ui, page: Rect, now: f64) {
        let Some(t) = &self.toast else { return };
        let age = (now - t.at) as f32;
        let (enter, exit) = (motion::fade(0.22), motion::fade(0.16));
        if age > t.life {
            self.toast = None;
            return;
        }
        let a_in = motion::out_expo(age / enter);
        let a = a_in.min(((t.life - age) / exit).clamp(0.0, 1.0));
        // It floats at the bottom, centred, and rises a little as it comes.
        let w_max = 560.0f32.min(page.width() - 32.0);
        let (g, d, h) = desk_cards::toast_lay(ui, t, w_max);
        let w = (g.size().x.max(d.as_ref().map_or(0.0, |d| d.size().x)) + 46.0 + if t.error { desk_cards::toast_action_w(ui, t) } else { 0.0 }).min(w_max);
        // Clear of the sheet's buttons when one is up.
        let bottom = toast_bottom(page, h, ui.ctx().content_rect(), self.sheet_rect.filter(|_| self.sheet_was.is_some()));
        let y = bottom - h / 2.0 + motion::space(8.0) * (1.0 - a_in);
        let rect = Rect::from_center_size(pos2(page.center().x, y), vec2(w, h));
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::unique("toast"));
        let mut child = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(rect));
        child.set_opacity(a);
        let body = if t.error { Rect::from_min_max(rect.min, pos2(rect.right() - desk_cards::toast_action_w(ui, t), rect.bottom())) } else { rect };
        let resp = child.interact(body, Id::unique("toast-body"), Sense::CLICK);
        let p = child.painter().clone();
        for shape in widgets::overlay_shapes(rect, 12) {
            p.add(shape);
        }
        let dot = if t.error { pal().danger } else { pal().ink };
        let gh = g.size().y;
        // An error that fits on one line is centred on the band; a longer
        // one says why on the line below, as other messages do.
        let gy = if t.error && d.is_none() { rect.center().y - g.size().y / 2.0 } else { rect.top() + 10.0 };
        if t.error { widgets::status_mark(&p, pos2(rect.left() + 17.0, gy + 9.0), 7.0, widgets::Mark::Error); } else { p.circle_filled(pos2(rect.left() + 17.0, gy + 9.0), 3.5, dot); }
        if t.error && g.elided {
            resp.clone().on_hover_text(format!("{}{}", t.text, t.detail.as_ref().map_or(String::new(), |d| format!(" {d}"))));
        }
        p.galley(pos2(rect.left() + 30.0, gy), g, pal().ink);
        if let Some(d) = d {
            p.galley(pos2(rect.left() + 30.0, rect.top() + 14.0 + gh), d, pal().ink2);
        }
        // An error says what to do next: try again (the first thing to try,
        // first in Tab order too), its own way out, or Health's checks.
        let mut open_health = false;
        let mut try_again = false;
        let mut act = None;
        if t.error {
            let actions = desk_cards::toast_actions(t);
            let mut x = rect.right() - 12.0 - actions.iter().map(|(label, _)| button_width(&child, label)).sum::<f32>() - 8.0 * actions.len().saturating_sub(1) as f32;
            for (label, which) in actions {
                let bw = button_width(&child, label);
                let b = Rect::from_min_size(pos2(x, rect.center().y - 15.0), vec2(bw, 30.0));
                x += bw + 8.0;
                let hint = match which {
                    desk_cards::ToastButton::Retry => "Do it again",
                    desk_cards::ToastButton::Act => t.act.as_ref().map_or("", ToastAct::hint),
                    desk_cards::ToastButton::Health => "Open Health to see what went wrong",
                };
                if button(&mut child, b, Id::unique(("toast-action", label)), label, Look::Quiet, true, Some(hint)).clicked() {
                    match which {
                        desk_cards::ToastButton::Retry => try_again = true,
                        desk_cards::ToastButton::Act => act = t.act.clone(),
                        desk_cards::ToastButton::Health => open_health = true,
                    }
                }
            }
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
        // The log opens beside it: the advice stays up to follow.
        let acted = matches!(act, Some(ToastAct::Snap(_)));
        match act {
            Some(ToastAct::ShowLog) => self.link.show_log(),
            Some(ToastAct::Snap(id)) => self.snap(&id),
            None => {}
        }
        let t = self.toast.as_mut().expect("shown above");
        if (dismiss || open_health || try_again || acted) && age < t.life - exit {
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

    /// Ctrl+1 to 7 switch tabs (Cmd on a Mac).
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
        // One key per page, in the bar's order: ⌘1 Desk, ⌘2 Arrange, ⌘3
        // Wallpaper, ⌘4 Clipboard & files, ⌘5 Keys & mouse, ⌘6 Health, ⌘7
        // Settings (and ⌘, as everywhere).
        for key in shell::PAGE_KEYS {
            let Some(tab) = shell::tab_for_key(key) else { continue };
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

/// The sheet's two layers: the scrim over the whole `window`, and the
/// sheet at `rect` over it.
fn sheet_areas(window: Rect, rect: Rect) -> (egui::Area, egui::Area) {
    let area = |id: &str, at: Pos2| {
        egui::Area::new(Id::unique(id)).order(egui::Order::Foreground).fixed_pos(at).constrain(false).movable(false).interactable(true).fade_in(false)
    };
    (area("sheet-scrim", window.min), area("sheet-layer", rect.min))
}

/// While a sheet is up: the scrim is egui's modal layer (nothing behind it
/// takes the pointer, or a place in the Tab order: the keyboard stays in
/// the sheet), and the sheet is always over its scrim.
fn hold_keyboard(ctx: &egui::Context, scrim: egui::LayerId, sheet: egui::LayerId) {
    ctx.memory_mut(|m| {
        m.set_modal_layer(scrim);
        m.areas_mut().move_to_top(scrim);
        m.areas_mut().move_to_top(sheet);
    });
}

/// A sheet's buttons: 36 tall (handoff 6's middle size).
const SHEET_BUTTON_H: f32 = 36.0;

/// The heights in a sheet's body (handoff 5.8): the title, the sentence
/// under it, then the pairing code's block or pairing's other ways in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SheetBody {
    title: f32,
    sub: f32,
    code: bool,
    /// Pairing's other ways in (0: none).
    alternatives: f32,
}

impl SheetBody {
    const GAP_SUB: f32 = 6.0;
    const GAP: f32 = 16.0;
    /// The pairing code's `surface2` block.
    const CODE_H: f32 = 72.0;

    fn height(&self) -> f32 {
        self.title + Self::GAP_SUB + self.sub
            + if self.code { Self::GAP + Self::CODE_H } else { 0.0 }
            + if self.alternatives > 0.0 { Self::GAP + self.alternatives } else { 0.0 }
    }
}

/// A sheet's width in a window `window_w` wide: 440 at least, wider for a
/// button that needs it (`widest`, with its padding), never closer than 16
/// to the window's sides.
fn sheet_width(window_w: f32, widest: f32) -> f32 {
    widest.max(SHEET_MIN_W).min((window_w - 2.0 * SHEET_MARGIN).max(0.0))
}

/// Where a sheet goes: centred in the window, `body` tall inside its
/// padding and above its `buttons` row; when the window is too short for
/// all of it, the body scrolls (`scrolls`) and shows `body` of it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SheetFit {
    rect: Rect,
    body: f32,
    scrolls: bool,
}

fn sheet_fit(window: Rect, width: f32, body: f32, buttons: f32) -> SheetFit {
    let chrome = 2.0 * SHEET_PAD + if buttons > 0.0 { SHEET_PAD + buttons } else { 0.0 };
    let room = (window.height() - 2.0 * SHEET_MARGIN - chrome).max(48.0);
    let shown = body.min(room);
    SheetFit { rect: Rect::from_center_size(window.center(), vec2(width, chrome + shown)), body: shown, scrolls: shown < body }
}

/// The pairing code (handoff 5.8): JetBrains Mono 500 at 32, spaced 0.08 em,
/// in two groups of three, centred on a `surface2` block.
fn pairing_code(ui: &Ui, block: Rect, code: &str) {
    const SIZE: f32 = 32.0;
    let p = ui.painter();
    p.rect_filled(block, CornerRadius::same(R_INNER), pal().surface2);
    let font = mono_medium(SIZE);
    let digits: Vec<char> = code.chars().collect();
    // Each digit on a tile of its own (as a code field is), three and three;
    // set close (0.08 em) when the sheet is too narrow for tiles.
    let glyph = text_width(ui, "0", font.clone());
    let (cell_w, gap) = (glyph + 18.0, 6.0);
    let group = if digits.len() == 6 { 14.0 } else { 0.0 };
    let cell_h = (block.height() - 16.0).clamp(24.0, 52.0);
    let tiled = cell_w * digits.len() as f32 + gap * digits.len().saturating_sub(1) as f32 + group;
    let tiles = tiled <= block.width() - 16.0;
    let adv = if tiles { cell_w + gap } else { glyph + 0.08 * SIZE };
    let total = if tiles { tiled } else { adv * digits.len() as f32 - 0.08 * SIZE + group };
    let mut x = block.center().x - total / 2.0;
    for (i, ch) in digits.iter().enumerate() {
        if i == 3 {
            x += group;
        }
        if tiles {
            let cell = Rect::from_min_size(pos2(x, block.center().y - cell_h / 2.0), vec2(cell_w, cell_h));
            p.rect(cell, CornerRadius::same(8), pal().raised, Stroke::new(1.0, pal().line2), StrokeKind::Inside);
            p.text(cell.center(), Align2::CENTER_CENTER, ch, font.clone(), pal().ink);
        } else {
            p.text(pos2(x, block.center().y), Align2::LEFT_CENTER, ch, font.clone(), pal().ink);
        }
        x += adv;
    }
    let spoken: Vec<String> = digits.iter().map(char::to_string).collect();
    let spoken = if spoken.len() == 6 { format!("{}, {}", spoken[..3].join(" "), spoken[3..].join(" ")) } else { spoken.join(" ") };
    a11y_text(ui, Id::unique("sheet-code"), block, Role::Label, &format!("Pairing code {spoken}"), None);
}

/// The room a toast sits at the bottom of: the desk's side of the window
/// (`desk`: its scene, and whether its panel is stacked under it), the
/// whole content otherwise. Under a small window's desk the panel holds
/// what the toast is about (its buttons, its name): the toast stands just
/// over the panel's top edge, on the foot of the desk.
fn toast_room(content: Rect, desk: Option<(Rect, bool)>) -> Rect {
    match desk {
        Some((scene, false)) => scene.shrink2(vec2(desk_panel::EDGE, 0.0)),
        Some((scene, true)) => Rect::from_min_max(scene.min, pos2(scene.max.x, scene.max.y + 12.0)).shrink2(vec2(desk_panel::EDGE, 0.0)),
        None => content,
    }
}

/// Where a toast `h` tall ends: 16 above the foot of `room`, unless a
/// `sheet` is there; then under the sheet if there is room, else over it,
/// else (a window too short for both) over the sheet's title, never on its
/// buttons.
fn toast_bottom(room: Rect, h: f32, window: Rect, sheet: Option<Rect>) -> f32 {
    let bottom = room.bottom() - 16.0;
    let Some(sheet) = sheet else { return bottom };
    if bottom - h > sheet.bottom() + 8.0 {
        return bottom;
    }
    if sheet.bottom() + 12.0 + h <= window.bottom() - 8.0 {
        sheet.bottom() + 12.0 + h
    } else if sheet.top() - 12.0 - h >= window.top() + 8.0 {
        sheet.top() - 12.0
    } else {
        sheet.top() + 8.0 + h
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        crate::chrome::place_buttons(ui.ctx(), frame);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let ui_started = std::time::Instant::now();
        let now = ui.input(|i| i.time);
        self.now = now;
        // The theme first: everything below paints with it.
        self.themer.frame(ui.ctx(), self.appearance(), now);
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
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        debug::place_xy(self, now);
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
        // The Desk: the desk left of (or above) its panel (desk_panel.rs).
        let desk_lay = (self.tab == Tab::Desk).then(|| desk_panel::desk_layout(full, f.bar.bottom()));
        let desk = desk_lay.as_ref().map_or(content, |l| l.scene);
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
        let scene = desk;
        // The computers are framed below a banner over the desk, and below
        // the transfer where a small window puts it at the top.
        let mut frame = scene;
        let mut compact = false;
        let mut tools = None;
        let mut ask_row = None;
        if self.tab == Tab::Desk {
            // The banner on top, the toolbar under it, the hint along the
            // foot; the computers fit the handoff's box in what is left.
            let band = tween(ui.ctx(), Id::unique("desk-band"), self.band_h(ui, scene.width() - 2.0 * desk_panel::EDGE), motion::space(0.22), motion::out_cubic);
            let (free, row, ask) = self.desk_room(scene, band, full.width() < desk_panel::STACK_BELOW);
            tools = row;
            ask_row = ask;
            frame = free;
            self.desk_stage = Some(desk_panel::desk_stage(scene, free));
            // Too little room for monitors anyone could read: named chips.
            compact = (scene.height() < COMPACT_BELOW || frame.height() < 100.0) && self.running().is_some_and(|s| !s.peers.is_empty());
        }
        // The top bar and the toolbar are made before the desk, so Tab meets
        // them first (the pages, the bar's right side, the toolbar left to
        // right, then the computers, then the card); they are painted on a
        // layer above it.
        {
            let over = egui::LayerId::new(egui::Order::Middle, Id::unique("chrome-layer"));
            let mut top = ui.new_child(UiBuilder::new().layer_id(over).max_rect(full).id_salt("chrome"));
            self.top_bar(&mut top, f.bar, now);
            if let Some(row) = tools {
                let narrow = full.width() < desk_panel::STACK_BELOW;
                self.desk_toolbar(&mut top, row, narrow);
                // A narrow window: layout learning's question in a row of
                // its own over the desk, beside the toolbar's column.
                if let Some(ask) = ask_row {
                    self.suggestion_row(&mut top, ask);
                }
            }
        }
        if self.tab == Tab::Desk {
            if compact {
                self.desk_chips(ui, frame);
            } else {
                self.desk(ui, frame, full, now);
                // oneMK off: the desk steps back to 40 % (the ground laid
                // over it at 60 %, so no face shows through another).
                let k = tween(ui.ctx(), Id::unique("desk-off-dim"), if self.off() { 0.6 } else { 0.0 }, motion::fade(0.2), motion::out_cubic);
                if k > 0.0 {
                    ui.painter().rect_filled(scene, 0.0, pal().bg.gamma_multiply(k));
                }
            }
        }
        // Behind the glass changed: blur again (else each panel keeps its blur).
        let sig = self.backdrop_sig(full);
        if sig != self.backdrop_was {
            self.backdrop_was = sig;
            self.glass.changed();
        }
        if self.tab == Tab::Desk {
            let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
            // Chips can't be dragged: the hint is for the 3D desk.
            if !compact {
                self.desk_hint(ui, scene);
            }
            self.desk_overlay(ui, Rect::from_min_max(scene.min + vec2(desk_panel::EDGE, desk_panel::EDGE), scene.max - vec2(desk_panel::EDGE, 8.0)), hovering);
        }
        // Every page draws its own title (page_ui::head; Keys & mouse its
        // header row): its content starts at the top.
        let mut body = content;
        // Broadcasting is said on every page (the desk says it in its band).
        if self.tab != Tab::Desk {
            // Keys & mouse draws its own title row (handoff 5.5) from the
            // top of the content: a banner goes above it, not under a title.
            if self.tab == Tab::Keys {
                body.min.y = content.top() + 12.0;
            }
            let banner = self.page_banner(ui, body);
            body.min.y += banner;
            if self.tab == Tab::Keys && banner == 0.0 {
                body.min.y = content.top();
            }
        }
        match self.tab {
            Tab::Files => {
                let mut child = page(ui, body, "files");
                self.files_panel(&mut child, body);
            }
            Tab::Arrange => {
                let mut child = page(ui, body, "arrange");
                self.arrange_page(&mut child, body);
            }
            Tab::Wallpaper => {
                let mut child = page(ui, body, "wallpaper");
                self.wallpaper_page(&mut child, body);
            }
            Tab::Health => {
                let mut child = page(ui, body, "health");
                self.health(&mut child, body, now);
            }
            Tab::Settings => {
                let mut child = page(ui, body, "settings");
                self.settings(&mut child, body);
            }
            Tab::Keys => {
                let mut child = page(ui, body, "keys");
                self.keys_page(&mut child, body, now);
            }
            _ => {}
        }
        if let Some(l) = &desk_lay {
            self.desk_panel(ui, l.panel, now);
        }
        widgets::set_locked(ui.ctx(), false);
        // The sheet: centred in the window, over everything but the toast
        // and the palette.
        self.sheet(ui, now);
        self.toast_view(ui, toast_room(content, desk_lay.as_ref().map(|l| (l.scene, l.stacked))), now);
        self.palette(ui.ctx(), now);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        crate::chrome::paint_fake_buttons(&ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("fake-window-buttons"))));
        self.guide.show(ui.ctx());
        self.update_notice(ui.ctx());
        // Tell the link whether a graph was drawn: it asks the daemon for
        // round trips only then (a minimised window draws nothing).
        let (minimized, focused) = ui.input(|i| (i.viewport().minimized, i.viewport().focused));
        let attention = daemon::Attention::of(minimized, focused);
        self.link.showing.frame(self.graph_wanted() && attention != daemon::Attention::Hidden);
        self.link.showing.attend(attention);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_BENCH").is_some() {
            // The UI thread's own work this frame (laying out and painting
            // shapes), apart from waiting for the display.
            let ms = ui_started.elapsed().as_secs_f64() * 1000.0;
            ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::Mutex<Vec<f64>>>>(Id::unique("bench-ui")).lock().unwrap().push(ms));
        }
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if crate::gallery::wanted() {
            crate::gallery::show(ui);
        }
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        debug::tab_walk(ui.ctx(), now);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        debug::shot(ui.ctx(), now);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        widgets::targets::flush();
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        {
            debug::tab_press(_ctx, raw_input);
            debug::point_at(_ctx, raw_input);
            debug::press_enter(_ctx, raw_input);
            debug::plan_drag(_ctx, raw_input);
            debug::plan_keys(self, _ctx, raw_input);
        }
        // Nothing here is under a pointer that has gone elsewhere: no
        // tooltip or highlight is left standing over the window.
        let controlling = self.running().is_some_and(|s| s.controlling.is_some());
        let t = raw_input.time.unwrap_or(0.0);
        let since = *self.controlling_since.get_or_insert(t);
        if !controlling {
            self.controlling_since = None;
        }
        let seen = raw_input.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }));
        // Seen here a moment after control began: here until control ends.
        self.pointer_here = controlling && (self.pointer_here || (seen && t - since > 0.3));
        if pointer_away(controlling, self.pointer_here, self.was_focused, raw_input.focused) {
            raw_input.events.push(egui::Event::PointerGone);
        }
        self.was_focused = raw_input.focused;
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
///   `ONE_MK_APP_FOCUS_BAR=Wallpaper|search|action|update-pill|health-line`
///   on that control of the top bar;
/// - `ONE_MK_APP_TOAST=1` shows an error toast; `ONE_MK_APP_AUTOSTART=1`
///   shows start at login as on;
/// - `ONE_MK_APP_CONFIRM=1` says the codes match a second after they
///   show, and keeps the "paired" sheet up.
/// - `ONE_MK_APP_TAB=arrange|health|settings|wallpaper` (and files, keyboard);
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
///   would; `ONE_MK_APP_KEY_LATER=capslock` chooses that key at
///   `ONE_MK_APP_KEY_AT` seconds (default 1.5), for films of the zoom glide;
///   `ONE_MK_APP_BUTTON=4` opens a mouse button; `ONE_MK_APP_KEY_TEST=
///   58d0,58u85` types into the test box (code, down/up, ms).
/// - `ONE_MK_APP_COMPUTERS_MENU=1` opens the Desk's Choose computer menu
///   (5 computers or more);
/// - `ONE_MK_APP_BROADCAST=1` turns broadcast on (or off), as the Desk's
///   switch does.
/// - `ONE_MK_APP_THEME=light|dark|system` paints with that theme whatever
///   Settings → Appearance says; `ONE_MK_APP_THEME_FLIP=secs` turns to the
///   other theme that many seconds in (the cross-fade) (style.rs `Themer`).
/// - `ONE_MK_APP_GALLERY=1` draws every shared control in every state, in
///   both themes, over the window (gallery.rs).
/// - `ONE_MK_APP_FILM=dir` saves every frame from `ONE_MK_APP_FILM_FROM`
///   for `ONE_MK_APP_FILM_FOR` seconds, each named by its time, then quits
///   (motion evidence: the intro, a drag, the Keys zoom).
/// - Overlays: `ONE_MK_APP_MENU=swap|timer|key-more|navigator|natural|chooser`
///   opens that popover (widgets::debug_open_menu); `ONE_MK_APP_REMOVE=name`
///   asks whether to remove that computer; `ONE_MK_APP_TOAST=info` a plain
///   toast; `ONE_MK_APP_PAIR_STATE=searching|hint|code|waiting|paired|ended`
///   with `ONE_MK_APP_INVITE=link` (the invitation came) or
///   `ONE_MK_APP_JOIN=text` (typed to join); `ONE_MK_APP_TIP=1` a tooltip held
///   open; `ONE_MK_APP_PALETTE=query` the palette.
/// - `ONE_MK_APP_SSID=name` stands in for the Wi-Fi network on Arrange
///   (screenshots must not show the real one).
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
        // ONE_MK_APP_SETTINGS=edges|power|games|shortcuts|commands|typing|pointer|general:
        // Settings opens on that category. ONE_MK_APP_HEALTH_PASSED=1: the
        // passed checks of Health are open. ONE_MK_APP_DELETE_PROFILE=name:
        // the sheet that asks whether to delete that layout is up.
        if let Some(slug) = var("ONE_MK_APP_SETTINGS")
            && once(&ctx, "settings-cat")
            && let Some(cat) = settings_page::Cat::from_slug(&slug)
        {
            app.settings_cat = cat;
        }
        if var("ONE_MK_APP_HEALTH_PASSED").is_some() && once(&ctx, "health-passed") {
            app.health_passed_open = true;
        }
        if let Some(name) = var("ONE_MK_APP_DELETE_PROFILE")
            && now > 0.5
            && once(&ctx, "delete-profile")
        {
            app.deleting_profile = Some((name, now));
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
                "code" => Step::Code { name: var("ONE_MK_APP_PAIR_NAME").unwrap_or_else(|| "Studio PC".into()), code: "726480".into() },
                "waiting" => Step::Waiting { name: "Studio PC".into() },
                "paired" => Step::Paired { id: "1111-2222".into(), name: "Studio PC".into() },
                "ended" => Step::Ended("The codes did not match. Nothing was saved.".into()),
                _ => Step::Searching { me: app.running().map_or_else(String::new, |status| status.name.clone()), port: Some(45173) },
            };
            app.pairing = Some(PairUi { proc: None, step, since: now, opened: now, hint: state == "hint", retry: None, me: app.running().map_or_else(String::new, |status| status.name.clone()), port: Some(45173) });
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
        // ONE_MK_APP_TOAST=info: a plain message instead of an error.
        if ready && var("ONE_MK_APP_TOAST").as_deref() == Some("info") && now > 1.0 && once(&ctx, "toast") {
            app.toast("Sent 3 files to Studio PC.", now);
        }
        // ONE_MK_APP_REMOVE=name: the sheet that asks whether to remove that computer.
        if ready && now > 0.5 && let Some(name) = var("ONE_MK_APP_REMOVE") && once(&ctx, "remove") {
            let id = app.running().and_then(|s| s.peers.iter().find(|p| p.name == name)).map_or_else(|| "1111-2222".to_owned(), |p| p.id.clone());
            app.removing = Some((id, name, now));
        }
        // ONE_MK_APP_INVITE=link: pairing's private invitation link has come;
        // ONE_MK_APP_JOIN=text: that typed into "Join with a link".
        if let Some(link) = var("ONE_MK_APP_INVITE") {
            ctx.data_mut(|data| data.insert_temp(Id::unique("invitation-link"), link));
        }
        if let Some(text) = var("ONE_MK_APP_JOIN") && once(&ctx, "join") {
            app.drafts.insert("join-link".into(), text);
        }
        // ONE_MK_APP_TIP=1: a tooltip held open (over the search button),
        // as the pointer resting there would show it.
        if var("ONE_MK_APP_TIP").is_some() && now > 1.0 {
            let at = pos2(ctx.content_rect().center().x, 56.0);
            egui::Tooltip::always_open(ctx.clone(), egui::LayerId::background(), Id::unique("debug-tip"), egui::PopupAnchor::Position(at)).show(|ui| {
                ui.label("Search: open the command palette  Ctrl K");
            });
        }
        if ready && var("ONE_MK_APP_TOAST").is_some_and(|t| t != "info") && now > 1.0 && once(&ctx, "toast") {
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
        // ONE_MK_APP_FOCUS_BAR=Wallpaper|search|action|update-pill|health-line:
        // the keyboard on that control of the top bar (a page by its name).
        if let Some(name) = var("ONE_MK_APP_FOCUS_BAR") && now > 1.0 {
            ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
            let id = if matches!(name.as_str(), "search" | "action" | "update-pill" | "health-line") { Id::unique(name.as_str()) } else { Id::unique(("nav", name.as_str())) };
            ctx.memory_mut(|m| m.request_focus(id));
        }
        if ready && app.tab != Tab::Keys && var("ONE_MK_APP_FOCUS").is_some() && now > 1.0 {
            ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
            if let Some(id) = app.selected_id() {
                let target = Id::unique(("position", &id)).with(1usize);
                ctx.memory_mut(|m| m.request_focus(target));
                ctx.request_repaint();
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
        // ONE_MK_APP_KEY_LATER=capslock (at ONE_MK_APP_KEY_AT seconds, default 1.5):
        // that key is chosen then, as a click would, so a film shows the zoom glide.
        if app.tab == Tab::Keys
            && let Some(code) = var("ONE_MK_APP_KEY_LATER").and_then(|k| one_mk_proto::keynames::parse_key(&k).ok())
            && now > var("ONE_MK_APP_KEY_AT").and_then(|t| t.parse::<f64>().ok()).unwrap_or(1.5)
            && once(&ctx, "key-later")
        {
            app.select_key(code as u16, None);
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

    /// `ONE_MK_APP_PALETTE_ENTER=secs`: that many seconds in, presses Return
    /// once (a real key event): with `ONE_MK_APP_PALETTE=query` it chooses
    /// the first result, as the person would.
    pub fn press_enter(ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(at) = var("ONE_MK_APP_PALETTE_ENTER").and_then(|v| v.parse::<f64>().ok()) else { return };
        if raw.time.unwrap_or(0.0) >= at && once(ctx, "palette-enter") {
            raw.events.push(egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        }
    }

    /// `ONE_MK_APP_PLAN_DRAG=Name;dx,dy[;alt][;hold]` (with
    /// `ONE_MK_APP_TARGETS`, on Arrange): real pointer events pick up that
    /// computer's block on the plan (or the control of that name, such as
    /// a computer on the desk) at 1.5 s, carry it dx, dy points over
    /// 0.8 s (Alt held with `alt`) and let go at 2.5 s (not with `hold`:
    /// a screenshot of the drag).
    pub fn plan_drag(ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(spec) = var("ONE_MK_APP_PLAN_DRAG") else { return };
        let mut parts = spec.split(';');
        let (Some(name), Some(d)) = (parts.next(), parts.next()) else { return };
        let Some((dx, dy)) = d.split_once(',').and_then(|(x, y)| Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))) else { return };
        let flags: Vec<&str> = parts.collect();
        let (alt, hold) = (flags.contains(&"alt"), flags.contains(&"hold"));
        let t = raw.time.unwrap_or(0.0);
        if t < 3.5 {
            ctx.request_repaint_after_secs(0.03);
        }
        if t < 1.5 {
            return;
        }
        let key = Id::unique("plan-drag-from");
        let Some(from) = ctx.data(|d| d.get_temp::<Pos2>(key)).or_else(|| widgets::targets::rect_of(&format!("plan {name}")).or_else(|| widgets::targets::rect_of(name)).map(|r| r.center())) else { return };
        ctx.data_mut(|d| d.insert_temp(key, from));
        let modifiers = if alt { Modifiers::ALT } else { Modifiers::NONE };
        let step = Id::unique("plan-drag-step");
        let done = ctx.data(|d| d.get_temp::<u8>(step).unwrap_or(0));
        if done == 0 {
            raw.events.push(egui::Event::ModifiersChanged(modifiers));
            raw.events.push(egui::Event::PointerMoved(from));
            raw.events.push(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
            ctx.data_mut(|d| d.insert_temp(step, 1u8));
            return;
        }
        let k = ((t - 1.55) / 0.8).clamp(0.0, 1.0) as f32;
        let at = from + vec2(dx, dy) * k;
        if done == 1 {
            raw.events.push(egui::Event::PointerMoved(at));
        }
        if done == 1 && t > 2.5 && !hold {
            raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
            ctx.data_mut(|d| d.insert_temp(step, 2u8));
        }
    }

    /// `ONE_MK_APP_PLAN_KEYS=Name;Right,Down+,Enter` (on Arrange): at 1.5 s
    /// the keyboard goes to that computer's first block on the plan, then a
    /// key every 0.15 s (real key events; `+` holds Shift).
    pub fn plan_keys(app: &App, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(spec) = var("ONE_MK_APP_PLAN_KEYS") else { return };
        let Some((name, keys)) = spec.split_once(';') else { return };
        let t = raw.time.unwrap_or(0.0);
        if t < 4.0 {
            ctx.request_repaint_after_secs(0.03);
        }
        let Some(peer) = app.running().and_then(|s| s.peers.iter().find(|p| p.name == name)).cloned() else { return };
        if t < 1.5 {
            return;
        }
        // The window has the keyboard (a bare Xvfb has no window manager to say so).
        raw.focused = true;
        let keys: Vec<&str> = keys.split(',').collect();
        let id = Id::unique("plan-keys-done");
        let (done, last) = ctx.data(|d| d.get_temp::<(usize, f64)>(id)).unwrap_or((0, 1.5));
        if done == 0 {
            ctx.data_mut(|d| d.insert_temp(Id::unique("keyboard-focus"), true));
            ctx.memory_mut(|m| m.request_focus(Id::unique(("plan-screen", &peer.id, &format!("{} · 1", peer.name)))));
            if t < 1.8 {
                return;
            }
        }
        if done >= keys.len() || t < last + 0.15 {
            return;
        }
        let k = keys[done];
        let shift = k.ends_with('+');
        let key = match k.trim_end_matches('+') {
            "Left" => Key::ArrowLeft,
            "Right" => Key::ArrowRight,
            "Up" => Key::ArrowUp,
            "Down" => Key::ArrowDown,
            "Esc" => Key::Escape,
            _ => Key::Enter,
        };
        let modifiers = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        raw.events.push(egui::Event::ModifiersChanged(modifiers));
        raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        raw.events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
        ctx.data_mut(|d| d.insert_temp(id, (done + 1, t)));
    }

    /// `ONE_MK_APP_PLACE_XY=x,y` (on Arrange): at one second, Exact place
    /// of the chosen computer holds that X and Y, as if typed.
    pub fn place_xy(app: &mut App, now: f64) {
        let Some(xy) = var("ONE_MK_APP_PLACE_XY") else { return };
        let Some((x, y)) = xy.split_once(',') else { return };
        if now > 1.0 && app.tab == Tab::Arrange && once(&app.ctx.clone(), "place-xy")
            && let Some(id) = app.selected_id()
        {
            app.drafts.insert(format!("place-x:{id}"), x.to_owned());
            app.drafts.insert(format!("place-y:{id}"), y.to_owned());
        }
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
        // `ONE_MK_APP_PRESS=click`: let go 0.3 s later, a whole click.
        let released = Id::unique("point-at-released");
        if var("ONE_MK_APP_PRESS").is_some_and(|v| v == "click") && raw.time.unwrap_or(0.0) > 2.1 && !ctx.data(|d| d.get_temp::<bool>(released).unwrap_or(false)) {
            raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            ctx.data_mut(|d| d.insert_temp(released, true));
        }
    }

    pub fn tab_walk(ctx: &egui::Context, _now: f64) {
        if var("ONE_MK_APP_TABWALK").is_some() {
            ctx.request_repaint_after_secs(0.05);
        }
    }

    /// `ONE_MK_APP_FILM=dir` (with `ONE_MK_APP_FILM_FROM` and `_FOR`, in
    /// seconds): every frame in that window saved as `dir/<time>.ppm`, then
    /// quits; motion evidence (each file named by when it was drawn).
    fn film(ctx: &egui::Context, now: f64) {
        let Some(dir) = std::env::var_os("ONE_MK_APP_FILM") else { return };
        let num = |name: &str, or: f64| std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(or);
        let (from, span) = (num("ONE_MK_APP_FILM_FROM", 1.0), num("ONE_MK_APP_FILM_FOR", 2.0));
        ctx.request_repaint();
        if now > from + span + 0.5 {
            std::process::exit(0);
        }
        if now < from || now > from + span {
            return;
        }
        let path = std::path::PathBuf::from(dir).join(format!("{now:08.3}.ppm"));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let Ok(mut f) = std::fs::File::create(&path) else { return };
            let _ = writeln!(f, "P6 {} {} 255", img.size[0], img.size[1]);
            let rgb: Vec<u8> = img.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b()]).collect();
            let _ = f.write_all(&rgb);
        })));
    }

    pub fn shot(ctx: &egui::Context, now: f64) {
        film(ctx, now);
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
    fn truncated_name_prefers_focused_then_hovered_device() {
        let truncated = [1, 2].into_iter().collect();
        assert_eq!(truncated_name_target(Some(1), Some(2), &truncated), Some((1, true)));
        assert_eq!(truncated_name_target(Some(0), Some(2), &truncated), Some((2, false)));
        assert_eq!(truncated_name_target(None, Some(0), &truncated), None);
    }

    #[test]
    fn hover_ends_when_the_pointer_goes_elsewhere_unannounced() {
        // On another computer: every frame, whatever the focus.
        assert!(pointer_away(true, false, true, true));
        assert!(pointer_away(true, false, false, false));
        // ...unless it is seen here after all (oneMK couldn't hold it):
        // then hover and clicks work (Switch back here must be clickable).
        assert!(!pointer_away(true, true, true, true));
        // Focus lost: once, at that moment; moving back over it hovers again.
        assert!(pointer_away(false, false, true, false));
        assert!(!pointer_away(false, false, false, false));
        assert!(!pointer_away(false, false, true, true));
        assert!(!pointer_away(false, false, false, true));
        // egui drops a tooltip once the pointer is gone.
        let tip_shown = |events: Vec<egui::Event>| {
            let ctx = egui::Context::default();
            let mut shown = false;
            for frame in 0..30 {
                let input = egui::RawInput { events: if frame == 0 { vec![egui::Event::PointerMoved(pos2(20.0, 20.0))] } else if frame == 20 { events.clone() } else { vec![] }, time: Some(f64::from(frame) * 0.1), ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| {
                    let r = ui.interact(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)), Id::unique("tip"), Sense::hover());
                    shown = r.hovered();
                    r.on_hover_text("Edges lead nowhere while this is on.");
                });
                out.textures_delta.clear();
            }
            shown
        };
        assert!(tip_shown(vec![]), "hovered while the pointer stays");
        assert!(!tip_shown(vec![egui::Event::PointerGone]), "not after it is gone");
    }

    #[test]
    fn crowded_slots_leave_room_for_every_computer() {
        let peers = vec![("left".into(), Side::Left, 29.0), ("top".into(), Side::Top, 29.0), ("right".into(), Side::Right, 29.0), ("bottom".into(), Side::Bottom, 29.0)];
        let mut places = slots((-29.0, 29.0), &peers);
        let widths: Vec<_> = peers.iter().map(|(name, _, half)| (name.clone(), *half)).collect();
        separate_crowded_slots(&mut places, &widths, 29.0, &HashSet::new());
        let mut spans = vec![(-29.0, 29.0)];
        for (name, half) in &widths {
            let (position, scale) = places[name];
            spans.push((position.x - half * scale, position.x + half * scale));
        }
        spans.sort_by(|first, second| first.0.total_cmp(&second.0));
        assert!(spans.windows(2).all(|pair| pair[1].0 - pair[0].1 >= 31.9));
    }

    #[test]
    fn arranged_multi_monitor_device_stays_where_it_was_dropped() {
        let mut me = Device::new("me", "This computer", Kind::Monitor, true, WALLS[0]);
        me.screens = vec![one_mk_proto::Rect::new(0, 0, 1536, 864)];
        let mut scene = Scene::default();
        scene.devices.push(me);
        // Two computers with five monitors trigger the crowded-desk layout.
        let screens = vec![
            one_mk_proto::Rect::new(0, 0, 1920, 1080),
            one_mk_proto::Rect::new(-2592, 98, 1512, 982),
            one_mk_proto::Rect::new(1920, 0, 1920, 1080),
            one_mk_proto::Rect::new(-1080, -840, 1080, 1920),
        ];
        let fit = scene.fit(Kind::Monitor, &screens).unwrap();
        let expected = (fit.pos((-3840, -185)), fit.scale);
        let mut places = HashMap::from([("peer".to_owned(), expected)]);
        let widths = vec![("peer".to_owned(), scene::half_width(Kind::Monitor, &screens))];
        let mounted = HashSet::from(["peer".to_owned()]);
        for _ in 0..3 {
            separate_crowded_slots(&mut places, &widths, 29.0, &mounted);
            assert_eq!(places["peer"], expected, "automatic spacing must preserve an exact placement");
        }
    }

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
    fn a_sheet_fits_the_smallest_window_centred_and_scrolls_inside_when_it_must() {
        let small = Rect::from_min_size(Pos2::ZERO, vec2(820.0, 540.0));
        // 440 at least, wider for a long button, never closer than 16 to the sides.
        assert_eq!(sheet_width(820.0, 0.0), SHEET_MIN_W);
        assert_eq!(sheet_width(820.0, 500.0), 500.0);
        assert_eq!(sheet_width(820.0, 2000.0), 820.0 - 2.0 * SHEET_MARGIN);
        // The pairing code: title, sentence, the 72-point block of digit tiles, buttons.
        let code = SheetBody { title: 22.0, sub: 19.0, code: true, alternatives: 0.0 };
        assert_eq!(code.height(), 22.0 + 6.0 + 19.0 + 16.0 + 72.0);
        // Looking for a computer, with the other ways in: fits 820 × 540 whole.
        let looking = SheetBody { title: 22.0, sub: 38.0, code: false, alternatives: 181.0 };
        let fit = sheet_fit(small, sheet_width(820.0, 0.0), looking.height(), SHEET_BUTTON_H);
        assert!(!fit.scrolls);
        assert_eq!(fit.rect.center(), small.center());
        assert_eq!(fit.rect.width(), 440.0);
        assert!(small.shrink(SHEET_MARGIN).contains_rect(fit.rect), "{:?}", fit.rect);
        assert_eq!(fit.rect.height(), 2.0 * SHEET_PAD + looking.height() + SHEET_PAD + SHEET_BUTTON_H);
        // More than the window holds: the sheet stops 16 from the edges and
        // its body scrolls; the buttons stay.
        let fit = sheet_fit(small, 440.0, 900.0, SHEET_BUTTON_H);
        assert!(fit.scrolls);
        assert_eq!(fit.rect.height(), 540.0 - 2.0 * SHEET_MARGIN);
        assert_eq!(fit.body, 540.0 - 2.0 * SHEET_MARGIN - 3.0 * SHEET_PAD - SHEET_BUTTON_H);
        // No buttons (paired): no room kept for them.
        assert_eq!(sheet_fit(small, 440.0, 50.0, 0.0).rect.height(), 2.0 * SHEET_PAD + 50.0);
    }

    #[test]
    fn a_toast_keeps_clear_of_a_sheets_buttons() {
        let window = Rect::from_min_size(Pos2::ZERO, vec2(820.0, 540.0));
        // No sheet: 16 above the foot of its room.
        assert_eq!(toast_bottom(window, 52.0, window, None), 524.0);
        // A short sheet: under it.
        let sheet = Rect::from_center_size(window.center(), vec2(440.0, 160.0));
        let b = toast_bottom(window, 52.0, window, Some(sheet));
        assert!(b - 52.0 >= sheet.bottom() && b <= window.bottom() - 8.0, "{b}");
        // A tall one: above it, or over its title, never on its buttons
        // (the sheet's last 24 + 36 + 24).
        for h in [380.0, 420.0, 508.0] {
            let sheet = Rect::from_center_size(window.center(), vec2(440.0, h));
            let b = toast_bottom(window, 52.0, window, Some(sheet));
            let buttons_top = sheet.bottom() - SHEET_PAD - SHEET_BUTTON_H;
            assert!(b <= buttons_top || b - 52.0 >= sheet.bottom(), "{h}: {b}");
            assert!(b - 52.0 >= window.top() && b <= window.bottom(), "{h}: {b} on screen");
        }
        // The Desk in a small window: just over the panel (4 under the
        // scene), clear of it by 8.
        let scene = Rect::from_min_max(pos2(0.0, 60.0), pos2(820.0, 260.0));
        assert_eq!(toast_bottom(toast_room(window, Some((scene, true))), 52.0, window, None), scene.bottom() - 4.0);
        assert_eq!(toast_room(window, Some((scene, false))).bottom(), scene.bottom());
    }

    /// The focus check Xvfb screenshots can't show: with a sheet up, Tab
    /// goes round the sheet's own buttons and never reaches what is behind
    /// it; without the hold, it would.
    #[test]
    fn the_keyboard_stays_in_the_sheet() {
        let tab_round = |hold: bool| {
            let ctx = egui::Context::default();
            let window = Rect::from_min_size(Pos2::ZERO, vec2(820.0, 540.0));
            let rect = Rect::from_center_size(window.center(), vec2(440.0, 160.0));
            let (mut behind, mut ours) = (vec![], vec![]);
            for frame in 0..12 {
                let tab = egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
                let input = egui::RawInput { screen_rect: Some(window), events: if frame > 0 { vec![tab] } else { vec![] }, time: Some(f64::from(frame) * 0.1), ..Default::default() };
                let mut ids = (vec![], vec![]);
                let mut out = ctx.run_ui(input, |ui| {
                    ids.0.push(ui.add(egui::Button::new("A control behind")).id);
                    ids.0.push(ui.add(egui::Button::new("Another")).id);
                    let (scrim, sheet) = sheet_areas(window, rect);
                    if hold {
                        hold_keyboard(ui.ctx(), scrim.layer(), sheet.layer());
                    }
                    scrim.show(ui.ctx(), |ui| {
                        ui.allocate_rect(window, Sense::CLICK);
                    });
                    sheet.show(ui.ctx(), |ui| {
                        ids.1.push(ui.add(egui::Button::new("Cancel")).id);
                        ids.1.push(ui.add(egui::Button::new("Remove")).id);
                    });
                });
                out.textures_delta.clear();
                if let Some(f) = ctx.memory(|m| m.focused()) {
                    if ids.0.contains(&f) { behind.push(f) } else if ids.1.contains(&f) { ours.push(f) }
                }
            }
            (behind, ours)
        };
        let (behind, ours) = tab_round(true);
        assert!(behind.is_empty(), "Tab reached the window behind the sheet");
        assert_eq!(ours.iter().collect::<HashSet<_>>().len(), 2, "Tab goes round both buttons: {ours:?}");
        let (behind, _) = tab_round(false);
        assert!(!behind.is_empty(), "without the hold Tab would leave the sheet (the check is real)");
    }

    #[test]
    fn sentences_name_the_side() {
        assert_eq!(to_side(Side::Left), "to the left");
        assert_eq!(edge_word(Side::Top), "top");
        assert_eq!(items(1), "1 item");
        assert_eq!(items(3), "3 items");
    }
}
