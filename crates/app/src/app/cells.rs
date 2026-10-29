//! Panel cells for the newer controls: this computer's switches (pointer
//! lock, lock all; gaming mode is drawn here for Advanced), a computer's quick actions (switch to it,
//! its clipboard), the clipboard and file settings, and transfers as they
//! run.

use super::*;
use crate::view;

const GAMING: [(&str, &str); 3] = [("off", "Off"), ("on", "On"), ("auto", "Auto")];

impl App {
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

    /// This computer: start at login, the pointer kept here, and locking
    /// every screen. Until another computer is paired (or while one·mk is
    /// off) only Start at login means anything, so only it shows.
    pub(super) fn controls(&mut self, ui: &mut Ui, c: Rect) {
        let row_h = 36.0;
        let top = c.top() - 8.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        let on = self.autostart.unwrap_or(false);
        let busy = self.busy.iter().any(|j| matches!(j, Job::Autostart(_)));
        let r = switch(ui, row(0), Id::unique("autostart"), "Start at login", on, self.autostart.is_some() && !busy);
        if self.autostart.is_none() {
            r.on_hover_text("Checking…");
        } else if r.clicked() {
            // Shown at once; the link reports what really happened.
            self.busy.push(Job::Autostart(!on));
            self.autostart = Some(!on);
            self.link.set_autostart(!on);
        }
        if self.running().is_none_or(|s| s.peers.is_empty()) {
            return;
        }
        let locked = self.pointer_locked();
        let r = switch(ui, row(1), Id::unique("lock-pointer"), "Keep the pointer here", locked, true);
        if r.clicked() {
            self.pending_lock = Some((!locked, self.now));
            self.busy.push(Job::LockPointer(!locked));
            self.link.lock_pointer(!locked);
        }
        r.on_hover_text("Edges lead nowhere while this is on.");
        for k in 1..3 {
            ui.painter().hline(c.x_range(), top + row_h * k as f32, Stroke::new(1.0, LINE));
        }
        // Every screen, locked at once.
        let l = row(2);
        ui.painter().text(l.left_center(), Align2::LEFT_CENTER, "Lock every screen", regular(T_BODY), INK);
        let label = "Lock all";
        let bw = button_width(ui, label) - 4.0;
        let b = Rect::from_min_size(pos2(l.right() - bw, l.center().y - 14.0), vec2(bw, 28.0));
        let busy = self.busy.contains(&Job::LockAll);
        if button(ui, b, Id::unique("lock-all"), label, Look::Quiet, !busy, Some("Lock this computer and every connected one")).clicked() {
            self.busy.push(Job::LockAll);
            self.link.lock_all();
        }
    }

    /// Gaming mode: a label and three choices (in Advanced).
    pub(super) fn gaming_row(&mut self, ui: &mut Ui, g: Rect) {
        let word = "Gaming mode";
        ui.painter().text(g.left_center(), Align2::LEFT_CENTER, word, regular(T_BODY), INK);
        let lw = text_width(ui, word, regular(T_BODY)) + 12.0;
        let seg_w = 150.0f32.min(g.width() - lw);
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
            radius: 6,
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
        let mut y = r.top() + 30.0;
        let shown = ((r.bottom() - y + 8.0) / 54.0).floor().max(1.0) as usize;
        for t in transfers.iter().take(shown) {
            self.transfer_row(ui, Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), 44.0)), t);
            y += 54.0;
        }
        if transfers.len() > shown {
            let more = format!("and {} more", transfers.len() - shown);
            ui.painter().text(pos2(r.left(), y.min(r.bottom() - 8.0)), Align2::LEFT_CENTER, more, regular(T_META), INK2);
        }
    }

    /// One transfer: what, how far, and a bar.
    pub(super) fn transfer_row(&self, ui: &Ui, r: Rect, t: &one_mk_ctl::status::TransferStatus) {
        let (title, detail, frac) = view::transfer_lines(t);
        let p = ui.painter();
        p.text(r.left_top(), Align2::LEFT_TOP, fit(p, &title, &regular(T_BODY), r.width()), regular(T_BODY), INK);
        let bar = Rect::from_min_size(pos2(r.left(), r.top() + 21.0), vec2(r.width(), 4.0));
        widgets::progress_bar(ui, bar, Id::unique(("transfer", t.id, &t.peer, t.sending)), frac);
        // Too long for the cell: leave out "x of y" before cutting anything.
        let detail = if text_width(ui, &detail, mono(T_META)) > r.width() && frac.is_some() {
            detail.split(" · ").enumerate().filter(|(k, _)| *k != 1).map(|(_, s)| s).collect::<Vec<_>>().join(" · ")
        } else {
            detail
        };
        p.text(pos2(r.left(), r.top() + 30.0), Align2::LEFT_TOP, fit(p, &detail, &mono(T_META), r.width()), mono(T_META), INK2);
        let label = format!("{title}: {detail}");
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
        let row_h = 30.0;
        let top = c.top() + 24.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        let p = self.prefs.clone();
        for (k, (key, label, on)) in [("text", "Text", p.clip_text), ("html", "Rich text (HTML)", p.clip_html), ("image", "Images", p.clip_image)].into_iter().enumerate() {
            if switch(ui, row(k), Id::unique(("clip", key)), label, on, true).clicked() {
                let v = if on { "off" } else { "on" };
                let flag = format!("--{key}");
                self.set_pref("what the clipboard shares", &["clipboard", &flag, v], |p| match key {
                    "text" => p.clip_text = !on,
                    "html" => p.clip_html = !on,
                    _ => p.clip_image = !on,
                });
            }
        }
        let r = switch(ui, row(3), Id::unique("allow-fetch"), "Others can get this clipboard", p.clip_allow_fetch, true);
        if r.clicked() {
            let v = if p.clip_allow_fetch { "off" } else { "on" };
            self.set_pref("who can get this clipboard", &["clipboard", "--allow-fetch", v], |p| p.clip_allow_fetch = !p.clip_allow_fetch);
        }
        r.on_hover_text("Any paired computer can copy this clipboard with Get clipboard, including passwords you copied.");
    }

    /// Notifications, the received files folder, and the clipboard's size limit.
    pub(super) fn files_settings(&mut self, ui: &mut Ui, c: Rect) {
        let row_h = 36.0;
        let top = c.top() - 8.0;
        let row = |k: usize| Rect::from_min_size(pos2(c.left(), top + row_h * k as f32), vec2(c.width(), row_h));
        let p = self.prefs.clone();
        if switch(ui, row(0), Id::unique("notify"), "Notify when files arrive", p.notify_files, true).clicked() {
            let v = if p.notify_files { "off" } else { "on" };
            self.set_pref("notifications", &["notify", v], |p| p.notify_files = !p.notify_files);
        }
        for k in 1..3 {
            ui.painter().hline(c.x_range(), top + row_h * k as f32, Stroke::new(1.0, LINE));
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
                _ => self.toast_error(format!("“{text}” isn't a size one·mk can use. Type a whole number of MB, like 25."), None, self.now),
            }
        }
    }
}
