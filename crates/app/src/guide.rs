//! The permission guide: while oneMK waits for a macOS permission, a
//! small card floats next to System Settings with an arrow at the list
//! where the switch is, says what to do, and watches. When the switch is
//! on it ticks, moves on to the next permission or leaves, and brings
//! oneMK back to the front. If System Settings is closed first, it says
//! so and offers to open it again.
//!
//! Three layers, each testable on its own:
//! - `place_card`: where the card goes, given the Settings window and the
//!   screens (pure);
//! - `Guide`: what the card says, driven by events and the clock (pure);
//! - `Overlay`: polls macOS (`one_mk_ctl::macos_windows`) and the
//!   permission source, runs the guide's effects and draws the card in a
//!   borderless, transparent, always-on-top viewport of its own.
//!
//! Everything in global points, origin at the top-left of the main
//! display, y down (macOS's CoreGraphics space). How it behaves, what is
//! unverified and how to test it on a Mac: docs/PERMISSION-GUIDE.md.

use crate::motion;
use crate::style::*;
use crate::widgets::{self, Look};
use egui::accesskit::{Live, Role};
use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2, ViewportId, pos2, vec2};
pub use one_mk_ctl::macos_windows::{Frame, Pane, SettingsWindow};

// ------------------------------------------------------------ permissions

/// A macOS permission oneMK needs, in the order they are asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    Accessibility,
    InputMonitoring,
}

/// The name oneMK has in System Settings' lists. The daemon (`one-mk`)
/// is the program that needs both permissions; see the TCC notes in
/// docs/PERMISSION-GUIDE.md for why this is not "oneMK".
pub const LIST_NAME: &str = "one-mk";

impl Permission {
    pub const ALL: [Permission; 2] = [Permission::Accessibility, Permission::InputMonitoring];

    pub fn pane(self) -> Pane {
        match self {
            Permission::Accessibility => Pane::Accessibility,
            Permission::InputMonitoring => Pane::InputMonitoring,
        }
    }

    /// The pane's name, as System Settings shows it.
    pub fn name(self) -> &'static str {
        match self {
            Permission::Accessibility => "Accessibility",
            Permission::InputMonitoring => "Input Monitoring",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// What it lets oneMK do.
    fn ability(self) -> &'static str {
        match self {
            Permission::Accessibility => "move the pointer and type on this Mac",
            Permission::InputMonitoring => "send your keyboard and mouse to other computers",
        }
    }

    /// The permission the daemon says is missing, from its status
    /// `input` lines (crates/input/src/macos.rs `Problem::message`).
    /// The daemon checks Accessibility first, so while it is missing
    /// nothing is known about Input Monitoring.
    pub fn missing_from_status(input: &[String]) -> Option<Permission> {
        input.iter().find_map(|l| {
            if l.starts_with("one-mk needs Accessibility permission") {
                Some(Permission::Accessibility)
            } else if l.starts_with("one-mk needs Input Monitoring permission") {
                Some(Permission::InputMonitoring)
            } else {
                None
            }
        })
    }
}

/// Where the guide learns whether a permission is on. Polled about twice
/// a second while the card shows.
pub trait PermissionSource {
    /// `Some(true)` when `p` is granted, `Some(false)` when not, `None`
    /// when this source can't tell (yet).
    fn granted(&mut self, p: Permission, now: f64) -> Option<bool>;
}

/// A source fed from the daemon's status `input` lines: set `lines`
/// whenever a new status arrives.
#[derive(Clone, Debug, Default)]
pub struct StatusPermissions {
    pub lines: Vec<String>,
}

impl PermissionSource for StatusPermissions {
    fn granted(&mut self, p: Permission, _now: f64) -> Option<bool> {
        match (Permission::missing_from_status(&self.lines), p) {
            (Some(missing), p) if missing == p => Some(false),
            // Input Monitoring is only checked once Accessibility is on.
            (Some(Permission::Accessibility), Permission::InputMonitoring) => None,
            _ => Some(true),
        }
    }
}

// ---------------------------------------------------------------- layout

/// Which way the arrow points (towards the Settings window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowDir {
    Left,
    Right,
    Up,
    Down,
}

impl ArrowDir {
    fn unit(self) -> Vec2 {
        match self {
            ArrowDir::Left => vec2(-1.0, 0.0),
            ArrowDir::Right => vec2(1.0, 0.0),
            ArrowDir::Up => vec2(0.0, -1.0),
            ArrowDir::Down => vec2(0.0, 1.0),
        }
    }
}

/// Where the card goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The card's top-left corner.
    pub pos: Pos2,
    /// Which way the arrow points; None: no arrow (no window to point at,
    /// or no room beside it).
    pub arrow_dir: Option<ArrowDir>,
    /// Where the arrow's tip is: just outside the Settings window, level
    /// with (or in line with) the list of switches.
    pub arrow_target: Option<Pos2>,
}

/// Room kept between the card and a screen's edge.
pub const EDGE: f32 = 12.0;
/// The arrow's length.
pub const ARROW_LEN: f32 = 38.0;
/// Between the arrow's tip and the Settings window.
pub const TIP_GAP: f32 = 8.0;
/// Between the arrow's tail and the card.
pub const TAIL_GAP: f32 = 6.0;
/// From the Settings window's edge to the card's.
pub const GUTTER: f32 = TIP_GAP + ARROW_LEN + TAIL_GAP;
/// The arrow keeps this far from the card's corners.
const ARROW_INSET: f32 = 26.0;
/// The tip keeps this far inside the window's span.
const SPAN_INSET: f32 = 18.0;
/// Used when no screen is known.
const DEFAULT_SCREEN: Frame = Frame::new(0.0, 0.0, 1440.0, 900.0);

/// Where the switches probably are in the Settings window: the list of
/// apps fills the right-hand pane under the pane's title and its
/// explanation, and each row's switch sits at its right end. Unverified
/// on a Mac (docs/PERMISSION-GUIDE.md); one place to adjust it.
pub fn switch_spot(a: &Frame) -> Pos2 {
    let y = a.y + (a.h * 0.32).clamp(110.0, 240.0).min(a.h * 0.8);
    let x = a.right() - 44.0_f32.min(a.w * 0.25);
    pos2(x, y)
}

fn inside(s: &Frame, p: Pos2) -> bool {
    p.x >= s.x && p.x <= s.right() && p.y >= s.y && p.y <= s.bottom()
}

/// `v` kept in `lo..=hi` (`lo` wins when the range is empty).
fn fit(v: f32, lo: f32, hi: f32) -> f32 {
    v.min(hi).max(lo)
}

/// The card's position along one axis inside `[lo, hi]`, centred on
/// `want` where possible. A card larger than the room starts at `lo`.
fn slide(want: f32, size: f32, lo: f32, hi: f32) -> f32 {
    let (lo, hi) = (lo + EDGE, hi - EDGE);
    if hi - lo < size {
        // Too big: show its start (title and close button) and let the
        // end run off; centre it when only the margins are short.
        return (lo - EDGE + ((hi - lo + 2.0 * EDGE) - size).max(0.0) / 2.0).min(lo);
    }
    fit(want - size / 2.0, lo, hi - size)
}

/// Where the card goes: beside the Settings window `anchor`, preferring
/// its right (where the switches are), then its left, below, above; on
/// the window's own screen first, then on a neighbouring one; else in a
/// corner. Never off-screen while the card fits on a screen.
pub fn place_card(anchor: Option<Frame>, screens: &[Frame], card: Vec2) -> Placement {
    let screens: Vec<Frame> = screens.iter().copied().filter(|s| s.w > 0.0 && s.h > 0.0).collect();
    let screens = if screens.is_empty() { vec![DEFAULT_SCREEN] } else { screens };
    let Some(a) = anchor.filter(|a| a.w > 0.0 && a.h > 0.0) else {
        return corner(&screens[0], card, true);
    };
    let centre = pos2(a.x + a.w / 2.0, a.y + a.h / 2.0);
    // The screen the window is mostly on: holding its centre, else the
    // one overlapping it most.
    let home = screens.iter().copied().find(|s| inside(s, centre)).or_else(|| {
        screens
            .iter()
            .copied()
            .map(|s| (s, overlap(&s, &a)))
            .filter(|(_, o)| *o > 0.0)
            .max_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(s, _)| s)
    });
    let Some(home) = home else {
        // The window is on no screen we know: don't point into the void.
        return corner(&screens[0], card, true);
    };
    let spot = switch_spot(&a);
    let order = [ArrowDir::Left, ArrowDir::Right, ArrowDir::Up, ArrowDir::Down];
    for s in std::iter::once(home).chain(screens.iter().copied().filter(|s| *s != home)) {
        // Above and below only on the window's own screen: across a bezel
        // the arrow would look broken.
        for dir in order {
            if s != home && matches!(dir, ArrowDir::Up | ArrowDir::Down) {
                continue;
            }
            if let Some(p) = beside(&a, &s, spot, dir, card) {
                return p;
            }
        }
    }
    corner(&home, card, false)
}

/// The card on `s` with its arrow pointing `dir` at the window `a`.
fn beside(a: &Frame, s: &Frame, spot: Pos2, dir: ArrowDir, card: Vec2) -> Option<Placement> {
    match dir {
        // The arrow points left: the card is to the window's right.
        ArrowDir::Left | ArrowDir::Right => {
            let x = if dir == ArrowDir::Left { a.right() + GUTTER } else { a.x - GUTTER - card.x };
            if x < s.x + EDGE || x + card.x > s.right() - EDGE {
                return None;
            }
            let y = slide(spot.y, card.y, s.y, s.bottom());
            // The tip stays level with the card's middle part, inside the
            // window's span and on the screen.
            let lo = (y + ARROW_INSET).max(a.y + SPAN_INSET).max(s.y + EDGE);
            let hi = (y + card.y - ARROW_INSET).min(a.bottom() - SPAN_INSET).min(s.bottom() - EDGE);
            if lo > hi {
                return None;
            }
            let ty = fit(spot.y, lo, hi);
            let tx = if dir == ArrowDir::Left { a.right() + TIP_GAP } else { a.x - TIP_GAP };
            Some(Placement { pos: pos2(x, y), arrow_dir: Some(dir), arrow_target: Some(pos2(tx, ty)) })
        }
        // The arrow points up: the card is below the window.
        ArrowDir::Up | ArrowDir::Down => {
            let y = if dir == ArrowDir::Up { a.bottom() + GUTTER } else { a.y - GUTTER - card.y };
            if y < s.y + EDGE || y + card.y > s.bottom() - EDGE {
                return None;
            }
            let x = slide(spot.x, card.x, s.x, s.right());
            let lo = (x + ARROW_INSET).max(a.x + SPAN_INSET).max(s.x + EDGE);
            let hi = (x + card.x - ARROW_INSET).min(a.right() - SPAN_INSET).min(s.right() - EDGE);
            if lo > hi {
                return None;
            }
            let tx = fit(spot.x, lo, hi);
            let ty = if dir == ArrowDir::Up { a.bottom() + TIP_GAP } else { a.y - TIP_GAP };
            Some(Placement { pos: pos2(x, y), arrow_dir: Some(dir), arrow_target: Some(pos2(tx, ty)) })
        }
    }
}

/// A corner of `s`, without an arrow: the top right (like a
/// notification) when there is nothing to point at, the bottom right
/// when Settings fills the screen (its list is at the top).
fn corner(s: &Frame, card: Vec2, top: bool) -> Placement {
    let x = slide(f32::INFINITY, card.x, s.x, s.right());
    let y = if top {
        slide(f32::NEG_INFINITY, card.y, s.y, s.bottom())
    } else {
        slide(f32::INFINITY, card.y, s.y, s.bottom())
    };
    Placement { pos: pos2(x, y), arrow_dir: None, arrow_target: None }
}

fn overlap(a: &Frame, b: &Frame) -> f32 {
    let w = a.right().min(b.right()) - a.x.max(b.x);
    let h = a.bottom().min(b.bottom()) - a.y.max(b.y);
    w.max(0.0) * h.max(0.0)
}

// ---------------------------------------------------------- state machine

/// How long the tick shows before the next step (or the goodbye).
pub const SUCCESS_HOLD: f64 = 1.2;
/// How long the card takes to leave.
pub const LEAVE_SECS: f64 = 0.32;
/// Settings may vanish for a moment (switching panes, Spaces): wait this
/// long before saying it closed.
pub const GONE_GRACE: f64 = 1.5;
/// After asking for the pane, how long Settings may take to show up.
pub const LAUNCH_GRACE: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Waiting for the switch.
    Waiting,
    /// The switch went on at `at`: the tick shows.
    Granted { at: f64 },
    /// Settings closed, hid, or never showed while waiting.
    SettingsGone { hidden: bool },
    /// All done (or the person closed the card) at `at`: the card leaves.
    Done { at: f64, dismissed: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideEvent {
    Granted(Permission),
    Revoked(Permission),
    Window(SettingsWindow),
    /// "Open Settings again".
    Retry,
    /// The card's close button.
    Dismiss,
}

/// What the guide asks the outside world to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    OpenPane(Pane),
    BringAppToFront,
    /// The card has left: drop the guide.
    Closed,
}

/// The guide's state: the steps, which one is showing, what is granted
/// and what is known about the Settings window.
#[derive(Clone, Debug)]
pub struct Guide {
    steps: Vec<Permission>,
    current: usize,
    granted: [bool; 2],
    phase: Phase,
    /// Since when the phase (and so the card's words) has been showing.
    phase_at: f64,
    window: SettingsWindow,
    /// When Settings was last lost from view; None while it shows.
    lost_at: Option<f64>,
    /// Whether Settings showed since the pane was last asked for.
    seen: bool,
    opened_at: f64,
    /// Where the Settings window was last seen.
    anchor: Option<Frame>,
    /// The phase the card was in when closed by hand.
    dismissed_from: Phase,
    finished: bool,
}

/// What the card shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// 1-based.
    pub step: usize,
    pub of: usize,
    pub permission: Permission,
    /// Which steps are done, in order (only the first `of` count).
    pub done: [bool; 2],
    pub phase: Phase,
    /// What the words say: the phase, except that a card being closed by
    /// hand keeps saying what it said.
    pub shown: Phase,
    /// Since when the words have been showing.
    pub phase_at: f64,
    /// The Settings window, while it shows.
    pub settings: Option<Frame>,
    /// Settings is there but another app covers it: hide the card.
    pub covered: bool,
    /// Where Settings was last seen.
    pub anchor: Option<Frame>,
    /// What comes after this step, if anything.
    pub next: Option<Permission>,
}

impl Guide {
    /// A guide through `steps` (duplicates dropped), with `granted`
    /// telling which are already on. Opens the first pane that is off;
    /// with nothing to do it is finished at once.
    pub fn new(steps: &[Permission], granted: impl Fn(Permission) -> bool, now: f64) -> (Self, Vec<Effect>) {
        let mut list: Vec<Permission> = Vec::new();
        for p in steps {
            if !list.contains(p) {
                list.push(*p);
            }
        }
        let mut g = Guide {
            granted: [false; 2],
            steps: list,
            current: 0,
            phase: Phase::Waiting,
            phase_at: now,
            window: SettingsWindow::NotRunning,
            lost_at: Some(now),
            seen: false,
            opened_at: now,
            anchor: None,
            dismissed_from: Phase::Waiting,
            finished: false,
        };
        for p in Permission::ALL {
            g.granted[p.index()] = granted(p);
        }
        let mut fx = Vec::new();
        match g.first_open() {
            Some(i) => {
                g.current = i;
                fx.push(Effect::OpenPane(g.steps[i].pane()));
            }
            None => {
                g.finished = true;
                g.phase = Phase::Done { at: now, dismissed: false };
            }
        }
        (g, fx)
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn steps(&self) -> &[Permission] {
        &self.steps
    }

    pub fn current(&self) -> Option<Permission> {
        self.steps.get(self.current).copied()
    }

    pub fn is_granted(&self, p: Permission) -> bool {
        self.granted[p.index()]
    }

    fn first_open(&self) -> Option<usize> {
        self.steps.iter().position(|p| !self.granted[p.index()])
    }

    fn set_phase(&mut self, phase: Phase, now: f64) {
        self.phase = phase;
        self.phase_at = now;
    }

    /// Shows the step at `i` and asks for its pane.
    fn go_to(&mut self, i: usize, now: f64) -> Vec<Effect> {
        self.current = i;
        self.set_phase(Phase::Waiting, now);
        self.opened_at = now;
        // A window already showing counts as seen: the URL only switches
        // its pane.
        self.seen = self.lost_at.is_none();
        vec![Effect::OpenPane(self.steps[i].pane())]
    }

    fn advance(&mut self, now: f64) -> Vec<Effect> {
        match self.first_open() {
            Some(i) => self.go_to(i, now),
            None => {
                // The words stay (and don't fade in again) as it leaves.
                self.phase = Phase::Done { at: now, dismissed: false };
                vec![Effect::BringAppToFront]
            }
        }
    }

    pub fn handle(&mut self, ev: GuideEvent, now: f64) -> Vec<Effect> {
        if self.finished {
            return Vec::new();
        }
        match ev {
            GuideEvent::Granted(p) => {
                if self.granted[p.index()] {
                    return Vec::new();
                }
                self.granted[p.index()] = true;
                if self.current() == Some(p) && matches!(self.phase, Phase::Waiting | Phase::SettingsGone { .. }) {
                    self.set_phase(Phase::Granted { at: now }, now);
                }
                // A later step granted early is skipped when we get there.
                Vec::new()
            }
            GuideEvent::Revoked(p) => {
                if !self.granted[p.index()] {
                    return Vec::new();
                }
                self.granted[p.index()] = false;
                let Some(i) = self.steps.iter().position(|s| *s == p) else { return Vec::new() };
                match self.phase {
                    // Leaving, or past this step: come back to it.
                    Phase::Done { dismissed: false, .. } => self.go_to(i, now),
                    _ if i < self.current => self.go_to(i, now),
                    // Ticked but not moved on yet: back to waiting, the
                    // pane is still open.
                    Phase::Granted { .. } if i == self.current => {
                        self.set_phase(Phase::Waiting, now);
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            GuideEvent::Window(w) => {
                self.window = w;
                match w {
                    SettingsWindow::Visible(f) | SettingsWindow::Covered(f) => {
                        self.anchor = Some(f);
                        self.lost_at = None;
                        self.seen = true;
                        if matches!(self.phase, Phase::SettingsGone { .. }) {
                            self.set_phase(Phase::Waiting, now);
                        }
                    }
                    SettingsWindow::Hidden | SettingsWindow::NotRunning => {
                        self.lost_at.get_or_insert(now);
                        if let Phase::SettingsGone { hidden } = self.phase {
                            // Hidden became closed, or the other way round.
                            let now_hidden = w == SettingsWindow::Hidden;
                            if hidden != now_hidden {
                                self.phase = Phase::SettingsGone { hidden: now_hidden };
                            }
                        }
                    }
                }
                Vec::new()
            }
            GuideEvent::Retry => match self.phase {
                Phase::Waiting | Phase::SettingsGone { .. } => self.go_to(self.current, now),
                _ => Vec::new(),
            },
            GuideEvent::Dismiss => {
                if !matches!(self.phase, Phase::Done { .. }) {
                    self.dismissed_from = self.phase;
                    self.phase = Phase::Done { at: now, dismissed: true };
                }
                Vec::new()
            }
        }
    }

    /// Moves time on: the tick hands over to the next step, a lost window
    /// becomes "closed", the goodbye ends.
    pub fn tick(&mut self, now: f64) -> Vec<Effect> {
        if self.finished {
            return Vec::new();
        }
        match self.phase {
            Phase::Granted { at } if now - at >= SUCCESS_HOLD => self.advance(now),
            Phase::Waiting => {
                let gone = match self.lost_at {
                    None => false,
                    Some(t) if self.seen => now - t >= GONE_GRACE,
                    Some(_) => now - self.opened_at >= LAUNCH_GRACE,
                };
                if gone {
                    let hidden = self.window == SettingsWindow::Hidden;
                    self.set_phase(Phase::SettingsGone { hidden }, now);
                }
                Vec::new()
            }
            Phase::Done { at, .. } if now - at >= LEAVE_SECS => {
                self.finished = true;
                vec![Effect::Closed]
            }
            _ => Vec::new(),
        }
    }

    pub fn view(&self) -> Option<View> {
        if self.finished {
            return None;
        }
        let permission = self.current()?;
        let mut done = [false; 2];
        for (i, p) in self.steps.iter().take(2).enumerate() {
            done[i] = self.granted[p.index()];
        }
        let next = self.steps.iter().skip(self.current + 1).find(|p| !self.granted[p.index()]).copied();
        let (settings, covered) = match self.window {
            SettingsWindow::Visible(f) => (Some(f), false),
            SettingsWindow::Covered(f) => (Some(f), true),
            _ => (None, false),
        };
        Some(View {
            step: self.current + 1,
            of: self.steps.len(),
            permission,
            done,
            phase: self.phase,
            shown: if let Phase::Done { dismissed: true, .. } = self.phase { self.dismissed_from } else { self.phase },
            phase_at: self.phase_at,
            settings,
            covered,
            anchor: self.anchor,
            next,
        })
    }
}

// ------------------------------------------------------------------ words

struct Words {
    title: String,
    body: String,
    footer: Footer,
}

enum Footer {
    Waiting,
    Note(String),
    Button(&'static str),
}

fn words(v: &View) -> Words {
    let p = v.permission;
    match v.shown {
        Phase::Waiting => Words {
            title: "Turn on oneMK".into(),
            body: match p {
                Permission::Accessibility => {
                    format!(
                        "Find {LIST_NAME} in the {} list and switch it on, so oneMK can {}.",
                        p.name(),
                        p.ability()
                    )
                }
                // The daemon gives the same advice (crates/input/src/macos.rs):
                // quitting would stop sharing until it is started again.
                Permission::InputMonitoring => format!(
                    "Find {LIST_NAME} in the {} list and switch it on. If macOS offers to quit and reopen it, choose Later.",
                    p.name()
                ),
            },
            footer: Footer::Waiting,
        },
        Phase::Granted { .. } | Phase::Done { dismissed: false, .. } => Words {
            title: format!("{} is on", p.name()),
            body: match v.next {
                Some(n) => format!("oneMK can now {}. One more to go: {}.", p.ability(), n.name()),
                None => format!("oneMK can now {}. That's everything it needs.", p.ability()),
            },
            footer: Footer::Note(match v.next {
                Some(n) => format!("Opening {}…", n.name()),
                None => "Taking you back to oneMK…".into(),
            }),
        },
        Phase::SettingsGone { hidden: true } => Words {
            title: "System Settings is out of sight".into(),
            body: format!("It's minimised or on another desktop, and {} isn't on yet.", p.name()),
            footer: Footer::Button("Show System Settings"),
        },
        Phase::SettingsGone { hidden: false } | Phase::Done { dismissed: true, .. } => Words {
            title: "System Settings closed".into(),
            body: format!("{} isn't on yet. Open it again to switch on {LIST_NAME}.", p.name()),
            footer: Footer::Button("Open Settings again"),
        },
    }
}

// ------------------------------------------------------------------ drawing

/// The card's width.
pub const CARD_W: f32 = 312.0;
const PAD: f32 = 18.0;
/// Room around the card in its window for the shadow.
const MARGIN: f32 = 30.0;
/// Corners as on the window's other small floating surfaces (its toasts).
const RADIUS: u8 = R_INNER;
const HEADER_H: f32 = 16.0;
const FOOTER_H: f32 = 30.0;

fn body_galley(ctx: &egui::Context, text: &str, width: f32, col: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.into(), regular(T_BODY), col, width);
    job.sections[0].format.line_height = Some((T_BODY * LEADING).round());
    ctx.fonts_mut(|f| f.layout_job(job))
}

fn title_galley(ctx: &egui::Context, text: &str, width: f32, col: Color32) -> std::sync::Arc<egui::Galley> {
    let job = egui::text::LayoutJob::simple(text.into(), semibold(T_TITLE), col, width);
    ctx.fonts_mut(|f| f.layout_job(job))
}

/// The card's size for this step: tall enough for every state's words, so
/// it doesn't jump when they change.
fn card_size(ctx: &egui::Context, v: &View) -> Vec2 {
    let w = CARD_W - 2.0 * PAD;
    let mut tallest: f32 = 0.0;
    for phase in [
        Phase::Waiting,
        Phase::Granted { at: 0.0 },
        Phase::SettingsGone { hidden: true },
        Phase::SettingsGone { hidden: false },
    ] {
        let words = words(&View { phase, shown: phase, ..*v });
        // The success title has the tick before it.
        let tw = if matches!(phase, Phase::Granted { .. }) { w - 30.0 } else { w };
        let h =
            title_galley(ctx, &words.title, tw, pal().ink).size().y + 6.0 + body_galley(ctx, &words.body, w, pal().ink2).size().y;
        tallest = tallest.max(h);
    }
    vec2(CARD_W, (PAD + HEADER_H + 14.0 + tallest + 16.0 + FOOTER_H + PAD).ceil())
}

/// What the person did on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Retry,
    Dismiss,
}

/// How far into the card's life we are, for its entrance and exit.
#[derive(Clone, Copy)]
struct Life {
    /// 0 → 1 as the card arrives; 1 → 0 as it leaves.
    shown: f32,
}

/// The arrow's nudge towards the window: three nudges, then a rest, so
/// it draws the eye without nagging; none with reduced motion.
fn nudge(now: f64, since: f64) -> (f32, bool) {
    if motion::reduced() {
        return (0.0, false);
    }
    const BEAT: f64 = 1.1;
    const CYCLE: f64 = BEAT * 3.0 + 1.8;
    let t = (now - since).max(0.0) % CYCLE;
    if t >= BEAT * 3.0 {
        return (0.0, false);
    }
    let k = ((t % BEAT) / BEAT) as f32;
    // Out fast, back slowly: a tap towards the target.
    let bump = if k < 0.3 { motion::out_cubic(k / 0.3) } else { 1.0 - motion::in_out_cubic((k - 0.3) / 0.7) };
    (bump * 6.0, true)
}

fn draw_arrow(p: &egui::Painter, dir: ArrowDir, tip: Pos2, alpha: f32) {
    let u = dir.unit();
    let n = vec2(-u.y, u.x);
    let tail = tip - u * ARROW_LEN;
    let head = 16.0;
    let half = 10.5;
    let base = tip - u * head;
    let tri = |grow: f32| {
        vec![tip + u * grow, base + n * (half + grow) - u * grow * 0.6, base - n * (half + grow) - u * grow * 0.6]
    };
    // A dark halo first, so it reads on light and dark windows alike.
    let halo = pal().bg.gamma_multiply(0.85 * alpha);
    p.line_segment([tail, base + u * 1.0], Stroke::new(9.0, halo));
    p.circle_filled(tail, 4.5, halo);
    p.add(egui::Shape::convex_polygon(tri(2.5), halo, Stroke::NONE));
    let col = pal().accent.gamma_multiply(alpha);
    p.line_segment([tail, base + u * 1.0], Stroke::new(4.0, col));
    p.circle_filled(tail, 2.0, col);
    p.add(egui::Shape::convex_polygon(tri(0.0), col, Stroke::NONE));
}

/// Draws the card at `card` (and its arrow, tip at `tip`) in `ui`;
/// returns what was clicked.
fn draw_card(
    ui: &mut Ui,
    card: Rect,
    arrow: Option<(ArrowDir, Pos2)>,
    v: &View,
    life: Life,
    now: f64,
) -> Option<Action> {
    let alpha = life.shown;
    if alpha <= 0.0 {
        return None;
    }
    let p = ui.painter().clone();
    // Shadow, body, hairline.
    let shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: pal().shadow((120.0 * alpha) as u8),
    };
    p.add(shadow.as_shape(card, CornerRadius::same(RADIUS)));
    p.rect_filled(card, CornerRadius::same(RADIUS), pal().surface.gamma_multiply(alpha));
    p.rect_stroke(card, CornerRadius::same(RADIUS), Stroke::new(1.0, pal().line2.gamma_multiply(alpha)), StrokeKind::Inside);

    let pointing = matches!(v.phase, Phase::Waiting) && v.settings.is_some();
    if let Some((dir, tip)) = arrow {
        let fade = motion::progress(now, v.phase_at, motion::fade(0.2), motion::out_cubic);
        let a = if pointing { fade } else { 1.0 - fade };
        if a > 0.0 {
            let (push, _) = nudge(now, v.phase_at);
            draw_arrow(&p, dir, tip + dir.unit() * push, a * alpha);
        }
    }

    let words = words(v);
    let inner = card.shrink(PAD);
    let ink = |c: Color32| c.gamma_multiply(alpha);
    // The new words fade in when the phase changes.
    let k = motion::progress(now, v.phase_at, motion::fade(0.22), motion::out_cubic);
    let rise = motion::space(4.0) * (1.0 - k);
    let wink = |c: Color32| c.gamma_multiply(alpha * k);

    // Header: step bars and "Step 1 of 2"; the close button.
    let hy = inner.top() + HEADER_H / 2.0;
    let mut x = inner.left();
    let label = if v.of > 1 { format!("Step {} of {}", v.step, v.of) } else { "oneMK setup".into() };
    if v.of > 1 {
        for i in 0..v.of.min(2) {
            let r = Rect::from_min_size(pos2(x, hy - 2.0), vec2(18.0, 4.0));
            let col = if v.done[i] {
                pal().accent
            } else if i + 1 == v.step {
                pal().ink
            } else {
                pal().surface2
            };
            p.rect_filled(r, CornerRadius::same(2), ink(col));
            x += 22.0;
        }
        x += 4.0;
    }
    p.text(pos2(x, hy), Align2::LEFT_CENTER, &label, regular(T_META), ink(pal().ink2));
    let close = Rect::from_center_size(pos2(card.right() - 10.0 - 13.0, card.top() + 10.0 + 13.0), vec2(26.0, 26.0));
    let mut action = None;
    if widgets::icon_button(ui, close, Id::unique("guide-close"), "Close this guide", true, |p, r, c| {
        draw_close(p, r, c.gamma_multiply(alpha))
    })
    .clicked()
    {
        action = Some(Action::Dismiss);
    }

    // Title (with a tick once granted) and the explanation.
    let mut y = inner.top() + HEADER_H + 14.0 + rise;
    let mut tx = inner.left();
    if matches!(v.shown, Phase::Granted { .. } | Phase::Done { dismissed: false, .. }) {
        let at = v.phase_at;
        let pop = motion::progress(now, at, motion::space(0.42), motion::back_out);
        let shown = motion::progress(now, at, motion::fade(0.16), motion::out_cubic);
        let draw = motion::progress(now, at + motion::space(0.12) as f64, motion::space(0.32), motion::out_cubic);
        let c = pos2(tx + 11.0, y + 10.0);
        let r = 11.0 * if motion::reduced() { 1.0 } else { 0.6 + 0.4 * pop };
        p.circle_filled(c, r, pal().accent.gamma_multiply(alpha * shown));
        widgets::draw_check(&p, c, 13.0, pal().on_accent.gamma_multiply(alpha * shown), draw);
        tx += 30.0;
    }
    let tg = title_galley(ui.ctx(), &words.title, inner.right() - tx, pal().ink);
    let title_rect = Rect::from_min_size(pos2(tx, y), tg.size());
    p.galley_with_override_text_color(title_rect.min, tg, wink(pal().ink));
    widgets::a11y_text(ui, Id::unique("guide-title"), title_rect, Role::Heading, &words.title, Some(Live::Polite));
    y = title_rect.bottom() + 6.0;
    let bg = body_galley(ui.ctx(), &words.body, inner.width(), pal().ink2);
    let body_rect = Rect::from_min_size(pos2(inner.left(), y), bg.size());
    p.galley_with_override_text_color(body_rect.min, bg, wink(pal().ink2));
    widgets::a11y_text(ui, Id::unique("guide-body"), body_rect, Role::Label, &words.body, None);

    // Footer: what's happening, or the one thing to do.
    let fy = inner.bottom() - FOOTER_H / 2.0 + rise;
    match words.footer {
        Footer::Waiting => {
            // A soft beat with the arrow's nudges: the guide is watching.
            // Still while the arrow rests and with reduced motion, so the
            // window can idle.
            let (_, nudging) = nudge(now, v.phase_at);
            let beat =
                if nudging { 0.5 - 0.5 * ((now - v.phase_at) * std::f64::consts::TAU / 1.1).cos() as f32 } else { 0.0 };
            let c = pos2(inner.left() + 4.0, fy);
            // Waiting is a state, not an action: ink, never Volt.
            p.circle_filled(c, 7.0, pal().ink2.gamma_multiply(alpha * k * 0.16 * beat));
            p.circle_filled(c, 3.5, wink(pal().ink2));
            let text = if v.covered || v.settings.is_none() {
                "Waiting for System Settings…"
            } else {
                "Watching for the switch…"
            };
            p.text(pos2(inner.left() + 16.0, fy), Align2::LEFT_CENTER, text, regular(T_META), wink(pal().ink2));
        }
        Footer::Note(text) => {
            p.text(pos2(inner.left(), fy), Align2::LEFT_CENTER, text, regular(T_META), wink(pal().ink2));
        }
        Footer::Button(label) => {
            let w = widgets::button_width(ui, label);
            let r = Rect::from_min_size(pos2(inner.left(), fy - FOOTER_H / 2.0), vec2(w, FOOTER_H));
            if k > 0.5 && widgets::button(ui, r, Id::unique("guide-retry"), label, Look::Primary, true, None).clicked()
            {
                action = Some(Action::Retry);
            }
        }
    }
    action
}

/// The card's rectangle and arrow, and the window that holds them, all in
/// global points.
struct Frame2 {
    card: Rect,
    arrow: Option<(ArrowDir, Pos2)>,
    window: Rect,
}

fn frame_for(pos: Pos2, size: Vec2, pl: &Placement) -> Frame2 {
    let card = Rect::from_min_size(pos, size);
    let arrow = pl.arrow_dir.zip(pl.arrow_target).map(|(d, t)| (d, t + (pos - pl.pos)));
    let mut window = card.expand(MARGIN);
    if let Some((d, tip)) = arrow {
        window = window.union(Rect::from_two_pos(tip + d.unit() * 4.0, tip - d.unit() * ARROW_LEN).expand(14.0));
    }
    Frame2 { card, arrow, window: Rect::from_min_max(window.min.floor(), window.max.ceil()) }
}

// ------------------------------------------------------------------ overlay

/// Where the Settings window comes from: macOS, or (debug builds) a
/// stand-in for screenshots and demos on any system.
enum Windows {
    Real,
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    Fake(debug::FakeWindow),
}

/// The guide on screen: owns the state, polls macOS and the permission
/// source, and draws the card in its own floating window.
pub struct Overlay {
    guide: Option<Guide>,
    source: Option<Box<dyn PermissionSource>>,
    windows: Windows,
    screens: Option<Vec<Frame>>,
    /// Draw a pretend desktop in the main window instead (screenshots).
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    stage: Option<debug::Stage>,
    next_window_poll: f64,
    next_source_poll: f64,
    last_frame: Option<Frame>,
    moved_at: f64,
    pos: Option<(motion::Tween, motion::Tween)>,
    /// The side the card was last on (its arrow's direction).
    side: Option<ArrowDir>,
    started_at: f64,
    /// Since when the card has been hidden (another app covers Settings).
    covered: motion::Tween,
    /// The card was shown for the permission gap that is still open, so it
    /// is not shown again (after "close") until a status says all is granted.
    offered: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            guide: None,
            source: None,
            windows: Windows::Real,
            screens: None,
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            stage: None,
            next_window_poll: 0.0,
            next_source_poll: 0.0,
            last_frame: None,
            moved_at: f64::NEG_INFINITY,
            pos: None,
            side: None,
            started_at: 0.0,
            covered: motion::Tween::new(0.0),
            offered: false,
        }
    }
}

/// The card's close button: a small cross. Drawn here rather than borrowed
/// from `widgets`, so the guide doesn't depend on the window's icon set.
fn draw_close(p: &egui::Painter, rect: Rect, col: Color32) {
    let (c, r) = (rect.center(), 4.0);
    let st = Stroke::new(1.5, col);
    p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], st);
    p.line_segment([c + vec2(-r, r), c + vec2(r, -r)], st);
}

const GUIDE_VIEWPORT: &str = "one-mk-permission-guide";

/// Whether the main window must be made with a GL config that has alpha:
/// eframe's glow backend chooses one config for every window from the main
/// window's `transparent` flag, and without alpha the guide's transparent
/// margin comes out black. The main window still paints itself opaque.
pub fn needs_transparent_root() -> bool {
    cfg!(target_os = "macos") || (cfg!(debug_assertions) && std::env::var_os("ONE_MK_APP_GUIDE").is_some())
}

impl Overlay {
    /// An idle overlay; in debug builds `ONE_MK_APP_GUIDE` and friends
    /// start it with stand-ins (see `debug`).
    pub fn from_env() -> Self {
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(o) = debug::from_env() {
            return o;
        }
        Self::default()
    }

    /// Starts guiding through `steps` (those already granted per `source`
    /// are skipped). Restarting while active starts over.
    pub fn start(&mut self, steps: &[Permission], mut source: Box<dyn PermissionSource>, now: f64) {
        let known: Vec<(Permission, bool)> =
            Permission::ALL.iter().map(|p| (*p, source.granted(*p, now) == Some(true))).collect();
        let (guide, fx) = Guide::new(steps, |p| known.iter().any(|(q, g)| *q == p && *g), now);
        self.guide = Some(guide);
        self.source = Some(source);
        self.pos = None;
        self.started_at = now;
        self.next_window_poll = now;
        self.next_source_poll = now + 0.5;
        self.covered = motion::Tween::new(0.0);
        self.run(fx, None);
    }

    /// Whether the card is up (or leaving).
    pub fn active(&self) -> bool {
        self.guide.as_ref().is_some_and(|g| !g.finished())
    }

    /// Tells the guide about a permission directly (instead of, or as
    /// well as, the source).
    pub fn set_granted(&mut self, p: Permission, granted: bool, now: f64) {
        let ev = if granted { GuideEvent::Granted(p) } else { GuideEvent::Revoked(p) };
        self.event(ev, now, None);
    }

    /// Called with every status the daemon reports (not when it isn't
    /// running: nothing is known then). On a Mac with a permission missing
    /// it opens the guide once per gap; while it is open it keeps it in step.
    pub fn on_status(&mut self, input: &[String], now: f64) {
        self.on_status_if(cfg!(target_os = "macos"), input, now);
    }

    fn on_status_if(&mut self, enabled: bool, input: &[String], now: f64) {
        if Permission::missing_from_status(input).is_some() {
            if !self.offered && !self.active() && enabled {
                let src = StatusPermissions { lines: input.to_vec() };
                self.start(&Permission::ALL, Box::new(src), now);
            }
            // A gap the open guide handles (also a revoke during its success
            // hold) counts as offered, so closing it is respected.
            if enabled && self.active() {
                self.offered = true;
            }
        } else if !self.active() {
            self.offered = false;
        }
        self.set_status_input(input, now);
    }

    /// Feeds the daemon's latest status `input` lines: they become the
    /// guide's source (so a poll never contradicts them) and take effect
    /// at once. Call it for every new status while the guide is active.
    pub fn set_status_input(&mut self, lines: &[String], now: f64) {
        if !self.active() {
            return;
        }
        let mut src = StatusPermissions { lines: lines.to_vec() };
        let known: Vec<(Permission, Option<bool>)> =
            Permission::ALL.iter().map(|p| (*p, src.granted(*p, now))).collect();
        self.source = Some(Box::new(src));
        for (p, g) in known {
            if let Some(g) = g {
                self.set_granted(p, g, now);
            }
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn stop(&mut self) {
        self.guide = None;
        self.source = None;
    }

    fn event(&mut self, ev: GuideEvent, now: f64, ctx: Option<&egui::Context>) {
        if let Some(g) = &mut self.guide {
            let fx = g.handle(ev, now);
            self.run(fx, ctx);
        }
    }

    fn run(&mut self, fx: Vec<Effect>, ctx: Option<&egui::Context>) {
        for e in fx {
            match e {
                Effect::OpenPane(pane) => match &mut self.windows {
                    // Errors show up as "System Settings didn't open" after
                    // the launch grace; nothing else to do here.
                    Windows::Real => {
                        let _ = one_mk_ctl::macos_windows::open_pane(pane);
                    }
                    #[cfg(any(debug_assertions, feature = "design-evidence"))]
                    Windows::Fake(f) => f.open(ctx.map_or(self.started_at, |c| c.input(|i| i.time))),
                },
                Effect::BringAppToFront => {
                    if matches!(self.windows, Windows::Real) {
                        one_mk_ctl::macos_windows::bring_to_front();
                    }
                    if let Some(ctx) = ctx {
                        ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Focus);
                    }
                }
                Effect::Closed => {
                    self.source = None;
                    self.pos = None;
                }
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context, now: f64) {
        let Some(g) = &self.guide else { return };
        if g.finished() {
            return;
        }
        if now >= self.next_window_poll {
            let w = match &mut self.windows {
                Windows::Real => one_mk_ctl::macos_windows::settings_window(),
                #[cfg(any(debug_assertions, feature = "design-evidence"))]
                Windows::Fake(f) => f.state(now),
            };
            let frame = match w {
                SettingsWindow::Visible(f) | SettingsWindow::Covered(f) => Some(f),
                _ => None,
            };
            if frame != self.last_frame {
                self.last_frame = frame;
                self.moved_at = now;
            }
            // Quick while the window is moving, so the card keeps up;
            // calm otherwise.
            self.next_window_poll = now + if now - self.moved_at < 1.0 { 1.0 / 30.0 } else { 0.25 };
            self.event(GuideEvent::Window(w), now, Some(ctx));
        }
        if now >= self.next_source_poll {
            self.next_source_poll = now + 0.5;
            let changes: Vec<GuideEvent> = match (&mut self.source, &self.guide) {
                (Some(src), Some(g)) => Permission::ALL
                    .iter()
                    .filter_map(|p| match src.granted(*p, now) {
                        Some(true) if !g.is_granted(*p) => Some(GuideEvent::Granted(*p)),
                        Some(false) if g.is_granted(*p) => Some(GuideEvent::Revoked(*p)),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for ev in changes {
                self.event(ev, now, Some(ctx));
            }
        }
        if let Some(g) = &mut self.guide {
            let fx = g.tick(now);
            self.run(fx, Some(ctx));
        }
    }

    fn screens(&self) -> Vec<Frame> {
        match &self.screens {
            Some(s) => s.clone(),
            None => one_mk_ctl::macos_windows::screens(),
        }
    }

    /// Call once per frame of the main window: polls, steps the guide and
    /// draws the card (in its own window). Costs nothing when idle.
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.guide.is_none() {
            return;
        }
        let now = ctx.input(|i| i.time);
        self.poll(ctx, now);
        let Some(view) = self.guide.as_ref().and_then(Guide::view) else {
            if self.guide.as_ref().is_some_and(Guide::finished) {
                self.guide = None;
            }
            return;
        };
        // Laid out in egui points; placed in the system's points (they
        // differ only when the window is zoomed).
        let zoom = ctx.zoom_factor();
        let size = card_size(ctx, &view);
        // Point at Settings while it shows; otherwise stay where it was.
        let anchor = view.settings.or(view.anchor);
        let screens = self.screens();
        let pl = place_card(anchor, &screens, size * zoom);
        // Follow smoothly (snap with reduced motion, and the first time).
        // Changing sides snaps too: gliding across Settings would cover it.
        if pl.arrow_dir != self.side {
            self.side = pl.arrow_dir;
            self.pos = None;
        }
        let (tx, ty) = self.pos.get_or_insert_with(|| (motion::Tween::new(pl.pos.x), motion::Tween::new(pl.pos.y)));
        let secs = motion::space(0.16);
        tx.go(pl.pos.x, now, secs, motion::out_cubic);
        ty.go(pl.pos.y, now, secs, motion::out_cubic);
        let pos = pos2(tx.value(now), ty.value(now));
        let moving = tx.moving(now) || ty.moving(now);
        let f = frame_for(pos, size * zoom, &pl);

        // Entrance, exit, and hiding while another app covers Settings.
        let arrive = motion::progress(now, self.started_at, motion::fade(0.24), motion::out_cubic);
        let leave = match view.phase {
            Phase::Done { at, .. } => {
                1.0 - motion::progress(now, at, motion::fade(LEAVE_SECS as f32), motion::in_cubic)
            }
            _ => 1.0,
        };
        self.covered.go(if view.covered { 1.0 } else { 0.0 }, now, motion::fade(0.2), motion::out_cubic);
        let shown = arrive * leave * (1.0 - self.covered.value(now));
        // Slide in from (and out to) the side away from the window.
        let away = f.arrow.map_or(vec2(0.0, -1.0), |(d, _)| -d.unit());
        let slide = away * motion::space(10.0) * (1.0 - arrive * leave);
        let life = Life { shown };

        let waiting = matches!(view.phase, Phase::Waiting);
        let (_, nudging) = nudge(now, view.phase_at);
        let busy = moving
            || arrive < 1.0
            || leave < 1.0
            || self.covered.moving(now)
            || now - view.phase_at < 1.4
            || (waiting && nudging);
        if busy {
            ctx.request_repaint();
        } else {
            // At rest: only keep polling (and wake for the next nudge).
            ctx.request_repaint_after_secs(((self.next_window_poll - now).max(0.0) + 0.001).min(0.25) as f32);
        }

        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if let Some(stage) = &self.stage {
            let arrow = f.arrow.map(|(d, t)| (d, t + slide));
            if let Some(a) = stage.draw(ctx, &screens, view.settings, f.card.translate(slide), arrow, &view, life, now)
            {
                self.act(a, now, ctx);
            }
            return;
        }
        if shown <= 0.0 && (view.covered || matches!(view.phase, Phase::Done { .. })) {
            // Fully hidden: no window at all.
            return;
        }
        let origin = f.window.min;
        let local = |p: Pos2| pos2((p.x + slide.x - origin.x) / zoom, (p.y + slide.y - origin.y) / zoom);
        let builder = egui::ViewportBuilder::default()
            .with_title("oneMK permission guide")
            .with_app_id("one-mk")
            .with_decorations(false)
            .with_transparent(true)
            .with_has_shadow(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false)
            .with_always_on_top()
            .with_position(pos2(origin.x / zoom, origin.y / zoom))
            .with_inner_size(f.window.size() / zoom);
        let card = Rect::from_min_size(local(f.card.min), size);
        let arrow = f.arrow.map(|(d, t)| (d, local(t)));
        let action = ctx.show_viewport_immediate(ViewportId::from_hash_of(GUIDE_VIEWPORT), builder, |ui, class| {
            if class == egui::ViewportClass::EmbeddedWindow {
                // No separate windows here: show the card inside the main
                // window, without an arrow.
                let (r, _) = ui.allocate_exact_size(card.size(), egui::Sense::hover());
                return draw_card(ui, r, None, &view, life, now);
            }
            draw_card(ui, card, arrow, &view, life, now)
        });
        if let Some(a) = action {
            self.act(a, now, ctx);
        }
    }

    fn act(&mut self, a: Action, now: f64, ctx: &egui::Context) {
        let ev = match a {
            Action::Retry => GuideEvent::Retry,
            Action::Dismiss => GuideEvent::Dismiss,
        };
        self.event(ev, now, Some(ctx));
    }
}

// ------------------------------------------------------------ debug aids

/// Development aids, debug builds only (like app.rs's `ONE_MK_APP_*`):
/// - `ONE_MK_APP_GUIDE=accessibility|input-monitoring|both` starts the guide;
/// - `ONE_MK_GUIDE_FAKE_RECT=x,y,w,h` stands in for the Settings window
///   (`hidden`, `closed`: none, as if minimised or quit); unset, macOS's
///   real one is used (none elsewhere);
/// - `ONE_MK_GUIDE_FAKE_SCREENS=x,y,w,h;x,y,w,h` the screens (else the real
///   ones, or one 1440x900 screen);
/// - `ONE_MK_GUIDE_FAKE_GRANT=secs` turns each permission on that long
///   after its step shows (unset: never);
/// - `ONE_MK_GUIDE_FAKE_CLOSE=secs` closes the stand-in window after that
///   long ("Open Settings again" brings it back);
/// - `ONE_MK_GUIDE_FAKE_MOVE=1` drifts the stand-in window to and fro;
/// - `ONE_MK_GUIDE_FAKE_COVER=a,b` covers it with another app from a to b s;
/// - `ONE_MK_GUIDE_STAGE=1|dark` draws a pretend desktop with a pretend
///   System Settings (light or dark) in the main window, with the card on
///   it, instead of a floating window: for screenshots (app.rs `shot`).
#[cfg(any(debug_assertions, feature = "design-evidence"))]
mod debug {
    use super::*;

    fn var(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    pub(super) fn frame(s: &str) -> Option<Frame> {
        let n: Vec<f32> = s.split(',').map(|v| v.trim().parse().ok()).collect::<Option<_>>()?;
        (n.len() == 4).then(|| Frame::new(n[0], n[1], n[2], n[3]))
    }

    pub(super) fn from_env() -> Option<Overlay> {
        let steps: Vec<Permission> = match var("ONE_MK_APP_GUIDE")?.as_str() {
            "accessibility" => vec![Permission::Accessibility],
            "input-monitoring" => vec![Permission::InputMonitoring],
            _ => vec![Permission::Accessibility, Permission::InputMonitoring],
        };
        let mut o = Overlay::default();
        if let Some(r) = var("ONE_MK_GUIDE_FAKE_RECT") {
            let base = match r.as_str() {
                "hidden" => SettingsWindow::Hidden,
                "closed" => SettingsWindow::NotRunning,
                r => frame(r).map_or(SettingsWindow::NotRunning, SettingsWindow::Visible),
            };
            let close = var("ONE_MK_GUIDE_FAKE_CLOSE").and_then(|s| s.parse().ok());
            let cover = var("ONE_MK_GUIDE_FAKE_COVER").and_then(|s| {
                let (a, b) = s.split_once(',')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            });
            o.windows = Windows::Fake(FakeWindow {
                base,
                close,
                cover,
                drift: var("ONE_MK_GUIDE_FAKE_MOVE").is_some(),
                reopened: None,
            });
        }
        if let Some(s) = var("ONE_MK_GUIDE_FAKE_SCREENS") {
            o.screens = Some(s.split(';').filter_map(frame).collect());
        } else if !cfg!(target_os = "macos") {
            o.screens = Some(vec![DEFAULT_SCREEN]);
        }
        o.stage = var("ONE_MK_GUIDE_STAGE").map(|s| Stage { dark: s == "dark" });
        let grant = var("ONE_MK_GUIDE_FAKE_GRANT").and_then(|s| s.parse().ok());
        let source: Box<dyn PermissionSource> = Box::new(FakeGrant { after: grant });
        o.start(&steps, source, 0.0);
        Some(o)
    }

    /// Grants Accessibility `after` seconds in, and Input Monitoring
    /// `after` seconds after its step shows (once Accessibility's tick has
    /// gone): each step is seen waiting for `after` seconds.
    struct FakeGrant {
        after: Option<f64>,
    }

    impl PermissionSource for FakeGrant {
        fn granted(&mut self, p: Permission, now: f64) -> Option<bool> {
            let Some(after) = self.after else { return Some(false) };
            let at = match p {
                Permission::Accessibility => after,
                Permission::InputMonitoring => 2.0 * after + SUCCESS_HOLD + 0.5,
            };
            Some(now >= at)
        }
    }

    pub(super) struct FakeWindow {
        base: SettingsWindow,
        close: Option<f64>,
        cover: Option<(f64, f64)>,
        drift: bool,
        reopened: Option<f64>,
    }

    impl FakeWindow {
        #[cfg(test)]
        pub(super) fn new(base: SettingsWindow, close: Option<f64>) -> Self {
            Self { base, close, cover: None, drift: false, reopened: None }
        }

        pub(super) fn open(&mut self, now: f64) {
            // The first open is the start; later ones bring it back if it
            // was gone (a showing window only switches panes).
            if now > 0.0 && !matches!(self.state(now), SettingsWindow::Visible(_) | SettingsWindow::Covered(_)) {
                self.reopened = Some(now);
            }
        }

        pub(super) fn state(&self, now: f64) -> SettingsWindow {
            if let Some(t) = self.reopened {
                // Settings takes a moment to come back.
                if now - t < 0.6 {
                    return SettingsWindow::NotRunning;
                }
            } else if self.close.is_some_and(|c| now >= c) {
                return SettingsWindow::NotRunning;
            }
            match self.base {
                SettingsWindow::Visible(mut f) => {
                    if self.drift {
                        f.x += (120.0 * (now * 0.9).sin()) as f32;
                        f.y += (40.0 * (now * 0.6).sin()) as f32;
                    }
                    if self.cover.is_some_and(|(a, b)| now >= a && now < b) {
                        SettingsWindow::Covered(f)
                    } else {
                        SettingsWindow::Visible(f)
                    }
                }
                // "Open again" brings a hidden or closed window back at
                // the middle of the main screen.
                _ if self.reopened.is_some() => SettingsWindow::Visible(Frame::new(360.0, 150.0, 715.0, 560.0)),
                other => other,
            }
        }
    }

    /// A pretend desktop for screenshots: the screens, a drawing of System
    /// Settings where the stand-in window is, and the card, at 1:1 when the
    /// main window is big enough.
    pub(super) struct Stage {
        dark: bool,
    }

    impl Stage {
        #[allow(clippy::too_many_arguments)]
        pub(super) fn draw(
            &self,
            ctx: &egui::Context,
            screens: &[Frame],
            settings: Option<Frame>,
            card: Rect,
            arrow: Option<(ArrowDir, Pos2)>,
            view: &View,
            life: Life,
            now: f64,
        ) -> Option<Action> {
            // The usable areas; the main screen's menu bar sits above its own.
            let mut screen_rects: Vec<Rect> =
                screens.iter().map(|s| Rect::from_min_size(pos2(s.x, s.y), vec2(s.w, s.h))).collect();
            let menu_bar = screen_rects
                .first()
                .filter(|r| r.top() > 0.0)
                .map(|r| Rect::from_min_max(pos2(r.left(), 0.0), pos2(r.right(), r.top())));
            if let (Some(bar), Some(first)) = (menu_bar, screen_rects.first_mut()) {
                *first = first.union(bar);
            }
            let bounds = screen_rects
                .iter()
                .copied()
                .reduce(Rect::union)
                .unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0)));
            let win = ctx.content_rect();
            let scale = (win.width() / bounds.width()).min(win.height() / bounds.height()).min(1.0);
            let offset = win.center() - bounds.center() * scale;
            let id = Id::unique("guide-stage");
            ctx.set_transform_layer(
                egui::LayerId::new(egui::Order::Foreground, id),
                egui::emath::TSTransform::new(offset, scale),
            );
            let mut action = None;
            egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(Pos2::ZERO).constrain(false).show(ctx, |ui| {
                // Cover the whole main window (in stage space).
                let all = Rect::from_min_max(
                    ((win.min - offset).to_vec2() / scale).to_pos2(),
                    ((win.max - offset).to_vec2() / scale).to_pos2(),
                );
                ui.set_clip_rect(all);
                let p = ui.painter().clone();
                p.rect_filled(all, 0.0, MAC_DESK);
                for (i, s) in screen_rects.iter().enumerate() {
                    wallpaper(&p, *s, i);
                }
                if let Some(bar) = menu_bar {
                    p.rect_filled(bar, 0.0, black(70));
                    p.circle_filled(pos2(bar.left() + 22.0, bar.center().y), 5.0, white(200));
                    for (k, w) in [34.0, 28.0, 30.0, 40.0].iter().enumerate() {
                        let x = bar.left() + 46.0 + k as f32 * 52.0;
                        p.rect_filled(
                            Rect::from_min_size(pos2(x, bar.center().y - 2.5), vec2(*w, 5.0)),
                            CornerRadius::same(2),
                            white(120),
                        );
                    }
                }
                if let Some(f) = settings {
                    settings_window(&p, Rect::from_min_size(pos2(f.x, f.y), vec2(f.w, f.h)), self.dark, view);
                }
                action = draw_card(ui, card, arrow, view, life, now);
            });
            action
        }
    }

    fn wallpaper(p: &egui::Painter, s: Rect, i: usize) {
        // A quiet two-tone gradient, a little different per screen.
        let [a, b] = MAC_WALLS[i.min(1)];
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(s.left_top(), a);
        mesh.colored_vertex(s.right_top(), widgets::mix(a, b, 0.5));
        mesh.colored_vertex(s.right_bottom(), b);
        mesh.colored_vertex(s.left_bottom(), widgets::mix(a, b, 0.5));
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(mesh);
    }

    /// A drawing of System Settings' privacy list (not a copy of it): a
    /// sidebar, a title, a few apps with switches, oneMK's off.
    fn settings_window(p: &egui::Painter, r: Rect, dark: bool, view: &View) {
        let [bg, side, card, line, ink, ink2, off] = if dark { MAC_DARK } else { MAC_LIGHT };
        let shadow = egui::epaint::Shadow { offset: [0, 18], blur: 50, spread: 0, color: black(110) };
        p.add(shadow.as_shape(r, CornerRadius::same(12)));
        p.rect_filled(r, CornerRadius::same(12), bg);
        let sw = (r.width() * 0.3).clamp(160.0, 230.0);
        let sb = Rect::from_min_size(r.min, vec2(sw, r.height()));
        p.rect_filled(sb, CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 }, side);
        for (k, c) in MAC_BUTTONS.into_iter().enumerate() {
            p.circle_filled(r.min + vec2(20.0 + k as f32 * 20.0, 20.0), 6.0, c);
        }
        // Sidebar: a search field and rows of grey, one chosen.
        p.rect_filled(Rect::from_min_size(r.min + vec2(12.0, 44.0), vec2(sw - 24.0, 26.0)), CornerRadius::same(7), off);
        for i in 0..((r.height() - 100.0) / 30.0).max(0.0) as usize {
            let y = r.top() + 92.0 + i as f32 * 30.0;
            let row = Rect::from_min_size(pos2(r.left() + 10.0, y - 12.0), vec2(sw - 20.0, 26.0));
            if i == 5 {
                p.rect_filled(row, CornerRadius::same(7), MAC_BLUE);
            }
            let col = if i == 5 { white(255) } else { ink2.gamma_multiply(0.7) };
            p.rect_filled(
                Rect::from_min_size(pos2(row.left() + 8.0, y - 7.0), vec2(14.0, 14.0)),
                CornerRadius::same(4),
                col.gamma_multiply(0.8),
            );
            p.rect_filled(
                Rect::from_min_size(pos2(row.left() + 30.0, y - 3.0), vec2(60.0 + (i * 37 % 50) as f32, 6.0)),
                CornerRadius::same(3),
                col.gamma_multiply(0.6),
            );
        }
        // The pane: back arrow and title, the explanation, the list.
        let x0 = sb.right() + 20.0;
        let w = r.right() - 20.0 - x0;
        let chev = Stroke::new(1.6, ink2);
        p.line_segment([pos2(x0 + 5.0, r.top() + 16.0), pos2(x0, r.top() + 22.0)], chev);
        p.line_segment([pos2(x0, r.top() + 22.0), pos2(x0 + 5.0, r.top() + 28.0)], chev);
        p.text(pos2(x0 + 18.0, r.top() + 22.0), Align2::LEFT_CENTER, view.permission.name(), semibold(T_BODY), ink);
        p.line_segment(
            [pos2(sb.right(), r.top() + 44.0), pos2(r.right() - 1.0, r.top() + 44.0)],
            Stroke::new(1.0, line),
        );
        let note = match view.permission {
            Permission::Accessibility => "Apps below may control this computer.",
            Permission::InputMonitoring => "Apps below may watch your keyboard and mouse.",
        };
        p.text(pos2(x0, r.top() + 66.0), Align2::LEFT_CENTER, note, regular(T_META), ink2);
        let rows = ["Terminal", LIST_NAME, "Script Editor", "Shortcuts"];
        let list_top = r.top() + 86.0;
        let row_h = 44.0;
        let list = Rect::from_min_size(pos2(x0, list_top), vec2(w, row_h * rows.len() as f32));
        p.rect_filled(list, CornerRadius::same(10), card);
        p.rect_stroke(list, CornerRadius::same(10), Stroke::new(1.0, line), StrokeKind::Inside);
        let granted = matches!(view.phase, Phase::Granted { .. } | Phase::Done { dismissed: false, .. });
        for (i, name) in rows.iter().enumerate() {
            let cy = list_top + row_h * (i as f32 + 0.5);
            if i > 0 {
                p.line_segment(
                    [pos2(x0 + 44.0, cy - row_h / 2.0), pos2(list.right(), cy - row_h / 2.0)],
                    Stroke::new(1.0, line),
                );
            }
            let ours = *name == LIST_NAME;
            let icon = Rect::from_center_size(pos2(x0 + 22.0, cy), vec2(24.0, 24.0));
            p.rect_filled(
                icon,
                CornerRadius::same(6),
                if ours { MAC_OURS } else { ink2.gamma_multiply(0.5) },
            );
            if ours {
                p.circle_filled(icon.center() - vec2(4.0, 0.0), 3.0, Palette::dark().accent);
                p.circle_filled(icon.center() + vec2(4.0, 0.0), 3.0, SCREEN_INK);
            }
            p.text(pos2(x0 + 44.0, cy), Align2::LEFT_CENTER, *name, regular(T_BODY), ink);
            let on = !ours || granted;
            let track = Rect::from_center_size(pos2(list.right() - 34.0, cy), vec2(38.0, 22.0));
            p.rect_filled(track, CornerRadius::same(11), if on { MAC_BLUE } else { off });
            let knob = if on { track.right() - 11.0 } else { track.left() + 11.0 };
            p.circle_filled(pos2(knob, cy), 9.5, white(255));
        }
        // The + and − buttons under the list.
        let by = list.bottom() + 22.0;
        p.text(pos2(x0 + 8.0, by), Align2::LEFT_CENTER, "+   −", regular(T_BODY), ink2);
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: Vec2 = vec2(312.0, 190.0);
    const MAC: Frame = Frame::new(0.0, 25.0, 1512.0, 857.0);

    fn on_screen(p: &Placement, s: &Frame) -> bool {
        p.pos.x >= s.x + EDGE - 0.01
            && p.pos.y >= s.y + EDGE - 0.01
            && p.pos.x + CARD.x <= s.right() - EDGE + 0.01
            && p.pos.y + CARD.y <= s.bottom() - EDGE + 0.01
    }

    fn card_rect(p: &Placement) -> Frame {
        Frame::new(p.pos.x, p.pos.y, CARD.x, CARD.y)
    }

    #[test]
    fn placement_table() {
        // (what, anchor, screens, expected arrow)
        let two = [MAC, Frame::new(1512.0, 0.0, 1920.0, 1080.0)];
        type Case = (&'static str, Option<Frame>, Vec<Frame>, Option<ArrowDir>);
        let cases: Vec<Case> = vec![
            (
                "middle of the screen: to its right",
                Some(Frame::new(300.0, 120.0, 715.0, 600.0)),
                vec![MAC],
                Some(ArrowDir::Left),
            ),
            (
                "against the right edge: to its left",
                Some(Frame::new(790.0, 120.0, 715.0, 600.0)),
                vec![MAC],
                Some(ArrowDir::Right),
            ),
            (
                "as wide as the screen, high up: below",
                Some(Frame::new(0.0, 25.0, 1512.0, 420.0)),
                vec![MAC],
                Some(ArrowDir::Up),
            ),
            (
                "as wide as the screen, low down: above",
                Some(Frame::new(0.0, 450.0, 1512.0, 420.0)),
                vec![MAC],
                Some(ArrowDir::Down),
            ),
            ("larger than the screen: a corner", Some(Frame::new(-40.0, 0.0, 1700.0, 1000.0)), vec![MAC], None),
            ("no window: a corner", None, vec![MAC], None),
            (
                "no screens known: still placed",
                Some(Frame::new(300.0, 120.0, 715.0, 600.0)),
                vec![],
                Some(ArrowDir::Left),
            ),
            (
                "against the right edge of the left screen: its left, same screen",
                Some(Frame::new(790.0, 120.0, 715.0, 600.0)),
                two.to_vec(),
                Some(ArrowDir::Right),
            ),
            (
                "filling the left screen: onto the right screen",
                Some(Frame::new(0.0, 25.0, 1512.0, 857.0)),
                two.to_vec(),
                Some(ArrowDir::Left),
            ),
            ("on the second screen", Some(Frame::new(2000.0, 200.0, 715.0, 600.0)), two.to_vec(), Some(ArrowDir::Left)),
            ("off every screen: a corner", Some(Frame::new(5000.0, 5000.0, 715.0, 600.0)), vec![MAC], None),
        ];
        for (what, anchor, screens, want) in cases {
            let p = place_card(anchor, &screens, CARD);
            assert_eq!(p.arrow_dir, want, "{what}: {p:?}");
            assert_eq!(p.arrow_dir.is_some(), p.arrow_target.is_some(), "{what}");
            let screens = if screens.is_empty() { vec![DEFAULT_SCREEN] } else { screens };
            assert!(screens.iter().any(|s| on_screen(&p, s)), "{what}: on a screen: {p:?}");
            if let Some(a) = anchor
                && p.arrow_dir.is_some()
            {
                assert_eq!(overlap(&card_rect(&p), &a), 0.0, "{what}: the card never covers the window");
            }
        }
    }

    #[test]
    fn the_arrow_tip_is_beside_the_window_level_with_the_card() {
        let a = Frame::new(300.0, 120.0, 715.0, 600.0);
        let p = place_card(Some(a), &[MAC], CARD);
        let tip = p.arrow_target.unwrap();
        assert_eq!(tip.x, a.right() + TIP_GAP);
        assert!(tip.y > p.pos.y + 20.0 && tip.y < p.pos.y + CARD.y - 20.0, "{p:?}");
        assert!(tip.y > a.y && tip.y < a.bottom());
        assert_eq!(p.pos.x, a.right() + GUTTER);
        // Level with the list of switches.
        assert_eq!(tip.y, switch_spot(&a).y);
    }

    #[test]
    fn a_window_at_the_top_or_bottom_edge_keeps_the_card_on_screen() {
        // A short window hugging the screen's top: the card can't centre
        // on the list, so it slides down and the arrow stays inside both.
        let a = Frame::new(200.0, 25.0, 715.0, 160.0);
        let p = place_card(Some(a), &[MAC], CARD);
        assert_eq!(p.arrow_dir, Some(ArrowDir::Left));
        assert!(on_screen(&p, &MAC));
        let tip = p.arrow_target.unwrap();
        assert!(tip.y >= a.y + SPAN_INSET && tip.y <= a.bottom() - SPAN_INSET, "{p:?}");
        let b = Frame::new(200.0, 760.0, 715.0, 122.0);
        let p = place_card(Some(b), &[MAC], CARD);
        assert!(on_screen(&p, &MAC), "{p:?}");
        assert!(p.arrow_dir.is_some(), "{p:?}");
        let tip = p.arrow_target.unwrap();
        assert!(tip.y >= p.pos.y + 20.0 && tip.y <= p.pos.y + CARD.y - 20.0, "{p:?}");
    }

    #[test]
    fn a_window_partly_off_screen_still_gets_an_on_screen_tip() {
        let a = Frame::new(-300.0, 100.0, 715.0, 600.0);
        let p = place_card(Some(a), &[MAC], CARD);
        assert_eq!(p.arrow_dir, Some(ArrowDir::Left));
        assert!(inside(&MAC, p.arrow_target.unwrap()));
    }

    #[test]
    fn a_tiny_screen_shows_the_cards_start() {
        let tiny = Frame::new(0.0, 0.0, 200.0, 150.0);
        let p = place_card(None, &[tiny], CARD);
        assert_eq!(p.pos, pos2(0.0, 0.0), "too big: pinned to the top-left");
        let p = place_card(Some(Frame::new(10.0, 10.0, 150.0, 120.0)), &[tiny], CARD);
        assert_eq!(p.arrow_dir, None);
        assert!(p.pos.x <= tiny.x + EDGE && p.pos.y <= tiny.y + EDGE);
        // Only the margins short: centred.
        let snug = Frame::new(0.0, 0.0, CARD.x + 10.0, CARD.y + 10.0);
        let p = place_card(None, &[snug], CARD);
        assert_eq!(p.pos, pos2(5.0, 5.0));
    }

    #[test]
    fn a_window_across_two_screens_uses_the_one_holding_its_middle() {
        let right = Frame::new(1512.0, 0.0, 1920.0, 1080.0);
        // Mostly on the right screen, sticking out to the left one.
        let a = Frame::new(1300.0, 200.0, 715.0, 600.0);
        let p = place_card(Some(a), &[MAC, right], CARD);
        assert_eq!(p.arrow_dir, Some(ArrowDir::Left));
        assert!(on_screen(&p, &right), "{p:?}");
    }

    #[test]
    fn a_screen_above_counts_too() {
        // A second screen above the main one; the window high on it.
        let above = Frame::new(0.0, -1080.0, 1920.0, 1080.0);
        let a = Frame::new(1100.0, -1000.0, 715.0, 600.0);
        let p = place_card(Some(a), &[MAC, above], CARD);
        assert!(on_screen(&p, &above), "{p:?}");
        assert!(p.arrow_dir.is_some());
    }

    // --- state machine

    fn grants(list: &'static [Permission]) -> impl Fn(Permission) -> bool {
        move |p| list.contains(&p)
    }

    const BOTH: [Permission; 2] = [Permission::Accessibility, Permission::InputMonitoring];
    const W: SettingsWindow = SettingsWindow::Visible(Frame::new(100.0, 100.0, 715.0, 600.0));

    #[test]
    fn grants_in_order_walk_both_steps_then_close() {
        let (mut g, fx) = Guide::new(&BOTH, grants(&[]), 0.0);
        assert_eq!(fx, vec![Effect::OpenPane(Pane::Accessibility)]);
        g.handle(GuideEvent::Window(W), 1.0);
        let v = g.view().unwrap();
        assert_eq!((v.step, v.of, v.permission), (1, 2, Permission::Accessibility));
        assert!(g.handle(GuideEvent::Granted(Permission::Accessibility), 2.0).is_empty());
        assert_eq!(g.phase(), Phase::Granted { at: 2.0 });
        assert!(g.tick(2.5).is_empty(), "the tick holds");
        assert_eq!(g.tick(2.0 + SUCCESS_HOLD + 1e-6), vec![Effect::OpenPane(Pane::InputMonitoring)]);
        let v = g.view().unwrap();
        assert_eq!((v.step, v.permission, v.phase), (2, Permission::InputMonitoring, Phase::Waiting));
        assert_eq!(v.done, [true, false]);
        g.handle(GuideEvent::Granted(Permission::InputMonitoring), 5.0);
        assert_eq!(g.tick(5.0 + SUCCESS_HOLD + 1e-6), vec![Effect::BringAppToFront]);
        assert!(matches!(g.phase(), Phase::Done { dismissed: false, .. }));
        assert!(g.view().is_some(), "still leaving");
        assert_eq!(g.tick(5.0 + SUCCESS_HOLD + LEAVE_SECS + 1e-6), vec![Effect::Closed]);
        assert!(g.finished() && g.view().is_none());
        assert!(g.tick(100.0).is_empty(), "closed once");
    }

    #[test]
    fn grants_out_of_order_skip_the_step_already_done() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.handle(GuideEvent::Window(W), 0.5);
        // Input Monitoring first (the person went ahead).
        g.handle(GuideEvent::Granted(Permission::InputMonitoring), 1.0);
        assert_eq!(g.phase(), Phase::Waiting, "still on Accessibility");
        assert_eq!(g.view().unwrap().next, None, "nothing after this one");
        g.handle(GuideEvent::Granted(Permission::Accessibility), 2.0);
        assert_eq!(g.tick(2.0 + SUCCESS_HOLD + 1e-6), vec![Effect::BringAppToFront], "straight to done");
    }

    #[test]
    fn already_granted_steps_are_skipped_at_start() {
        let (g, fx) = Guide::new(&BOTH, grants(&[Permission::Accessibility]), 0.0);
        assert_eq!(fx, vec![Effect::OpenPane(Pane::InputMonitoring)]);
        let v = g.view().unwrap();
        assert_eq!((v.step, v.of, v.done), (2, 2, [true, false]));
        let (g, fx) = Guide::new(&BOTH, grants(&BOTH), 0.0);
        assert!(fx.is_empty() && g.finished());
        let (g, fx) = Guide::new(&[], grants(&[]), 0.0);
        assert!(fx.is_empty() && g.finished());
    }

    #[test]
    fn duplicate_steps_and_double_events_change_nothing() {
        let (mut g, _) = Guide::new(&[Permission::Accessibility, Permission::Accessibility], grants(&[]), 0.0);
        assert_eq!(g.steps().len(), 1);
        g.handle(GuideEvent::Granted(Permission::Accessibility), 1.0);
        assert!(g.handle(GuideEvent::Granted(Permission::Accessibility), 1.1).is_empty());
        assert_eq!(g.phase(), Phase::Granted { at: 1.0 }, "the second grant doesn't restart the tick");
        assert!(g.handle(GuideEvent::Revoked(Permission::InputMonitoring), 1.2).is_empty(), "never granted");
        assert_eq!(g.handle(GuideEvent::Retry, 1.3), vec![], "nothing to retry while ticking");
    }

    #[test]
    fn closing_settings_says_so_after_a_moment_and_retry_reopens() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.handle(GuideEvent::Window(W), 1.0);
        g.handle(GuideEvent::Window(SettingsWindow::NotRunning), 3.0);
        g.tick(3.0 + GONE_GRACE - 0.1);
        assert_eq!(g.phase(), Phase::Waiting, "a moment's grace (switching panes)");
        // Back within the grace: nothing happened.
        g.handle(GuideEvent::Window(W), 4.0);
        g.tick(10.0);
        assert_eq!(g.phase(), Phase::Waiting);
        g.handle(GuideEvent::Window(SettingsWindow::NotRunning), 11.0);
        g.tick(11.0 + GONE_GRACE);
        assert_eq!(g.phase(), Phase::SettingsGone { hidden: false });
        assert_eq!(g.handle(GuideEvent::Retry, 14.0), vec![Effect::OpenPane(Pane::Accessibility)]);
        assert_eq!(g.phase(), Phase::Waiting);
        // Settings takes its time to launch: no complaint until the launch grace.
        g.tick(14.0 + GONE_GRACE + 1.0);
        assert_eq!(g.phase(), Phase::Waiting);
        g.tick(14.0 + LAUNCH_GRACE);
        assert_eq!(g.phase(), Phase::SettingsGone { hidden: false });
        // It shows: back to waiting.
        g.handle(GuideEvent::Window(W), 30.0);
        assert_eq!(g.phase(), Phase::Waiting);
    }

    #[test]
    fn hidden_and_closed_are_told_apart() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.handle(GuideEvent::Window(W), 1.0);
        g.handle(GuideEvent::Window(SettingsWindow::Hidden), 2.0);
        g.tick(2.0 + GONE_GRACE);
        assert_eq!(g.phase(), Phase::SettingsGone { hidden: true });
        g.handle(GuideEvent::Window(SettingsWindow::NotRunning), 5.0);
        assert_eq!(g.phase(), Phase::SettingsGone { hidden: false }, "then quit");
    }

    #[test]
    fn never_showing_settings_counts_as_closed_after_the_launch_grace() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.tick(LAUNCH_GRACE - 0.1);
        assert_eq!(g.phase(), Phase::Waiting);
        g.tick(LAUNCH_GRACE);
        assert_eq!(g.phase(), Phase::SettingsGone { hidden: false });
    }

    #[test]
    fn a_grant_while_settings_is_closed_still_counts() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.tick(LAUNCH_GRACE);
        assert!(matches!(g.phase(), Phase::SettingsGone { .. }));
        // e.g. granted through the system's own prompt, or by MDM.
        g.handle(GuideEvent::Granted(Permission::Accessibility), 9.0);
        assert_eq!(g.phase(), Phase::Granted { at: 9.0 });
        assert_eq!(g.tick(9.0 + SUCCESS_HOLD + 1e-6), vec![Effect::OpenPane(Pane::InputMonitoring)]);
    }

    #[test]
    fn revoking_after_a_grant_goes_back_to_that_step() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.handle(GuideEvent::Window(W), 0.5);
        g.handle(GuideEvent::Granted(Permission::Accessibility), 1.0);
        // Turned off again during the tick: back to waiting, pane still open.
        assert!(g.handle(GuideEvent::Revoked(Permission::Accessibility), 1.5).is_empty());
        assert_eq!(g.phase(), Phase::Waiting);
        g.handle(GuideEvent::Granted(Permission::Accessibility), 2.0);
        g.tick(2.0 + SUCCESS_HOLD + 1e-6);
        assert_eq!(g.current(), Some(Permission::InputMonitoring));
        // Accessibility turned off while on step two: back to step one.
        assert_eq!(
            g.handle(GuideEvent::Revoked(Permission::Accessibility), 5.0),
            vec![Effect::OpenPane(Pane::Accessibility)]
        );
        assert_eq!(g.current(), Some(Permission::Accessibility));
        // While leaving: back again.
        g.handle(GuideEvent::Granted(Permission::Accessibility), 6.0);
        g.handle(GuideEvent::Granted(Permission::InputMonitoring), 6.0);
        g.tick(6.0 + SUCCESS_HOLD + 1e-6);
        assert!(matches!(g.phase(), Phase::Done { .. }));
        assert_eq!(
            g.handle(GuideEvent::Revoked(Permission::InputMonitoring), 7.3),
            vec![Effect::OpenPane(Pane::InputMonitoring)]
        );
        assert_eq!(g.phase(), Phase::Waiting);
        // After it closed, a revoke is someone else's business (start again).
        g.handle(GuideEvent::Granted(Permission::InputMonitoring), 8.0);
        g.tick(8.0 + SUCCESS_HOLD + 1e-6);
        g.tick(8.0 + SUCCESS_HOLD + LEAVE_SECS + 1e-6);
        assert!(g.finished());
        assert!(g.handle(GuideEvent::Revoked(Permission::Accessibility), 20.0).is_empty());
    }

    #[test]
    fn dismissing_leaves_without_bringing_the_app_forward() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        assert!(g.handle(GuideEvent::Dismiss, 1.0).is_empty());
        assert_eq!(g.phase(), Phase::Done { at: 1.0, dismissed: true });
        assert!(g.handle(GuideEvent::Dismiss, 1.1).is_empty());
        assert_eq!(g.tick(1.0 + LEAVE_SECS + 1e-6), vec![Effect::Closed]);
    }

    #[test]
    fn the_anchor_follows_the_window_and_stays_when_it_goes() {
        let (mut g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
        g.handle(GuideEvent::Window(W), 1.0);
        let moved = Frame::new(400.0, 200.0, 715.0, 600.0);
        g.handle(GuideEvent::Window(SettingsWindow::Visible(moved)), 1.1);
        assert_eq!(g.view().unwrap().settings, Some(moved));
        g.handle(GuideEvent::Window(SettingsWindow::Covered(moved)), 1.2);
        assert!(g.view().unwrap().covered);
        g.handle(GuideEvent::Window(SettingsWindow::NotRunning), 1.3);
        let v = g.view().unwrap();
        assert_eq!((v.settings, v.anchor, v.covered), (None, Some(moved), false));
        // Covered is not gone: no "closed" however long it lasts.
        g.handle(GuideEvent::Window(SettingsWindow::Covered(moved)), 2.0);
        g.tick(60.0);
        assert_eq!(g.phase(), Phase::Waiting);
    }

    #[test]
    fn status_lines_map_to_permissions() {
        let ax = vec!["one-mk needs Accessibility permission to share this Mac's mouse and keyboard. ...".to_owned()];
        let im = vec!["one-mk needs Input Monitoring permission to send this Mac's mouse ...".to_owned()];
        let tap = vec!["one-mk couldn't start watching ... Accessibility and Input Monitoring.".to_owned()];
        assert_eq!(Permission::missing_from_status(&ax), Some(Permission::Accessibility));
        assert_eq!(Permission::missing_from_status(&im), Some(Permission::InputMonitoring));
        assert_eq!(Permission::missing_from_status(&tap), None);
        let mut s = StatusPermissions { lines: ax };
        assert_eq!(s.granted(Permission::Accessibility, 0.0), Some(false));
        assert_eq!(s.granted(Permission::InputMonitoring, 0.0), None, "not checked yet");
        s.lines = im;
        assert_eq!(s.granted(Permission::Accessibility, 0.0), Some(true));
        assert_eq!(s.granted(Permission::InputMonitoring, 0.0), Some(false));
        s.lines.clear();
        assert_eq!(s.granted(Permission::InputMonitoring, 0.0), Some(true));
    }

    #[test]
    fn the_card_is_tall_enough_for_every_state() {
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let (g, _) = Guide::new(&BOTH, grants(&[]), 0.0);
            let v = g.view().unwrap();
            let s = card_size(ctx, &v);
            assert_eq!(s.x, CARD_W);
            assert!(s.y > 140.0 && s.y < 260.0, "{s:?}");
            let (g, _) = Guide::new(&[Permission::InputMonitoring], grants(&[]), 0.0);
            assert!(card_size(ctx, &g.view().unwrap()).y >= 140.0);
        });
        out.textures_delta.clear();
    }

    /// The whole overlay, headless (no separate windows: the card shows in
    /// an embedded window): Settings closes, the card says so, a real click
    /// on "Open Settings again" brings it back.
    #[test]
    fn clicking_open_settings_again_brings_settings_back() {
        struct Never;
        impl PermissionSource for Never {
            fn granted(&mut self, _: Permission, _: f64) -> Option<bool> {
                Some(false)
            }
        }
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let at = Frame::new(150.0, 95.0, 715.0, 560.0);
        let mut o = Overlay {
            windows: Windows::Fake(debug::FakeWindow::new(SettingsWindow::Visible(at), Some(1.0))),
            screens: Some(vec![MAC]),
            ..Overlay::default()
        };
        o.start(&BOTH, Box::new(Never), 0.0);
        let frame = |o: &mut Overlay, t: f64, events: Vec<egui::Event>| {
            let input = egui::RawInput { time: Some(t), events, ..Default::default() };
            let mut out = ctx.run_pass(input, |ctx| o.show(ctx));
            out.textures_delta.clear();
        };
        let mut t = 0.0;
        while t < 3.5 {
            frame(&mut o, t, vec![]);
            t += 0.1;
        }
        let phase = || o.guide.as_ref().unwrap().phase();
        assert_eq!(phase(), Phase::SettingsGone { hidden: false });
        let button = ctx.read_response(Id::unique("guide-retry")).expect("the button is drawn");
        let at_button = button.rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at_button,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut o, t, vec![egui::Event::PointerMoved(at_button)]);
        frame(&mut o, t + 0.05, vec![press(true)]);
        frame(&mut o, t + 0.1, vec![press(false)]);
        assert_eq!(o.guide.as_ref().unwrap().phase(), Phase::Waiting, "retried");
        // Settings comes back a moment later; the card points at it again.
        let mut t = t + 0.2;
        while t < 5.0 {
            frame(&mut o, t, vec![]);
            t += 0.1;
        }
        let v = o.guide.as_ref().unwrap().view().unwrap();
        assert_eq!((v.phase, v.settings), (Phase::Waiting, Some(at)));
        // The close button ends it.
        let close = ctx.read_response(Id::unique("guide-close")).expect("the close button is drawn").rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: close,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut o, t, vec![egui::Event::PointerMoved(close)]);
        frame(&mut o, t + 0.05, vec![press(true)]);
        frame(&mut o, t + 0.1, vec![press(false)]);
        assert!(matches!(o.guide.as_ref().unwrap().phase(), Phase::Done { dismissed: true, .. }));
        frame(&mut o, t + 1.0, vec![]);
        frame(&mut o, t + 1.1, vec![]);
        assert!(!o.active() && o.guide.is_none(), "gone");
    }

    #[test]
    fn grants_told_directly_count_and_stop_ends_it() {
        let mut o = Overlay {
            windows: Windows::Fake(debug::FakeWindow::new(SettingsWindow::NotRunning, None)),
            ..Overlay::default()
        };
        o.start(&BOTH, Box::new(StatusPermissions::default()), 0.0);
        assert!(!o.active(), "an empty status means everything is on: nothing to do");
        let ax = vec!["one-mk needs Accessibility permission to share this Mac's mouse and keyboard.".to_owned()];
        o.start(&BOTH, Box::new(StatusPermissions { lines: ax }), 0.0);
        assert!(o.active());
        o.set_granted(Permission::Accessibility, true, 1.0);
        assert_eq!(o.guide.as_ref().unwrap().phase(), Phase::Granted { at: 1.0 });
        // A later status saying Input Monitoring is missing: Accessibility
        // stays on, Input Monitoring stays off.
        let im = vec!["one-mk needs Input Monitoring permission to send this Mac's mouse and keyboard.".to_owned()];
        o.set_status_input(&im, 1.5);
        let g = o.guide.as_ref().unwrap();
        assert!(g.is_granted(Permission::Accessibility) && !g.is_granted(Permission::InputMonitoring));
        // Then nothing missing: Input Monitoring is on too.
        o.set_status_input(&[], 2.0);
        assert!(o.guide.as_ref().unwrap().is_granted(Permission::InputMonitoring));
        o.stop();
        assert!(!o.active());
    }

    #[test]
    fn the_nudge_rests_and_stops_with_reduced_motion() {
        let (_, moving) = nudge(0.2, 0.0);
        assert!(moving);
        let (_, resting) = nudge(3.3 + 0.9, 0.0);
        assert!(!resting, "rests after three beats");
        let (d, _) = nudge(0.33, 0.0);
        assert!(d > 4.0 && d <= 6.0, "{d}");
    }
}

#[cfg(test)]
mod on_status_tests {
    use super::*;

    fn missing() -> Vec<String> {
        vec!["one-mk needs Accessibility permission to share this Mac's mouse and keyboard. Open System Settings".into()]
    }

    #[test]
    fn the_card_opens_once_per_gap_and_not_where_it_is_not_enabled() {
        let mut o = Overlay::default();
        o.on_status_if(false, &missing(), 0.0);
        assert!(!o.active(), "only on a Mac");
        o.on_status_if(true, &missing(), 1.0);
        assert!(o.active());
        // Closed by the person: later statuses with the same gap don't reopen it.
        o.stop();
        o.on_status_if(true, &missing(), 2.0);
        assert!(!o.active(), "no nagging");
        // Everything granted, then revoked later: offered again.
        o.on_status_if(true, &[], 3.0);
        o.on_status_if(true, &missing(), 4.0);
        assert!(o.active());
    }

    #[test]
    fn closing_after_a_grant_and_revoke_during_the_success_hold_is_respected() {
        let mut o = Overlay::default();
        o.on_status_if(true, &missing(), 0.0);
        o.on_status_if(true, &[], 0.5); // granted; the card is still on screen
        assert!(o.active());
        o.on_status_if(true, &missing(), 0.6); // revoked before it finished
        o.stop(); // the person closes it
        o.on_status_if(true, &missing(), 5.0);
        assert!(!o.active(), "no reopening for the same gap");
    }
}
