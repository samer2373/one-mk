//! The live latency graph: a card under the selected computer on the Desk
//! tab, and a compact row per computer on the Health tab. It draws what
//! `crate::rtt` works out; nothing here moves.

use super::*;
use crate::rtt::{self, Quality, Ring, Summary};

/// Health's graph beside a computer's name.
const HEALTH_GRAPH_H: f32 = 28.0;
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

/// Health's graph, in the box `r`: the last couple of minutes as a line,
/// drawn for a page: a faint baseline at half height, a soft fill under the line,
/// the newest reading as a dot on a halo, lost packets as rings.
fn health_trace(p: &Painter, r: Rect, ring: &Ring, now: f64, q: Option<Quality>) {
    let t = ring.trace(now, r.width(), r.height());
    let col = tone(q);
    let line_col = col;
    let width = if q == Some(Quality::Slow) { 2.0 } else { 1.5 };
    let mid = (r.center().y).round() + 0.5;
    p.add(egui::Shape::dashed_line(&[pos2(r.left(), mid), pos2(r.right(), mid)], Stroke::new(1.0, pal().line), 2.0, 3.0));
    p.hline(r.x_range(), r.bottom().round() - 0.5, Stroke::new(1.0, pal().line2));
    for line in &t.lines {
        let pts: Vec<Pos2> = line.iter().map(|[x, y]| r.min + vec2(*x, *y)).collect();
        if pts.len() >= 2 {
            // The soft fill: a strip from the line down to the bottom, fading out.
            let mut mesh = egui::epaint::Mesh::default();
            let (top_c, low_c) = (line_col.gamma_multiply(0.16), line_col.gamma_multiply(0.0));
            for p in &pts {
                mesh.colored_vertex(*p, top_c);
                mesh.colored_vertex(pos2(p.x, r.bottom() - 1.0), low_c);
            }
            for k in 0..pts.len() as u32 - 1 {
                let (a, b) = (2 * k, 2 * k + 2);
                mesh.add_triangle(a, b, a + 1);
                mesh.add_triangle(b, b + 1, a + 1);
            }
            p.add(egui::Shape::mesh(mesh));
            p.add(egui::Shape::line(pts, Stroke::new(width, line_col)));
        } else if let [one] = pts[..] {
            p.circle_filled(one, 1.5, line_col);
        }
    }
    if let Some([x, y]) = t.lines.last().and_then(|l| l.last()) {
        let at = r.min + vec2(*x, *y);
        let dot = if col == pal().ink2 { pal().ok } else { col };
        p.circle_filled(at, 5.0, dot.gamma_multiply(0.2));
        p.circle_filled(at, 2.5, dot);
    }
    for x in &t.losses {
        let x = r.left() + x;
        p.line_segment([pos2(x, r.top() + 5.0), pos2(x, r.bottom() - 1.0)], Stroke::new(1.0, pal().warn.gamma_multiply(0.6)));
        p.circle_stroke(pos2(x, r.top() + 2.5), 2.5, Stroke::new(1.0, pal().warn));
    }
}

/// A round trip as a page shows it: the number in JetBrains Mono at 500,
/// its unit quieter after it. Painted right-aligned at `right`; returns
/// how wide it came out.
fn paint_ms(p: &Painter, right: Pos2, text: &str, ink: Color32) -> f32 {
    let (number, rest) = text.split_once(' ').map_or((text, ""), |(n, r)| (n, r));
    let mut job = egui::text::LayoutJob::default();
    job.append(number, 0.0, egui::TextFormat { font_id: mono_medium(T_BODY + 1.0), color: ink, ..Default::default() });
    if !rest.is_empty() {
        job.append(&format!(" {rest}"), 0.0, egui::TextFormat { font_id: mono(T_META), color: pal().ink2, ..Default::default() });
    }
    let galley = p.layout_job(job);
    let w = galley.size().x;
    p.galley(pos2(right.x - w, right.y - galley.size().y / 2.0), galley, ink);
    w
}

impl App {
    /// The clock the graphs are drawn on: the window's own, or the moment
    /// a scripted history (a screenshot) ended.
    pub(super) fn rtt_now(&self) -> f64 {
        self.rtt_frozen.unwrap_or(self.now)
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
        let warm = matches!(quality, Some(Quality::Stalls | Quality::Slow));
        let vw = if mono_value {
            paint_ms(p, pos2(row.right(), mid), &value, if warm { pal().warn } else { pal().ink })
        } else {
            p.text(pos2(row.right(), mid), Align2::RIGHT_CENTER, &value, regular(T_BODY), pal().ink2).width()
        };
        // The line, when the row has room for it beside the name.
        let graph_w = if row.width() >= 520.0 { 150.0 } else if row.width() >= 400.0 { 104.0 } else { 0.0 };
        let name_room = row.width() - 26.0 - vw.max(44.0) - 16.0 - if graph_w > 0.0 { graph_w + 16.0 } else { 0.0 };
        let name = fit(p, &peer.name, &regular(T_BODY), name_room.max(40.0));
        p.text(pos2(row.left() + 22.0, mid), Align2::LEFT_CENTER, &name, regular(T_BODY), pal().ink);
        if name != peer.name {
            ui.interact(Rect::from_min_size(pos2(row.left() + 22.0, row.top()), vec2(name_room.max(40.0), row.height())), Id::unique(("latency-name", &peer.id)), Sense::hover()).on_hover_text(&peer.name);
        }
        if let (true, Some(ring), Some(s)) = (graph_w > 0.0, self.rtts.of(&peer.id), summary.filter(|s| s.count >= rtt::MIN_SAMPLES)) {
            let g = Rect::from_min_size(pos2(row.right() - vw.max(44.0) - 16.0 - graph_w, mid - HEALTH_GRAPH_H / 2.0), vec2(graph_w, HEALTH_GRAPH_H));
            health_trace(p, g, ring, self.rtt_now(), s.quality());
        }
        a11y_text(ui, Id::unique(("latency-row", &peer.id)), row, Role::ListItem, &words, None);
    }
}
