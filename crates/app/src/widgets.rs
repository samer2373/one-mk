//! The window's controls, painted by hand but behaving like real ones:
//! each takes keyboard focus and shows it, tells assistive technology its
//! role, name and state, and has hover, pressed and disabled looks.

use crate::motion::{self, tween};
use crate::style::*;
use egui::accesskit::Role;
use egui::{Align2, Color32, CornerRadius, FontId, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

pub fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.into(), font, INK).size().x
}

pub fn button_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, semibold(T_BODY)) + 28.0
}

/// Tells assistive technology what a painted control is.
pub fn a11y(r: &Response, role: Role, label: &str, selected: Option<bool>) {
    let (label, enabled) = (label.to_owned(), r.enabled());
    r.widget_info(move || match selected {
        Some(s) => WidgetInfo::selected(role, enabled, s, &label),
        None => WidgetInfo::labeled(role, enabled, &label),
    });
}

/// A node for painted text, so screen readers can read it. `live` makes
/// changes announced (headlines, toasts).
pub fn a11y_text(ui: &Ui, id: Id, rect: Rect, role: Role, label: &str, live: Option<egui::accesskit::Live>) {
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_role(role);
        n.set_label(label);
        n.set_bounds(egui::accesskit::Rect { x0: rect.min.x as f64, y0: rect.min.y as f64, x1: rect.max.x as f64, y1: rect.max.y as f64 });
        if let Some(l) = live {
            n.set_live(l);
        }
    });
}

const KB: &str = "keyboard-focus";
const LOCK: &str = "controls-locked";

/// Rings show only while the keyboard is driving: Tab or an arrow turns
/// them on, a click turns them off.
pub fn track_focus_mode(ctx: &egui::Context) {
    let (keys, pointer) = ctx.input(|i| {
        let keys = i.events.iter().any(|e| {
            matches!(e, egui::Event::Key { key, pressed: true, .. }
                if matches!(key, egui::Key::Tab | egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown))
        });
        (keys, i.pointer.any_pressed())
    });
    if keys || pointer {
        ctx.data_mut(|d| d.insert_temp(Id::unique(KB), keys));
    }
}

pub fn keyboard_focus(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(Id::unique(KB)).unwrap_or(false))
}

/// While a sheet is open, the controls behind it stop taking input and focus.
pub fn set_locked(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::unique(LOCK), on));
}

pub fn locked(ui: &Ui) -> bool {
    ui.ctx().data(|d| d.get_temp(Id::unique(LOCK)).unwrap_or(false))
}

pub fn sense(ui: &Ui, s: Sense) -> Sense {
    if locked(ui) { Sense::hover() } else { s }
}

/// The signal-coloured ring around whatever the keyboard is on.
pub fn focus_ring(ui: &Ui, r: &Response, rect: Rect, radius: u8) {
    if r.has_focus() && keyboard_focus(ui.ctx()) {
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(radius + 2), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The one action that matters most here: amber.
    Primary,
    Quiet,
}

/// A button that presses in a little. `a11y` names it for screen readers
/// when the visible label alone would be vague.
pub fn button(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let r = ui.interact(rect, id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let down = enabled && r.is_pointer_button_down_on();
    let press = tween(&ctx, id.with("press"), f32::from(down), if down { 0.08 } else { 0.16 }, motion::out_cubic);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    // Only the shape scales: glyphs re-rasterised at a new size shimmer.
    let rr = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.03 * press));
    let radius = if rect.height() >= 32.0 { 10 } else { 8 };
    let (bg, lit, fg) = match (look, enabled) {
        (_, false) => (Color32::from_rgb(0x1E, 0x23, 0x2A), Color32::from_rgb(0x1E, 0x23, 0x2A), INK2),
        (Look::Primary, true) => (SIGNAL, SIGNAL_LIT, SIGNAL_INK),
        (Look::Quiet, true) => (BUTTON, BUTTON_LIT, INK),
    };
    let p = ui.painter();
    p.rect_filled(rr, CornerRadius::same(radius), mix(bg, lit, hover));
    focus_ring(ui, &r, rr, radius);
    p.text(rr.center(), Align2::CENTER_CENTER, label, semibold(T_BODY), fg);
    if r.hovered() && enabled {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    a11y(&r, Role::Button, a11y_label.unwrap_or(label), None);
    r
}

/// A small square button with a drawn icon; its name shows on hover.
pub fn icon_button(ui: &mut Ui, rect: Rect, id: Id, name: &str, enabled: bool, icon: impl FnOnce(&Painter, Rect, Color32)) -> Response {
    let r = ui.interact(rect, id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(7), mix(Color32::TRANSPARENT, BUTTON_LIT, hover));
    focus_ring(ui, &r, rect, 7);
    icon(p, rect, if enabled { mix(INK2, INK, hover) } else { mix(INK2, PANEL, 0.4) });
    if r.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    a11y(&r, Role::Button, name, None);
    r.on_hover_text(name)
}

/// Two arrows chasing round a circle: measure again. `turn` spins it.
pub fn draw_refresh(p: &Painter, rect: Rect, col: Color32, turn: f32) {
    let (c, r) = (rect.center(), 5.5);
    let a0 = -0.6 + turn;
    let pts: Vec<Pos2> = (0..=20).map(|i| {
        let a = a0 + i as f32 / 20.0 * 4.9;
        c + vec2(a.cos(), a.sin()) * r
    }).collect();
    let end = *pts.last().unwrap();
    let a1 = a0 + 4.9;
    let tangent = vec2(-a1.sin(), a1.cos());
    let out = vec2(a1.cos(), a1.sin());
    p.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
    p.add(egui::Shape::convex_polygon(vec![end + tangent * 3.2, end + out * 2.8, end - out * 2.8], col, Stroke::NONE));
}

/// A cross: remove.
pub fn draw_cross(p: &Painter, rect: Rect, col: Color32) {
    let (c, r) = (rect.center(), 4.0);
    let st = Stroke::new(1.5, col);
    p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], st);
    p.line_segment([c + vec2(-r, r), c + vec2(r, -r)], st);
}

/// A tick drawn as two strokes; `t` draws it in.
pub fn draw_check(p: &Painter, c: Pos2, size: f32, col: Color32, t: f32) {
    let a = c + vec2(-0.42, 0.02) * size;
    let b = c + vec2(-0.12, 0.3) * size;
    let e = c + vec2(0.44, -0.3) * size;
    let (l1, l2) = (a.distance(b), b.distance(e));
    let mut left = (l1 + l2) * t.clamp(0.0, 1.0);
    let st = Stroke::new((size * 0.13).max(1.5), col);
    let k = (left / l1).min(1.0);
    p.line_segment([a, a + (b - a) * k], st);
    left -= l1;
    if left > 0.0 {
        p.line_segment([b, b + (e - b) * (left / l2).min(1.0)], st);
    }
}

/// A labelled switch across `row`; the response is clicked when flipped.
pub fn switch(ui: &mut Ui, row: Rect, id: Id, label: &str, on: bool, enabled: bool) -> Response {
    let r = ui.interact(row, id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let knob_t = tween(&ctx, id.with("knob"), f32::from(on), motion::space(0.26), motion::back_out);
    let fill = tween(&ctx, id.with("track"), f32::from(on), motion::fade(0.2), motion::linear);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let p = ui.painter();
    p.text(row.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), if enabled { INK } else { INK2 });
    let track = Rect::from_min_size(pos2(row.right() - 34.0, row.center().y - 10.0), vec2(34.0, 20.0));
    let off = mix(TRACK_OFF, Color32::from_rgb(0x46, 0x4F, 0x5C), hover);
    // On is told by fill and knob side, in ink: amber stays for crossings
    // and the main action.
    let col = mix(off, if enabled { INK } else { INK2 }, fill);
    p.rect_filled(track, CornerRadius::same(10), col);
    p.rect_stroke(track, CornerRadius::same(10), Stroke::new(1.0, mix(TRACK_EDGE, Color32::TRANSPARENT, fill)), StrokeKind::Inside);
    let knob = pos2(track.left() + 10.0 + 14.0 * knob_t, track.center().y);
    p.circle_filled(knob, 7.5, mix(INK, PANEL, fill));
    focus_ring(ui, &r, track, 10);
    if r.hovered() && enabled {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (label, enabled) = (label.to_owned(), r.enabled());
    r.widget_info(move || WidgetInfo::selected(Role::Switch, enabled, on, &label));
    r
}

/// A row of choices with a pill that slides to the chosen one: the tabs
/// and the segmented controls. Arrow keys move along it and choose.
pub struct Pills<'a> {
    pub id: Id,
    pub track: Rect,
    pub cells: Vec<Rect>,
    pub labels: &'a [&'a str],
    /// What each choice is called for screen readers.
    pub names: Vec<String>,
    pub chosen: usize,
    pub enabled: Vec<bool>,
    pub role: Role,
    pub font: f32,
    pub radius: u8,
}

impl Pills<'_> {
    /// Equal cells across `track`, inset by `inset`.
    pub fn even(track: Rect, n: usize, inset: f32) -> Vec<Rect> {
        let w = (track.width() - inset * 2.0) / n as f32;
        (0..n).map(|k| Rect::from_min_size(pos2(track.left() + inset + k as f32 * w, track.top() + inset), vec2(w, track.height() - inset * 2.0))).collect()
    }

    /// Paints the control; returns the choice made this frame, if any.
    pub fn show(&self, ui: &mut Ui, arrowed: bool) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        p.rect(self.track, CornerRadius::same(self.radius + 3), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let to = self.cells[self.chosen.min(self.cells.len() - 1)];
        let secs = motion::space(0.22);
        let x = tween(&ctx, self.id.with("x"), to.left(), secs, motion::out_quart);
        let w = tween(&ctx, self.id.with("w"), to.width(), secs, motion::out_quart);
        p.rect_filled(Rect::from_min_size(pos2(x, to.top()), vec2(w, to.height())), CornerRadius::same(self.radius), PILL);
        let mut picked = None;
        // egui moves focus with the arrows but doesn't report it as gained,
        // so remember which choice had it last frame.
        let was = ctx.data(|d| d.get_temp::<Option<usize>>(self.id.with("focus")).flatten());
        let mut now_focused = None;
        for (k, cell) in self.cells.iter().enumerate() {
            let enabled = self.enabled[k];
            let r = ui.interact(*cell, self.id.with(k), if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
            let on = k == self.chosen;
            // Arrowing onto a choice chooses it, as radio groups do.
            if r.has_focus() {
                now_focused = Some(k);
            }
            if enabled && !on && (r.clicked() || (r.has_focus() && arrowed && was != Some(k))) {
                picked = Some(k);
            }
            let lit = tween(&ctx, self.id.with(("on", k)), f32::from(on), motion::fade(0.15), motion::linear);
            let base = if !enabled { mix(INK2, RAISED, 0.5) } else if r.hovered() { INK_HOVER } else { INK2 };
            let font = if on { semibold(self.font) } else { regular(self.font) };
            let text = fit(&p, self.labels[k], &font, cell.width() - 8.0);
            p.text(cell.center(), Align2::CENTER_CENTER, text, font, mix(base, INK, lit));
            focus_ring(ui, &r, *cell, self.radius);
            if r.hovered() && enabled && !on {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let (name, enabled) = (self.names[k].clone(), r.enabled());
            let role = self.role;
            r.widget_info(move || WidgetInfo::selected(role, enabled, on, &name));
        }
        ctx.data_mut(|d| d.insert_temp(self.id.with("focus"), now_focused));
        picked
    }
}

/// Wrapped text; returns its height.
pub fn para(ui: &Ui, pos: Pos2, width: f32, text: &str, font: FontId, col: Color32) -> f32 {
    let size = font.size;
    let mut job = egui::text::LayoutJob::simple(text.into(), font, col, width);
    job.sections[0].format.line_height = Some((size * LEADING).round());
    let g = ui.painter().layout_job(job);
    let h = g.size().y;
    ui.painter().galley(pos, g, col);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_ends_at_each_colour() {
        assert_eq!(mix(INK, SIGNAL, 0.0), INK);
        assert_eq!(mix(INK, SIGNAL, 1.0), SIGNAL);
        assert_eq!(mix(INK, SIGNAL, 7.0), SIGNAL);
    }

    /// Focus one choice, press →, and the next one is chosen.
    #[test]
    fn arrows_choose_along_a_row() {
        let ctx = egui::Context::default();
        let labels = ["Left", "Right", "Above"];
        let (mut chosen, mut arrowed) = (0, false);
        for frame in 0..4 {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() };
            let key = frame == 1;
            if key {
                input.events.push(egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
            }
            let mut out = ctx.run_ui(input, |ui| {
                {
                    let track = Rect::from_min_size(pos2(10.0, 10.0), vec2(300.0, 28.0));
                    let pills = Pills {
                        id: Id::unique("test-pills"),
                        track,
                        cells: Pills::even(track, 3, 2.0),
                        labels: &labels,
                        names: labels.iter().map(|l| (*l).to_owned()).collect(),
                        chosen,
                        enabled: vec![true; 3],
                        role: Role::RadioButton,
                        font: T_BODY,
                        radius: 6,
                    };
                    if let Some(k) = pills.show(ui, arrowed) {
                        chosen = k;
                    }
                    if frame == 0 {
                        ui.memory_mut(|m| m.request_focus(Id::unique("test-pills").with(0usize)));
                    }
                }
            });
            out.textures_delta.clear();
            arrowed = key;
        }
        assert_eq!(chosen, 1);
    }

    #[test]
    fn even_cells_fill_the_track() {
        let track = Rect::from_min_size(pos2(10.0, 0.0), vec2(206.0, 28.0));
        let cells = Pills::even(track, 4, 3.0);
        assert_eq!(cells.len(), 4);
        assert!((cells[0].left() - 13.0).abs() < 1e-4);
        assert!((cells[3].right() - 213.0).abs() < 1e-4);
        assert!((cells[1].height() - 22.0).abs() < 1e-4);
    }
}
