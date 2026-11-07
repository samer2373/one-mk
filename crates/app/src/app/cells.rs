//! Panel cells for the newer controls: this computer's switches (pointer
//! lock, lock all; gaming mode is drawn here for Advanced), a computer's quick actions (switch to it,
//! its clipboard), the clipboard and file settings, and transfers as they
//! run.

use super::*;
use crate::view;

const GAMING: [(&str, &str); 3] = [("off", "Off"), ("on", "On"), ("auto", "Auto")];

fn transfer_height(transfer: &one_mk_ctl::status::TransferStatus) -> f32 {
    if transfer.rate == 0 { 60.0 } else { 42.0 }
}

#[cfg(any(debug_assertions, feature = "design-evidence", test))]
fn bounded_scroll(offset: f32, content_height: f32, viewport_height: f32) -> f32 {
    offset.clamp(0.0, (content_height - viewport_height).max(0.0))
}

#[derive(Default)]
pub(super) struct FilesUi {
    picker: Option<(String, Receiver<Option<Vec<PathBuf>>>)>,
    spatial: bool,
    options: bool,
    all_transfers: bool,
}

impl App {
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
        let transfers = self.running().map_or(0, |status| status.transfers.len());
        let recipients = self.running().map(|status| status.peers.clone()).unwrap_or_default();
        let width = rect.width() - 10.0;
        let chip_widths: Vec<f32> = recipients.iter().map(|peer| (text_width(ui, &format!("{}{}", peer.name, if peer.online { "" } else { " · Offline" }), semibold(T_BODY)) + 54.0).clamp(132.0, (width - 2.0 * PAD).min(340.0))).collect();
        let mut rows = 1;
        let mut used = 0.0;
        for chip_width in &chip_widths {
            if used > 0.0 && used + chip_width > width - 2.0 * PAD { rows += 1; used = 0.0; }
            used += chip_width + 8.0;
        }
        let shown_transfers = if self.files_ui.all_transfers { transfers } else { transfers.min(if rect.width() < 700.0 { 1 } else { 4 }) };
        let transfer_height = self.running().map_or(0.0, |status| status.transfers.iter().take(shown_transfers).map(transfer_height).sum::<f32>()) + shown_transfers.saturating_sub(1) as f32 * 8.0;
        let reconnect;
        let hint = if transfers > 0 && recipients.iter().all(|peer| !peer.online) { "Transfer details and reconnect steps are below." } else if !recipients.is_empty() && recipients.iter().all(|peer| !peer.online) {
            reconnect = format!("Open oneMK on {} to reconnect, then choose files or drop them here.", recipients[0].name);
            reconnect.as_str()
        } else if self.files_ui.spatial { "Drop files on a computer, or choose a recipient above." } else { "Choose a recipient to pick files, or drop files on its chip." };
        let hint_h = ui.painter().layout(hint.to_owned(), regular(T_META), INK2, width - 32.0).size().y;
        let head_h = 32.0 + rows as f32 * 44.0 + hint_h + 40.0 + if self.files_ui.spatial { 148.0 } else { 0.0 } + if transfers > 0 { 28.0 + transfer_height } else { 0.0 };
        // One inset (16) for every group on the page; each as tall as its rows.
        let settings_h = 2.0 * 16.0 + 28.0 + 5.0 * 36.0 + 6.0;
        let files_h = 2.0 * 16.0 + 28.0 + 4.0 * 36.0 + 6.0;
        let items = self.clipboard_history.as_ref().and_then(|h| h.as_ref().ok()).map_or(0, Vec::len);
        let listed = self.prefs.history_enabled && items > 0;
        let history_h = 2.0 * PAD + 104.0 + 44.0 + if listed { 50.0 * items as f32 } else { 30.0 };
        let privacy_h = ui.painter().layout("History off · Nothing you copy is kept. If enabled, unmarked secrets may be retained.".to_owned(), regular(T_META), INK2, width - 32.0).size().y;
        let history_h = if listed { history_h } else { 136.0 + privacy_h };
        let clipboard_h = if self.files_ui.options { settings_h } else { 80.0 };
        let total = head_h + M + clipboard_h + M + history_h + M + files_h;
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect).id_salt("files-panel"));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let scroll = egui::ScrollArea::vertical().id_salt("files-page").auto_shrink([false, false]);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let scroll = std::env::var("ONE_MK_APP_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()).map_or(scroll.clone(), |offset| scroll.vertical_scroll_offset(bounded_scroll(offset, total, rect.height())));
        scroll.show(&mut child, |ui| {
            let (all, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
            let head = Rect::from_min_size(all.min, vec2(all.width(), head_h));
            let clipboard = Rect::from_min_size(pos2(all.left(), head.bottom() + M), vec2(all.width(), clipboard_h));
            let history = Rect::from_min_size(pos2(all.left(), clipboard.bottom() + M), vec2(all.width(), history_h));
            let files = Rect::from_min_size(pos2(all.left(), history.bottom() + M), vec2(all.width(), files_h));
            for g in [head, clipboard, files, history] {
                widgets::group(ui, g, R_GROUP);
            }
            let inner = head.shrink(16.0);
            title_row(ui, inner, "Send files", 110.0);
            let spatial = Rect::from_min_size(pos2(inner.right() - 100.0, inner.top() - 4.0), vec2(100.0, 28.0));
            if button(ui, spatial, Id::unique("files-spatial"), if self.files_ui.spatial { "Hide desk" } else { "Show desk" }, Look::Quiet, true, None).clicked() { self.files_ui.spatial = !self.files_ui.spatial; }
            let (mut x, mut y) = (inner.left(), inner.top() + 32.0);
            let dropped: Vec<PathBuf> = ui.input(|input| input.raw.dropped_files.iter().map(|file| file.path().to_path_buf()).collect());
            let pointer = ui.input(|input| input.pointer.hover_pos());
            let mut drop_id = None;
            for (peer, chip_width) in recipients.iter().zip(&chip_widths) {
                if x > inner.left() && x + chip_width > inner.right() { x = inner.left(); y += 44.0; }
                let chip = Rect::from_min_size(pos2(x, y), vec2(*chip_width, 36.0));
                let label = format!("{}  {}{}", if peer.online { "↗" } else { "○" }, peer.name, if peer.online { "" } else { " · Offline" });
                let response = widgets::content_button(ui, chip, Id::unique(("files-recipient", &peer.id)), &label, Look::Quiet, peer.online && self.files_ui.picker.is_none(), Some(&format!("Send files to {}. Choose files, or drop files on this recipient.", peer.name)));
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
                if peer.online && pointer.is_some_and(|pointer| chip.contains(pointer)) {
                    if ui.input(|input| !input.raw.hovered_files.is_empty()) { ui.painter().rect_stroke(chip.expand(2.0), R_INNER, Stroke::new(2.0, SIGNAL), StrokeKind::Outside); }
                    if !dropped.is_empty() { drop_id = Some(peer.id.clone()); }
                }
                x += chip_width + 8.0;
            }
            y += 44.0;
            if recipients.is_empty() { para(ui, pos2(inner.left(), y - 38.0), inner.width(), "Pair a computer to send files to it.", regular(T_BODY), INK2); }
            y += para(ui, pos2(inner.left(), y), inner.width(), hint, regular(T_META), INK2) + 8.0;
            if self.files_ui.spatial {
                self.desk(ui, Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 140.0)), rect, self.now);
                y += 148.0;
            } else if !dropped.is_empty() {
                if let Some(index) = drop_id.and_then(|id| self.online_targets().into_iter().find(|index| self.scene.devices[*index].id == id)) { self.send_to(index, dropped); }
                else { self.drop_files(dropped, None, self.now); }
            }
            if transfers > 0 { self.files_head(ui, Rect::from_min_max(pos2(inner.left(), y), inner.max), "Transfers", ""); }
            if self.files_ui.options {
                self.clipboard_cell(ui, clipboard.shrink(16.0));
            } else {
                let inner = clipboard.shrink(16.0);
                title_row(ui, inner, "Clipboard", 140.0);
                let summary = format!("Text {} · Images {} · Others can fetch {}", if self.prefs.clip_text { "on" } else { "off" }, if self.prefs.clip_image { "on" } else { "off" }, if self.prefs.clip_allow_fetch { "on" } else { "off" });
                para(ui, inner.min + vec2(0.0, 28.0), inner.width(), &summary, regular(T_META), INK2);
            }
            let options = Rect::from_min_size(pos2(clipboard.right() - PAD - 124.0, clipboard.top() + PAD - 4.0), vec2(124.0, 28.0));
            if button(ui, options, Id::unique("files-clipboard-options"), if self.files_ui.options { "Hide options" } else { "Sharing options" }, Look::Quiet, true, None).clicked() { self.files_ui.options = !self.files_ui.options; }
            self.clipboard_history_cell(ui, history.shrink(16.0));
            self.files_settings(ui, files.shrink(16.0));
        });
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

    /// This computer: start at login, the pointer kept here, broadcast and
    /// locking every screen. Until another computer is paired (or while
    /// oneMK is off) only Start at login means anything, so only it shows;
    /// broadcast shows while a computer is connected (or it is on).
    pub(super) fn controls(&mut self, ui: &mut Ui, c: Rect) {
        // This computer's own switches; what acts on every computer at
        // once sits in the page's title row.
        let p = ui.painter().clone();
        // Beside this computer's own card (chosen, or nothing paired yet),
        // the name is already there: say what these are instead.
        let beside_me = self.running().is_some_and(|s| s.peers.is_empty()) || self.selected_id().as_deref() == Some("me");
        p.text(pos2(c.left(), c.top() + 10.0), Align2::LEFT_CENTER, if beside_me { "Startup & files" } else { "This computer" }, semibold(T_TITLE), INK);
        if !beside_me {
            // Off, the name isn't known yet: what it runs, at least, so the
            // card keeps its shape.
            let sub = self.running().map_or_else(|| os_name(me_os()).to_owned(), |s| format!("{} · {}", s.name, os_name(me_os())));
            p.text(pos2(c.left(), c.top() + 32.0), Align2::LEFT_CENTER, fit(&p, &sub, &regular(T_BODY), c.width()), regular(T_BODY), INK3);
        }
        let row_h = 40.0;
        let top = c.top() + if beside_me { 30.0 } else { 50.0 };
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        let on = self.autostart.unwrap_or(false);
        let busy = self.busy.iter().any(|j| matches!(j, Job::Autostart(_)));
        let r = switch_help(ui, row(0), Id::unique("autostart"), "Start at login", "autostart", on, self.autostart.is_some() && !busy);
        if self.autostart.is_none() {
            r.on_hover_text("Checking…");
        } else if r.clicked() {
            // Shown at once; the link reports what really happened.
            self.busy.push(Job::Autostart(!on));
            self.autostart = Some(!on);
            self.link.set_autostart(!on);
        }
        let mut next = 1;
        if self.running().is_some_and(|s| !s.peers.is_empty()) {
            let locked = self.pointer_locked();
            let r = switch_help(ui, row(next), Id::unique("lock-pointer"), "Keep the pointer here", "lock-pointer", locked, true);
            if r.clicked() {
                self.pending_lock = Some((!locked, self.now));
                self.busy.push(Job::LockPointer(!locked));
                self.link.lock_pointer(!locked);
            }
            r.on_hover_text("Edges lead nowhere while this is on.");
            next += 1;
        }
        if self.running().is_some() {
            let r1 = row(next);
            p.text(r1.left_center(), Align2::LEFT_CENTER, "Received files", regular(T_BODY), INK);
            let label = "Open";
            let bw = button_width(ui, label) - 4.0;
            let b = Rect::from_min_size(pos2(r1.right() - bw, r1.center().y - 14.0), vec2(bw, 28.0));
            let busy = self.busy.contains(&Job::Open);
            if button(ui, b, Id::unique("open-dl-desk"), label, Look::Quiet, !busy, Some("Open received files folder")).clicked() {
                self.busy.push(Job::Open);
                self.link.open_downloads();
            }
            next += 1;
        }
        for k in 1..next {
            p.hline(c.x_range(), top + row_h * k as f32, Stroke::new(1.0, LINE));
        }
    }

    /// Gaming mode: a label and three choices (in Advanced).
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

    /// Under the inspector: move the mouse and keyboard there (or back),
    /// and its clipboard.
    pub(super) fn quick_actions(&mut self, ui: &mut Ui, c: Rect, y: f32, peer: Option<&PeerStatus>) {
        let Some(s) = self.running().cloned() else { return };
        let away = s.controlling.is_some() || s.controlled_by.is_some();
        let switch_job = match peer {
            Some(p) if p.online && s.controlling.as_ref() != Some(&p.name) => Some((Job::Switch { to: p.id.clone(), name: p.name.clone() }, "Switch to it", format!("Move the mouse and keyboard to {}", p.name))),
            None if away => Some((Job::Switch { to: "here".into(), name: s.name.clone() }, "Switch back here", "Bring the mouse and keyboard back to this computer".to_owned())),
            _ => None,
        };
        if let Some((job, label, name)) = switch_job {
            let bw = button_width(ui, label) - 4.0;
            let r = Rect::from_min_size(pos2(c.left(), y), vec2(bw, 28.0));
            let busy = self.busy.contains(&job);
            if button(ui, r, Id::unique(("quick-switch", peer.map(|p| p.id.clone()))), label, Look::Quiet, !busy, Some(&name)).clicked() {
                self.busy.push(job.clone());
                if let Job::Switch { to, name } = job {
                    self.link.switch(&to, &name);
                }
            }
        }
        let Some(p) = peer.filter(|p| p.online) else { return };
        // The clipboard: two small buttons, named on hover and for screen readers.
        let mut x = c.right() - 28.0;
        for send in [false, true] {
            let r = Rect::from_min_size(pos2(x, y), vec2(28.0, 28.0));
            x -= 32.0;
            let job = Job::Clip { send, id: p.id.clone(), name: p.name.clone() };
            let busy = self.busy.contains(&job);
            let name = if send { format!("Send this clipboard to {}", p.name) } else { format!("Get {}'s clipboard", p.name) };
            if widgets::icon_button(ui, r, Id::unique(("quick-clip", send, &p.id)), &name, !busy, |pp, rr, col| widgets::draw_clip(pp, rr, col, send)).clicked() {
                self.busy.push(job.clone());
                self.link.clip(send, &p.id, &p.name);
            }
        }
        let label = "Clipboard";
        let lw = text_width(ui, label, regular(T_META));
        if x + 28.0 - 8.0 - lw > c.left() + 110.0 {
            ui.painter().text(pos2(x + 28.0 - 6.0, y + 14.0), Align2::RIGHT_CENTER, label, regular(T_META), INK2);
        }
    }

    /// The Files tab's first cell: how to send, then transfers as they run.
    pub(super) fn files_head(&mut self, ui: &mut Ui, r: Rect, title: &str, sub: &str) {
        let transfers = self.running().map(|s| s.transfers.clone()).unwrap_or_default();
        if transfers.is_empty() {
            let th = para(ui, r.left_top(), r.width(), title, semibold(T_TITLE), INK);
            let sh = para(ui, r.left_top() + vec2(0.0, th + 6.0), r.width(), sub, regular(T_BODY), INK2);
            a11y_text(ui, Id::unique("headline"), Rect::from_min_size(r.min, vec2(r.width(), th + sh + 6.0)), Role::Status, &format!("{title}. {sub}"), Some(Live::Polite));
            return;
        }
        let head = if transfers.len() == 1 { "1 transfer".to_owned() } else { format!("{} transfers", transfers.len()) };
        para(ui, r.left_top(), r.width(), &head, semibold(T_TITLE), INK);
        let mut y = r.top() + 28.0;
        let shown = if self.files_ui.all_transfers { transfers.len() } else { transfers.len().min(if r.width() < 658.0 { 1 } else { 4 }) };
        if transfers.len() > shown || self.files_ui.all_transfers {
            let label = if self.files_ui.all_transfers { "Show less".to_owned() } else { format!("Show all {}", transfers.len()) };
            let width = button_width(ui, &label);
            let rect = Rect::from_min_size(pos2(r.right() - width, r.top() - 4.0), vec2(width, 28.0));
            if button(ui, rect, Id::unique("files-all-transfers"), &label, Look::Quiet, true, None).clicked() { self.files_ui.all_transfers = !self.files_ui.all_transfers; }
        }
        for t in transfers.iter().take(shown) {
            let height = transfer_height(t);
            self.transfer_row(ui, Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), height)), t);
            y += height + 8.0;
        }
    }

    /// One transfer: what, how far, and a bar.
    pub(super) fn transfer_row(&self, ui: &Ui, r: Rect, t: &one_mk_ctl::status::TransferStatus) {
        let (title, detail, frac) = view::transfer_lines(t);
        let p = ui.painter();
        p.text(r.left_top(), Align2::LEFT_TOP, fit(p, &title, &regular(T_BODY), r.width()), regular(T_BODY), INK);
        let bar = Rect::from_min_size(pos2(r.left(), r.bottom() - 4.0), vec2(r.width(), 4.0));
        if t.rate == 0 {
            p.rect_filled(bar, 1.0, WARN.gamma_multiply(0.22));
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac.unwrap_or(0.0), bar.height())), 1.0, WARN);
        } else {
            widgets::progress_bar(ui, bar, Id::unique(("transfer", t.id, &t.peer, t.sending)), frac);
        }
        // Too long for the cell: leave out "x of y" before cutting anything.
        let detail = if text_width(ui, &detail, mono(T_META)) > r.width() && frac.is_some() {
            detail.split(" · ").enumerate().filter(|(k, _)| *k != 1).map(|(_, s)| s).collect::<Vec<_>>().join(" · ")
        } else {
            detail
        };
        p.text(pos2(r.left(), r.top() + 17.0), Align2::LEFT_TOP, fit(p, &detail, &mono(T_META), r.width()), mono(T_META), if t.rate == 0 { WARN } else { INK2 });
        let offline = self.running().is_some_and(|status| status.peers.iter().any(|peer| peer.name == t.peer && !peer.online));
        let recovery = if t.rate == 0 && offline { format!("Open oneMK on {} to reconnect.", t.peer) } else if t.rate == 0 { "Connected · waiting for transfer progress.".into() } else { String::new() };
        if !recovery.is_empty() {
            p.text(pos2(r.left(), r.top() + 34.0), Align2::LEFT_TOP, fit(p, &recovery, &regular(T_META), r.width()), regular(T_META), INK2);
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
    pub(super) fn clipboard_cell(&mut self, ui: &mut Ui, c: Rect) {
        title_row(ui, c, "Clipboard", 0.0);
        // 36-point rows (the 6-point grid), a hairline between them.
        let row_h = 36.0;
        let top = c.top() + 28.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        for k in 1..5 {
            ui.painter().hline(c.x_range(), top + row_h * k as f32, Stroke::new(1.0, LINE));
        }
        let p = self.prefs.clone();
        for (k, (key, label, on)) in [("text", "Text", p.clip_text), ("html", "Rich text (HTML)", p.clip_html), ("image", "Images", p.clip_image)].into_iter().enumerate() {
            if switch_help(ui, row(k), Id::unique(("clip", key)), label, &format!("clip-{key}"), on, true).clicked() {
                let v = if on { "off" } else { "on" };
                let flag = format!("--{key}");
                self.set_pref("what the clipboard shares", &["clipboard", &flag, v], |p| match key {
                    "text" => p.clip_text = !on,
                    "html" => p.clip_html = !on,
                    _ => p.clip_image = !on,
                });
            }
        }
        let r = switch_help(ui, row(3), Id::unique("allow-fetch"), "Others can get this clipboard", "allow-fetch", p.clip_allow_fetch, true);
        if r.clicked() {
            let v = if p.clip_allow_fetch { "off" } else { "on" };
            self.set_pref("who can get this clipboard", &["clipboard", "--allow-fetch", v], |p| p.clip_allow_fetch = !p.clip_allow_fetch);
        }
        let sensitive_label = if c.width() < 220.0 { "Skip secrets" } else { "Skip sensitive copies" };
        let sensitive = switch_help(ui, row(4), Id::unique("clip-sensitive"), sensitive_label, "clip-sensitive", p.clip_skip_sensitive, true);
        if sensitive.clicked() {
            self.set_pref("sensitive copies", &["clipboard", "--skip-sensitive", if p.clip_skip_sensitive { "off" } else { "on" }], |prefs| prefs.clip_skip_sensitive = !p.clip_skip_sensitive);
        }
    }

    /// Notifications, the received files folder, and the clipboard's size limit.
    pub(super) fn files_settings(&mut self, ui: &mut Ui, c: Rect) {
        // Titled and spaced as the Clipboard card beside it.
        title_row(ui, c, "Files", 0.0);
        // 36-point rows (the 6-point grid), a hairline between them.
        let row_h = 36.0;
        let top = c.top() + 28.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        for k in 1..4 {
            ui.painter().hline(c.x_range(), top + row_h * k as f32, Stroke::new(1.0, LINE));
        }
        let p = self.prefs.clone();
        if switch_help(ui, row(0), Id::unique("notify"), "Notify when files arrive", "notify", p.notify_files, true).clicked() {
            let v = if p.notify_files { "off" } else { "on" };
            self.set_pref("notifications", &["notify", v], |p| p.notify_files = !p.notify_files);
        }
        let r1 = row(1);
        ui.painter().text(r1.left_center(), Align2::LEFT_CENTER, "Received files", regular(T_BODY), INK);
        let label = "Open";
        let bw = button_width(ui, label) - 4.0;
        let b = Rect::from_min_size(pos2(r1.right() - bw, r1.center().y - 14.0), vec2(bw, 28.0));
        let busy = self.busy.contains(&Job::Open);
        if button(ui, b, Id::unique("open-dl"), label, Look::Quiet, !busy, Some("Open received files folder")).clicked() {
            self.busy.push(Job::Open);
            self.link.open_downloads();
        }
        // The size limit: a number to type, in MB.
        let r = row(2);
        ui.painter().text(r.left_center(), Align2::LEFT_CENTER, "Largest clipboard", regular(T_BODY), INK);
        let unit = "MB";
        let uw = text_width(ui, unit, regular(T_META)) + 8.0;
        let fr = Rect::from_min_size(pos2(r.right() - 64.0 - uw, r.center().y - 13.0), vec2(64.0, 26.0));
        ui.painter().text(pos2(r.right(), r.center().y), Align2::RIGHT_CENTER, unit, regular(T_META), INK2);
        let cur = p.clip_max_mb.to_string();
        if let Some(text) = self.edit_field(ui, fr, "clip-max", &cur, "25", "Largest clipboard to send or accept, in MB", true, true) {
            match text.parse::<u32>() {
                Ok(n) if (1..=4096).contains(&n) => {
                    let v = n.to_string();
                    self.set_pref("the clipboard size limit", &["clipboard", "--max-mb", &v], |p| p.clip_max_mb = n);
                }
                _ => self.toast_error(format!("“{text}” isn't a size oneMK can use. Type a whole number of MB, like 25."), None, self.now),
            }
        }
        if switch_help(ui, row(3), Id::unique("file-drop-zones"), "Drop files at screen edges", "file-drop-zones", p.drop_zones, true).clicked() {
            self.set_pref("file drop zones", &["drag", if p.drop_zones { "--off" } else { "--on" }], |prefs| prefs.drop_zones = !p.drop_zones);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_scroll_never_leaves_the_content() {
        assert_eq!(bounded_scroll(600.0, 700.0, 500.0), 200.0);
        assert_eq!(bounded_scroll(600.0, 200.0, 500.0), 0.0);
        assert_eq!(bounded_scroll(-12.0, 700.0, 500.0), 0.0);
    }

    #[test]
    fn stalled_transfer_reserves_its_recovery_line() {
        let transfer = one_mk_ctl::status::TransferStatus { id: 8, sending: true, peer: "Studio PC".into(), files: 1, done: 500_000, total: 9_000_000, rate: 0 };
        assert_eq!(transfer_height(&transfer), 60.0);
        assert_eq!(transfer_height(&one_mk_ctl::status::TransferStatus { rate: 100, ..transfer }), 42.0);
    }
}
