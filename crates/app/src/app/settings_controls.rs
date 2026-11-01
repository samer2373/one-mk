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
            let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 54.0));
            ui.painter().text(row.left_top(), Align2::LEFT_TOP, label, regular(T_BODY), INK2);
            let listening = recording == Some(index);
            let shown = if listening { "Press keys…".into() } else { crate::view::chord_label(chord, cfg!(target_os = "macos")) };
            ui.painter().text(row.left_top() + vec2(0.0, 23.0), Align2::LEFT_TOP, &shown, regular(T_META), INK);
            a11y_text(ui, Id::unique(("shortcut-value", index)), row, Role::Label, &format!("{label}: {shown}"), None);
            let change = Rect::from_min_size(pos2(row.right() - 74.0, row.top() + 9.0), vec2(74.0, 28.0));
            if button(ui, change, Id::unique(("shortcut-change", index)), if listening { "Cancel" } else { "Change" }, Look::Quiet, true, Some(&format!("Change {label}"))).clicked() {
                recording = if listening { None } else { Some(index) };
                started = !listening;
            }
            top += 56.0;
        }
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
        if let Some(note) = self.running().and_then(|status| crate::view::hotkeys_note(&status.hotkeys, &status.hotkeys_detail)) {
            para(ui, pos2(rect.left(), top), rect.width(), &crate::view::window_issue(&note), regular(T_META), INK2);
        }
    }

    pub(super) fn hook_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let mut top = rect.top();
        for (index, (key, label)) in crate::view::HOOK_EVENTS.iter().enumerate() {
            ui.painter().text(pos2(rect.left(), top), Align2::LEFT_TOP, label, regular(T_BODY), INK2);
            let current = self.prefs.hooks.iter().find(|(event, _)| event == label).map(|(_, command)| command.as_str()).unwrap_or("").to_owned();
            let event = key.trim_start_matches("on_");
            let (computer, timeout, shell) = self.prefs.hook_options.get(event).cloned().unwrap_or_else(|| (String::new(), self.prefs.hooks_timeout, self.prefs.hooks_shell));
            let mut command = self.drafts.get(*key).cloned().unwrap_or(current.clone());
            let field = Rect::from_min_size(pos2(rect.left(), top + 23.0), vec2(rect.width(), 28.0));
            widgets::field(ui, field, Id::unique(("hook-command", key)), &mut command, "Add a command", &format!("Command when {}", label.to_lowercase()), !self.prefs.unreadable, true);
            self.drafts.insert((*key).into(), command.clone());
            let mut chosen = self.drafts.get(&format!("{key}-computer")).cloned().unwrap_or(computer);
            let mut seconds = self.drafts.get(&format!("{key}-seconds")).cloned().unwrap_or(timeout.to_string());
            let mut use_shell = self.drafts.get(&format!("{key}-shell")).map_or(shell, |value| value == "on");
            let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(pos2(rect.left(), top + 58.0), vec2((rect.width() - 94.0).max(80.0), 28.0))));
            child.horizontal(|ui| {
                ui.label("With");
                egui::ComboBox::from_id_salt(("hook-computer", key)).selected_text(if chosen.is_empty() { "Any computer".into() } else {
                    self.running().and_then(|status| status.peers.iter().find(|peer| chosen.replace('-', "").starts_with(&peer.id.replace('-', "")))).map_or("Offline computer".into(), |peer| peer.name.clone())
                }).width((rect.width() - 144.0).max(70.0)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, String::new(), "Any computer");
                    if let Some(status) = self.running() { for peer in &status.peers { ui.selectable_value(&mut chosen, peer.id.clone(), &peer.name); } }
                });
            });
            ui.painter().text(pos2(rect.right() - 88.0, top + 71.0), Align2::LEFT_CENTER, "Secs", regular(T_META), INK2);
            widgets::field(ui, Rect::from_min_size(pos2(rect.right() - 44.0, top + 58.0), vec2(44.0, 26.0)), Id::unique(("hook-seconds", key)), &mut seconds, "5", "Time limit in seconds, 1 to 60", true, true);
            let row = Rect::from_min_size(pos2(rect.left(), top + 88.0), vec2(rect.width(), 30.0));
            if switch(ui, row, Id::unique(("hook-shell", key)), "Run as a shell command", use_shell, true).clicked() { use_shell = !use_shell; }
            self.drafts.insert(format!("{key}-computer"), chosen.clone());
            self.drafts.insert(format!("{key}-seconds"), seconds.clone());
            self.drafts.insert(format!("{key}-shell"), if use_shell { "on" } else { "off" }.into());
            let save = Rect::from_min_size(pos2(rect.left(), top + 122.0), vec2(76.0, 28.0));
            if button(ui, save, Id::unique(("hook-save", index)), if current.is_empty() { "Add" } else { "Save" }, Look::Primary, !command.trim().is_empty(), Some(&format!("Save command: {label}"))).clicked() {
                match number(&seconds, 60).filter_positive() {
                    Ok(seconds) => self.settings_command("the command", hook_args(key, &command, &chosen, seconds, use_shell)),
                    Err(error) => self.toast_error(error, None, self.now),
                }
            }
            let remove = Rect::from_min_size(pos2(rect.left() + 84.0, top + 122.0), vec2(76.0, 28.0));
            if button(ui, remove, Id::unique(("hook-remove", index)), "Remove", Look::Quiet, !current.is_empty(), Some(&format!("Remove command: {label}"))).clicked() {
                self.settings_command("the command", vec!["hooks".into(), format!("--{}", key.replace('_', "-")), String::new()]);
                self.drafts.remove(*key);
            }
            top += 166.0;
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
            ui.painter().text(pos2(rect.left(), top), Align2::LEFT_TOP, label, regular(T_BODY), INK);
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
        ui.painter().text(row.left_center(), Align2::LEFT_CENTER, "Stop when idle (minutes)", regular(T_META), INK);
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
