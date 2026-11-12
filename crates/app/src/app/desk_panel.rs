//! The Desk (handoff 5.1): the 3D desk in the room left of one panel, a
//! quiet toolbar at the desk's top left, a hint line along its bottom, and
//! the panel (handoff 4.2) describing the chosen computer: how fast it
//! answers, what can be done with it, where it sits and its own settings.
//! Below 1000 points the panel goes under the desk (handoff 7).

use super::*;
use crate::icons::{self, Icon};
use crate::rtt::Quality;
use one_mk_wallpaper::Command;

/// The panel's distance from the window's edges.
pub(super) const EDGE: f32 = 16.0;
/// Narrower than this, the panel goes under the desk.
pub(super) const STACK_BELOW: f32 = 1000.0;
/// The panel's height under the desk.
pub(super) const STACKED_H: f32 = 260.0;
const PANEL_PAD: f32 = 20.0;
const GAP: f32 = 18.0;
const ROW: f32 = 46.0;
/// The toolbar's buttons.
pub(super) const TOOL_H: f32 = 32.0;
/// The toolbar as a column (a narrow window): as wide as its widest button,
/// Choose computer's icon and arrow.
const TOOL_COLUMN: f32 = 50.0;
/// The line along the desk's bottom.
pub(super) const HINT: &str = "Click a computer · drag it to where it sits · drop files on it to send";
/// Room the hint line keeps free at the desk's bottom (the camera keeps
/// another 30 points clear above it).
const HINT_ROOM: f32 = 22.0;
/// The desk at scale 1 (the handoff's mock): its box and the room it asks for.
const DESK_BOX: (f32, f32) = (964.0, 396.0);
const DESK_ROOM: (f32, f32) = (1010.0, 440.0);
const OFFLINE: &str = "Not found on the network. Start oneMK on it, on the same network as this computer. Its place on the desk is kept.";

/// The panel's width for a window `w` wide (handoff 4.2).
pub(super) fn panel_width(w: f32) -> f32 {
    if w >= 1700.0 { 380.0 } else if w >= 1280.0 { 350.0 } else { 310.0 }
}

/// How big the desk is drawn in a scene `w` × `h` (handoff 5.1).
pub(super) fn desk_scale(w: f32, h: f32) -> f32 {
    (w / DESK_ROOM.0).min(h / DESK_ROOM.1).clamp(0.5, 1.45)
}

/// Where the Desk's parts go in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DeskLayout {
    /// The room for the desk: under the top bar, left of (or above) the panel.
    pub scene: Rect,
    pub panel: Rect,
    /// The panel is under the desk.
    pub stacked: bool,
}

/// The Desk in a window `full`, under a top bar ending at `bar_bottom`.
pub(super) fn desk_layout(full: Rect, bar_bottom: f32) -> DeskLayout {
    if full.width() < STACK_BELOW {
        let panel = Rect::from_min_max(pos2(full.left() + EDGE, full.bottom() - EDGE - STACKED_H), pos2(full.right() - EDGE, full.bottom() - EDGE));
        let scene = Rect::from_min_max(pos2(full.left(), bar_bottom), pos2(full.right(), panel.top() - 4.0));
        DeskLayout { scene, panel, stacked: true }
    } else {
        let w = panel_width(full.width());
        let panel = Rect::from_min_max(pos2(full.right() - EDGE - w, bar_bottom + EDGE), pos2(full.right() - EDGE, full.bottom() - EDGE));
        let scene = Rect::from_min_max(pos2(full.left(), bar_bottom), pos2(panel.left(), full.bottom()));
        DeskLayout { scene, panel, stacked: false }
    }
}

/// The box the computers are fitted into: the handoff's desk at its scale,
/// centred in `scene` (a little above the middle), kept inside `free` (what
/// the banner, the toolbar and the hint leave). The real desk can be wider
/// than the mock's (computers far apart): it may use the free width, so a
/// wide arrangement is not shrunk below the scale's height.
pub(super) fn desk_stage(scene: Rect, free: Rect) -> Rect {
    let s = desk_scale(scene.width(), scene.height());
    let size = vec2(free.width().max(DESK_BOX.0 * s), DESK_BOX.1 * s).min(free.size());
    let mut r = Rect::from_min_size(pos2(scene.center().x - size.x / 2.0, scene.center().y - size.y / 2.0 - 10.0), size);
    let dx = (free.left() - r.left()).max(0.0) - (r.right() - free.right()).max(0.0);
    let dy = (free.top() - r.top()).max(0.0) - (r.bottom() - free.bottom()).max(0.0);
    r = r.translate(vec2(dx, dy));
    r.intersect(free)
}

/// "2 changed", "1 changed", "None".
pub(super) fn changed_words(n: usize) -> String {
    if n == 0 { "None".to_owned() } else { format!("{n} changed") }
}

/// What a computer is called in "Keys for this Mac".
pub(super) fn keys_label(os: u8) -> &'static str {
    match os {
        one_mk_proto::os::MACOS => "Keys for this Mac",
        one_mk_proto::os::WINDOWS | one_mk_proto::os::LINUX => "Keys for this PC",
        _ => "Keys for this computer",
    }
}

/// "This computer · Linux · 2 screens".
pub(super) fn me_words(os: &str, screens: usize) -> String {
    match screens {
        0 => format!("This computer · {os}"),
        1 => format!("This computer · {os} · 1 screen"),
        n => format!("This computer · {os} · {n} screens"),
    }
}

/// "ms · smooth", for the number beside it.
pub(super) fn latency_word(q: Option<Quality>) -> String {
    format!("ms · {}", q.map_or("few readings", Quality::word).to_lowercase())
}

/// The pointer (handoff's symbol, its path on the 64 grid) `h` high with
/// its tip at `tip`: where the pointer is now.
pub(super) fn pointer_mark(p: &Painter, tip: Pos2, h: f32, col: Color32) {
    // The logo's pointer path, 25 units high, in triangles (it is not convex).
    let k = h / 25.0;
    let pts = icons::SYMBOL_POINTER.map(|(x, y)| pos2(tip.x + (x - 24.0) * k, tip.y + (y - 17.0) * k));
    let mut m = egui::Mesh::default();
    for q in pts {
        m.colored_vertex(q, col);
    }
    for [a, b, c] in [[0, 1, 2], [0, 2, 5], [0, 5, 6], [2, 3, 4], [2, 4, 5]] {
        m.add_triangle(a, b, c);
    }
    p.add(egui::Shape::mesh(m));
}

/// A quiet line of the last two minutes in `r`: the accent, amber while it
/// stalls; lost packets as amber ticks.
/// Its ground is a faint dotted baseline across the whole span, so a line
/// only seconds old reads as the start of the graph, not a stray mark; the
/// line keeps 3 points clear of the top and bottom, and the latest reading
/// is a dot.
fn sparkline(p: &Painter, r: Rect, ring: &crate::rtt::Ring, now: f64, q: Option<Quality>) {
    let inner = r.shrink2(vec2(0.0, 3.0));
    let t = ring.trace(now, inner.width(), inner.height());
    let col = if matches!(q, Some(Quality::Stalls | Quality::Slow)) { pal().warn } else { pal().accent };
    let base = r.bottom() - 0.5;
    let mut x = r.left();
    while x < r.right() {
        p.hline(egui::Rangef::new(x, (x + 2.0).min(r.right())), base, Stroke::new(1.0, pal().line2));
        x += 5.0;
    }
    let mut last = None;
    for line in &t.lines {
        let pts: Vec<Pos2> = line.iter().map(|[x, y]| inner.min + vec2(*x, *y)).collect();
        match pts[..] {
            [] => {}
            [one] => {
                p.circle_filled(one, 1.5, col);
            }
            _ => {
                let curve = desk_cards::smooth(&pts);
                // A wash under the line, down to the baseline.
                let mut m = egui::Mesh::default();
                let wash = col.gamma_multiply(0.10);
                for (k, q) in curve.iter().enumerate() {
                    m.colored_vertex(*q, wash);
                    m.colored_vertex(pos2(q.x, base), col.gamma_multiply(0.0));
                    if k > 0 {
                        let i = (2 * k) as u32;
                        m.add_triangle(i - 2, i - 1, i);
                        m.add_triangle(i - 1, i + 1, i);
                    }
                }
                p.add(egui::Shape::mesh(m));
                p.add(egui::Shape::line(curve, Stroke::new(1.5, col)));
            }
        }
        last = line.last().map(|[x, y]| inner.min + vec2(*x, *y)).or(last);
    }
    if let Some(at) = last {
        p.circle_filled(at, 3.5, pal().surface);
        p.circle_filled(at, 2.25, col);
    }
    for x in &t.losses {
        let x = r.left() + x;
        p.line_segment([pos2(x, r.top() + 4.0), pos2(x, r.bottom())], Stroke::new(1.0, pal().warn.gamma_multiply(0.6)));
        p.circle_stroke(pos2(x, r.top() + 2.5), 2.5, Stroke::new(1.0, pal().warn));
    }
}

/// A toolbar chip's ground: `surface` (on, `accent_soft` in the accent's
/// edge) on a 1-point contact shadow, corner 9.
fn tool_ground(p: &Painter, r: Rect, lit: bool) {
    let radius = CornerRadius::same(widgets::button_radius(TOOL_H));
    let lift = if pal().theme == Theme::Light { 0.06 } else { 0.4 };
    p.add(egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: pal().shadow_ink.gamma_multiply(lift) }.as_shape(r, radius));
    let (fill, edge) = if lit { (pal().accent_soft, Stroke::new(1.5, pal().accent)) } else { (pal().surface, Stroke::new(1.0, pal().card_edge)) };
    p.rect(r, radius, fill, edge, StrokeKind::Inside);
    if !lit && pal().theme == Theme::Dark {
        p.add(widgets::top_light(r, widgets::button_radius(TOOL_H)));
    }
}

/// The Desk's own state.
#[derive(Default)]
pub(super) struct DeskUi {
    /// Files being chosen to send: to whom, and the answer when it comes.
    picker: Option<(String, Receiver<Option<Vec<PathBuf>>>)>,
    /// How tall the panel's contents were last drawn.
    content_h: f32,
    /// What the panel described, and when that changed (it fades in anew).
    shown: Option<String>,
    shown_at: f64,
}

impl App {
    /// The free room for the computers in `scene`, under the band (`band`
    /// tall) and the toolbar, above the hint line; the toolbar's place; and
    /// in a narrow window the row for layout learning's question. A narrow
    /// window's desk is short (the panel is under it): the toolbar stands as
    /// a column at its left, so the computers keep its height.
    /// How many buttons the toolbar has: Choose computer (five computers or
    /// more), Type on every computer and Timer (a computer online, or while
    /// broadcasting), and Lock every screen.
    pub(super) fn tool_count(&self) -> usize {
        let Some(status) = self.running() else { return 1 };
        let online = status.peers.iter().any(|p| p.online) || self.broadcasting();
        usize::from(status.peers.len() + 1 >= 5) + if online { 2 } else { 0 } + 1
    }

    pub(super) fn desk_room(&self, scene: Rect, band: f32, narrow: bool) -> (Rect, Option<Rect>, Option<Rect>) {
        let mut free = scene.shrink2(vec2(EDGE, 0.0));
        free.min.y += EDGE + band;
        let mut ask = None;
        let tools = self.desk_views_shown().then(|| {
            if narrow {
                // One column; where a banner leaves less height than the
                // buttons need (the smallest window, a long sentence) the
                // column wraps into more, rather than run into the panel.
                let per = (((free.height() + 8.0) / (TOOL_H + 8.0)).floor() as usize).max(1);
                let columns = self.tool_count().div_ceil(per).max(1) as f32;
                let width = (TOOL_COLUMN + 8.0) * columns - 8.0;
                let col = Rect::from_min_size(free.min, vec2(width, free.height()));
                free = free.shrink2(vec2(width + 8.0, 0.0));
                if self.suggestion_waiting() {
                    let row = Rect::from_min_size(free.min, vec2(free.width(), TOOL_H));
                    free.min.y = row.bottom() + 8.0;
                    ask = Some(row);
                }
                return col;
            }
            let row = Rect::from_min_size(free.min, vec2(free.width(), TOOL_H));
            free.min.y = row.bottom() + 8.0;
            row
        });
        if self.desk_views_shown() {
            free.max.y -= HINT_ROOM;
        } else {
            free.max.y -= EDGE;
        }
        (free, tools, ask)
    }

    /// The hint along the desk's bottom (while computers are paired, and
    /// not while a sheet asks something there or files are carried over).
    pub(super) fn desk_hint(&self, ui: &Ui, scene: Rect) {
        if !self.desk_views_shown() || self.sheet_open() || ui.input(|i| !i.raw.hovered_files.is_empty()) {
            return;
        }
        let p = ui.painter();
        let text = fit(p, HINT, &regular(T_META), scene.width() - 2.0 * EDGE);
        p.text(pos2(scene.center().x, scene.bottom() - 20.0), Align2::CENTER_BOTTOM, text, regular(T_META), pal().ink3);
    }

    /// The toolbar at the desk's top left (handoff 5.1): Choose computer
    /// (5 computers or more), Type on every computer, Timer and Lock every
    /// screen; icons only in a narrow window or when the labels don't fit,
    /// and down a column in a narrow window. Returns where its last button
    /// ends.
    pub(super) fn desk_toolbar(&mut self, ui: &mut Ui, row: Rect, narrow_window: bool) -> f32 {
        let Some(status) = self.running().cloned() else { return row.left() };
        let online = status.peers.iter().any(|p| p.online);
        let on = self.broadcasting();
        let menu = status.peers.len() + 1 >= 5;
        let label_w = |ui: &Ui, s: &str| text_width(ui, s, medium(T_BODY)) + 46.0;
        let full: f32 = [menu.then(|| label_w(ui, "Choose computer") + 12.0), (online || on).then(|| label_w(ui, "Type on every computer")), (online || on).then(|| label_w(ui, "Timer")), Some(label_w(ui, "Lock every screen"))]
            .into_iter()
            .flatten()
            .map(|w| w + 8.0)
            .sum();
        let icons_only = narrow_window || full > row.width();
        let (mut x, mut y) = (row.left(), row.top());
        let mut place = |w: f32| {
            // A column that has run out of height goes on in the next one (see `desk_room`).
            if narrow_window && y > row.top() && y + TOOL_H > row.bottom() + 0.5 {
                y = row.top();
                x += TOOL_COLUMN + 8.0;
            }
            let r = Rect::from_min_size(pos2(x, y), vec2(w, TOOL_H));
            if narrow_window {
                y += TOOL_H + 8.0;
            } else {
                x += w + 8.0;
            }
            r
        };
        let tool = |ui: &mut Ui, r: Rect, id: &str, icon: Icon, label: &str, lit: bool, enabled: bool, name: &str| {
            // Chips that float on the floor: `surface` in a card's edge on a
            // 1-point contact shadow (a quiet fill on the ground reads muddy);
            // the button over it brings hover, press and focus.
            tool_ground(ui.painter(), r, lit);
            let resp = button(ui, r, Id::unique(id), "", Look::Text, enabled, Some(name));
            let p = ui.painter();
            let ink = if lit { pal().accent } else if enabled { pal().ink2 } else { pal().ink3 };
            let icon_at = if label.is_empty() { Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0)) } else { Rect::from_min_size(pos2(r.left() + 12.0, r.center().y - 8.0), vec2(16.0, 16.0)) };
            icons::draw(p, icon, icon_at, ink);
            if !label.is_empty() {
                p.text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, label, medium(T_BODY), if enabled || lit { pal().ink } else { pal().ink3 });
            }
            resp
        };
        if menu {
            let label = if icons_only { "" } else { "Choose computer" };
            let r = place(if icons_only { 50.0 } else { label_w(ui, label) + 12.0 });
            let resp = tool(ui, r, "desk-computers-menu", Icon::Desk, label, false, !self.sheet_open(), "Choose computer: show full names and connection states");
            let c = pos2(r.right() - 14.0, r.center().y);
            let p = ui.painter();
            p.line_segment([c + vec2(-3.0, -1.5), c + vec2(0.0, 1.5)], Stroke::new(1.5, pal().ink2));
            p.line_segment([c + vec2(0.0, 1.5), c + vec2(3.0, -1.5)], Stroke::new(1.5, pal().ink2));
            let resp = if icons_only { resp.on_hover_text("Choose computer") } else { resp };
            // Development aid: ONE_MK_APP_COMPUTERS_MENU=1 opens it (screenshots).
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            if std::env::var_os("ONE_MK_APP_COMPUTERS_MENU").is_some() && self.now > 1.0 && self.now < 1.2 {
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
            }
            if let Some(id) = self.computers_menu(&resp, &status, self.selected_id().as_deref()) {
                self.select(id);
            }
        }
        if online || on {
            let busy = self.busy.iter().any(|j| matches!(j, Job::Broadcast(_)));
            let key = if on { format!(" {}", self.broadcast_stop_hint()) } else { String::new() };
            let label = if icons_only { "" } else { "Type on every computer" };
            let r = place(if icons_only { TOOL_H } else { label_w(ui, label) });
            let enabled = !busy && !on;
            let resp = tool(ui, r, "broadcast", Icon::Broadcast, label, on, enabled, "Type on every computer");
            resp.widget_info(|| egui::WidgetInfo::selected(Role::Switch, enabled, on, "Type on every computer"));
            let tip = if on { format!("Typing on every computer.{key}") } else { "Everything you type, click and scroll here goes to every connected computer at once.".to_owned() };
            if resp.on_hover_text(tip).clicked() {
                self.ask_broadcast(!on);
            }
            // When typing on every computer stops by itself.
            let label = if icons_only { "" } else { "Timer" };
            let r = place(if icons_only { TOOL_H } else { label_w(ui, label) });
            let name = "Timer: when typing on every computer stops by itself";
            let resp = tool(ui, r, "broadcast-options", Icon::Clock, label, false, !on, name).on_hover_text(name);
            widgets::debug_open_menu(ui, &resp, "timer");
            // A field inside: a click in it must not close the menu (a menu's
            // default), only a click outside it or Escape.
            egui::Popup::menu(&resp).width(280.0).align(egui::RectAlign::BOTTOM_START).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                let (row, _) = ui.allocate_exact_size(vec2(280.0, 34.0), Sense::hover());
                self.broadcast_timeout(ui, row.shrink2(vec2(4.0, 0.0)));
                let (note, _) = ui.allocate_exact_size(vec2(280.0, 22.0), Sense::hover());
                ui.painter().text(note.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, "Use zero to keep broadcasting until you stop it.", regular(T_META), pal().ink2);
                a11y_text(ui, Id::unique("broadcast-timeout-note"), note, Role::Label, "Use zero to keep broadcasting until you stop it.", None);
            });
        }
        let label = if icons_only { "" } else { "Lock every screen" };
        let r = place(if icons_only { TOOL_H } else { label_w(ui, label) });
        let busy = self.busy.contains(&Job::LockAll);
        let resp = tool(ui, r, "lock-all", Icon::Lock, label, false, !busy, "Lock every screen: lock this computer and every connected one");
        let resp = if icons_only { resp.on_hover_text("Lock every screen") } else { resp };
        if resp.clicked() {
            self.busy.push(Job::LockAll);
            self.link.lock_all();
        }
        r.right()
    }

    /// The panel (handoff 4.2): a card with what the chosen computer is and
    /// what can be done with it; it scrolls when it holds more than fits.
    pub(super) fn desk_panel(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        if let Some(paths) = self.desk_ui.picker.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
            let (id, _) = self.desk_ui.picker.take().expect("picker answered");
            if let Some(paths) = paths.filter(|p| !p.is_empty()) {
                match self.online_targets().into_iter().find(|i| self.scene.devices[*i].id == id) {
                    Some(i) => self.send_to(i, paths),
                    None => self.toast_error("That computer disconnected. Reconnect it, then choose the files again.", None, now),
                }
            }
        }
        // Beside the desk the panel is as tall as what it holds (no void
        // under its last row), easing to a new height when that changes;
        // under the desk (a narrow window) it keeps its band.
        let rect = if rect.width() <= 420.0 && self.desk_ui.content_h > 0.0 {
            let want = (self.desk_ui.content_h + 2.0 * PANEL_PAD).min(rect.height());
            let h = tween(ui.ctx(), Id::unique("desk-panel-h"), want, motion::space(0.18), motion::out_cubic);
            Rect::from_min_size(rect.min, vec2(rect.width(), h.min(rect.height())))
        } else {
            rect
        };
        // A window shrunk to almost nothing has no room for it.
        if rect.width() <= 2.0 * PANEL_PAD + 1.0 || rect.height() <= 2.0 * PANEL_PAD + 1.0 {
            return;
        }
        widgets::panel(ui, rect);
        let inner = rect.shrink(PANEL_PAD);
        // A new computer described: its contents fade in, rising a little.
        let shown = format!("{:?}{:?}", self.status.as_ref().map(Option::is_some), self.selected_id());
        if self.desk_ui.shown.as_ref() != Some(&shown) {
            let first = self.desk_ui.shown.is_none();
            self.desk_ui.shown = Some(shown);
            self.desk_ui.shown_at = if first { f64::NEG_INFINITY } else { now };
        }
        let e = motion::progress(now, self.desk_ui.shown_at, motion::fade(0.18), motion::out_cubic);
        if e < 1.0 {
            ui.ctx().request_repaint();
        }
        // Under a sheet the panel steps back.
        let dim = tween(ui.ctx(), Id::unique("panel-dim"), if self.sheet_open() { 0.3 } else { 1.0 }, motion::fade(0.2), motion::out_cubic);
        // The scroll bar (when it scrolls) runs in the right padding.
        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(inner.min, pos2(rect.right() - 4.0, inner.bottom()))).id_salt("desk-panel"));
        child.set_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
        child.set_opacity(e * dim);
        let want = self.desk_ui.content_h.max(inner.height());
        let rise = motion::space(4.0) * (1.0 - e);
        let mut used = 0.0;
        egui::ScrollArea::vertical().id_salt("desk-panel-scroll").auto_shrink([false, false]).show(&mut child, |ui| {
            let (all, _) = ui.allocate_exact_size(vec2(inner.width(), want), Sense::hover());
            used = self.panel_body(ui, all.translate(vec2(0.0, rise)), now) - rise;
        });
        if (used - self.desk_ui.content_h).abs() > 0.5 {
            self.desk_ui.content_h = used;
            ui.ctx().request_repaint();
        }
    }

    /// The panel's contents in `c`; how tall they are.
    fn panel_body(&mut self, ui: &mut Ui, c: Rect, now: f64) -> f32 {
        let Some(status) = self.status.clone() else {
            let h = para(ui, c.min, c.width(), "Looking for oneMK…", semibold(T_TITLE), pal().ink);
            return h;
        };
        let Some(status) = status else { return self.off_panel(ui, c) };
        if status.peers.is_empty() {
            return self.first_panel(ui, c, &status);
        }
        let sel = self.selected_id();
        match status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() {
            Some(peer) => self.peer_panel(ui, c, &status, &peer, now),
            None => self.me_panel(ui, c, &status),
        }
    }

    /// A title and the sentence under it, read out together.
    fn panel_words(&self, ui: &Ui, c: Rect, title: &str, sub: &str, key: &str) -> f32 {
        let th = para(ui, c.min, c.width(), title, semibold(T_TITLE), pal().ink);
        let sh = para(ui, c.min + vec2(0.0, th + 6.0), c.width(), sub, regular(T_BODY), pal().ink2);
        a11y_text(ui, Id::unique(("panel-words", key)), Rect::from_min_size(c.min, vec2(c.width(), th + 6.0 + sh)), Role::Status, &format!("{title}. {sub}"), Some(Live::Polite));
        th + 6.0 + sh
    }

    /// An empty state's emblem: the icon on a soft tile (`accent_soft` for
    /// a first step, `surface2` for a stop), 44 square; its height and gap.
    fn emblem(&self, ui: &Ui, c: Rect, icon: Icon, lit: bool) -> f32 {
        let p = ui.painter();
        let tile = Rect::from_min_size(c.min, vec2(44.0, 44.0));
        let (fill, ink) = if lit { (pal().accent_soft, pal().accent_text) } else { (pal().surface2, pal().ink2) };
        p.rect(tile, CornerRadius::same(R_INNER), fill, Stroke::new(1.0, pal().line), StrokeKind::Inside);
        icons::draw(p, icon, Rect::from_center_size(tile.center(), vec2(22.0, 22.0)), ink);
        tile.height() + SP_4
    }

    /// The panel's footer: a hairline across the panel, then `draw` below it.
    fn footer_rule(&self, ui: &Ui, c: Rect, y: f32) -> f32 {
        ui.painter().hline(egui::Rangef::new(c.left() - PANEL_PAD + 1.0, c.right() + PANEL_PAD - 1.0), y + 0.5, Stroke::new(1.0, pal().line));
        y + SP_2
    }

    /// oneMK is off: what that means, and Start oneMK.
    fn off_panel(&mut self, ui: &mut Ui, c: Rect) -> f32 {
        let (title, sub) = self.headline_text();
        let top = c.top() + self.emblem(ui, c, Icon::Power, false);
        let mut y = top + self.panel_words(ui, Rect::from_min_max(pos2(c.left(), top), c.max), &title, &sub, "off") + GAP;
        let busy = self.busy.contains(&Job::Start);
        let b = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 38.0));
        if button(ui, b, Id::unique("start-in-card"), if busy { "Starting…" } else { "Start oneMK" }, Look::Primary, !busy && !self.sheet_open(), Some("Start oneMK on this computer")).clicked() {
            self.busy.push(Job::Start);
            self.link.start();
        }
        y = self.footer_rule(ui, c, b.bottom() + GAP);
        y = self.autostart_row(ui, c, y, false);
        y - c.top()
    }

    /// Nothing paired: the first step, with its one action.
    fn first_panel(&mut self, ui: &mut Ui, c: Rect, _status: &Status) -> f32 {
        let text = "Open oneMK on both computers on the same network. Compare the six-digit codes to pair securely.";
        let top = c.top() + self.emblem(ui, c, Icon::Plus, true);
        let mut y = top + self.panel_words(ui, Rect::from_min_max(pos2(c.left(), top), c.max), "Pair your first computer", text, "first") + GAP;
        let b = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 38.0));
        if button(ui, b, Id::unique("pair-in-card"), "Pair a computer", Look::Primary, !self.sheet_open(), None).clicked() {
            let now = self.now;
            self.start_pairing(now);
        }
        y = self.footer_rule(ui, c, b.bottom() + GAP);
        y = self.alone_row(ui, c, y, _status);
        y = self.autostart_row(ui, c, y, true);
        y = self.received_row(ui, c, y, false);
        y - c.top()
    }

    /// Pairing is optional: what already works on this computer alone, as
    /// two quiet buttons under a line saying so. Returns the next y.
    fn alone_row(&mut self, ui: &mut Ui, c: Rect, y: f32, status: &Status) -> f32 {
        let head = "Or use oneMK on this computer alone";
        let hh = para(ui, pos2(c.left(), y), c.width(), head, semibold(T_BODY), pal().ink);
        let mut choices: Vec<(&str, Tab)> = Vec::new();
        if status.wallpaper.as_ref().is_some_and(|w| w.available) { choices.push(("Wallpaper", Tab::Wallpaper)); }
        // Changing keys here needs macOS or Windows; Linux can't yet.
        if cfg!(any(windows, target_os = "macos")) { choices.push(("Change keys", Tab::Keys)); }
        choices.push(("Clipboard history", Tab::Files));
        // Each button is as wide as its words and they wrap to a new row, so
        // no label is cut whatever the font or the card's width.
        let gap = 8.0;
        let mut x = 0.0;
        let mut row_top = y + hh + 8.0;
        for (label, tab) in &choices {
            let w = (widgets::button_width(ui, label) + 8.0).min(c.width());
            if x > 0.0 && x + w > c.width() {
                x = 0.0;
                row_top += 34.0 + gap;
            }
            let b = Rect::from_min_size(pos2(c.left() + x, row_top), vec2(w, 34.0));
            if button(ui, b, Id::unique(("alone", *label)), label, Look::Quiet, !self.sheet_open(), Some(&format!("{label}, on this computer"))).clicked() {
                self.set_tab(*tab);
            }
            x += w + gap;
        }
        row_top + 34.0 + GAP
    }

    /// The header: the screen as a small swatch, the name, and a line with
    /// its status mark. `mark` is drawn at the line's start.
    fn panel_header(&self, ui: &Ui, c: Rect, id: &str, name: &str, sub: &str, mark: impl FnOnce(&Painter, Pos2)) -> f32 {
        let p = ui.painter();
        let h = 42.0;
        let sw = Rect::from_min_size(pos2(c.left(), c.top() + (h - 31.0) / 2.0), vec2(46.0, 31.0));
        p.add(egui::epaint::Shadow { offset: [0, 1], blur: 3, spread: 0, color: pal().shadow_ink.gamma_multiply(if pal().theme == Theme::Light { 0.12 } else { 0.5 }) }.as_shape(sw, CornerRadius::same(6)));
        p.rect_filled(sw, CornerRadius::same(6), crate::style::BEZEL);
        // Its screen as the desk shows it: its wallpaper, or dark while it is away.
        let device = self.scene.index(id).map(|i| &self.scene.devices[i]);
        if device.is_some_and(|d| !d.online) {
            p.rect_filled(sw.shrink(3.0), CornerRadius::same(3), crate::style::SCREEN_PLATE);
        } else {
            let tex = crate::scene::wallpaper(ui.ctx(), device.map_or(ME_WALL, |d| d.wall));
            p.add(egui::epaint::RectShape::filled(sw.shrink(3.0), CornerRadius::same(3), crate::style::UNTINTED).with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
        }
        let x = sw.right() + 12.0;
        let room = c.right() - x;
        let shown = fit(p, name, &semibold(T_TITLE), room);
        p.text(pos2(x, c.top() + 11.0), Align2::LEFT_CENTER, &shown, semibold(T_TITLE), pal().ink);
        if shown != name {
            ui.interact(Rect::from_min_size(pos2(x, c.top()), vec2(room, 22.0)), Id::unique(("panel-name", id)), Sense::hover()).on_hover_text(name);
        }
        mark(p, pos2(x + 3.5, c.top() + 32.0));
        p.text(pos2(x + 13.0, c.top() + 32.0), Align2::LEFT_CENTER, fit(p, sub, &regular(T_META), room - 13.0), regular(T_META), pal().ink2);
        a11y_text(ui, Id::unique(("panel-head", id)), Rect::from_min_size(c.min, vec2(c.width(), h)), Role::Heading, &format!("{name}, {sub}"), None);
        h
    }

    /// This computer: its own switches, its files, its ID.
    fn me_panel(&mut self, ui: &mut Ui, c: Rect, status: &Status) -> f32 {
        let here = self.pointer_on().as_deref() == Some("me");
        let screens = self.scene.devices.first().map_or(0, |d| d.screens.len());
        let sub = me_words(os_name(me_os()), screens);
        let mut y = c.top() + self.panel_header(ui, c, "me", &status.name, &sub, |p, at| {
            if here { pointer_mark(p, at - vec2(3.5, 5.5), 11.0, pal().accent) } else { widgets::status_mark(p, at, 7.0, widgets::Mark::Ok) }
        }) + GAP;
        // Control is elsewhere: the way back, as the one action.
        if status.controlling.is_some() || status.controlled_by.is_some() {
            let job = Job::Switch { to: "here".into(), name: status.name.clone() };
            let busy = self.busy.contains(&job);
            let b = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 38.0));
            if button(ui, b, Id::unique("quick-switch-here"), "Switch back here", Look::Primary, !busy, Some("Bring the mouse and keyboard back to this computer")).clicked() {
                self.busy.push(job);
                self.link.switch("here", &status.name);
            }
            y = b.bottom() + GAP;
        }
        y -= SP_2;
        y = self.autostart_row(ui, c, y, true);
        // Keep the pointer here.
        let locked = self.pointer_locked();
        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        if self.switch_row(ui, row, "lock-pointer", "Keep the pointer here", Some("Edges lead nowhere, for games"), locked, true).clicked() {
            self.pending_lock = Some((!locked, self.now));
            self.busy.push(Job::LockPointer(!locked));
            self.link.lock_pointer(!locked);
        }
        widgets::hairline(ui.painter(), c.x_range(), row.bottom());
        y = self.received_row(ui, c, row.bottom(), true);
        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        widgets::paint_row(ui.painter(), row, "ID", None, None);
        ui.painter().text(row.right_center(), Align2::RIGHT_CENTER, &status.id, mono(T_BODY), pal().ink2);
        a11y_text(ui, Id::unique("panel-me-id"), row, Role::Label, &format!("ID {}", status.id), None);
        row.bottom() - c.top()
    }

    /// Start at login, a row from `y`; where the next row starts.
    fn autostart_row(&mut self, ui: &mut Ui, c: Rect, y: f32, line_after: bool) -> f32 {
        let on = self.autostart.unwrap_or(false);
        let busy = self.busy.iter().any(|j| matches!(j, Job::Autostart(_)));
        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        let r = self.switch_row(ui, row, "autostart", "Start at login", None, on, self.autostart.is_some() && !busy);
        if self.autostart.is_none() {
            r.on_hover_text("Checking…");
        } else if r.clicked() {
            // Shown at once; the link reports what really happened.
            self.busy.push(Job::Autostart(!on));
            self.autostart = Some(!on);
            self.link.set_autostart(!on);
        }
        if line_after {
            widgets::hairline(ui.painter(), c.x_range(), row.bottom());
        }
        row.bottom()
    }

    /// Received files → Open, a row from `y`.
    fn received_row(&mut self, ui: &mut Ui, c: Rect, y: f32, line_after: bool) -> f32 {
        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        widgets::paint_row(ui.painter(), row, "Received files", None, None);
        let w = text_width(ui, "Open", widgets::button_font(Look::Text)) + 26.0;
        // Its text flush with the row's right edge.
        let b = Rect::from_min_size(pos2(row.right() - w + 13.0, row.center().y - 14.0), vec2(w, 28.0));
        let busy = self.busy.contains(&Job::Open);
        if widgets::content_button(ui, b, Id::unique("open-dl-desk"), "Open", Look::Text, !busy, Some("Open received files folder")).clicked() {
            self.busy.push(Job::Open);
            self.link.open_downloads();
        }
        if line_after {
            widgets::hairline(ui.painter(), c.x_range(), row.bottom());
        }
        row.bottom()
    }

    /// A row with a switch: its label, a caption under it when given, and
    /// the "i" when there is an explanation filed under `key`.
    #[allow(clippy::too_many_arguments)]
    fn switch_row(&self, ui: &mut Ui, row: Rect, key: &str, label: &str, caption: Option<&str>, on: bool, enabled: bool) -> egui::Response {
        let id = Id::unique(("panel-switch", key));
        let r = ui.interact(widgets::hit_rect(row), id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
        let ctx = ui.ctx().clone();
        let press = tween(&ctx, id.with("press"), f32::from(enabled && r.is_pointer_button_down_on()), motion::fade(0.06), motion::out_cubic);
        let knob = tween(&ctx, id.with("knob"), f32::from(on), motion::space(0.26), motion::back_out);
        let fill = tween(&ctx, id.with("track"), f32::from(on), motion::fade(0.2), motion::linear);
        let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
        // The label line (with its "i"); the caption under it.
        let line = if caption.is_some() { Rect::from_min_size(pos2(row.left(), row.center().y - 18.0), vec2(row.width(), 20.0)) } else { Rect::from_center_size(row.center(), vec2(row.width(), 20.0)) };
        let shown = widgets::label_help(ui, line, id, label, key, enabled, widgets::SWITCH.x + 8.0);
        if let Some(cap) = caption {
            let p = ui.painter();
            p.text(pos2(row.left(), row.center().y + 9.0), Align2::LEFT_CENTER, fit(p, cap, &regular(T_META), row.width() - widgets::SWITCH.x - 12.0), regular(T_META), pal().ink2);
        }
        let track = Rect::from_min_size(pos2(row.right() - widgets::SWITCH.x, row.center().y - widgets::SWITCH.y / 2.0), widgets::SWITCH);
        widgets::paint_switch(ui.painter(), track, widgets::Switch { on, knob, fill, undimmed: false }, widgets::State { hover, press, enabled, focus: r.has_focus() && widgets::keyboard_focus(&ctx) });
        if r.hovered() && enabled {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let (name, cut) = (label.to_owned(), shown != label);
        r.widget_info(move || egui::WidgetInfo::selected(Role::Switch, enabled, on, &name));
        if cut { r.on_hover_text(label) } else { r }
    }

    /// A row that is a button: its label, its value at the right.
    fn value_row(&self, ui: &mut Ui, row: Rect, key: &str, label: &str, value: &str) -> egui::Response {
        let id = Id::unique(("panel-row", key));
        let r = ui.interact(widgets::hit_rect(row), id, widgets::sense(ui, Sense::click()));
        let hover = tween(ui.ctx(), id.with("hover"), f32::from(r.hovered()), 0.12, motion::out_cubic);
        let wash = row.expand2(vec2(8.0, -4.0));
        ui.painter().rect_filled(wash, CornerRadius::same(8), pal().hover.gamma_multiply(hover));
        widgets::paint_row(ui.painter(), row, label, None, Some(value));
        widgets::focus_ring(ui, &r, wash, 8);
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        widgets::a11y(&r, Role::Button, &format!("{label}: {value}"), None);
        r
    }

    /// A paired computer: how fast it answers, what to do with it, where
    /// it sits and its own settings; Remove at the foot.
    fn peer_panel(&mut self, ui: &mut Ui, c: Rect, status: &Status, peer: &PeerStatus, now: f64) -> f32 {
        let sub = format!("{} · {}", os_name(peer.os), if peer.online { "online" } else { "offline" });
        let online = peer.online;
        let mut y = c.top() + self.panel_header(ui, c, &peer.id, &peer.name, &sub, |p, at| {
            widgets::status_mark(p, at, 7.0, if online { widgets::Mark::Ok } else { widgets::Mark::Off })
        }) + GAP;
        if peer.online {
            y = self.latency_line(ui, c, y, status, peer, now) + GAP;
            y = self.peer_actions(ui, c, y, status, peer) + GAP;
        } else {
            // Away: why, in a quiet well, with the "off" mark.
            let th = widgets::para_code_h(ui, c.width() - 42.0, OFFLINE, regular(T_BODY));
            let well = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), th + 2.0 * 14.0));
            widgets::well(ui.painter(), well);
            widgets::status_mark(ui.painter(), pos2(well.left() + 18.0, well.top() + 14.0 + 9.0), 8.0, widgets::Mark::Off);
            para(ui, pos2(well.left() + 30.0, well.top() + 14.0), c.width() - 42.0, OFFLINE, regular(T_BODY), pal().ink2);
            y = well.bottom() + GAP;
        }
        // Where it sits: set here without dragging.
        widgets::section_label(ui.painter(), pos2(c.left(), y), Align2::LEFT_TOP, "Sits", c.width());
        let track = Rect::from_min_size(pos2(c.left(), y + 22.0), vec2(c.width(), 34.0));
        let side = self.side_shown(&peer.id, peer.side);
        let labels: Vec<&str> = PLACES.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique(("position", &peer.id)),
            track,
            cells: Pills::even(track, 4, 3.0),
            labels: &labels,
            names: PLACES.iter().map(|(_, l)| format!("Position of {}: {l}", peer.name)).collect(),
            chosen: PLACES.iter().position(|(s, _)| *s == side),
            enabled: vec![true; 4],
            role: Role::RadioButton,
            font: T_META,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.choose_side(&peer.id, PLACES[k].0, true);
        }
        y = track.bottom() + SP_2;
        // Ctrl and Cmd: chosen from a menu.
        let current = self.pending_swap.get(&peer.id).cloned().unwrap_or_else(|| peer.swap.clone());
        let words = SWAPS.iter().find(|(v, _)| *v == current).map_or("Auto", |(_, l)| *l);
        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        let resp = self.value_row(ui, row, &format!("swap-{}", peer.id), "Ctrl and Cmd", words);
        let mut chosen = None;
        widgets::debug_open_menu(ui, &resp, "swap");
        egui::Popup::menu(&resp).width(200.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            for (val, label) in SWAPS {
                let name = format!("Ctrl and Cmd for {}: {label}", peer.name);
                if widgets::menu_item(ui, 200.0, widgets::MenuItem::choice(label, &name, current == val)).clicked() {
                    chosen = Some(val);
                    ui.close();
                }
            }
        });
        if let Some(val) = chosen.filter(|v| *v != current) {
            self.pending_swap.insert(peer.id.clone(), val.to_owned());
            self.busy.push(Job::Swap { id: peer.id.clone(), name: peer.name.clone(), swap: val.to_owned() });
            self.link.set_swap(&peer.id, &peer.name, val);
            self.sync_scene();
        }
        widgets::hairline(ui.painter(), c.x_range(), row.bottom());
        // Its keys: how many are changed for it; the Keys page, for it.
        let facts = crate::tune::Facts::of(&crate::tune::Scope::Peer(peer.name.clone()), &[crate::tune::Peer { name: peer.name.clone(), id: peer.id.clone(), os: peer.os }]);
        let n = self.prefs.input.specs(&facts).iter().filter(|(_, at)| at.is_ok()).count();
        let row = Rect::from_min_size(pos2(c.left(), row.bottom()), vec2(c.width(), ROW));
        if self.value_row(ui, row, &format!("keys-{}", peer.id), keys_label(peer.os), &changed_words(n)).on_hover_text(format!("Change keys for {}", peer.name)).clicked() {
            self.keys.scope = crate::tune::Scope::Peer(peer.name.clone());
            self.set_tab(Tab::Keys);
        }
        widgets::hairline(ui.painter(), c.x_range(), row.bottom());
        // The wallpaper: whether this computer plays its part.
        let wallpaper = status.wallpaper.clone().unwrap_or_default();
        let plan = wallpaper.plan.as_ref();
        let on = plan.is_none_or(|plan| plan.computers.contains(&peer.id));
        let row = Rect::from_min_size(pos2(c.left(), row.bottom()), vec2(c.width(), ROW));
        let r = self.switch_row(ui, row, &format!("wallpaper-{}", peer.id), "Wallpaper plays here", None, on, wallpaper.available && plan.is_some());
        if r.clicked() {
            self.wallpaper_command(Command::Computer { id: peer.id.clone(), enabled: !on });
        }
        // Destructive, in the footer under a hairline: a danger text button
        // (it asks first), right after the rows (the panel ends there).
        y = self.footer_rule(ui, c, row.bottom() + SP_2);
        let label = "Remove this computer";
        let w = text_width(ui, label, widgets::button_font(Look::Danger)) + 26.0;
        // Its text flush with the panel's left edge.
        let b = Rect::from_min_size(pos2(c.left() - 13.0, y), vec2(w, 30.0));
        if widgets::content_button(ui, b, Id::unique(("remove", &peer.id)), label, Look::Danger, !self.sheet_open(), Some(&format!("{label}: {} (asks first)", peer.name))).clicked() {
            self.removing = Some((peer.id.clone(), peer.name.clone(), now));
        }
        b.bottom() - c.top()
    }

    /// The round trip now (36 mono), what it means, and the last two
    /// minutes as a small line; Measure again beside the words.
    fn latency_line(&mut self, ui: &mut Ui, c: Rect, y: f32, status: &Status, peer: &PeerStatus, now: f64) -> f32 {
        // A readout well: the number (32 mono) and its word on one baseline,
        // Measure again at the right; under them, once there are readings
        // enough, the last two minutes across the well's whole width.
        let summary = self.summary_of(&peer.id);
        let q = summary.and_then(|s| s.quality());
        let ring = self.rtts.of(&peer.id).filter(|_| summary.is_some_and(|s| s.count >= rtt::MIN_SAMPLES));
        let graphed = ring.is_some();
        // A wide panel (under the desk) has room for the graph beside the
        // number, so the well stays one row there.
        let inline = c.width() >= 480.0;
        let well = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), if graphed && !inline { 100.0 } else { 64.0 }));
        widgets::well(ui.painter(), well);
        let c = well.shrink2(vec2(14.0, 0.0));
        let r = Rect::from_min_size(pos2(c.left(), well.top() + 14.0), vec2(c.width(), 36.0));
        let measure = self.latency.contains(&peer.id) || self.failed_measure.contains(&peer.id);
        let spark_w = if inline { (r.width() * 0.4).min(320.0) } else { 0.0 };
        let room = r.width() - if measure { 32.0 } else { 0.0 } - if graphed && inline { spark_w + 16.0 } else { 0.0 };
        let p = ui.painter().clone();
        let mut x = r.left();
        match summary {
            Some(s) => {
                let n = rtt::ms_number(s.now_us);
                let g = p.layout_no_wrap(n, mono_medium(T_BIG), pal().ink);
                // The number's baseline at the row's foot.
                p.galley(pos2(x, r.bottom() - g.size().y + 4.0), g.clone(), pal().ink);
                x += g.size().x + 10.0;
                let word = latency_word(q);
                let col = if matches!(q, Some(Quality::Stalls | Quality::Slow)) { pal().warn } else { pal().ink2 };
                let word = fit(&p, &word, &mono(T_BODY), (r.left() + room - x).max(20.0));
                let wg = p.layout_no_wrap(word, mono(T_BODY), col);
                p.galley(pos2(x, r.bottom() - wg.size().y - 2.0), wg.clone(), col);
                x += wg.size().x + 6.0;
            }
            None => {
                let text = if !status.broadcast.is_empty() { "Latency unavailable during broadcast" } else { "No samples yet" };
                let shown = fit(&p, text, &regular(T_BODY), room);
                p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, &shown, regular(T_BODY), pal().ink2);
                x += text_width(ui, &shown, regular(T_BODY)) + 6.0;
                if shown != text {
                    ui.interact(Rect::from_min_size(r.min, vec2(room, r.height())), Id::unique(("peer-latency-cut", &peer.id)), Sense::hover()).on_hover_text(text);
                }
            }
        }
        let spark = if inline {
            let right = r.right() - if measure { 32.0 } else { 0.0 };
            Rect::from_min_max(pos2(right - spark_w, r.top() + 4.0), pos2(right, r.bottom()))
        } else {
            Rect::from_min_max(pos2(c.left(), r.bottom() + 10.0), pos2(c.right(), well.bottom() - 12.0))
        };
        if let Some(ring) = ring {
            sparkline(&p, spark, ring, self.rtt_now(), q);
        }
        if measure {
            let busy = self.measuring.is_some();
            let b = Rect::from_min_size(pos2(r.right() - 24.0, r.center().y - 10.0), vec2(24.0, 24.0));
            let spin = if self.measuring.as_ref() == Some(&peer.id) && !motion::reduced() {
                ui.ctx().request_repaint();
                (now * 5.0) as f32
            } else {
                0.0
            };
            if widgets::icon_button(ui, b, Id::unique(("measure", &peer.id)), if busy { "Measuring…" } else { "Measure again" }, !busy, |pp, rr, col| widgets::draw_refresh(pp, rr, col, spin)).clicked() {
                self.measure(&peer.id);
            }
        }
        // The graph and the number, as words (on hover and for screen readers).
        let words = self.latency_words(peer);
        a11y_text(ui, Id::unique(("peer-latency", &peer.id)), r, Role::Label, &words, None);
        ui.interact(Rect::from_min_max(r.min, pos2(x.min(r.left() + room), r.max.y)), Id::unique(("peer-latency-tip", &peer.id)), Sense::hover()).on_hover_text(&words);
        if graphed {
            ui.interact(spark, Id::unique(("peer-spark-tip", &peer.id)), Sense::hover()).on_hover_text(format!("Round trip, last {}", rtt::span_words()));
        }
        well.bottom()
    }

    /// Switch to it (the one primary action), then Send files and Get
    /// clipboard side by side.
    fn peer_actions(&mut self, ui: &mut Ui, c: Rect, y: f32, status: &Status, peer: &PeerStatus) -> f32 {
        let (job, label, name) = if status.controlling.as_ref() == Some(&peer.name) {
            (Job::Switch { to: "here".into(), name: status.name.clone() }, "Switch back here", "Bring the mouse and keyboard back to this computer".to_owned())
        } else {
            (Job::Switch { to: peer.id.clone(), name: peer.name.clone() }, "Switch to it", format!("Switch to it: move the mouse and keyboard to {}", peer.name))
        };
        let busy = self.busy.contains(&job);
        let b = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 38.0));
        if widgets::content_button(ui, b, Id::unique(("quick-switch", &peer.id)), label, Look::Primary, !busy, Some(&name)).clicked() {
            self.busy.push(job.clone());
            if let Job::Switch { to, name } = job {
                self.link.switch(&to, &name);
            }
        }
        let half = (c.width() - 8.0) / 2.0;
        let send = Rect::from_min_size(pos2(c.left(), b.bottom() + 8.0), vec2(half, 36.0));
        let get = Rect::from_min_size(pos2(send.right() + 8.0, send.top()), vec2(half, 36.0));
        if widgets::content_button(ui, send, Id::unique(("send-files", &peer.id)), "Send files", Look::Quiet, self.desk_ui.picker.is_none(), Some(&format!("Send files to {}. Choose files, or drop them on it.", peer.name))).clicked() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.desk_ui.picker = Some((peer.id.clone(), rx));
            let ctx = ui.ctx().clone();
            let title = format!("Send files to {}", peer.name);
            std::thread::spawn(move || {
                let _ = tx.send(rfd::FileDialog::new().set_title(title).pick_files());
                ctx.request_repaint();
            });
        }
        let job = Job::Clip { send: false, id: peer.id.clone(), name: peer.name.clone() };
        let busy = self.busy.contains(&job);
        if widgets::content_button(ui, get, Id::unique(("quick-clip", false, &peer.id)), "Get clipboard", Look::Quiet, !busy, Some(&format!("Get clipboard: get {}'s clipboard", peer.name))).clicked() {
            self.busy.push(job);
            self.link.clip(false, &peer.id, &peer.name);
        }
        send.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h))
    }

    #[test]
    fn the_panel_widens_with_the_window() {
        assert_eq!(panel_width(1100.0), 310.0);
        assert_eq!(panel_width(1279.0), 310.0);
        assert_eq!(panel_width(1280.0), 350.0);
        assert_eq!(panel_width(1699.0), 350.0);
        assert_eq!(panel_width(1700.0), 380.0);
        assert_eq!(panel_width(1920.0), 380.0);
    }

    #[test]
    fn the_panel_sits_right_with_a_16_point_margin() {
        for (w, h) in [(1100.0, 720.0), (1440.0, 900.0), (1920.0, 1200.0)] {
            let l = desk_layout(window(w, h), 60.0);
            assert!(!l.stacked);
            assert_eq!(l.panel.right(), w - 16.0);
            assert_eq!(l.panel.top(), 76.0);
            assert_eq!(l.panel.bottom(), h - 16.0);
            assert_eq!(l.panel.width(), panel_width(w));
            assert_eq!(l.scene.right(), l.panel.left());
            assert_eq!(l.scene.top(), 60.0);
        }
    }

    #[test]
    fn a_narrow_window_puts_the_panel_under_the_desk() {
        for (w, h) in [(820.0, 540.0), (999.0, 700.0)] {
            let l = desk_layout(window(w, h), 60.0);
            assert!(l.stacked);
            assert_eq!(l.panel.height(), STACKED_H);
            assert_eq!((l.panel.left(), l.panel.right()), (16.0, w - 16.0));
            assert_eq!(l.panel.bottom(), h - 16.0);
            assert!(l.scene.bottom() <= l.panel.top());
            assert_eq!(l.scene.width(), w);
        }
        assert!(!desk_layout(window(1000.0, 700.0), 60.0).stacked);
    }

    #[test]
    fn the_desk_scale_follows_the_handoff() {
        assert!((desk_scale(1010.0, 440.0) - 1.0).abs() < 1e-6);
        assert!((desk_scale(1074.0, 840.0) - 1074.0 / 1010.0).abs() < 1e-6);
        assert!((desk_scale(1524.0, 1140.0) - 1.45).abs() < 1e-6, "capped at 1.45");
        assert!((desk_scale(820.0, 204.0) - 0.5).abs() < 1e-6, "at least 0.5");
        assert!((desk_scale(900.0, 300.0) - 300.0 / 440.0).abs() < 1e-6, "height bound");
    }

    #[test]
    fn the_stage_is_centred_and_stays_in_the_free_room() {
        let scene = Rect::from_min_max(pos2(0.0, 60.0), pos2(1074.0, 900.0));
        let free = Rect::from_min_max(pos2(16.0, 124.0), pos2(1058.0, 860.0));
        let s = desk_stage(scene, free);
        assert!((s.center().x - scene.center().x).abs() < 0.5);
        assert!((s.height() - 396.0 * desk_scale(1074.0, 840.0)).abs() < 0.5);
        assert!(s.width() >= 964.0 * desk_scale(1074.0, 840.0) - 0.5);
        assert!(free.contains_rect(s));
        // Little room (820 × 540): pushed below the toolbar, cut to what is free.
        let scene = Rect::from_min_max(pos2(0.0, 60.0), pos2(820.0, 260.0));
        let free = Rect::from_min_max(pos2(16.0, 124.0), pos2(804.0, 220.0));
        let s = desk_stage(scene, free);
        assert!(free.contains_rect(s.shrink(0.01)), "{s:?}");
        assert!(s.height() <= 96.0 + 0.01);
    }

    #[test]
    fn the_words_say_what_the_handoff_says() {
        assert_eq!(HINT, "Click a computer · drag it to where it sits · drop files on it to send");
        assert_eq!(OFFLINE, "Not found on the network. Start oneMK on it, on the same network as this computer. Its place on the desk is kept.");
        assert_eq!(latency_word(Some(Quality::Smooth)), "ms · smooth");
        assert_eq!(latency_word(Some(Quality::Stalls)), "ms · some stalls");
        assert_eq!(me_words("Linux", 2), "This computer · Linux · 2 screens");
        assert_eq!(me_words("macOS", 1), "This computer · macOS · 1 screen");
        assert_eq!(changed_words(0), "None");
        assert_eq!(changed_words(2), "2 changed");
        assert_eq!(keys_label(one_mk_proto::os::MACOS), "Keys for this Mac");
        assert_eq!(keys_label(one_mk_proto::os::LINUX), "Keys for this PC");
    }
}
