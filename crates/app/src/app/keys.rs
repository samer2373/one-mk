//! The Keys & mouse page: a drawn keyboard where a click and a key press
//! change what a key types on the other computers, quick setups, the rules
//! in sentences, a "Test it" box that runs the real key mapper here
//! (nothing is sent), and the mouse: its buttons, pointer speed and
//! scrolling. Changes go through `one-mk keymap|pointer|scroll|buttons`,
//! one at a time, off the window's thread; what is shown is read back from
//! `settings.toml`. The pure parts are in `crate::tune`.

use super::*;
use crate::tune::{self, Capture, Device, Facts, Input, KeyRule, Scope, ScrollChange, Tester};
use egui::{Event, EventFilter};
use one_mk_proto::keynames;
use one_mk_proto::remap::What;
use std::time::Instant;

const TEST_ID: &str = "keys-test";
const CAPTURE_ID: &str = "keys-capture";
/// The largest a key is drawn (points per key).
const UNIT_MAX: f32 = 34.0;
const ROW: f32 = 36.0;
const GAP: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum View {
    #[default]
    Keyboard,
    Mouse,
}

/// What a key press or a choice from the list is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Want {
    /// What the selected key becomes (tapped, if it has a hold).
    To,
    /// What it becomes when held.
    Hold,
    /// What the selected mouse button does.
    Button,
}

/// The page's own state.
#[derive(Default)]
pub(super) struct KeysUi {
    pub view: View,
    /// "For:" which computers changes are made and shown for.
    pub scope: Scope,
    /// The keypad is drawn too ("More keys").
    pub keypad: bool,
    /// The key being changed, and the layer of the rule being changed.
    pub selected: Option<u16>,
    pub layer: Option<String>,
    /// The app the rule being changed is only for ("Only in app…").
    pub app: Option<String>,
    /// The key the keyboard's focus is on (arrows move it).
    pub focus_key: u16,
    /// Waiting for a key press.
    pub capture: Option<(Want, Capture)>,
    /// The list is open, with what was typed to search it.
    pub picker: Option<(Want, String)>,
    /// "Different when held" is open.
    pub more: bool,
    pub button: Option<u32>,
    /// An error and the area it belongs to (keyboard, list, buttons, pointer, scroll).
    pub error: Option<(&'static str, String)>,
    /// `one-mk keymap check`, and whether it should run again.
    pub problems: Option<Result<Vec<String>, String>>,
    pub check_due: bool,
    tester: Option<(Tester, Input, Facts)>,
    tester_error: Option<String>,
    test_focused: bool,
    n: u64,
    /// Values being dragged or just set, until settings.toml has them.
    pub drafts: HashMap<&'static str, f64>,
    pub device: Option<Device>,
    choices: Option<(bool, Vec<tune::Choice>)>,
    /// Cards are as tall as they were last drawn.
    heights: HashMap<&'static str, f32>,
    /// An editor just opened: bring it into view.
    reveal: bool,
}

impl KeysUi {
    pub fn new() -> Self {
        KeysUi { check_due: true, focus_key: 58, ..Default::default() }
    }
}

fn mac() -> bool {
    cfg!(target_os = "macos")
}

/// A card whose height is known only after its contents are drawn: the
/// background goes in first (behind), its shape is set at the end.
struct Card {
    bg: egui::layers::ShapeIdx,
    top: Pos2,
    w: f32,
    inner: Rect,
}

fn card_begin(ui: &Ui, top: Pos2, w: f32, title: &str) -> Card {
    let bg = ui.painter().add(egui::Shape::Noop);
    let r = Rect::from_min_size(top, vec2(w, 200.0));
    let p = ui.painter();
    p.text(r.left_top() + vec2(18.0, 26.0), Align2::LEFT_CENTER, title, semibold(T_TITLE), INK);
    a11y_text(ui, Id::unique(("card", title)), Rect::from_min_size(r.min, vec2(w, 44.0)), Role::Heading, title, None);
    Card { bg, top, w, inner: Rect::from_min_max(r.min + vec2(18.0, 48.0), pos2(r.right() - 18.0, f32::INFINITY)) }
}

impl Card {
    /// Closes the card below `bottom`; its height.
    fn end(self, ui: &Ui, bottom: f32) -> f32 {
        let r = Rect::from_min_max(self.top, pos2(self.top.x + self.w, bottom + 14.0));
        ui.painter().set(self.bg, egui::Shape::Rect(egui::epaint::RectShape::new(r, CornerRadius::same(12), PANEL, Stroke::new(1.0, LINE), StrokeKind::Inside)));
        r.height()
    }
}

/// Buttons left to right, onto a new line when the row is full; returns
/// each one's rect and the bottom.
fn flow(ui: &Ui, left: f32, top: f32, width: f32, labels: &[&str]) -> (Vec<Rect>, f32) {
    let (mut x, mut y) = (left, top);
    let mut out = vec![];
    for l in labels {
        let w = button_width(ui, l) - 4.0;
        if x > left && x + w > left + width {
            x = left;
            y += 38.0;
        }
        out.push(Rect::from_min_size(pos2(x, y), vec2(w.min(width), 30.0)));
        x += w + 8.0;
    }
    (out, y + 30.0)
}

/// A chip that is on or off: a quick setup.
fn chip(ui: &mut Ui, rect: Rect, id: Id, label: &str, on: bool, enabled: bool) -> egui::Response {
    let r = ui.interact(rect, id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let hover = tween(ui.ctx(), id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    let p = ui.painter();
    let radius = CornerRadius::same((rect.height() / 2.0) as u8);
    p.rect(rect, radius, mix(if on { PILL } else { PANEL }, BUTTON_LIT, hover), Stroke::new(1.0, if on { INK2 } else { KEY_EDGE }), StrokeKind::Inside);
    let mut x = rect.left() + 14.0;
    if on {
        widgets::draw_check(p, pos2(x + 5.0, rect.center().y), 11.0, INK, 1.0);
        x += 16.0;
    }
    let text = fit(p, label, &regular(T_META + 1.0), rect.right() - 12.0 - x);
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, regular(T_META + 1.0), if !enabled { INK2 } else if on { INK } else { mix(INK2, INK, hover) });
    widgets::focus_ring(ui, &r, rect, radius.nw);
    if r.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (name, enabled) = (label.to_owned(), r.enabled());
    r.widget_info(move || egui::WidgetInfo::selected(Role::CheckBox, enabled, on, &name));
    r
}

fn chip_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, regular(T_META + 1.0)) + 46.0
}

/// A key's legend in Geist Mono, a size or two smaller before it is cut.
fn cap_text(p: &Painter, text: &str, max_w: f32, start: f32) -> (String, egui::FontId) {
    for size in [start, start - 1.0, start - 2.0] {
        let f = mono(size.max(9.0));
        if p.layout_no_wrap(text.to_owned(), f.clone(), INK).size().x <= max_w {
            return (text.to_owned(), f);
        }
    }
    let f = mono((start - 2.0).max(9.0));
    (fit(p, text, &f, max_w), f)
}

/// Room for the quick setups beside the keyboard.
const SIDE_W: f32 = 270.0;

/// A slider from 0 to 1: dragged, clicked, or moved with the arrows when
/// focused. Returns the new position and whether to apply it now (let go,
/// or a step with the keyboard).
fn slider(ui: &mut Ui, rect: Rect, id: Id, t: f32, name: &str, value: f64) -> (Option<f32>, bool) {
    let r = ui.interact(rect, id, widgets::sense(ui, Sense::click_and_drag()));
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 18.0, 4.0));
    let mut out = None;
    let mut apply = false;
    if (r.dragged() || r.clicked()) && let Some(p) = r.interact_pointer_pos() {
        out = Some(((p.x - track.left()) / track.width()).clamp(0.0, 1.0));
        apply = r.clicked();
    }
    if r.drag_stopped() {
        apply = true;
        out = out.or(Some(t));
    }
    if r.has_focus() {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { horizontal_arrows: true, ..Default::default() }));
        let step = ui.input(|i| f32::from(i.key_pressed(Key::ArrowRight)) - f32::from(i.key_pressed(Key::ArrowLeft)));
        if step != 0.0 {
            out = Some((t + step / 46.0).clamp(0.0, 1.0));
            apply = true;
        }
    }
    let shown = out.unwrap_or(t);
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(2), TRACK_OFF);
    let x = track.left() + track.width() * shown;
    p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.bottom())), CornerRadius::same(2), INK2);
    // The middle is "as it is" (1×).
    p.line_segment([pos2(track.center().x, track.top() - 5.0), pos2(track.center().x, track.top() - 2.0)], Stroke::new(1.0, KEY_EDGE));
    let knob = pos2(x, track.center().y);
    p.circle_filled(knob, 8.0, INK);
    if r.has_focus() && widgets::keyboard_focus(ui.ctx()) {
        p.circle_stroke(knob, 11.0, Stroke::new(2.0, SIGNAL));
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (name, enabled) = (name.to_owned(), r.enabled());
    r.widget_info(move || egui::WidgetInfo::slider(enabled, value, &name));
    (out, apply)
}

/// Draws a pencil: edit.
fn draw_pencil(p: &Painter, rect: Rect, col: Color32) {
    let c = rect.center();
    let st = Stroke::new(1.4, col);
    p.line_segment([c + vec2(-4.5, 4.5), c + vec2(3.5, -3.5)], st);
    p.line_segment([c + vec2(-2.5, 5.5), c + vec2(5.0, -2.0)], st);
    p.line_segment([c + vec2(3.5, -3.5), c + vec2(5.0, -2.0)], st);
    p.line_segment([c + vec2(-4.5, 4.5), c + vec2(-5.5, 6.0)], st);
}

/// Draws a cross: remove, close.
fn draw_cross(p: &Painter, rect: Rect, col: Color32) {
    let c = rect.center();
    let st = Stroke::new(1.5, col);
    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], st);
    p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], st);
}

/// An error line: a red dot and the message, wrapped. Its height.
fn error_line(ui: &Ui, left: f32, y: f32, width: f32, text: &str) -> f32 {
    ui.painter().circle_filled(pos2(left + 3.0, y + 9.0), 3.0, ERROR);
    let h = widgets::para_code(ui, pos2(left + 14.0, y), width - 14.0, text, regular(T_BODY), INK);
    a11y_text(ui, Id::unique(("keys-error", text)), Rect::from_min_size(pos2(left, y), vec2(width, h)), Role::Alert, text, Some(Live::Assertive));
    h
}

impl App {
    /// The page takes the keyboard (a key being pressed for a rule, or the
    /// test box): the window's own shortcuts wait.
    pub(super) fn keys_grab(&self, ctx: &egui::Context) -> bool {
        self.tab == Tab::Keys && (self.keys.capture.is_some() || ctx.memory(|m| m.has_focus(Id::unique(TEST_ID))))
    }

    fn scope_peers(&self) -> Vec<tune::Peer> {
        self.running().map_or(vec![], |s| s.peers.iter().map(|p| tune::Peer { name: p.name.clone(), id: p.id.clone(), os: p.os }).collect())
    }

    fn facts(&self) -> Facts {
        Facts::of(&self.keys.scope, &self.scope_peers())
    }

    fn tuning_busy(&self) -> bool {
        self.busy.iter().any(|j| matches!(j, Job::Tune { .. }))
    }

    /// Runs `cmds` in order through `one-mk`; an error shows in `area`.
    fn tune_cmds(&mut self, what: &str, area: &'static str, cmds: Vec<Vec<String>>, key: Option<String>) {
        if cmds.is_empty() {
            return;
        }
        self.keys.n += 1;
        let job = Job::Tune { what: what.into(), area, n: self.keys.n };
        self.busy.push(job.clone());
        if self.keys.error.as_ref().is_some_and(|(a, _)| *a == area) {
            self.keys.error = None;
        }
        self.link.tune(job, cmds, key);
    }

    /// A tuning command finished.
    pub(super) fn tuned(&mut self, area: &'static str, what: &str, error: Option<String>) {
        self.keys.check_due = true;
        if let Some(e) = error {
            self.keys.drafts.clear();
            self.prefs = self.link.prefs();
            self.keys.error = Some((area, format!("Couldn't change {what}: {e}")));
        }
    }

    /// The rule for a key in the chosen scope and layer, if there is one.
    fn own_rule(&self, code: u16) -> Option<(usize, KeyRule)> {
        let name = keynames::key_name(u32::from(code));
        let want = KeyRule { scope: self.keys.scope.clone(), layer: self.keys.layer.clone(), ..KeyRule::to(&name, "none") };
        let i = self.prefs.input.keys.iter().position(|r| {
            r.scope == want.scope
                && tune::same_app(&r.app, &self.keys.app)
                && r.layer.as_ref().map(|l| l.to_ascii_lowercase()) == want.layer.as_ref().map(|l| l.to_ascii_lowercase())
                && r.from.parse::<keynames::Trigger>().is_ok_and(|t| t == keynames::Trigger::key(code))
        })?;
        Some((i, self.prefs.input.keys[i].clone()))
    }

    /// What `value` (a key, chord or action, as written) does for the
    /// key or button being changed: checked, then saved.
    pub(super) fn choose(&mut self, want: Want, value: String) {
        self.keys.capture = None;
        self.keys.picker = None;
        if want == Want::Button {
            let Some(n) = self.keys.button else { return };
            if let Err(e) = tune::check_button(n, &value) {
                self.keys.error = Some(("buttons", e));
                return;
            }
            let name = tune::BUTTONS.iter().find(|b| b.0 == n).map_or("the button", |b| b.1);
            self.tune_cmds(&name.to_lowercase(), "buttons", vec![tune::button_args(n, Some(&value), &self.keys.scope)], None);
            return;
        }
        let Some(code) = self.keys.selected else { return };
        let from = keynames::key_name(u32::from(code));
        let own = self.own_rule(code).map(|(_, r)| r);
        let base = KeyRule { scope: self.keys.scope.clone(), layer: self.keys.layer.clone(), app: self.keys.app.clone(), ..Default::default() };
        let tap_of = |r: &KeyRule| r.tap.clone().or_else(|| match &r.to {
            Some(tune::To::One(s)) => Some(s.clone()),
            _ => None,
        });
        let rule = match (want, own.as_ref().and_then(|r| r.hold.clone())) {
            (Want::To, Some(hold)) => KeyRule { tap: Some(value), hold: Some(hold), timeout_ms: own.as_ref().and_then(|r| r.timeout_ms), from, ..base },
            (Want::Hold, _) => {
                let tap = own.as_ref().and_then(tap_of).unwrap_or_else(|| from.clone());
                KeyRule { tap: Some(tap), hold: Some(value), timeout_ms: own.as_ref().and_then(|r| r.timeout_ms), from, ..base }
            }
            _ => KeyRule { to: Some(tune::To::One(value)), from, ..base },
        };
        self.save_rule(rule);
    }

    fn save_rule(&mut self, rule: KeyRule) {
        if let Err(e) = tune::check_add(&self.prefs.input, &rule) {
            self.keys.error = Some(("keyboard", e));
            return;
        }
        let what = rule.from.parse::<keynames::Trigger>().map_or(rule.from.clone(), |t| tune::trigger_text(&t));
        self.tune_cmds(&what, "keyboard", vec![tune::add_args(&rule)], None);
    }

    pub(super) fn select_key(&mut self, code: u16, layer: Option<String>) {
        self.keys.selected = Some(code);
        self.keys.focus_key = code;
        self.keys.layer = layer;
        self.keys.app = None;
        self.keys.capture = None;
        self.keys.picker = None;
        self.keys.more = self.own_rule(code).is_some_and(|(_, r)| r.hold.is_some());
        self.keys.reveal = true;
        if self.keys.error.as_ref().is_some_and(|(a, _)| *a == "keyboard") {
            self.keys.error = None;
        }
    }

    /// Development aid: types `spec` into the test box (`58d0,58u85`: a
    /// code, down or up, at a time in ms), as if it had just happened.
    #[cfg(debug_assertions)]
    pub(super) fn debug_type(&mut self, ctx: &egui::Context, spec: &str) {
        ctx.memory_mut(|m| m.request_focus(Id::unique(TEST_ID)));
        self.keys.test_focused = true;
        let steps: Vec<(u16, bool, u64)> = spec
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                let at = s.find(['d', 'u'])?;
                Some((s[..at].parse().ok()?, &s[at..=at] == "d", s[at + 1..].parse().ok()?))
            })
            .collect();
        let end = steps.iter().map(|s| s.2).max().unwrap_or(0);
        let base = Instant::now() - std::time::Duration::from_millis(end);
        if let Some(t) = self.tester() {
            for (code, down, at) in steps {
                let when = base + std::time::Duration::from_millis(at);
                t.tick(when);
                t.key(code, down, when);
            }
        }
    }

    /// The rules compiled for the chosen computers, for the test box.
    fn tester(&mut self) -> Option<&mut Tester> {
        let facts = self.facts();
        let input = &self.prefs.input;
        let fresh = !self.keys.tester.as_ref().is_some_and(|(_, i, f)| i == input && *f == facts);
        if fresh {
            match input.keymap(&facts) {
                Ok(map) => {
                    self.keys.tester_error = None;
                    match &mut self.keys.tester {
                        Some((t, i, f)) => {
                            t.set_keymap(map, input.tap());
                            (*i, *f) = (input.clone(), facts);
                        }
                        None => self.keys.tester = Some((Tester::new(map, input.tap()), input.clone(), facts)),
                    }
                }
                Err(e) => {
                    self.keys.tester_error = Some(e);
                    self.keys.tester = None;
                }
            }
        }
        self.keys.tester.as_mut().map(|(t, ..)| t)
    }

    /// Key events for a key being pressed for a rule, or for the test box.
    fn keys_input(&mut self, ui: &mut Ui) {
        let take = |ui: &mut Ui, tab_too: bool| -> Vec<Event> {
            ui.input_mut(|i| {
                let (taken, kept): (Vec<Event>, Vec<Event>) = std::mem::take(&mut i.events).into_iter().partition(|e| match e {
                    Event::Key { key: Key::Tab, .. } => tab_too,
                    Event::Key { .. } | Event::Copy | Event::Cut | Event::Paste(_) | Event::Text(_) => true,
                    _ => false,
                });
                i.events = kept;
                taken
            })
        };
        if let Some((want, mut cap)) = self.keys.capture {
            let id = Id::unique(CAPTURE_ID);
            ui.memory_mut(|m| {
                m.request_focus(id);
                m.set_focus_lock_filter(id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true });
            });
            for e in take(ui, true) {
                let Some((code, down, repeat)) = tune::event_key(&e) else { continue };
                if repeat {
                    continue;
                }
                if code == 1 && down && !cap.holding() {
                    self.keys.capture = None;
                    return;
                }
                if let Some(chord) = cap.key(code, down) {
                    self.choose(want, chord.to_string());
                    return;
                }
            }
            self.keys.capture = Some((want, cap));
            return;
        }
        let id = Id::unique(TEST_ID);
        let focused = ui.memory(|m| m.has_focus(id));
        if self.keys.test_focused && !focused && let Some((t, ..)) = &mut self.keys.tester {
            t.release();
        }
        self.keys.test_focused = focused;
        if !focused {
            return;
        }
        ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        let events = take(ui, false);
        let now = Instant::now();
        if let Some(t) = self.tester() {
            for e in &events {
                if let Some((code, down, _)) = tune::event_key(e) {
                    t.key(code, down, now);
                }
            }
        }
    }

    // ---------- the page ----------

    pub(super) fn keys_page(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        if self.keys.focus_key == 0 {
            self.keys.focus_key = 58;
        }
        if self.keys.check_due && !self.tuning_busy() {
            self.keys.check_due = false;
            self.link.keymap_check();
        }
        self.keys_input(ui);
        // A tap/hold key in the test box decides when its time is up.
        if let Some((t, ..)) = &mut self.keys.tester
            && let Some(d) = t.deadline()
        {
            let i = Instant::now();
            if i >= d {
                t.tick(i);
            } else if motion::reduced() {
                ui.ctx().request_repaint_after(d - i);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(33).min(d - i));
            }
        }
        let outer = Rect::from_min_max(rect.min + vec2(24.0, 14.0), rect.max - vec2(24.0, 12.0));
        let w = outer.width().min(1080.0);
        let area = Rect::from_min_max(pos2(outer.center().x - w / 2.0, outer.top()), pos2(outer.center().x + w / 2.0, outer.bottom()));

        // Keyboard or mouse, and for which computers.
        let track = Rect::from_min_size(area.min, vec2(210.0, 32.0));
        let labels = ["Keyboard", "Mouse"];
        let pills = Pills {
            id: Id::unique("keys-view"),
            track,
            cells: Pills::even(track, 2, 3.0),
            labels: &labels,
            names: vec!["Keyboard: change keys".into(), "Mouse: buttons, pointer and scrolling".into()],
            chosen: Some(usize::from(self.keys.view == View::Mouse)),
            enabled: vec![true; 2],
            role: Role::Tab,
            font: T_BODY,
            radius: 7,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.keys.view = if k == 0 { View::Keyboard } else { View::Mouse };
            self.keys.capture = None;
            self.keys.picker = None;
        }
        self.scope_menu(ui, Rect::from_min_max(pos2(track.right() + 16.0, area.top()), pos2(area.right(), area.top() + 32.0)));
        if self.tuning_busy() {
            let p = ui.painter();
            let x = track.right() + 16.0;
            widgets::draw_refresh(p, Rect::from_center_size(pos2(x + 8.0, track.center().y), vec2(16.0, 16.0)), INK2, if motion::reduced() { 0.0 } else { (now * 5.0) as f32 });
            p.text(pos2(x + 22.0, track.center().y), Align2::LEFT_CENTER, "Saving…", regular(T_META), INK2);
            ui.ctx().request_repaint_after_secs(1.0 / 30.0);
        }

        let body = Rect::from_min_max(pos2(area.left(), area.top() + 46.0), area.max);
        let mut child = ui.new_child(UiBuilder::new().max_rect(body.expand2(vec2(6.0, 0.0))));
        child.set_clip_rect(body.expand2(vec2(8.0, 4.0)));
        let salt = if self.keys.view == View::Keyboard { "keys-keyboard" } else { "keys-mouse" };
        if motion::reduced() {
            child.style_mut().scroll_animation = egui::style::ScrollAnimation::none();
        }
        egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(&mut child, |ui| match self.keys.view {
            View::Keyboard => self.keyboard_view(ui, body.width(), now),
            View::Mouse => self.mouse_view(ui, body.width()),
        });
    }

    /// "For: All computers ▾": the computers changes are for.
    fn scope_menu(&mut self, ui: &mut Ui, room: Rect) {
        let label = format!("For: {}", self.keys.scope.label());
        let lw = (button_width(ui, &label) + 14.0).min(room.width()).min(380.0);
        let r = Rect::from_min_size(pos2(room.right() - lw, room.top()), vec2(lw, 32.0));
        let shown = fit(ui.painter(), &label, &semibold(T_BODY), lw - 40.0);
        let resp = button(ui, r, Id::unique("keys-scope"), "", Look::Quiet, true, Some(&format!("{label}. Choose which computers the changes are for")));
        let p = ui.painter();
        p.text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, &shown, semibold(T_BODY), INK);
        let c = pos2(r.right() - 16.0, r.center().y);
        p.add(egui::Shape::line(vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)], Stroke::new(1.5, INK2)));
        let peers = self.scope_peers();
        let mut chosen = None;
        egui::Popup::menu(&resp).width(260.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            let mut opts = vec![Scope::All, Scope::Os(os::MACOS), Scope::Os(os::WINDOWS), Scope::Os(os::LINUX)];
            opts.extend(peers.iter().map(|p| Scope::Peer(p.name.clone())));
            let p = ui.painter().clone();
            for (k, s) in opts.iter().enumerate() {
                if k == 4 {
                    let (r, _) = ui.allocate_exact_size(vec2(260.0, 22.0), Sense::hover());
                    p.text(r.left_center() + vec2(8.0, 2.0), Align2::LEFT_CENTER, "One computer", regular(T_META), INK2);
                }
                let (r, resp) = ui.allocate_exact_size(vec2(260.0, 28.0), Sense::click());
                let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(resp.hovered()), 0.12, motion::out_cubic);
                p.rect_filled(r, CornerRadius::same(6), mix(Color32::TRANSPARENT, PILL, hover));
                widgets::focus_ring(ui, &resp, r, 6);
                let on = *s == self.keys.scope;
                if on {
                    widgets::draw_check(&p, pos2(r.left() + 16.0, r.center().y), 10.0, INK, 1.0);
                }
                let text = fit(&p, &s.label(), &regular(T_BODY), r.width() - 40.0);
                p.text(pos2(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, &text, regular(T_BODY), if on { INK } else { INK_HOVER });
                widgets::a11y(&resp, Role::MenuItemRadio, &format!("For: {}", s.label()), Some(on));
                if resp.clicked() {
                    chosen = Some(s.clone());
                    ui.close();
                }
            }
        });
        if let Some(s) = chosen
            && s != self.keys.scope
        {
            self.keys.scope = s;
            self.keys.capture = None;
            self.keys.picker = None;
        }
    }

    // ---------- keyboard ----------

    fn keyboard_view(&mut self, ui: &mut Ui, w: f32, now: f64) {
        let input = self.prefs.input.clone();
        let facts = self.facts();
        let (kw, kh) = tune::keyboard_size(self.keys.keypad);
        let presets = tune::presets(mac());
        // In a wide window the quick setups stand beside the keyboard;
        // otherwise they flow under it onto as many lines as they need.
        let inner = w - 36.0;
        let side = inner - kw * 30.0 - 24.0 >= SIDE_W;
        let unit = if side { ((inner - SIDE_W - 24.0) / kw).min(UNIT_MAX) } else { (inner / kw).min(UNIT_MAX) }.floor();
        let kb_h = (18.0 + 26.0 + 10.0 + kh * unit + 16.0).max(if side { 54.0 + 26.0 + 38.0 * presets.len() as f32 + 8.0 } else { 0.0 });
        let head_w = text_width(ui, "Quick setups", semibold(T_BODY)) + 16.0;
        let mut chips = vec![];
        let (mut x, mut y) = (head_w, 0.0);
        for (k, p) in presets.iter().enumerate() {
            if side {
                chips.push(Rect::from_min_size(pos2(w - 18.0 - SIDE_W, 54.0 + 26.0 + 38.0 * k as f32 - kb_h - 14.0), vec2(SIDE_W, 30.0)));
                continue;
            }
            let cw = chip_width(ui, &p.label).min(w);
            if x > head_w && x + cw > w {
                (x, y) = (0.0, y + 38.0);
            }
            chips.push(Rect::from_min_size(pos2(x, y), vec2(cw, 30.0)));
            x += cw + 8.0;
        }
        let chips_h = if side { 2.0 } else { y + 30.0 + 16.0 };
        let two = w >= 700.0;
        let col_w = if two { (w - GAP) / 2.0 } else { w };
        let h = |k: &str| self.keys.heights.get(k).copied().unwrap_or(220.0);
        let lower = if two { h("left").max(h("test")) } else { h("left") + GAP + h("test") };
        let total = kb_h + 14.0 + chips_h + lower;
        let (all, _) = ui.allocate_exact_size(vec2(w, total), Sense::hover());
        let top = all.top();

        self.keyboard_card(ui, Rect::from_min_size(all.min, vec2(w, kb_h)), unit, side, &input, &facts);

        let cy = top + kb_h + 14.0;
        let head = if side { pos2(all.left() + w - 18.0 - SIDE_W, top + 54.0 + 10.0) } else { pos2(all.left(), cy + 15.0) };
        ui.painter().text(head, Align2::LEFT_CENTER, "Quick setups", semibold(T_BODY), INK);
        let busy = self.tuning_busy();
        for (p, r) in presets.iter().zip(&chips) {
            let on = tune::preset_on(&input, p, &self.keys.scope);
            if chip(ui, r.translate(vec2(all.left(), cy)), Id::unique(("preset", p.id)), &p.label, on, !busy).clicked() {
                // Each rule is checked as `one-mk keymap add` would, in turn.
                let mut sim = input.clone();
                let mut bad = None;
                if !on {
                    for r in &p.rules {
                        let r = KeyRule { scope: self.keys.scope.clone(), ..r.clone() };
                        if let Err(e) = tune::check_add(&sim, &r) {
                            bad = Some(e);
                            break;
                        }
                        sim.keys.push(r);
                    }
                }
                match bad {
                    Some(e) => self.keys.error = Some(("keyboard", format!("“{}” can't be used: {e}", p.label))),
                    None => self.tune_cmds(&format!("“{}”", p.label), "keyboard", tune::preset_commands(p, &self.keys.scope, !on), None),
                }
            }
        }

        let ly = cy + chips_h;
        let left = pos2(all.left(), ly);
        let lh = if self.keys.selected.is_some() { self.key_editor(ui, left, col_w, &input, &facts) } else { self.rules_card(ui, left, col_w, &input) };
        if std::mem::take(&mut self.keys.reveal) {
            // Its title and first choices, at least, in view.
            ui.scroll_to_rect(Rect::from_min_size(left, vec2(col_w, lh.min(130.0))), None);
        }
        let test_at = if two { pos2(all.left() + col_w + GAP, ly) } else { pos2(all.left(), ly + lh + GAP) };
        let th = self.test_card(ui, test_at, col_w, now);
        for (k, v) in [("left", lh), ("test", th)] {
            if (self.keys.heights.get(k).copied().unwrap_or(0.0) - v).abs() > 0.5 {
                self.keys.heights.insert(k, v);
                ui.ctx().request_repaint();
            }
        }
    }

    fn keyboard_card(&mut self, ui: &mut Ui, r: Rect, unit: f32, side: bool, input: &Input, facts: &Facts) {
        let p = ui.painter().clone();
        p.rect(r, CornerRadius::same(12), PANEL, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let label = if self.keys.keypad { "Fewer keys" } else { "More keys" };
        let bw = button_width(ui, label) - 8.0;
        let b = Rect::from_min_size(pos2(r.right() - 18.0 - bw, r.top() + 14.0), vec2(bw, 28.0));
        let intro = "Click a key, then choose what it types on the other computers.";
        let iw = b.left() - r.left() - 30.0;
        p.text(pos2(r.left() + 18.0, b.center().y), Align2::LEFT_CENTER, fit(&p, intro, &regular(T_BODY), iw), regular(T_BODY), INK2);
        a11y_text(ui, Id::unique("keys-intro"), Rect::from_min_size(r.min, vec2(iw, 40.0)), Role::Label, intro, None);
        if button(ui, b, Id::unique("keys-more"), label, Look::Quiet, true, Some(if self.keys.keypad { "Hide the keypad" } else { "Show the keypad too" })).clicked() {
            self.keys.keypad = !self.keys.keypad;
        }
        let caps = tune::keyboard(self.keys.keypad, mac());
        let (kw, _) = tune::keyboard_size(self.keys.keypad);
        let room = if side { r.width() - 36.0 - SIDE_W - 24.0 } else { r.width() - 36.0 };
        let left = r.left() + 18.0 + (room - kw * unit) / 2.0;
        let origin = pos2(left, r.top() + 18.0 + 26.0 + 10.0);
        // What each key does for these computers (plain keys, base layer).
        let mut target: HashMap<u16, What> = HashMap::new();
        for (s, _) in input.specs(facts) {
            if s.layer.is_none() && !s.from.has_mods() && !keynames::is_button(u32::from(s.from.key)) {
                target.entry(s.from.key).or_insert(s.what);
            }
        }
        let held: Vec<u16> = if self.keys.test_focused { self.keys.tester.as_ref().map_or(vec![], |(t, ..)| t.pressed.down().to_vec()) } else { vec![] };
        if !caps.iter().any(|c| c.code == self.keys.focus_key) {
            self.keys.focus_key = 58;
        }
        let mut go = None;
        for (i, c) in caps.iter().enumerate() {
            let rect = Rect::from_min_size(origin + vec2(c.x * unit, c.y * unit), vec2(c.w * unit, c.h * unit)).shrink(2.0);
            let id = Id::unique(("cap", c.code));
            let focusable = c.code == self.keys.focus_key;
            let sense = widgets::sense(ui, if focusable { Sense::click() } else { Sense::CLICK });
            let resp = ui.interact(rect, id, sense);
            if resp.has_focus() {
                ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
                let (dx, dy) = ui.input(|k| {
                    let d = |a: Key, b: Key| i8::from(k.key_pressed(b)) - i8::from(k.key_pressed(a));
                    (d(Key::ArrowLeft, Key::ArrowRight), d(Key::ArrowUp, Key::ArrowDown))
                });
                if (dx, dy) != (0, 0)
                    && let Some(n) = tune::neighbour(&caps, i, dx, dy)
                {
                    go = Some(caps[n].code);
                }
            }
            if resp.clicked() {
                self.select_key(c.code, None);
            }
            let what = target.get(&c.code);
            let selected = self.keys.selected == Some(c.code) && self.keys.layer.is_none();
            let down = held.contains(&c.code);
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            let fill = if down { TRACK_EDGE } else if what.is_some() { BUTTON_LIT } else { BUTTON };
            p.rect(rect, CornerRadius::same(5), mix(fill, PILL, hover * 0.6), Stroke::new(1.0, if what.is_some() { INK2 } else { KEY_EDGE }), StrokeKind::Inside);
            let name = tune::cap_label(c.code);
            match what {
                Some(w) => {
                    let small = mono(9.0);
                    p.text(rect.left_top() + vec2(4.0, 2.0), Align2::LEFT_TOP, fit(&p, &name, &small, rect.width() - 6.0), small, INK2);
                    let (t, f) = cap_text(&p, &tune::cap_target(w), rect.width() - 4.0, 11.0);
                    p.text(pos2(rect.center().x, rect.bottom() - 3.0), Align2::CENTER_BOTTOM, t, f, INK);
                }
                None => {
                    let (t, f) = cap_text(&p, &name, rect.width() - 4.0, if unit >= 30.0 { T_META } else { 11.0 });
                    p.text(rect.center(), Align2::CENTER_CENTER, t, f, if down { INK } else { INK2 });
                }
            }
            if selected {
                p.rect_stroke(rect, CornerRadius::same(5), Stroke::new(2.0, SIGNAL), StrokeKind::Inside);
            }
            widgets::focus_ring(ui, &resp, rect, 5);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let full = tune::key_label(u32::from(c.code));
            let does = match what {
                Some(w) => {
                    let rule = KeyRule { from: keynames::key_name(u32::from(c.code)), ..Default::default() };
                    let s = match w {
                        What::To(o) => tune::out_text(o),
                        What::TapHold { tap, hold, .. } => format!("{} when tapped, {} when held", tune::out_text(tap), tune::out_text(hold)),
                    };
                    let _ = rule;
                    format!("{full}: {s}")
                }
                None => format!("{full}: unchanged"),
            };
            let resp = if what.is_some() { resp.on_hover_text(&does) } else { resp };
            widgets::a11y(&resp, Role::Button, &format!("{does}. Arrows move between keys"), Some(selected));
        }
        if let Some(code) = go {
            self.keys.focus_key = code;
            ui.memory_mut(|m| m.request_focus(Id::unique(("cap", code))));
        }
    }

    /// The rules, as sentences, with what `one-mk keymap check` found.
    fn rules_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input) -> f32 {
        let n = input.keys.len();
        let title = if n == 0 { "Changed keys".to_owned() } else { format!("Changed keys ({n})") };
        let card = card_begin(ui, at, w, &title);
        let c = card.inner;
        let mut y = c.top() - 6.0;
        if self.prefs.unreadable {
            y += error_line(ui, c.left(), y, c.width(), "settings.toml couldn't be read, so nothing here can be shown or changed. Fix it, or open it from Advanced.") + 8.0;
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "list") {
            y += error_line(ui, c.left(), y, c.width(), &e) + 8.0;
        }
        match &self.keys.problems {
            Some(Ok(list)) => {
                for prob in list.iter().take(4) {
                    y += error_line(ui, c.left(), y, c.width(), &tune::plain_problem(prob)) + 6.0;
                }
                if list.len() > 4 {
                    ui.painter().text(pos2(c.left() + 14.0, y + 9.0), Align2::LEFT_CENTER, format!("and {} more: run `one-mk keymap check`", list.len() - 4), regular(T_META), INK2);
                    y += 24.0;
                }
            }
            Some(Err(e)) => y += error_line(ui, c.left(), y, c.width(), &format!("The rules couldn't be checked: {e}")) + 6.0,
            None => {}
        }
        if n == 0 {
            let text = "No keys are changed yet. Click a key above, or start with a quick setup.";
            y += para(ui, pos2(c.left(), y + 2.0), c.width(), text, regular(T_BODY), INK2) + 4.0;
            return card.end(ui, y);
        }
        let busy = self.tuning_busy();
        let mut act = None;
        for (i, r) in input.keys.iter().enumerate() {
            let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 34.0));
            let (text, ok) = match tune::sentence(input, r) {
                Ok(s) => (s, true),
                Err(e) => (format!("{} ({e})", tune::as_written(r)), false),
            };
            let tag = match &r.scope {
                Scope::All => String::new(),
                s => s.label(),
            };
            let bx = row.right() - 26.0;
            let edit = Rect::from_center_size(pos2(bx - 32.0, row.center().y), vec2(28.0, 28.0));
            let del = Rect::from_center_size(pos2(bx + 13.0, row.center().y), vec2(28.0, 28.0));
            let tw = if tag.is_empty() { 0.0 } else { text_width(ui, &tag, regular(T_META)).min(c.width() * 0.3) + 12.0 };
            let room = edit.left() - 8.0 - row.left() - tw - if ok { 0.0 } else { 14.0 };
            let x = if ok {
                row.left()
            } else {
                ui.painter().circle_filled(pos2(row.left() + 3.0, row.center().y), 3.0, ERROR);
                row.left() + 14.0
            };
            let shown = fit(ui.painter(), &text, &regular(T_BODY), room);
            ui.painter().text(pos2(x, row.center().y), Align2::LEFT_CENTER, &shown, regular(T_BODY), INK);
            if !tag.is_empty() {
                let t = fit(ui.painter(), &tag, &regular(T_META), tw - 12.0);
                ui.painter().text(pos2(edit.left() - 10.0, row.center().y), Align2::RIGHT_CENTER, t, regular(T_META), INK2);
            }
            let hover_r = Rect::from_min_max(row.min, pos2(edit.left() - 8.0, row.bottom()));
            let full = if tag.is_empty() { text.clone() } else { format!("{text} ({tag})") };
            let resp = ui.interact(hover_r, Id::unique(("rule-text", i)), Sense::hover());
            if shown != text {
                resp.on_hover_text(&full);
            }
            a11y_text(ui, Id::unique(("rule", i)), row, Role::ListItem, &full, None);
            let editable = r.from.parse::<keynames::Trigger>().is_ok_and(|t| !t.has_mods());
            if widgets::icon_button(ui, edit, Id::unique(("rule-edit", i)), &format!("Change: {text}"), editable && !busy, draw_pencil).clicked() {
                act = Some((i, true));
            }
            if widgets::icon_button(ui, del, Id::unique(("rule-del", i)), &format!("Remove: {text}"), !busy, draw_cross).clicked() {
                act = Some((i, false));
            }
            if i + 1 < n {
                ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, LINE));
            }
            y += 34.0;
        }
        match act {
            Some((i, true)) => {
                let r = input.keys[i].clone();
                if let Ok(t) = r.from.parse::<keynames::Trigger>() {
                    self.keys.scope = r.scope.clone();
                    self.select_key(t.key, r.layer.clone());
                    self.keys.app = r.app.clone();
                    self.keys.more = r.hold.is_some();
                }
            }
            Some((i, false)) => {
                let r = &input.keys[i];
                let what = format!("the rule for {}", r.from.parse::<keynames::Trigger>().map_or(r.from.clone(), |t| tune::trigger_text(&t)));
                self.tune_cmds(&what, "list", vec![tune::remove_args(r)], None);
            }
            None => {}
        }
        card.end(ui, y - 4.0)
    }

    /// Changing one key: press the key it becomes, choose it from the
    /// list, or make it do nothing; "Different when held" under More.
    fn key_editor(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let code = self.keys.selected.unwrap_or(58);
        let name = tune::key_label(u32::from(code));
        let title = match &self.keys.layer {
            Some(l) => match input.layer_key(l) {
                Some(k) => format!("{} + {name}", tune::key_label(u32::from(k))),
                None => format!("{name} in “{l}”"),
            },
            None => name.clone(),
        };
        let card = card_begin(ui, at, w, &title);
        let c = card.inner;
        let close = Rect::from_min_size(pos2(at.x + w - 12.0 - 28.0, at.y + 12.0), vec2(28.0, 28.0));
        let esc = !self.keys_grab(ui.ctx()) && self.keys.picker.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) && !ui.ctx().text_edit_focused();
        if widgets::icon_button(ui, close, Id::unique("key-close"), &format!("Done with {title}"), true, draw_cross).clicked() || esc {
            self.keys.selected = None;
            self.keys.capture = None;
            self.keys.picker = None;
            ui.memory_mut(|m| m.request_focus(Id::unique(("cap", code))));
        }
        let own = self.own_rule(code).map(|(_, r)| r);
        let shown = input
            .specs(facts)
            .into_iter()
            .find(|(s, _)| s.from == keynames::Trigger::key(code) && s.layer.as_deref() == self.keys.layer.as_deref().map(str::to_ascii_lowercase).as_deref())
            .map(|(_, at)| at);
        let now_text = match (&own, shown) {
            (Some(r), _) => tune::sentence(input, r).map(|s| s.split_once(" → ").map_or(s.clone(), |(_, t)| t.to_owned())).unwrap_or_else(|e| e),
            (None, Some(Ok(i))) => {
                let r = &input.keys[i];
                let s = tune::sentence(input, r).unwrap_or_default();
                format!("{} ({})", s.split_once(" → ").map_or(s.as_str(), |(_, t)| t), r.scope.label().to_lowercase())
            }
            _ => format!("Types {name}, as it is."),
        };
        let mut y = c.top() - 6.0;
        y += widgets::para_code(ui, pos2(c.left(), y), c.width(), &format!("Now: {now_text}"), regular(T_BODY), INK) + 2.0;
        let scope_note = match &self.keys.scope {
            Scope::All => "Changes here are for every computer.".to_owned(),
            Scope::Os(o) => format!("Changes here are only for {}.", tune::os_plural(*o)),
            Scope::Peer(p) => format!("Changes here are only for {p}."),
        };
        let scope_note = match &self.keys.app {
            Some(a) => format!("{}, {}.", scope_note.trim_end_matches('.'), tune::app_text(a)),
            None => scope_note,
        };
        y += para(ui, pos2(c.left(), y), c.width(), &scope_note, regular(T_META), INK2) + 10.0;
        let busy = self.tuning_busy();
        y = self.app_field(ui, pos2(c.left(), y), c.width(), &name);
        match (self.keys.capture, self.keys.picker.clone()) {
            (Some((want, _)), _) => y = self.capture_box(ui, c.left(), y, c.width(), &name, want),
            (_, Some((want, query))) => y = self.picker(ui, c.left(), y, c.width(), want, query, 7),
            _ => {
                let mut labels = vec!["Press the new key", "Choose from a list", "Does nothing"];
                if own.is_some() {
                    labels.push("Back to normal");
                }
                let (rects, bottom) = flow(ui, c.left(), y, c.width(), &labels);
                let names = [format!("Press the key {name} should become"), format!("Choose what {name} becomes from a list"), format!("Make {name} do nothing"), format!("Make {name} type itself again")];
                for (k, r) in rects.iter().enumerate() {
                    let look = if k == 0 { Look::Primary } else { Look::Quiet };
                    if button(ui, *r, Id::unique(("key-act", k)), labels[k], look, !busy, Some(&names[k])).clicked() {
                        match k {
                            0 => self.keys.capture = Some((Want::To, Capture::default())),
                            1 => self.keys.picker = Some((Want::To, String::new())),
                            2 => self.choose(Want::To, "none".into()),
                            _ => {
                                if let Some(r) = &own {
                                    self.tune_cmds(&name, "keyboard", vec![tune::remove_args(r)], None);
                                }
                            }
                        }
                    }
                }
                y = bottom + 12.0;
                // Rarely needed: a different key when held.
                let open = self.keys.more;
                let dr = Rect::from_min_size(pos2(c.left() - 4.0, y), vec2(c.width().min(260.0), 26.0));
                let resp = ui.interact(dr, Id::unique("key-more"), widgets::sense(ui, Sense::click()));
                let p = ui.painter();
                let m = pos2(dr.left() + 10.0, dr.center().y);
                let pts = if open { vec![m + vec2(-4.0, -2.0), m + vec2(0.0, 2.0), m + vec2(4.0, -2.0)] } else { vec![m + vec2(-2.0, -4.0), m + vec2(2.0, 0.0), m + vec2(-2.0, 4.0)] };
                p.add(egui::Shape::line(pts, Stroke::new(1.5, INK2)));
                p.text(pos2(dr.left() + 22.0, dr.center().y), Align2::LEFT_CENTER, "More: hold for another key", regular(T_BODY), if resp.hovered() { INK } else { INK2 });
                widgets::focus_ring(ui, &resp, dr, 6);
                let label = "More: a different key when held";
                resp.widget_info(move || egui::WidgetInfo::selected(Role::Button, true, open, label));
                if resp.clicked() {
                    self.keys.more = !open;
                }
                y += 32.0;
                if open {
                    let hold = own.as_ref().and_then(|r| r.hold.clone());
                    let hold_text = hold.as_ref().and_then(|h| h.parse().ok()).map_or("nothing else yet: it types the same".to_owned(), |o| tune::out_text(&o));
                    y += para(ui, pos2(c.left(), y), c.width(), &format!("Tapped, it types what is chosen above. Held, it gives: {hold_text}."), regular(T_META), INK2) + 8.0;
                    let mut labels = vec!["Press the key for holding", "Choose"];
                    if hold.is_some() {
                        labels.push("Not when held");
                    }
                    let (rects, bottom) = flow(ui, c.left(), y, c.width(), &labels);
                    for (k, r) in rects.iter().enumerate() {
                        let names = [format!("Press the key {name} gives when held"), format!("Choose what {name} gives when held"), format!("Make {name} the same when held")];
                        if button(ui, *r, Id::unique(("key-hold", k)), labels[k], Look::Quiet, !busy, Some(&names[k])).clicked() {
                            match k {
                                0 => self.keys.capture = Some((Want::Hold, Capture::default())),
                                1 => self.keys.picker = Some((Want::Hold, String::new())),
                                _ => {
                                    if let Some(r) = own.clone() {
                                        let tap = r.tap.clone().unwrap_or_else(|| r.from.clone());
                                        self.save_rule(KeyRule { to: Some(tune::To::One(tap)), tap: None, hold: None, timeout_ms: None, ..r });
                                    }
                                }
                            }
                        }
                    }
                    y = bottom + 8.0;
                    if let Some(r) = own.clone().filter(|r| r.hold.is_some()) {
                        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
                        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, "Held after", regular(T_BODY), INK);
                        let fr = Rect::from_min_size(pos2(c.left() + text_width(ui, "Held after", regular(T_BODY)) + 12.0, row.center().y - 13.0), vec2(64.0, 26.0));
                        ui.painter().text(pos2(fr.right() + 8.0, row.center().y), Align2::LEFT_CENTER, "ms", regular(T_META), INK2);
                        let cur = r.timeout_ms.unwrap_or(input.tap_ms.unwrap_or(200)).to_string();
                        if let Some(text) = self.edit_field(ui, fr, "key-timeout", &cur, "200", "Held after this many milliseconds", !busy, true) {
                            match text.parse::<u32>() {
                                Ok(ms) if (one_mk_proto::remap::MIN_TAP_MS..=one_mk_proto::remap::MAX_TAP_MS).contains(&ms) => {
                                    self.save_rule(KeyRule { timeout_ms: Some(ms), ..r });
                                }
                                _ => self.keys.error = Some(("keyboard", format!("Type a time from {} to {} ms.", one_mk_proto::remap::MIN_TAP_MS, one_mk_proto::remap::MAX_TAP_MS))),
                            }
                        }
                        y += ROW;
                    }
                }
            }
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "keyboard") {
            y += 6.0;
            y += error_line(ui, c.left(), y, c.width(), &e);
        }
        card.end(ui, y)
    }

    /// "Only in app…": the rule being changed applies only while this app
    /// has the focus on the computer typed on. Changing it moves the rule
    /// there; with no rule yet, the next change is made for that app.
    fn app_field(&mut self, ui: &mut Ui, at: Pos2, w: f32, name: &str) -> f32 {
        let (left, y) = (at.x, at.y);
        let own = self.keys.selected.and_then(|code| self.own_rule(code)).map(|(_, r)| r);
        let own = own.as_ref();
        let busy = self.tuning_busy();
        let label = "Only in app";
        let row = Rect::from_min_size(pos2(left, y), vec2(w, ROW));
        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), INK);
        let x = left + text_width(ui, label, regular(T_BODY)) + 12.0;
        let fr = Rect::from_min_size(pos2(x, row.center().y - 13.0), vec2((row.right() - x).min(240.0), 26.0));
        let cur = self.keys.app.clone().unwrap_or_default();
        let help = "Only in app: the program's name on the other computer, like chrome.exe on Windows, com.apple.Terminal on a Mac or firefox on Linux. Empty for every app.";
        if let Some(text) = self.edit_field(ui, fr, "key-app", &cur, "Any app", help, !busy, true) {
            let new = (!text.is_empty()).then_some(text);
            if let Some(Err(e)) = new.as_ref().map(|a| a.parse::<one_mk_proto::appmatch::AppPattern>()) {
                self.keys.error = Some(("keyboard", format!("Only in app: {}", e.trim_start_matches("app: "))));
                return y + ROW;
            }
            if let Some(r) = own.filter(|r| !tune::same_app(&r.app, &new)) {
                let moved = KeyRule { app: new.clone(), ..r.clone() };
                if let Err(e) = tune::check_add(&self.prefs.input, &moved) {
                    self.keys.error = Some(("keyboard", e));
                    return y + ROW;
                }
                self.tune_cmds(name, "keyboard", vec![tune::remove_args(r), tune::add_args(&moved)], None);
            }
            self.keys.app = new;
        }
        y + ROW + 4.0
    }

    /// "Press the key you want…", while the window waits for it.
    fn capture_box(&mut self, ui: &mut Ui, left: f32, y: f32, w: f32, name: &str, want: Want) -> f32 {
        let ask = match want {
            Want::To => format!("Press the key {name} should become…"),
            Want::Hold => format!("Press the key {name} gives when held…"),
            Want::Button => "Press the key or shortcut it should give…".to_owned(),
        };
        let note = "Esc cancels. Shortcuts work too. The keypad, Fn and a few others can't be pressed here: choose them from the list.";
        let bw = button_width(ui, "Cancel") - 6.0;
        let tw = w - bw - 38.0;
        // Measure first: the box is as tall as what it says.
        let lay = |text: &str, font: egui::FontId| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), INK, tw);
            job.sections[0].format.line_height = Some((font.size * LEADING).round());
            ui.painter().layout_job(job).size().y
        };
        let (ah, nh) = (lay(&ask, semibold(T_BODY)), lay(note, regular(T_META)));
        let r = Rect::from_min_size(pos2(left, y), vec2(w, (12.0 + ah + 4.0 + nh + 12.0).max(52.0)));
        let id = Id::unique(CAPTURE_ID);
        let resp = ui.interact(r, id, Sense::focusable_noninteractive());
        ui.painter().rect(r, CornerRadius::same(9), GROUND, Stroke::new(1.5, SIGNAL), StrokeKind::Inside);
        para(ui, pos2(left + 14.0, y + 12.0), tw, &ask, semibold(T_BODY), INK);
        para(ui, pos2(left + 14.0, y + 16.0 + ah), tw, note, regular(T_META), INK2);
        let full = format!("{ask} Esc cancels. Note: {}.", tune::NOT_REPORTED);
        widgets::a11y(&resp, Role::Label, &full, None);
        let resp = ui.interact(r.shrink2(vec2(bw + 20.0, 0.0)).translate(vec2(-(bw + 20.0) / 2.0, 0.0)), id.with("tip"), Sense::hover());
        resp.on_hover_text(format!("Note: {}.", tune::NOT_REPORTED));
        let br = Rect::from_min_size(pos2(r.right() - 10.0 - bw, y + 10.0), vec2(bw, 28.0));
        if button(ui, br, Id::unique("capture-cancel"), "Cancel", Look::Quiet, true, Some("Cancel: don't change anything")).clicked() {
            self.keys.capture = None;
        }
        r.bottom() + 8.0
    }

    /// The searchable list of what a key or button can become.
    #[allow(clippy::too_many_arguments)]
    fn picker(&mut self, ui: &mut Ui, left: f32, y: f32, w: f32, want: Want, query: String, rows: usize) -> f32 {
        let buttons = want == Want::Button;
        if self.keys.choices.as_ref().is_none_or(|(b, _)| *b != buttons) {
            self.keys.choices = Some((buttons, tune::choices(buttons)));
        }
        let bw = button_width(ui, "Cancel") - 6.0;
        let fr = Rect::from_min_size(pos2(left, y), vec2(w - bw - 8.0, 30.0));
        let fid = Id::unique(("field", "key-search"));
        let first = !ui.memory(|m| m.has_focus(fid)) && query.is_empty() && !ui.ctx().data(|d| d.get_temp::<bool>(fid.with("opened")).unwrap_or(false));
        if first {
            ui.memory_mut(|m| m.request_focus(fid));
            ui.ctx().data_mut(|d| d.insert_temp(fid.with("opened"), true));
        }
        let mut q = query;
        let resp = widgets::field(ui, fr, fid, &mut q, "Search: esc, volume, next computer…", "Search for a key or an action", true, false);
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if button(ui, Rect::from_min_size(pos2(fr.right() + 8.0, y), vec2(bw, 30.0)), Id::unique("pick-cancel"), "Cancel", Look::Quiet, true, Some("Close the list")).clicked()
            || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)))
        {
            self.keys.picker = None;
            ui.ctx().data_mut(|d| d.remove::<bool>(fid.with("opened")));
            return y + 38.0;
        }
        let all = &self.keys.choices.as_ref().expect("filled above").1;
        let hits: Vec<tune::Choice> = tune::search(all, &q).into_iter().cloned().collect();
        self.keys.picker = Some((want, q));
        let mut picked = enter.then(|| hits.first().map(|c| c.value.clone())).flatten();
        let rows = hits.len().clamp(3, rows);
        let list = Rect::from_min_size(pos2(left, y + 38.0), vec2(w, 30.0 * rows as f32 + 6.0));
        ui.painter().rect(list, CornerRadius::same(8), GROUND, Stroke::new(1.0, LINE), StrokeKind::Inside);
        if hits.is_empty() {
            ui.painter().text(list.center(), Align2::CENTER_CENTER, "Nothing is called that. Try another word.", regular(T_BODY), INK2);
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(list.shrink(3.0)));
        child.set_clip_rect(list.shrink(2.0));
        egui::ScrollArea::vertical().id_salt(("pick-list", buttons)).auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (k, c) in hits.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), widgets::sense(ui, Sense::click()));
                let hover = tween(ui.ctx(), resp.id.with("hover"), f32::from(resp.hovered() || resp.has_focus()), 0.1, motion::out_cubic);
                let p = ui.painter();
                p.rect_filled(r, CornerRadius::same(6), mix(Color32::TRANSPARENT, PILL, hover));
                widgets::focus_ring(ui, &resp, r.shrink(2.0), 5);
                let vw = text_width(ui, &c.value, mono(T_META)).min(r.width() * 0.4);
                p.text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, fit(p, &c.label, &regular(T_BODY), r.width() - vw - 30.0), regular(T_BODY), INK);
                widgets::mono_text(p, pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, &c.value, T_META, INK2, vw);
                widgets::a11y(&resp, Role::ListItem, &c.label, None);
                if resp.clicked() {
                    picked = Some(c.value.clone());
                }
                let _ = k;
            }
        });
        if let Some(v) = picked {
            ui.ctx().data_mut(|d| d.remove::<bool>(fid.with("opened")));
            self.choose(want, v);
        }
        list.bottom() + 8.0
    }

    /// "Test it": keys typed here go through the rules as a visit would
    /// send them; nothing is sent and no action runs.
    fn test_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, now: f64) -> f32 {
        let card = card_begin(ui, at, w, "Test it");
        let c = card.inner;
        let has = self.keys.tester.as_ref().is_some_and(|(t, ..)| !t.pressed.chips.is_empty() || t.note.is_some());
        let clear = Rect::from_min_size(pos2(at.x + w - 12.0 - 28.0, at.y + 12.0), vec2(28.0, 28.0));
        if has && widgets::icon_button(ui, clear, Id::unique("test-clear"), "Clear the test box", true, draw_cross).clicked()
            && let Some((t, ..)) = &mut self.keys.tester
        {
            t.clear();
        }
        let mut y = c.top() - 6.0;
        let who = match &self.keys.scope {
            Scope::All => "every computer".to_owned(),
            Scope::Os(o) => tune::os_plural(*o).to_owned(),
            Scope::Peer(p) => p.clone(),
        };
        let intro = format!("Type in the box to see what {who} would get. Nothing is sent.");
        y += para(ui, pos2(c.left(), y), c.width(), &intro, regular(T_BODY), INK2) + 10.0;
        let id = Id::unique(TEST_ID);
        let r = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 78.0));
        let resp = ui.interact(r, id, widgets::sense(ui, Sense::click()));
        if resp.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let focused = resp.has_focus();
        let _ = self.tester();
        let p = ui.painter().clone();
        p.rect(r, CornerRadius::same(9), GROUND, Stroke::new(1.0, if focused { INK2 } else if resp.hovered() { TRACK_EDGE } else { KEY_EDGE }), StrokeKind::Inside);
        if focused {
            p.rect_stroke(r.expand(2.0), CornerRadius::same(11), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
        }
        let lw = text_width(ui, "They get", regular(T_META)).max(text_width(ui, "You press", regular(T_META))) + 14.0;
        let empty = self.keys.tester.as_ref().is_none_or(|(t, ..)| t.pressed.chips.is_empty());
        let mut spoken = String::new();
        if let Some(e) = &self.keys.tester_error {
            para(ui, pos2(r.left() + 12.0, r.top() + 10.0), r.width() - 24.0, &format!("These rules can't be tried: {e}"), regular(T_META), INK);
        } else if empty {
            let hint = if focused { "Type now: try the keys you changed. Tab leaves." } else { "Click here, then type" };
            p.text(r.center(), Align2::CENTER_CENTER, hint, regular(T_BODY), INK2);
        } else if let Some((t, ..)) = &self.keys.tester {
            for (k, (label, log)) in [("You press", &t.pressed), ("They get", &t.sent)].into_iter().enumerate() {
                let ly = r.top() + 22.0 + 34.0 * k as f32;
                p.text(pos2(r.left() + 12.0, ly), Align2::LEFT_CENTER, label, regular(T_META), INK2);
                // Newest last; the oldest drop off the left when there's no room.
                let mut x = r.right() - 10.0;
                let mut shown = vec![];
                for chip in log.chips.iter().rev() {
                    let text = match &chip.note {
                        Some(n) => format!("{} · {n}", chip.text),
                        None => chip.text.clone(),
                    };
                    let cw = text_width(ui, &text, mono(T_META)) + 14.0;
                    if x - cw < r.left() + 12.0 + lw {
                        break;
                    }
                    x -= cw;
                    shown.push((Rect::from_min_size(pos2(x, ly - 11.0), vec2(cw, 22.0)), text));
                    x -= 6.0;
                }
                let dx = shown.last().map_or(0.0, |(rr, _)| rr.left() - (r.left() + 12.0 + lw));
                for (rr, text) in shown.iter().rev() {
                    let rr = rr.translate(vec2(-dx, 0.0));
                    p.rect(rr, CornerRadius::same(5), BUTTON, Stroke::new(1.0, KEY_EDGE), StrokeKind::Inside);
                    widgets::mono_text(&p, rr.center(), Align2::CENTER_CENTER, text, T_META, INK, rr.width());
                }
                spoken += &format!("{label}: {}. ", log.chips.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(", "));
            }
        }
        let name = "Test it: type here to see what the other computer would get";
        widgets::a11y(&resp, Role::TextInput, name, None);
        a11y_text(ui, Id::unique("keys-test-out"), r, Role::Status, &spoken, Some(Live::Polite));
        y = r.bottom() + 10.0;
        let i_now = Instant::now();
        let (note, progress) = match &self.keys.tester {
            Some((t, ..)) => (t.note.clone(), t.progress(i_now)),
            None => (None, None),
        };
        if let Some(f) = progress {
            widgets::progress_bar(ui, Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), 3.0)), Id::unique("test-progress"), Some(f));
            y += 9.0;
        }
        // How a tap/hold key was decided, once there is one.
        if let Some(note) = note {
            y += para(ui, pos2(c.left(), y), c.width(), &note, regular(T_META), INK2) + 10.0;
        }
        ui.painter().hline(c.x_range(), y, Stroke::new(1.0, LINE));
        y += 10.0;
        let safe = format!("`{}` always brings the mouse and keyboard back here. No rule can change it.", crate::view::chord_label("ctrl+alt+shift+esc", mac()));
        y += widgets::para_code(ui, pos2(c.left(), y), c.width(), &safe, regular(T_META), INK2);
        a11y_text(ui, Id::unique("keys-safe"), Rect::from_min_size(pos2(c.left(), y - 30.0), vec2(c.width(), 30.0)), Role::Label, &safe.replace('`', ""), None);
        let _ = now;
        card.end(ui, y)
    }

    // ---------- mouse ----------

    fn mouse_view(&mut self, ui: &mut Ui, w: f32) {
        let input = self.prefs.input.clone();
        let facts = self.facts();
        let two = w >= 700.0;
        let col_w = if two { (w - GAP) / 2.0 } else { w };
        let h = |k: &str| self.keys.heights.get(k).copied().unwrap_or(300.0);
        let total = if two { h("buttons").max(h("pointer") + GAP + h("scroll")) } else { h("buttons") + h("pointer") + h("scroll") + 2.0 * GAP };
        let (all, _) = ui.allocate_exact_size(vec2(w, total), Sense::hover());
        let bh = self.buttons_card(ui, all.min, col_w, &input, &facts);
        let right = if two { pos2(all.left() + col_w + GAP, all.top()) } else { pos2(all.left(), all.top() + bh + GAP) };
        let ph = self.pointer_card(ui, right, col_w, &input, &facts);
        let sh = self.scroll_card(ui, right + vec2(0.0, ph + GAP), col_w, &input, &facts);
        for (k, v) in [("buttons", bh), ("pointer", ph), ("scroll", sh)] {
            if (self.keys.heights.get(k).copied().unwrap_or(0.0) - v).abs() > 0.5 {
                self.keys.heights.insert(k, v);
                ui.ctx().request_repaint();
            }
        }
    }

    fn buttons_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let card = card_begin(ui, at, w, "Buttons");
        let c = card.inner;
        let mut y = c.top() - 6.0;
        let busy = self.tuning_busy();
        // The one quick setup for buttons.
        let side = "Side buttons go back and forward";
        let on = [(4u32, "back"), (5, "forward")].iter().all(|(n, to)| {
            input.buttons.iter().any(|b| b.scope == self.keys.scope && b.from.trim() == n.to_string() && b.to == tune::To::One((*to).into()))
        });
        let cr = Rect::from_min_size(pos2(c.left(), y), vec2(chip_width(ui, side).min(c.width()), 30.0));
        if chip(ui, cr, Id::unique("preset-side"), side, on, !busy).clicked() {
            let cmds = if on {
                vec![tune::button_args(4, None, &self.keys.scope), tune::button_args(5, None, &self.keys.scope)]
            } else {
                let mut a = vec!["buttons".to_owned(), "preset".into()];
                a.extend(self.keys.scope.args());
                a.extend(["--".into(), "side-back-forward".into()]);
                vec![a]
            };
            self.tune_cmds("the side buttons", "buttons", cmds, None);
        }
        y += 44.0;
        // The mouse, and what each button does.
        let shown = tune::buttons_shown(input, facts);
        let m = Rect::from_min_size(pos2(c.left() + 4.0, y), vec2(104.0, 168.0));
        let rows_x = m.right() + 22.0;
        let mut pick = None;
        let parts = mouse_parts(m);
        let p = ui.painter().clone();
        let body = Rect::from_min_max(m.min, m.max);
        p.rect(body, CornerRadius { nw: 50, ne: 50, sw: 44, se: 44 }, BUTTON, Stroke::new(1.0, KEY_EDGE), StrokeKind::Inside);
        for (n, r, _) in &parts {
            let id = Id::unique(("mouse-part", *n));
            let resp = ui.interact(*r, id, widgets::sense(ui, Sense::CLICK));
            let changed = shown.iter().any(|(k, _, ch)| k == n && *ch);
            let sel = self.keys.button == Some(*n);
            let hover = f32::from(resp.hovered());
            let fill = if changed { BUTTON_LIT } else { BUTTON };
            match n {
                1 | 2 => {
                    let rad = if *n == 1 { CornerRadius { nw: 50, ne: 0, sw: 0, se: 0 } } else { CornerRadius { nw: 0, ne: 50, sw: 0, se: 0 } };
                    p.rect(*r, rad, mix(fill, PILL, hover * 0.6), Stroke::new(1.0, KEY_EDGE), StrokeKind::Inside);
                    if sel {
                        p.rect_stroke(*r, rad, Stroke::new(2.0, SIGNAL), StrokeKind::Inside);
                    }
                }
                _ => {
                    p.rect(*r, CornerRadius::same(5), mix(if changed { INK2 } else { TRACK_OFF }, INK, hover * 0.3), Stroke::new(1.0, KEY_EDGE), StrokeKind::Inside);
                    if sel {
                        p.rect_stroke(r.expand(2.0), CornerRadius::same(6), Stroke::new(2.0, SIGNAL), StrokeKind::Outside);
                    }
                }
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                pick = Some(*n);
            }
        }
        // The rows say the same, and take the keyboard.
        for (k, (n, does, changed)) in shown.iter().enumerate() {
            let row = Rect::from_min_size(pos2(rows_x, y + k as f32 * 34.0), vec2(c.right() - rows_x, 32.0));
            let id = Id::unique(("mouse-row", *n));
            let resp = ui.interact(row, id, widgets::sense(ui, Sense::click()));
            let sel = self.keys.button == Some(*n);
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            p.rect_filled(row, CornerRadius::same(6), mix(if sel { PILL } else { Color32::TRANSPARENT }, PILL, hover));
            widgets::focus_ring(ui, &resp, row, 6);
            let name = tune::BUTTONS[k].1;
            let nw = text_width(ui, name, regular(T_META)) + 10.0;
            p.text(pos2(row.left() + 8.0, row.center().y), Align2::LEFT_CENTER, name, regular(T_META), INK2);
            let room = row.width() - nw - 16.0;
            p.text(pos2(row.right() - 8.0, row.center().y), Align2::RIGHT_CENTER, fit(&p, does, &regular(T_BODY), room), if *changed { semibold(T_BODY) } else { regular(T_BODY) }, INK);
            widgets::a11y(&resp, Role::Button, &format!("{name}: {does}"), Some(sel));
            if resp.clicked() {
                pick = Some(*n);
            }
        }
        if let Some(n) = pick {
            self.keys.button = if self.keys.button == Some(n) { None } else { Some(n) };
            self.keys.reveal = self.keys.button.is_some();
            self.keys.capture = None;
            self.keys.picker = None;
        }
        y = (m.bottom()).max(y + 5.0 * 34.0) + 14.0;
        if let Some(n) = self.keys.button {
            let name = tune::BUTTONS.iter().find(|b| b.0 == n).map_or("Button", |b| b.1);
            let changed = shown.iter().any(|(k, _, ch)| *k == n && *ch);
            ui.painter().hline(c.x_range(), y - 6.0, Stroke::new(1.0, LINE));
            ui.painter().text(pos2(c.left(), y + 12.0), Align2::LEFT_CENTER, format!("{name}: what should it do?"), semibold(T_BODY), INK);
            if std::mem::take(&mut self.keys.reveal) {
                ui.scroll_to_rect(Rect::from_min_size(pos2(c.left(), y - 8.0), vec2(c.width(), 90.0)), None);
            }
            y += 30.0;
            match (self.keys.capture, self.keys.picker.clone()) {
                (Some((Want::Button, _)), _) => y = self.capture_box(ui, c.left(), y, c.width(), name, Want::Button),
                (_, Some((Want::Button, q))) => y = self.picker(ui, c.left(), y, c.width(), Want::Button, q, 6),
                _ => {
                    let mut labels = vec!["Press a key or shortcut", "Choose from a list"];
                    if changed {
                        labels.push("As it is");
                    }
                    let (rects, bottom) = flow(ui, c.left(), y, c.width(), &labels);
                    for (k, r) in rects.iter().enumerate() {
                        let look = if k == 0 { Look::Primary } else { Look::Quiet };
                        let full = [format!("{name}: press the key or shortcut it gives"), format!("{name}: choose what it does from a list"), format!("{name}: do what it does without one·mk")];
                        if button(ui, *r, Id::unique(("button-act", k)), labels[k], look, !busy, Some(&full[k])).clicked() {
                            match k {
                                0 => self.keys.capture = Some((Want::Button, Capture::default())),
                                1 => self.keys.picker = Some((Want::Button, String::new())),
                                _ => self.tune_cmds(&name.to_lowercase(), "buttons", vec![tune::button_args(n, None, &self.keys.scope)], None),
                            }
                        }
                    }
                    y = bottom + 10.0;
                    // Rare: run a command.
                    let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
                    ui.painter().text(row.left_center(), Align2::LEFT_CENTER, "Or run a command", regular(T_BODY), INK2);
                    let lw = text_width(ui, "Or run a command", regular(T_BODY)) + 12.0;
                    let fr = Rect::from_min_max(pos2(row.left() + lw, row.center().y - 14.0), pos2(row.right(), row.center().y + 14.0));
                    if let Some(cmd) = self.edit_field(ui, fr, "button-run", "", "Type a command, then Enter", &format!("{name}: a command to run when it's pressed (Enter saves)"), !busy, true)
                        && !cmd.is_empty()
                    {
                        self.choose(Want::Button, format!("run: {cmd}"));
                    }
                    y += ROW;
                }
            }
        } else {
            y += para(ui, pos2(c.left(), y), c.width(), "Click a button on the mouse, or in the list, to change what it does.", regular(T_BODY), INK2);
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "buttons") {
            y += 6.0 + error_line(ui, c.left(), y + 6.0, c.width(), &e);
        }
        card.end(ui, y)
    }

    /// A speed: a slider (even by ratio, 1× in the middle) and the number.
    fn speed_row(&mut self, ui: &mut Ui, row: Rect, key: &'static str, label: &str, value: f64) -> Option<f64> {
        let lw = text_width(ui, label, regular(T_BODY)) + 16.0;
        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), INK);
        let shown = self.keys.drafts.get(key).copied().unwrap_or(value);
        let fw = 58.0;
        let sr = Rect::from_min_max(pos2(row.left() + lw, row.top()), pos2(row.right() - fw - 26.0, row.bottom()));
        let (t, apply) = slider(ui, sr, Id::unique(("slider", key)), tune::speed_to_t(shown), label, shown);
        let mut out = None;
        if let Some(t) = t {
            let x = tune::t_to_speed(t);
            self.keys.drafts.insert(key, x);
            if apply {
                out = Some(x);
            }
        }
        let fr = Rect::from_min_size(pos2(row.right() - fw - 16.0, row.center().y - 13.0), vec2(fw, 26.0));
        ui.painter().text(pos2(row.right(), row.center().y), Align2::RIGHT_CENTER, "×", regular(T_BODY), INK2);
        let cur = tune::speed_text(shown).trim_end_matches('×').to_owned();
        if let Some(text) = self.edit_field(ui, fr, key, &cur, "1", &format!("{label}, from 0.1 to 10 times"), true, true) {
            match tune::parse_speed(&text) {
                Ok(x) => {
                    self.keys.drafts.insert(key, x);
                    out = Some(x);
                }
                Err(e) => self.keys.error = Some((if key.starts_with("pointer") { "pointer" } else { "scroll" }, e)),
            }
        }
        out
    }

    fn pointer_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let card = card_begin(ui, at, w, "Pointer");
        let c = card.inner;
        let mut y = c.top() - 6.0;
        let (speed, accel) = tune::pointer_shown(input, facts);
        if let Some(x) = self.speed_row(ui, Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW)), "pointer-speed", "Speed", speed) {
            self.tune_cmds("the pointer speed", "pointer", vec![tune::pointer_args(Some(x), None, &self.keys.scope)], Some("pointer-speed".into()));
        }
        y += ROW + 6.0;
        let row = Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        let label = "Acceleration";
        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), INK);
        let lw = text_width(ui, label, regular(T_BODY)) + 16.0;
        let track = Rect::from_min_max(pos2(row.left() + lw, row.center().y - 14.0), pos2(row.right(), row.center().y + 14.0));
        let labels = ["Like here", "Off", "Curve"];
        let values = ["system", "off", "curve"];
        let can = tune::RAW_MOTION;
        let pills = Pills {
            id: Id::unique("pointer-accel"),
            track,
            cells: Pills::even(track, 3, 2.0),
            labels: &labels,
            names: vec!["Acceleration: as this computer does it".into(), "Acceleration off (not available here)".into(), "Acceleration curve (not available here)".into()],
            chosen: values.iter().position(|v| *v == accel),
            enabled: vec![true, can, can],
            role: Role::RadioButton,
            font: T_META,
            radius: 6,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.tune_cmds("the acceleration", "pointer", vec![tune::pointer_args(None, Some(values[k]), &self.keys.scope)], None);
        }
        y += ROW + 4.0;
        if !can {
            y += para(ui, pos2(c.left(), y), c.width(), tune::accel_unavailable(), regular(T_META), INK2);
        }
        if accel != "system" && !can {
            y += 6.0 + error_line(ui, c.left(), y + 6.0, c.width(), &format!("settings.toml asks for acceleration {accel}, which can't be done here: this computer's applies."));
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "pointer") {
            y += 6.0 + error_line(ui, c.left(), y + 6.0, c.width(), &e);
        }
        card.end(ui, y)
    }

    fn scroll_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let card = card_begin(ui, at, w, "Scrolling");
        let c = card.inner;
        let mut y = c.top() - 6.0;
        let dev = self.keys.device;
        let s = tune::scroll_shown(input, facts, dev);
        let row = |y: f32| Rect::from_min_size(pos2(c.left(), y), vec2(c.width(), ROW));
        let seg = |ui: &mut Ui, app: &App, r: Rect, id: &str, label: &str, labels: &[&str], names: Vec<String>, chosen: Option<usize>| -> Option<usize> {
            ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), INK);
            let lw = if label.is_empty() { 0.0 } else { text_width(ui, label, regular(T_BODY)) + 16.0 };
            let track = Rect::from_min_max(pos2(r.left() + lw, r.center().y - 14.0), pos2(r.right(), r.center().y + 14.0));
            let n = labels.len();
            Pills { id: Id::unique(id), track, cells: Pills::even(track, n, 2.0), labels, names, chosen, enabled: vec![true; n], role: Role::RadioButton, font: T_META, radius: 6 }.show(ui, app.arrowed)
        };
        let devs = ["Both", "Wheel", "Touchpad"];
        let chosen = Some(match dev {
            None => 0,
            Some(Device::Wheel) => 1,
            Some(Device::Touchpad) => 2,
        });
        let names = vec!["Scroll settings for: mouse wheels and touchpads".into(), "Scroll settings for: mouse wheels".into(), "Scroll settings for: touchpads".into()];
        if let Some(k) = seg(ui, self, row(y), "scroll-device", "Settings for", &devs, names, chosen) {
            self.keys.device = [None, Some(Device::Wheel), Some(Device::Touchpad)][k];
        }
        y += ROW + 4.0;
        let mut change = None;
        if switch(ui, row(y), Id::unique("scroll-rev-v"), "Reverse up and down", s.reverse_vertical, true).clicked() {
            change = Some(ScrollChange::ReverseVertical(!s.reverse_vertical));
        }
        y += ROW;
        if switch(ui, row(y), Id::unique("scroll-rev-h"), "Reverse left and right", s.reverse_horizontal, true).clicked() {
            change = Some(ScrollChange::ReverseHorizontal(!s.reverse_horizontal));
        }
        y += ROW;
        if let Some(x) = self.speed_row(ui, row(y), "scroll-speed", "Speed", s.speed) {
            change = Some(ScrollChange::Speed(x));
        }
        y += ROW + 4.0;
        let modes = ["As it comes", "Smooth", "Notched"];
        let mode_values = ["auto", "smooth", "discrete"];
        let names = vec!["Steps: as the device sends them".into(), "Steps: smooth (notches become pixels)".into(), "Steps: whole notches (lines)".into()];
        if let Some(k) = seg(ui, self, row(y), "scroll-mode", "Steps", &modes, names, mode_values.iter().position(|v| *v == s.mode)) {
            change = Some(ScrollChange::Mode(mode_values[k]));
        }
        y += ROW + 6.0;
        ui.painter().text(pos2(c.left(), y + 9.0), Align2::LEFT_CENTER, "Natural scrolling", regular(T_BODY), INK);
        y += 22.0;
        let nat = ["As here", "Match each", "Always", "Never"];
        let nat_values: [Option<&'static str>; 4] = [None, Some("auto"), Some("on"), Some("off")];
        let names = vec![
            "Natural scrolling: as on this computer".into(),
            "Natural scrolling: match the computer I'm on (natural on Macs, traditional on PCs)".into(),
            "Natural scrolling: always".into(),
            "Natural scrolling: never".into(),
        ];
        let at_nat = nat_values.iter().position(|v| *v == s.natural.as_deref());
        if let Some(k) = seg(ui, self, row(y), "scroll-natural", "", &nat, names, at_nat) {
            change = Some(ScrollChange::Natural(nat_values[k]));
        }
        y += ROW;
        let explain = match s.natural.as_deref() {
            Some("auto") => "Match the computer I'm on: natural on Macs, traditional on Windows and Linux.",
            Some("on") => "Content follows your fingers on every computer.",
            Some("off") => "Content moves against your fingers on every computer.",
            _ => "Scrolling feels on the other computers as it does here.",
        };
        y += para(ui, pos2(c.left(), y), c.width(), explain, regular(T_META), INK2) + 8.0;
        let alt = if mac() { "Option" } else { "Alt" };
        if switch(ui, row(y), Id::unique("scroll-shift"), "Hold Shift to scroll sideways", s.shift_sideways, true).clicked() {
            change = Some(ScrollChange::Modifier("shift", if s.shift_sideways { "unchanged" } else { "horizontal" }));
        }
        y += ROW;
        if switch(ui, row(y), Id::unique("scroll-alt"), &format!("Hold {alt} to scroll 5× faster"), s.alt_faster, true).clicked() {
            change = Some(ScrollChange::Modifier("alt", if s.alt_faster { "unchanged" } else { "x5" }));
        }
        y += ROW;
        if let Some(ch) = change {
            let key = matches!(ch, ScrollChange::Speed(_)).then(|| "scroll-speed".to_owned());
            let cmds = tune::scroll_commands(input, &self.keys.scope, dev, &ch);
            self.tune_cmds("scrolling", "scroll", cmds, key);
        }
        if let Some((_, e)) = self.keys.error.clone().filter(|(a, _)| *a == "scroll") {
            y += 6.0 + error_line(ui, c.left(), y + 6.0, c.width(), &e);
        }
        card.end(ui, y - 4.0)
    }
}

/// The drawn mouse's parts: button number, where, and what it is.
fn mouse_parts(m: Rect) -> Vec<(u32, Rect, &'static str)> {
    let split = m.top() + m.height() * 0.42;
    let mid = m.center().x;
    let wheel = Rect::from_center_size(pos2(mid, m.top() + m.height() * 0.2), vec2(14.0, 30.0));
    vec![
        (1, Rect::from_min_max(m.min, pos2(mid, split)), "left"),
        (2, Rect::from_min_max(pos2(mid, m.top()), pos2(m.right(), split)), "right"),
        (3, wheel, "wheel"),
        (5, Rect::from_min_size(pos2(m.left() - 8.0, m.top() + m.height() * 0.48), vec2(10.0, 22.0)), "forward"),
        (4, Rect::from_min_size(pos2(m.left() - 8.0, m.top() + m.height() * 0.48 + 26.0), vec2(10.0, 22.0)), "back"),
    ]
}
