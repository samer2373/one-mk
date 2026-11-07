use super::*;

pub(super) fn number(text: &str, max: u32) -> Result<u32, String> {
    text.trim().parse::<u32>().ok().filter(|value| *value <= max).ok_or_else(|| format!("Enter a whole number from 0 to {max}."))
}

pub(super) fn shortcut_args(index: usize, chord: &str) -> Vec<String> {
    let flag = ["--jump", "--cycle", "--lock-pointer", "--home-key", "--broadcast", "--fetch-clipboard"][index];
    vec!["hotkeys".into(), flag.into(), chord.into()]
}

fn hook_args(event: &str, command: &str, computer: &str, seconds: u32, shell: bool) -> Vec<String> {
    vec!["hooks".into(), format!("--{}", event.replace('_', "-")), command.into(), "--computer".into(), computer.into(),
        "--event-timeout".into(), seconds.to_string(), "--event-shell".into(), if shell { "on" } else { "off" }.into()]
}

pub(super) fn shortcut_error(error: &str) -> &'static str {
    if error.contains("would do two things") { "That shortcut is already used. Choose different keys." }
    else if error.contains("always brings the pointer back") { "Those keys always bring the pointer back here. Choose different keys." }
    else if error.contains("take over typing") { "Include Control, Alt or the system key so normal typing still works." }
    else { "Couldn't save that shortcut. Choose different keys and try again." }
}

pub(super) fn recorded(events: &[egui::Event], jump: bool) -> Option<Result<String, ()>> {
    for event in events {
        if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
            if *key == Key::Escape { return Some(Err(())); }
            if *key == Key::Backspace { return Some(Ok("none".into())); }
            let Some(code) = crate::tune::evdev_of(*key) else { continue };
            let name = one_mk_proto::keynames::key_name(u32::from(code));
            let mut parts = Vec::new();
            if modifiers.ctrl { parts.push("ctrl"); }
            if modifiers.alt { parts.push("alt"); }
            if modifiers.shift { parts.push("shift"); }
            if modifiers.mac_cmd || (!cfg!(target_os = "macos") && modifiers.command && !modifiers.ctrl) { parts.push("super"); }
            if jump {
                if !matches!(*key, Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9) { continue; }
                parts.push("{n}");
            } else { parts.push(&name); }
            return Some(Ok(parts.join("+")));
        }
    }
    None
}

impl App {
    pub(super) fn pairing_controls(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let link_id = Id::unique("invitation-link");
        let link = ui.ctx().data(|data| data.get_temp::<String>(link_id));
        let invite = Rect::from_min_size(rect.min, vec2(166.0, 28.0));
        let mut method = None;
        if button(ui, invite, Id::unique("pair-invite"), "Invite with a link", Look::Quiet, link.is_none(), None).clicked() { method = Some(crate::pairing::Method::Invite); }
        let mut top = rect.top() + 36.0;
        if let Some(link) = link {
            let mut shown = link.clone();
            let field = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width() - 64.0, 28.0));
            widgets::field(ui, field, Id::unique("pair-invitation"), &mut shown, "", "Private invitation link", false, true);
            let copy = Rect::from_min_size(pos2(rect.right() - 58.0, top), vec2(58.0, 28.0));
            if button(ui, copy, Id::unique("pair-copy-invite"), "Copy", Look::Quiet, true, None).clicked() { ui.ctx().copy_text(link); }
        } else {
            para(ui, pos2(rect.left(), top), rect.width(), "Share the private link only with the person pairing.", regular(T_META), INK2);
        }
        top += 40.0;
        for (key, label, hint) in [("join-link", "Join", "Join with a link"), ("connect-address", "Connect", "Connect by address")] {
            let mut value = self.drafts.get(key).cloned().unwrap_or_default();
            let field = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width() - 80.0, 28.0));
            let response = widgets::field(ui, field, Id::unique(key), &mut value, hint, hint, true, false);
            self.drafts.insert(key.into(), value.clone());
            let action = Rect::from_min_size(pos2(rect.right() - 74.0, top), vec2(74.0, 28.0));
            if (button(ui, action, Id::unique((key, "go")), label, Look::Primary, !value.trim().is_empty(), Some(hint)).clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)))) && !value.trim().is_empty() {
                method = Some(if key == "join-link" { crate::pairing::Method::Join(value) } else { crate::pairing::Method::Connect(value) });
                self.drafts.remove(key);
            }
            top += 38.0;
        }
        para(ui, pos2(rect.left(), top), rect.width(), "Address: a computer name or IP, with its pairing port after a colon.", regular(T_META), INK2);
        if let Some(method) = method {
            ui.ctx().data_mut(|data| data.remove::<String>(link_id));
            let port = self.running().map_or(one_mk_proto::DEFAULT_PORT, |status| status.port);
            if let Some(pair) = &mut self.pairing { pair.proc = None; }
            match self.link.pair_with(method, port) {
                Ok(proc) => if let Some(pair) = &mut self.pairing {
                    pair.proc = Some(proc);
                    pair.step = Step::Searching { me: pair.me.clone(), port: None };
                    pair.since = now;
                    pair.retry = None;
                },
                Err(error) => {
                    if let Some(pair) = &mut self.pairing { pair.step = Step::Ended("Check the link or address and try again.".into()); }
                    self.toast_error("Couldn't start pairing. Check the link or address and try again.", Some(error.to_string()), now);
                }
            }
        }
    }
    pub(super) fn edge_setting(&mut self, side: &str, flag: &str, value: &str) {
        let mut args = vec!["edge".into(), flag.into(), value.into()];
        if !side.is_empty() { args.extend(["--side".into(), side.into()]); }
        self.settings_command("the edge rules", args);
    }
    pub(super) fn settings_command(&mut self, what: &str, args: Vec<String>) {
        self.busy.push(Job::Set { what: what.into(), args: args.clone() });
        self.link.setting(what, args);
    }

    pub(super) fn settings_segment(&self, ui: &mut Ui, rect: Rect, key: &str, choices: &[(&str, &str)], current: &str) -> Option<String> {
        let labels: Vec<_> = choices.iter().map(|(_, label)| *label).collect();
        Pills { id: Id::unique(key), track: rect, cells: Pills::even(rect, choices.len(), 2.0), labels: &labels,
            names: labels.iter().map(|label| format!("{key}: {label}")).collect(), chosen: choices.iter().position(|(value, _)| *value == current),
            enabled: vec![true; choices.len()], role: Role::RadioButton, font: T_META }.show(ui, self.arrowed).map(|index| choices[index].0.to_owned())
    }

    pub(super) fn shortcut_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let mut top = rect.top();
        let recorder = Id::unique("shortcut-recorder");
        let mut recording = ui.ctx().data(|data| data.get_temp::<usize>(recorder));
        let mut started = false;
        let shortcuts = self.prefs.hotkeys.clone();
        for (index, (label, chord)) in shortcuts.iter().enumerate() {
            // A row each, as the system's own shortcut lists: the name on
            // the left, the keys in a recessed recorder on the right; click
            // the recorder and press the new keys.
            let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), SHORTCUT_ROW));
            let listening = recording == Some(index);
            let shown = if listening { "Press keys…".into() } else { crate::view::chord_label(chord, cfg!(target_os = "macos")) };
            let font = mono(T_META);
            let kw = (text_width(ui, &shown, font.clone()) + 24.0).max(112.0).min(row.width() * 0.55);
            let kr = Rect::from_min_size(pos2(row.right() - kw, row.center().y - 14.0), vec2(kw, 28.0));
            let room = kr.left() - row.left() - 12.0;
            ui.painter().text(row.left_center(), Align2::LEFT_CENTER, fit(ui.painter(), label, &regular(T_BODY), room), regular(T_BODY), INK);
            let id = Id::unique(("shortcut-change", index));
            let resp = ui.interact(kr, id, widgets::sense(ui, Sense::click()));
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.1, motion::out_cubic);
            let p = ui.painter();
            let well = if resp.is_pointer_button_down_on() { Color32::from_black_alpha(130) } else { mix(Color32::from_black_alpha(70), Color32::from_black_alpha(40), hover) };
            p.rect_filled(kr, CornerRadius::same(8), well);
            let edge = if listening { Stroke::new(1.5, ACCENT) } else { Stroke::new(1.0, mix(FIELD_EDGE, FIELD_EDGE_LIT, hover)) };
            p.rect_stroke(kr, CornerRadius::same(8), edge, StrokeKind::Inside);
            widgets::mono_text(p, kr.center(), Align2::CENTER_CENTER, &shown, T_META, if listening { INK2 } else { INK }, kw - 16.0);
            widgets::focus_ring(ui, &resp, kr, 8);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            widgets::a11y(&resp, Role::Button, &format!("{} {label}", if listening { "Cancel changing" } else { "Change" }), None);
            let resp = resp.on_hover_text(if listening { "Press the new keys. Esc cancels; Backspace turns it off." } else { "Click, then press the new keys" });
            if resp.clicked() {
                recording = if listening { None } else { Some(index) };
                started = !listening;
            }
            if index + 1 < shortcuts.len() {
                ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, LINE));
            }
            a11y_text(ui, Id::unique(("shortcut-value", index)), Rect::from_min_max(row.min, pos2(kr.left() - 4.0, row.bottom())), Role::Label, &format!("{label}: {shown}"), None);
            top += SHORTCUT_ROW;
        }
        top += 10.0;
        if !started && let Some(index) = recording {
            let events = ui.input(|input| input.events.clone());
            if let Some(result) = recorded(&events, index == 0) {
                if let Ok(chord) = result { self.settings_command("the shortcut", shortcut_args(index, &chord)); }
                recording = None;
            }
        }
        ui.ctx().data_mut(|data| { if let Some(index) = recording { data.insert_temp(recorder, index); } else { data.remove::<usize>(recorder); } });
        let emergency = crate::view::chord_label("ctrl+alt+shift+esc", cfg!(target_os = "macos"));
        top += para(ui, pos2(rect.left(), top), rect.width(), &format!("Always back here: {emergency}. While changing: Esc cancels; Backspace turns it off. For computer numbers, press a number from 1 to 9."), regular(T_META), INK2) + 8.0;
        // "Ready, change them in Settings" says nothing here, in Settings.
        if let Some(note) = self.running().filter(|status| status.hotkeys != "on").and_then(|status| crate::view::hotkeys_note(&status.hotkeys, &status.hotkeys_detail)) {
            para(ui, pos2(rect.left(), top), rect.width(), &crate::view::window_issue(&note), regular(T_META), INK2);
        }
    }

    pub(super) fn hook_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let mut top = rect.top();
        for (index, (key, label)) in crate::view::HOOK_EVENTS.iter().enumerate() {
            // One event: its name and which computer it is about; the
            // command with its button; then the time limit and the shell.
            if index > 0 {
                ui.painter().hline(rect.x_range(), top - 9.0, Stroke::new(1.0, HAIRLINE));
            }
            let current = self.prefs.hooks.iter().find(|(event, _)| event == label).map(|(_, command)| command.as_str()).unwrap_or("").to_owned();
            let event = key.trim_start_matches("on_");
            let (computer, timeout, shell) = self.prefs.hook_options.get(event).cloned().unwrap_or_else(|| (String::new(), self.prefs.hooks_timeout, self.prefs.hooks_shell));
            let mut command = self.drafts.get(*key).cloned().unwrap_or(current.clone());
            let mut chosen = self.drafts.get(&format!("{key}-computer")).cloned().unwrap_or(computer);
            let mut seconds = self.drafts.get(&format!("{key}-seconds")).cloned().unwrap_or(timeout.to_string());
            let mut use_shell = self.drafts.get(&format!("{key}-shell")).map_or(shell, |value| value == "on");
            let head = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 28.0));
            ui.painter().text(head.left_center(), Align2::LEFT_CENTER, label, regular(T_BODY), INK);
            let mut options = vec![(String::new(), "Any computer".to_owned())];
            if let Some(status) = self.running() { options.extend(status.peers.iter().map(|peer| (peer.id.clone(), peer.name.clone()))); }
            let shown = options.iter().find(|(id, _)| id.is_empty() && chosen.is_empty() || !id.is_empty() && chosen.replace('-', "").starts_with(&id.replace('-', ""))).map_or("A computer that's gone".to_owned(), |(_, name)| name.clone());
            let w = (text_width(ui, &shown, regular(T_META)) + 40.0).clamp(110.0, (rect.width() * 0.5).max(110.0));
            if let Some(id) = chooser(ui, Rect::from_min_size(pos2(head.right() - w, head.top()), vec2(w, 28.0)), Id::unique(("hook-computer", key)), &shown, &format!("Which computer: {label}"), &options, &chosen) {
                chosen = id;
            }
            let save_w = 70.0;
            let field = Rect::from_min_size(pos2(rect.left(), top + 34.0), vec2(rect.width() - save_w - 8.0, 30.0));
            widgets::field(ui, field, Id::unique(("hook-command", key)), &mut command, "Add a command", &format!("Command when {}", label.to_lowercase()), !self.prefs.unreadable, true);
            let save = Rect::from_min_size(pos2(field.right() + 8.0, field.top()), vec2(save_w, 30.0));
            let changed = command.trim() != current.trim();
            if !current.is_empty() && command.trim().is_empty() {
                if button(ui, save, Id::unique(("hook-remove", index)), "Remove", Look::Quiet, true, Some(&format!("Remove command: {label}"))).clicked() {
                    self.settings_command("the command", vec!["hooks".into(), format!("--{}", key.replace('_', "-")), String::new()]);
                    self.drafts.remove(*key);
                }
            } else if button(ui, save, Id::unique(("hook-save", index)), if current.is_empty() { "Add" } else { "Save" }, if changed || current.is_empty() { Look::Primary } else { Look::Quiet }, !command.trim().is_empty(), Some(&format!("Save command: {label}"))).clicked() {
                match number(&seconds, 60).filter_positive() {
                    Ok(seconds) => self.settings_command("the command", hook_args(key, &command, &chosen, seconds, use_shell)),
                    Err(error) => self.toast_error(error, None, self.now),
                }
            }
            let opts_y = top + 70.0;
            ui.painter().text(pos2(rect.left(), opts_y + 15.0), Align2::LEFT_CENTER, "Stop it after", regular(T_META), INK2);
            let after = text_width(ui, "Stop it after", regular(T_META));
            widgets::field(ui, Rect::from_min_size(pos2(rect.left() + after + 8.0, opts_y + 2.0), vec2(40.0, 26.0)), Id::unique(("hook-seconds", key)), &mut seconds, "5", "Time limit in seconds, 1 to 60", true, true);
            ui.painter().text(pos2(rect.left() + after + 56.0, opts_y + 15.0), Align2::LEFT_CENTER, "seconds", regular(T_META), INK2);
            let shell_row = Rect::from_min_size(pos2(rect.right() - 170.0, opts_y), vec2(170.0, 30.0));
            if switch_help(ui, shell_row, Id::unique(("hook-shell", key)), "Run in a shell", "hook-shell", use_shell, true).clicked() { use_shell = !use_shell; }
            self.drafts.insert((*key).into(), command.clone());
            self.drafts.insert(format!("{key}-computer"), chosen.clone());
            self.drafts.insert(format!("{key}-seconds"), seconds.clone());
            self.drafts.insert(format!("{key}-shell"), if use_shell { "on" } else { "off" }.into());
            top += HOOK_H;
        }
        para(ui, pos2(rect.left(), top), rect.width(), "Commands run here with your permissions. Arrivals and departures refer to this computer. Only add commands you trust.", regular(T_META), INK2);
    }

    pub(super) fn extra_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let mut top = rect.top();
        for (key, label, choices, current) in [
            ("wrap", "Pointer wrap", &[("off", "Off"), ("on", "Left / right"), ("all", "All edges")][..], self.prefs.edge_wrap.clone()),
            ("learn", "Learn computer positions", &[("off", "Off"), ("suggest", "Ask me"), ("auto", "Place them")][..], self.prefs.edge_learn.clone()),
            ("pointer", "Draw a pointer on computers with no mouse", &[("auto", "Auto"), ("always", "Always"), ("never", "Never")][..], self.prefs.pointer_draw.clone()),
        ] {
            let help = match key { "wrap" => "edge-wrap", "learn" => "edge-learn", _ => "pointer-overlay" };
            label_help_inline(ui, Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 18.0)), Id::unique(("extra-label", key)), label, help, true);
            let track = Rect::from_min_size(pos2(rect.left(), top + 26.0), vec2(rect.width(), 28.0));
            if let Some(value) = self.settings_segment(ui, track, label, choices, &current) {
                let args = if key == "pointer" { vec!["pointer-overlay".into(), value] } else { vec!["edge".into(), format!("--{key}"), value] };
                self.settings_command(label, args);
            }
            top += 70.0;
        }
        self.broadcast_timeout(ui, Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 34.0)));
    }

    pub(super) fn broadcast_timeout(&mut self, ui: &mut Ui, row: Rect) {
        label_help(ui, row, Id::unique("broadcast-label"), "Stop when idle (minutes)", "broadcast-idle", true, 54.0 + 12.0);
        let field = Rect::from_min_size(pos2(row.right() - 54.0, row.top() + 3.0), vec2(54.0, 28.0));
        if let Some(text) = self.edit_field(ui, field, "broadcast-idle", &self.prefs.broadcast_idle.to_string(), "15", "Broadcast idle timeout in minutes; zero means never", true, true) {
            match number(&text, 1440) {
                Ok(minutes) => self.set_pref("broadcast idle timeout", &["broadcast", "--idle-off", &minutes.to_string()], |prefs| prefs.broadcast_idle = minutes),
                Err(error) => self.toast_error(error, None, self.now),
            }
        }
    }
}

trait Positive {
    fn filter_positive(self) -> Result<u32, String>;
}

impl Positive for Result<u32, String> {
    fn filter_positive(self) -> Result<u32, String> { self.and_then(|value| if value > 0 { Ok(value) } else { Err("Enter a whole number from 1 to 60.".into()) }) }
}

/// One event's height in the commands card.
pub(super) const HOOK_H: f32 = 120.0;
/// A keyboard shortcut's row in Settings.
pub(super) const SHORTCUT_ROW: f32 = 44.0;

/// A quiet button showing the choice, with a menu of the others.
pub(super) fn chooser(ui: &mut Ui, r: Rect, id: Id, shown: &str, name: &str, options: &[(String, String)], current: &str) -> Option<String> {
    let resp = button(ui, r, id, "", Look::Quiet, true, Some(name));
    let p = ui.painter();
    p.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, fit(p, shown, &regular(T_META), r.width() - 36.0), regular(T_META), INK);
    let c = pos2(r.right() - 14.0, r.center().y);
    p.add(egui::Shape::line(vec![c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)], Stroke::new(1.5, INK2)));
    let mut picked = None;
    egui::Popup::menu(&resp).width(220.0).align(egui::RectAlign::BOTTOM_END).show(|ui| {
        for (value, title) in options {
            let (row, item) = ui.allocate_exact_size(vec2(220.0, 28.0), Sense::click());
            let hover = tween(ui.ctx(), item.id.with("hover"), f32::from(item.hovered()), 0.1, motion::out_cubic);
            let p = ui.painter().clone();
            p.rect_filled(row, CornerRadius::same(6), if item.is_pointer_button_down_on() { widgets::pressed(BUTTON, 1.0) } else { mix(Color32::TRANSPARENT, BUTTON, hover) });
            widgets::focus_ring(ui, &item, row, 6);
            let on = value == current;
            if on {
                widgets::draw_check(&p, pos2(row.left() + 16.0, row.center().y), 10.0, INK, 1.0);
            }
            p.text(pos2(row.left() + 30.0, row.center().y), Align2::LEFT_CENTER, fit(&p, title, &regular(T_BODY), row.width() - 40.0), regular(T_BODY), if on { INK } else { INK_HOVER });
            item.widget_info(|| egui::WidgetInfo::selected(Role::MenuItemRadio, true, on, title));
            if item.clicked() {
                picked = Some(value.clone());
                ui.close();
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_numbers_are_bounded() {
        assert_eq!(number("0", 1440).unwrap(), 0);
        for value in ["-1", "1.5", "1441", "", "9999999999999999"] { assert!(number(value, 1440).is_err()); }
        assert!(number("0", 60).filter_positive().is_err());
    }

    #[test]
    fn shortcut_commands_cover_fetch_and_home() {
        assert_eq!(shortcut_args(5, "ctrl+alt+v"), ["hotkeys", "--fetch-clipboard", "ctrl+alt+v"]);
        assert_eq!(shortcut_args(3, "none"), ["hotkeys", "--home-key", "none"]);
    }

    #[test]
    fn hooks_keep_commands_as_one_argument() {
        assert_eq!(hook_args("on_peer_connect", "echo 'hello world'", "abcd-1234", 17, true),
            ["hooks", "--on-peer-connect", "echo 'hello world'", "--computer", "abcd-1234", "--event-timeout", "17", "--event-shell", "on"]);
        assert_eq!(hook_args("on_enter", "echo hi", "", 1, false).last().unwrap(), "off");
    }

    #[test]
    fn shortcut_failures_are_plain_and_actionable() {
        assert!(shortcut_error("Ctrl+K would do two things: cycle and home").contains("already used"));
        assert!(shortcut_error("always brings the pointer back").contains("always bring"));
        assert!(shortcut_error("would take over typing").contains("normal typing"));
        assert!(!shortcut_error("--jump: settings.toml").contains("--"));
    }

    #[test]
    fn recorder_receives_keys_from_egui_and_cancels_or_clears() {
        let context = egui::Context::default();
        for (key, jump, expected) in [(Key::V, false, Ok("ctrl+alt+v")), (Key::Num2, true, Ok("ctrl+alt+{n}")),
            (Key::Escape, false, Err(())), (Key::Backspace, false, Ok("none"))] {
            let input = egui::RawInput { events: vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false,
                modifiers: egui::Modifiers { ctrl: true, alt: true, ..Default::default() } }], ..Default::default() };
            let mut output = context.run_pass(input, |context| {
                assert_eq!(recorded(&context.input(|input| input.events.clone()), jump), Some(expected.map(str::to_owned)));
            });
            output.textures_delta.clear();
        }
    }
}
