//! The window's frame: a glass sidebar on the left (in a narrow window, a
//! rail of icons), the page to its right with its title on top. The
//! sidebar holds what is true on every page: where to go, which computers
//! there are and how they answer, where the pointer is, and the one
//! action that matters most right now.

use super::*;
use crate::chrome;
use crate::icons::{self, Icon};

pub(super) const SIDE_W: f32 = 236.0;
pub(super) const RAIL_W: f32 = 52.0;
/// Narrower than this, the sidebar becomes a rail.
pub(super) const RAIL_BELOW: f32 = 1000.0;
/// The page title's row.
pub(super) const HEAD_H: f32 = 48.0;
const ROW_H: f32 = 34.0;

pub(super) struct Frame {
    pub side: Rect,
    pub content: Rect,
    pub rail: bool,
}

pub(super) fn frame(full: Rect) -> Frame {
    let rail = full.width() < RAIL_BELOW;
    // macOS: the window buttons sit in the sidebar's top corner, beside the
    // logo; a rail is too narrow for them, so it starts below them.
    let top = M.max(chrome::page_top());
    let side_top = if rail { top } else { M };
    let side = Rect::from_min_size(full.min + vec2(M, side_top), vec2(if rail { RAIL_W } else { SIDE_W }, (full.height() - side_top - M).max(0.0)));
    let content = Rect::from_min_max(pos2(side.right() + M, full.top() + top), pos2(full.right() - M, full.bottom() - M));
    Frame { side, content, rail }
}

const NAV: [(Tab, Icon); 6] =
    [(Tab::Desk, Icon::Desk), (Tab::Layout, Icon::Layout), (Tab::Files, Icon::Clipboard), (Tab::Keys, Icon::Keys), (Tab::Health, Icon::Health), (Tab::Advanced, Icon::Advanced)];

/// How tall `text` sets in the sidebar's small paragraphs.
fn para_h(ui: &Ui, text: &str, w: f32) -> f32 {
    let f = regular(T_META);
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), f.clone(), INK, w);
    job.sections[0].format.line_height = Some((f.size * LEADING).round());
    ui.painter().layout_job(job).size().y
}

/// A row that can be clicked: its hover and press state, focus ring and
/// name for screen readers.
fn row_button(ui: &mut Ui, rect: Rect, id: Id, name: &str, role: Role, chosen: bool, enabled: bool) -> (egui::Response, f32) {
    let r = ui.interact(rect, id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let p = ui.painter();
    if chosen {
        p.rect_filled(rect, CornerRadius::same(8), ON_GLASS_LIT);
        p.hline(rect.x_range().shrink(8.0), rect.top() + 0.5, Stroke::new(1.0, Color32::from_white_alpha(22)));
    } else if hover > 0.01 {
        p.rect_filled(rect, CornerRadius::same(8), ON_GLASS.gamma_multiply(hover));
    }
    widgets::focus_ring(ui, &r, rect, 8);
    if r.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    widgets::a11y(&r, role, name, Some(chosen));
    (r, hover)
}

impl App {
    /// Where the pointer is now: this computer ("me") or a paired one's id.
    pub(super) fn pointer_on(&self) -> Option<String> {
        let s = self.running()?;
        Some(match &s.controlling {
            Some(name) => s.peers.iter().find(|p| &p.name == name).map_or("me".to_owned(), |p| p.id.clone()),
            None => "me".to_owned(),
        })
    }

    /// Something on Health needs attention.
    fn health_trouble(&self) -> bool {
        self.running().is_some_and(|s| !s.input.is_empty())
            || self.doctor.as_ref().is_some_and(|(d, _)| d.as_ref().is_ok_and(|c| c.iter().any(|c| c.verdict == crate::view::Verdict::Fail)))
    }

    /// The top-bar action, done.
    pub(super) fn do_action(&mut self, job: Option<Job>, now: f64) {
        match job {
            Some(Job::Start) => {
                self.busy.push(Job::Start);
                self.link.start();
            }
            Some(Job::TakeBack) => {
                self.busy.push(Job::TakeBack);
                self.link.take_back();
            }
            Some(Job::Broadcast(on)) => self.ask_broadcast(on),
            _ => self.start_pairing(now),
        }
    }

    pub(super) fn sidebar(&mut self, ui: &mut Ui, f: &Frame, now: f64) {
        widgets::glass_panel(ui, "sidebar", f.side, R_PANEL);
        let mut ui = ui.new_child(UiBuilder::new().max_rect(f.side).id_salt("sidebar"));
        ui.set_clip_rect(f.side);
        if f.rail { self.rail(&mut ui, f.side, now) } else { self.side_full(&mut ui, f.side, now) }
    }

    fn nav_enabled(&self, tab: Tab) -> bool {
        !self.off() || tab.when_off()
    }

    fn side_full(&mut self, ui: &mut Ui, side: Rect, now: f64) {
        let x = side.left() + 10.0;
        let w = side.width() - 20.0;
        let mut y = side.top() + 18.0;
        // On macOS the window buttons come first on this row (chrome.rs).
        let (logo_x, logo_y) = if chrome::mac_frame() { (chrome::BUTTONS_X + chrome::BUTTONS_W - 4.0, chrome::BUTTONS_Y.center()) } else { (x + 10.0, y + 12.0) };
        draw_lockup(ui.painter(), &self.lockup, pos2(logo_x, logo_y), 22.0);
        y += 46.0;
        let trouble = self.health_trouble();
        for (tab, icon) in NAV {
            let r = Rect::from_min_size(pos2(x, y), vec2(w, ROW_H));
            let name = TABS.iter().find(|t| t.0 == tab).map_or("", |t| t.1);
            let enabled = self.nav_enabled(tab);
            let (resp, hover) = row_button(ui, r, Id::unique(("nav", name)), name, Role::Tab, self.tab == tab, enabled);
            let on = self.tab == tab;
            let col = if !enabled { mix(INK3, GLASS_SOLID, 0.45) } else if on { INK } else { mix(INK2, INK, hover) };
            icons::draw(ui.painter(), icon, Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(16.0, 16.0)), if on { INK } else if enabled { INK3 } else { col });
            ui.painter().text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, name, regular(T_BODY), col);
            if tab == Tab::Health && trouble {
                ui.painter().circle_filled(pos2(r.right() - 14.0, r.center().y), 3.5, ERROR);
            }
            if resp.clicked() {
                self.set_tab(tab);
            }
            y += ROW_H + 2.0;
        }
        // Computers: this one, then the paired ones, as many as fit above
        // the footer; the last row says how many more when they don't.
        let list_bottom = side.bottom() - self.foot_h(ui, w) - 8.0;
        if let Some(status) = self.running().cloned()
            && y + 14.0 + 22.0 + ROW_H <= list_bottom
        {
            let pointer = self.pointer_on();
            y += 14.0;
            ui.painter().text(pos2(x + 10.0, y + 8.0), Align2::LEFT_CENTER, "This computer", regular(T_META), INK3);
            y += 22.0;
            let r = Rect::from_min_size(pos2(x, y), vec2(w, ROW_H));
            let here = pointer.as_deref() == Some("me");
            self.computer_row(ui, r, "me", &status.name, here, true, if here { "pointer here" } else { "" }, false);
            y += ROW_H + 12.0;
            if y + 22.0 + ROW_H <= list_bottom {
                ui.painter().text(pos2(x + 10.0, y + 8.0), Align2::LEFT_CENTER, "Paired", regular(T_META), INK3);
                ui.painter().text(pos2(x + w - 10.0, y + 8.0), Align2::RIGHT_CENTER, status.peers.len().to_string(), mono(T_META), INK3);
                y += 22.0;
                if status.peers.is_empty() {
                    ui.painter().text(pos2(x + 10.0, y + ROW_H / 2.0), Align2::LEFT_CENTER, "None yet", regular(T_BODY), INK3);
                }
                let sel = self.selected_id().filter(|_| self.tab.desk());
                let n = status.peers.len();
                let fit = (((list_bottom - y + 2.0) / (ROW_H + 2.0)).floor().max(0.0)) as usize;
                let shown = if n > fit { fit.saturating_sub(1) } else { n };
                for p in status.peers.iter().take(shown) {
                    let r = Rect::from_min_size(pos2(x, y), vec2(w, ROW_H));
                    let value = match status.link_of(&p.id).filter(|_| p.online) {
                        Some(l) => rtt::ms(l.rtt_us),
                        None if p.online => String::new(),
                        None => "offline".to_owned(),
                    };
                    let here = pointer.as_deref() == Some(p.id.as_str());
                    self.computer_row(ui, r, &p.id, &p.name, here, p.online, &value, sel.as_deref() == Some(p.id.as_str()));
                    y += ROW_H + 2.0;
                }
                if n > shown && fit > 0 {
                    let r = Rect::from_min_size(pos2(x, y), vec2(w, ROW_H));
                    let more = format!("{} more on the desk", n - shown);
                    let (resp, _) = row_button(ui, r, Id::unique("side-more"), &more, Role::Button, false, true);
                    ui.painter().text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, &more, regular(T_BODY), INK2);
                    if resp.clicked() {
                        self.set_tab(Tab::Desk);
                    }
                }
            }
        }
        self.side_foot(ui, side, now);
    }

    /// A computer in the sidebar: the pointer if it is there, else whether
    /// it is connected; its name; how fast it answers or that it is off.
    /// Choosing it shows it on the desk.
    #[allow(clippy::too_many_arguments)]
    fn computer_row(&mut self, ui: &mut Ui, r: Rect, id: &str, name: &str, here: bool, online: bool, value: &str, chosen: bool) {
        let label = if value.is_empty() { name.to_owned() } else { format!("{name}, {value}") };
        let (resp, _) = row_button(ui, r, Id::unique(("pc", id)), &label, Role::Button, chosen, true);
        let p = ui.painter();
        let mark = pos2(r.left() + 18.0, r.center().y);
        if here {
            icons::pointer(p, mark - vec2(4.0, 7.0), 14.0, SIGNAL);
        } else if online {
            p.circle_filled(mark, 3.5, OK);
        } else {
            p.circle_stroke(mark, 3.0, Stroke::new(1.5, INK3));
        }
        let vw = if value.is_empty() { 0.0 } else { text_width(ui, value, if value.ends_with("ms") { mono(T_META) } else { regular(T_META) }) + 12.0 };
        let font = regular(T_BODY);
        let shown = fit(p, name, &font, r.width() - 46.0 - vw);
        p.text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, shown, font, if online || id == "me" { if chosen { INK } else { INK2 } } else { INK3 });
        if !value.is_empty() {
            let f = if value.ends_with("ms") { mono(T_META) } else { regular(T_META) };
            p.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, value, f, INK3);
        }
        if resp.clicked() {
            self.set_tab(Tab::Desk);
            self.select(id.to_owned());
        }
    }

    /// The update box's height: a title, a line, and a button or a bar.
    fn update_h(&self, ui: &Ui, w: f32) -> f32 {
        match self.update_box() {
            None => 0.0,
            Some(super::updates::UpdateBox::Ready { can: false, advice, .. }) => 44.0 + para_h(ui, &advice, w - 28.0) + 46.0,
            Some(super::updates::UpdateBox::Failed { said }) => 44.0 + para_h(ui, &said, w - 28.0).min(54.0) + 46.0,
            Some(_) => 44.0 + 22.0 + 46.0,
        }
    }

    fn foot_h(&self, ui: &Ui, w: f32) -> f32 {
        let u = self.update_h(ui, w);
        36.0 + 10.0 + 28.0 + 12.0 + if u > 0.0 { u + 10.0 } else { 0.0 }
    }

    /// The bottom of the sidebar: an update when there is one, the action
    /// that matters most, and whether the service runs (with Stop).
    fn side_foot(&mut self, ui: &mut Ui, side: Rect, now: f64) {
        use super::updates::UpdateBox;
        let x = side.left() + 10.0;
        let w = side.width() - 20.0;
        let mut y = side.bottom() - 12.0 - 28.0 - 10.0 - 36.0;
        if let Some(what) = self.update_box() {
            let h = self.update_h(ui, w);
            let r = Rect::from_min_size(pos2(x, y - h - 10.0), vec2(w, h));
            for s in widgets::glass_shapes_as(ui.ctx(), Id::unique("update-box").value(), r, f32::from(R_INNER) + 2.0, widgets::CONTROL_GLASS) {
                ui.painter().add(s);
            }
            let p = ui.painter().clone();
            let (tx, tw) = (r.left() + 14.0, w - 28.0);
            let line = |y: f32, text: &str, col: Color32| {
                p.text(pos2(tx, y), Align2::LEFT_CENTER, fit(&p, text, &regular(T_META), tw), regular(T_META), col);
            };
            let title = match &what {
                UpdateBox::Ready { version, .. } => format!("oneMK {version} is ready"),
                UpdateBox::MoveFirst => "Move to Applications".into(),
                UpdateBox::Working { .. } => "Updating oneMK".into(),
                UpdateBox::Failed { .. } => "The update didn't finish".into(),
            };
            p.text(pos2(tx, r.top() + 22.0), Align2::LEFT_CENTER, fit(&p, &title, &semibold(T_BODY), tw), semibold(T_BODY), INK);
            let b = Rect::from_min_size(pos2(r.left() + 10.0, r.bottom() - 42.0), vec2(w - 20.0, 32.0));
            let locked = self.sheet_open();
            match what {
                UpdateBox::Ready { version, can: true, .. } => {
                    line(r.top() + 44.0, "Restarts in a few seconds", INK2);
                    if button(ui, b, Id::unique("update"), "Update now", Look::Quiet, !locked, Some(&format!("Update oneMK to {version}"))).clicked() {
                        self.begin_update();
                    }
                }
                UpdateBox::Ready { can: false, advice, .. } => {
                    para(ui, pos2(tx, r.top() + 36.0), tw, &advice, regular(T_META), INK2);
                    if button(ui, b, Id::unique("update-download"), "Download", Look::Quiet, !locked, Some("Open the download page")).clicked() {
                        self.link.download_update();
                    }
                }
                UpdateBox::MoveFirst => {
                    line(r.top() + 44.0, "It's running from the disk image", INK2);
                    if button(ui, b, Id::unique("update-move"), "Move to Applications", Look::Quiet, !locked, None).clicked() {
                        self.update_ui.failed = false;
                        self.busy.push(Job::Update);
                        self.link.move_to_applications();
                    }
                }
                UpdateBox::Working { done, said } => {
                    line(r.top() + 44.0, &said, INK2);
                    // A capsule that fills; without a percentage, a short
                    // bead that travels along it.
                    let track = Rect::from_min_size(pos2(b.left() + 4.0, b.center().y - 3.0), vec2(b.width() - 8.0, 6.0));
                    p.rect_filled(track, CornerRadius::same(3), Color32::from_white_alpha(28));
                    let fill = match done {
                        Some(k) => Rect::from_min_size(track.min, vec2(track.width() * k.clamp(0.04, 1.0), track.height())),
                        None => {
                            let t = if motion::reduced() { 0.5 } else { ((now * 0.8).fract()) as f32 };
                            let bw = track.width() * 0.28;
                            Rect::from_min_size(pos2(track.left() + (track.width() - bw) * t, track.top()), vec2(bw, track.height()))
                        }
                    };
                    p.rect_filled(fill, CornerRadius::same(3), ACCENT);
                    ui.ctx().request_repaint_after_secs(1.0 / 30.0);
                    widgets::a11y_text(ui, Id::unique("update-progress"), r, Role::ProgressIndicator, &said, Some(Live::Polite));
                }
                UpdateBox::Failed { said } => {
                    let shown = if said.is_empty() { "Get the new version from the download page." } else { said.as_str() };
                    let mut job = egui::text::LayoutJob::simple(shown.to_owned(), regular(T_META), INK2, tw);
                    job.wrap.max_rows = 3;
                    let g = p.layout_job(job);
                    p.galley(pos2(tx, r.top() + 36.0), g, INK2);
                    if button(ui, b, Id::unique("update-download"), "Download", Look::Quiet, !locked, Some("Open the download page")).clicked() {
                        self.link.download_update();
                    }
                }
            }
        }
        let (label, look, enabled, job) = self.action();
        let a = Rect::from_min_size(pos2(x, y), vec2(w, 36.0));
        let shown = !self.sheet_open() && self.status.is_some();
        let a11y_label = match label.as_str() {
            "Take back" => Some("Take back control of this computer"),
            "Use keyboard to stop" | "Keyboard stop" => Some("Use the keyboard shortcut shown in the broadcast panel"),
            _ => None,
        };
        if button(ui, a, Id::unique("action"), &label, look, enabled && shown, a11y_label).clicked() {
            self.do_action(job, now);
        }
        y += 36.0 + 10.0;
        let row = Rect::from_min_size(pos2(x, y), vec2(w, 28.0));
        let p = ui.painter();
        let (dot, text) = match self.status.as_ref() {
            None => (INK3, "Looking for oneMK…"),
            Some(None) => (INK3, "oneMK is off"),
            Some(Some(_)) => (OK, "oneMK is running"),
        };
        p.circle_filled(pos2(row.left() + 14.0, row.center().y), 3.0, dot);
        p.text(pos2(row.left() + 26.0, row.center().y), Align2::LEFT_CENTER, text, regular(T_META), INK2);
        if self.running().is_some() {
            let busy = self.busy.contains(&Job::Stop);
            let label = if busy { "Stopping…" } else { "Stop" };
            let bw = text_width(ui, label, regular(T_META)) + 16.0;
            let b = Rect::from_min_size(pos2(row.right() - bw, row.top()), vec2(bw, 28.0));
            let (resp, hover) = row_button(ui, b, Id::unique("stop"), "Stop the sharing service, not just broadcast", Role::Button, false, !busy);
            ui.painter().text(b.center(), Align2::CENTER_CENTER, label, regular(T_META), mix(INK2, INK, hover));
            if resp.clicked() {
                self.busy.push(Job::Stop);
                self.link.stop();
            }
        }
    }

    /// The narrow window's sidebar: icons, one dot per computer, and the
    /// action as a round amber button. Names show on hover.
    fn rail(&mut self, ui: &mut Ui, side: Rect, now: f64) {
        let cx = side.center().x;
        let mut y = side.top() + 12.0;
        // The mark alone: the left part of the lockup, which is square-ish.
        let [tw, th] = self.lockup.size();
        let mark_w = 30.0;
        let frac = (th as f32 * 1.5 / tw as f32).min(1.0);
        let r = Rect::from_center_size(pos2(cx, y + 14.0), vec2(mark_w, mark_w * th as f32 / (tw as f32 * frac)));
        ui.painter().image(self.lockup.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(frac, 1.0)), Color32::WHITE);
        y += 40.0;
        let trouble = self.health_trouble();
        for (tab, icon) in NAV {
            let r = Rect::from_center_size(pos2(cx, y + 18.0), vec2(40.0, 36.0));
            let name = TABS.iter().find(|t| t.0 == tab).map_or("", |t| t.1);
            let enabled = self.nav_enabled(tab);
            let (resp, hover) = row_button(ui, r, Id::unique(("nav", name)), name, Role::Tab, self.tab == tab, enabled);
            let on = self.tab == tab;
            let col = if !enabled { mix(INK3, GLASS_SOLID, 0.45) } else if on { INK } else { mix(INK3, INK, hover) };
            icons::draw(ui.painter(), icon, Rect::from_center_size(r.center(), vec2(18.0, 18.0)), col);
            if tab == Tab::Health && trouble {
                ui.painter().circle_filled(r.right_top() + vec2(-8.0, 8.0), 3.0, ERROR);
            }
            if resp.on_hover_text(name).clicked() {
                self.set_tab(tab);
            }
            y += 38.0;
        }
        y += 6.0;
        ui.painter().hline((cx - 12.0)..=(cx + 12.0), y, Stroke::new(1.0, Color32::from_white_alpha(18)));
        y += 8.0;
        if let Some(status) = self.running().cloned() {
            let pointer = self.pointer_on();
            let sel = self.selected_id().filter(|_| self.tab.desk());
            let mut rows: Vec<(String, String, bool)> = vec![("me".into(), status.name.clone(), true)];
            rows.extend(status.peers.iter().map(|p| (p.id.clone(), p.name.clone(), p.online)));
            let extra = 36.0 * (f32::from(self.running().is_some()) + f32::from(self.update_available.is_some()));
            let bottom = side.bottom() - 12.0 - 40.0 - 8.0 - extra;
            for (id, name, online) in rows {
                if y + 32.0 > bottom {
                    break;
                }
                let r = Rect::from_center_size(pos2(cx, y + 15.0), vec2(40.0, 30.0));
                let (resp, _) = row_button(ui, r, Id::unique(("pc", id.as_str())), &name, Role::Button, sel.as_deref() == Some(id.as_str()), true);
                let initials: String = name.split(|c: char| c.is_whitespace() || c == '-' || c == '_').filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                let p = ui.painter();
                let mark = pos2(r.left() + 9.0, r.center().y);
                if pointer.as_deref() == Some(id.as_str()) {
                    icons::pointer(p, mark - vec2(3.0, 5.0), 10.0, SIGNAL);
                } else if online {
                    p.circle_filled(mark, 3.0, OK);
                } else {
                    p.circle_stroke(mark, 2.5, Stroke::new(1.5, INK3));
                }
                p.text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, initials, semibold(T_META), if online { INK2 } else { INK3 });
                if resp.on_hover_text(&name).clicked() {
                    self.set_tab(Tab::Desk);
                    self.select(id.clone());
                }
                y += 32.0;
            }
        }
        // Over the action: Update when there is one, and Stop.
        let mut by = side.bottom() - 12.0 - 40.0 - 8.0 - 18.0;
        if self.running().is_some() {
            let busy = self.busy.contains(&Job::Stop);
            let r = Rect::from_center_size(pos2(cx, by), vec2(40.0, 32.0));
            let name = if busy { "Stopping…" } else { "Stop the sharing service" };
            if widgets::icon_button(ui, r, Id::unique("stop"), name, !busy, |p, rr, col| icons::draw(p, Icon::Power, Rect::from_center_size(rr.center(), vec2(16.0, 16.0)), col)).clicked() {
                self.busy.push(Job::Stop);
                self.link.stop();
            }
            by -= 36.0;
        }
        if let Some(version) = self.update_available.clone() {
            let busy = self.busy.contains(&Job::Update);
            let r = Rect::from_center_size(pos2(cx, by), vec2(40.0, 32.0));
            // What the last attempt said, on hover, with the version.
            let name = match &self.update_output {
                Some(out) => format!("Update oneMK to {version}\n\n{}", out.trim()),
                None if busy => "Updating…".to_owned(),
                None => format!("Update oneMK to {version}"),
            };
            let resp = widgets::icon_button(ui, r, Id::unique("update"), &name, !busy && !self.sheet_open(), |p, rr, col| icons::draw(p, Icon::Update, Rect::from_center_size(rr.center(), vec2(16.0, 16.0)), col));
            ui.painter().circle_filled(r.right_top() + vec2(-9.0, 7.0), 3.0, SIGNAL);
            if resp.clicked() {
                self.update_output = None;
                self.busy.push(Job::Update);
                self.link.update();
            }
        }
        let (label, look, enabled, job) = self.action();
        let a = Rect::from_center_size(pos2(cx, side.bottom() - 12.0 - 20.0), vec2(40.0, 40.0));
        let shown = !self.sheet_open() && self.status.is_some();
        let r = button(ui, a, Id::unique("action"), "", look, enabled && shown, Some(&label));
        let col = if look == Look::Primary && enabled { SIGNAL_INK } else { INK };
        let icon = match job {
            Some(Job::Start) => Icon::Power,
            Some(Job::TakeBack) => Icon::Desk,
            Some(Job::Broadcast(_)) => Icon::Broadcast,
            _ if self.broadcasting() => Icon::Broadcast,
            _ if label.starts_with("Pair") => Icon::Plus,
            _ => Icon::Chevron,
        };
        icons::draw(ui.painter(), icon, Rect::from_center_size(a.center(), vec2(18.0, 18.0)), col);
        if r.on_hover_text(&label).clicked() {
            self.do_action(job, now);
        }
    }

    /// A page's title row: the title, a quieter word after it, and the
    /// room left on the right for the page's own actions.
    pub(super) fn page_head(&self, ui: &Ui, content: Rect, title: &str, sub: &str) -> Rect {
        let r = Rect::from_min_size(content.min + vec2(4.0, 0.0), vec2(content.width() - 4.0, HEAD_H));
        let p = ui.painter();
        // A firm title, and what is true now beside it, quieter and smaller,
        // on the same baseline.
        let big = semibold(24.0);
        let tw = text_width(ui, title, big.clone());
        let g = p.layout_no_wrap(title.to_owned(), big.clone(), INK);
        let top = r.center().y - g.size().y / 2.0;
        let baseline = top + g.rows.first().map_or(g.size().y * 0.8, |row| row.pos.y + row.row.glyphs.first().map_or(0.0, |gl| gl.pos.y));
        p.galley(pos2(r.left(), top), g, INK);
        if !sub.is_empty() {
            let small = regular(15.0);
            let room = (r.width() * 0.6 - tw - 14.0).max(0.0);
            let sg = p.layout_no_wrap(fit(p, sub, &small, room), small, INK3);
            let ascent = sg.rows.first().map_or(sg.size().y * 0.8, |row| row.pos.y + row.row.glyphs.first().map_or(0.0, |gl| gl.pos.y));
            p.galley(pos2(r.left() + tw + 12.0, baseline - ascent), sg, INK3);
        }
        a11y_text(ui, Id::unique(("page", title)), r, Role::Heading, &format!("{title} {sub}"), None);
        r
    }
}
