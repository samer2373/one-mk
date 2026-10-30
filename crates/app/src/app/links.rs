//! The live latency graph: a card under the selected computer on the Desk
//! tab, and a compact row per computer on the Health tab. It draws what
//! `crate::rtt` works out; nothing here moves.

use super::*;
use crate::rtt::{self, Quality, Ring, Summary};

/// The graph's height on the Desk card, and in a Health row.
const CARD_GRAPH_H: f32 = 40.0;
/// Beside the Desk card's graph: the number now and its word.
const GUTTER: f32 = 76.0;
/// What the Desk card takes, from its top edge down.
pub(super) const CARD_H: f32 = CARD_GRAPH_H + 18.0;
/// Where the card starts in the inspector: under the quick actions.
pub(super) const CARD_Y: f32 = 154.0;
const ROW_GRAPH_H: f32 = 22.0;
pub(super) const ROW_H: f32 = 34.0;

/// The colour for a state: calm grey while it is smooth, amber when not.
/// The word beside it says the same, so colour is never the only signal.
fn tone(q: Option<Quality>) -> Color32 {
    match q {
        Some(Quality::Stalls | Quality::Slow) => SIGNAL,
        _ => INK2,
    }
}

/// The line, in the box `r`: grey or amber, with a dot at the newest
/// reading and a marker where packets were lost.
fn sparkline(p: &Painter, r: Rect, ring: &Ring, now: f64, q: Option<Quality>) {
    let t = ring.trace(now, r.width(), r.height());
    let col = tone(q);
    let width = if q == Some(Quality::Slow) { 2.0 } else { 1.5 };
    p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, LINE));
    for line in &t.lines {
        let pts: Vec<Pos2> = line.iter().map(|[x, y]| r.min + vec2(*x, *y)).collect();
        match pts[..] {
            [] => {}
            [one] => {
                p.circle_filled(one, 1.5, col);
            }
            _ => {
                p.add(egui::Shape::line(pts, Stroke::new(width, col)));
            }
        }
    }
    if let Some([x, y]) = t.lines.last().and_then(|l| l.last()) {
        p.circle_filled(r.min + vec2(*x, *y), 2.5, if q.is_some() && col == INK2 { INK } else { col });
    }
    // Lost packets: a thin amber line top to bottom ending in a ring, so
    // it reads apart from a slow reading and not only by colour.
    for x in &t.losses {
        let x = r.left() + x;
        p.line_segment([pos2(x, r.top() + 4.0), pos2(x, r.bottom())], Stroke::new(1.0, SIGNAL.gamma_multiply(0.6)));
        p.circle_stroke(pos2(x, r.top() + 2.5), 2.5, Stroke::new(1.0, SIGNAL));
    }
}

impl App {
    /// The clock the graphs are drawn on: the window's own, or the moment
    /// a scripted history (a screenshot) ended.
    fn rtt_now(&self) -> f64 {
        self.rtt_frozen.unwrap_or(self.now)
    }

    /// The panel under the desk: tall enough for the inspector with the
    /// latency card, on the Desk tab once a computer is paired.
    pub(super) fn panel_h(&self) -> f32 {
        if self.tab == Tab::Files {
            return 390.0;
        }
        let with_card = self.tab == Tab::Desk && self.running().is_some_and(|s| !s.peers.is_empty());
        if with_card { PANEL_H.max(CARD_Y + CARD_H + 2.0 * PAD) } else { PANEL_H }
    }

    /// A graph is on screen right now (so the daemon is worth asking).
    pub(super) fn graph_wanted(&self) -> bool {
        if self.rtt_frozen.is_some() {
            return false;
        }
        let Some(s) = self.running() else { return false };
        match self.tab {
            Tab::Desk => self.selected_id().is_some_and(|id| s.peers.iter().any(|p| p.id == id && p.online)),
            Tab::Health => s.peers.iter().any(|p| p.online),
            _ => false,
        }
    }

    /// A connected computer that the daemon reports no link for: a daemon
    /// from before the graph, still running after an upgrade.
    fn older_daemon(&self, id: &str) -> bool {
        self.rtts.of(id).is_none() && self.running().is_some_and(|s| s.link_of(id).is_none())
    }

    fn summary_of(&self, id: &str) -> Option<Summary> {
        self.rtts.of(id).and_then(|r| r.summary(self.rtt_now()))
    }

    /// Under the selected computer on the Desk tab, from `y` and
    /// [`CARD_H`] tall: its latency. The line is on the left; the number
    /// now and the word for it are beside it; the numbers over the span
    /// are below.
    pub(super) fn latency_card(&self, ui: &Ui, c: Rect, y: f32, peer: &PeerStatus) {
        let p = ui.painter();
        let now = self.rtt_now();
        let summary = self.summary_of(&peer.id);
        let q = summary.and_then(|s| s.quality());
        let graph = Rect::from_min_size(pos2(c.left(), y), vec2(c.width() - GUTTER - 8.0, CARD_GRAPH_H));
        p.rect(graph, CornerRadius::same(6), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        match (self.rtts.of(&peer.id), q) {
            (Some(ring), Some(_)) => sparkline(p, graph.shrink2(vec2(6.0, 6.0)), ring, now, q),
            _ => {
                let text = if self.older_daemon(&peer.id) { "Restart one·mk to see latency" } else { "Measuring…" };
                p.text(graph.center(), Align2::CENTER_CENTER, fit(p, text, &regular(T_META), graph.width() - 8.0), regular(T_META), INK2);
            }
        }
        let right = c.right();
        if let Some(s) = summary {
            p.text(pos2(right, y + 12.0), Align2::RIGHT_CENTER, rtt::ms(s.now_us), mono_medium(T_BODY), INK);
        }
        if let Some(q) = q {
            p.text(pos2(right, y + 32.0), Align2::RIGHT_CENTER, q.word(), regular(T_META), tone(Some(q)));
        }
        if let Some(s) = summary.filter(|_| q.is_some()) {
            let foot = y + CARD_GRAPH_H + 10.0;
            let full = format!("min {} · avg {} · max {}", rtt::ms_number(s.min_us), rtt::ms_number(s.avg_us), rtt::ms(s.max_us));
            let text = fit(p, &full, &mono(T_META), c.width());
            p.text(pos2(c.left(), foot), Align2::LEFT_CENTER, &text, mono(T_META), INK2);
            let span = format!("last {}", rtt::span_words());
            if text == full && text_width(ui, &full, mono(T_META)) + text_width(ui, &span, regular(T_META)) + 16.0 <= c.width() {
                p.text(pos2(right, foot), Align2::RIGHT_CENTER, &span, regular(T_META), INK2);
            }
        }
        let all = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), CARD_H));
        a11y_text(ui, Id::unique(("latency-card", &peer.id)), all, Role::Image, &self.latency_words(peer), None);
    }

    /// "Latency to desk: 4 ms, smooth. Over the last 2 minutes: …".
    fn latency_words(&self, peer: &PeerStatus) -> String {
        if !peer.online {
            return format!("Latency to {}: offline.", peer.name);
        }
        match self.summary_of(&peer.id) {
            Some(s) => s.describe(&peer.name),
            None if self.older_daemon(&peer.id) => format!("Latency to {}: not shown, restart one·mk to see it.", peer.name),
            None => format!("Latency to {}: still measuring.", peer.name),
        }
    }

    /// The Health tab's list of computers, one compact row each, in a box
    /// from `y`; returns its height. Nothing is drawn (and 0 returned)
    /// with no computers paired.
    pub(super) fn latency_rows(&self, ui: &Ui, x: f32, y: f32, w: f32) -> f32 {
        let Some(s) = self.running().filter(|s| !s.peers.is_empty()) else { return 0.0 };
        let p = ui.painter();
        let head = 36.0;
        let h = head + ROW_H * s.peers.len() as f32 + 8.0;
        let boxr = Rect::from_min_size(pos2(x, y), vec2(w, h));
        p.rect(boxr, CornerRadius::same(10), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        p.text(pos2(x + 16.0, y + 20.0), Align2::LEFT_CENTER, "Connection to each computer", semibold(T_BODY), INK);
        let sub = format!("last {}", rtt::span_words());
        p.text(pos2(x + w - 16.0, y + 20.0), Align2::RIGHT_CENTER, &sub, regular(T_META), INK2);
        a11y_text(ui, Id::unique("latency-head"), Rect::from_min_size(boxr.min, vec2(w, head)), Role::Heading, "Connection to each computer", None);
        let now = self.rtt_now();
        // Narrow: the graph goes first, the numbers and the word stay.
        let graph_w = if w >= 520.0 { 110.0 } else { 0.0 };
        let word_w = 84.0;
        let num_w = 52.0;
        for (k, peer) in s.peers.iter().enumerate() {
            let row = Rect::from_min_size(pos2(x + 8.0, y + head + ROW_H * k as f32), vec2(w - 16.0, ROW_H));
            if k > 0 {
                p.hline(row.x_range().shrink(8.0), row.top() - 0.5, Stroke::new(1.0, LINE));
            }
            let mid = row.center().y;
            let word_right = row.right() - 8.0;
            let num_right = word_right - word_w - 8.0;
            let (words, summary) = (self.latency_words(peer), self.summary_of(&peer.id).filter(|_| peer.online));
            let name_w = row.width() - 16.0 - word_w - num_w - graph_w - if graph_w > 0.0 { 16.0 } else { 8.0 };
            let name = fit(p, &peer.name, &regular(T_BODY), name_w);
            p.text(pos2(row.left() + 8.0, mid), Align2::LEFT_CENTER, &name, regular(T_BODY), INK);
            if name != peer.name {
                ui.interact(Rect::from_min_size(pos2(row.left() + 8.0, row.top()), vec2(name_w, ROW_H)), Id::unique(("latency-name", &peer.id)), Sense::hover()).on_hover_text(&peer.name);
            }
            if !peer.online {
                p.text(pos2(word_right, mid), Align2::RIGHT_CENTER, "offline", regular(T_META), INK2);
            } else {
                let q = summary.and_then(|s| s.quality());
                let waiting = if self.older_daemon(&peer.id) { "Restart one·mk" } else { "Measuring…" };
                let word = q.map_or(waiting, Quality::word);
                p.text(pos2(word_right, mid), Align2::RIGHT_CENTER, word, regular(T_META), if q.is_some() { tone(q) } else { INK2 });
                if let Some(s) = summary {
                    p.text(pos2(num_right, mid), Align2::RIGHT_CENTER, rtt::ms(s.now_us), mono(T_META), INK);
                }
                if let (true, Some(ring), Some(s)) = (graph_w > 0.0, self.rtts.of(&peer.id), summary.filter(|s| s.count >= rtt::MIN_SAMPLES)) {
                    let g = Rect::from_min_size(pos2(num_right - num_w - 8.0 - graph_w, mid - ROW_GRAPH_H / 2.0), vec2(graph_w, ROW_GRAPH_H));
                    sparkline(p, g, ring, now, s.quality());
                }
            }
            a11y_text(ui, Id::unique(("latency-row", &peer.id)), row, Role::ListItem, &words, None);
        }
        h
    }
}
