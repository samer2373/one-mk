//! "Detect key" and "Detect button" (Keys & mouse): the person presses the
//! key or button they mean, and its editor opens, even for one the drawn
//! keyboard or mouse doesn't have. oneMK is asked (`one-mk input-devices
//! detect`) where it sees this computer's own input, and says which device
//! it came from where it can tell ("Only on ..."); where it can't (a Wayland
//! desktop while the pointer is here) or isn't running, the window's own
//! key and mouse events stand in, and the card says what that leaves out.

use super::*;
use crate::detect::{Detected, Outcome};

/// How long oneMK listens, in seconds.
const LISTEN_SECS: u32 = 60;

/// Listening for a press.
#[derive(Clone, Debug)]
pub(in crate::app) struct Listening {
    pub n: u64,
    /// For the keyboard (keys only) or the mouse (buttons, and keys a mouse sends).
    pub view: View,
    /// oneMK was asked.
    pub daemon: bool,
    /// The window's own key and mouse events count too.
    pub window: bool,
    /// What can't be detected here, and how to get it anyway.
    pub note: Option<String>,
    /// The window's own keys: the modifiers held.
    capture: Capture,
}

/// Mouse button `n`'s name in the panel: the drawn ones' names, then "Button 6"...
pub(in crate::app) fn button_name(n: u32) -> String {
    tune::BUTTONS.iter().find(|b| b.0 == n).map_or_else(|| format!("Button {n}"), |b| b.1.to_owned())
}

impl App {
    /// "Detect key" / "Detect button" pressed: listen.
    pub(in crate::app) fn start_detect(&mut self, view: View) {
        self.keys.capture = None;
        self.keys.picker = None;
        self.keys.detect_n += 1;
        let n = self.keys.detect_n;
        let status = self.running();
        let backend = status.map(|s| s.backend.as_str().to_owned());
        let away = status.is_some_and(|s| s.controlling.is_some());
        let online = status.is_some_and(|s| s.peers.iter().any(|p| p.online));
        let what = if view == View::Keyboard { "key" } else { "button or key" };
        let (daemon, window, note) = match backend.as_deref() {
            None => (false, true, Some(format!(
                "oneMK isn't running, so only what this window sees can be detected: the keypad, Fn, media keys and the \
                 mouse's extra buttons may not come through. Start oneMK for those, or choose the {what} from the list."))),
            Some(b) => match one_mk_proto::detect::detect_support(b) {
                Ok(_) => (true, false, None),
                Err(why) if why == one_mk_proto::detect::LOCAL_UNSEEN && !away => {
                    let how = if online {
                        "move the pointer onto another computer's screen, then press it there"
                    } else {
                        "connect another computer and move the pointer onto its screen, then press it there"
                    };
                    (true, true, Some(format!(
                        "Ordinary keys work here. On this Wayland desktop oneMK sees the others (media keys, the keypad, \
                         a mouse's extra keys) only while it controls another computer: {how}.")))
                }
                Err(_) if away => (true, false, None),
                Err(why) => (true, true, Some(format!("{}: only what this window sees can be detected.", sentence(why)))),
            },
        };
        // Screenshots of listening (`ONE_MK_APP_DETECT`) ask no oneMK.
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let daemon_asked = daemon && std::env::var_os("ONE_MK_APP_DETECT").is_none();
        #[cfg(not(any(debug_assertions, feature = "design-evidence")))]
        let daemon_asked = daemon;
        if daemon_asked {
            self.link.detect(n, view == View::Keyboard, LISTEN_SECS);
        }
        if self.keys.devices.is_none() {
            self.link.input_devices();
        }
        self.keys.listening = Some(Listening { n, view, daemon, window, note, capture: Capture::default() });
        self.keys.reveal = true;
        if self.keys.error.as_ref().is_some_and(|(a, _)| matches!(*a, "keyboard" | "buttons")) {
            self.keys.error = None;
        }
    }

    /// Esc, Cancel, or leaving the page or view: stop listening.
    pub(in crate::app) fn cancel_detect(&mut self) {
        if let Some(l) = self.keys.listening.take()
            && l.daemon
        {
            self.link.cancel_detect();
        }
    }

    /// oneMK answered detection `n`.
    pub(in crate::app) fn detect_done(&mut self, n: u64, outcome: Outcome) {
        let Some(l) = self.keys.listening.clone().filter(|l| l.n == n) else { return };
        let area = if l.view == View::Keyboard { "keyboard" } else { "buttons" };
        match outcome {
            // A left click in this window is a click on the window (Cancel, "Listening…", a tab),
            // not the button being looked for: it stops listening. (The left button is on the
            // drawn mouse anyway.)
            Outcome::Found(d) if d.button == Some(1) && self.pointer_in_window() => self.keys.listening = None,
            Outcome::Found(d) => {
                self.keys.listening = None;
                self.apply_detected(d);
            }
            Outcome::Cancelled => self.keys.listening = None,
            Outcome::Timeout(secs, note) => {
                self.keys.listening = None;
                let why = match note {
                    Some(n) if n == one_mk_proto::detect::LOCAL_UNSEEN => {
                        " On this desktop oneMK sees keys only while it controls another computer: move the pointer there first.".to_owned()
                    }
                    Some(n) => format!(" ({n})"),
                    None => " Keys a keyboard handles itself (often Fn) and vendor buttons that need the maker's software never reach oneMK."
                        .to_owned(),
                };
                self.keys.error = Some((area, format!("Nothing was pressed in {secs} seconds.{why} Choose it from the list instead.")));
            }
            // oneMK can't help; the window's own events still can, if they count.
            Outcome::Failed(why) if l.window => {
                if let Some(listening) = self.keys.listening.as_mut() {
                    listening.daemon = false;
                    listening.note = Some(why);
                }
            }
            Outcome::Failed(why) => {
                self.keys.listening = None;
                self.keys.error = Some((area, why));
            }
        }
    }

    /// Whether the pointer is over this window (where oneMK's left button is a click on it).
    fn pointer_in_window(&self) -> bool {
        self.ctx.input(|i| i.pointer.has_pointer() || i.pointer.latest_pos().is_some_and(|p| i.content_rect().contains(p)))
    }

    /// The window's own events while listening: Esc cancels; where they
    /// count, a key (with the modifiers held) or a click is the press.
    /// Keys are kept from the window's shortcuts and the test box meanwhile.
    pub(in crate::app) fn detect_input(&mut self, ui: &mut Ui) {
        let Some(mut l) = self.keys.listening.clone() else { return };
        let keys: Vec<Event> = ui.input_mut(|i| {
            let (taken, kept): (Vec<Event>, Vec<Event>) = std::mem::take(&mut i.events)
                .into_iter()
                .partition(|e| matches!(e, Event::Key { .. } | Event::Copy | Event::Cut | Event::Paste(_) | Event::Text(_)));
            i.events = kept;
            taken
        });
        for e in &keys {
            let Some((code, down, repeat)) = tune::event_key(e) else { continue };
            if repeat {
                continue;
            }
            if code == 1 && down && !l.capture.holding() {
                self.cancel_detect();
                return;
            }
            if !l.window {
                continue;
            }
            if let Some(chord) = l.capture.key(code, down) {
                let groups = (chord.mods | (chord.mods >> 4)) & 0xf;
                self.cancel_detect();
                self.apply_detected(Detected::key(u32::from(chord.key), groups));
                return;
            }
        }
        if l.window && l.view == View::Mouse {
            let clicks: Vec<(egui::PointerButton, Pos2)> = ui.input(|i| {
                i.events.iter().filter_map(|e| match e {
                    Event::PointerButton { button, pressed: true, pos, .. } => Some((*button, *pos)),
                    _ => None,
                }).collect()
            });
            // A left click here is a click on the window (it stops listening, as through oneMK).
            if let Some((b, _)) = clicks.into_iter().next() {
                self.cancel_detect();
                if b != egui::PointerButton::Primary {
                    self.apply_detected(Detected::button(crate::detect::pointer_button(b)));
                }
                return;
            }
        }
        if let Some(listening) = self.keys.listening.as_mut() {
            listening.capture = l.capture;
        }
    }

    /// A press was found: its editor opens, as if it had been clicked.
    pub(in crate::app) fn apply_detected(&mut self, d: Detected) {
        let on_mouse = self.keys.view == View::Mouse;
        let label = if d.groups != 0 { tune::trigger_text(&d.trigger()) } else { tune::key_label(d.code) };
        if let Some(why) = &d.unusable {
            let area = if d.is_button { "buttons" } else { "keyboard" };
            self.keys.error = Some((area, format!("{} can't be changed: {why}.", if d.is_button { "That mouse button".to_owned() } else { label })));
            return;
        }
        match d.button.filter(|_| d.is_button && d.groups == 0) {
            Some(n) => {
                self.keys.view = View::Mouse;
                self.keys.button = Some(n);
                self.keys.button_kind = None;
                self.keys.reveal = true;
                if n > 5 && !self.keys.other_buttons.contains(&n) {
                    self.keys.other_buttons.push(n);
                }
                if self.keys.error.as_ref().is_some_and(|(a, _)| *a == "buttons") {
                    self.keys.error = None;
                }
            }
            None => {
                // A key, or a shortcut a mouse's key sends: a key rule.
                self.keys.view = View::Keyboard;
                self.select_key(d.code as u16, None);
                self.keys.trigger_groups = d.groups;
            }
        }
        self.keys.detected = Some(d);
        self.keys.detected_on_mouse = on_mouse;
    }

    /// "Detect key" (on the stage's toolbar or the compact panel's) and
    /// "Detect button" (in the mouse's panel); while listening, it says so.
    pub(in crate::app) fn detect_button(&mut self, ui: &mut Ui, rect: Rect, view: View) {
        let on = self.keys.listening.as_ref().is_some_and(|l| l.view == view);
        let (label, full) = match (view, on) {
            (View::Keyboard, false) => ("Detect key", "Detect key: press a key on your keyboard to change it, even one not drawn here"),
            (View::Mouse, false) => ("Detect button", "Detect button: press a button on your mouse to change it, even one not drawn here"),
            (_, true) => ("Listening…", "Listening for a press. Activate to stop"),
        };
        let response = small_button(ui, rect, Id::unique(("detect", view == View::Keyboard)), label, Some(full));
        if on {
            // The accent's ring while listening.
            ui.painter().rect_stroke(rect, CornerRadius::same(widgets::button_radius(rect.height())), Stroke::new(1.5, pal().accent), StrokeKind::Inside);
        }
        if response.clicked() {
            if on { self.cancel_detect() } else { self.start_detect(view) }
        }
    }

    /// The card that says a press is awaited, and on which devices. Its bottom.
    pub(in crate::app) fn listening_card(&mut self, ui: &mut Ui, at: Pos2, w: f32, now: f64) -> f32 {
        let Some(l) = self.keys.listening.clone() else { return at.y };
        let keyboard = l.view == View::Keyboard;
        let ask = if keyboard { "Press the key on your keyboard…" } else { "Press the button on your mouse…" };
        let heard = self.listened_to(keyboard, l.daemon && !l.window);
        let esc = if keyboard { "Esc cancels." } else { "Esc or a left click here cancels." };
        let bw = button_width(ui, "Cancel") - 6.0;
        let tw = w - bw - 52.0;
        let lay = |text: &str, font: egui::FontId, width: f32| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), pal().ink, width);
            job.sections[0].format.line_height = Some((font.size * LEADING).round());
            ui.painter().layout_job(job).size().y
        };
        let (ah, hh) = (lay(ask, semibold(T_BODY), tw), lay(&format!("{heard} {esc}"), regular(T_META), tw));
        let nh = l.note.as_deref().map_or(0.0, |n| lay(n, regular(T_META), w - 28.0) + 10.0);
        let r = Rect::from_min_size(at, vec2(w, 12.0 + ah + 4.0 + hh + nh + 14.0));
        if std::mem::take(&mut self.keys.reveal) || !ui.clip_rect().contains_rect(r.expand(4.0)) {
            ui.scroll_to_rect(r.expand(6.0), None);
        }
        let p = ui.painter();
        p.rect(r, CornerRadius::same(10), pal().bg, Stroke::new(1.5, pal().accent), StrokeKind::Inside);
        // A dot that breathes while it listens (still, with less motion).
        let pulse = if motion::reduced() { 1.0 } else { 0.55 + 0.45 * ((now * 3.0).sin() as f32 * 0.5 + 0.5) };
        let dot = pos2(r.left() + 20.0, r.top() + 12.0 + ah.min(22.0) / 2.0);
        p.circle_filled(dot, 7.0, pal().accent.gamma_multiply(0.25 * pulse));
        p.circle_filled(dot, 4.0, pal().accent.gamma_multiply(pulse));
        if !motion::reduced() {
            ui.ctx().request_repaint_after_secs(1.0 / 30.0);
        }
        let left = r.left() + 36.0;
        para(ui, pos2(left, r.top() + 12.0), tw, ask, semibold(T_BODY), pal().ink);
        para(ui, pos2(left, r.top() + 16.0 + ah), tw, &format!("{heard} {esc}"), regular(T_META), pal().ink2);
        if let Some(note) = &l.note {
            let y = r.top() + 16.0 + ah + hh + 10.0;
            widgets::hairline(ui.painter(), egui::Rangef::new(r.left() + 14.0, r.right() - 14.0), y - 5.0);
            para(ui, pos2(r.left() + 14.0, y), w - 28.0, note, regular(T_META), pal().ink2);
        }
        let full = [ask, heard.as_str(), esc, l.note.as_deref().unwrap_or("")].join(" ");
        a11y_text(ui, Id::unique("detect-listening"), r, Role::Alert, full.trim(), Some(Live::Polite));
        let br = Rect::from_min_size(pos2(r.right() - 10.0 - bw, r.top() + 10.0), vec2(bw, 28.0));
        if button(ui, br, Id::unique("detect-cancel"), "Cancel", Look::Quiet, true, Some("Stop listening")).clicked() {
            self.cancel_detect();
        }
        r.bottom()
    }

    /// "Listening to every keyboard on this computer", or which ones.
    fn listened_to(&self, keyboard: bool, daemon: bool) -> String {
        let kind = if keyboard { "keyboard" } else { "mouse" };
        if !daemon {
            return format!("Listening in this window, to every {kind}.");
        }
        let names: Vec<String> = match &self.keys.devices {
            Some(Ok(d)) if d.available => d
                .devices
                .iter()
                .filter(|d| d.connected && !d.is_virtual() && if keyboard { d.keyboard } else { d.pointer })
                .map(|d| d.name.clone())
                .collect(),
            _ => vec![],
        };
        match names.as_slice() {
            [] => format!("Listening to every {kind} on this computer."),
            [one] => format!("Listening to {one}."),
            [one, two] => format!("Listening to {one} and {two}."),
            [one, rest @ ..] => format!("Listening to {one} and {} others.", rest.len()),
        }
    }

    /// After a press was detected on a device this computer can name:
    /// whether the change is for that device only. Its bottom (`at.y` when
    /// there is nothing to offer).
    pub(in crate::app) fn device_offer(&mut self, ui: &mut Ui, at: Pos2, w: f32, mouse: bool) -> f32 {
        let Some(d) = self.keys.detected.clone() else { return at.y };
        let Some((fp, name)) = d.device.clone() else { return at.y };
        let this = if mouse {
            self.keys.button.is_some_and(|n| d.button == Some(n) && d.is_button)
        } else {
            self.keys.selected == Some(d.code as u16) && self.keys.trigger_groups == d.groups && self.keys.layer.is_none()
        };
        if !this {
            return at.y;
        }
        let (on, inner) = match &self.keys.scope {
            Scope::Device(f, inner) => (*f == fp, (**inner).clone()),
            other => (false, other.clone()),
        };
        let all = if mouse {
            "Every mouse"
        } else if self.keys.detected_on_mouse {
            "Every keyboard and mouse"
        } else {
            "Every keyboard"
        };
        let all = all.to_owned();
        let only = format!("Only on {name}");
        ui.painter().text(pos2(at.x, at.y + 9.0), Align2::LEFT_CENTER, "Pressed on", regular(T_META), pal().ink2);
        let room = w - 4.0;
        let widths = [chip_width(ui, &all, !on).min(room), chip_width(ui, &only, on).min(room)];
        let (rects, bottom) = quick_setup_rects(room, 0.0, &widths);
        let busy = self.tuning_busy();
        let top = at.y + 22.0;
        for (k, rect) in rects.into_iter().enumerate() {
            let rect = rect.translate(vec2(at.x, top - 38.0));
            let (label, chosen) = if k == 0 { (&all, !on) } else { (&only, on) };
            let response = chip(ui, rect, Id::unique(("detect-device", k)), label, chosen, !busy, true);
            let help = if k == 0 { "Changes made now are for every device" } else { "Changes made now are only for this device" };
            ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(help));
            if response.clicked() && !chosen {
                self.keys.scope = if k == 0 { inner.clone() } else { Scope::Device(fp.clone(), Box::new(inner.clone())) };
            }
        }
        top + bottom - 38.0 + 10.0
    }

    /// Mouse buttons the drawn mouse doesn't have (6 to 8) that were detected
    /// or have a rule: listed with what they do, so they can be found again.
    /// Its bottom.
    pub(in crate::app) fn other_buttons(&mut self, ui: &mut Ui, at: Pos2, w: f32, input: &Input, facts: &Facts) -> f32 {
        let mut list: Vec<u32> = self.keys.other_buttons.clone();
        for n in 6..=8 {
            if tune::button_has_rule(input, n) && !list.contains(&n) {
                list.push(n);
            }
        }
        list.sort_unstable();
        if list.is_empty() {
            return at.y;
        }
        let card = card_begin(ui, at, w, "Other buttons");
        let mut y = card.inner.top();
        let specs = input.specs(facts);
        for n in list {
            let code = keynames::button_code(n).map(|c| c as u16);
            let does = specs.iter().find(|(s, _)| Some(s.from.key) == code && !s.from.has_mods() && s.layer.is_none()).and_then(|(s, _)| match &s.what {
                What::To(o) => Some(tune::button_text(o)),
                What::TapHold { .. } => None,
            });
            let (does, changed) = does.map_or(("Unchanged".to_owned(), false), |d| (d, true));
            let name = button_name(n);
            let row = Rect::from_min_size(pos2(at.x, y), vec2(w, 34.0));
            let id = Id::unique(("other-button", n));
            let resp = ui.interact(row, id, widgets::sense(ui, Sense::click()));
            let sel = self.keys.button == Some(n);
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered() || resp.has_focus()), 0.1, motion::out_cubic);
            let fill = if sel { pal().accent_soft } else { mix(Color32::TRANSPARENT, pal().hover, hover) };
            ui.painter().rect_filled(row, CornerRadius::same(8), fill);
            widgets::focus_ring(ui, &resp, row, 8);
            let p = ui.painter();
            p.text(pos2(row.left() + 10.0, row.center().y), Align2::LEFT_CENTER, &name, if sel { semibold(T_BODY) } else { regular(T_BODY) }, if sel { pal().accent_text } else { pal().ink });
            let nw = text_width(ui, &name, semibold(T_BODY));
            let shown = fit(p, &does, &regular(T_BODY), row.width() - nw - 40.0);
            p.text(pos2(row.right() - 10.0, row.center().y), Align2::RIGHT_CENTER, shown, regular(T_BODY), if changed { pal().accent_text } else { pal().ink2 });
            widgets::a11y(&resp, Role::ListItem, &format!("{name}: {does}. Change it"), Some(sel));
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() && !self.tuning_busy() {
                self.keys.button = if sel { None } else { Some(n) };
                self.keys.reveal = !sel;
                self.keys.capture = None;
                self.keys.picker = None;
            }
            y += 36.0;
        }
        card.end(ui, y - 2.0);
        y - 2.0
    }
}

/// A backend's reason as a sentence's start.
fn sentence(why: &str) -> String {
    let mut c = why.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
