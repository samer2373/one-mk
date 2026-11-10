//! The live latency graph: a card under the selected computer on the Desk
//! tab, and a compact row per computer on the Health tab. It draws what
//! `crate::rtt` works out; nothing here moves.

use super::*;
use crate::rtt::{self, Quality, Ring, Summary};

const ROW_GRAPH_H: f32 = 22.0;
/// A computer's row in Health's Connections card.
pub(super) const ROW_H: f32 = 52.0;

/// The colour for a state: calm grey while it is smooth, amber when not.
/// The word beside it says the same, so colour is never the only signal.
fn tone(q: Option<Quality>) -> Color32 {
    match q {
        Some(Quality::Stalls | Quality::Slow) => pal().warn,
        _ => pal().ink2,
    }
}

/// The line, in the box `r`: grey or amber, with a dot at the newest
/// reading and a marker where packets were lost.
fn sparkline(p: &Painter, r: Rect, ring: &Ring, now: f64, q: Option<Quality>) {
    let t = ring.trace(now, r.width(), r.height());
    let col = tone(q);
    let width = if q == Some(Quality::Slow) { 2.0 } else { 1.5 };
    p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, pal().line));
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
        // The newest reading: cyan while it answers well (live, connected),
        // amber when it doesn't.
        p.circle_filled(r.min + vec2(*x, *y), 2.5, if col == pal().ink2 { pal().ok } else { col });
    }
    // Lost packets: a thin amber line top to bottom ending in a ring, so
    // it reads apart from a slow reading and not only by colour.
    for x in &t.losses {
        let x = r.left() + x;
        p.line_segment([pos2(x, r.top() + 4.0), pos2(x, r.bottom())], Stroke::new(1.0, pal().warn.gamma_multiply(0.6)));
        p.circle_stroke(pos2(x, r.top() + 2.5), 2.5, Stroke::new(1.0, pal().warn));
    }
}

impl App {
    /// The clock the graphs are drawn on: the window's own, or the moment
    /// a scripted history (a screenshot) ended.
    pub(super) fn rtt_now(&self) -> f64 {
        self.rtt_frozen.unwrap_or(self.now)
    }

    /// The panel under the desk: tall enough for the inspector with the
    /// latency card, on the Desk tab once a computer is paired.
    pub(super) fn panel_h(&self) -> f32 {
        if self.tab == Tab::Files {
            // The page scrolls, so the desk above keeps enough room for its
            // monitors to stay readable drop targets in a small window.
            // Narrow, the desk above is a row of named chips: the page takes the room.
            if self.ctx.content_rect().width() < 1040.0 {
                // Chips need a row (two at most): the page takes the rest.
                let peers = self.running().map_or(0, |s| s.peers.len());
                let chips = if peers > 3 { 2.0 * 52.0 } else { 52.0 };
                return (self.ctx.content_rect().height() - shell::HEAD_H - chips - 3.0 * M).max(210.0);
            }
            return (self.ctx.content_rect().height() * 0.56).clamp(210.0, 460.0);
        }
        // A narrow window stacks Arrange's cards: give them half the height.
        if self.tab == Tab::Arrange && self.ctx.content_rect().width() < 1040.0 {
            return (self.ctx.content_rect().height() * 0.46).max(PANEL_H);
        }
        let with_card = self.tab == Tab::Desk && self.running().is_some_and(|s| !s.peers.is_empty());
        let short = self.ctx.content_rect().height() < 640.0;
        match (with_card, short) {
            (true, false) => 228.0,
            (true, true) => 220.0,
            (false, _) => PANEL_H,
        }
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

    pub(super) fn summary_of(&self, id: &str) -> Option<Summary> {
        self.rtts.of(id).and_then(|r| r.summary(self.rtt_now()))
    }

    /// "Latency to desk: 4 ms, smooth. Over the last 2 minutes: …".
    pub(super) fn latency_words(&self, peer: &PeerStatus) -> String {
        if !peer.online {
            return format!("Latency to {}: offline.", peer.name);
        }
        match self.summary_of(&peer.id) {
            Some(s) => s.describe(&peer.name),
            None => format!("Latency to {}: no samples yet.", peer.name),
        }
    }

    /// One computer's row in Health's Connections card, in `row`: a status
    /// mark (● answering well, ◆ stalls or slow, ○ offline), its name, the
    /// last couple of minutes as a line when there is room, and the round
    /// trip in mono with what it says.
    pub(super) fn connection_row(&self, ui: &Ui, peer: &PeerStatus, row: Rect) {
        let p = ui.painter();
        let mid = row.center().y;
        let (words, summary) = (self.latency_words(peer), self.summary_of(&peer.id).filter(|_| peer.online));
        let quality = summary.and_then(|s| s.quality());
        let mark = if !peer.online {
            widgets::Mark::Off
        } else if matches!(quality, Some(Quality::Stalls | Quality::Slow)) {
            widgets::Mark::Warn
        } else {
            widgets::Mark::Ok
        };
        widgets::status_mark(p, pos2(row.left() + 4.0, mid), 8.0, mark);
        // The value at the right: "4 ms", "38 ms · some stalls", or a word.
        let (value, mono_value) = match (peer.online, summary) {
            (false, _) => ("Offline".to_owned(), false),
            (true, Some(s)) => match quality {
                Some(q) if q != Quality::Smooth => (format!("{} · {}", rtt::ms(s.now_us), q.word().to_lowercase()), true),
                _ => (rtt::ms(s.now_us), true),
            },
            (true, None) => ("No samples yet".to_owned(), false),
        };
        let font = if mono_value { mono(T_BODY) } else { regular(T_BODY) };
        let ink = if mono_value { pal().ink } else { pal().ink2 };
        let vw = text_width(ui, &value, font.clone());
        p.text(pos2(row.right(), mid), Align2::RIGHT_CENTER, &value, font, ink);
        // The line, when the row has room for it beside the name.
        let graph_w = if row.width() >= 440.0 { 96.0 } else { 0.0 };
        let name_room = row.width() - 26.0 - vw - 12.0 - if graph_w > 0.0 { graph_w + 12.0 } else { 0.0 };
        let name = fit(p, &peer.name, &regular(T_BODY), name_room.max(40.0));
        p.text(pos2(row.left() + 22.0, mid), Align2::LEFT_CENTER, &name, regular(T_BODY), pal().ink);
        if name != peer.name {
            ui.interact(Rect::from_min_size(pos2(row.left() + 22.0, row.top()), vec2(name_room.max(40.0), row.height())), Id::unique(("latency-name", &peer.id)), Sense::hover()).on_hover_text(&peer.name);
        }
        if let (true, Some(ring), Some(s)) = (graph_w > 0.0, self.rtts.of(&peer.id), summary.filter(|s| s.count >= rtt::MIN_SAMPLES)) {
            let g = Rect::from_min_size(pos2(row.right() - vw - 12.0 - graph_w, mid - ROW_GRAPH_H / 2.0), vec2(graph_w, ROW_GRAPH_H));
            sparkline(p, g, ring, self.rtt_now(), s.quality());
        }
        a11y_text(ui, Id::unique(("latency-row", &peer.id)), row, Role::ListItem, &words, None);
    }
}
