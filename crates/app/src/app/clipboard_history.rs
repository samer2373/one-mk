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
        let toggle = Rect::from_min_size(pos2(rect.left(), rect.top() + 30.0), vec2(rect.width(), 28.0));
        if switch(ui, toggle, Id::unique("history-enabled"), "Keep clipboard history", self.prefs.history_enabled, true).clicked() {
            let enabled = !self.prefs.history_enabled;
            self.set_pref("clipboard history", &["clipboard", "history", if enabled { "on" } else { "off" }], |prefs| prefs.history_enabled = enabled);
            self.clipboard_reveal = None;
            self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
        }
        let privacy = if !self.prefs.history_enabled {
            "Off: copies are not retained. Turning history on may retain unmarked secrets."
        } else if self.prefs.history_persist {
            "Saved unencrypted on disk. Previews are hidden until Show. Clear deletes saved items too. Unmarked secrets may be retained."
        } else {
            "Memory only; hidden until Show. Unmarked secrets may be retained. App skipping is X11-only."
        };
        let mut height = para(ui, pos2(rect.left(), rect.top() + 64.0), rect.width(), privacy, regular(T_META), INK2);
        let failure = history_failure(self.running());
        let text = history_message(online, self.clipboard_history.as_ref(), self.prefs.history_enabled, failure);
        // A saved-history failure next to a list: say so above it, or the
        // list reads as everything there is.
        if let Some(failure) = failure.filter(|failure| text != Some(*failure)) {
            let top = rect.top() + 72.0 + height;
            let shown = para(ui, pos2(rect.left(), top), rect.width(), failure, regular(T_META), ERROR);
            a11y_text(ui, Id::unique("history-failure"), Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), shown)), Role::Status, failure, Some(Live::Polite));
            height += 8.0 + shown;
        }
        let area = Rect::from_min_max(pos2(rect.left(), rect.top() + 72.0 + height), rect.max);
        if !area.is_positive() { return; }
        if let Some(text) = text {
            let color = if failure == Some(text) { ERROR } else { INK2 };
            para(ui, area.min, area.width(), text, regular(T_BODY), color);
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
                let (row, _) = ui.allocate_exact_size(vec2(area.width(), 44.0), Sense::hover());
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
                ui.painter().text(pos2(row.left(), row.top() + 35.0), Align2::LEFT_CENTER, shown, regular(T_META), INK2);
            }
        });
    }
}

/// The daemon's saved-history failure ("Could not load/save ..."), if any:
/// routine privacy notes in the same field are not failures.
fn history_failure(status: Option<&Status>) -> Option<&str> {
    status.map(|status| status.clipboard_history_note.as_str()).filter(|note| note.starts_with("Could not"))
}

/// What the history area says instead of a list, if anything. An empty list
/// while saved history failed to load is that failure, not "nothing yet".
fn history_message<'a>(online: bool, history: Option<&'a Result<Vec<crate::daemon::HistoryItem>, String>>, enabled: bool, failure: Option<&'a str>) -> Option<&'a str> {
    if !online { return Some("Start one·mk to see clipboard history."); }
    match history {
        None => Some("Loading history…"),
        Some(Err(error)) => Some(error.as_str()),
        Some(Ok(items)) if items.is_empty() && failure.is_some() => failure,
        Some(Ok(items)) if items.is_empty() && !enabled => Some("Clipboard history is disabled. No copies are being retained."),
        Some(Ok(items)) if items.is_empty() => Some("No clipboard items yet. Copy something, then choose Refresh."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_saved_history_is_not_shown_as_an_empty_history() {
        let error = "Could not load clipboard history: invalid header; clear it with one-mk clipboard history clear.";
        let status = Status { clipboard_history_enabled: true, clipboard_history_note: error.into(), ..Default::default() };
        let failure = history_failure(Some(&status));
        assert_eq!(failure, Some(error));
        let empty = Ok(Vec::new());
        assert_eq!(history_message(true, Some(&empty), true, failure), Some(error));
        let items = Ok(vec![crate::daemon::HistoryItem { id: 1, preview: String::new(), detail: String::new(), timestamp: 0 }]);
        assert_eq!(history_message(true, Some(&items), true, failure), None, "the list still shows; the failure goes above it");
        let routine = Status { clipboard_history_note: "Clipboard history is kept in memory only.".into(), ..Default::default() };
        assert_eq!(history_failure(Some(&routine)), None);
        assert_eq!(history_failure(None), None);
        assert_eq!(history_message(true, Some(&empty), true, None), Some("No clipboard items yet. Copy something, then choose Refresh."));
        assert_eq!(history_message(false, Some(&empty), true, failure), Some("Start one·mk to see clipboard history."));
    }
}
