use super::page_ui::{self, Page, ROW, Words, toggle_row};
use super::*;
use crate::widgets::Mark;

/// A copied item's row: what it was (hidden until Show), where from and when.
const ITEM: f32 = 56.0;

impl App {
    /// The clipboard's history as a card in column `col`: keeping it, keeping
    /// it after a restart, Refresh and Clear (two steps) at the label's end,
    /// what is kept in words, and the items, each a click to copy it here.
    pub(super) fn history_card(&mut self, ui: &mut Ui, page: &mut Page, col: usize) {
        let mut card = page.card(ui, col, "History");
        let online = self.running().is_some();
        let clear_label = if self.clipboard_clear_confirm { "Sure?" } else { "Clear" };
        let clear_width = button_width(ui, "Clear").max(button_width(ui, "Sure?"));
        let refresh_width = button_width(ui, "Refresh");
        let at = card.label_right();
        let refresh = Rect::from_min_size(pos2(at.x - clear_width - 8.0 - refresh_width, at.y - 14.0), vec2(refresh_width, 28.0));
        if button(ui, refresh, Id::unique("history-refresh"), "Refresh", Look::Quiet, self.prefs.history_enabled && online, Some("Refresh clipboard history")).clicked() {
            self.clipboard_reveal = None;
            self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
            self.clipboard_clear_confirm = false;
            self.clipboard_history = None;
            self.link.history();
        }
        let clear = Rect::from_min_size(pos2(at.x - clear_width, at.y - 14.0), vec2(clear_width, 28.0));
        if button(ui, clear, Id::unique("history-clear"), clear_label, Look::Quiet, online && self.prefs.history_enabled, Some("Clear deletes all memory and saved history. Click again to confirm; Refresh cancels.")).clicked() {
            if self.clipboard_clear_confirm { self.link.history_change(None); }
            self.clipboard_clear_confirm = !self.clipboard_clear_confirm;
            self.clipboard_reveal = None;
        }
        let row = card.row(ui, ROW);
        if toggle_row(ui, row, Id::unique("history-enabled"), &Words::new("Keep clipboard history"), Some("history-enabled"), self.prefs.history_enabled, true).clicked() {
            let enabled = !self.prefs.history_enabled;
            self.set_pref("clipboard history", &["clipboard", "history", if enabled { "on" } else { "off" }], |prefs| prefs.history_enabled = enabled);
            self.clipboard_reveal = None;
            self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
        }
        let row = card.row(ui, ROW);
        let enabled = self.prefs.history_enabled;
        if toggle_row(ui, row, Id::unique("history-persist"), &Words::new("Keep history after restart").dim(!enabled), Some("history-persist"), self.prefs.history_persist, enabled).clicked() {
            let enabled = !self.prefs.history_persist;
            self.set_pref("saved clipboard history", &["clipboard", "history", "persist", if enabled { "on" } else { "off" }], |prefs| prefs.history_persist = enabled);
        }
        let privacy = if !self.prefs.history_enabled {
            "History off · Nothing you copy is kept. If enabled, unmarked secrets may be retained."
        } else if self.prefs.history_persist {
            "Saved unencrypted on disk. Previews are hidden until Show. Clear deletes saved items too. Unmarked secrets may be retained."
        } else {
            "Memory only; hidden until Show. Unmarked secrets may be retained."
        };
        let inner = card.w - 2.0 * page_ui::CARD_PAD;
        let h = page_ui::text_h(ui, privacy, inner, regular(T_META));
        let row = card.row(ui, h + 24.0);
        para(ui, row.min + vec2(0.0, 12.0), inner, privacy, regular(T_META), pal().ink2);
        let failure = history_failure(self.running());
        let text = history_message(online, self.clipboard_history.as_ref(), self.prefs.history_enabled, failure);
        // A saved-history failure next to a list: say so above it, or the
        // list reads as everything there is.
        if let Some(failure) = failure.filter(|failure| text != Some(*failure)) {
            self.history_note(ui, &mut card, failure, true);
        }
        if let Some(text) = text {
            self.history_note(ui, &mut card, text, failure == Some(text));
            page.finish(ui, card);
            return;
        }
        let items = self.clipboard_history.as_ref().and_then(|result| result.as_ref().ok()).cloned().unwrap_or_default();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        for item in items {
            let age = now.saturating_sub(item.timestamp);
            let age = if age < 60 { "just now".into() } else if age < 3600 { format!("{} min ago", age / 60) } else if age < 86400 { format!("{} h ago", age / 3600) } else { format!("{} days ago", age / 86400) };
            let detail = format!("{} · {age}", item.detail);
            // A list row, as a system list: what was copied, and under
            // it where from and when; click the row to copy it here.
            let row = card.row(ui, ITEM);
            let revealed = self.clipboard_reveal.as_ref().filter(|(id, _)| *id == item.id).map(|(_, content)| content.as_str());
            let show = Rect::from_min_size(pos2(row.right() - 64.0, row.center().y - 14.0), vec2(64.0, 28.0));
            let copy = Rect::from_min_max(pos2(row.left() - 10.0, row.top() + 3.0), pos2(show.left() - 8.0, row.bottom() - 3.0));
            let preview = revealed.unwrap_or("Hidden · choose Show to see it");
            let accessible = format!("Copy clipboard item {} here: {preview}. {detail}", item.id);
            let id = Id::unique(("history-copy", item.id));
            let response = ui.interact(copy, id, widgets::sense(ui, Sense::click()));
            let hover = tween(ui.ctx(), id.with("hover"), f32::from(response.hovered()), 0.12, motion::out_cubic);
            let down = tween(ui.ctx(), id.with("press"), f32::from(response.is_pointer_button_down_on()), 0.08, motion::out_cubic);
            let p = ui.painter();
            p.rect_filled(copy, CornerRadius::same(8), mix(mix(Color32::TRANSPARENT, pal().surface2, hover), pal().surface2_hover, down));
            let tx = row.left();
            let tw = copy.right() - tx - 10.0;
            p.text(pos2(tx, row.top() + 18.0), Align2::LEFT_CENTER, fit(p, preview, &regular(T_BODY), tw), regular(T_BODY), if revealed.is_some() { pal().ink } else { pal().ink2 });
            p.text(pos2(tx, row.top() + 38.0), Align2::LEFT_CENTER, fit(p, &detail, &regular(T_META), tw), regular(T_META), pal().ink3);
            widgets::focus_ring(ui, &response, copy, 8);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            widgets::a11y(&response, Role::Button, &accessible, None);
            let response = response.on_hover_text("Copy this here");
            if response.clicked() {
                response.request_focus();
                self.link.history_change(Some(item.id));
            }
            if button(ui, show, Id::unique(("history-show", item.id)), if revealed.is_some() { "Hide" } else { "Show" }, Look::Quiet, true, Some("Reveal or hide this item's preview on screen")).clicked() {
                self.clipboard_reveal_epoch = self.clipboard_reveal_epoch.wrapping_add(1);
                if revealed.is_some() { self.clipboard_reveal = None; }
                else { self.link.history_reveal(item.id, self.clipboard_reveal_epoch); }
            }
        }
        page.finish(ui, card);
    }

    /// A line of words in the history card: what it says instead of a list,
    /// or a failure (with the error mark and `danger`).
    fn history_note(&self, ui: &Ui, card: &mut page_ui::Card, text: &str, failure: bool) {
        let inner = card.w - 2.0 * page_ui::CARD_PAD;
        let indent = if failure { page_ui::MARK_INDENT } else { 0.0 };
        let h = page_ui::text_h(ui, text, inner - indent, regular(T_BODY));
        let row = card.row(ui, h + 24.0);
        if failure {
            widgets::status_mark(ui.painter(), pos2(row.left() + 5.0, row.top() + 21.0), 8.0, Mark::Error);
        }
        para(ui, row.min + vec2(indent, 12.0), inner - indent, text, regular(T_BODY), if failure { pal().ink } else { pal().ink2 });
        a11y_text(ui, Id::unique(("history-state", text)), row, Role::Status, text, Some(Live::Polite));
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
    if !online { return Some("Start oneMK to see clipboard history."); }
    match history {
        None => Some("Loading history…"),
        Some(Err(error)) => Some(error.as_str()),
        Some(Ok(items)) if items.is_empty() && failure.is_some() => failure,
        // Off and empty: the line above already says nothing is kept.
        Some(Ok(items)) if items.is_empty() && !enabled => None,
        Some(Ok(items)) if items.is_empty() => Some("No clipboard items yet. Copy something, then choose Refresh."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_saved_history_is_not_shown_as_an_empty_history() {
        let error = "Could not load clipboard history. Use Clear to remove unreadable saved copies, then Refresh.";
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
        assert_eq!(history_message(false, Some(&empty), true, failure), Some("Start oneMK to see clipboard history."));
    }
}
