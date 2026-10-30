use super::*;

impl App {
    pub(super) fn clipboard_history_cell(&mut self, ui: &mut Ui, rect: Rect) {
        title_row(ui, rect, "History", 125.0);
        let refresh = Rect::from_min_size(pos2(rect.right() - 124.0, rect.top() - 4.0), vec2(72.0, 28.0));
        if button(ui, refresh, Id::unique("history-refresh"), "Refresh", Look::Quiet, true, Some("Refresh clipboard history")).clicked() {
            self.clipboard_reveal = None;
            self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
            self.clipboard_clear_confirm = false;
            self.clipboard_history = None;
            self.link.history();
        }
        let clear = Rect::from_min_size(pos2(rect.right() - 48.0, rect.top() - 4.0), vec2(48.0, 28.0));
        let online = self.running().is_some();
        let clear_label = if self.clipboard_clear_confirm { "Sure?" } else { "Clear" };
        if button(ui, clear, Id::unique("history-clear"), clear_label, Look::Quiet, online, Some("Clear deletes all memory and saved history. Click again to confirm; Refresh cancels.")).clicked() {
            if self.clipboard_clear_confirm { self.link.history_change(None); }
            self.clipboard_clear_confirm = !self.clipboard_clear_confirm;
            self.clipboard_reveal = None;
        }
        let privacy = if !self.prefs.history_enabled {
            "History is off by default for privacy, including upgrades. Enable history_max in settings.toml to retain copies."
        } else if self.prefs.history_persist {
            "Saved unencrypted on disk. Previews are hidden until Show. Clear deletes saved items too. Unmarked secrets may be retained."
        } else {
            "Kept in memory only. Previews are hidden until Show. Native sensitivity hints are checked, but unmarked secrets may be retained. App skipping is X11-only, not Wayland-native."
        };
        let height = para(ui, pos2(rect.left(), rect.top() + 30.0), rect.width(), privacy, regular(T_META), INK2);
        let area = Rect::from_min_max(pos2(rect.left(), rect.top() + 38.0 + height), rect.max);
        if !area.is_positive() { return; }
        let text = if !online { Some("Start one·mk to see clipboard history.") } else {
            match &self.clipboard_history {
                None => Some("Loading history…"),
                Some(Err(error)) => Some(error.as_str()),
                Some(Ok(items)) if items.is_empty() && !self.prefs.history_enabled => Some("Clipboard history is disabled. No copies are being retained."),
                Some(Ok(items)) if items.is_empty() => Some("No clipboard items yet. Copy something, then choose Refresh."),
                _ => None,
            }
        };
        if let Some(text) = text {
            para(ui, area.min, area.width(), text, regular(T_BODY), INK2);
            a11y_text(ui, Id::unique("history-state"), area, Role::Status, text, Some(Live::Polite));
            return;
        }
        let items = self.clipboard_history.as_ref().and_then(|result| result.as_ref().ok()).cloned().unwrap_or_default();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let mut child = ui.new_child(UiBuilder::new().max_rect(area).id_salt("history-list"));
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(area.height()).show(&mut child, |ui| {
            for item in items {
                let age = now.saturating_sub(item.timestamp);
                let age = if age < 60 { "just now".into() } else if age < 3600 { format!("{} min ago", age / 60) } else { format!("{} h ago", age / 3600) };
                let detail = format!("{} · {age}", item.detail);
                let (row, _) = ui.allocate_exact_size(vec2(area.width(), 50.0), Sense::hover());
                let revealed = self.clipboard_reveal.as_ref().filter(|(id, _)| *id == item.id).map(|(_, content)| content.as_str());
                let copy = Rect::from_min_size(row.min, vec2(row.width() - 64.0, 28.0));
                let preview = revealed.unwrap_or("[hidden; choose Show]");
                let label = fit(ui.painter(), preview, &semibold(T_BODY), copy.width() - 28.0);
                let accessible = format!("Copy clipboard item {} here: {preview}. {detail}", item.id);
                let response = button(ui, copy, Id::unique(("history-copy", item.id)), &label, Look::Quiet, true, Some(&accessible)).on_hover_text(&accessible);
                if response.clicked() {
                    response.request_focus();
                    self.link.history_change(Some(item.id));
                }
                let show = Rect::from_min_size(pos2(row.right() - 60.0, row.top()), vec2(60.0, 28.0));
                if button(ui, show, Id::unique(("history-show", item.id)), if revealed.is_some() { "Hide" } else { "Show" }, Look::Quiet, true, Some("Reveal or hide this item's preview on screen")).clicked() {
                    self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
                    if revealed.is_some() { self.clipboard_reveal = None; }
                    else { self.link.history_reveal(item.id, self.clipboard_reveal_epoch); }
                }
                let shown = fit(ui.painter(), &detail, &regular(T_META), row.width());
                ui.painter().text(pos2(row.left(), row.top() + 37.0), Align2::LEFT_CENTER, shown, regular(T_META), INK2);
            }
        });
    }
}
