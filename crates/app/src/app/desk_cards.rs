//! The Desk's cards and what floats over the desk: the chosen computer
//! (where it sits, how fast it answers over the last two minutes, and what
//! can be done with it), this computer's own switches, the page's actions
//! for every computer at once, the running transfer and the hint that the
//! computers can be dragged.

use super::*;
use crate::icons::{self, Icon};
use crate::rtt::{Quality, Ring};

/// How wide the band's banner and toast are at most: the desk's width, so
/// their right edges line up with the cards and the toolbar.
pub(super) const BAND_W: f32 = f32::INFINITY;

pub(super) struct NoticeLay {
    pub title: std::sync::Arc<egui::Galley>,
    pub sub: std::sync::Arc<egui::Galley>,
    pub h: f32,
    pub w: f32,
    pub take_back: bool,
}

/// A toast laid out `w` wide: its message, its detail, and its height
/// (with room for the action an error carries).
pub(super) fn toast_lay(ui: &Ui, t: &Toast, w: f32) -> (std::sync::Arc<egui::Galley>, Option<std::sync::Arc<egui::Galley>>, f32) {
    let text_w = w - 46.0 - if t.error { toast_action_w(t) } else { 0.0 };
    // An error is one line: what failed, then (quieter) why; the whole of
    // it on hover and for screen readers.
    if t.error {
        let mut job = egui::text::LayoutJob::default();
        job.append(&t.text, 0.0, egui::TextFormat::simple(regular(T_BODY), INK));
        if let Some(d) = &t.detail {
            job.append(&format!("  {d}"), 0.0, egui::TextFormat::simple(regular(T_META), INK2));
        }
        job.wrap = egui::text::TextWrapping { max_width: text_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
        return (ui.painter().layout_job(job), None, 46.0);
    }
    let g = ui.painter().layout(t.text.clone(), regular(T_BODY), INK, text_w);
    let d = t.detail.as_ref().map(|d| ui.painter().layout(d.clone(), regular(T_META), INK2, text_w));
    let h = 10.0 + g.size().y + d.as_ref().map_or(0.0, |d| d.size().y + 4.0) + 10.0;
    (g, d, h.max(if t.error { 52.0 } else { 0.0 }))
}

/// Room for an error toast's actions (Try again, when it can, and Open Health).
pub(super) fn toast_action_w(t: &Toast) -> f32 {
    if t.retry.is_some() { 236.0 } else { 130.0 }
}

/// The row the Computers / Arrange switch takes at the desk's top.
pub(super) const DESK_VIEWS_H: f32 = 46.0;

/// Text with an icon before it, as a quiet button with no fill until
/// hovered; its width.
#[allow(clippy::too_many_arguments)]
fn ghost(ui: &mut Ui, at: Pos2, h: f32, id: Id, icon: Option<Icon>, label: &str, name: &str, enabled: bool) -> (egui::Response, f32) {
    let font = regular(T_BODY);
    let w = if label.is_empty() { h } else { text_width(ui, label, font.clone()) + if icon.is_some() { 34.0 } else { 18.0 } };
    let r = Rect::from_min_size(at, vec2(w, h));
    let resp = ui.interact(widgets::hit_rect(r), id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && resp.hovered()), 0.1, motion::out_cubic);
    let p = ui.painter();
    if enabled && resp.is_pointer_button_down_on() {
        p.rect_filled(r, CornerRadius::same(8), widgets::pressed(BUTTON, 1.0));
    } else if hover > 0.01 {
        p.rect_filled(r, CornerRadius::same(8), BUTTON.gamma_multiply(hover));
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
        // Live and connected: cyan (Volt is never a status).
        p.circle_filled(at, 3.5, OK);
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
            // No reading to show: what is wrong and what to do, as words.
            p.text(pos2(body.left(), body.top() + 10.0), Align2::LEFT_CENTER, "Not found on the network", semibold(T_BODY), INK);
            let mut hint = egui::text::LayoutJob::simple("Start oneMK on it, on the same network as this computer.".into(), regular(T_BODY), INK2, col_w);
            hint.wrap.max_rows = 3;
            p.galley(pos2(body.left(), body.top() + 22.0), p.layout_job(hint), INK2);
        } else if let Some(s) = summary {
            let n = rtt::ms_number(s.now_us);
            let nw = text_width(ui, &n, big.clone());
            p.text(pos2(body.left(), body.top() + 18.0), Align2::LEFT_CENTER, &n, big, INK);
            p.text(pos2(body.left() + nw + 4.0, body.top() + 23.0), Align2::LEFT_CENTER, "ms", regular(T_BODY), INK2);
            // A number is already shown: say how much it rests on, not "Measuring…".
            let word = q.map_or("Few readings so far", Quality::word);
            let ww = text_width(ui, word, regular(T_BODY));
            p.text(pos2(body.left(), body.top() + 48.0), Align2::LEFT_CENTER, word, regular(T_BODY), if matches!(q, Some(Quality::Stalls | Quality::Slow)) { WARN } else { INK2 });
            if q.is_some() {
                let (lo, hi) = (rtt::ms_number(s.min_us), rtt::ms(s.p95_us));
                // Lost packets are named apart from slowness (stalls).
                let usual = match s.lost {
                    0 if hi == format!("{lo} ms") => format!(" · usually {hi}"),
                    0 => format!(" · usually {lo}–{hi}"),
                    1 => " · 1 packet lost".to_owned(),
                    n => format!(" · {n} packets lost"),
                };
                p.text(pos2(body.left() + ww, body.top() + 48.0), Align2::LEFT_CENTER, fit(&p, &usual, &regular(T_BODY), col_w - ww), regular(T_BODY), INK3);
            }
        } else {
            let text = if self.older_daemon(&peer.id) { "Restart oneMK to see latency" } else { "Measuring…" };
            let _ = big;
            widgets::draw_dots(&p, Rect::from_min_size(pos2(body.left() - 4.0, body.top() + 10.0), vec2(24.0, 16.0)), INK3);
            p.text(pos2(body.left(), body.top() + 48.0), Align2::LEFT_CENTER, fit(&p, text, &regular(T_BODY), col_w), regular(T_BODY), INK2);
        }
        if peer.online && (self.latency.contains(&peer.id) || self.failed_measure.contains(&peer.id)) {
            let busy = self.measuring.is_some();
            // Beside the number it measures.
            let num_w = summary.map_or(text_width(ui, "…", mono_medium(30.0)), |s| text_width(ui, &rtt::ms_number(s.now_us), mono_medium(30.0)) + 4.0 + text_width(ui, "ms", regular(T_BODY)));
            let r = Rect::from_min_size(pos2(body.left() + num_w + 10.0, body.top() + 6.0), vec2(28.0, 28.0));
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

        // The graph and the number, as words for screen readers.
        a11y_text(ui, Id::unique(("peer-latency", &peer.id)), body, Role::Label, &self.latency_words(&peer), None);
        // Right: the last two minutes.
        let graph = Rect::from_min_max(pos2(body.left() + col_w + 24.0, c.top() + 40.0), body.max);
        let plot = Rect::from_min_max(pos2(graph.left(), graph.top() + 18.0), graph.max);
        match self.rtts.of(&peer.id).filter(|_| peer.online) {
            // A handful of readings: say so in a line rather than a big,
            // nearly empty plot.
            Some(_) if summary.is_some_and(|s| s.count < 8) => {
                let n = summary.map_or(0, |s| s.count);
                let text = format!("{n} {} so far · the graph fills in over {}", if n == 1 { "reading" } else { "readings" }, rtt::span_words());
                p.text(pos2(graph.left() + 30.0, graph.top() + 6.0), Align2::LEFT_CENTER, fit(&p, &text, &regular(T_META), graph.width() - 30.0), regular(T_META), INK3);
            }
            Some(ring) if summary.is_some() => {
                p.text(pos2(graph.left() + 30.0, graph.top() + 6.0), Align2::LEFT_CENTER, format!("Round trip, last {}", rtt::span_words()), regular(T_META), INK3);
                chart(&p, plot, ring, self.rtt_now(), summary);
            }
            _ => {
                // No readings: no empty axes, one quiet line where the
                // graph's title would be.
                let text = if !peer.online { "No round trips while it is offline" } else if self.older_daemon(&peer.id) { "" } else { "The round-trip graph starts with the first replies" };
                p.text(pos2(graph.left() + 30.0, graph.top() + 6.0), Align2::LEFT_CENTER, fit(&p, text, &regular(T_META), graph.width() - 30.0), regular(T_META), INK3);
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
        // Nothing paired: this card is the first step, with the one action.
        if status.peers.is_empty() {
            let th = para(ui, c.left_top(), c.width(), "Pair your first computer", semibold(T_TITLE), INK);
            let text = "Open oneMK on the other computer too, then choose Pair a computer here. Both need to be on the same network; a six-digit code on both screens confirms it.";
            let sh = para(ui, c.left_top() + vec2(0.0, th + 6.0), c.width().min(520.0), text, regular(T_BODY), INK2);
            let bw = button_width(ui, "Pair a computer") + 8.0;
            let b = Rect::from_min_size(pos2(c.left(), c.top() + th + 6.0 + sh + 16.0), vec2(bw, 36.0));
            if button(ui, b, Id::unique("pair-in-card"), "Pair a computer", Look::Primary, !self.sheet_open(), None).clicked() {
                let now = self.now;
                self.start_pairing(now);
            }
            a11y_text(ui, Id::unique("first-step"), Rect::from_min_size(c.min, vec2(c.width(), th + sh + 6.0)), Role::Status, &format!("Pair your first computer. {text}"), None);
            return;
        }
        let cy = c.top() + 12.0;
        icons::pointer(&p, pos2(c.left(), cy - 7.0), 14.0, SIGNAL);
        let nw = text_width(ui, &status.name, semibold(T_TITLE));
        p.text(pos2(c.left() + 22.0, cy), Align2::LEFT_CENTER, &status.name, semibold(T_TITLE), INK);
        p.text(pos2(c.left() + 32.0 + nw, cy), Align2::LEFT_CENTER, format!("This computer · {}", os_name(me_os())), regular(T_BODY), INK3);
        let online = status.peers.iter().filter(|p| p.online).count();
        let paired = match status.peers.len() {
            0 => "None yet".to_owned(),
            n => format!("{online} of {n} connected"),
        };
        info_row(ui, c, c.top() + 52.0, "Paired", &paired, false, 1.0);
        info_row(ui, c, c.top() + 76.0, "ID", &status.id, true, 1.0);
        if status.peers.is_empty() && c.height() > 150.0 {
            // What pairing needs, so the first step is clear from here too.
            p.hline(c.x_range(), c.top() + 100.0, Stroke::new(1.0, LINE));
            para(ui, pos2(c.left(), c.top() + 114.0), c.width().min(460.0), "To share this keyboard and mouse, open oneMK on the other computer as well, then choose Pair a computer. Both need to be on the same network.", regular(T_BODY), INK2);
        }
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
        // Laid out from the right edge, then made left to right, so Tab
        // reaches them in reading order: broadcast, Timer, Lock.
        let y = head.center().y - 17.0;
        let narrow = head.width() < 780.0;
        let online = self.running().is_some_and(|s| s.peers.iter().any(|p| p.online));
        let on = self.broadcasting();
        let lock_label = if narrow { "Lock all" } else { "Lock every screen" };
        let lock_w = text_width(ui, lock_label, semibold(T_BODY)) + 46.0;
        let lock_r = Rect::from_min_size(pos2(head.right() - lock_w, y), vec2(lock_w, 34.0));
        let mut x = lock_r.left();
        let timer_r = (online || on).then(|| {
            let tw = text_width(ui, "Timer", semibold(T_BODY)) + 46.0;
            x -= tw + 8.0;
            Rect::from_min_size(pos2(x, y), vec2(tw, 34.0))
        });
        let bc_label = "Type on every computer";
        let bc_r = (online || on).then(|| {
            let w = if narrow { 34.0 } else { text_width(ui, bc_label, semibold(T_BODY)) + 34.0 + 58.0 };
            x -= w + 8.0;
            Rect::from_min_size(pos2(x, y), vec2(w, 34.0))
        });
        if let Some(r) = bc_r {
            let busy = self.busy.iter().any(|j| matches!(j, Job::Broadcast(_)));
            let key = if on { format!(" {}", self.broadcast_stop_hint()) } else { String::new() };
            if narrow {
                // Narrow: the same action as an icon, lit while it is on.
                let name = if on { format!("Typing on every computer.{key}") } else { "Type on every computer: everything you type, click and scroll here goes to every connected computer at once".to_owned() };
                let resp = button(ui, r, Id::unique("broadcast"), "", Look::Glass, !busy && !on, Some(&name));
                let enabled = !busy && !on;
                resp.widget_info(|| egui::WidgetInfo::selected(Role::Switch, enabled, on, "Type on every computer"));
                if on {
                    ui.painter().rect_stroke(r, CornerRadius::same(17), Stroke::new(1.5, SIGNAL), StrokeKind::Inside);
                }
                icons::draw(ui.painter(), Icon::Broadcast, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), if on { SIGNAL } else { INK2 });
                if resp.on_hover_text(&name).clicked() {
                    self.ask_broadcast(!on);
                }
            } else {
                let resp = button(ui, r, Id::unique("broadcast"), "", Look::Glass, !busy && !on, Some("Broadcast: type, click and scroll on every connected computer at once"));
                // A switch, with its state, for screen readers too.
                let enabled = !busy && !on;
                resp.widget_info(|| egui::WidgetInfo::selected(Role::Switch, enabled, on, "Type on every computer"));
                icons::draw(ui.painter(), Icon::Broadcast, Rect::from_min_size(pos2(r.left() + 12.0, r.center().y - 8.0), vec2(16.0, 16.0)), INK2);
                ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, bc_label, semibold(T_BODY), INK);
                // A small switch, drawn as the window's other switches are.
                let track = Rect::from_min_size(pos2(r.right() - 46.0, r.center().y - 9.0), vec2(32.0, 18.0));
                let knob_t = tween(ui.ctx(), Id::unique("broadcast-knob"), f32::from(on), motion::space(0.2), motion::out_cubic);
                ui.painter().rect_filled(track, CornerRadius::same(9), mix(SWITCH_OFF, ACCENT, knob_t));
                ui.painter().rect_stroke(track, CornerRadius::same(9), Stroke::new(1.0, mix(SWITCH_EDGE, Color32::TRANSPARENT, knob_t)), StrokeKind::Inside);
                ui.painter().circle_filled(pos2(track.left() + 9.0 + 14.0 * knob_t, track.center().y), 7.0, Color32::WHITE);
                if resp.on_hover_text(format!("Everything you type, click and scroll here goes to every connected computer at once.{key}")).clicked() {
                    self.ask_broadcast(!on);
                }
            }
        }
        if let Some(rect) = timer_r {
            // When typing on every computer stops by itself: an icon and a word.
            let name = "Timer: when typing on every computer stops by itself";
            let response = button(ui, rect, Id::unique("broadcast-options"), "", Look::Glass, !on, Some(name));
            icons::draw(ui.painter(), Icon::Clock, Rect::from_min_size(pos2(rect.left() + 12.0, rect.center().y - 8.0), vec2(16.0, 16.0)), if on { INK3 } else { INK2 });
            ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, "Timer", semibold(T_BODY), if on { INK3 } else { INK });
            let response = response.on_hover_text(name);
            egui::Popup::menu(&response).width(280.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                let (row, _) = ui.allocate_exact_size(vec2(280.0, 34.0), Sense::hover());
                self.broadcast_timeout(ui, row);
                ui.label("Use zero to keep broadcasting until you stop it.");
            });
        }
        let busy = self.busy.contains(&Job::LockAll);
        if button(ui, lock_r, Id::unique("lock-all"), "", Look::Glass, !busy, Some("Lock this computer and every connected one")).clicked() {
            self.busy.push(Job::LockAll);
            self.link.lock_all();
        }
        icons::draw(ui.painter(), Icon::Lock, Rect::from_min_size(pos2(lock_r.left() + 12.0, lock_r.center().y - 8.0), vec2(16.0, 16.0)), INK2);
        ui.painter().text(pos2(lock_r.left() + 34.0, lock_r.center().y), Align2::LEFT_CENTER, lock_label, semibold(T_BODY), INK);
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
        if let Some(n) = self.notice_lay(ui, desk.width()) {
            let r = Rect::from_min_size(pos2(desk.left(), top), vec2(n.w, n.h));
            widgets::glass_panel(ui, "notice", r, 14);
            let title_h = n.title.size().y;
            // Volt only while something is on (typing on every computer);
            // a plain notice (waiting, being controlled) gets a neutral dot.
            let dot = if self.broadcasting() { SIGNAL } else { INK2 };
            p.circle_filled(pos2(r.left() + 18.0, r.top() + 14.0 + title_h / 2.0), 4.0, dot);
            let label = format!("{}. {}", n.title.text(), n.sub.text());
            p.galley(pos2(r.left() + 32.0, r.top() + 14.0), n.title, INK);
            p.galley(pos2(r.left() + 32.0, r.top() + 14.0 + title_h + 4.0), n.sub, INK2);
            if n.take_back {
                // Being controlled: the way back, named, where the eye already is.
                let bw = button_width(ui, "Take back");
                let b = Rect::from_min_size(pos2(r.right() - 14.0 - bw, r.center().y - 16.0), vec2(bw, 32.0));
                let busy = self.busy.contains(&Job::TakeBack);
                if button(ui, b, Id::unique("notice-take-back"), "Take back", Look::Primary, !busy && !self.sheet_open(), Some("Take back control of this computer")).clicked() {
                    self.busy.push(Job::TakeBack);
                    self.link.take_back();
                }
            }
            a11y_text(ui, Id::unique("headline"), r, Role::Status, &label, Some(Live::Polite));
            top = r.bottom() + 8.0;
        }
        // The toast, when there is one, comes next in the band (drawn by
        // `toast_view`, which keeps its timing).
        if let Some(h) = self.toast_band_h(ui, desk.width()) {
            self.toast_slot = Some(Rect::from_min_size(pos2(desk.left(), top), vec2(BAND_W.min(desk.width()), h)));
            top += h + 8.0;
        } else {
            self.toast_slot = None;
        }
        if let Some(t) = self.running().and_then(|s| s.transfers.first()).cloned() {
            let (title, detail, frac) = crate::view::transfer_lines(&t);
            let more = self.running().map_or(0, |s| s.transfers.len() - 1);
            let detail = if more > 0 { format!("{detail} · and {more} more") } else { detail };
            let w = desk.width().min(520.0);
            // In a small window the desk's front is crowded: the top instead.
            // A small window's desk front is crowded: the top instead, unless
            // a banner is there already.
            let y = top;
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
        if self.tab == Tab::Desk && several && !self.prefs_hint_dismissed() && !self.sheet_open() {
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

    /// The banner's text, laid out for a desk `desk_w` wide; its size, and
    /// whether it carries Take back (this computer is being controlled).
    pub(super) fn notice_lay(&self, ui: &Ui, desk_w: f32) -> Option<NoticeLay> {
        let (title, sub) = self.notice.shown().cloned()?;
        let take_back = self.running().is_some_and(|s| s.controlled_by.is_some());
        let w = BAND_W.min(desk_w);
        let room = w - 46.0 - if take_back { button_width(ui, "Take back") + 14.0 } else { 0.0 };
        let job = |text: &str, font: egui::FontId, col: Color32| {
            let mut j = egui::text::LayoutJob::simple(text.to_owned(), font, col, room);
            j.wrap.max_rows = 3;
            ui.painter().layout_job(j)
        };
        let (title, sub) = (job(&title, semibold(T_BODY), INK), job(&sub, regular(T_META), INK2));
        let h = (14.0 + title.size().y + 4.0 + sub.size().y + 14.0).max(if take_back { 56.0 } else { 0.0 });
        Some(NoticeLay { title, sub, h, w, take_back })
    }

    /// How tall the toast is in the desk's band, if one shows.
    pub(super) fn toast_band_h(&self, ui: &Ui, desk_w: f32) -> Option<f32> {
        let t = self.toast.as_ref()?;
        Some(toast_lay(ui, t, BAND_W.min(desk_w)).2)
    }

    /// The band over the desk (banner, toast, a small window's transfer),
    /// with its gaps: the desk is framed below it, never under it.
    pub(super) fn band_h(&self, ui: &Ui, desk_w: f32) -> f32 {
        // Only the Desk and Arrange views draw the band.
        if !matches!(self.tab, Tab::Desk | Tab::Layout) {
            return 0.0;
        }
        let mut h = 0.0;
        if let Some(n) = self.notice_lay(ui, desk_w) {
            h += n.h + 8.0;
        }
        if let Some(t) = self.toast_band_h(ui, desk_w) {
            h += t + 8.0;
        }
        if self.running().is_some_and(|s| !s.transfers.is_empty()) {
            h += 52.0 + 8.0;
        }
        h
    }

    /// Room kept free along the desk's bottom for what floats there.
    pub(super) fn desk_foot_h(&self, desk_w: f32) -> f32 {
        let hint = self.tab == Tab::Desk && desk_w > 860.0 && self.running().is_some_and(|s| !s.peers.is_empty()) && !self.prefs_hint_dismissed();
        if hint { 36.0 + 8.0 } else { 0.0 }
    }

    /// The desk when there is too little room for monitors anyone could
    /// read: each computer as a named chip (where the pointer is, whether it
    /// is connected, where it sits); choosing one shows it in the card below.
    pub(super) fn desk_chips(&mut self, ui: &mut Ui, r: Rect) {
        let Some(status) = self.running().cloned() else { return };
        let pointer = self.pointer_on();
        let sel = self.selected_id();
        let mut items: Vec<(String, String, String, bool)> = vec![("me".into(), status.name.clone(), "This computer".into(), true)];
        for p in &status.peers {
            let side = self.side_shown(&p.id, p.side);
            items.push((p.id.clone(), p.name.clone(), if p.online { on_your(side).to_owned() } else { "offline".into() }, p.online));
        }
        let hovering_files = ui.input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Option<Vec<std::path::PathBuf>> = ui.input(|i| (!i.raw.dropped_files.is_empty()).then(|| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect()));
        let drop_at = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
        let mut drop_to: Option<(String, Vec<std::path::PathBuf>)> = None;
        const H: f32 = 44.0;
        let widths: Vec<f32> = items.iter().map(|(_, n, w, _)| (text_width(ui, n, semibold(T_BODY)).max(text_width(ui, w, regular(T_META))) + 50.0).clamp(120.0, 220.0)).collect();
        // Rows that wrap, centred in the room there is.
        let mut rows: Vec<Vec<usize>> = vec![vec![]];
        let mut x = 0.0;
        for (k, w) in widths.iter().enumerate() {
            if x > 0.0 && x + w > r.width() - 24.0 {
                rows.push(vec![]);
                x = 0.0;
            }
            rows.last_mut().expect("one row").push(k);
            x += w + 8.0;
        }
        let total_h = rows.len() as f32 * (H + 8.0) - 8.0;
        // On Clipboard & files the chips sit at the top, close to the page
        // they serve; on the desk, centred where the monitors would be.
        let mut y = if self.tab == Tab::Files { r.top() + 12.0 } else { r.center().y - total_h / 2.0 };
        for row in rows {
            let rw: f32 = row.iter().map(|k| widths[*k] + 8.0).sum::<f32>() - 8.0;
            let mut x = r.center().x - rw / 2.0;
            for k in row {
                let (id, name, words, online) = &items[k];
                let cr = Rect::from_min_size(pos2(x, y), vec2(widths[k], H));
                x += widths[k] + 8.0;
                let rid = Id::unique(("desk-chip", id.as_str()));
                let resp = ui.interact(cr, rid, widgets::sense(ui, Sense::click()));
                let hover = tween(ui.ctx(), rid.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
                let chosen = sel.as_deref() == Some(id.as_str());
                let fill = if chosen { BUTTON_LIT } else { mix(ELEVATED, BUTTON, hover) };
                let p = ui.painter();
                p.rect(cr, CornerRadius::same(R_INNER), widgets::pressed(fill, f32::from(resp.is_pointer_button_down_on())), Stroke::new(if chosen { 1.5 } else { 1.0 }, if chosen { INK2 } else { ELEVATED_EDGE }), StrokeKind::Inside);
                let mark = pos2(cr.left() + 18.0, cr.center().y);
                if pointer.as_deref() == Some(id.as_str()) {
                    icons::pointer(p, mark - vec2(5.0, 7.0), 13.0, SIGNAL);
                } else if *online {
                    p.circle_filled(mark, 3.5, OK);
                } else {
                    p.circle_stroke(mark, 3.0, Stroke::new(1.5, INK3));
                }
                // On Clipboard & files each chip is a drop target too.
                let target = self.tab == Tab::Files && *online && id != "me";
                let words: &str = if target && hovering_files { "Drop here" } else { words };
                if target && hovering_files {
                    p.rect_stroke(cr.expand(2.0), CornerRadius::same(R_INNER + 2), Stroke::new(1.5, SIGNAL), StrokeKind::Outside);
                }
                if target && let Some(paths) = dropped.as_ref().filter(|_| drop_at.is_some_and(|q| cr.contains(q))) {
                    drop_to = Some((id.clone(), paths.clone()));
                }
                let tw = cr.width() - 46.0;
                p.text(pos2(cr.left() + 34.0, cr.top() + 15.0), Align2::LEFT_CENTER, fit(p, name, &semibold(T_BODY), tw), semibold(T_BODY), if *online { INK } else { INK2 });
                p.text(pos2(cr.left() + 34.0, cr.top() + 31.0), Align2::LEFT_CENTER, fit(p, words, &regular(T_META), tw), regular(T_META), INK3);
                widgets::focus_ring(ui, &resp, cr, R_INNER);
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                widgets::a11y(&resp, Role::Button, &format!("{name}, {words}"), Some(chosen));
                let resp = resp.on_hover_text(name);
                if resp.clicked() {
                    self.select(id.clone());
                }
            }
            y += H + 8.0;
        }
        // Files let go: on a chip, to that computer; elsewhere, asked.
        if let Some(paths) = dropped {
            let now = self.now;
            match drop_to.and_then(|(id, p)| self.scene.devices.iter().position(|d| d.id == id).map(|i| (i, p))) {
                Some((i, p)) => self.send_to(i, p),
                None => self.drop_files(paths, None, now),
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
