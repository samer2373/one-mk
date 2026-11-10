//! The Arrange page (handoff 5.2): where each computer sits (one row each,
//! with the chosen one's side and exact place under it), the saved layouts
//! (profiles), whether the pointer teaches oneMK where computers sit, and
//! layout learning's question, when the daemon has one
//! (`docs/EDGE-PUSH.md`), as a banner. Above the cards, a small plan of the
//! screens' edges: flat rectangles, the edges that touch in the accent.
//! Changes go through `one-mk layout` and `one-mk profile`.

use super::page_ui::{self, Page, ROW, Words};
use super::*;
use crate::widgets::Mark;
use one_mk_wallpaper::Canvas;

const SUGGEST_YES: &str = "Yes, place it";
const SUGGEST_NO: &str = "No";
const SUGGEST_WHY: &str = "You pushed the pointer against that edge twice.";
const SUB: &str = "Exact screen edges, to pixel. Most people only need to drag computers on the desk.";

/// "main screen", "second screen"…: how this computer's screens are named in
/// a sentence (a computer with one screen says "your screen").
fn screen_name(index: usize, screens: usize) -> String {
    if screens <= 1 {
        return "your screen".to_owned();
    }
    match index {
        0 => "main screen".to_owned(),
        1 => "second screen".to_owned(),
        2 => "third screen".to_owned(),
        n => format!("screen {}", n + 1),
    }
}

/// Where a computer sits, in a few words: "Left of main screen". Without
/// the screen it touches, the old words: "On your left".
pub(super) fn sits_words(side: Side, touching: Option<usize>, screens: usize) -> String {
    let before = match side {
        Side::Left => "Left of",
        Side::Right => "Right of",
        Side::Top => "Above",
        Side::Bottom => "Below",
    };
    match touching {
        Some(index) => format!("{before} {}", screen_name(index, screens)),
        None => {
            let words = on_your(side);
            let mut chars = words.chars();
            chars.next().map_or_else(String::new, |first| first.to_uppercase().collect::<String>() + chars.as_str())
        }
    }
}

/// How far two spans overlap (0 when they don't).
fn overlap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

/// Which of this computer's screens (by place in `mine`) the computer whose
/// screens are `theirs` (in this computer's pixels) touches on `side`: the
/// one it shares the longest stretch of edge with.
pub(super) fn touching_screen(mine: &[one_mk_proto::Rect], theirs: &[one_mk_wallpaper::Rect], side: Side) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, screen) in mine.iter().enumerate() {
        let (x, y, w, h) = (f64::from(screen.x), f64::from(screen.y), f64::from(screen.w), f64::from(screen.h));
        for other in theirs {
            let shared = match side {
                Side::Left if (other.x + other.width - x).abs() <= 1.0 => overlap((y, y + h), (other.y, other.y + other.height)),
                Side::Right if (other.x - (x + w)).abs() <= 1.0 => overlap((y, y + h), (other.y, other.y + other.height)),
                Side::Top if (other.y + other.height - y).abs() <= 1.0 => overlap((x, x + w), (other.x, other.x + other.width)),
                Side::Bottom if (other.y - (y + h)).abs() <= 1.0 => overlap((x, x + w), (other.x, other.x + other.width)),
                _ => 0.0,
            };
            if shared > 0.0 && best.is_none_or(|(_, most)| shared > most) {
                best = Some((index, shared));
            }
        }
    }
    best.map(|(index, _)| index)
}

/// The stretches where screens of different computers share an edge, as
/// (from, to) points in the canvas' own pixels: where the pointer crosses.
pub(super) fn touching_edges(canvas: &Canvas) -> Vec<((f64, f64), (f64, f64))> {
    let mut edges = vec![];
    for (i, a) in canvas.monitors.iter().enumerate() {
        for b in &canvas.monitors[i + 1..] {
            if a.computer == b.computer {
                continue;
            }
            let (a, b) = (a.rect, b.rect);
            // `a` left of `b`, or `b` left of `a`; then above, below.
            for (left, right) in [(a, b), (b, a)] {
                if (left.x + left.width - right.x).abs() <= 1.0 {
                    let (from, to) = (left.y.max(right.y), (left.y + left.height).min(right.y + right.height));
                    if to > from {
                        let x = (left.x + left.width + right.x) / 2.0;
                        edges.push(((x, from), (x, to)));
                    }
                }
                if (left.y + left.height - right.y).abs() <= 1.0 {
                    let (from, to) = (left.x.max(right.x), (left.x + left.width).min(right.x + right.width));
                    if to > from {
                        let y = (left.y + left.height + right.y) / 2.0;
                        edges.push(((from, y), (to, y)));
                    }
                }
            }
        }
    }
    edges
}

/// What the warning under a computer says, shortly, when its screens don't
/// touch ours or share an edge with another computer's.
fn short_warning(apart: bool, warnings: &[String]) -> Option<&'static str> {
    if apart || warnings.iter().any(|w| w.contains("does not overlap")) {
        Some("None of its screens touches yours, so the pointer can't cross to it. Drag it next to a monitor.")
    } else if !warnings.is_empty() {
        Some("It shares part of an edge with another computer; the pointer goes to the one placed first.")
    } else {
        None
    }
}

/// The suggestion banner's question and reason laid out `w` wide, its
/// height, and the room its two buttons take.
fn suggestion_text(ui: &Ui, question: &str, w: f32) -> (std::sync::Arc<egui::Galley>, std::sync::Arc<egui::Galley>, f32, f32) {
    let buttons_w = button_width(ui, SUGGEST_YES) + 8.0 + button_width(ui, SUGGEST_NO);
    let text_w = (w - buttons_w - 48.0).max(120.0);
    let q = ui.painter().layout(question.to_owned(), semibold(T_BODY), pal().ink, text_w);
    let d = ui.painter().layout(SUGGEST_WHY.to_owned(), regular(T_META), pal().ink2, text_w);
    let h = (q.size().y + 2.0 + d.size().y).max(34.0) + 24.0;
    (q, d, h, buttons_w)
}

/// The farthest an offset goes on either axis: what `one-mk layout
/// --offset DX,DY` accepts (`one_mk_net::MAX_OFFSET`, what the wire carries).
pub(super) const MAX_OFFSET: i64 = 1 << 24;
const PLACE_WHOLE: &str = "Use whole pixels for X and Y, like -1920 and 0.";
const PLACE_BOUNDS: &str = "X and Y must be within -16777216 and 16777216 pixels.";
/// How near an edge of ours a block let go on the plan snaps to it, in
/// points of the plan.
const PLAN_REACH: f64 = 10.0;
/// A field's height (the handoff's 38).
const FIELD_H: f32 = 38.0;

/// X and Y as typed in Exact place, checked as `one-mk layout --offset`
/// checks them: whole pixels, within ±2^24. Returns the offset or what is
/// wrong, in a sentence.
pub(super) fn parse_place(x: &str, y: &str) -> Result<(i32, i32), &'static str> {
    let num = |s: &str| s.trim().replace('\u{2212}', "-").parse::<i64>().ok();
    let (Some(x), Some(y)) = (num(x), num(y)) else { return Err(PLACE_WHOLE) };
    if x.abs() > MAX_OFFSET || y.abs() > MAX_OFFSET {
        return Err(PLACE_BOUNDS);
    }
    Ok((x as i32, y as i32))
}

/// An arrow key moves a computer on the plan by a pixel (ten with Shift).
pub(super) fn nudge(at: (i32, i32), key: Key, shift: bool) -> Option<(i32, i32)> {
    let step = if shift { 10 } else { 1 };
    let (dx, dy) = match key {
        Key::ArrowLeft => (-step, 0),
        Key::ArrowRight => (step, 0),
        Key::ArrowUp => (0, -step),
        Key::ArrowDown => (0, step),
        _ => return None,
    };
    let m = MAX_OFFSET as i32;
    Some(((at.0 + dx).clamp(-m, m), (at.1 + dy).clamp(-m, m)))
}

/// The side of ours a computer at `offset` is on: the one its screens
/// touch, or else the one it is mostly beyond.
pub(super) fn side_for(mine: &Desktop, theirs: &Desktop, offset: (i32, i32)) -> Side {
    if let Some(side) = Side::ALL.into_iter().find(|s| !one_mk_proto::touching(mine, theirs, *s, offset).is_empty()) {
        return side;
    }
    let (b, r) = (mine.bounds(), theirs.bounds());
    let (rx, ry) = (r.x + offset.0, r.y + offset.1);
    let dx = f64::from(2 * rx + r.w - 2 * b.x - b.w) / f64::from((b.w + r.w).max(1));
    let dy = f64::from(2 * ry + r.h - 2 * b.y - b.h) / f64::from((b.h + r.h).max(1));
    match (dx.abs() >= dy.abs(), dx < 0.0, dy < 0.0) {
        (true, true, _) => Side::Left,
        (true, false, _) => Side::Right,
        (false, _, true) => Side::Top,
        (false, _, false) => Side::Bottom,
    }
}

/// Where a computer whose (0,0) the pointer (or the arrow keys) put at
/// `at` lands, as on the desk (`scene::landing`): beside the nearest edge
/// of ours, so the pointer crosses; or, `apart` (Place freely, or Alt
/// held), exactly there, touching nothing. `reach` is how near an edge
/// snaps to it, in our pixels.
pub(super) fn plan_landing(mine: &Desktop, theirs: &Desktop, at: (i32, i32), reach: f64, apart: bool) -> scene::Landing {
    let edges: Vec<Edge> = Side::ALL.into_iter().flat_map(|s| mine.exposed_edges(s)).collect();
    let b = theirs.bounds();
    let corner = (f64::from(at.0) + f64::from(b.x), f64::from(at.1) + f64::from(b.y));
    let dropped = scene::drop_place(&mine.screens, &edges, b, corner, if apart { 0.0 } else { reach });
    scene::landing(mine, theirs, dropped, apart)
}

/// The plan's scale and where its top left is, in our pixels: kept while
/// a computer is carried, so the plan doesn't grow under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PlanFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Points of the plan per pixel.
    pub scale: f64,
}

impl PlanFrame {
    /// The frame for `bounds`, as large as `inner_w` and `max_h` allow.
    pub fn fit(bounds: one_mk_wallpaper::Rect, inner_w: f32, max_h: f64) -> PlanFrame {
        let scale = (f64::from(inner_w) / bounds.width).min(max_h / bounds.height);
        PlanFrame { x: bounds.x, y: bounds.y, w: bounds.width, h: bounds.height, scale }
    }
    /// How big the plan is on screen.
    pub fn size(&self) -> egui::Vec2 {
        vec2((self.w * self.scale) as f32, (self.h * self.scale) as f32)
    }
    /// Where a pixel of ours is on screen, with the plan's top left at `top_left`.
    pub fn screen(&self, top_left: Pos2, x: f64, y: f64) -> Pos2 {
        top_left + vec2(((x - self.x) * self.scale) as f32, ((y - self.y) * self.scale) as f32)
    }
    /// The pixel of ours under a point on screen.
    pub fn px(&self, top_left: Pos2, p: Pos2) -> (f64, f64) {
        (self.x + f64::from(p.x - top_left.x) / self.scale, self.y + f64::from(p.y - top_left.y) / self.scale)
    }
}

/// Where a carried computer's (0,0) is with the pointer at `pointer`
/// (in our pixels), held by `grab` (the pointer's distance from it).
pub(super) fn carried_to(pointer: (f64, f64), grab: (f64, f64)) -> (i32, i32) {
    let m = MAX_OFFSET as f64;
    (((pointer.0 - grab.0).round()).clamp(-m, m) as i32, ((pointer.1 - grab.1).round()).clamp(-m, m) as i32)
}

/// Where computer `id`'s (0,0) is on `canvas`, in our pixels (its place,
/// or where it is drawn when it has none).
pub(super) fn shown_offset(canvas: &Canvas, id: &str) -> Option<(i32, i32)> {
    let m = canvas.monitors.iter().find(|m| m.computer == id)?;
    let local = m.local?;
    Some(((m.rect.x - local.x).round() as i32, (m.rect.y - local.y).round() as i32))
}

/// `at` moved as little as needed for screens spanning `span` (in their
/// pixels) to stay between `lo` and `hi` (ours): the plan's room.
pub(super) fn keep_within(at: (i32, i32), span: one_mk_proto::Rect, lo: (f64, f64), hi: (f64, f64)) -> (i32, i32) {
    let axis = |at: i32, from: i32, len: i32, lo: f64, hi: f64| {
        let (min, max) = ((lo - f64::from(from)).ceil(), (hi - f64::from(from + len)).floor());
        if min > max { at } else { (f64::from(at)).clamp(min, max) as i32 }
    };
    (axis(at.0, span.x, span.w, lo.0, hi.0), axis(at.1, span.y, span.h, lo.1, hi.1))
}

/// A computer being placed on the plan.
pub(super) struct PlanDrag {
    pub id: String,
    frame: PlanFrame,
    /// From its (0,0) to the pointer, in our pixels (none: the keyboard).
    grab: Option<(f64, f64)>,
    /// Where the pointer or the arrow keys put its (0,0), before snapping.
    at: (i32, i32),
    /// Where it lands if let go now.
    landing: scene::Landing,
}

impl App {
    /// The layout warnings the daemon has about `name`.
    fn layout_warnings(status: &Status, name: &str) -> Vec<String> {
        status.layout.warnings.iter().filter(|w| w.contains(name)).cloned().collect()
    }

    /// Layout learning's question as a banner at `y`, when the daemon asks
    /// one and it wasn't answered here: "Yes, place it" places the computer
    /// there (`one-mk layout`), "No" hides the question (`one-mk edge
    /// --dismiss`); either way it goes at once. Returns its height.
    /// How tall the suggestion banner is `w` wide, when there is one to show
    /// (the Desk makes room for it above the computers).
    pub(super) fn suggestion_h(&self, ui: &Ui, w: f32) -> Option<f32> {
        let g = self.running()?.suggestion.as_ref()?;
        if self.answered.as_ref() == Some(&(g.id.clone(), g.side)) {
            return None;
        }
        Some(suggestion_text(ui, &one_mk_ctl::status::is_it_there(&g.name, g.side), w).2)
    }

    /// Whether the Desk asks layout learning's question in its toolbar's
    /// row (a narrow window, where a banner above would leave the computers
    /// too little room) rather than in a banner above it.
    pub(super) fn suggestion_in_row(&self, ui: &Ui) -> bool {
        ui.ctx().content_rect().width() < desk_panel::STACK_BELOW && self.desk_views_shown()
    }

    /// The question in one line, `r` (the toolbar row's free part): the
    /// banner's look, its words, Yes and No; why it asks, on hover.
    pub(super) fn suggestion_row(&mut self, ui: &mut Ui, r: Rect) {
        let Some(g) = self.running().and_then(|s| s.suggestion.clone()) else {
            self.answered = None;
            return;
        };
        if self.answered.as_ref() == Some(&(g.id.clone(), g.side)) || r.width() < 220.0 {
            return;
        }
        let question = one_mk_ctl::status::is_it_there(&g.name, g.side);
        page_ui::banner_frame(ui, r, false);
        let (yes_w, no_w) = (button_width(ui, SUGGEST_YES), button_width(ui, SUGGEST_NO));
        let no_r = Rect::from_min_size(pos2(r.right() - 2.0 - no_w, r.top() + 2.0), vec2(no_w, r.height() - 4.0));
        let yes_r = Rect::from_min_size(pos2(no_r.left() - 6.0 - yes_w, r.top() + 2.0), vec2(yes_w, r.height() - 4.0));
        let room = yes_r.left() - r.left() - 24.0;
        let p = ui.painter();
        let shown = fit(p, &question, &semibold(T_BODY), room);
        let text = p.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, shown, semibold(T_BODY), pal().ink);
        ui.interact(text, Id::unique(("suggestion-why", &g.id)), Sense::hover()).on_hover_text(format!("{question} {SUGGEST_WHY}"));
        a11y_text(ui, Id::unique(("suggestion", &g.id)), r, Role::Status, &format!("{question} {SUGGEST_WHY}"), Some(Live::Polite));
        let place = format!("Yes: place {} {}", g.name, on_your(g.side));
        let keep = format!("No: {} stays where it is", g.name);
        if button(ui, yes_r, Id::unique(("suggestion-yes", &g.id)), SUGGEST_YES, Look::Primary, true, Some(&place)).clicked() {
            self.answered = Some((g.id.clone(), g.side));
            self.choose_side(&g.id, g.side, false);
        }
        if button(ui, no_r, Id::unique(("suggestion-no", &g.id)), SUGGEST_NO, Look::Quiet, true, Some(&keep)).clicked() {
            self.answered = Some((g.id.clone(), g.side));
            self.link.set("the layout suggestion", &["edge", "--dismiss"]);
        }
    }

    pub(super) fn suggestion_banner(&mut self, ui: &mut Ui, x: f32, y: f32, w: f32) -> Option<f32> {
        let Some(g) = self.running().and_then(|s| s.suggestion.clone()) else {
            self.answered = None; // asked again later, it shows again
            return None;
        };
        if self.answered.as_ref() == Some(&(g.id.clone(), g.side)) {
            return None;
        }
        let question = one_mk_ctl::status::is_it_there(&g.name, g.side);
        let (q, d, h, buttons_w) = suggestion_text(ui, &question, w);
        let rect = Rect::from_min_size(pos2(x, y), vec2(w, h));
        page_ui::banner_frame(ui, rect, false);
        let qh = q.size().y;
        let text_top = rect.center().y - (qh + 2.0 + d.size().y) / 2.0;
        let p = ui.painter();
        p.galley(pos2(rect.left() + 16.0, text_top), q, pal().ink);
        p.galley(pos2(rect.left() + 16.0, text_top + qh + 2.0), d, pal().ink2);
        a11y_text(ui, Id::unique(("suggestion", &g.id)), rect, Role::Status, &format!("{question} {SUGGEST_WHY}"), Some(Live::Polite));
        let row_y = rect.center().y - 15.0;
        let yes_r = Rect::from_min_size(pos2(rect.right() - 16.0 - buttons_w, row_y), vec2(button_width(ui, SUGGEST_YES), 30.0));
        let no_r = Rect::from_min_size(pos2(yes_r.right() + 8.0, row_y), vec2(button_width(ui, SUGGEST_NO), 30.0));
        let place = format!("Yes: place {} {}", g.name, on_your(g.side));
        let keep = format!("No: {} stays where it is", g.name);
        if button(ui, yes_r, Id::unique(("suggestion-yes", &g.id)), SUGGEST_YES, Look::Primary, true, Some(&place)).clicked() {
            self.answered = Some((g.id.clone(), g.side));
            self.choose_side(&g.id, g.side, false);
        }
        if button(ui, no_r, Id::unique(("suggestion-no", &g.id)), SUGGEST_NO, Look::Quiet, true, Some(&keep)).clicked() {
            self.answered = Some((g.id.clone(), g.side));
            self.link.set("the layout suggestion", &["edge", "--dismiss"]);
        }
        Some(h)
    }

    pub(super) fn arrange_page(&mut self, ui: &mut Ui, area: Rect) {
        let Some(status) = self.running().cloned() else { return };
        // The computer whose side and place are edited: the chosen one, or
        // the first (this computer has none to place).
        let selected = self.selected_id();
        let edit = status.peers.iter().find(|p| Some(&p.id) == selected.as_ref()).or(status.peers.first()).map(|p| p.id.clone());
        let canvas = one_mk_ctl::wallpaper::canvas(&status);
        page_ui::scroll("arrange-scroll").show_viewport(ui, |ui, _| {
            let origin = ui.cursor().min;
            let mut page = Page::begin(ui, origin, area.width(), "Arrange", SUB);
            // Layout learning's question first: it is waiting for an answer.
            let y = page.below() - 4.0;
            if let Some(h) = self.suggestion_banner(ui, page.x, y, page.w) {
                page.skip_to(y + h + 16.0);
            }
            self.plan_card(ui, &mut page, &status, selected.as_deref());

            // Where each computer sits, and the chosen one's place.
            let mut card = page.card(ui, 0, "Where each computer sits");
            self.sits_rows(ui, &mut card, &status, &canvas, selected.as_deref());
            page.finish(ui, card);
            if let Some(peer) = edit.as_ref().and_then(|id| status.peers.iter().find(|p| &p.id == id)) {
                let mut card = page.card(ui, 0, &peer.name);
                // Connected or not, as a word beside the name (a shape too).
                let (word, mark) = if peer.online { ("Connected", Mark::Ok) } else { ("Offline", Mark::Off) };
                let at = card.label_right();
                let p = ui.painter();
                let galley = p.layout_no_wrap(word.to_owned(), regular(T_META), pal().ink2);
                p.galley(pos2(at.x - galley.size().x, at.y - galley.size().y / 2.0), galley.clone(), pal().ink2);
                widgets::status_mark(p, pos2(at.x - galley.size().x - 10.0, at.y), 7.0, mark);
                self.place_rows(ui, &mut card, &status, peer);
                page.finish(ui, card);
            }

            // Saved layouts, and layout learning.
            let mut card = page.card(ui, 1, "Layouts");
            self.profile_rows(ui, &mut card, &status);
            page.finish(ui, card);
            ui.allocate_space(vec2(area.width(), page.height()));
        });
    }

    /// What Exact place holds for `id` while it is being typed: the offset,
    /// or what is wrong with it; `None` when nothing was typed.
    fn place_draft(&self, id: &str) -> Option<Result<(i32, i32), &'static str>> {
        let (x, y) = (self.drafts.get(&format!("place-x:{id}")), self.drafts.get(&format!("place-y:{id}")));
        if x.is_none() && y.is_none() {
            return None;
        }
        Some(parse_place(x.map_or("", String::as_str), y.map_or("", String::as_str)))
    }

    /// The status as the plan shows it: each computer where it is being
    /// placed (carried, moved by the arrow keys, typed in Exact place), or
    /// where it was chosen here, or where the daemon says it is.
    fn plan_status(&self, status: &Status) -> Status {
        let mut shown = status.clone();
        shown.offsets.clear();
        let mine = Desktop::new(status.screens.clone());
        for peer in &mut shown.peers {
            let (mut side, mut offset) = self.place_shown(&peer.id, (peer.side, status.offset_of(&peer.id)));
            if let Some(d) = self.plan.as_ref().filter(|d| d.id == peer.id) {
                (side, offset) = match d.landing {
                    scene::Landing::Beside(s, o) | scene::Landing::Apart(s, o) => (s, Some(o)),
                    scene::Landing::Along(s) => (s, None),
                };
            } else if let Some(Ok(o)) = self.place_draft(&peer.id)
                && !status.screens.is_empty()
                && !peer.screens.is_empty()
            {
                (side, offset) = (side_for(&mine, &Desktop::new(peer.screens.clone()), o), Some(o));
            }
            peer.side = side;
            if let Some(o) = offset {
                shown.offsets.push((peer.id.clone(), o));
            }
        }
        shown
    }

    /// A computer let go on the plan (or placed with Enter) goes where it
    /// landed, through the same `one-mk layout` as a drag on the desk.
    fn plan_let_go(&mut self, d: PlanDrag) {
        match d.landing {
            scene::Landing::Beside(side, o) | scene::Landing::Apart(side, o) => self.choose_place(&d.id, side, Some(o), false),
            scene::Landing::Along(side) => self.choose_side(&d.id, side, false),
        }
    }

    /// The plan of screens: one flat rectangle per screen, named by its
    /// computer, the edges where two computers touch in the accent. A click
    /// chooses a computer; dragging one (or the arrow keys, Enter to place,
    /// Esc to cancel) places it: beside the nearest edge of ours, as on the
    /// desk, or anywhere with Place freely (or Alt held), apart.
    fn plan_card(&mut self, ui: &mut Ui, page: &mut Page, status: &Status, selected: Option<&str>) {
        let before = one_mk_ctl::wallpaper::canvas(&self.plan_status(status));
        let Some(bounds) = before.bounds().filter(|_| before.monitors.len() > 1) else {
            self.plan = None;
            return;
        };
        let mut card = page.wide(ui, "");
        let inner_w = page.w - 48.0;
        let frame = self.plan.as_ref().map_or_else(|| PlanFrame::fit(bounds, inner_w, 150.0), |d| d.frame);
        let size = frame.size();
        let area = card.row(ui, size.y + 24.0 + 40.0);
        let top_left = pos2(area.center().x - size.x / 2.0, area.top() + 24.0);
        let mine = Desktop::new(status.screens.clone());
        let movable = |id: &str| -> Option<Desktop> {
            status.peers.iter().find(|p| p.id == id).filter(|p| !p.screens.is_empty() && !status.screens.is_empty()).map(|p| Desktop::new(p.screens.clone()))
        };
        let reach = PLAN_REACH / frame.scale;
        let (alt, shift, escape, enter) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift, i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
        let apart = self.plan_free || alt;
        let block = |r: one_mk_wallpaper::Rect| Rect::from_min_max(frame.screen(top_left, r.x, r.y), frame.screen(top_left, r.x + r.width, r.y + r.height)).shrink(1.5);

        // Each block where it is drawn now: what the pointer and keys do.
        let mut responses = Vec::with_capacity(before.monitors.len());
        let (mut pick, mut start, mut moved_to, mut let_go, mut nudged, mut focused) = (None, None, None, false, None, None);
        for monitor in &before.monitors {
            let can_move = monitor.computer != status.id && movable(&monitor.computer).is_some();
            let id = Id::unique(("plan-screen", &monitor.computer, &monitor.name));
            let resp = ui.interact(block(monitor.rect), id, widgets::sense(ui, if can_move { Sense::click_and_drag() } else { Sense::click() }));
            if resp.clicked() {
                pick = Some(monitor.computer.clone());
            }
            if can_move {
                if resp.drag_started() {
                    start = resp.interact_pointer_pos().map(|p| (monitor.computer.clone(), p));
                }
                if resp.dragged() {
                    moved_to = resp.interact_pointer_pos();
                }
                let_go |= resp.drag_stopped();
                if resp.has_focus() {
                    // The arrow keys and Esc are the block's while it has the keyboard.
                    ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true }));
                    focused = Some(monitor.computer.clone());
                    for key in [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown] {
                        if ui.input(|i| i.key_pressed(key)) {
                            nudged = Some((monitor.computer.clone(), key));
                        }
                    }
                }
            }
            responses.push(resp);
        }
        if let Some(id) = pick {
            self.select(if id == status.id { "me".to_owned() } else { id });
        }
        // Picked up: held where the pointer took it.
        if let Some((id, p)) = start
            && let (Some(theirs), Some(at)) = (movable(&id), shown_offset(&before, &id))
        {
            let px = frame.px(top_left, p);
            let grab = (px.0 - f64::from(at.0), px.1 - f64::from(at.1));
            self.select(id.clone());
            self.plan = Some(PlanDrag { id, frame, grab: Some(grab), at, landing: plan_landing(&mine, &theirs, at, reach, apart) });
        }
        // An arrow key: a pixel (ten with Shift) from where it is shown.
        if let Some((id, key)) = nudged
            && movable(&id).is_some()
        {
            let from = match &self.plan {
                Some(d) if d.id == id => Some(d.at),
                _ => shown_offset(&before, &id),
            };
            if let Some(at) = from.and_then(|at| nudge(at, key, shift)) {
                let landing = scene::Landing::Along(Side::Left);
                self.plan = Some(PlanDrag { id, frame, grab: None, at, landing });
            }
        }
        if let Some(d) = self.plan.as_mut() {
            if let (Some(p), Some(grab)) = (moved_to, d.grab) {
                d.at = carried_to(frame.px(top_left, p), grab);
            }
            // It stays on the plan (above the line under it), not under the cards.
            if let Some(theirs) = movable(&d.id) {
                let room = Rect::from_min_max(area.min + vec2(0.0, 4.0), area.max - vec2(0.0, 30.0));
                d.at = keep_within(d.at, theirs.bounds(), frame.px(top_left, room.min), frame.px(top_left, room.max));
            }
            match movable(&d.id) {
                Some(theirs) => d.landing = plan_landing(&mine, &theirs, d.at, reach, apart),
                None => self.plan = None,
            }
        }
        let keyboard = self.plan.as_ref().is_some_and(|d| d.grab.is_none());
        let id_of = self.plan.as_ref().map(|d| d.id.clone());
        if self.plan.is_some() && (escape || (keyboard && focused != id_of)) {
            // Esc, or the keyboard went elsewhere: it stays where it was.
            self.plan = None;
        } else if (let_go && !keyboard) || (enter && keyboard && focused == id_of) {
            if let Some(d) = self.plan.take() {
                self.plan_let_go(d);
            }
        } else if let_go {
            self.plan = None;
        }

        // Painted where it is now.
        let canvas = one_mk_ctl::wallpaper::canvas(&self.plan_status(status));
        let carried = self.plan.as_ref().map(|d| d.id.clone());
        // Placed elsewhere (a typed place, a layout chosen), a computer
        // glides there; carried, it follows the pointer exactly.
        let mut glide: HashMap<String, egui::Vec2> = HashMap::new();
        for monitor in &canvas.monitors {
            if glide.contains_key(&monitor.computer) {
                continue;
            }
            let Some(o) = shown_offset(&canvas, &monitor.computer) else { continue };
            let secs = if carried.as_deref() == Some(monitor.computer.as_str()) { 0.0 } else { motion::space(0.18) };
            let gx = tween(ui.ctx(), Id::unique(("plan-glide", &monitor.computer, 0)), o.0 as f32, secs, motion::out_cubic);
            let gy = tween(ui.ctx(), Id::unique(("plan-glide", &monitor.computer, 1)), o.1 as f32, secs, motion::out_cubic);
            glide.insert(monitor.computer.clone(), vec2((f64::from(gx - o.0 as f32) * frame.scale) as f32, (f64::from(gy - o.1 as f32) * frame.scale) as f32));
        }
        let p = ui.painter_at(area.expand(2.0));
        let mut named = std::collections::BTreeSet::new();
        for (k, monitor) in canvas.monitors.iter().enumerate() {
            let rect = block(monitor.rect).translate(glide.get(&monitor.computer).copied().unwrap_or_default());
            let Some(response) = responses.get(k) else { continue };
            let is_me = monitor.computer == status.id;
            let chosen = Some(monitor.computer.as_str()) == selected || (selected == Some("me") && is_me) || carried.as_deref() == Some(monitor.computer.as_str());
            let fill = if chosen { pal().accent_soft } else { mix(pal().surface2, pal().surface2_hover, f32::from(response.hovered())) };
            let fill = if monitor.online { fill } else { fill.gamma_multiply(0.55) };
            p.rect_filled(rect, CornerRadius::same(5), fill);
            let name = if is_me { status.name.clone() } else { status.peers.iter().find(|peer| peer.id == monitor.computer).map_or_else(|| monitor.name.clone(), |peer| peer.name.clone()) };
            if named.insert(monitor.computer.clone()) {
                let label = if monitor.online { name.clone() } else { format!("{name} · Offline") };
                let colour = if chosen { pal().accent_text } else if monitor.online { pal().ink2 } else { pal().ink3 };
                let shown = fit(&p, &label, &regular(T_META), rect.width() - 10.0);
                if rect.width() > 36.0 && rect.height() > 18.0 {
                    p.text(rect.center(), Align2::CENTER_CENTER, shown, regular(T_META), colour);
                }
            }
            widgets::focus_ring(ui, response, rect, 5);
            let can_move = !is_me && movable(&monitor.computer).is_some();
            let words = if can_move {
                format!("{name}: drag it to place it, or use the arrow keys (Shift moves 10 pixels), Enter to place it, Esc to cancel")
            } else {
                format!("Choose {name} to place it")
            };
            widgets::a11y(response, Role::Button, &words, Some(chosen));
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            widgets::targets::note(&format!("plan {name}"), rect);
            if response.hovered() && carried.is_none() {
                ui.ctx().set_cursor_icon(if can_move { egui::CursorIcon::Grab } else { egui::CursorIcon::PointingHand });
            }
        }
        if self.plan.as_ref().is_some_and(|d| d.grab.is_some()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        // Carried with snapping: where the pointer has it, as an outline;
        // the block shows where it lands.
        if let Some(d) = self.plan.as_ref().filter(|d| d.grab.is_some())
            && let scene::Landing::Beside(_, o) = d.landing
            && o != d.at
            && let Some(peer) = status.peers.iter().find(|peer| peer.id == d.id)
        {
            for s in &peer.screens {
                let (x, y) = (f64::from(s.x + d.at.0), f64::from(s.y + d.at.1));
                let r = Rect::from_min_max(frame.screen(top_left, x, y), frame.screen(top_left, x + f64::from(s.w), y + f64::from(s.h))).shrink(1.5);
                p.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, pal().line2), StrokeKind::Inside);
            }
        }
        for (from, to) in touching_edges(&canvas) {
            p.line_segment([frame.screen(top_left, from.0, from.1), frame.screen(top_left, to.0, to.1)], Stroke::new(2.0, pal().accent));
        }
        a11y_text(ui, Id::unique("arrange-plan"), area, Role::Image, "Plan of every computer's screens; the edges where the pointer crosses are marked", None);

        // Under the plan: how to place, or (placed apart) the warning.
        let line_y = area.bottom() - 20.0;
        let landing_apart = self.plan.as_ref().is_some_and(|d| matches!(d.landing, scene::Landing::Apart(..)));
        let typed_apart = carried.is_none()
            && status.peers.iter().any(|peer| match self.place_draft(&peer.id) {
                Some(Ok(o)) if !peer.screens.is_empty() && !status.screens.is_empty() => {
                    let theirs = Desktop::new(peer.screens.clone());
                    one_mk_proto::touching(&mine, &theirs, side_for(&mine, &theirs, o), o).is_empty()
                }
                _ => false,
            });
        if landing_apart || typed_apart {
            let text = short_warning(true, &[]).unwrap_or_default();
            let shown = fit(&p, text, &regular(T_META), area.width() - 24.0);
            let w = text_width(ui, &shown, regular(T_META));
            let x = area.center().x - (w + 16.0) / 2.0;
            widgets::status_mark(&p, pos2(x + 4.0, line_y), 8.0, Mark::Warn);
            p.text(pos2(x + 16.0, line_y), Align2::LEFT_CENTER, shown, regular(T_META), pal().ink2);
            a11y_text(ui, Id::unique("plan-warning"), Rect::from_center_size(pos2(area.center().x, line_y), vec2(area.width(), 18.0)), Role::Status, text, Some(Live::Polite));
        } else {
            let hint = "Drag a computer to place it. Arrow keys move the chosen one; Enter places it.";
            p.text(pos2(area.center().x, line_y), Align2::CENTER_CENTER, fit(&p, hint, &regular(T_META), area.width() - 24.0), regular(T_META), pal().ink3);
        }

        // Place freely: let go anywhere, even apart (Alt does it for one drag).
        let row = card.row(ui, ROW);
        let caption = format!("Let go anywhere, even apart (or hold {})", if cfg!(target_os = "macos") { "Option" } else { "Alt" });
        if page_ui::toggle_row(ui, row, Id::unique("plan-free"), &Words::new("Place freely").caption(&caption), None, self.plan_free, true).clicked() {
            self.plan_free = !self.plan_free;
        }
        page.finish(ui, card);
    }

    /// One row per computer: this one first, then each paired one and where
    /// it sits (◆ and the existing sentence when its screens don't touch).
    fn sits_rows(&mut self, ui: &mut Ui, card: &mut page_ui::Card, status: &Status, canvas: &Canvas, selected: Option<&str>) {
        // This computer: its screens in a sentence.
        let b = if status.screens.is_empty() { one_mk_proto::Rect::new(0, 0, 0, 0) } else { one_mk_proto::Desktop::new(status.screens.clone()).bounds() };
        let me_text = match status.screens.len() {
            0 => "Its screen size isn't known yet: oneMK learns it once input sharing has started.".to_owned(),
            1 => format!("One monitor, {} × {}.", b.w, b.h),
            n => format!("{n} monitors, {} × {} in all.", b.w, b.h),
        };
        let row_w = card.w - 2.0 * page_ui::CARD_PAD;
        let cap_h = page_ui::text_h(ui, &me_text, row_w - 110.0, regular(T_META));
        let row = card.row(ui, ROW.max(27.0 + cap_h + 11.0));
        self.sits_row(ui, row, status.name.clone(), "me".to_owned(), "This computer", None, Some(&me_text), selected == Some("me"), None);
        if status.peers.is_empty() {
            let row = card.row(ui, ROW);
            ui.painter().text(row.left_center(), Align2::LEFT_CENTER, fit(ui.painter(), "Pair a computer to place it around this one.", &regular(T_BODY), row.width()), regular(T_BODY), pal().ink2);
            return;
        }
        for peer in &status.peers {
            let reported = (peer.side, status.offset_of(&peer.id));
            let shown = self.place_shown(&peer.id, reported);
            let apart = self.apart.contains(&peer.id);
            let warnings = Self::layout_warnings(status, &peer.name);
            let warning = short_warning(apart, &warnings);
            let touching = (shown == reported && !apart)
                .then(|| {
                    let theirs: Vec<_> = canvas.monitors.iter().filter(|m| m.computer == peer.id).map(|m| m.rect).collect();
                    touching_screen(&status.screens, &theirs, shown.0)
                })
                .flatten();
            let value = if apart || warnings.iter().any(|w| w.contains("does not overlap")) { "Doesn't touch your screens".to_owned() } else { sits_words(shown.0, touching, status.screens.len()) };
            let value_w = text_width(ui, &value, regular(T_BODY)) + 16.0;
            let cap_w = row_w - page_ui::MARK_INDENT - value_w;
            let h = match warning {
                Some(text) => ROW.max(27.0 + page_ui::text_h(ui, text, cap_w, regular(T_META)) + 11.0),
                None => ROW,
            };
            let row = card.row(ui, h);
            self.sits_row(ui, row, peer.name.clone(), peer.id.clone(), &value, warning.map(|_| Mark::Warn), warning, selected == Some(peer.id.as_str()), Some(peer.online));
        }
    }

    /// A computer's row: name, its place at the right, a click chooses it.
    #[allow(clippy::too_many_arguments)]
    fn sits_row(&mut self, ui: &mut Ui, row: Rect, name: String, id: String, value: &str, mark: Option<Mark>, caption: Option<&str>, chosen: bool, online: Option<bool>) {
        let hit = row.expand2(vec2(page_ui::CARD_PAD - 6.0, -3.0));
        let response = ui.interact(hit, Id::unique(("sits-row", &id)), widgets::sense(ui, Sense::click()));
        let p = ui.painter();
        if chosen {
            p.rect_filled(hit, CornerRadius::same(10), pal().accent_soft);
        } else if response.hovered() {
            p.rect_filled(hit, CornerRadius::same(10), pal().hover);
        }
        let value_w = if value.is_empty() { 0.0 } else { text_width(ui, value, regular(T_BODY)) + 16.0 };
        let mut words = Words::new(&name);
        if caption.is_some() {
            words = words.raised();
        }
        if let Some(mark) = mark {
            words = words.mark(mark);
        }
        page_ui::paint_words(ui, row, &words, value_w);
        if let Some(caption) = caption {
            let (x, room) = page_ui::words_box(row, mark.is_some(), value_w);
            para(ui, pos2(x, row.top() + 27.0), room, caption, regular(T_META), pal().ink2);
        }
        let colour = if chosen { pal().accent_text } else { pal().ink2 };
        ui.painter().text(row.right_center(), Align2::RIGHT_CENTER, fit(ui.painter(), value, &regular(T_BODY), row.width() * 0.6), regular(T_BODY), colour);
        widgets::focus_ring(ui, &response, hit, 10);
        let state = match online { Some(false) => ", offline", _ => "" };
        let sentence = match caption { Some(c) => format!("{name}: {value}{state}. {c}"), None => format!("{name}: {value}{state}") };
        widgets::a11y(&response, Role::Button, &sentence, Some(chosen));
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            self.select(id);
        }
    }

    /// The chosen computer: its side as four choices, what that means, and
    /// its exact place with the ways back to anywhere along the side.
    fn place_rows(&mut self, ui: &mut Ui, card: &mut page_ui::Card, status: &Status, peer: &PeerStatus) {
        let (side, offset) = self.place_shown(&peer.id, (peer.side, status.offset_of(&peer.id)));
        let row = card.row(ui, ROW);
        page_ui::paint_words(ui, row, &Words::new("Sits"), 280.0);
        let track = Rect::from_min_size(pos2(row.right() - 280.0, row.center().y - 15.0), vec2(280.0, 30.0));
        let labels: Vec<&str> = PLACES.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique(("layout-position", &peer.id)),
            track,
            cells: Pills::even(track, 4, 2.0),
            labels: &labels,
            names: PLACES.iter().map(|(_, l)| format!("Position of {}: {l}", peer.name)).collect(),
            chosen: PLACES.iter().position(|(s, _)| *s == side),
            enabled: vec![true; 4],
            role: Role::RadioButton,
            font: T_BODY,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.choose_side(&peer.id, PLACES[k].0, true);
        }
        let apart = self.apart.contains(&peer.id);
        let sentence = match offset {
            None => format!("{} is {}: the pointer crosses anywhere along that edge, scaled to fit.", peer.name, on_your(side)),
            Some(_) if apart => format!("{} is {}, placed apart from your monitors.", peer.name, on_your(side)),
            Some(_) => format!("{} is {}, beside your monitors: the pointer crosses where its screens touch yours.", peer.name, on_your(side)),
        };
        let inner = card.w - 2.0 * page_ui::CARD_PAD;
        let h = page_ui::text_h(ui, &sentence, inner, regular(T_BODY));
        let row = card.row(ui, h + 24.0);
        para(ui, row.min + vec2(0.0, 12.0), inner, &sentence, regular(T_BODY), pal().ink2);
        a11y_text(ui, Id::unique(("arrange-sentence", &peer.id)), row, Role::Label, &sentence, Some(Live::Polite));

        // Exact place: where its top left corner sits in our pixels, as X
        // and Y to type (checked as `one-mk layout --offset` checks them;
        // the plan shows it there while it is typed).
        let text = match offset {
            Some((x, y)) => format!("Its top-left corner sits at {x}, {y} of your screens."),
            None if peer.screens.is_empty() => "Its monitors are known once it is connected; then drag it beside one of yours.".to_owned(),
            None => format!("Anywhere along your {} edge. Drag it next to a monitor to place it exactly.", edge_word(side)),
        };
        let h = page_ui::text_h(ui, &text, inner, regular(T_META));
        let (kx, ky) = (format!("place-x:{}", peer.id), format!("place-y:{}", peer.id));
        let error = self.place_draft(&peer.id).and_then(Result::err);
        let buttons = offset.is_some();
        let fields_h = 12.0 + FIELD_H + if error.is_some() { 6.0 + 16.0 } else { 0.0 };
        let row = card.row(ui, 14.0 + 18.0 + 4.0 + h + fields_h + if buttons { 12.0 + 30.0 } else { 0.0 } + 14.0);
        ui.painter().text(row.left_top() + vec2(0.0, 23.0), Align2::LEFT_CENTER, "Exact place", regular(T_BODY), pal().ink);
        let ch = para(ui, row.min + vec2(0.0, 36.0), inner, &text, regular(T_META), pal().ink2);
        a11y_text(ui, Id::unique(("exact-place", &peer.id)), Rect::from_min_size(row.min + vec2(0.0, 14.0), vec2(inner, 22.0 + ch)), Role::Label, &text, Some(Live::Polite));

        // X [   ]  Y [   ]  Place
        let can = !status.screens.is_empty() && !peer.screens.is_empty();
        let (cx, cy) = offset.map_or((String::new(), String::new()), |(x, y)| (x.to_string(), y.to_string()));
        let hint = shown_offset(&one_mk_ctl::wallpaper::canvas(status), &peer.id).map_or(("X".to_owned(), "Y".to_owned()), |(x, y)| (x.to_string(), y.to_string()));
        let mut xs = self.drafts.get(&kx).cloned().unwrap_or_else(|| cx.clone());
        let mut ys = self.drafts.get(&ky).cloned().unwrap_or_else(|| cy.clone());
        let fy = row.top() + 36.0 + ch + 12.0;
        let field_w = ((inner - 2.0 * 18.0 - 12.0 - 14.0 - button_width(ui, "Place")) / 2.0).clamp(80.0, 132.0);
        let p = ui.painter().clone();
        p.text(pos2(row.left(), fy + FIELD_H / 2.0), Align2::LEFT_CENTER, "X", mono(T_META), pal().ink2);
        let xr = Rect::from_min_size(pos2(row.left() + 18.0, fy), vec2(field_w, FIELD_H));
        let rx = widgets::field(ui, xr, Id::unique(("field", &kx)), &mut xs, &hint.0, &format!("X: how far right {}'s top-left corner sits from yours, in pixels", peer.name), can, true);
        p.text(pos2(xr.right() + 14.0, fy + FIELD_H / 2.0), Align2::LEFT_CENTER, "Y", mono(T_META), pal().ink2);
        let yr = Rect::from_min_size(pos2(xr.right() + 14.0 + 18.0, fy), vec2(field_w, FIELD_H));
        let ry = widgets::field(ui, yr, Id::unique(("field", &ky)), &mut ys, &hint.1, &format!("Y: how far down {}'s top-left corner sits from yours, in pixels", peer.name), can, true);
        if xs == cx && ys == cy {
            self.drafts.remove(&kx);
            self.drafts.remove(&ky);
        } else {
            self.drafts.insert(kx.clone(), xs.clone());
            self.drafts.insert(ky.clone(), ys.clone());
        }
        let left = rx.lost_focus() || ry.lost_focus();
        let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if left && escape {
            // Esc: back to where it is.
            self.drafts.remove(&kx);
            self.drafts.remove(&ky);
        }
        let typed = self.place_draft(&peer.id).and_then(Result::ok).filter(|o| Some(*o) != offset);
        let ready = can && typed.is_some();
        let place_r = Rect::from_min_size(pos2(yr.right() + 12.0, fy + (FIELD_H - 30.0) / 2.0), vec2(button_width(ui, "Place"), 30.0));
        let clicked = button(ui, place_r, Id::unique(("place-xy", &peer.id)), "Place", Look::Quiet, ready, Some(&format!("Place {} at this X and Y", peer.name))).clicked();
        if let Some(o) = typed.filter(|_| ready && (clicked || (left && enter))) {
            self.drafts.remove(&kx);
            self.drafts.remove(&ky);
            let side = side_for(&Desktop::new(status.screens.clone()), &Desktop::new(peer.screens.clone()), o);
            self.choose_place(&peer.id, side, Some(o), false);
        }
        let mut y = fy + FIELD_H;
        if let Some(e) = error {
            p.text(pos2(row.left(), y + 6.0 + 8.0), Align2::LEFT_CENTER, fit(&p, e, &regular(T_META), inner), regular(T_META), pal().danger);
            a11y_text(ui, Id::unique(("exact-place-error", &peer.id)), Rect::from_min_size(pos2(row.left(), y + 6.0), vec2(inner, 16.0)), Role::Status, e, Some(Live::Polite));
            y += 6.0 + 16.0;
        }
        if buttons {
            let y = y + 12.0;
            let label = "Anywhere along the side";
            let b = Rect::from_min_size(pos2(row.left(), y), vec2(button_width(ui, label).min(inner), 30.0));
            let shown = fit(ui.painter(), label, &semibold(T_BODY), b.width() - 28.0);
            if button(ui, b, Id::unique(("place-anywhere", &peer.id)), &shown, Look::Quiet, true, Some(&format!("Let {} take the pointer anywhere along your {} edge", peer.name, edge_word(side)))).clicked() {
                self.drafts.remove(&kx);
                self.drafts.remove(&ky);
                self.choose_side(&peer.id, side, false);
            }
            // Placed apart: one step to where the screens touch.
            if apart {
                let label = "Snap to an edge";
                let below = b.right() + 8.0 + button_width(ui, label) > row.right();
                let at = if below { pos2(row.left(), b.bottom() + 8.0) } else { pos2(b.right() + 8.0, b.top()) };
                let s = Rect::from_min_size(at, vec2(button_width(ui, label).min(inner), 30.0));
                let shown = fit(ui.painter(), label, &semibold(T_BODY), s.width() - 28.0);
                if button(ui, s, Id::unique(("place-snap", &peer.id)), &shown, Look::Quiet, true, Some(&format!("Move {} beside the nearest edge of your screens, so the pointer can cross", peer.name))).clicked() {
                    self.snap(&peer.id);
                }
                if below {
                    card.more(38.0);
                }
            }
        }
    }

    /// Saved layouts: use one, save this one, delete one, let one switch in
    /// by itself on a Wi-Fi network; and layout learning on or off.
    fn profile_rows(&mut self, ui: &mut Ui, card: &mut page_ui::Card, status: &Status) {
        let wifi = self.wifi.get_or_insert_with(|| {
            let context = ui.ctx().clone();
            one_mk_ctl::wifi::Watch::start(move |_| context.request_repaint())
        });
        if ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(true)))) {
            wifi.refresh();
        }
        let network = wifi.state();
        let current = status.layout.profile.clone();
        let mut names = status.layout.profiles.clone();
        // The open profile first, so its options sit in view.
        if let Some(open) = &self.profile_open {
            names.sort_by_key(|name| name != open);
        }
        let just_opened = std::mem::take(&mut self.profile_scroll);
        let mut act: Option<(&'static str, String)> = None;
        let mut auto: Option<(String, Option<String>)> = None;
        let mut toggle_open = None;
        if names.is_empty() {
            let text = if status.peers.is_empty() {
                "Once computers are paired, save where they are to switch back in one click."
            } else {
                "Save where every computer is now, to switch back in one click."
            };
            let inner = card.w - 2.0 * page_ui::CARD_PAD;
            let h = page_ui::text_h(ui, text, inner, regular(T_BODY));
            let row = card.row(ui, h + 24.0);
            para(ui, row.min + vec2(0.0, 12.0), inner, text, regular(T_BODY), pal().ink2);
        }
        for name in &names {
            let expanded = self.profile_open.as_ref() == Some(name);
            let auto_text = self.prefs.auto.get(name).cloned();
            let auto_label = auto_text.as_ref().map(|text| format!("Comes on by itself on {text}"));
            let in_use = current.as_ref() == Some(name);
            let row = card.row(ui, ROW);
            if expanded && just_opened {
                ui.scroll_to_rect(row, None);
            }
            let more = Rect::from_center_size(pos2(row.right() - 13.0, row.center().y), vec2(26.0, 26.0));
            let at = 26.0 + 6.0;
            let reserved = if in_use { text_width(ui, "In use", semibold(T_BODY)) + at + 12.0 } else { button_width(ui, "Use") + at + 12.0 };
            page_ui::paint_words(ui, row, &Words::new(name).caption_opt(auto_label.as_deref()), reserved);
            if in_use {
                page_ui::value(ui, row, at, "In use", false, true);
            } else if page_ui::row_button(ui, row, at, Id::unique(("profile-use", name)), "Use", Look::Quiet, true, Some(&format!("Use profile {name}"))).clicked() {
                act = Some(("use", name.clone()));
            }
            let label = if expanded { format!("Close options for {name}") } else { format!("More for profile {name}") };
            if widgets::icon_button(ui, more, Id::unique(("profile-more", name)), &label, true, widgets::draw_dots).clicked() {
                toggle_open = Some(name.clone());
            }
            if expanded {
                // One line: a Wi-Fi name to switch to it by itself, and deleting it.
                let block = card.more(48.0);
                let y = block.top() + 8.0;
                let bw = |l: &str| button_width(ui, l);
                let mut x = block.right();
                let mut place = |w: f32| {
                    x -= w;
                    let r = Rect::from_min_size(pos2(x, y), vec2(w, 30.0));
                    x -= 6.0;
                    r
                };
                let del = place(bw("Delete"));
                if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
                    let off = auto_text.is_some().then(|| place(bw("Turn off")));
                    let set = place(bw("Set"));
                    let fr = Rect::from_min_max(pos2(block.left(), y), pos2(x, y + 30.0));
                    let key = format!("ssid-{name}");
                    let mut ssid = self.drafts.get(&key).cloned().unwrap_or_default();
                    let hint = if auto_text.is_some() { "Another Wi-Fi name" } else { "Wi-Fi name" };
                    let r = widgets::field(ui, fr, Id::unique(("field", &key)), &mut ssid, hint, &format!("Wi-Fi network that switches to {name} by itself"), true, false);
                    self.drafts.insert(key.clone(), ssid.clone());
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let ok = !ssid.trim().is_empty();
                    if (button(ui, set, Id::unique(("profile-auto", name)), "Set", Look::Quiet, ok, Some(&format!("Switch to {name} by itself on this Wi-Fi"))).clicked() || enter) && ok {
                        auto = Some((name.clone(), Some(ssid.trim().to_owned())));
                        self.drafts.remove(&key);
                    }
                    if let Some(or) = off
                        && button(ui, or, Id::unique(("profile-auto-off", name)), "Turn off", Look::Quiet, true, Some(&format!("Stop switching to {name} by itself"))).clicked()
                    {
                        auto = Some((name.clone(), None));
                    }
                }
                if button(ui, del, Id::unique(("profile-del", name)), "Delete", Look::Danger, true, Some(&format!("Delete profile {name}"))).clicked() {
                    act = Some(("delete", name.clone()));
                }
            }
        }

        // Layout learning, as one switch: on asks (or places), off never learns.
        let learning = self.prefs.edge_learn != "off";
        let row = card.row(ui, ROW);
        if page_ui::toggle_row(ui, row, Id::unique("layout-learn"), &Words::new("Learn positions from how I move the pointer"), Some("edge-learn"), learning, true).clicked() {
            let value = if learning { "off" } else { "suggest" };
            self.settings_command("Learn computer positions", vec!["edge".into(), "--learn".into(), value.into()]);
        }

        // Saving the layout as it is now, under a name. Nothing paired: there
        // is no layout to save yet.
        let row = card.row(ui, ROW);
        let bw = button_width(ui, "Save");
        let fr = Rect::from_min_size(pos2(row.left(), row.center().y - 17.0), vec2(row.width() - bw - 8.0, 34.0));
        let mut text = self.drafts.get("profile-new").cloned().unwrap_or_default();
        let r = widgets::field(ui, fr, Id::unique(("field", "profile-new")), &mut text, "Name this layout", "Name for a new profile of this layout", true, false);
        self.drafts.insert("profile-new".into(), text.clone());
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let b = Rect::from_min_size(pos2(row.right() - bw, row.center().y - 17.0), vec2(bw, 34.0));
        let can = !text.trim().is_empty() && !status.peers.is_empty();
        if (button(ui, b, Id::unique("profile-save"), "Save", Look::Quiet, can, Some("Save this layout as a profile")).clicked() || enter) && can {
            let n = text.trim().to_owned();
            self.drafts.remove("profile-new");
            self.busy.push(Job::Profile { verb: "save", name: n.clone() });
            self.link.profile("save", &n);
        }

        // The network a profile can come on by (and how to allow looking).
        let label = if !network.ssids.is_empty() {
            format!("Current network: {}", network.ssids.join(", "))
        } else if network.permission == one_mk_ctl::wifi::Permission::Allowed && !network.unavailable {
            "Not connected to Wi-Fi".to_owned()
        } else {
            "Current network: unavailable".to_owned()
        };
        let inner = card.w - 2.0 * page_ui::CARD_PAD;
        let explanation = network.explanation();
        let button_label = network.permission_button();
        let h = 14.0 + page_ui::text_h(ui, &label, inner, regular(T_META)) + explanation.map_or(0.0, |e| 6.0 + page_ui::text_h(ui, e, inner, regular(T_META))) + button_label.map_or(0.0, |_| 8.0 + 30.0) + 14.0;
        let row = card.row(ui, h);
        let mut y = row.top() + 14.0;
        y += para(ui, pos2(row.left(), y), inner, &label, regular(T_META), pal().ink2);
        if let Some(explanation) = explanation {
            y += 6.0 + para(ui, pos2(row.left(), y + 6.0), inner, explanation, regular(T_META), pal().ink2);
        }
        if let Some(label) = button_label {
            let r = Rect::from_min_size(pos2(row.left(), y + 8.0), vec2(button_width(ui, label), 30.0));
            if button(ui, r, Id::unique("wifi-permission"), label, Look::Quiet, true, None).clicked()
                && let Some(wifi) = &self.wifi
            {
                wifi.request_permission();
            }
        }

        if let Some(n) = toggle_open {
            self.profile_open = if self.profile_open.as_ref() == Some(&n) { None } else { Some(n) };
            self.profile_scroll = self.profile_open.is_some();
        }
        if let Some((verb, n)) = act {
            if verb == "delete" {
                // Deleting can't be undone: the sheet asks first (sheet_act does it).
                self.deleting_profile = Some((n, self.now));
            } else {
                self.busy.push(Job::Profile { verb, name: n.clone() });
                self.link.profile(verb, &n);
            }
        }
        if let Some((n, ssid)) = auto {
            self.busy.push(Job::ProfileAuto { name: n.clone(), ssid: ssid.clone() });
            self.link.profile_auto(&n, ssid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::Rect as Px;
    use one_mk_wallpaper::{Monitor, Rect as Wr};

    fn screen(x: i32, y: i32, w: i32, h: i32) -> Px {
        Px::new(x, y, w, h)
    }

    fn monitor(computer: &str, x: f64, y: f64, w: f64, h: f64) -> Monitor {
        Monitor { computer: computer.into(), name: computer.into(), rect: Wr { x, y, width: w, height: h }, local: None, scale: 1.0, online: true }
    }

    #[test]
    fn places_are_named_by_the_screen_they_touch() {
        assert_eq!(sits_words(Side::Left, Some(0), 2), "Left of main screen");
        assert_eq!(sits_words(Side::Right, Some(1), 2), "Right of second screen");
        assert_eq!(sits_words(Side::Top, Some(0), 1), "Above your screen");
        assert_eq!(sits_words(Side::Bottom, Some(3), 4), "Below screen 4");
        // Without a known neighbour the old words, capitalised.
        assert_eq!(sits_words(Side::Left, None, 2), "On your left");
        assert_eq!(sits_words(Side::Top, None, 2), "Above");
    }

    #[test]
    fn the_screen_a_computer_touches_is_the_one_it_shares_most_edge_with() {
        let mine = [screen(0, 0, 2560, 1440), screen(2560, 180, 1920, 1080)];
        // Left of the first screen; right of the second (beside both: the longer shared edge wins).
        let left = [Wr { x: -1470.0, y: 120.0, width: 1470.0, height: 956.0 }];
        assert_eq!(touching_screen(&mine, &left, Side::Left), Some(0));
        let right = [Wr { x: 4480.0, y: 300.0, width: 1920.0, height: 1080.0 }];
        assert_eq!(touching_screen(&mine, &right, Side::Right), Some(1));
        // Above, clear of everything: touches nothing.
        let apart = [Wr { x: 320.0, y: -2100.0, width: 1920.0, height: 1080.0 }];
        assert_eq!(touching_screen(&mine, &apart, Side::Top), None);
        // One pixel off still touches (screens round).
        let near = [Wr { x: -1471.0, y: 0.0, width: 1470.0, height: 900.0 }];
        assert_eq!(touching_screen(&mine, &near, Side::Left), Some(0));
    }

    #[test]
    fn touching_edges_are_where_two_computers_meet() {
        let canvas = Canvas { monitors: vec![monitor("me", 0.0, 0.0, 2560.0, 1440.0), monitor("me", 2560.0, 180.0, 1920.0, 1080.0), monitor("mac", -1470.0, 120.0, 1470.0, 956.0), monitor("far", 320.0, -2100.0, 1920.0, 1080.0)] };
        let edges = touching_edges(&canvas);
        // The Mac meets the first screen along 120..1076; nothing else crosses, and a
        // computer's own two screens meeting is not an edge to cross.
        assert_eq!(edges, vec![((0.0, 120.0), (0.0, 1076.0))]);
        let stacked = Canvas { monitors: vec![monitor("a", 0.0, 0.0, 100.0, 50.0), monitor("b", 40.0, 50.0, 100.0, 50.0)] };
        assert_eq!(touching_edges(&stacked), vec![((40.0, 50.0), (100.0, 50.0))]);
    }

    /// Ours: 2560×1440 and, right of it, 1920×1080 lower by 180. Theirs: a 1470×956 laptop.
    fn desks() -> (Desktop, Desktop) {
        (Desktop::new(vec![screen(0, 0, 2560, 1440), screen(2560, 180, 1920, 1080)]), Desktop::new(vec![screen(0, 0, 1470, 956)]))
    }

    #[test]
    fn exact_place_is_checked_as_the_command_line_checks_it() {
        assert_eq!(parse_place("-1470", "120"), Ok((-1470, 120)));
        assert_eq!(parse_place(" 0 ", "+5"), Ok((0, 5)));
        // A typographic minus, as some keyboards type it.
        assert_eq!(parse_place("\u{2212}1920", "0"), Ok((-1920, 0)));
        assert_eq!(parse_place("16777216", "-16777216"), Ok((16_777_216, -16_777_216)), "2^24 itself is allowed");
        assert_eq!(parse_place("16777217", "0"), Err(PLACE_BOUNDS));
        assert_eq!(parse_place("0", "-16777217"), Err(PLACE_BOUNDS));
        assert_eq!(parse_place("99999999999999", "0"), Err(PLACE_BOUNDS), "beyond i32 is out of bounds, not garbage");
        for (x, y) in [("", "0"), ("12.5", "0"), ("1920x", "0"), ("0", "abc"), ("1,2", "3")] {
            assert_eq!(parse_place(x, y), Err(PLACE_WHOLE), "{x:?}, {y:?}");
        }
    }

    #[test]
    fn arrow_keys_nudge_by_a_pixel_or_ten_with_shift() {
        assert_eq!(nudge((0, 0), Key::ArrowLeft, false), Some((-1, 0)));
        assert_eq!(nudge((0, 0), Key::ArrowRight, true), Some((10, 0)));
        assert_eq!(nudge((5, 5), Key::ArrowUp, false), Some((5, 4)));
        assert_eq!(nudge((5, 5), Key::ArrowDown, true), Some((5, 15)));
        assert_eq!(nudge((0, 0), Key::Enter, false), None);
        let edge = MAX_OFFSET as i32;
        assert_eq!(nudge((edge, -edge), Key::ArrowRight, true), Some((edge, -edge)), "never past what the wire carries");
        // Moved along our left edge by the keyboard, it stays touching there.
        let (mine, theirs) = desks();
        let mut at = (-1470, 120);
        for _ in 0..3 {
            at = nudge(at, Key::ArrowDown, true).unwrap();
        }
        assert_eq!(plan_landing(&mine, &theirs, at, 100.0, false), scene::Landing::Beside(Side::Left, (-1470, 150)));
    }

    #[test]
    fn the_plan_maps_points_to_pixels_and_a_carried_block_follows_the_pointer() {
        let bounds = one_mk_wallpaper::Rect { x: -1470.0, y: 0.0, width: 5950.0, height: 1440.0 };
        let frame = PlanFrame::fit(bounds, 1000.0, 300.0);
        assert!((frame.scale - 1000.0 / 5950.0).abs() < 1e-9, "as wide as the room allows");
        assert!(PlanFrame::fit(bounds, 1000.0, 150.0).size().y <= 150.0 + 1e-3, "no taller than allowed");
        let top_left = pos2(100.0, 50.0);
        let p = frame.screen(top_left, 0.0, 120.0);
        let back = frame.px(top_left, p);
        assert!((back.0 - 0.0).abs() < 0.01 && (back.1 - 120.0).abs() < 0.01);
        // Picked up at its middle; the pointer goes 17 points left and 10 down.
        let at = (-1470, 120);
        let start = frame.screen(top_left, -1470.0 + 735.0, 120.0 + 478.0);
        let px = frame.px(top_left, start);
        let grab = (px.0 - f64::from(at.0), px.1 - f64::from(at.1));
        let now = carried_to(frame.px(top_left, start + vec2(-17.0, 10.0)), grab);
        assert_eq!(now, ((-1470.0 - 17.0 / frame.scale).round() as i32, (120.0 + 10.0 / frame.scale).round() as i32));
    }

    #[test]
    fn let_go_on_the_plan_it_snaps_beside_us_unless_placed_apart() {
        let (mine, theirs) = desks();
        // Let go 300 pixels left of our left edge, level with it.
        let away = (-1470 - 300, 200);
        assert_eq!(plan_landing(&mine, &theirs, away, 50.0, false), scene::Landing::Beside(Side::Left, (-1470, 200)), "snaps to the edge it was near");
        assert_eq!(plan_landing(&mine, &theirs, away, 50.0, true), scene::Landing::Apart(Side::Left, away), "stays where it was let go");
        // Touching already: beside, either way.
        assert_eq!(plan_landing(&mine, &theirs, (-1470, 120), 50.0, true), scene::Landing::Beside(Side::Left, (-1470, 120)));
        // Far above, snapping: beside our top edge, where the pointer can cross.
        match plan_landing(&mine, &theirs, (500, -2400), 50.0, false) {
            scene::Landing::Beside(Side::Top, (_, y)) => assert_eq!(y, -956),
            other => panic!("expected beside the top, got {other:?}"),
        }
        // Over our screens, even placed freely, it never covers them.
        assert!(matches!(plan_landing(&mine, &theirs, (400, 300), 50.0, true), scene::Landing::Beside(..)));
    }

    #[test]
    fn a_typed_place_is_on_the_side_it_touches_or_mostly_is() {
        let (mine, theirs) = desks();
        assert_eq!(side_for(&mine, &theirs, (-1470, 120)), Side::Left);
        assert_eq!(side_for(&mine, &theirs, (4480, 300)), Side::Right);
        assert_eq!(side_for(&mine, &theirs, (300, -2000)), Side::Top);
        assert_eq!(side_for(&mine, &theirs, (300, 1440)), Side::Bottom);
        assert_eq!(side_for(&mine, &theirs, (-1800, 200)), Side::Left, "apart, but on the left");
    }

    #[test]
    fn a_carried_block_stays_on_the_plan() {
        let span = screen(0, 0, 1470, 956);
        assert_eq!(keep_within((-5000, 9000), span, (-3000.0, -1000.0), (6000.0, 3000.0)), (-3000, 3000 - 956));
        assert_eq!(keep_within((10, 20), span, (-3000.0, -1000.0), (6000.0, 3000.0)), (10, 20));
        // Bigger than the room: left alone.
        assert_eq!(keep_within((10, 20), span, (0.0, 0.0), (100.0, 100.0)), (10, 20));
    }

    #[test]
    fn where_a_computer_is_shown_on_the_canvas() {
        let mut m = monitor("mac", -1470.0, 120.0, 1470.0, 956.0);
        m.local = Some(Wr { x: 0.0, y: 0.0, width: 1470.0, height: 956.0 });
        let canvas = Canvas { monitors: vec![m] };
        assert_eq!(shown_offset(&canvas, "mac"), Some((-1470, 120)));
        assert_eq!(shown_offset(&canvas, "pc"), None);
    }

    #[test]
    fn a_warning_is_said_shortly() {
        assert!(short_warning(true, &[]).unwrap().starts_with("None of its screens touches yours"));
        assert!(short_warning(false, &["layout of B does not overlap yours".to_owned()]).unwrap().starts_with("None of its"));
        assert!(short_warning(false, &["B shares an edge".to_owned()]).unwrap().starts_with("It shares part"));
        assert_eq!(short_warning(false, &[]), None);
    }
}
