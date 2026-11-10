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

impl App {
    /// The layout warnings the daemon has about `name`.
    fn layout_warnings(status: &Status, name: &str) -> Vec<String> {
        status.layout.warnings.iter().filter(|w| w.contains(name)).cloned().collect()
    }

    /// Layout learning's question as a banner at `y`, when the daemon asks
    /// one and it wasn't answered here: "Yes, place it" places the computer
    /// there (`one-mk layout`), "No" hides the question (`one-mk edge
    /// --dismiss`); either way it goes at once. Returns its height.
    fn suggestion_banner(&mut self, ui: &mut Ui, x: f32, y: f32, w: f32) -> Option<f32> {
        let Some(g) = self.running().and_then(|s| s.suggestion.clone()) else {
            self.answered = None; // asked again later, it shows again
            return None;
        };
        if self.answered.as_ref() == Some(&(g.id.clone(), g.side)) {
            return None;
        }
        let question = one_mk_ctl::status::is_it_there(&g.name, g.side);
        let buttons_w = button_width(ui, SUGGEST_YES) + 8.0 + button_width(ui, SUGGEST_NO);
        let text_w = (w - buttons_w - 48.0).max(120.0);
        let q = ui.painter().layout(question.clone(), semibold(T_BODY), pal().ink, text_w);
        let d = ui.painter().layout(SUGGEST_WHY.to_owned(), regular(T_META), pal().ink2, text_w);
        let h = (q.size().y + 2.0 + d.size().y).max(34.0) + 24.0;
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
            self.plan_card(ui, &mut page, &canvas, &status, selected.as_deref());

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

    /// The plan of screens: one flat rectangle per screen, named by its
    /// computer, the edges where two computers touch in the accent. A
    /// click on one chooses that computer.
    fn plan_card(&mut self, ui: &mut Ui, page: &mut Page, canvas: &Canvas, status: &Status, selected: Option<&str>) {
        let Some(bounds) = canvas.bounds().filter(|_| canvas.monitors.len() > 1) else { return };
        let mut card = page.wide(ui, "");
        let inner_w = page.w - 48.0;
        let scale = (f64::from(inner_w) / bounds.width).min(150.0 / bounds.height);
        let (pw, ph) = ((bounds.width * scale) as f32, (bounds.height * scale) as f32);
        let area = card.more(ph + 48.0);
        let top_left = pos2(area.center().x - pw / 2.0, area.top() + 24.0);
        let map = |x: f64, y: f64| top_left + vec2(((x - bounds.x) * scale) as f32, ((y - bounds.y) * scale) as f32);
        let gap = 1.5;
        let mut named = std::collections::BTreeSet::new();
        for monitor in &canvas.monitors {
            let r = monitor.rect;
            let rect = Rect::from_min_max(map(r.x, r.y), map(r.x + r.width, r.y + r.height)).shrink(gap);
            let chosen = Some(monitor.computer.as_str()) == selected || (selected == Some("me") && monitor.computer == status.id);
            let id = Id::unique(("plan-screen", &monitor.computer, &monitor.name));
            let response = ui.interact(rect, id, widgets::sense(ui, Sense::click()));
            let p = ui.painter();
            let fill = if chosen { pal().accent_soft } else { mix(pal().surface2, pal().surface2_hover, f32::from(response.hovered())) };
            let fill = if monitor.online { fill } else { fill.gamma_multiply(0.55) };
            p.rect_filled(rect, CornerRadius::same(5), fill);
            if named.insert(monitor.computer.clone()) {
                let computer = if monitor.computer == status.id { status.name.clone() } else { status.peers.iter().find(|peer| peer.id == monitor.computer).map_or_else(|| monitor.name.clone(), |peer| peer.name.clone()) };
                let label = if monitor.online { computer } else { format!("{computer} · Offline") };
                let colour = if chosen { pal().accent_text } else if monitor.online { pal().ink2 } else { pal().ink3 };
                let shown = fit(p, &label, &regular(T_META), rect.width() - 10.0);
                if rect.width() > 36.0 && rect.height() > 18.0 {
                    p.text(rect.center(), Align2::CENTER_CENTER, shown, regular(T_META), colour);
                }
            }
            widgets::focus_ring(ui, &response, rect, 5);
            let name = status.peers.iter().find(|peer| peer.id == monitor.computer).map_or(status.name.clone(), |peer| peer.name.clone());
            widgets::a11y(&response, Role::Button, &format!("Choose {name} to place it"), Some(chosen));
            if response.clicked() {
                let id = monitor.computer.clone();
                self.select(if id == status.id { "me".to_owned() } else { id });
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        for (from, to) in touching_edges(canvas) {
            ui.painter().line_segment([map(from.0, from.1), map(to.0, to.1)], Stroke::new(2.0, pal().accent));
        }
        a11y_text(ui, Id::unique("arrange-plan"), area, Role::Image, "Plan of every computer's screens; the edges where the pointer crosses are marked", None);
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

        // Exact place.
        let text = match offset {
            Some((x, y)) => format!("Its top-left corner sits at {x}, {y} of your screens."),
            None if peer.screens.is_empty() => "Its monitors are known once it is connected; then drag it beside one of yours.".to_owned(),
            None => format!("Anywhere along your {} edge. Drag it next to a monitor to place it exactly.", edge_word(side)),
        };
        let h = page_ui::text_h(ui, &text, inner, regular(T_META));
        let buttons = offset.is_some();
        let row = card.row(ui, 14.0 + 18.0 + 4.0 + h + if buttons { 12.0 + 30.0 } else { 0.0 } + 14.0);
        ui.painter().text(row.left_top() + vec2(0.0, 23.0), Align2::LEFT_CENTER, "Exact place", regular(T_BODY), pal().ink);
        let ch = para(ui, row.min + vec2(0.0, 36.0), inner, &text, regular(T_META), pal().ink2);
        a11y_text(ui, Id::unique(("exact-place", &peer.id)), Rect::from_min_size(row.min + vec2(0.0, 14.0), vec2(inner, 22.0 + ch)), Role::Label, &text, Some(Live::Polite));
        if buttons {
            let y = row.top() + 36.0 + ch + 12.0;
            let label = "Anywhere along the side";
            let b = Rect::from_min_size(pos2(row.left(), y), vec2(button_width(ui, label).min(inner), 30.0));
            let shown = fit(ui.painter(), label, &semibold(T_BODY), b.width() - 28.0);
            if button(ui, b, Id::unique(("place-anywhere", &peer.id)), &shown, Look::Quiet, true, Some(&format!("Let {} take the pointer anywhere along your {} edge", peer.name, edge_word(side)))).clicked() {
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
                self.profile_open = None;
            }
            self.busy.push(Job::Profile { verb, name: n.clone() });
            self.link.profile(verb, &n);
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

    #[test]
    fn a_warning_is_said_shortly() {
        assert!(short_warning(true, &[]).unwrap().starts_with("None of its screens touches yours"));
        assert!(short_warning(false, &["layout of B does not overlap yours".to_owned()]).unwrap().starts_with("None of its"));
        assert!(short_warning(false, &["B shares an edge".to_owned()]).unwrap().starts_with("It shares part"));
        assert_eq!(short_warning(false, &[]), None);
    }
}
