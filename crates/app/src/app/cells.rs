//! Panel cells for the newer controls: this computer's switches (pointer
//! lock, lock all; gaming mode is drawn here for Settings), a computer's quick actions (switch to it,
//! its clipboard), the clipboard and file settings, and transfers as they
//! run.

use super::page_ui::{self, Cap, Page, ROW, Words, toggle_row};
use super::*;
use crate::view;
use crate::widgets::Mark;

const GAMING: [(&str, &str); 3] = [("off", "Off"), ("on", "On"), ("auto", "Auto")];

/// A transfer's row: its words, its numbers, a 5 point bar; a stalled one
/// has a line more for what to do.
fn transfer_height(transfer: &one_mk_ctl::status::TransferStatus) -> f32 {
    if transfer.rate == 0 { 82.0 } else { 64.0 }
}

#[derive(Default)]
pub(super) struct FilesUi {
    picker: Option<(String, Receiver<Option<Vec<PathBuf>>>)>,
    all_transfers: bool,
}

impl App {
    /// Clipboard & files (handoff 5.4): the transfers as they run, who files
    /// can be sent to, what the clipboard shares, its history, and the file
    /// settings, as cards.
    pub(super) fn files_panel(&mut self, ui: &mut Ui, rect: Rect) {
        if let Some(paths) = self.files_ui.picker.as_ref().and_then(|(_, receiver)| receiver.try_recv().ok()) {
            let (id, _) = self.files_ui.picker.take().expect("picker completed");
            if let Some(paths) = paths.filter(|paths| !paths.is_empty()) {
                if let Some(index) = self.online_targets().into_iter().find(|index| self.scene.devices[*index].id == id) {
                    self.send_to(index, paths);
                } else {
                    self.toast_error("That computer disconnected. Reconnect it, then choose the files again.", None, self.now);
                }
            }
        }
        let status = self.running().cloned();
        let dropped: Vec<PathBuf> = ui.input(|input| input.raw.dropped_files.iter().map(|file| file.path().to_path_buf()).collect());
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
        let mut drop_id = None;
        let out = page_ui::scroll("files-page").show_viewport(ui, |ui, _| {
            let origin = ui.cursor().min;
            let mut page = Page::begin(ui, origin, rect.width(), "Clipboard & files", "What you copy and send follows you between computers. Drop files on a computer on the desk to send them.");
            let transfers = status.as_ref().map(|status| status.transfers.clone()).unwrap_or_default();
            let recipients = status.as_ref().map(|status| status.peers.clone()).unwrap_or_default();

            // Transfers, while there are any.
            if !transfers.is_empty() {
                let shown = if self.files_ui.all_transfers { transfers.len() } else { transfers.len().min(if page.col_w < 658.0 { 1 } else { 4 }) };
                let mut card = page.card(ui, 0, "Transfers");
                if transfers.len() > shown || self.files_ui.all_transfers {
                    let label = if self.files_ui.all_transfers { "Show less".to_owned() } else { format!("Show all {}", transfers.len()) };
                    let width = button_width(ui, &label);
                    let at = card.label_right();
                    let r = Rect::from_min_size(pos2(at.x - width, at.y - 14.0), vec2(width, 28.0));
                    if button(ui, r, Id::unique("files-all-transfers"), &label, Look::Text, true, None).clicked() {
                        self.files_ui.all_transfers = !self.files_ui.all_transfers;
                    }
                }
                for t in transfers.iter().take(shown) {
                    let row = card.row(ui, transfer_height(t));
                    self.transfer_row(ui, row, t);
                }
                page.finish(ui, card);
            }

            // Who files can go to.
            let reconnect;
            let hint = if !transfers.is_empty() && recipients.iter().all(|peer| !peer.online) { "Transfer details and reconnect steps are above." } else if !recipients.is_empty() && recipients.iter().all(|peer| !peer.online) {
                reconnect = format!("Open oneMK on {} to reconnect, then choose files or drop them here.", recipients[0].name);
                reconnect.as_str()
            } else { "Choose a recipient to pick files, or drop files on its row." };
            let mut card = page.card(ui, 0, "Send files");
            if recipients.is_empty() {
                let row = card.row(ui, page_ui::EMPTY_ROW);
                page_ui::empty_row(ui, row, "Pair a computer to send files to it.");
            }
            for peer in &recipients {
                let row = card.row(ui, ROW);
                let name = format!("{}{}", peer.name, if peer.online { "" } else { " · Offline" });
                let label = "Choose files…";
                let words = Words::new(&name).mark(if peer.online { Mark::Ok } else { Mark::Off });
                page_ui::paint_words(ui, row, &words, button_width(ui, label) + 12.0);
                let can = peer.online && self.files_ui.picker.is_none();
                let response = page_ui::row_button(ui, row, 0.0, Id::unique(("files-recipient", &peer.id)), label, Look::Quiet, can, Some(&format!("Send files to {}. Choose files, or drop files on this recipient.", peer.name)));
                widgets::a11y(&response, Role::Button, &format!("Send files to {}{}", peer.name, if peer.online { ". Choose files or drop here" } else { ", offline" }), None);
                if response.clicked() {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    self.files_ui.picker = Some((peer.id.clone(), receiver));
                    let context = ui.ctx().clone();
                    let title = format!("Send files to {}", peer.name);
                    std::thread::spawn(move || {
                        let _ = sender.send(rfd::FileDialog::new().set_title(title).pick_files());
                        context.request_repaint();
                    });
                }
                if peer.online && pointer.is_some_and(|pointer| row.expand2(vec2(page_ui::CARD_PAD, 0.0)).contains(pointer)) {
                    if hovering {
                        ui.painter().rect_stroke(row.expand2(vec2(page_ui::CARD_PAD - 4.0, -3.0)), CornerRadius::same(R_INNER), Stroke::new(2.0, pal().accent), StrokeKind::Inside);
                    }
                    if !dropped.is_empty() {
                        drop_id = Some(peer.id.clone());
                    }
                }
            }
            let inner = card.w - 2.0 * page_ui::CARD_PAD;
            let hint_h = page_ui::text_h(ui, hint, inner, regular(T_META));
            let row = card.row(ui, hint_h + 24.0);
            para(ui, row.min + vec2(0.0, 12.0), inner, hint, regular(T_META), pal().ink2);
            page.finish(ui, card);

            // The files' own settings: under the sending in one column;
            // with two, in whichever column keeps them even.
            if !page.two {
                let mut card = page.card(ui, 0, "Files");
                self.files_rows(ui, &mut card);
                page.finish(ui, card);
            }

            // The clipboard: what it shares, and who may take it.
            let mut card = page.card(ui, 1, "Clipboard");
            self.clipboard_rows(ui, &mut card);
            page.finish(ui, card);
            if page.two {
                // The taller of the two (Files) goes where there is more room, History to the other.
                let col = page.shorter();
                let mut card = page.card(ui, col, "Files");
                self.files_rows(ui, &mut card);
                page.finish(ui, card);
                let col = page.shorter();
                self.history_card(ui, &mut page, col);
            } else {
                self.history_card(ui, &mut page, 0);
            }
            // Where received files go, at the foot of the page.
            if let Some(dir) = received_dir() {
                page.footer(ui, origin.y + rect.height(), &format!("Received files go to `{dir}`"));
            }
            ui.allocate_space(vec2(rect.width(), page.height()));
        });
        let _ = out;
        if !dropped.is_empty() {
            if let Some(index) = drop_id.and_then(|id| self.online_targets().into_iter().find(|index| self.scene.devices[*index].id == id)) {
                self.send_to(index, dropped);
            } else {
                self.drop_files(dropped, None, self.now);
            }
        }
    }

    /// One transfer: what, how far (a 5 point bar, and the numbers in mono),
    /// and, when it stalls, the way to go on. There is nothing to press: the
    /// daemon has no way to cancel or retry one, and a finished one is gone
    /// from its list.
    fn transfer_row(&self, ui: &Ui, r: Rect, t: &one_mk_ctl::status::TransferStatus) {
        let (title, detail, frac) = view::transfer_lines(t);
        let stalled = t.rate == 0;
        // Too long for the card: leave out "x of y" before cutting anything.
        let (x, room) = page_ui::words_box(r, stalled, 0.0);
        let detail = if text_width(ui, &detail, mono(T_META)) > room && frac.is_some() {
            detail.split(" · ").enumerate().filter(|(k, _)| *k != 1).map(|(_, s)| s).collect::<Vec<_>>().join(" · ")
        } else {
            detail
        };
        let mut words = Words::new(&title).caption(&detail).cap(if stalled { Cap::WarnMono } else { Cap::Mono });
        if stalled {
            words = words.mark(Mark::Warn);
        }
        page_ui::paint_words(ui, r, &words, 0.0);
        let offline = self.running().is_some_and(|status| status.peers.iter().any(|peer| peer.name == t.peer && !peer.online));
        let recovery = if stalled && offline { format!("Open oneMK on {} to reconnect.", t.peer) } else if stalled { "Connected · waiting for transfer progress.".into() } else { String::new() };
        let p = ui.painter();
        if !recovery.is_empty() {
            p.text(pos2(x, r.top() + 54.0), Align2::LEFT_CENTER, fit(p, &recovery, &regular(T_META), room), regular(T_META), pal().ink2);
        }
        let bar = Rect::from_min_size(pos2(x, r.top() + if recovery.is_empty() { 50.0 } else { 68.0 }), vec2(room, 5.0));
        if stalled {
            widgets::paint_progress(p, bar, frac.or(Some(0.0)), true);
        } else {
            widgets::progress_bar(ui, bar, Id::unique(("transfer", t.id, &t.peer, t.sending)), frac);
        }
        let label = format!("{title}: {detail}. {recovery}");
        ui.ctx().accesskit_node_builder(Id::unique(("transfer-node", t.id, &t.peer)), |n| {
            n.set_role(Role::ProgressIndicator);
            n.set_label(label);
            if let Some(f) = frac {
                n.set_numeric_value(f64::from(f) * 100.0);
                n.set_min_numeric_value(0.0);
                n.set_max_numeric_value(100.0);
            }
            n.set_bounds(egui::accesskit::Rect { x0: r.min.x as f64, y0: r.min.y as f64, x1: r.max.x as f64, y1: r.max.y as f64 });
        });
    }

    /// What the clipboard shares, and whether others may take it.
    fn clipboard_rows(&mut self, ui: &mut Ui, card: &mut page_ui::Card) {
        let p = self.prefs.clone();
        for (key, label, on) in [("text", "Text", p.clip_text), ("html", "Rich text (HTML)", p.clip_html), ("image", "Images", p.clip_image)] {
            let row = card.row(ui, ROW);
            if toggle_row(ui, row, Id::unique(("clip", key)), &Words::new(label), Some(&format!("clip-{key}")), on, true).clicked() {
                let v = if on { "off" } else { "on" };
                let flag = format!("--{key}");
                self.set_pref("what the clipboard shares", &["clipboard", &flag, v], |p| match key {
                    "text" => p.clip_text = !on,
                    "html" => p.clip_html = !on,
                    _ => p.clip_image = !on,
                });
            }
        }
        let row = card.row(ui, ROW);
        if toggle_row(ui, row, Id::unique("allow-fetch"), &Words::new("Others can get this clipboard"), Some("allow-fetch"), p.clip_allow_fetch, true).clicked() {
            let v = if p.clip_allow_fetch { "off" } else { "on" };
            self.set_pref("who can get this clipboard", &["clipboard", "--allow-fetch", v], |p| p.clip_allow_fetch = !p.clip_allow_fetch);
        }
        let row = card.row(ui, ROW);
        if toggle_row(ui, row, Id::unique("clip-sensitive"), &Words::new("Skip sensitive copies"), Some("clip-sensitive"), p.clip_skip_sensitive, true).clicked() {
            self.set_pref("sensitive copies", &["clipboard", "--skip-sensitive", if p.clip_skip_sensitive { "off" } else { "on" }], |prefs| prefs.clip_skip_sensitive = !p.clip_skip_sensitive);
        }
    }

    /// Notifications, the received files folder, and the clipboard's size limit.
    fn files_rows(&mut self, ui: &mut Ui, card: &mut page_ui::Card) {
        let p = self.prefs.clone();
        let row = card.row(ui, ROW);
        if toggle_row(ui, row, Id::unique("notify"), &Words::new("Notify when files arrive"), Some("notify"), p.notify_files, true).clicked() {
            let v = if p.notify_files { "off" } else { "on" };
            self.set_pref("notifications", &["notify", v], |p| p.notify_files = !p.notify_files);
        }
        let row = card.row(ui, ROW);
        let busy = self.busy.contains(&Job::Open);
        page_ui::paint_words(ui, row, &Words::new("Received files"), button_width(ui, "Open") + 12.0);
        if page_ui::row_button(ui, row, 0.0, Id::unique("open-dl"), "Open", Look::Quiet, !busy, Some("Open received files folder")).clicked() {
            self.busy.push(Job::Open);
            self.link.open_downloads();
        }
        // The size limit: a number to type, in MB.
        let row = card.row(ui, ROW);
        let unit = "MB";
        let uw = text_width(ui, unit, regular(T_META)) + 8.0;
        page_ui::paint_words(ui, row, &Words::new("Largest clipboard"), 64.0 + uw + 12.0);
        let fr = Rect::from_min_size(pos2(row.right() - 64.0 - uw, row.center().y - 15.0), vec2(64.0, 30.0));
        ui.painter().text(pos2(row.right(), row.center().y), Align2::RIGHT_CENTER, unit, mono(T_META), pal().ink2);
        let cur = p.clip_max_mb.to_string();
        if let Some(text) = self.edit_field(ui, fr, "clip-max", &cur, "25", "Largest clipboard to send or accept, in MB", true, true) {
            match text.parse::<u32>() {
                Ok(n) if (1..=view::CLIP_MAX_MB).contains(&n) => {
                    let v = n.to_string();
                    self.set_pref("the clipboard size limit", &["clipboard", "--max-mb", &v], |p| p.clip_max_mb = n);
                }
                _ => self.toast_error(format!("“{text}” isn't a size oneMK can use. Type a whole number of MB from 1 to {}, like 25.", view::CLIP_MAX_MB), None, self.now),
            }
        }
        // The last row (file edge drops) only exists where the desktop can offer it.
        if view::drop_zones_available() {
            let row = card.row(ui, ROW);
            if toggle_row(ui, row, Id::unique("file-drop-zones"), &Words::new("Drop on screen edges (experimental)"), Some("file-drop-zones"), p.drop_zones, true).clicked() {
                self.set_pref("file drop zones", &["drag", if p.drop_zones { "--off" } else { "--on" }], |prefs| prefs.drop_zones = !p.drop_zones);
            }
        }
    }

    /// A field showing `current`; returns the new text when the person
    /// changed it and pressed Enter or moved on (Esc keeps the old value).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn edit_field(&mut self, ui: &mut Ui, rect: Rect, key: &str, current: &str, hint: &str, label: &str, enabled: bool, mono_text: bool) -> Option<String> {
        let id = Id::unique(("field", key));
        let focused = ui.memory(|m| m.has_focus(id));
        let mut text = if focused { self.drafts.get(key).cloned().unwrap_or_else(|| current.to_owned()) } else { current.to_owned() };
        let r = widgets::field(ui, rect, id, &mut text, hint, label, enabled, mono_text);
        if r.has_focus() {
            self.drafts.insert(key.to_owned(), text.clone());
        }
        if r.lost_focus() {
            self.drafts.remove(key);
            let esc = ui.input(|i| i.key_pressed(Key::Escape));
            if !esc && text.trim() != current {
                return Some(text.trim().to_owned());
            }
        }
        None
    }

    /// Changes a setting through `one-mk`, showing it at once.
    pub(super) fn set_pref(&mut self, what: &str, args: &[&str], apply: impl FnOnce(&mut view::Prefs)) {
        apply(&mut self.prefs);
        self.busy.push(Job::Set { what: what.into(), args: args.iter().map(|a| (*a).to_owned()).collect() });
        self.link.set(what, args);
    }

    pub(super) fn pointer_locked(&self) -> bool {
        match self.pending_lock {
            Some((on, _)) => on,
            None => self.running().is_some_and(|s| s.pointer_locked),
        }
    }

    /// Broadcast is on, or asked to be (shown so until the daemon says).
    pub(super) fn broadcasting(&self) -> bool {
        match self.pending_broadcast {
            Some((on, _)) => on,
            None => self.running().is_some_and(|s| !s.broadcast.is_empty()),
        }
    }

    pub(super) fn broadcast_stop_hint(&self) -> String {
        self.running().map(|status| status.broadcast_stop_hint.clone()).filter(|hint| !hint.is_empty())
            .unwrap_or_else(|| "Press Ctrl+Alt+Shift+Esc to stop broadcasting. The window cannot receive clicks during broadcast.".into())
    }

    /// Starts or stops broadcast, showing it at once.
    pub(super) fn ask_broadcast(&mut self, on: bool) {
        self.pending_broadcast = Some((on, self.now));
        self.busy.push(Job::Broadcast(on));
        self.link.broadcast(on);
    }

    /// Gaming mode: a label and three choices (in Settings).
    pub(super) fn gaming_row(&mut self, ui: &mut Ui, g: Rect) {
        let word = "Gaming mode";
        let lw = text_width(ui, word, regular(T_BODY)) + 12.0 + 26.0;
        let seg_w = 150.0f32.min(g.width() - lw);
        label_help(ui, g, Id::unique("gaming-label"), word, "gaming", true, seg_w + 12.0);
        let track = Rect::from_min_size(pos2(g.right() - seg_w, g.center().y - 13.0), vec2(seg_w, 26.0));
        let labels: Vec<&str> = GAMING.iter().map(|(_, l)| *l).collect();
        let pills = Pills {
            id: Id::unique("gaming"),
            track,
            cells: Pills::even(track, 3, 2.0),
            labels: &labels,
            names: GAMING.iter().map(|(_, l)| format!("Gaming mode: {l}")).collect(),
            chosen: GAMING.iter().position(|(v, _)| *v == self.prefs.gaming),
            enabled: vec![true; 3],
            role: Role::RadioButton,
            font: T_META,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            let v = GAMING[k].0;
            self.set_pref("gaming mode", &["gaming", v], |p| p.gaming = v.into());
        }
        let tip = Rect::from_min_size(g.min, vec2(lw, g.height()));
        ui.interact(tip, Id::unique("gaming-tip"), Sense::hover())
            .on_hover_text("On another computer, send raw mouse movement (for games). Auto: while it shows a fullscreen window.");
    }

    /// Settings → Appearance: System, Light or Dark. The window changes at
    /// once (a 200 ms cross-fade); `settings.toml` keeps the choice.
    pub(super) fn appearance_row(&mut self, ui: &mut Ui, g: Rect) {
        use crate::style::Appearance;
        let word = "Appearance";
        let lw = text_width(ui, word, regular(T_BODY)) + 12.0;
        let seg_w = 200.0f32.min(g.width() - lw);
        label_help(ui, g, Id::unique("appearance-label"), word, "appearance", true, seg_w + 12.0);
        let track = Rect::from_min_size(pos2(g.right() - seg_w, g.center().y - 13.0), vec2(seg_w, 26.0));
        let labels: Vec<&str> = Appearance::ALL.iter().map(|a| a.label()).collect();
        let pills = Pills {
            id: Id::unique("appearance"),
            track,
            cells: Pills::even(track, 3, 2.0),
            labels: &labels,
            names: Appearance::ALL.iter().map(|a| format!("Appearance: {}", a.label())).collect(),
            chosen: Appearance::ALL.iter().position(|a| *a == self.appearance()),
            enabled: vec![true; 3],
            role: Role::RadioButton,
            font: T_META,
        };
        if let Some(k) = pills.show(ui, self.arrowed) {
            self.set_appearance(Appearance::ALL[k]);
        }
    }

    /// Light, Dark, or the system's: shown at once and kept for this
    /// session even if it can't be saved; saved in `settings.toml` by the
    /// window itself.
    pub(super) fn set_appearance(&mut self, appearance: crate::style::Appearance) {
        self.appearance_choice.choose(appearance);
        self.prefs.appearance = appearance;
        self.busy.push(Job::Appearance(appearance));
        self.link.save_appearance(appearance);
    }

    /// The appearance shown: the one chosen in this window until the file
    /// says the same (then the file's, so `one-mk appearance` is followed).
    pub(super) fn appearance(&self) -> crate::style::Appearance {
        self.appearance_choice.shown(self.prefs.appearance)
    }
}

/// The folder received files go to (what Received files → Open opens), as
/// a person reads it: `~` for the home folder. Worked out once.
fn received_dir() -> Option<&'static str> {
    static DIR: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dirs = directories::UserDirs::new()?;
        let dir = crate::daemon::received_dir()?;
        Some(match dir.strip_prefix(dirs.home_dir()) {
            Ok(rest) if !cfg!(windows) => format!("~/{}", rest.display()),
            _ => dir.display().to_string(),
        })
    })
    .as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_transfer_reserves_its_recovery_line() {
        let transfer = one_mk_ctl::status::TransferStatus { id: 8, sending: true, peer: "Studio PC".into(), files: 1, done: 500_000, total: 9_000_000, rate: 0 };
        assert_eq!(transfer_height(&transfer), 82.0);
        assert_eq!(transfer_height(&one_mk_ctl::status::TransferStatus { rate: 100, ..transfer }), 64.0);
    }
}
