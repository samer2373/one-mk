//! What floats over the desk (the banner, the running transfer, the drop
//! hint), the toasts' layout, and the desk as named chips in a small
//! window. The Desk's panel and toolbar are in desk_panel.rs.

use super::*;

/// How wide the band's banner is at most: the desk's width, so its right
/// edge lines up with the cards and the toolbar.
pub(super) const BAND_W: f32 = f32::INFINITY;

pub(super) struct NoticeLay {
    pub title: std::sync::Arc<egui::Galley>,
    pub sub: std::sync::Arc<egui::Galley>,
    pub h: f32,
    pub w: f32,
    pub take_back: bool,
    /// Something needs the user (being controlled, a permission, a
    /// computer that refuses this one): the attention fill.
    pub attention: bool,
}

/// The most lines a toast's message, and what it says under it, take.
const TOAST_ROWS: usize = 3;
const TOAST_DETAIL_ROWS: usize = 6;

/// A toast laid out `w` wide: its message, its detail, and its height
/// (with room for the action an error carries).
pub(super) fn toast_lay(ui: &Ui, t: &Toast, w: f32) -> (std::sync::Arc<egui::Galley>, Option<std::sync::Arc<egui::Galley>>, f32) {
    let text_w = w - 46.0 - if t.error { toast_action_w(ui, t) } else { 0.0 };
    // An error is one line when it fits: what failed, then (quieter) why.
    // Otherwise why goes on the lines below, whole: never cut off.
    if t.error {
        let mut job = egui::text::LayoutJob::default();
        job.append(&t.text, 0.0, egui::TextFormat::simple(regular(T_BODY), pal().ink));
        if let Some(d) = &t.detail {
            job.append(&format!("  {d}"), 0.0, egui::TextFormat::simple(regular(T_META), pal().ink2));
        }
        job.wrap = egui::text::TextWrapping { max_width: text_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
        let line = ui.painter().layout_job(job);
        if !line.elided || t.detail.is_none() {
            return (line, None, 46.0);
        }
    }
    // Whole, up to a few lines: a long account of what failed (a program's
    // own output) must not make a toast taller than the window. What is
    // cut ends in "…", and the full text is on hover and for a screen reader.
    let lines = |text: &str, font: egui::FontId, color: Color32, rows: usize| {
        let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, text_w);
        job.wrap.max_rows = rows;
        job.wrap.overflow_character = Some('…');
        ui.painter().layout_job(job)
    };
    let g = lines(&t.text, regular(T_BODY), pal().ink, TOAST_ROWS);
    let d = t.detail.as_ref().map(|d| lines(d, regular(T_META), pal().ink2, TOAST_DETAIL_ROWS));
    let h = 10.0 + g.size().y + d.as_ref().map_or(0.0, |d| d.size().y + 4.0) + 10.0;
    (g, d, h.max(if t.error { 52.0 } else { 0.0 }))
}

/// A button on an error toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToastButton {
    Retry,
    Act,
    Health,
}

/// An error toast's buttons, left to right: Try again (when it can), its
/// own action, Open Health (when Health can help).
pub(super) fn toast_actions(t: &Toast) -> Vec<(&'static str, ToastButton)> {
    let mut out = Vec::new();
    if t.retry.is_some() { out.push(("Try again", ToastButton::Retry)); }
    if let Some(act) = &t.act { out.push((act.label(), ToastButton::Act)); }
    if t.health { out.push(("Open Health", ToastButton::Health)); }
    out
}

/// Room for an error toast's buttons, with a margin each side.
pub(super) fn toast_action_w(ui: &Ui, t: &Toast) -> f32 {
    let actions = toast_actions(t);
    if actions.is_empty() { return 12.0; }
    24.0 + actions.iter().map(|(label, _)| button_width(ui, label)).sum::<f32>() + 8.0 * (actions.len() - 1) as f32
}

/// A smooth line through the readings: each gap between two readings is
/// drawn as a short curve (Catmull-Rom), so the line has no corners.
pub(super) fn smooth(pts: &[Pos2]) -> Vec<Pos2> {
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

impl App {
    /// Over the desk: the banner, the transfer that is running, and where
    /// files go while they are carried over the window.
    pub(super) fn desk_overlay(&mut self, ui: &mut Ui, desk: Rect, hovering_files: bool) {
        let p = ui.painter().clone();
        if hovering_files {
            let r = Rect::from_center_size(pos2(desk.center().x, desk.bottom() - 24.0), vec2(280.0, 40.0));
            widgets::glass_panel(ui, "drop-hint", r, 14);
            p.text(r.center(), Align2::CENTER_CENTER, "Drop on a computer to send", semibold(T_BODY), pal().accent);
            return;
        }
        // What needs saying right now (broadcasting, being controlled,
        // waiting for permission...): a banner over the desk, read out.
        // On the Desk, above the toolbar (desk_panel.rs makes room).
        let mut top = desk.top();
        if let Some(n) = self.notice_lay(ui, desk.width()) {
            let r = Rect::from_min_size(pos2(desk.left(), top), vec2(n.w, n.h));
            self.paint_notice(ui, r, n);
            top = r.bottom() + 8.0;
        }
        // Layout learning's question, as on Arrange (place.rs).
        if !self.suggestion_in_row(ui)
            && let Some(h) = self.suggestion_banner(ui, desk.left(), top, desk.width())
        {
            top += h + 8.0;
        }
        if let Some(t) = self.running().and_then(|s| s.transfers.first()).cloned() {
            let (title, detail, frac) = crate::view::transfer_lines(&t);
            let more = self.running().map_or(0, |s| s.transfers.len() - 1);
            let detail = if more > 0 { format!("{detail} · and {more} more") } else { detail };
            let w = desk.width().min(520.0);
            let r = Rect::from_min_size(pos2(desk.left(), top), vec2(w, 56.0));
            banner_ground(&p, r, false);
            p.text(pos2(r.left() + 14.0, r.top() + 16.0), Align2::LEFT_CENTER, fit(&p, &title, &regular(T_BODY), w - 28.0), regular(T_BODY), pal().ink);
            p.text(pos2(r.left() + 14.0, r.top() + 33.0), Align2::LEFT_CENTER, fit(&p, &detail, &regular(T_META), w - 28.0), regular(T_META), pal().ink2);
            // The handoff's progress: 5 points, the accent (amber when stalled).
            let bar = Rect::from_min_size(pos2(r.left() + 14.0, r.bottom() - 12.0), vec2(w - 28.0, 5.0));
            widgets::paint_progress(&p, bar, Some(frac.unwrap_or(0.18)), t.rate == 0);
            a11y_text(ui, Id::unique("transfer"), r, Role::ProgressIndicator, &format!("{title}. {detail}"), Some(Live::Polite));
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
        let (title, sub) = (job(&title, semibold(T_BODY), pal().ink), job(&sub, regular(T_META), pal().ink2));
        let h = (14.0 + title.size().y + 4.0 + sub.size().y + 14.0).max(if take_back { 56.0 } else { 0.0 });
        // Being controlled, a permission still missing, a computer that no
        // longer trusts this one: these need the user.
        let attention = self.running().is_some_and(|s| s.controlled_by.is_some() || (!s.input.is_empty() && !s.peers.is_empty()) || s.peers.iter().any(|p| !p.online && s.removed_us(&p.id)) || !s.unrecognised.is_empty());
        Some(NoticeLay { title, sub, h, w, take_back, attention })
    }

    /// A banner (handoff 4.3): `surface` with a hairline edge, corner 12;
    /// what needs the user on `warn_soft` with a ◆. The title, what to do,
    /// and Take back at its right while this computer is controlled.
    pub(super) fn paint_notice(&mut self, ui: &mut Ui, r: Rect, n: NoticeLay) {
        let p = ui.painter().clone();
        banner_ground(&p, r, n.attention);
        let title_h = n.title.size().y;
        let mark = pos2(r.left() + 18.0, r.top() + 14.0 + title_h / 2.0);
        // Cobalt only while something is on (typing on every computer); a
        // plain notice (waiting, controlling another) gets a neutral dot.
        if n.attention {
            widgets::status_mark(&p, mark, 8.0, widgets::Mark::Warn);
        } else {
            p.circle_filled(mark, 4.0, if self.broadcasting() { pal().accent } else { pal().ink2 });
        }
        let label = format!("{}. {}", n.title.text(), n.sub.text());
        p.galley(pos2(r.left() + 32.0, r.top() + 14.0), n.title, pal().ink);
        p.galley(pos2(r.left() + 32.0, r.top() + 14.0 + title_h + 4.0), n.sub, pal().ink2);
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
    }

    /// On a page without the desk: the banner while typing goes to every
    /// computer (with how to stop it), at the page's top. Returns the room
    /// it takes.
    pub(super) fn page_banner(&mut self, ui: &mut Ui, body: Rect) -> f32 {
        if !self.broadcasting() {
            return 0.0;
        }
        let Some(n) = self.notice_lay(ui, body.width()) else { return 0.0 };
        let r = Rect::from_min_size(body.min, vec2(n.w, n.h));
        let h = n.h;
        self.paint_notice(ui, r, n);
        h + 12.0
    }

    /// The band over the desk (banner, a small window's transfer),
    /// with its gaps: the desk is framed below it, never under it.
    pub(super) fn band_h(&self, ui: &Ui, desk_w: f32) -> f32 {
        // Only the Desk draws the band.
        if self.tab != Tab::Desk {
            return 0.0;
        }
        let mut h = 0.0;
        if let Some(n) = self.notice_lay(ui, desk_w) {
            h += n.h + 8.0;
        }
        if !self.suggestion_in_row(ui)
            && let Some(s) = self.suggestion_h(ui, desk_w)
        {
            h += s + 8.0;
        }

        if self.running().is_some_and(|s| !s.transfers.is_empty()) {
            h += 56.0 + 8.0;
        }
        h
    }

    /// The desk when there is too little room for monitors anyone could
    /// read: each computer as a named chip (where the pointer is, whether it
    /// is connected, where it sits); choosing one shows it in the card below.
    pub(super) fn desk_chips(&mut self, ui: &mut Ui, r: Rect) {
        let Some(status) = self.running_shared() else { return };
        let pointer = self.pointer_on();
        let sel = self.selected_id();
        let mut items: Vec<(String, String, String, bool)> = vec![("me".into(), status.name.clone(), "This computer".into(), true)];
        for p in &status.peers {
            let side = self.side_shown(&p.id, p.side);
            items.push((p.id.clone(), p.name.clone(), if p.online { on_your(side).to_owned() } else { "offline".into() }, p.online));
        }
        let dropped: Option<Vec<std::path::PathBuf>> = ui.input(|i| (!i.raw.dropped_files.is_empty()).then(|| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect()));
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
        // Centred where the monitors would be.
        let mut y = r.center().y - total_h / 2.0;
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
                let fill = if chosen { pal().surface2_hover } else { mix(pal().surface, pal().surface2, hover) };
                let p = ui.painter();
                p.rect(cr, CornerRadius::same(R_INNER), widgets::pressed(fill, f32::from(resp.is_pointer_button_down_on())), Stroke::new(if chosen { 1.5 } else { 1.0 }, if chosen { pal().ink2 } else { pal().line }), StrokeKind::Inside);
                let mark = pos2(cr.left() + 18.0, cr.center().y);
                if pointer.as_deref() == Some(id.as_str()) {
                    desk_panel::pointer_mark(p, mark - vec2(4.0, 6.5), 13.0, pal().accent);
                } else if *online {
                    widgets::status_mark(p, mark, 7.0, widgets::Mark::Ok);
                } else {
                    widgets::status_mark(p, mark, 7.0, widgets::Mark::Off);
                }
                let tw = cr.width() - 46.0;
                p.text(pos2(cr.left() + 34.0, cr.top() + 15.0), Align2::LEFT_CENTER, fit(p, name, &semibold(T_BODY), tw), semibold(T_BODY), if *online { pal().ink } else { pal().ink2 });
                p.text(pos2(cr.left() + 34.0, cr.top() + 31.0), Align2::LEFT_CENTER, fit(p, words, &regular(T_META), tw), regular(T_META), pal().ink3);
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
        // Files let go over the chips: asked where they go.
        if let Some(paths) = dropped {
            let now = self.now;
            self.drop_files(paths, None, now);
        }
    }

    /// Whether the toolbar's row is at the desk's top: on the Desk, once a
    /// computer is paired.
    pub(super) fn desk_views_shown(&self) -> bool { self.tab == Tab::Desk && self.running().is_some_and(|s| !s.peers.is_empty()) }

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

}

/// A banner's ground (handoff 4.3): `surface`, or `warn_soft` when it needs
/// the user, a hairline edge, corner 12.
pub(super) fn banner_ground(p: &Painter, r: Rect, attention: bool) {
    // A card's elevation (it floats over the floor), its fill per need.
    for s in widgets::card_shadow(r, CornerRadius::same(12)) {
        p.add(s);
    }
    p.rect(r, CornerRadius::same(12), if attention { pal().warn_soft } else { pal().surface }, Stroke::new(1.0, pal().card_edge), StrokeKind::Inside);
    if !attention && pal().theme == Theme::Dark {
        p.add(widgets::top_light(r, 12));
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
