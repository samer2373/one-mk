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

/// The smallest a pointer target may be, in points (WCAG 2.5.8 and the
/// council's floor): a control may look smaller, its hit area doesn't.
pub const MIN_TARGET: f32 = 24.0;

/// Where a control takes the pointer: its own rectangle, grown about its
/// centre to at least `MIN_TARGET` square.
pub fn hit_rect(rect: Rect) -> Rect {
    Rect::from_center_size(rect.center(), rect.size().max(vec2(MIN_TARGET, MIN_TARGET)))
}

/// The pressed look shared by every control family: how far a fill moves
/// toward the ground while the pointer is down on it.
pub const PRESS_DARKEN: f32 = 0.28;

/// `fill` as it looks while pressed (`t` 0..1).
pub fn pressed(fill: Color32, t: f32) -> Color32 {
    mix(fill, GROUND, PRESS_DARKEN * t)
}

pub fn pressed_lift(fill: Color32, t: f32) -> Color32 {
    mix(fill, INK, 0.35 * t)
}

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

pub fn pressed_ink(surface: Color32, normal: Color32) -> Color32 {
    if contrast(normal, surface) >= 4.5 { normal } else if contrast(Color32::BLACK, surface) >= 4.5 { Color32::BLACK } else { Color32::WHITE }
}

fn switch_color(color: Color32, enabled: bool) -> Color32 {
    if enabled { color } else { mix(color, PANEL, 0.6) }
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

/// The signal-coloured ring around whatever the keyboard is on.
pub fn focus_ring(ui: &Ui, r: &Response, rect: Rect, radius: u8) {
    if r.has_focus() && keyboard_focus(ui.ctx()) {
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(radius + 2), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The one action that matters most here: the accent.
    Primary,
    /// Everything else: a quiet solid capsule.
    Quiet,
    /// A control floating over the desk (the toolbar): glass, like the
    /// panels it sits among.
    Glass,
}

/// A button that presses in a little. `a11y` names it for screen readers
/// when the visible label alone would be vague.
pub fn button(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let rect = Rect::from_center_size(rect.center(), vec2(rect.width().max(button_width(ui, label)), rect.height()));
    button_painted(ui, rect, id, label, look, enabled, a11y_label)
}

pub fn content_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let shown = fit(ui.painter(), label, &semibold(T_BODY), (rect.width() - 18.0).max(0.0));
    let response = button_painted(ui, rect, id, &shown, look, enabled, Some(a11y_label.unwrap_or(label)));
    if enabled && response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), pressed_lift(BUTTON_LIT, 1.0));
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, shown, semibold(T_BODY), pressed_ink(pressed_lift(BUTTON_LIT, 1.0), INK));
        focus_ring(ui, &response, rect, (rect.height() / 2.0) as u8);
    }
    response
}

fn button_painted(ui: &mut Ui, rect: Rect, id: Id, label: &str, look: Look, enabled: bool, a11y_label: Option<&str>) -> Response {
    let r = ui.interact(hit_rect(rect), id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let down = enabled && r.is_pointer_button_down_on();
    let press = tween(&ctx, id.with("press"), f32::from(down), motion::fade(if down { 0.06 } else { 0.14 }), motion::out_cubic);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.1, motion::out_cubic);
    // Only the shape scales: glyphs re-rasterised at a new size shimmer.
    let rr = Rect::from_center_size(rect.center(), rect.size() * (1.0 - motion::space(0.02) * press));
    // Capsules, as the system's own glass buttons are.
    let radius = (rr.height() / 2.0).round().min(255.0) as u8;
    let fg = match (look, enabled) {
        (_, false) => INK3,
        (Look::Primary, true) => SIGNAL_INK,
        (Look::Quiet | Look::Glass, true) => INK,
    };
    let p = ui.painter().clone();
    match (look, enabled) {
        (Look::Primary, true) => {
            // Flat, solid Volt: no gradient, no lit rim, no glow. Lighter
            // under the pointer, deeper while pressed.
            let fill = mix(mix(SIGNAL, SIGNAL_LIT, hover), SIGNAL_INK, 0.16 * press);
            p.rect_filled(rr, CornerRadius::same(radius), fill);
        }
        // One emphasis system: the toolbar's buttons are quiet fills too
        // (no raised rim); only Primary is strong.
        (Look::Glass, true) => {
            p.rect_filled(rr, CornerRadius::same(radius), pressed(mix(BUTTON, BUTTON_LIT, hover), press));
        }
        (Look::Quiet, true) => {
            p.rect(rr, CornerRadius::same(radius), pressed(mix(BUTTON, BUTTON_LIT, hover), press), Stroke::new(1.0, CONTROL_EDGE), StrokeKind::Inside);
        }
        (_, false) => {
            p.rect(rr, CornerRadius::same(radius), RAISED, Stroke::new(1.0, LINE), StrokeKind::Inside);
        }
    }
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
    let r = ui.interact(hit_rect(rect), id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.1, motion::out_cubic);
    let down = enabled && r.is_pointer_button_down_on();
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(7), if down { pressed_lift(BUTTON_LIT, 1.0) } else { mix(Color32::TRANSPARENT, BUTTON_LIT, hover) });
    focus_ring(ui, &r, rect, 7);
    let ink = if enabled { mix(INK2, INK, hover) } else { mix(INK2, PANEL, 0.4) };
    icon(p, rect, if down { pressed_ink(pressed_lift(BUTTON_LIT, 1.0), ink) } else { ink });
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
    let col = if enabled { INK } else { INK2 };
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
        p.text(row.left_center() + vec2(0.0, 8.0), Align2::LEFT_CENTER, fit(p, h.caption, &regular(T_META), room), regular(T_META), INK2);
    } else {
        p.text(row.left_center(), Align2::LEFT_CENTER, &shown, font, col);
    }
    let ic = pos2(row.right() - right - 11.0, row.center().y);
    let ir = Rect::from_center_size(ic, vec2(MIN_TARGET, MIN_TARGET));
    let r = ui.interact(ir, id.with("info"), sense(ui, Sense::click()));
    let hover = tween(ui.ctx(), id.with("info-hover"), f32::from(r.hovered()), 0.1, motion::out_cubic);
    let p = ui.painter();
    let ink = mix(INK2, INK, hover);
    let well = if r.is_pointer_button_down_on() { pressed_lift(BUTTON_LIT, 1.0) } else { mix(Color32::TRANSPARENT, BUTTON, hover) };
    let ink = if r.is_pointer_button_down_on() { pressed_ink(well, ink) } else { ink };
    p.circle_filled(ic, 11.0, well);
    p.circle_stroke(ic, 8.0, Stroke::new(1.0, ink));
    p.text(ic, Align2::CENTER_CENTER, "i", semibold(T_META), ink);
    focus_ring(ui, &r, ir, 11);
    a11y(&r, Role::Button, &format!("About {label}"), None);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let pop = egui::Popup::menu(&r).width(300.0).align(egui::RectAlign::BOTTOM_END);
    // Development aid: ONE_MK_APP_HELP=<key> keeps that explanation open.
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    let pop = if std::env::var("ONE_MK_APP_HELP").is_ok_and(|v| v == key) { pop.open(true) } else { pop };
    pop.show(|ui| {
        // A picture on its own well, then the sentence, inset with the menu's margin.
        const W: f32 = 300.0;
        const IN: f32 = 8.0;
        ui.set_max_width(W);
        let mut job = egui::text::LayoutJob::simple(h.detail.to_owned(), regular(T_BODY), INK_HOVER, W - 2.0 * IN);
        job.sections[0].format.line_height = Some((T_BODY * LEADING).round());
        let text = ui.painter().layout_job(job);
        let pic = if h.scene != crate::help::Scene::None { 112.0 } else { 0.0 };
        let (all, _) = ui.allocate_exact_size(vec2(W, IN + pic + if pic > 0.0 { IN } else { 0.0 } + text.size().y + IN - 2.0), Sense::hover());
        if pic > 0.0 {
            let scene = Rect::from_min_size(all.min + vec2(IN, IN), vec2(W - 2.0 * IN, pic));
            ui.painter().rect_filled(scene, CornerRadius::same(R_INNER), Color32::from_black_alpha(60));
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
        ui.painter().galley(pos2(all.left() + IN, y), text, INK_HOVER);
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
    let enabled = enabled && ui.is_enabled();
    let r = ui.interact(hit_rect(row), id, if enabled { sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let press = tween(&ctx, id.with("press"), f32::from(enabled && r.is_pointer_button_down_on()), motion::fade(0.06), motion::out_cubic);
    let knob_t = tween(&ctx, id.with("knob"), f32::from(on), motion::space(0.26), motion::back_out);
    let fill = tween(&ctx, id.with("track"), f32::from(on), motion::fade(0.2), motion::linear);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let shown = label_help(ui, row, id, label, key, enabled, 46.0);
    let p = ui.painter();
    let track = Rect::from_min_size(pos2(row.right() - 38.0, row.center().y - 11.0), vec2(38.0, 22.0));
    // Off: a recessed capsule. On: filled with the accent, as the system's
    // switches are; the knob is a lit white bead that slides across.
    let off = mix(SWITCH_OFF, SWITCH_OFF_LIT, hover);
    let on_col = if enabled { mix(ACCENT, ACCENT_LIT, hover) } else { INK3 };
    let base = mix(off, on_col, fill);
    let pressed_track = if on { mix(base, GROUND, 0.45) } else { mix(base, Color32::WHITE, 0.30) };
    p.rect_filled(track, CornerRadius::same(11), switch_color(mix(base, pressed_track, press), enabled));
    // Off, the track's edge is what shows the control: 3:1 on the group.
    p.rect_stroke(track, CornerRadius::same(11), Stroke::new(1.0, switch_color(mix(SWITCH_EDGE, Color32::TRANSPARENT, fill), enabled)), StrokeKind::Inside);
    let knob = pos2(track.left() + 11.0 + 16.0 * knob_t, track.center().y);
    // Pressed, the knob stretches toward where it will go (as the
    // system's switches do): a look of its own, apart from hover.
    let stretch = 5.0 * press;
    let toward = if on { -1.0 } else { 1.0 };
    let kr = Rect::from_center_size(knob + vec2(toward * stretch / 2.0, 0.0), vec2(18.0 + stretch, 18.0));
    p.add(egui::epaint::Shadow { offset: [0, 1], blur: 4, spread: 0, color: Color32::from_black_alpha(90) }.as_shape(kr, CornerRadius::same(9)));
    p.rect_filled(kr, CornerRadius::same(9), switch_color(Color32::WHITE, enabled));
    focus_ring(ui, &r, track, 11);
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
    /// Equal cells across `track`, inset by `inset`.
    pub fn even(track: Rect, n: usize, inset: f32) -> Vec<Rect> {
        let w = (track.width() - inset * 2.0) / n as f32;
        (0..n).map(|k| Rect::from_min_size(pos2(track.left() + inset + k as f32 * w, track.top() + inset), vec2(w, track.height() - inset * 2.0))).collect()
    }

    /// Paints the control; returns the choice made this frame, if any.
    pub fn show(&self, ui: &mut Ui, arrowed: bool) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        // A recessed track, and a lighter capsule that slides to the choice.
        let round = |r: Rect| (r.height() / 2.0).round().min(255.0) as u8;
        p.rect(self.track, CornerRadius::same(round(self.track)), TRACK, Stroke::new(1.0, TRACK_LINE), StrokeKind::Inside);
        let to = self.cells[self.chosen.unwrap_or(0).min(self.cells.len() - 1)];
        let secs = motion::space(0.22);
        let x = tween(&ctx, self.id.with("x"), to.left(), secs, motion::out_quart);
        let w = tween(&ctx, self.id.with("w"), to.width(), secs, motion::out_quart);
        let shown = tween(&ctx, self.id.with("shown"), f32::from(self.chosen.is_some()), motion::fade(0.15), motion::linear);
        if shown > 0.0 {
            let pill = Rect::from_min_size(pos2(x, to.top()), vec2(w, to.height()));
            // The chosen one lifts out of the track: a lighter capsule on
            // a soft shadow.
            p.add(egui::epaint::Shadow { offset: [0, 1], blur: 4, spread: 0, color: Color32::from_black_alpha((70.0 * shown) as u8) }.as_shape(pill, CornerRadius::same(round(pill))));
            p.rect(pill, CornerRadius::same(round(pill)), THUMB.gamma_multiply(shown), Stroke::new(1.0, CONTROL_EDGE.gamma_multiply(shown)), StrokeKind::Inside);
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
                let fill = if r.is_pointer_button_down_on() { Color32::from_white_alpha(30) } else { Color32::from_white_alpha(12) };
                p.rect_filled(*cell, CornerRadius::same(round(*cell)), fill);
            }
            // The chosen one pressed: its capsule sinks a step (darker),
            // apart from its chosen look and from the focus ring.
            if enabled && on && r.is_pointer_button_down_on() {
                p.rect_filled(*cell, CornerRadius::same(round(*cell)), Color32::from_black_alpha(70));
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
            let base = if !enabled { mix(INK2, RAISED, 0.3) } else if r.hovered() { INK_HOVER } else { INK2 };
            let font = if on { semibold(self.font) } else { regular(self.font) };
            let text = fit(&p, self.labels[k], &font, cell.width() - 8.0);
            p.text(cell.center(), Align2::CENTER_CENTER, text, font, mix(base, INK, lit));
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

/// A one-line text field. `text` is edited in place; the response says
/// when it lost focus or Enter was pressed (to apply what was typed).
#[allow(clippy::too_many_arguments)]
pub fn field(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, hint: &str, label: &str, enabled: bool, mono_text: bool) -> Response {
    let enabled = enabled && !locked(ui);
    let font = if mono_text { mono(T_META) } else { regular(T_BODY) };
    let focused = ui.memory(|m| m.has_focus(id));
    let hover = ui.rect_contains_pointer(rect) && enabled;
    let p = ui.painter();
    // Recessed into the glass: darker than it, with a rim that lights up
    // when the field has the keyboard.
    let fr = CornerRadius::same(((rect.height() / 2.0).min(10.0)) as u8);
    p.rect_filled(rect, fr, if enabled { Color32::from_black_alpha(70) } else { Color32::from_black_alpha(40) });
    // The field's edge shows where to type: 3:1 against the group.
    let edge = if focused { ACCENT } else if hover { FIELD_EDGE_LIT } else { FIELD_EDGE };
    p.rect_stroke(rect, fr, Stroke::new(if focused { 1.5 } else { 1.0 }, edge), StrokeKind::Inside);
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .font(font)
        .text_color(if enabled { INK } else { INK2 })
        .hint_text(egui::RichText::new(hint).color(INK3))
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .align(Align2::LEFT_CENTER)
        .desired_width(rect.width() - 18.0);
    let r = ui.add_enabled_ui(enabled, |ui| ui.put(rect.shrink2(vec2(9.0, 0.0)), edit)).inner;
    if r.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(9), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
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

/// A thin bar `frac` full (Volt: something crossing); `None` while the
/// size isn't known: a short segment instead, still.
pub fn progress_bar(ui: &Ui, rect: Rect, id: Id, frac: Option<f32>) {
    let p = ui.painter();
    let r = CornerRadius::same((rect.height() / 2.0) as u8);
    p.rect_filled(rect, r, TRACK_OFF);
    let shown = tween(ui.ctx(), id, frac.unwrap_or(0.0), motion::space(0.3), motion::out_cubic);
    let w = match frac {
        Some(_) => rect.width() * shown.clamp(0.0, 1.0),
        None => rect.width() * 0.18,
    };
    if w > 0.5 {
        p.rect_filled(Rect::from_min_size(rect.min, vec2(w.max(rect.height()), rect.height())), r, SIGNAL);
    }
}

/// A clipboard (a board, its clip, two lines of text) and an arrow
/// beside it: going out (`send`) or coming in.
pub fn draw_clip(p: &Painter, rect: Rect, col: Color32, send: bool) {
    let c = rect.center();
    let st = Stroke::new(1.4, col);
    let body = Rect::from_min_size(pos2(c.x - 8.5, c.y - 6.0), vec2(10.0, 13.0));
    p.rect_stroke(body, CornerRadius::same(2), st, StrokeKind::Middle);
    let clip = Rect::from_center_size(pos2(body.center().x, body.top()), vec2(5.0, 3.0));
    p.rect_filled(clip, CornerRadius::same(1), col);
    for k in [0.0, 3.0] {
        p.line_segment([pos2(body.left() + 2.5, c.y + k), pos2(body.right() - 2.5, c.y + k)], Stroke::new(1.0, col));
    }
    // The arrow points away from the board, or at it.
    let (a, b, y) = (body.right() + 2.0, body.right() + 8.5, c.y - 1.0);
    let (from, to) = if send { (a, b) } else { (b, a) };
    let dir = if send { 1.0 } else { -1.0 };
    p.line_segment([pos2(from, y), pos2(to, y)], st);
    p.line_segment([pos2(to, y), pos2(to - 2.8 * dir, y - 2.8)], st);
    p.line_segment([pos2(to, y), pos2(to - 2.8 * dir, y + 2.8)], st);
}

/// Three dots in a row: more choices.
pub fn draw_dots(p: &Painter, rect: Rect, col: Color32) {
    for k in [-1.0, 0.0, 1.0] {
        p.circle_filled(rect.center() + vec2(k * 5.0, 0.0), 1.6, col);
    }
}

/// Geist Mono kerns `--` so that egui draws the second dash on top of
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

/// Wrapped text where `like this` is a command, set in mono; returns its height.
pub fn para_code(ui: &Ui, pos: Pos2, width: f32, text: &str, font: FontId, col: Color32) -> f32 {
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
    let g = ui.painter().layout_job(job);
    let h = g.size().y;
    ui.painter().galley(pos, g, col);
    h
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
        out.push(egui::epaint::Shadow { offset: [0, 16], blur: 40, spread: 0, color: Color32::from_black_alpha(110) }.as_shape(rect, radius).into());
    }
    out.push(rounded_gradient(rect, r, ELEVATED_TOP, ELEVATED));
    out.push(egui::Shape::Rect(egui::epaint::RectShape::stroke(rect, radius, Stroke::new(1.0, ELEVATED_EDGE), StrokeKind::Inside)));
    out
}

/// The content layer: a group of rows on a quiet fill and a hairline. Not
/// glass: glass is for what floats above the content (the sidebar, the
/// panels over the desk, the toolbar), as the system uses it.
pub fn group(ui: &Ui, rect: Rect, radius: u8) {
    for s in group_shapes(rect, radius) {
        ui.painter().add(s);
    }
}

pub fn group_shapes(rect: Rect, radius: u8) -> Vec<egui::Shape> {
    vec![egui::Shape::Rect(egui::epaint::RectShape::new(rect, CornerRadius::same(radius), GROUP, Stroke::new(1.0, GROUP_EDGE), StrokeKind::Inside))]
}

/// The same as shapes, to place behind contents drawn first.
pub fn glass_shapes(ctx: &egui::Context, key: u64, rect: Rect, radius: u8) -> Vec<egui::Shape> {
    glass_shapes_as(ctx, key, rect, f32::from(radius), PANEL_GLASS)
}

/// Which glass: a panel holds text, so it is tinted for it; a control is
/// clearer and bends more, as it is small and sits on a panel.
#[derive(Clone, Copy)]
pub struct GlassKind {
    /// Real glass (only the command palette in the Night design); else a
    /// solid raised surface: a lifted fill, a hairline, a soft shadow.
    glass: bool,
    see: [Color32; 2],
    solid: [Color32; 2],
    clear: [Color32; 2],
    bend: f32,
    shadow: bool,
}

pub const PANEL_GLASS: GlassKind = GlassKind { glass: false, see: [GLASS_TOP, GLASS], clear: [GLASS_CLEAR_TOP, GLASS_CLEAR], solid: [GLASS_SOLID_TOP, GLASS_SOLID], bend: 1.0, shadow: true };
pub const CONTROL_GLASS: GlassKind = GlassKind { glass: false, see: [CONTROL_TOP, CONTROL], clear: [CONTROL_TOP, CONTROL], solid: [CONTROL_SOLID_TOP, CONTROL_SOLID], bend: 1.6, shadow: false };

/// The command palette: the one piece of real glass, over a dimmed window.
pub const PALETTE_GLASS: GlassKind = GlassKind { glass: true, see: [PALETTE_TOP, PALETTE], clear: [PALETTE_TOP, PALETTE], solid: [ELEVATED, ELEVATED], bend: 0.8, shadow: true };

pub fn glass_shapes_as(ctx: &egui::Context, key: u64, rect: Rect, r: f32, kind: GlassKind) -> Vec<egui::Shape> {
    if !kind.glass {
        return raised_shapes(rect, r, kind.shadow);
    }
    let g = glass(ctx);
    let radius = CornerRadius::same(r.round().clamp(0.0, 255.0) as u8);
    let mut out = vec![];
    if kind.shadow {
        out.push(egui::epaint::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(80) }.as_shape(rect, radius).into());
    }
    if g.works() {
        let [top, bottom] = if crate::chrome::see_through() { kind.clear } else { kind.see };
        g.backdrop_shapes(&mut out, key, rect, r, crate::glass::Look { top, bottom, bend: kind.bend });
        // A hairline for the shape's edge where the light doesn't reach.
        out.push(egui::Shape::Rect(egui::epaint::RectShape::stroke(rect, radius, Stroke::new(1.0, GLASS_HAIRLINE), StrokeKind::Inside)));
    } else {
        out.push(rounded_gradient(rect, r, kind.solid[0], kind.solid[1]));
        out.push(egui::Shape::Rect(egui::epaint::RectShape::stroke(rect, radius, Stroke::new(1.0, GLASS_RIM), StrokeKind::Inside)));
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
        let c = GLASS_EDGE.gamma_multiply(t * t * (3.0 - 2.0 * t));
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
        assert_eq!(mix(INK, SIGNAL, 0.0), INK);
        assert_eq!(mix(INK, SIGNAL, 1.0), SIGNAL);
        assert_eq!(mix(INK, SIGNAL, 7.0), SIGNAL);
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

    #[test]
    fn icon_info_and_chip_press_is_measurably_distinct_from_hover() {
        for hover in [BUTTON, BUTTON_LIT, THUMB] {
            let down = pressed_lift(hover, 1.0);
            assert!(down.r().abs_diff(hover.r()) >= 32);
            assert!(down.g().abs_diff(hover.g()) >= 32);
            assert!(down.b().abs_diff(hover.b()) >= 32);
        }
    }

    #[test]
    fn pressed_control_pairs_meet_text_and_icon_contrast() {
        for progress in 1..=100 {
            let amount = progress as f32 / 100.0;
            for fill in [BUTTON, BUTTON_LIT, THUMB, Color32::from_rgb(0x60, 0x6A, 0x76)] {
                let surface = pressed_lift(fill, amount);
                for normal in [INK, INK2, ACCENT] {
                    let ink = pressed_ink(surface, normal);
                    assert!(contrast(ink, surface) >= 4.5, "chip/info/icon: {ink:?} on {surface:?}");
                    assert!(contrast(ink, surface) >= 3.0);
                }
            }
        }
        for (family, ink, surface) in [
            ("segment", INK, mix(THUMB, GROUND, 0.28)),
            ("capsule", INK, pressed(BUTTON_LIT, 1.0)),
            ("primary capsule", SIGNAL_INK, mix(SIGNAL_LIT, SIGNAL_INK, 0.16)),
            ("sidebar row", INK, SIDE_CHOSEN),
        ] {
            assert!(contrast(ink, surface) >= 4.5, "{family}");
            assert!(contrast(ink, surface) >= 3.0, "{family}");
        }
    }

    #[test]
    fn disabled_switch_dims_track_edge_and_knob() {
        for color in [SWITCH_OFF, SWITCH_EDGE, Color32::WHITE] {
            assert_eq!(switch_color(color, true), color);
            assert!(contrast(switch_color(color, false), PANEL) < contrast(color, PANEL));
        }
        let context = egui::Context::default();
        let mut enabled = true;
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.disable();
            enabled = switch_help(ui, Rect::from_min_size(Pos2::ZERO, vec2(300.0, 32.0)), Id::unique("disabled-switch"), "Also on this computer", "also-on-this-computer", false, true).enabled();
        });
        assert!(!enabled);
        output.textures_delta.clear();
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
            ring |= output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == SIGNAL && rect.stroke.width == 2.0));
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
            let r = mono_text(&p, Pos2::ZERO, Align2::LEFT_TOP, "run --port", 12.0, INK, 500.0);
            let plain = p.layout_no_wrap("run x-port".into(), mono(12.0), INK).size().x;
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
        for fill in [BUTTON, BUTTON_LIT, SIDE_HOVER, SIDE_CHOSEN] {
            let down = pressed(fill, 1.0);
            assert_ne!(down, fill);
            assert!(down.r() <= fill.r() && down.g() <= fill.g(), "pressed goes deeper, not lighter");
        }
        assert_eq!(pressed(BUTTON, 0.0), BUTTON);
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
