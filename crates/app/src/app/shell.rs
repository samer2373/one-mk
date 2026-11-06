//! The window's frame: a solid sidebar flush to the left edge (in a narrow
//! window, a rail of icons), and the page to its right with its title in
//! the band across the top, where the window is moved from. The sidebar
//! holds what is true on every page: where to go (each page with its
//! shortcut), which computers there are and how they answer, where the
//! pointer is, the one action that matters most right now, and whether
//! everything works (a status line that opens Health).

use super::*;
use crate::chrome;
use crate::icons::{self, Icon};
use unicode_segmentation::UnicodeSegmentation;

fn rail_name(ui: &Ui, name: &str, width: f32) -> String {
    let font = regular(T_META);
    if text_width(ui, name, font.clone()) <= width { return name.to_owned(); }
    let mut shown = String::new();
    for grapheme in name.graphemes(true) {
        if text_width(ui, &format!("{shown}{grapheme}…"), font.clone()) > width { break; }
        shown.push_str(grapheme);
    }
    format!("{shown}…")
}

fn rail_visible(total: usize, slots: usize) -> usize {
    if total > slots { slots.saturating_sub(1) } else { total }
}

fn computers_menu(response: &egui::Response, status: &Status) -> Option<String> {
    let mut selected = None;
    egui::Popup::menu(response).width(300.0).show(|ui| {
        for (id, name, online) in std::iter::once(("me", status.name.as_str(), true)).chain(status.peers.iter().map(|peer| (peer.id.as_str(), peer.name.as_str(), peer.online))) {
            let label = format!("{name}\n{}", if id == "me" { "This computer" } else if online { "Connected" } else { "Offline" });
            if ui.add(egui::Button::new(egui::RichText::new(label).font(regular(T_BODY))).wrap()).clicked() {
                selected = Some(id.to_owned());
                ui.close();
            }
        }
    });
    selected
}

pub(super) const SIDE_W: f32 = 232.0;
pub(super) const RAIL_W: f32 = 96.0;
/// Narrower than this, the sidebar becomes a rail.
pub(super) const RAIL_BELOW: f32 = 1000.0;
/// The band across the top: the page title (and on macOS the window
/// buttons, in the sidebar's part). The window is moved from it, so
/// nothing in it takes a click.
pub(super) const HEAD_H: f32 = 44.0;
const ROW_H: f32 = 32.0;

pub(super) struct Frame {
    pub side: Rect,
    pub content: Rect,
    pub rail: bool,
}

pub(super) fn frame(full: Rect) -> Frame {
    let rail = full.width() < RAIL_BELOW;
    let side = Rect::from_min_size(full.min, vec2(if rail { RAIL_W } else { SIDE_W }, full.height()));
    // The page's title starts after the macOS window buttons when a rail
    // is too narrow to hold them.
    let left = if rail && chrome::mac_frame() { (chrome::BUTTONS_X + chrome::BUTTONS_W + 8.0).max(side.right() + M) } else { side.right() + 2.0 * M };
    let content = Rect::from_min_max(pos2(left, full.top()), pos2(full.right() - M, full.bottom() - M));
    Frame { side, content, rail }
}

/// The pages in the sidebar, each with its shortcut key (with ⌘ or Ctrl).
/// Layout lives in Desk (its Arrange view), Health behind the status line
/// at the bottom, Advanced is Settings.
const NAV: [(Tab, Icon, &str); 6] = [(Tab::Desk, Icon::Desk, "1"), (Tab::Wallpaper, Icon::Wallpaper, "2"), (Tab::Files, Icon::Clipboard, "3"), (Tab::Keys, Icon::Keys, "4"), (Tab::Health, Icon::Health, "5"), (Tab::Advanced, Icon::Advanced, ",")];

/// The sidebar's page that `tab` belongs to.
fn nav_of(tab: Tab) -> Tab {
    match tab {
        Tab::Layout => Tab::Desk,
        t => t,
    }
}

/// The shortcut's modifier as this system writes it.
pub(super) fn cmd() -> &'static str {
    if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" }
}

/// How tall `text` sets in the sidebar's small paragraphs.
fn para_h(ui: &Ui, text: &str, w: f32) -> f32 {
    let f = regular(T_META);
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), f.clone(), INK, w);
    job.sections[0].format.line_height = Some((f.size * LEADING).round());
    ui.painter().layout_job(job).size().y
}

/// A row that can be clicked: its hover and press state, focus ring and
/// name for screen readers.
/// Rows that are plain actions (no chosen state): Stop, and "more".
fn id_kind(id: Id) -> &'static str {
    if id == Id::unique("stop") || id == Id::unique("side-more") { "plain" } else { "row" }
}

fn row_button(ui: &mut Ui, rect: Rect, id: Id, name: &str, role: Role, chosen: bool, enabled: bool) -> (egui::Response, f32) {
    let r = ui.interact(widgets::hit_rect(rect), id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.1, motion::out_cubic);
    let down = enabled && r.is_pointer_button_down_on();
    let p = ui.painter();
    if chosen {
        // Chosen: a lighter fill and a visible edge, a step above hover.
        let fill = mix(SIDE_CHOSEN, Color32::from_rgb(0x24, 0x28, 0x30), 0.5);
        p.rect(rect, CornerRadius::same(8), if down { widgets::pressed(fill, 1.0) } else { fill }, Stroke::new(1.0, Color32::from_white_alpha(22)), StrokeKind::Inside);
    } else if down {
        // Pressed: a deeper well than the hover, never the chosen look.
        // rest < hover < pressed < chosen (which adds its edge).
        p.rect_filled(rect, CornerRadius::same(8), SIDE_CHOSEN);
    } else if hover > 0.01 {
        // Hover is a hint, clearly weaker than the chosen page: half way
        // from the sidebar to the hover tone, no edge.
        p.rect_filled(rect, CornerRadius::same(8), mix(SIDEBAR, SIDE_HOVER, 0.25 * hover));
    }
    widgets::focus_ring(ui, &r, rect, 8);
    if r.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // Plain buttons and links have no chosen state (else they read as
    // toggle buttons); tabs and computer rows do.
    let state = (role != Role::Link && !matches!(id_kind(id), "plain")).then_some(chosen);
    widgets::a11y(&r, role, name, state);
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
        self.running().is_some_and(|s| s.input_lines().iter().any(|line| line.severity == one_mk_ctl::status::Severity::Problem))
            || self.health_checks().is_some_and(|checks| checks.iter().any(|check| matches!(check.verdict, crate::view::Verdict::Fail | crate::view::Verdict::Warn)))
    }

    fn health_tone(&self) -> Color32 {
        if self.running().is_some_and(|status| status.input_lines().iter().any(|line| line.severity == one_mk_ctl::status::Severity::Problem)) || self.health_checks().is_some_and(|checks| checks.iter().any(|check| check.verdict == crate::view::Verdict::Fail)) { ERROR } else { WARN }
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
        ui.painter().rect_filled(f.side, 0.0, SIDEBAR);
        ui.painter().vline(f.side.right() - 0.5, f.side.y_range(), Stroke::new(1.0, HAIRLINE));
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
        // The top band: the window buttons on macOS, the logo at its end.
        let band = HEAD_H / 2.0;
        // The mark's left edge lines up with the page icons below it.
        let lock_h = 26.0;
        let lock_w = lockup_ink_w(lock_h);
        let logo_x = if chrome::mac_frame() { side.right() - 18.0 - lock_w } else { x + 10.0 };
        draw_lockup(ui.painter(), &self.lockup, pos2(logo_x, band), lock_h);
        let mut y = HEAD_H + 6.0;
        // The way to everything: the command palette.
        let sr = Rect::from_min_size(pos2(x, y), vec2(w, 30.0));
        let resp = ui.interact(sr, Id::unique("search"), widgets::sense(ui, Sense::click()));
        let hover = tween(ui.ctx(), Id::unique("search-hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
        let fill = if resp.is_pointer_button_down_on() { widgets::pressed(SIDE_HOVER, 1.0) } else { mix(SEARCH_FILL, SIDE_HOVER, hover) };
        ui.painter().rect(sr, CornerRadius::same(8), fill, Stroke::new(1.0, SIDE_CHOSEN_EDGE), StrokeKind::Inside);
        icons::search(ui.painter(), pos2(sr.left() + 16.0, sr.center().y), INK3);
        ui.painter().text(pos2(sr.left() + 30.0, sr.center().y), Align2::LEFT_CENTER, "Search", regular(T_BODY), INK3);
        let kc = format!("{}K", cmd());
        let kw = text_width(ui, &kc, regular(T_META)) + 10.0;
        let kr = Rect::from_min_size(pos2(sr.right() - 8.0 - kw, sr.center().y - 9.0), vec2(kw, 18.0));
        ui.painter().rect(kr, CornerRadius::same(5), Color32::from_white_alpha(10), Stroke::new(0.5, Color32::from_white_alpha(36)), StrokeKind::Inside);
        ui.painter().text(kr.center(), Align2::CENTER_CENTER, kc, regular(T_META), INK3);
        widgets::focus_ring(ui, &resp, sr, 8);
        // A button that opens the command palette (where the typing
        // happens, in a real text field): not a text field itself.
        widgets::a11y(&resp, Role::Button, "Search: open the command palette", None);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.open_palette(now);
        }
        y += 30.0 + 12.0;
        for (tab, icon, key) in NAV {
            let r = Rect::from_min_size(pos2(x, y), vec2(w, ROW_H));
            let name = TABS.iter().find(|t| t.0 == tab).map_or("", |t| t.1);
            let enabled = self.nav_enabled(tab);
            let on = nav_of(self.tab) == tab;
            let (resp, hover) = row_button(ui, r, Id::unique(("nav", name)), name, Role::Tab, on, enabled);
            let col = if !enabled { INK3.gamma_multiply(0.6) } else if on { INK } else { mix(INK2, INK, hover) };
            icons::draw(ui.painter(), icon, Rect::from_center_size(pos2(r.left() + 17.0, r.center().y), vec2(15.0, 15.0)), col);
            ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, name, regular(T_BODY), col);
            ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, format!("{}{key}", cmd()), mono(T_META), if on { INK3 } else { SHORTCUT_INK });
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
            ui.painter().text(pos2(x + 10.0, y + 8.0), Align2::LEFT_CENTER, "This computer", semibold(T_META), INK3);
            y += 22.0;
            let r = Rect::from_min_size(pos2(x, y), vec2(w, ROW_H));
            let here = pointer.as_deref() == Some("me");
            self.computer_row(ui, r, "me", &status.name, here, true, "", false);
            y += ROW_H + 12.0;
            if y + 22.0 + ROW_H <= list_bottom {
                ui.painter().text(pos2(x + 10.0, y + 8.0), Align2::LEFT_CENTER, "Paired", semibold(T_META), INK3);
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
                    // The same reading the card shows (its newest), so the
                    // two never disagree; the daemon's last link otherwise.
                    let value = match status.link_of(&p.id).filter(|_| p.online) {
                        Some(l) => rtt::ms(self.summary_of(&p.id).map_or(l.rtt_us, |s| s.now_us)),
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
        // A Mac's host name ends in ".local", which says nothing here.
        let name = name.strip_suffix(".local").unwrap_or(name);
        // The pointer mark shows where the pointer is; screen readers hear it.
        let label = [Some(name), here.then_some("pointer here"), (!value.is_empty()).then_some(value)].into_iter().flatten().collect::<Vec<_>>().join(", ");
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
                UpdateBox::Ready { version, can: true, password, .. } => {
                    line(r.top() + 44.0, if password { "Asks for your password, then restarts" } else { "Restarts in a few seconds" }, INK2);
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
        let first_pair = self.tab == Tab::Desk && self.running().is_some_and(|status| status.peers.is_empty());
        let banner_take_back = self.tab == Tab::Desk && matches!(job, Some(Job::TakeBack));
        if self.broadcasting() {
            self.broadcast_status(ui, a);
        } else if !first_pair && !banner_take_back && button(ui, a, Id::unique("action"), &label, look, enabled && shown, a11y_label).clicked() {
            self.do_action(job, now);
        }
        y += 36.0 + 10.0;
        // Whether everything works, in a word, and the way to Health.
        let row = Rect::from_min_size(pos2(x, y), vec2(w, 28.0));
        let trouble = self.health_trouble();
        let (dot, text) = match self.status.as_ref() {
            None => (INK3, "Looking for oneMK…".to_owned()),
            Some(None) => (INK3, "Sharing is off".to_owned()),
            Some(Some(_)) if trouble => (self.health_tone(), "On · check Health".to_owned()),
            // Only "all good" when a computer is actually connected.
            Some(Some(s)) if s.peers.is_empty() => (INK3, "On · not paired".to_owned()),
            Some(Some(s)) if !s.peers.iter().any(|p| p.online) => (INK3, "On · none connected".to_owned()),
            Some(Some(s)) if s.peers.iter().any(|peer| !peer.online) => (WARN, "On · some offline".to_owned()),
            Some(Some(_)) => (OK, "On · connected".to_owned()),
        };
        let stop_w = if self.running().is_some() { text_width(ui, "Stop", regular(T_META)) + 16.0 } else { 0.0 };
        let status_r = Rect::from_min_max(row.min, pos2(row.right() - stop_w - 4.0, row.bottom()));
        let (resp, hover) = row_button(ui, status_r, Id::unique("health-line"), &format!("{text}. Open Health"), Role::Link, self.tab == Tab::Health, true);
        let p = ui.painter();
        p.circle_filled(pos2(status_r.left() + 12.0, row.center().y), 3.0, dot);
        p.text(pos2(status_r.left() + 24.0, row.center().y), Align2::LEFT_CENTER, fit(p, &text, &regular(T_META), status_r.width() - 30.0), regular(T_META), mix(INK2, INK, hover));
        if resp.clicked() {
            self.set_tab(Tab::Health);
        }
        if self.running().is_some() {
            let busy = self.busy.contains(&Job::Stop);
            let label = if busy { "Stopping…" } else { "Stop" };
            let bw = text_width(ui, label, regular(T_META)) + 16.0;
            let b = Rect::from_min_size(pos2(row.right() - bw, row.top()), vec2(bw, 28.0));
            let (resp, hover) = row_button(ui, b, Id::unique("stop"), "Stop oneMK: turn off sharing on this computer", Role::Button, false, !busy);
            ui.painter().text(b.center(), Align2::CENTER_CENTER, label, regular(T_META), mix(INK3, INK, hover));
            if resp.clicked() {
                self.busy.push(Job::Stop);
                self.link.stop();
            }
        }
    }

    /// The narrow window's sidebar: icons, one dot per computer, and the
    /// action as a round Volt button. Names show on hover.
    fn rail(&mut self, ui: &mut Ui, side: Rect, now: f64) {
        let cx = side.center().x;
        // On macOS the window buttons take the top; elsewhere the mark
        // stands there alone.
        let mut y = if chrome::mac_frame() {
            chrome::BUTTONS_Y.max + 6.0
        } else {
            let (h, c) = (20.0, pos2(cx, side.top() + 26.0));
            icons::pointer(ui.painter(), c - vec2(67.5, 62.0) * (h / 127.0), h, SIGNAL);
            side.top() + 52.0
        };
        let trouble = self.health_trouble();
        for (tab, icon, _) in NAV {
            let r = Rect::from_center_size(pos2(cx, y + 18.0), vec2(40.0, 36.0));
            let name = TABS.iter().find(|t| t.0 == tab).map_or("", |t| t.1);
            let enabled = self.nav_enabled(tab);
            let (resp, hover) = row_button(ui, r, Id::unique(("nav", name)), name, Role::Tab, self.tab == tab, enabled);
            let on = self.tab == tab;
            let col = if !enabled { mix(INK3, GLASS_SOLID, 0.45) } else if on { INK } else { mix(INK3, INK, hover) };
            icons::draw(ui.painter(), icon, Rect::from_center_size(r.center(), vec2(18.0, 18.0)), col);
            if tab == Tab::Health && trouble {
                ui.painter().circle_filled(r.right_top() + vec2(-8.0, 8.0), 3.0, self.health_tone());
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
            let expander = Rect::from_center_size(pos2(cx, y + 14.0), vec2(side.width() - 12.0, 28.0));
            let response = button(ui, expander, Id::unique("rail-computers"), "PCs", Look::Quiet, true, Some("Select a computer: show full names and connection states"));
            let selected = computers_menu(&response, &status);
            if let Some(id) = selected { self.set_tab(Tab::Desk); self.select(id); }
            y += 32.0;
            let pointer = self.pointer_on();
            let sel = self.selected_id().filter(|_| self.tab.desk());
            let mut rows: Vec<(String, String, bool)> = vec![("me".into(), status.name.clone(), true)];
            rows.extend(status.peers.iter().map(|p| (p.id.clone(), p.name.clone(), p.online)));
            let extra = 36.0 * (f32::from(self.running().is_some()) + f32::from(self.update_available.is_some()));
            let bottom = side.bottom() - 12.0 - 40.0 - 8.0 - extra;
            let slots = ((bottom - y) / 32.0).floor().max(1.0) as usize;
            let visible = rail_visible(rows.len(), slots);
            let hidden = rows.len() - visible;
            for (id, name, online) in rows.into_iter().take(visible) {
                // A short name, not initials: as much of it as fits.
                let r = Rect::from_center_size(pos2(cx, y + 15.0), vec2(side.width() - 12.0, 30.0));
                let (resp, _) = row_button(ui, r, Id::unique(("pc", id.as_str())), &name, Role::Button, sel.as_deref() == Some(id.as_str()), true);
                let shown = rail_name(ui, &name, r.width() - 23.0);
                let p = ui.painter();
                let mark = pos2(r.left() + 9.0, r.center().y);
                if pointer.as_deref() == Some(id.as_str()) {
                    icons::pointer(p, mark - vec2(3.0, 5.0), 10.0, SIGNAL);
                } else if online {
                    p.circle_filled(mark, 3.0, OK);
                } else {
                    p.circle_stroke(mark, 2.5, Stroke::new(1.5, INK3));
                }
                p.text(pos2(r.left() + 19.0, r.center().y), Align2::LEFT_CENTER, shown, regular(T_META), if online { INK2 } else { INK3 });
                if resp.on_hover_text(&name).clicked() {
                    self.set_tab(Tab::Desk);
                    self.select(id.clone());
                }
                y += 32.0;
            }
            if hidden > 0 {
                let rect = Rect::from_min_size(pos2(side.left() + 6.0, y), vec2(side.width() - 12.0, 30.0));
                let response = button(ui, rect, Id::unique("rail-overflow"), &format!("+{hidden}"), Look::Quiet, true, Some("More computers: show all names and connection states"));
                if let Some(id) = computers_menu(&response, &status) { self.set_tab(Tab::Desk); self.select(id); }
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
        let a = Rect::from_center_size(pos2(cx, side.bottom() - 12.0 - 20.0), vec2(side.width() - 12.0, 40.0));
        if self.tab == Tab::Desk && self.running().is_some_and(|status| status.peers.is_empty()) { return; }
        if self.tab == Tab::Desk && matches!(job, Some(Job::TakeBack)) { return; }
        let shown = !self.sheet_open() && self.status.is_some();
        if self.broadcasting() { self.broadcast_status(ui, a); return; }
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

    fn broadcast_status(&self, ui: &Ui, rect: Rect) {
        let hint = self.broadcast_stop_hint();
        let shortcut = hint.strip_prefix("Press ").and_then(|text| text.split_once(" to stop").map(|(key, _)| key)).unwrap_or(&hint);
        let rect = if rect.width() < 100.0 { Rect::from_min_max(rect.min - vec2(0.0, 12.0), rect.max) } else { rect };
        ui.painter().text(rect.center_top() + vec2(0.0, 7.0), Align2::CENTER_TOP, "Keyboard stop", regular(12.0), INK2);
        if rect.width() < 100.0 && let Some((first, rest)) = shortcut.split_once("+Shift") {
            ui.painter().text(rect.center_top() + vec2(0.0, 22.0), Align2::CENTER_TOP, format!("{first}+"), regular(12.0), INK2);
            ui.painter().text(rect.center_bottom() - vec2(0.0, 2.0), Align2::CENTER_BOTTOM, format!("Shift{rest}"), regular(12.0), INK2);
        } else {
            ui.painter().text(rect.center_bottom() - vec2(0.0, 2.0), Align2::CENTER_BOTTOM, shortcut, regular(12.0), INK2);
        }
        a11y_text(ui, Id::unique("broadcast-stop-status"), rect, Role::Label, &format!("Keyboard stop. {hint}"), None);
    }

    /// A page's title row: the title, a quieter word after it, and the
    /// room left on the right for the page's own actions.
    pub(super) fn page_head(&self, ui: &Ui, content: Rect, title: &str, sub: &str) -> Rect {
        let r = Rect::from_min_size(content.min + vec2(4.0, 0.0), vec2(content.width() - 4.0, HEAD_H));
        let p = ui.painter();
        // Compact, as a tool's title bar: the page, and what is true now
        // beside it, quieter.
        let big = semibold(T_TITLE);
        let tw = text_width(ui, title, big.clone());
        p.text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, title, big, INK);
        if !sub.is_empty() {
            let small = regular(T_BODY);
            let room = (r.width() * 0.6 - tw - 12.0).max(0.0);
            p.text(pos2(r.left() + tw + 10.0, r.center().y + 0.5), Align2::LEFT_CENTER, fit(p, sub, &small, room), small, INK3);
        }
        a11y_text(ui, Id::unique(("page", title)), r, Role::Heading, &format!("{title} {sub}"), None);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_rail_reserves_an_explicit_overflow_target() {
        for slots in 1..=6 {
            let visible = rail_visible(6, slots);
            let hidden = 6 - visible;
            assert_eq!(visible + hidden, 6);
            assert!(visible + usize::from(hidden > 0) <= slots);
        }
    }

    #[test]
    fn short_names_preserve_complete_graphemes() {
        let context = egui::Context::default();
        crate::style::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            for name in ["Samer's workstation", "Мастерская", "工作站", "e\u{301}cole 👩‍💻 studio"] {
                let shown = rail_name(ui, name, 61.0);
                assert!(text_width(ui, &shown, regular(T_META)) <= 61.0);
                let prefix = shown.strip_suffix('…').unwrap_or(&shown);
                assert!(name.grapheme_indices(true).any(|(end, _)| &name[..end] == prefix) || name == prefix);
            }
        });
        output.textures_delta.clear();
    }
}
