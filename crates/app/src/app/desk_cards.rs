//! The Desk's cards and what floats over the desk: the chosen computer
//! (where it sits, how fast it answers over the last two minutes, and what
//! can be done with it), this computer's own switches, the page's actions
//! for every computer at once, the running transfer and the hint that the
//! computers can be dragged.

use super::*;
use crate::icons::{self, Icon};
use crate::rtt::{Quality, Ring};

/// The row the Computers / Arrange switch takes at the desk's top.
pub(super) const DESK_VIEWS_H: f32 = 46.0;

/// Text with an icon before it, as a quiet button with no fill until
/// hovered; its width.
#[allow(clippy::too_many_arguments)]
fn ghost(ui: &mut Ui, at: Pos2, h: f32, id: Id, icon: Option<Icon>, label: &str, name: &str, enabled: bool) -> (egui::Response, f32) {
    let font = regular(T_BODY);
    let w = if label.is_empty() { h } else { text_width(ui, label, font.clone()) + if icon.is_some() { 34.0 } else { 18.0 } };
    let r = Rect::from_min_size(at, vec2(w, h));
    let resp = ui.interact(r, id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && resp.hovered()), 0.12, motion::out_cubic);
    let p = ui.painter();
    if hover > 0.01 {
        p.rect_filled(r, CornerRadius::same(8), ON_GLASS.gamma_multiply(hover));
    }
    let col = if enabled { mix(INK2, INK, hover) } else { INK3 };
    let mut x = r.left() + if label.is_empty() { (w - 16.0) / 2.0 } else { 9.0 };
    if let Some(i) = icon {
        icons::draw(p, i, Rect::from_min_size(pos2(x, r.center().y - 8.0), vec2(16.0, 16.0)), if enabled { mix(INK3, INK, hover) } else { INK3 });
        x += 22.0;
    }
    if !label.is_empty() {
        p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, font, col);
    }
    widgets::focus_ring(ui, &resp, r, 8);
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    widgets::a11y(&resp, Role::Button, name, None);
    (resp, w)
}

/// A smooth line through the readings: each gap between two readings is
/// drawn as a short curve (Catmull-Rom), so the line has no corners.
fn smooth(pts: &[Pos2]) -> Vec<Pos2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len() * 4);
    for i in 0..pts.len() - 1 {
        let (p0, p1, p2, p3) = (pts[i.saturating_sub(1)], pts[i], pts[i + 1], pts[(i + 2).min(pts.len() - 1)]);
        for k in 0..4 {
            let t = k as f32 / 4.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3);
            out.push(pos2(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
        }
    }
    out.push(*pts.last().expect("three at least"));
    out
}

/// The round trip over the last two minutes in `r`: faint lines at 0 and
/// the top with their values, the line, the newest reading in Volt, the
/// highest one named when it stands out, and lost packets marked.
fn chart(p: &Painter, r: Rect, ring: &Ring, now: f64, summary: Option<rtt::Summary>) {
    let left = 30.0;
    let plot = Rect::from_min_max(pos2(r.left() + left, r.top() + 8.0), pos2(r.right() - 6.0, r.bottom() - 20.0));
    let t = ring.trace(now, plot.width(), plot.height());
    let grid = Stroke::new(1.0, Color32::from_white_alpha(14));
    for (frac, us) in [(0.0, 0), (1.0, t.top_us)] {
        let y = plot.bottom() - frac * plot.height();
        p.hline(plot.x_range(), y, grid);
        let label = if us == 0 { "0".to_owned() } else { rtt::ms_number(us) };
        p.text(pos2(plot.left() - 8.0, y), Align2::RIGHT_CENTER, label, mono(T_META), INK3);
    }
    p.text(pos2(plot.left(), r.bottom() - 6.0), Align2::LEFT_CENTER, format!("{} ago", rtt::span_words()), regular(T_META), INK3);
    p.text(pos2(plot.right(), r.bottom() - 6.0), Align2::RIGHT_CENTER, "now", regular(T_META), INK3);
    for line in &t.lines {
        let pts: Vec<Pos2> = line.iter().map(|[x, y]| plot.min + vec2(*x, *y)).collect();
        match pts[..] {
            [] => {}
            [one] => {
                p.circle_filled(one, 1.5, INK);
            }
            _ => {
                p.add(egui::Shape::line(smooth(&pts), Stroke::new(1.6, INK)));
            }
        }
    }
    // The highest reading, named, when it is well above the usual.
    if let Some(s) = summary.filter(|s| s.max_us > s.avg_us.saturating_mul(3) / 2 && s.max_us >= 2_000)
        && let Some(peak) = t.lines.iter().flatten().min_by(|a, b| a[1].total_cmp(&b[1]))
    {
        let at = plot.min + vec2(peak[0], peak[1]);
        p.circle_filled(at, 2.5, INK);
        let right = at.x + 60.0 < plot.right();
        p.text(at + vec2(if right { 8.0 } else { -8.0 }, 2.0), if right { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER }, rtt::ms(s.max_us), mono(T_META), INK2);
    }
    if let Some([x, y]) = t.lines.last().and_then(|l| l.last()) {
        let at = plot.min + vec2(*x, *y);
        p.circle_filled(at, 4.5, GROUND);
        p.circle_filled(at, 3.5, SIGNAL);
    }
    // Lost packets: a short amber tick from the top, with a ring, so they
    // read apart from a slow reading and not only by colour.
    for x in &t.losses {
        let x = plot.left() + x;
        p.line_segment([pos2(x, plot.top() + 4.0), pos2(x, plot.bottom())], Stroke::new(1.0, WARN.gamma_multiply(0.5)));
        p.circle_stroke(pos2(x, plot.top() + 2.5), 2.5, Stroke::new(1.0, WARN));
    }
}

impl App {
    /// The chosen computer's card, or this computer's when none is chosen.
    pub(super) fn peer_card(&mut self, ui: &mut Ui, c: Rect, sel: Option<String>, now: f64) {
        let Some(status) = self.running().cloned() else { return };
        let Some(peer) = status.peers.iter().find(|p| Some(&p.id) == sel.as_ref()).cloned() else {
            self.me_card(ui, c, &status);
            return;
        };
        let p = ui.painter().clone();
        // Title row: whether it is there, its name, what it is and where.
        let cy = c.top() + 12.0;
        if peer.online {
            p.circle_filled(pos2(c.left() + 4.0, cy), 3.5, OK);
        } else {
            p.circle_stroke(pos2(c.left() + 4.0, cy), 3.0, Stroke::new(1.5, INK3));
        }
        let side = self.side_shown(&peer.id, peer.side);
        let seg_w = if c.width() > 560.0 { 248.0 } else { 204.0 };
        let col_w = 200.0f32.min(c.width() * 0.4);
        let name = fit(&p, &peer.name, &semibold(T_TITLE), col_w - 16.0);
        p.text(pos2(c.left() + 16.0, cy), Align2::LEFT_CENTER, &name, semibold(T_TITLE), INK);
        if name != peer.name {
            ui.interact(Rect::from_min_size(c.min, vec2(col_w, 24.0)), Id::unique(("peer-name", &peer.id)), Sense::hover()).on_hover_text(&peer.name);
        }
        let words = format!("{} · {}", os_name(peer.os), if peer.online { on_your(side) } else { "offline" });
        p.text(pos2(c.left(), cy + 22.0), Align2::LEFT_CENTER, fit(&p, &words, &regular(T_BODY), col_w), regular(T_BODY), INK3);
        // Where it sits: a control, so it can be set without dragging.
        let mut x = c.right();
        let (rm, rw) = {
            let w = text_width(ui, "Remove…", regular(T_BODY)) + 18.0;
            (Rect::from_min_size(pos2(x - w + 9.0, cy - 14.0), vec2(w, 28.0)), w)
        };
        let (resp, _) = ghost(ui, rm.min, 28.0, Id::unique(("remove", &peer.id)), None, "Remove…", &format!("Remove {} (asks first)", peer.name), !self.sheet_open());
        if resp.clicked() {
            self.removing = Some((peer.id.clone(), peer.name.clone(), now));
        }
        x -= rw;
        let track = Rect::from_min_size(pos2(x - seg_w, cy - 14.0), vec2(seg_w, 28.0));
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
            font: T_BODY,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.choose_side(&peer.id, PLACES[k].0, true);
        }
        if track.left() - 40.0 > c.left() + col_w + 24.0 {
            p.text(pos2(track.left() - 8.0, cy), Align2::RIGHT_CENTER, "Sits", regular(T_META), INK3);
        }

        // Left: the number now, what it means; actions at the bottom.
        let body = Rect::from_min_max(pos2(c.left(), c.top() + 52.0), c.max);
        let summary = self.summary_of(&peer.id);
        let q = summary.and_then(|s| s.quality());
        let big = mono_medium(30.0);
        if !peer.online {
            p.text(pos2(body.left(), body.top() + 18.0), Align2::LEFT_CENTER, "—", big.clone(), INK3);
            p.text(pos2(body.left(), body.top() + 46.0), Align2::LEFT_CENTER, "Not found on the network", regular(T_BODY), INK2);
            // Two lines rather than cut short in a narrow column.
            let mut hint = egui::text::LayoutJob::simple("Start oneMK on it, on the same Wi-Fi.".into(), regular(T_META), INK3, col_w);
            hint.wrap.max_rows = 2;
            p.galley(pos2(body.left(), body.top() + 58.0), p.layout_job(hint), INK3);
        } else if let Some(s) = summary {
            let n = rtt::ms_number(s.now_us);
            let nw = text_width(ui, &n, big.clone());
            p.text(pos2(body.left(), body.top() + 18.0), Align2::LEFT_CENTER, &n, big, INK);
            p.text(pos2(body.left() + nw + 4.0, body.top() + 23.0), Align2::LEFT_CENTER, "ms", regular(T_BODY), INK2);
            let word = q.map_or("Measuring…", Quality::word);
            let ww = text_width(ui, word, regular(T_BODY));
            p.text(pos2(body.left(), body.top() + 48.0), Align2::LEFT_CENTER, word, regular(T_BODY), if matches!(q, Some(Quality::Stalls | Quality::Slow)) { WARN } else { INK2 });
            if q.is_some() {
                let (lo, hi) = (rtt::ms_number(s.min_us), rtt::ms(s.p95_us));
                let usual = if hi == format!("{lo} ms") { format!(" · usually {hi}") } else { format!(" · usually {lo}–{hi}") };
                p.text(pos2(body.left() + ww, body.top() + 48.0), Align2::LEFT_CENTER, fit(&p, &usual, &regular(T_BODY), col_w - ww), regular(T_BODY), INK3);
            }
        } else {
            let text = if self.older_daemon(&peer.id) { "Restart oneMK to see latency" } else { "Measuring…" };
            p.text(pos2(body.left(), body.top() + 18.0), Align2::LEFT_CENTER, "…", big, INK3);
            p.text(pos2(body.left(), body.top() + 48.0), Align2::LEFT_CENTER, fit(&p, text, &regular(T_BODY), col_w), regular(T_BODY), INK2);
        }
        if peer.online && (self.latency.contains(&peer.id) || self.failed_measure.contains(&peer.id)) {
            let busy = self.measuring.is_some();
            let r = Rect::from_min_size(pos2(body.left() + col_w - 28.0, body.top() + 4.0), vec2(28.0, 28.0));
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
        }
        self.peer_actions(ui, pos2(body.left(), body.bottom() - 32.0), &status, &peer);

        // Right: the last two minutes.
        let graph = Rect::from_min_max(pos2(body.left() + col_w + 24.0, c.top() + 40.0), body.max);
        p.text(pos2(graph.left() + 30.0, graph.top() + 6.0), Align2::LEFT_CENTER, format!("Round trip, last {}", rtt::span_words()), regular(T_META), INK3);
        let plot = Rect::from_min_max(pos2(graph.left(), graph.top() + 18.0), graph.max);
        match self.rtts.of(&peer.id).filter(|_| peer.online) {
            Some(ring) if summary.is_some() => chart(&p, plot, ring, self.rtt_now(), summary),
            _ => {
                let text = if !peer.online { "Nothing to measure while it is offline" } else { "The line starts with the first replies" };
                p.text(plot.center(), Align2::CENTER_CENTER, fit(&p, text, &regular(T_META), plot.width()), regular(T_META), INK3);
            }
        }
    }

    /// Switch to it, and its clipboard, along the bottom of the card.
    fn peer_actions(&mut self, ui: &mut Ui, at: Pos2, s: &Status, peer: &PeerStatus) {
        let mut x = at.x;
        if peer.online && s.controlling.as_ref() != Some(&peer.name) {
            let job = Job::Switch { to: peer.id.clone(), name: peer.name.clone() };
            let label = "Switch to it";
            let w = button_width(ui, label) - 4.0;
            let busy = self.busy.contains(&job);
            if button(ui, Rect::from_min_size(pos2(x, at.y), vec2(w, 32.0)), Id::unique(("quick-switch", &peer.id)), label, Look::Quiet, !busy, Some(&format!("Move the mouse and keyboard to {}", peer.name))).clicked() {
                self.busy.push(job.clone());
                self.link.switch(&peer.id, &peer.name);
            }
            x += w + 4.0;
        }
        if !peer.online {
            return;
        }
        for (send, icon, label) in [(true, Icon::Send, "Send"), (false, Icon::Get, "Get")] {
            let job = Job::Clip { send, id: peer.id.clone(), name: peer.name.clone() };
            let busy = self.busy.contains(&job);
            let name = if send { format!("Send this clipboard to {}", peer.name) } else { format!("Get {}'s clipboard", peer.name) };
            let (resp, w) = ghost(ui, pos2(x, at.y + 2.0), 28.0, Id::unique(("quick-clip", send, &peer.id)), Some(icon), label, &name, !busy);
            if resp.on_hover_text(&name).clicked() {
                self.busy.push(job.clone());
                self.link.clip(send, &peer.id, &peer.name);
            }
            x += w + 2.0;
        }
    }

    /// No other computer chosen: this one, and the way back to it.
    fn me_card(&mut self, ui: &mut Ui, c: Rect, status: &Status) {
        let p = ui.painter().clone();
        let cy = c.top() + 12.0;
        icons::pointer(&p, pos2(c.left(), cy - 7.0), 14.0, SIGNAL);
        let nw = text_width(ui, &status.name, semibold(T_TITLE));
        p.text(pos2(c.left() + 16.0, cy), Align2::LEFT_CENTER, &status.name, semibold(T_TITLE), INK);
        p.text(pos2(c.left() + 26.0 + nw, cy), Align2::LEFT_CENTER, format!("This computer · {}", os_name(me_os())), regular(T_BODY), INK3);
        let online = status.peers.iter().filter(|p| p.online).count();
        let paired = match status.peers.len() {
            0 => "None yet".to_owned(),
            n => format!("{online} of {n} connected"),
        };
        info_row(ui, c, c.top() + 52.0, "Paired", &paired, false, 1.0);
        info_row(ui, c, c.top() + 76.0, "ID", &status.id, true, 1.0);
        self.quick_actions(ui, c, c.bottom() - 32.0, None);
    }

    /// The page's actions for every computer at once, at the right of its
    /// title: typing on all of them, and locking every screen.
    pub(super) fn desk_actions(&mut self, ui: &mut Ui, head: Rect) {
        // The Desk's two views: the computers, and arranging them (Layout).
        if self.desk_views_shown() {
            let labels = ["Computers", "Arrange"];
            let track = Rect::from_min_size(pos2(head.left(), head.center().y - 15.0), vec2(200.0, 30.0));
            let pills = Pills {
                id: Id::unique("desk-view"),
                track,
                cells: Pills::even(track, 2, 2.0),
                labels: &labels,
                names: vec!["Desk: your computers".into(), "Desk: arrange where each computer sits".into()],
                chosen: Some(usize::from(self.tab == Tab::Layout)),
                enabled: vec![true, true],
                role: Role::Tab,
                font: T_BODY,
            };
            if let Some(k) = pills.show(ui, self.arrowed) {
                self.set_tab(if k == 1 { Tab::Layout } else { Tab::Desk });
            }
        }
        if self.tab != Tab::Desk || self.running().is_none_or(|s| s.peers.is_empty()) {
            return;
        }
        let mut x = head.right();
        let y = head.center().y - 17.0;
        let narrow = head.width() < 780.0;
        let label = if narrow { "Lock all" } else { "Lock every screen" };
        let w = text_width(ui, label, semibold(T_BODY)) + 46.0;
        x -= w;
        let busy = self.busy.contains(&Job::LockAll);
        let r = Rect::from_min_size(pos2(x, y), vec2(w, 34.0));
        if button(ui, r, Id::unique("lock-all"), "", Look::Glass, !busy, Some("Lock this computer and every connected one")).clicked() {
            self.busy.push(Job::LockAll);
            self.link.lock_all();
        }
        icons::draw(ui.painter(), Icon::Lock, Rect::from_min_size(pos2(r.left() + 12.0, r.center().y - 8.0), vec2(16.0, 16.0)), INK2);
        ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, label, semibold(T_BODY), INK);
        let online = self.running().is_some_and(|s| s.peers.iter().any(|p| p.online));
        let on = self.broadcasting();
        if online || on {
            x -= 64.0 + 8.0;
            let rect = Rect::from_min_size(pos2(x, y), vec2(64.0, 34.0));
            let response = button(ui, rect, Id::unique("broadcast-options"), "Timer", Look::Glass, !on, Some("Broadcast idle timer"));
            egui::Popup::menu(&response).width(280.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                let (row, _) = ui.allocate_exact_size(vec2(280.0, 34.0), Sense::hover());
                self.broadcast_timeout(ui, row);
                ui.label("Use zero to keep broadcasting until you stop it.");
            });
        }
        if (online || on) && narrow {
            // Narrow: the same action as an icon, lit while it is on.
            x -= 34.0 + 8.0;
            let r = Rect::from_min_size(pos2(x, y), vec2(34.0, 34.0));
            let busy = self.busy.iter().any(|j| matches!(j, Job::Broadcast(_)));
            let key = if on { format!(" {}", self.broadcast_stop_hint()) } else { String::new() };
            let name = if on { format!("Typing on every computer.{key}") } else { "Type on every computer: everything you type, click and scroll here goes to every connected computer at once".to_owned() };
            let resp = button(ui, r, Id::unique("broadcast"), "", Look::Quiet, !busy && !on, Some(&name));
            if on {
                ui.painter().rect_stroke(r, CornerRadius::same(10), Stroke::new(1.5, SIGNAL), StrokeKind::Inside);
            }
            icons::draw(ui.painter(), Icon::Broadcast, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), if on { SIGNAL } else { INK2 });
            if resp.on_hover_text(&name).clicked() {
                self.ask_broadcast(!on);
            }
        }
        if (online || on) && !narrow {
            let label = "Type on every computer";
            let w = text_width(ui, label, semibold(T_BODY)) + 34.0 + 58.0;
            x -= w + 8.0;
            let r = Rect::from_min_size(pos2(x, y), vec2(w, 34.0));
            let busy = self.busy.iter().any(|j| matches!(j, Job::Broadcast(_)));
            let key = if on { format!(" {}", self.broadcast_stop_hint()) } else { String::new() };
            let resp = button(ui, r, Id::unique("broadcast"), "", Look::Quiet, !busy && !on, Some("Broadcast: type, click and scroll on every connected computer at once"));
            icons::draw(ui.painter(), Icon::Broadcast, Rect::from_min_size(pos2(r.left() + 12.0, r.center().y - 8.0), vec2(16.0, 16.0)), INK2);
            ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, label, semibold(T_BODY), INK);
            let track = Rect::from_min_size(pos2(r.right() - 46.0, r.center().y - 9.0), vec2(32.0, 18.0));
            ui.painter().rect_filled(track, CornerRadius::same(9), if on { INK } else { TRACK_OFF });
            ui.painter().circle_filled(pos2(if on { track.right() - 9.0 } else { track.left() + 9.0 }, track.center().y), 6.5, if on { GROUND } else { INK });
            if resp.on_hover_text(format!("Everything you type, click and scroll here goes to every connected computer at once.{key}")).clicked() {
                self.ask_broadcast(!on);
            }
        }
    }

    /// Over the desk: the transfer that is running, and the hint that
    /// computers can be dragged (until one has been).
    pub(super) fn desk_overlay(&mut self, ui: &mut Ui, desk: Rect, hovering_files: bool) {
        if self.tab != Tab::Desk && self.tab != Tab::Layout {
            return;
        }
        let p = ui.painter().clone();
        if hovering_files {
            let r = Rect::from_center_size(pos2(desk.center().x, desk.bottom() - 24.0), vec2(280.0, 40.0));
            widgets::glass_panel(ui, "drop-hint", r, 14);
            p.text(r.center(), Align2::CENTER_CENTER, "Drop on a computer to send", semibold(T_BODY), SIGNAL);
            return;
        }
        // What needs saying right now (broadcasting, being controlled,
        // waiting for permission...): a banner over the desk, read out.
        // Under the Computers / Arrange switch, which sits at the desk's top.
        let mut top = desk.top() + if self.desk_views_shown() { DESK_VIEWS_H } else { 0.0 };
        if let Some((title, sub)) = self.notice.shown().cloned() {
            let w = 560.0f32.min(desk.width());
            let job = |text: &str, font: egui::FontId, col: Color32| {
                let mut j = egui::text::LayoutJob::simple(text.to_owned(), font, col, w - 46.0);
                j.wrap.max_rows = 3;
                ui.painter().layout_job(j)
            };
            let (tg, sg) = (job(&title, semibold(T_BODY), INK), job(&sub, regular(T_META), INK2));
            let h = 14.0 + tg.size().y + 4.0 + sg.size().y + 14.0;
            let r = Rect::from_min_size(pos2(desk.left(), top), vec2(w, h));
            widgets::glass_panel(ui, "notice", r, 14);
            let title_h = tg.size().y;
            p.circle_filled(pos2(r.left() + 18.0, r.top() + 14.0 + title_h / 2.0), 4.0, SIGNAL);
            p.galley(pos2(r.left() + 32.0, r.top() + 14.0), tg, INK);
            p.galley(pos2(r.left() + 32.0, r.top() + 14.0 + title_h + 4.0), sg, INK2);
            a11y_text(ui, Id::unique("headline"), r, Role::Status, &format!("{title}. {sub}"), Some(Live::Polite));
            top = r.bottom() + 8.0;
        }
        if let Some(t) = self.running().and_then(|s| s.transfers.first()).cloned() {
            let (title, detail, frac) = crate::view::transfer_lines(&t);
            let more = self.running().map_or(0, |s| s.transfers.len() - 1);
            let detail = if more > 0 { format!("{detail} · and {more} more") } else { detail };
            let w = 360.0f32.min(desk.width() * 0.6);
            // In a small window the desk's front is crowded: the top instead.
            // A small window's desk front is crowded: the top instead, unless
            // a banner is there already.
            let y = if desk.width() < 780.0 && top == desk.top() { top } else { desk.bottom() - 52.0 };
            let r = Rect::from_min_size(pos2(desk.left(), y), vec2(w, 52.0));
            widgets::glass_panel(ui, "transfer", r, 14);
            p.text(pos2(r.left() + 14.0, r.top() + 16.0), Align2::LEFT_CENTER, fit(&p, &title, &regular(T_BODY), w - 28.0), regular(T_BODY), INK);
            p.text(pos2(r.left() + 14.0, r.top() + 33.0), Align2::LEFT_CENTER, fit(&p, &detail, &regular(T_META), w - 28.0), regular(T_META), INK2);
            let bar = Rect::from_min_size(pos2(r.left() + 14.0, r.bottom() - 9.0), vec2(w - 28.0, 3.0));
            p.rect_filled(bar, CornerRadius::same(2), Color32::from_white_alpha(30));
            p.rect_filled(Rect::from_min_size(bar.min, vec2((bar.width() * frac.unwrap_or(0.18)).max(3.0), 3.0)), CornerRadius::same(2), SIGNAL);
            a11y_text(ui, Id::unique("transfer"), r, Role::ProgressIndicator, &format!("{title}. {detail}"), Some(Live::Polite));
        }
        let several = self.running().is_some_and(|s| !s.peers.is_empty());
        if self.tab == Tab::Desk && several && !self.prefs_hint_dismissed() {
            let text = "Drag a computer to where it sits on your desk";
            let w = text_width(ui, text, regular(T_BODY)) + 48.0;
            if desk.width() > 860.0 {
                let r = Rect::from_min_size(pos2(desk.right() - w, desk.bottom() - 36.0), vec2(w, 36.0));
                widgets::glass_panel(ui, "drag-hint", r, 12);
                icons::draw(&p, Icon::Move, Rect::from_min_size(pos2(r.left() + 14.0, r.center().y - 8.0), vec2(16.0, 16.0)), INK3);
                p.text(pos2(r.left() + 38.0, r.center().y), Align2::LEFT_CENTER, text, regular(T_BODY), INK2);
            }
        }
    }

    /// Whether the Computers / Arrange switch is on the desk's top row.
    pub(super) fn desk_views_shown(&self) -> bool { matches!(self.tab, Tab::Desk | Tab::Layout) && !self.off() }

    /// Decides the banner for this frame (see [`NoticeHold`]).
    pub(super) fn refresh_notice(&mut self, ui: &Ui, now: f64) {
        let moving = self.running().is_some_and(|s| s.controlled_by.is_some() || s.controlling.is_some());
        let fresh = self.desk_notice();
        if let Some(left) = self.notice.update(fresh, moving, now) {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left + 0.05));
        }
    }

    /// The status worth a banner over the desk: what isn't plain sharing.
    pub(super) fn desk_notice(&self) -> Option<(String, String)> {
        let s = self.running()?;
        let online = s.peers.iter().filter(|p| p.online).count();
        let unusual = !s.broadcast.is_empty()
            || s.controlled_by.is_some()
            || s.controlling.is_some()
            || (!s.input.is_empty() && !s.peers.is_empty())
            || (online == 0 && !s.peers.is_empty())
            || (online == 1 && s.peers.iter().any(|p| p.online && self.apart.contains(&p.id)));
        unusual.then(|| self.headline_text())
    }

    /// The drag hint goes for good once a computer has been placed by
    /// hand (a marker file in the settings folder says so).
    fn prefs_hint_dismissed(&self) -> bool {
        self.dragged_once
    }

    pub(super) fn drag_hint_file(&self) -> std::path::PathBuf {
        self.link.dir.join(".window-dragged")
    }
}

/// The banner over the desk. It takes room from the computers below it,
/// which then fit their frame again, so one about control moving between
/// computers (it comes and goes with every crossing) is kept for a few
/// seconds after it would go: the desk holds still while the pointer goes
/// back and forth. Anything else (a computer that is away, a missing
/// permission) goes the moment it ends.
#[derive(Default)]
pub(super) struct NoticeHold {
    shown: Option<(String, String)>,
    moving: bool,
    until: f64,
}

impl NoticeHold {
    const HOLD: f64 = 4.0;

    pub(super) fn shown(&self) -> Option<&(String, String)> {
        self.shown.as_ref()
    }

    /// `fresh` is the banner the status calls for now, `moving` whether that
    /// is about control moving. Returns how long the banner is still held,
    /// if it is (the caller draws again then).
    pub(super) fn update(&mut self, fresh: Option<(String, String)>, moving: bool, now: f64) -> Option<f64> {
        match fresh {
            Some(n) => {
                self.shown = Some(n);
                self.moving = moving;
                self.until = now + Self::HOLD;
                None
            }
            None if self.shown.is_some() && self.moving && now < self.until => Some(self.until - now),
            None => {
                self.shown = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod notice_tests {
    use super::NoticeHold;

    fn n(t: &str) -> Option<(String, String)> {
        Some((t.into(), String::new()))
    }

    #[test]
    fn a_banner_about_moving_control_outlasts_a_crossing_back() {
        let mut h = NoticeHold::default();
        assert_eq!(h.update(n("Controlling Mac"), true, 10.0), None);
        // The pointer came home a moment later: nothing to say, but the desk holds still.
        let left = h.update(None, false, 11.0).expect("held");
        assert!((left - 3.0).abs() < 1e-9);
        assert!(h.shown().is_some());
        // The next crossing renews it.
        h.update(n("Mac is controlling this computer"), true, 12.0);
        assert!(h.update(None, false, 15.0).is_some(), "still held 3 s after the last one");
        assert_eq!(h.update(None, false, 16.5), None);
        assert!(h.shown().is_none(), "gone once the hold is over");
    }

    #[test]
    fn other_banners_go_the_moment_they_end() {
        let mut h = NoticeHold::default();
        h.update(n("Waiting for permission"), false, 10.0);
        assert_eq!(h.update(None, false, 10.1), None);
        assert!(h.shown().is_none());
    }

    #[test]
    fn a_banner_of_another_kind_replaces_a_held_one_at_once() {
        let mut h = NoticeHold::default();
        h.update(n("Controlling Mac"), true, 10.0);
        h.update(n("Mac is offline"), false, 10.5);
        assert_eq!(h.update(None, false, 10.6), None, "the offline banner is not held");
        assert!(h.shown().is_none());
    }
}
