//! The window's controls, painted by hand but behaving like real ones:
//! each takes keyboard focus and shows it, tells assistive technology its
//! role, name and state, and has hover, pressed and disabled looks.

use crate::motion::{self, tween};
use crate::style::*;
use egui::accesskit::Role;
use egui::{Align2, Color32, CornerRadius, FontId, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};

pub use crate::style::mix;

pub fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.into(), font, pal().ink).size().x
}

pub fn button_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, semibold(T_BODY)) + 28.0
}

/// The smallest a pointer target may be, in points (WCAG 2.5.8 and the
/// council's floor): a control may look smaller, its hit area doesn't.
pub const MIN_TARGET: f32 = 24.0;

/// Where a control takes the pointer: its own rectangle, grown about its
/// centre to at least `MIN_TARGET` square.
pub fn hit_rect(rect: Rect) -> Rect {
    Rect::from_center_size(rect.center(), rect.size().max(vec2(MIN_TARGET, MIN_TARGET)))
}

/// `fill` as it looks while pressed (`t` 0..1): the pressed look shared by
/// every control family, as deep as the theme presses.
pub fn pressed(fill: Color32, t: f32) -> Color32 {
    pal().pressed(fill, t)
}

pub fn pressed_lift(fill: Color32, t: f32) -> Color32 {
    pressed(fill, t)
}

#[cfg(test)]
pub fn contrast(first: Color32, second: Color32) -> f64 {
    let luminance = |color: Color32| {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    };
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

pub fn pressed_ink(_surface: Color32, normal: Color32) -> Color32 {
    normal
}

fn switch_color(color: Color32, enabled: bool) -> Color32 {
    if enabled { color } else { mix(color, pal().surface, 0.6) }
}

/// Tells assistive technology what a painted control is.
/// Development aid (`ONE_MK_APP_TARGETS=file`): every named control and
/// where it is, written out each frame, so tests can click them by name.
#[cfg(any(debug_assertions, feature = "design-evidence"))]
pub mod targets {
    use std::sync::Mutex;
    static NOW: Mutex<Vec<(String, [f32; 4])>> = Mutex::new(Vec::new());
    /// The last finished frame's controls.
    static LAST: Mutex<Vec<(String, [f32; 4])>> = Mutex::new(Vec::new());

    pub fn note(label: &str, r: egui::Rect) {
        if std::env::var_os("ONE_MK_APP_TARGETS").is_some() {
            NOW.lock().unwrap_or_else(|e| e.into_inner()).push((label.to_owned(), [r.min.x, r.min.y, r.max.x, r.max.y]));
        }
    }

    static NAMES: Mutex<Option<std::collections::HashMap<egui::Id, String>>> = Mutex::new(None);

    /// The name a control (by id) was given; survives scrolling.
    pub fn note_id(id: egui::Id, label: &str) {
        if std::env::var_os("ONE_MK_APP_TARGETS").is_some() {
            NAMES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(id, label.to_owned());
        }
    }

    /// Where last frame's control called `label` (exactly) was.
    pub fn rect_of(label: &str) -> Option<egui::Rect> {
        let last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        last.iter().find(|(l, _)| l == label).map(|(_, q)| egui::Rect::from_min_max(egui::pos2(q[0], q[1]), egui::pos2(q[2], q[3])))
    }

    pub fn name_of(id: egui::Id) -> Option<String> {
        NAMES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&id).cloned())
    }

    /// The name noted last frame for the control at `r`, if any.
    pub fn name_at(r: egui::Rect) -> Option<String> {
        let now = LAST.lock().unwrap_or_else(|e| e.into_inner());
        now.iter().rev().find(|(_, q)| (q[0] - r.min.x).abs() < 0.5 && (q[1] - r.min.y).abs() < 0.5 && (q[2] - r.max.x).abs() < 0.5 && (q[3] - r.max.y).abs() < 0.5).map(|(l, _)| l.clone())
    }

    /// Writes this frame's controls (as JSON lines) and starts the next.
    pub fn flush() {
        let Some(path) = std::env::var_os("ONE_MK_APP_TARGETS") else { return };
        let all = std::mem::take(&mut *NOW.lock().unwrap_or_else(|e| e.into_inner()));
        LAST.lock().unwrap_or_else(|e| e.into_inner()).clone_from(&all);
        let text: String = all.iter().map(|(l, r)| format!("{}\t{}\t{}\t{}\t{}\n", l.replace(['\t', '\n'], " "), r[0], r[1], r[2], r[3])).collect();
        let tmp = std::path::PathBuf::from(&path).with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

pub fn a11y(r: &Response, role: Role, label: &str, selected: Option<bool>) {
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note(label, r.rect);
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note_id(r.id, label);
    // Tabbed onto, a control is brought into view (a list that scrolls).
    if r.gained_focus() && keyboard_focus(&r.ctx) {
        r.scroll_to_me(None);
    }
    let (label, enabled) = (label.to_owned(), r.enabled());
    r.widget_info(move || match selected {
        Some(s) => WidgetInfo::selected(role, enabled, s, &label),
        None => WidgetInfo::labeled(role, enabled, &label),
    });
}

/// A node for painted text, so screen readers can read it. `live` makes
/// changes announced (headlines, toasts).
pub fn a11y_text(ui: &Ui, id: Id, rect: Rect, role: Role, label: &str, live: Option<egui::accesskit::Live>) {
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note(label, rect);
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_role(role);
        // A label's words are its value (AccessKit's model): set both, so
        // AT-SPI and the others read them as the name.
        if role == Role::Label {
            n.set_value(label);
        }
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

/// The focus ring round whatever the keyboard is on: 2 points of the
/// theme's focus colour, 2 points outside the control.
pub fn focus_ring(ui: &Ui, r: &Response, rect: Rect, radius: u8) {
    if r.has_focus() && keyboard_focus(ui.ctx()) {
        paint_focus(ui.painter(), rect, radius);
    }
}

/// The ring itself (also for the widget gallery).
pub fn paint_focus(p: &Painter, rect: Rect, radius: u8) {
    p.rect_stroke(rect.expand(2.0), CornerRadius::same(radius.saturating_add(2)), Stroke::new(2.0, pal().focus), StrokeKind::Outside);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// The one action that matters most here: the accent.
    Primary,
    /// Everything else: a quiet `surface2` fill.
    Quiet,
    /// A link: no fill, the accent's text colour.
    #[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))]
    Text,
    /// A destructive link: no fill, the danger colour.
    Danger,
}

/// How a control is being touched, 0..1 each (tweened by its caller).
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub hover: f32,
    pub press: f32,
    pub enabled: bool,
    pub focus: bool,
}

impl State {
    #[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))] // The widget gallery (debug) and the new layout use it.
    pub const REST: State = State { hover: 0.0, press: 0.0, enabled: true, focus: false };
}

/// The handoff's button corner: 10, 9 for the small ones (34 and less).
pub fn button_radius(height: f32) -> u8 {
    if height <= 34.5 { 9 } else { 10 }
}

/// A button's label face: 600 for the primary and links, 500 otherwise.
pub fn button_font(look: Look) -> FontId {
    match look {
        Look::Primary | Look::Text | Look::Danger => semibold(T_BODY),
        Look::Quiet => medium(T_BODY),
    }
}

/// Paints a button in `rect` (pressing scales it to 0.98); returns the
/// shape's rectangle.
pub fn paint_button(p: &Painter, rect: Rect, label: &str, look: Look, s: State) -> Rect {
    // Only the shape scales: glyphs re-rasterised at a new size shimmer.
    let rr = Rect::from_center_size(rect.center(), rect.size() * (1.0 - motion::space(0.02) * s.press));
    let radius = CornerRadius::same(button_radius(rect.height()));
    let pal = pal();
    let fg = match look {
        _ if !s.enabled => pal.ink3,
        Look::Primary => pal.on_accent,
        Look::Quiet => pal.ink,
        Look::Text => pal.accent_text,
        Look::Danger => pal.danger,
    };
    match look {
        Look::Primary | Look::Quiet if !s.enabled => {
            p.rect_filled(rr, radius, pal.surface2);
        }
        // Flat, solid accent: no gradient, no rim, no glow; 6 % deeper under
        // the pointer, 10 % deeper pressed.
        Look::Primary => {
            p.rect_filled(rr, radius, mix(mix(pal.accent, pal.accent_hover, s.hover), pal.accent_press, s.press));
        }
        // Quiet: `surface2`, the `line2` overlay under the pointer.
        Look::Quiet => {
            p.rect_filled(rr, radius, pressed(mix(pal.surface2, pal.surface2_hover, s.hover), s.press));
        }
        // Links: no fill; a faint wash under the pointer.
        Look::Text | Look::Danger => {
            p.rect_filled(rr, radius, pal.hover.gamma_multiply(s.hover.max(s.press)));
        }
    }
    if s.focus {
        paint_focus(p, rr, button_radius(rect.height()));
    }
    p.text(rr.center(), Align2::CENTER_CENTER, label, button_font(look), fg);
    rr
}

/// A button that presses in a little. `a11y` names it for screen readers
/// when the visible label alone would be vague.
pub fn button(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let rect = Rect::from_center_size(rect.center(), vec2(rect.width().max(button_width(ui, label)), rect.height()));
    button_painted(ui, rect, id, label, look, enabled, a11y_label)
}

pub fn content_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let shown = fit(ui.painter(), label, &button_font(look), (rect.width() - 18.0).max(0.0));
    button_painted(ui, rect, id, &shown, look, enabled, Some(a11y_label.unwrap_or(label)))
}

fn button_painted(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let r = ui.interact(hit_rect(rect), id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let down = enabled && r.is_pointer_button_down_on();
    let press = tween(&ctx, id.with("press"), f32::from(down), motion::fade(if down { 0.06 } else { 0.14 }), motion::out_cubic);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.1, motion::out_cubic);
    let focus = r.has_focus() && keyboard_focus(&ctx);
    paint_button(ui.painter(), rect, label, look, State { hover, press, enabled, focus });
    if r.hovered() && enabled {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    a11y(&r, Role::Button, a11y_label.unwrap_or(label), None);
    r
}

/// A small square button with a drawn icon; its name shows on hover.
pub fn icon_button(ui: &mut Ui, rect: Rect, id: Id, name: &str, enabled: bool, icon: impl FnOnce(&Painter, Rect, Color32)) -> Response {
    let r = ui.interact(hit_rect(rect), id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.1, motion::out_cubic);
    let down = enabled && r.is_pointer_button_down_on();
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), if down { pressed_lift(pal().surface2_hover, 1.0) } else { mix(Color32::TRANSPARENT, pal().surface2_hover, hover) });
    focus_ring(ui, &r, rect, 8);
    let ink = if enabled { mix(pal().ink2, pal().ink, hover) } else { mix(pal().ink2, pal().surface, 0.4) };
    icon(p, rect, if down { pressed_ink(pressed_lift(pal().surface2_hover, 1.0), ink) } else { ink });
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

/// A row's label. With help (see `help.rs`) a caption sits under it and an
/// "i" opens the long explanation and its picture; `right` is the room the
/// control keeps free. Returns the label as shown.
pub fn label_help(ui: &mut Ui, row: Rect, id: Id, label: &str, key: &str, enabled: bool, right: f32) -> String {
    let font = regular(T_BODY);
    let col = if enabled { pal().ink } else { pal().ink2 };
    let Some(h) = crate::help::get(key) else {
        let shown = fit(ui.painter(), label, &font, row.width() - right);
        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, &shown, font, col);
        return shown;
    };
    let room = row.width() - right - 26.0;
    let shown = fit(ui.painter(), label, &font, room);
    let p = ui.painter();
    // A tall row has room for the caption; a short one keeps only the "i".
    if row.height() >= 44.0 {
        p.text(row.left_center() - vec2(0.0, 8.0), Align2::LEFT_CENTER, &shown, font, col);
        p.text(row.left_center() + vec2(0.0, 8.0), Align2::LEFT_CENTER, fit(p, h.caption, &regular(T_META), room), regular(T_META), pal().ink2);
    } else {
        p.text(row.left_center(), Align2::LEFT_CENTER, &shown, font, col);
    }
    // The "i" follows the label (16 across), on the label's own line.
    let label_y = if row.height() >= 44.0 { row.center().y - 8.0 } else { row.center().y };
    let ic = pos2(row.left() + text_width(ui, &shown, regular(T_BODY)) + 18.0, label_y);
    let ir = Rect::from_center_size(ic, vec2(MIN_TARGET, MIN_TARGET));
    let r = ui.interact(ir, id.with("info"), sense(ui, Sense::click()));
    let hover = tween(ui.ctx(), id.with("info-hover"), f32::from(r.hovered()), 0.1, motion::out_cubic);
    let p = ui.painter();
    let ink = mix(pal().ink2, pal().ink, hover);
    let well = if r.is_pointer_button_down_on() { pressed_lift(pal().surface2_hover, 1.0) } else { mix(Color32::TRANSPARENT, pal().surface2, hover) };
    let ink = if r.is_pointer_button_down_on() { pressed_ink(well, ink) } else { ink };
    p.circle_filled(ic, 11.0, well);
    p.circle_stroke(ic, 8.0, Stroke::new(1.0, ink));
    p.text(ic, Align2::CENTER_CENTER, "i", semibold(T_META), ink);
    focus_ring(ui, &r, ir, 11);
    a11y(&r, Role::Button, &format!("About {label}"), None);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let pop = egui::Popup::menu(&r).width(300.0).align(egui::RectAlign::BOTTOM_START);
    // Development aid: ONE_MK_APP_HELP=<key> keeps that explanation open.
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    let pop = if std::env::var("ONE_MK_APP_HELP").is_ok_and(|v| v == key) { pop.open(true) } else { pop };
    pop.show(|ui| {
        // A picture on its own well, then the sentence, inset with the menu's margin.
        const W: f32 = 300.0;
        const IN: f32 = 8.0;
        ui.set_max_width(W);
        let mut job = egui::text::LayoutJob::simple(h.detail.to_owned(), regular(T_BODY), pal().ink_hover, W - 2.0 * IN);
        job.sections[0].format.line_height = Some((T_BODY * LEADING).round());
        let text = ui.painter().layout_job(job);
        let pic = if h.scene != crate::help::Scene::None { 112.0 } else { 0.0 };
        let (all, _) = ui.allocate_exact_size(vec2(W, IN + pic + if pic > 0.0 { IN } else { 0.0 } + text.size().y + IN - 2.0), Sense::hover());
        if pic > 0.0 {
            let scene = Rect::from_min_size(all.min + vec2(IN, IN), vec2(W - 2.0 * IN, pic));
            ui.painter().rect_filled(scene, CornerRadius::same(R_INNER), pal().bg);
            // The picture plays twice from when the explainer opens, then
            // rests on its last frame: an open explainer costs nothing at
            // rest. Opened again, it plays again.
            const LOOP: f64 = 4.0;
            let clock = ui.input(|i| i.time);
            let key = id.with("help-clock");
            let frame = ui.ctx().cumulative_frame_nr();
            let (start, seen) = ui.ctx().data(|d| d.get_temp::<(f64, u64)>(key)).unwrap_or((clock, frame));
            let start = crate::help::playback_start(clock, frame, start, seen);
            ui.ctx().data_mut(|d| d.insert_temp(key, (start, frame)));
            let elapsed = clock - start;
            let playing = !motion::reduced() && elapsed < 2.0 * LOOP;
            // At rest (and with reduced motion) the picture's last frame.
            let t = if playing { crate::help::phase(elapsed, LOOP) } else { 0.9 };
            crate::help::draw(ui.painter(), scene, h.scene, t);
            if playing {
                ui.ctx().request_repaint_after_secs(1.0 / 24.0);
            }
        }
        let y = all.top() + IN + if pic > 0.0 { pic + IN } else { 0.0 };
        ui.painter().galley(pos2(all.left() + IN, y), text, pal().ink_hover);
        a11y_text(ui, id.with("detail"), all, Role::Label, h.detail, None);
    });
    shown
}

/// `label_help` for a label with its control right after it (not at the
/// far end); returns where that control may start.
pub fn label_help_inline(ui: &mut Ui, row: Rect, id: Id, label: &str, key: &str, enabled: bool) -> f32 {
    let w = text_width(ui, label, regular(T_BODY));
    let extra = if crate::help::get(key).is_some() { 26.0 } else { 0.0 };
    label_help(ui, row, id, label, key, enabled, row.width() - w - extra);
    w + extra + 16.0
}

/// A labelled switch across `row`, with the caption and explanation filed
/// under `key` in `help.rs`; the response is clicked when flipped.
pub fn switch_help(ui: &mut Ui, row: Rect, id: Id, label: &str, key: &str, on: bool, enabled: bool) -> Response {
    switch_help_painted(ui, row, id, label, key, on, (enabled, false))
}

pub fn switch_help_dimmed(ui: &mut Ui, row: Rect, id: Id, label: &str, key: &str, on: bool, enabled: bool) -> Response {
    switch_help_painted(ui, row, id, label, key, on, (enabled, true))
}

fn switch_help_painted(ui: &mut Ui, row: Rect, id: Id, label: &str, key: &str, on: bool, availability: (bool, bool)) -> Response {
    let (enabled, dim_disabled) = availability;
    let enabled = enabled && (!dim_disabled || ui.is_enabled());
    let undimmed = enabled || !dim_disabled;
    let r = ui.interact(hit_rect(row), id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let press = tween(&ctx, id.with("press"), f32::from(enabled && r.is_pointer_button_down_on()), motion::fade(0.06), motion::out_cubic);
    let knob_t = tween(&ctx, id.with("knob"), f32::from(on), motion::space(0.26), motion::back_out);
    let fill = tween(&ctx, id.with("track"), f32::from(on), motion::fade(0.2), motion::linear);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let shown = label_help(ui, row, id, label, key, enabled, 44.0);
    let track = Rect::from_min_size(pos2(row.right() - SWITCH.x, row.center().y - SWITCH.y / 2.0), SWITCH);
    paint_switch(ui.painter(), track, Switch { on, knob: knob_t, fill, undimmed }, State { hover, press, enabled, focus: r.has_focus() && keyboard_focus(&ctx) });
    if r.gained_focus() && keyboard_focus(&ctx) {
        r.scroll_to_me(None);
    }
    if r.hovered() && enabled {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note(label, r.rect);
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note_id(r.id, label);
    let (label, enabled) = (label.to_owned(), enabled && r.enabled());
    let cut = shown != label;
    r.widget_info({
        let label = label.clone();
        move || WidgetInfo::selected(Role::Switch, enabled, on, &label)
    });
    if cut { r.on_hover_text(label) } else { r }
}

/// A switch's track: 36 × 22.
pub const SWITCH: egui::Vec2 = egui::Vec2 { x: 36.0, y: 22.0 };

/// Where a switch is: on or off, and how far its knob (back-out, 260 ms)
/// and its fill (200 ms) have gone toward on.
#[derive(Clone, Copy, Debug)]
pub struct Switch {
    pub on: bool,
    pub knob: f32,
    pub fill: f32,
    /// Drawn at full strength even when disabled.
    pub undimmed: bool,
}

impl Switch {
    #[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))] // The widget gallery (debug) and the new layout use it.
    pub fn at(on: bool) -> Switch {
        let t = f32::from(on);
        Switch { on, knob: t, fill: t, undimmed: false }
    }
}

/// Paints a switch in `track` (any size; the handoff's is [`SWITCH`]):
/// `switch_off` or the accent, a white knob 2 points in from the edge.
pub fn paint_switch(p: &Painter, track: Rect, sw: Switch, s: State) {
    let pal = pal();
    let r = (track.height() / 2.0).round() as u8;
    let undimmed = s.enabled || sw.undimmed;
    let off = mix(pal.switch_off, pal.switch_off_lit, s.hover);
    let on_col = if s.enabled { mix(pal.accent, pal.accent_hover, s.hover) } else { pal.ink3 };
    let base = mix(off, on_col, sw.fill);
    let pressed_track = if sw.on { mix(base, pal.bg, 0.3) } else { mix(base, pal.tint_ink, 0.12) };
    p.rect_filled(track, CornerRadius::same(r), switch_color(mix(base, pressed_track, s.press), undimmed));
    let d = track.height() - 4.0;
    let knob = pos2(track.left() + 2.0 + d / 2.0 + (track.width() - 4.0 - d) * sw.knob, track.center().y);
    // Pressed, the knob stretches toward where it will go (as the
    // system's switches do): a look of its own, apart from hover.
    let stretch = 4.0 * s.press;
    let toward = if sw.on { -1.0 } else { 1.0 };
    let kr = Rect::from_center_size(knob + vec2(toward * stretch / 2.0, 0.0), vec2(d + stretch, d));
    p.add(egui::epaint::Shadow { offset: [0, 1], blur: 3, spread: 0, color: pal.shadow(70) }.as_shape(kr, CornerRadius::same((d / 2.0) as u8)));
    p.rect_filled(kr, CornerRadius::same((d / 2.0) as u8), switch_color(if s.enabled { pal.switch_knob } else { mix(pal.switch_knob, pal.ink3, 0.6) }, undimmed));
    if s.focus {
        paint_focus(p, track, r);
    }
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
    /// `None`: none of them (a value set some other way).
    pub chosen: Option<usize>,
    pub enabled: Vec<bool>,
    pub role: Role,
    pub font: f32,
}

impl Pills<'_> {
    /// Equal cells across `track`, inset by `inset` and 2 points apart.
    pub fn even(track: Rect, n: usize, inset: f32) -> Vec<Rect> {
        const GAP: f32 = 2.0;
        let w = (track.width() - inset * 2.0 - GAP * (n.max(1) - 1) as f32) / n as f32;
        (0..n).map(|k| Rect::from_min_size(pos2(track.left() + inset + k as f32 * (w + GAP), track.top() + inset), vec2(w, track.height() - inset * 2.0))).collect()
    }

    /// Paints the control; returns the choice made this frame, if any.
    pub fn show(&self, ui: &mut Ui, arrowed: bool) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        // The track (`surface2`, corner 10), and the chosen segment's
        // capsule (`seg_on`, corner 7) that slides to the choice in 220 ms.
        let track_r = |r: Rect| (r.height() / 2.0).min(10.0).round() as u8;
        let round = |r: Rect| (r.height() / 2.0).min(7.0).round() as u8;
        p.rect_filled(self.track, CornerRadius::same(track_r(self.track)), pal().surface2);
        let to = self.cells[self.chosen.unwrap_or(0).min(self.cells.len() - 1)];
        let secs = motion::space(0.22);
        let x = tween(&ctx, self.id.with("x"), to.left(), secs, motion::out_quart);
        let w = tween(&ctx, self.id.with("w"), to.width(), secs, motion::out_quart);
        let shown = tween(&ctx, self.id.with("shown"), f32::from(self.chosen.is_some()), motion::fade(0.15), motion::linear);
        if shown > 0.0 {
            let pill = Rect::from_min_size(pos2(x, to.top()), vec2(w, to.height()));
            // The chosen one lifts out of the track on a 1-point shadow.
            p.add(egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: pal().shadow((60.0 * shown) as u8) }.as_shape(pill, CornerRadius::same(round(pill))));
            p.rect_filled(pill, CornerRadius::same(round(pill)), pal().seg_on.gamma_multiply(shown));
        }
        let mut picked = None;
        // egui moves focus with the arrows but doesn't report it as gained,
        // so remember which choice had it last frame.
        let was = ctx.data(|d| d.get_temp::<Option<usize>>(self.id.with("focus")).flatten());
        let mut now_focused = None;
        for (k, cell) in self.cells.iter().enumerate() {
            let enabled = self.enabled[k];
            let r = ui.interact(hit_rect(*cell), self.id.with(k), if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
            let on = Some(k) == self.chosen;
            // Under the pointer, a faint lift; pressed, a deeper well. The
            // chosen one's capsule (above) is the selection, apart from both.
            if enabled && !on && (r.hovered() || r.is_pointer_button_down_on()) {
                // Pressed is a stronger lift than hover (a dark well can't
                // show on the dark track).
                let fill = if r.is_pointer_button_down_on() { pressed(pal().surface2_hover, 0.5) } else { pal().surface2_hover };
                p.rect_filled(*cell, CornerRadius::same(round(*cell)), fill);
            }
            // The chosen one pressed: its capsule sinks a step (darker),
            // apart from its chosen look and from the focus ring.
            if enabled && on && r.is_pointer_button_down_on() {
                p.rect_filled(*cell, CornerRadius::same(round(*cell)), pressed(pal().seg_on, 1.0));
            }
            if r.gained_focus() && keyboard_focus(&ctx) {
                r.scroll_to_me(None);
            }
            // Arrowing onto a choice chooses it, as radio groups do.
            if r.has_focus() {
                now_focused = Some(k);
            }
            if enabled && !on && (r.clicked() || (r.has_focus() && arrowed && was != Some(k))) {
                picked = Some(k);
            }
            let lit = tween(&ctx, self.id.with(("on", k)), f32::from(on), motion::fade(0.15), motion::linear);
            // Disabled choices stay readable (4:1), just clearly quieter than INK2.
            let base = if !enabled { pal().ink3 } else if r.hovered() { pal().ink_hover } else { pal().ink2 };
            let font = if on { semibold(self.font) } else { regular(self.font) };
            let text = fit(&p, self.labels[k], &font, cell.width() - 8.0);
            p.text(cell.center(), Align2::CENTER_CENTER, text, font, mix(base, pal().ink, lit));
            focus_ring(ui, &r, *cell, round(*cell));
            if r.hovered() && enabled && !on {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            targets::note(&self.names[k], r.rect);
            #[cfg(any(debug_assertions, feature = "design-evidence"))]
            targets::note_id(r.id, &self.names[k]);
            let (name, enabled) = (self.names[k].clone(), r.enabled());
            let role = self.role;
            r.widget_info(move || WidgetInfo::selected(role, enabled, on, &name));
        }
        ctx.data_mut(|d| d.insert_temp(self.id.with("focus"), now_focused));
        picked
    }
}

/// A field's frame: `bg` in a 1-point `line2` edge (lit under the
/// pointer), 1.5 points of the accent while it has the keyboard; corner 10.
pub fn paint_field(p: &Painter, rect: Rect, hover: bool, focused: bool, enabled: bool) -> CornerRadius {
    let fr = CornerRadius::same(((rect.height() / 2.0).min(10.0)) as u8);
    p.rect_filled(rect, fr, if enabled { pal().bg } else { pal().well });
    let edge = if focused { pal().accent } else if hover && enabled { pal().field_edge_lit } else { pal().line2 };
    p.rect_stroke(rect, fr, Stroke::new(if focused { 1.5 } else { 1.0 }, edge), StrokeKind::Inside);
    fr
}

/// A one-line text field. `text` is edited in place; the response says
/// when it lost focus or Enter was pressed (to apply what was typed).
#[allow(clippy::too_many_arguments)]
pub fn field(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, hint: &str, label: &str, enabled: bool, mono_text: bool) -> Response {
    let enabled = enabled && !locked(ui);
    let font = if mono_text { mono(T_META) } else { regular(T_BODY) };
    let focused = ui.memory(|m| m.has_focus(id));
    let hover = ui.rect_contains_pointer(rect) && enabled;
    let fr = paint_field(ui.painter(), rect, hover, focused, enabled);
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .font(font)
        .text_color(if enabled { pal().ink } else { pal().ink2 })
        .hint_text(egui::RichText::new(hint).color(pal().ink3))
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .align(Align2::LEFT_CENTER)
        .desired_width(rect.width() - 18.0);
    let r = ui.add_enabled_ui(enabled, |ui| ui.put(rect.shrink2(vec2(9.0, 0.0)), edit)).inner;
    if r.has_focus() && keyboard_focus(ui.ctx()) {
        paint_focus(ui.painter(), rect, fr.nw);
    }
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note(label, r.rect);
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    targets::note_id(r.id, label);
    if r.gained_focus() && keyboard_focus(ui.ctx()) {
        r.scroll_to_me(None);
    }
    let label = label.to_owned();
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_label(label);
        n.set_value(text.clone());
    });
    r
}

/// A thin bar `frac` full (the accent: something crossing); `None` while
/// the size isn't known: a short segment instead, still.
pub fn progress_bar(ui: &Ui, rect: Rect, id: Id, frac: Option<f32>) {
    let shown = tween(ui.ctx(), id, frac.unwrap_or(0.0), motion::space(0.3), motion::out_cubic);
    paint_progress(ui.painter(), rect, frac.map(|_| shown), false);
}

/// The bar itself: `surface2` track, corner 3 (the handoff's is 5 tall),
/// the accent or, `stalled`, the warning colour.
pub fn paint_progress(p: &Painter, rect: Rect, frac: Option<f32>, stalled: bool) {
    let r = CornerRadius::same((rect.height() / 2.0).min(3.0) as u8);
    p.rect_filled(rect, r, pal().surface2);
    let w = match frac {
        Some(f) => rect.width() * f.clamp(0.0, 1.0),
        None => rect.width() * 0.18,
    };
    if w > 0.5 {
        p.rect_filled(Rect::from_min_size(rect.min, vec2(w.max(rect.height()), rect.height())), r, if stalled { pal().warn } else { pal().accent });
    }
}

/// A status mark: always a shape and a colour. ● ok, ◆ needs you (a turned
/// square), ○ off (a ring of `ink3`), ■ error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Warn,
    Off,
    Error,
}

impl Mark {
    /// The mark a status colour stands for (anything else: off).
    pub fn of(tone: Color32) -> Mark {
        let p = pal();
        if tone == p.ok { Mark::Ok } else if tone == p.warn { Mark::Warn } else if tone == p.danger { Mark::Error } else { Mark::Off }
    }
}

/// A status mark `d` points across (7–8) centred on `c`.
pub fn status_mark(p: &Painter, c: Pos2, d: f32, mark: Mark) {
    let pal = pal();
    let r = d / 2.0;
    match mark {
        Mark::Ok => {
            p.circle_filled(c, r, pal.ok);
        }
        Mark::Warn => {
            let k = r * 1.2;
            p.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -k), c + vec2(k, 0.0), c + vec2(0.0, k), c + vec2(-k, 0.0)], pal.warn, Stroke::NONE));
        }
        Mark::Off => {
            p.circle_stroke(c, r - 0.75, Stroke::new(1.5, pal.ink3));
        }
        Mark::Error => {
            p.rect_filled(Rect::from_center_size(c, vec2(d - 0.5, d - 0.5)), CornerRadius::same(1), pal.danger);
        }
    }
}

/// A chip or pill: 30 tall, a capsule edged in `line2`; on, the accent's
/// edge on `accent_soft` with `accent_text`.
#[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))] // The widget gallery (debug) and the new layout use it.
pub fn paint_chip(p: &Painter, rect: Rect, label: &str, on: bool, s: State) {
    let pal = pal();
    let r = CornerRadius::same((rect.height() / 2.0).round() as u8);
    let fill = if on { pal.accent_soft } else { Color32::TRANSPARENT };
    p.rect_filled(rect, r, mix(fill, if on { pal.pressed(pal.accent_soft, 0.4) } else { pal.hover }, s.hover.max(s.press)));
    p.rect_stroke(rect, r, Stroke::new(1.0, if on { pal.accent } else { pal.line2 }), StrokeKind::Inside);
    let fg = if !s.enabled { pal.ink3 } else if on { pal.accent_text } else { pal.ink };
    p.text(rect.center(), Align2::CENTER_CENTER, label, if on { medium(T_BODY) } else { regular(T_BODY) }, fg);
    if s.focus {
        paint_focus(p, rect, r.nw);
    }
}

/// A key or shortcut in mono, on a `surface2` chip (padding 4 × 8, corner
/// 6), with `anchor` placing it like `Painter::text`; returns its rectangle.
pub fn kbd(p: &Painter, at: Pos2, anchor: Align2, text: &str) -> Rect {
    let g = p.layout_no_wrap(text.to_owned(), mono(T_META), pal().ink2);
    let rect = anchor.anchor_size(at, g.size() + vec2(16.0, 8.0));
    p.rect_filled(rect, CornerRadius::same(6), pal().surface2);
    p.galley(rect.center() - g.size() / 2.0, g, pal().ink2);
    rect
}

/// A row's hairline: across `x` at `y`, in `line` (none after the last row).
#[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))] // The widget gallery (debug) and the new layout use it.
pub fn hairline(p: &Painter, x: egui::Rangef, y: f32) {
    p.hline(x, y, Stroke::new(1.0, pal().line));
}

/// Row heights: on a page (52; in a panel, 46).
#[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))] // The widget gallery (debug) and the new layout use it.
pub const ROW_PAGE: f32 = 52.0;

/// A row: its label (13) with a caption (12, `ink2`) under it when there is
/// one, and a value (13, `ink2`) at the right.
#[cfg_attr(not(any(debug_assertions, feature = "design-evidence")), allow(dead_code))] // The widget gallery (debug) and the new layout use it.
pub fn paint_row(p: &Painter, row: Rect, label: &str, caption: Option<&str>, value: Option<&str>) {
    let pal = pal();
    let value_w = value.map_or(0.0, |v| p.layout_no_wrap(v.to_owned(), regular(T_BODY), pal.ink2).size().x + 12.0);
    let room = row.width() - value_w;
    match caption {
        Some(c) => {
            p.text(row.left_center() - vec2(0.0, 8.0), Align2::LEFT_CENTER, fit(p, label, &regular(T_BODY), room), regular(T_BODY), pal.ink);
            p.text(row.left_center() + vec2(0.0, 9.0), Align2::LEFT_CENTER, fit(p, c, &regular(T_META), room), regular(T_META), pal.ink2);
        }
        None => {
            p.text(row.left_center(), Align2::LEFT_CENTER, fit(p, label, &regular(T_BODY), room), regular(T_BODY), pal.ink);
        }
    }
    if let Some(v) = value {
        p.text(row.right_center(), Align2::RIGHT_CENTER, v, regular(T_BODY), pal.ink2);
    }
}

/// Three dots in a row: more choices.
pub fn draw_dots(p: &Painter, rect: Rect, col: Color32) {
    for k in [-1.0, 0.0, 1.0] {
        p.circle_filled(rect.center() + vec2(k * 5.0, 0.0), 1.6, col);
    }
}

/// A mono font may kern `--` so that egui draws the second dash on top of
/// the next letter (`run --port` reads `run--port`). Kerning stops at a
/// section's end, so mono text is cut between neighbouring dashes.
fn mono_pieces(text: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    let b = text.as_bytes();
    for i in 0..b.len().saturating_sub(1) {
        if b[i] == b'-' && b[i + 1] == b'-' {
            out.push(&text[start..=i]);
            start = i + 1;
        }
    }
    out.push(&text[start..]);
    out
}

/// Adds a section of its own (`LayoutJob::append` would merge it into
/// the one before, and shape them together again).
fn push_piece(job: &mut egui::text::LayoutJob, text: &str, format: egui::TextFormat) {
    let start = job.text.len();
    job.text += text;
    let byte_range = egui::epaint::text::ByteIndex(start)..egui::epaint::text::ByteIndex(job.text.len());
    job.sections.push(egui::text::LayoutSection { leading_space: 0.0, byte_range, format });
}

/// One line of mono text, shortened to `max_w`, placed like `Painter::text`.
pub fn mono_text(p: &Painter, pos: Pos2, align: Align2, text: &str, size: f32, col: Color32, max_w: f32) -> Rect {
    let font = mono(size);
    let text = fit(p, text, &font, max_w);
    let mut job = egui::text::LayoutJob::default();
    for piece in mono_pieces(&text) {
        push_piece(&mut job, piece, egui::TextFormat::simple(font.clone(), col));
    }
    let g = p.layout_job(job);
    let rect = align.anchor_size(pos, g.size());
    p.galley(rect.min, g, col);
    rect
}

/// How tall `para_code` sets `text` `width` wide (nothing is drawn).
pub fn para_code_h(ui: &Ui, width: f32, text: &str, font: FontId) -> f32 {
    code_galley(ui, width, text, font, pal().ink).size().y
}

/// Wrapped text where `like this` is a command, set in mono; returns its height.
pub fn para_code(ui: &Ui, pos: Pos2, width: f32, text: &str, font: FontId, col: Color32) -> f32 {
    let g = code_galley(ui, width, text, font, col);
    let h = g.size().y;
    ui.painter().galley(pos, g, col);
    h
}

fn code_galley(ui: &Ui, width: f32, text: &str, font: FontId, col: Color32) -> std::sync::Arc<egui::Galley> {
    let size = font.size;
    let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping { max_width: width, ..Default::default() }, ..Default::default() };
    for (k, part) in text.split('`').enumerate() {
        let code = k % 2 == 1;
        let f = if code { mono(size - 1.0) } else { font.clone() };
        let mut fmt = egui::TextFormat::simple(f, col);
        fmt.line_height = Some((size * LEADING).round());
        for piece in if code { mono_pieces(part) } else { vec![part] } {
            push_piece(&mut job, piece, fmt.clone());
        }
    }
    ui.painter().layout_job(job)
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

/// The window's glass, kept in the context so any panel can use it.
pub fn set_glass(ctx: &egui::Context, glass: crate::glass::Glass) {
    ctx.data_mut(|d| d.insert_temp(Id::unique("glass"), glass));
}

pub fn glass(ctx: &egui::Context) -> crate::glass::Glass {
    ctx.data(|d| d.get_temp::<crate::glass::Glass>(Id::unique("glass"))).unwrap_or_default()
}

/// A glass panel: a soft shadow, the blurred backdrop, its frost (lit
/// from above: lighter at the top), a bright top edge and a faint rim.
/// `key` names the panel so its blur is kept.
pub fn glass_panel(ui: &Ui, key: impl std::hash::Hash + std::fmt::Debug, rect: Rect, radius: u8) {
    for s in glass_shapes(ui.ctx(), Id::unique(("glass", key)).value(), rect, radius) {
        ui.painter().add(s);
    }
}

/// A solid surface lifted off the window: what floats over the desk.
pub fn raised_shapes(rect: Rect, r: f32, shadow: bool) -> Vec<egui::Shape> {
    let radius = CornerRadius::same(r.round().clamp(0.0, 255.0) as u8);
    let mut out = vec![];
    if shadow {
        out.extend(card_shadow(rect, radius));
    }
    out.push(egui::Shape::Rect(egui::epaint::RectShape::new(rect, radius, pal().surface, Stroke::new(1.0, pal().line), StrokeKind::Inside)));
    out
}

/// The theme's shadow under panels and cards: in light, 0 1 2 at 4 % and
/// 0 8 24 at 5 % of ink; in dark, none (the surfaces' contrast is enough).
pub fn card_shadow(rect: Rect, radius: CornerRadius) -> Vec<egui::Shape> {
    let p = pal();
    if p.card_shadow <= 0.0 {
        return vec![];
    }
    let ink = |a: f32| p.shadow_ink.gamma_multiply(a * p.card_shadow);
    vec![
        egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: ink(0.04) }.as_shape(rect, radius).into(),
        egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: ink(0.05) }.as_shape(rect, radius).into(),
    ]
}

/// The content layer: a group of rows on a quiet fill and a hairline. Not
/// glass: glass is for what floats above the content (the top bar, the
/// panels over the desk, the toolbar), as the system uses it.
pub fn group(ui: &Ui, rect: Rect, radius: u8) {
    for s in group_shapes(rect, radius) {
        ui.painter().add(s);
    }
}

/// A card: `surface`, a 1-point `line`, the theme's shadow.
pub fn group_shapes(rect: Rect, radius: u8) -> Vec<egui::Shape> {
    let mut out = card_shadow(rect, CornerRadius::same(radius));
    out.push(egui::Shape::Rect(egui::epaint::RectShape::new(rect, CornerRadius::same(radius), pal().surface, Stroke::new(1.0, pal().line), StrokeKind::Inside)));
    out
}

/// The right-hand panel's corner (handoff 4.2).
const PANEL_R: u8 = 16;

/// The right-hand panel (handoff 4.2), the one ground for the Desk's and
/// Keys & mouse's: `surface`, a 1-point `line`, corner 16, the theme's shadow.
pub fn panel(ui: &Ui, rect: Rect) {
    for s in group_shapes(rect, PANEL_R) {
        ui.painter().add(s);
    }
}

/// The same as shapes, to place behind contents drawn first.
pub fn glass_shapes(ctx: &egui::Context, key: u64, rect: Rect, radius: u8) -> Vec<egui::Shape> {
    glass_shapes_as(ctx, key, rect, f32::from(radius), PANEL_GLASS)
}

/// Which glass: a panel holds text, so it is tinted for it; a control is
/// clearer and bends more, as it is small and sits on a panel. Colours come
/// from the active palette when it is drawn.
#[derive(Clone, Copy)]
pub struct GlassKind {
    /// Real glass (only the command palette); else a solid raised surface:
    /// a lifted fill, a hairline, a soft shadow.
    glass: bool,
    bend: f32,
    shadow: bool,
}

pub const PANEL_GLASS: GlassKind = GlassKind { glass: false, bend: 1.0, shadow: true };

/// The command palette: the one piece of real glass, over a dimmed window.
pub const PALETTE_GLASS: GlassKind = GlassKind { glass: true, bend: 0.8, shadow: true };

pub fn glass_shapes_as(ctx: &egui::Context, key: u64, rect: Rect, r: f32, kind: GlassKind) -> Vec<egui::Shape> {
    if !kind.glass {
        return raised_shapes(rect, r, kind.shadow);
    }
    let g = glass(ctx);
    let radius = CornerRadius::same(r.round().clamp(0.0, 255.0) as u8);
    let mut out = vec![];
    if kind.shadow {
        out.push(egui::epaint::Shadow { offset: [0, 12], blur: 32, spread: 0, color: pal().shadow(80) }.as_shape(rect, radius).into());
    }
    if g.works() {
        let [top, bottom] = [pal().glass_top, pal().glass];
        g.backdrop_shapes(&mut out, key, rect, r, crate::glass::Look { top, bottom, bend: kind.bend });
        // A hairline for the shape's edge where the light doesn't reach.
        out.push(egui::Shape::Rect(egui::epaint::RectShape::stroke(rect, radius, Stroke::new(1.0, pal().line), StrokeKind::Inside)));
    } else {
        out.push(rounded_gradient(rect, r, pal().surface, pal().surface));
        out.push(egui::Shape::Rect(egui::epaint::RectShape::stroke(rect, radius, Stroke::new(1.0, pal().line), StrokeKind::Inside)));
        out.push(lit_edge(rect.shrink(0.5), r - 0.5));
    }
    out
}

/// Light catching a panel's top edge: along the top and round both top
/// corners, fading out as the corners turn down.
fn lit_edge(rect: Rect, r: f32) -> egui::Shape {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    const STEPS: usize = 10;
    let mut pts: Vec<(Pos2, f32)> = vec![];
    // Left corner from its side (dark) to the top (lit), then the right.
    for i in 0..=STEPS {
        let a = std::f32::consts::PI + i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
        pts.push((pos2(rect.left() + r + r * a.cos(), rect.top() + r + r * a.sin()), i as f32 / STEPS as f32));
    }
    for i in 0..=STEPS {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
        pts.push((pos2(rect.right() - r + r * a.cos(), rect.top() + r + r * a.sin()), 1.0 - i as f32 / STEPS as f32));
    }
    // A 1-point band, inward from the edge, its alpha eased along it.
    let mut m = Mesh::default();
    for (k, &(p, t)) in pts.iter().enumerate() {
        let next = pts[(k + 1).min(pts.len() - 1)].0;
        let prev = pts[k.saturating_sub(1)].0;
        let d = (next - prev).normalized();
        let inward = vec2(-d.y, d.x);
        let c = pal().glass_edge.gamma_multiply(t * t * (3.0 - 2.0 * t));
        m.vertices.push(Vertex { pos: p, uv: WHITE_UV, color: c });
        m.vertices.push(Vertex { pos: p + inward, uv: WHITE_UV, color: c });
    }
    for k in 0..pts.len() as u32 - 1 {
        let (a, b) = (2 * k, 2 * k + 2);
        m.add_triangle(a, a + 1, b + 1);
        m.add_triangle(a, b + 1, b);
    }
    egui::Shape::mesh(m)
}

/// A rounded rectangle filled from `top` at its top to `bottom` at its
/// bottom (premultiplied colours), as rows of quads: corners follow the
/// arc; the rim drawn over it smooths their edge.
pub fn rounded_gradient(rect: Rect, radius: f32, top: Color32, bottom: Color32) -> egui::Shape {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    // Rows: down the top arc, the straight middle, down the bottom arc.
    const STEPS: usize = 8;
    let mut ys: Vec<f32> = (0..=STEPS).map(|i| rect.top() + r * (1.0 - (i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2).cos())).collect();
    ys.extend((0..=STEPS).map(|i| rect.bottom() - r + r * (i as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2).sin()));
    let inset = |y: f32| {
        let d = if y < rect.top() + r { rect.top() + r - y } else if y > rect.bottom() - r { y - (rect.bottom() - r) } else { 0.0 };
        r - (r * r - d * d).max(0.0).sqrt()
    };
    let mut m = Mesh::default();
    for &y in &ys {
        let t = ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let c = mix(top, bottom, t);
        let dx = inset(y);
        m.vertices.push(Vertex { pos: pos2(rect.left() + dx, y), uv: WHITE_UV, color: c });
        m.vertices.push(Vertex { pos: pos2(rect.right() - dx, y), uv: WHITE_UV, color: c });
    }
    for k in 0..ys.len() as u32 - 1 {
        let (a, b) = (2 * k, 2 * k + 2);
        m.add_triangle(a, a + 1, b + 1);
        m.add_triangle(a, b + 1, b);
    }
    egui::Shape::mesh(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_ends_at_each_colour() {
        assert_eq!(mix(pal().ink, pal().accent, 0.0), pal().ink);
        assert_eq!(mix(pal().ink, pal().accent, 1.0), pal().accent);
        assert_eq!(mix(pal().ink, pal().accent, 7.0), pal().accent);
    }

    #[test]
    fn fixed_buttons_paint_complete_labels_at_every_review_scale() {
        let labels = ["Clear", "Sure?", "Refresh", "Choose from a list", "Key detail", "Whole board", "Compact", "Pair a computer", "Lock every screen", "Download", "Try again"];
        for (width, scale) in [(1200.0, 1.0), (1200.0, 2.0), (760.0, 1.0)] {
            let ctx = egui::Context::default();
            crate::style::install(&ctx);
            ctx.set_pixels_per_point(scale);
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 780.0))), ..Default::default() }, |ui| {
                for (index, label) in labels.iter().enumerate() {
                    let rect = Rect::from_min_size(pos2(150.0, 20.0 + index as f32 * 40.0), vec2(12.0, 32.0));
                    let response = button(ui, rect, Id::unique(index), label, Look::Quiet, true, None);
                    assert!(response.rect.width() >= button_width(ui, label));
                }
            });
            output.textures_delta.clear();
            for label in labels {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label)), "Missing complete fixed label {label} at {width}/{scale}");
            }
        }
    }

    /// Runs `check` once with each theme's palette active.
    fn each_theme(check: impl Fn(&crate::style::Palette)) {
        for p in [crate::style::Palette::light(), crate::style::Palette::dark()] {
            crate::style::set_palette(p, None);
            check(&p);
        }
        crate::style::set_palette(crate::style::Palette::dark(), None);
    }

    #[test]
    fn icon_info_and_chip_press_is_measurably_distinct_from_hover() {
        each_theme(|p| {
            for hover in [p.surface2, p.surface2_hover, p.seg_on] {
                let down = pressed_lift(hover, 1.0);
                let delta: u32 = [(down.r(), hover.r()), (down.g(), hover.g()), (down.b(), hover.b())].iter().map(|(a, b)| u32::from(a.abs_diff(*b))).sum();
                assert!(delta >= 30, "{:?}: pressed {down:?} vs {hover:?}", p.theme);
            }
        });
    }

    #[test]
    fn pressed_control_pairs_meet_text_and_icon_contrast() {
        each_theme(|p| {
            for progress in 1..=100 {
                let amount = progress as f32 / 100.0;
                for fill in [p.surface2, p.surface2_hover, p.seg_on] {
                    let surface = pressed_lift(fill, amount);
                    // Secondary ink is never on a hovered fill: hover lights it to ink.
                    let inks: &[Color32] = if fill == p.surface2_hover { &[p.ink] } else { &[p.ink, p.ink2] };
                    for &normal in inks {
                        let ink = pressed_ink(surface, normal);
                        assert_eq!(ink, normal);
                        assert!(contrast(ink, surface) >= if normal == p.ink2 { 3.0 } else { 4.5 }, "{:?} chip/info/icon: {ink:?} on {surface:?}", p.theme);
                    }
                }
            }
            for (family, ink, surface) in [
                ("segment", p.ink, pressed(p.seg_on, 1.0)),
                ("capsule", p.ink, pressed(p.surface2_hover, 1.0)),
                ("primary capsule", p.on_accent, p.accent_press),
                ("sidebar row", p.ink, pressed(p.surface2_hover, 0.3)),
            ] {
                assert!(contrast(ink, surface) >= 4.5, "{:?} {family}", p.theme);
            }
        });
    }

    #[test]
    fn held_states_keep_their_ink_and_clear_hover_delta() {
        each_theme(|p| {
            for (hover, held) in [(p.surface2_hover, pressed(p.surface2_hover, 1.0)), (p.seg_on, pressed(p.seg_on, 1.0))] {
                assert_ne!(hover, held);
                assert!(contrast(p.ink, held) >= 4.5, "{:?}", p.theme);
            }
            assert!(pressed(p.surface2_hover, 1.0).r() < p.surface2_hover.r());
        });
    }

    #[test]
    fn disabled_switch_dims_track_edge_and_knob() {
        for color in [pal().switch_off, pal().ink2, pal().switch_knob] {
            assert_eq!(switch_color(color, true), color);
            assert!(contrast(switch_color(color, false), pal().surface) < contrast(color, pal().surface));
        }
        let context = egui::Context::default();
        let mut enabled = true;
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.disable();
            enabled = switch_help_dimmed(ui, Rect::from_min_size(Pos2::ZERO, vec2(300.0, 32.0)), Id::unique("disabled-switch"), "Also on this computer", "also-on-this-computer", false, true).enabled();
        });
        assert!(!enabled);
        output.textures_delta.clear();
        for dimmed in [false, true] {
            let context = egui::Context::default();
            let mut output = context.run_ui(Default::default(), |ui| {
                switch_help_painted(ui, Rect::from_min_size(Pos2::ZERO, vec2(300.0, 32.0)), Id::unique("switch-appearance"), "Also on this computer", "also-on-this-computer", false, (false, dimmed));
            });
            for expected in [switch_color(pal().switch_off, !dimmed), switch_color(mix(pal().switch_knob, pal().ink3, 0.6), !dimmed)] {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == expected)), "dimmed={dimmed}, missing {expected:?}");
            }
            output.textures_delta.clear();
        }
    }

    #[test]
    fn keyboard_focus_paints_a_two_point_ring() {
        let ctx = egui::Context::default();
        let id = Id::unique("focus-test");
        ctx.data_mut(|data| data.insert_temp(Id::unique(KB), true));
        let mut ring = false;
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.memory_mut(|memory| memory.request_focus(id));
                button(ui, Rect::from_min_size(pos2(20.0, 20.0), vec2(80.0, 32.0)), id, "Clear", Look::Quiet, true, None);
            });
            output.textures_delta.clear();
            ring |= output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == pal().focus && rect.stroke.width == 2.0));
        }
        assert!(ring);
    }

    #[test]
    fn navigation_legends_fit_the_working_board_without_abbreviation() {
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for key in crate::tune::keyboard(false, false) {
                let label = crate::tune::cap_label(key.code);
                if label.chars().count() < 4 { continue; }
                let available = key.w * 36.0 - 10.0;
                assert_eq!(fit(ui.painter(), &label, &regular(11.0), available), label);
            }
            assert_eq!(crate::tune::cap_label(96), "Ent");
            assert_eq!(crate::tune::key_label(96), "Keypad Enter");
        });
        output.textures_delta.clear();
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
                        chosen: Some(chosen),
                        enabled: vec![true; 3],
                        role: Role::RadioButton,
                        font: T_BODY,
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
    fn dashes_in_mono_text_keep_their_room() {
        assert_eq!(mono_pieces("one-mk run --port 1"), ["one-mk run -", "-port 1"]);
        assert_eq!(mono_pieces("---x"), ["-", "-", "-x"]);
        assert_eq!(mono_pieces("plain"), ["plain"]);
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let r = mono_text(&p, Pos2::ZERO, Align2::LEFT_TOP, "run --port", 12.0, pal().ink, 500.0);
            let plain = p.layout_no_wrap("run x-port".into(), mono(12.0), pal().ink).size().x;
            assert!((r.width() - plain).abs() < 0.5, "every character a full cell: {} vs {plain}", r.width());
        });
        out.textures_delta.clear();
    }

    #[test]
    fn every_hit_area_is_at_least_24_points_and_keeps_its_centre() {
        for size in [vec2(16.0, 16.0), vec2(22.0, 22.0), vec2(60.0, 22.0), vec2(38.0, 22.0), vec2(120.0, 32.0)] {
            let visual = Rect::from_min_size(pos2(20.0, 30.0), size);
            let hit = hit_rect(visual);
            assert_eq!(hit.center(), visual.center());
            assert!(hit.width() >= MIN_TARGET && hit.height() >= MIN_TARGET, "{size:?}");
            assert!(hit.contains_rect(visual));
        }
        // A segment cell of a 26-point track inset by 2: 22 tall, hit 24.
        let track = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 26.0));
        assert!(hit_rect(Pills::even(track, 4, 2.0)[0]).height() >= MIN_TARGET);
    }

    /// Pressed differs from rest and from hover, for the fills that have one.
    #[test]
    fn pressed_is_its_own_look() {
        for fill in [pal().surface2, pal().surface2_hover, pal().bg] {
            let down = pressed(fill, 1.0);
            assert_ne!(down, fill);
            assert!(down.r() <= fill.r() && down.g() <= fill.g(), "pressed goes deeper, not lighter");
        }
        assert_eq!(pressed(pal().surface2, 0.0), pal().surface2);
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
