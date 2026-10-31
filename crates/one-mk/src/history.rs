use crate::control::{self, Request};
use anyhow::{Result, bail};
use one_mk_clipboard::{Clipboard, history::Entry};
use one_mk_ctl::client::HistoryAction;
use std::path::Path;

#[derive(clap::Subcommand, Debug)]
pub enum ClipboardCommand {
    /// Recall local clipboard history. Kept in memory unless history_persist is enabled.
    History {
        #[command(subcommand)]
        operation: Option<Operation>,
        /// Print machine-readable JSON; text content is included only by show.
        #[arg(long, global = true)]
        json: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum Operation {
    /// Retain clipboard copies in memory (explicit privacy opt-in).
    On,
    /// Stop retaining clipboard copies and clear retained history.
    Off,
    /// List newest first, with stable item numbers.
    List,
    /// Print full text or HTML (terminal controls escaped); --json preserves exact text.
    Show { #[arg(value_parser = clap::value_parser!(u64).range(1..u64::MAX), help = "Item number from one-mk clipboard history")] number: u64 },
    /// Restore an item here only; does not send it to another computer.
    Copy { #[arg(value_parser = clap::value_parser!(u64).range(1..u64::MAX), help = "Item number from one-mk clipboard history")] number: u64 },
    /// Delete one item by its number in list.
    Delete { #[arg(value_parser = clap::value_parser!(u64).range(1..u64::MAX), help = "Item number from one-mk clipboard history")] number: u64 },
    /// Forget every item, including saved history when enabled.
    Clear,
}

impl ClipboardCommand {
    pub async fn run(self, dir: &Path) -> Result<()> {
        let Self::History { operation, json } = self;
        if matches!(operation, Some(Operation::On | Operation::Off)) {
            let enabled = matches!(operation, Some(Operation::On));
            crate::settings::Settings::update(dir, |settings| {
                settings.clipboard.history_max = if enabled { 20 } else { 0 };
                Ok(())
            })?;
            crate::settings::apply(dir).await?;
            if json { println!("{{\"enabled\":{enabled}}}"); }
            else { println!("Clipboard history is {}.", if enabled { "on; retained in memory unless persistence is explicitly enabled" } else { "off" }); }
            return Ok(());
        }
        let action = match operation.unwrap_or(Operation::List) {
            Operation::On | Operation::Off => unreachable!(),
            Operation::List => HistoryAction::List, Operation::Show { number } => HistoryAction::Show(number),
            Operation::Copy { number } => HistoryAction::Copy(number), Operation::Delete { number } => HistoryAction::Delete(number),
            Operation::Clear => HistoryAction::Clear,
        };
        let Some(reply) = control::request(dir, Request::ClipHistory { action, json }).await
            .map_err(|_| anyhow::anyhow!("Could not reach one-mk; restart it and try the history command again"))? else {
            bail!("one-mk is not running; start it with `one-mk run`");
        };
        if let Some(error) = reply.strip_prefix("error ") { bail!("{}", error.trim()); }
        let Some(body) = reply.strip_prefix("ok\n") else { bail!("Could not read clipboard history; restart one-mk and try again"); };
        print!("{body}");
        Ok(())
    }
}

pub async fn handle(cb: &Clipboard, action: HistoryAction, json: bool) -> Result<String> {
    let durable = matches!(action, HistoryAction::Delete(_) | HistoryAction::Clear);
    if let HistoryAction::Copy(id) = action {
        let entry = cb.history().lock().unwrap_or_else(|error| error.into_inner()).get(id)
            .ok_or_else(|| anyhow::anyhow!("Clipboard item {id} is no longer in history; run one-mk clipboard history list"))?;
        if entry.data.len() > cb.filter().max_bytes() { bail!("This item is over the clipboard size limit; raise it with one-mk clipboard --max-mb N"); }
        cb.write_from((*entry.data).clone(), entry.origin, false).await
            .map_err(|_| anyhow::anyhow!("Could not restore the clipboard; check clipboard access and try again"))?;
        return Ok(if json { "{\"copied\":true}\n".into() } else { "Copied the item to this computer only.\n".into() });
    }
    let shared = cb.history().clone();
    let response = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut history = shared.lock().unwrap_or_else(|error| error.into_inner());
        match action {
            HistoryAction::List => {
                let entries = history.list();
                if json { return Ok(format!("[{}]\n", entries.iter().map(entry_json).collect::<Vec<_>>().join(","))); }
                if history.disabled() { return Ok("Clipboard history is off. Enable with `one-mk clipboard history on`.\n".into()); }
                if entries.is_empty() { return Ok("Clipboard history is empty.\n".into()); }
                Ok(format!("Item\tType\tSize\tRecorded\tOrigin\tShow content\n{}", entries.iter().map(|entry| format!("{}\t{}\t{}\t{}\tfrom {}\tone-mk clipboard history show {}\n", entry.id, entry.kind(), crate::clip::mb(entry.data.len()), crate::readable_time(entry.timestamp), entry.origin, entry.id)).collect::<String>()))
            }
            HistoryAction::Show(id) => {
                let entry = history.get(id).ok_or_else(|| anyhow::anyhow!("Clipboard item {id} is no longer in history; run one-mk clipboard history list"))?;
                let content = entry.data.text.as_ref().or(entry.data.html.as_ref()).cloned().unwrap_or_else(|| format!("{} ({})", entry.preview(), crate::clip::mb(entry.data.len())));
                Ok(if json { format!("{{\"content\":{}}}\n", quote(&content)) } else { format!("{}\n", terminal_text(&content)) })
            }
            HistoryAction::Delete(id) => {
                if !history.delete(id) { bail!("Clipboard item {id} is no longer in history; run one-mk clipboard history list"); }
                if let Some(error) = &history.error { bail!("{error}"); }
                Ok(if json { "{\"deleted\":true}\n".into() } else { "Deleted the clipboard item.\n".into() })
            }
            HistoryAction::Clear => {
                history.clear();
                if let Some(error) = &history.error { bail!("{error}"); }
                Ok(if json { "{\"cleared\":true}\n".into() } else { "Cleared clipboard history.\n".into() })
            }
            HistoryAction::Copy(_) => bail!("Could not restore clipboard history; try again"),
        }
    }).await.map_err(|_| anyhow::anyhow!("Could not read clipboard history; restart one-mk and try again"))??;
    if durable { cb.flush_history().await?; }
    Ok(response)
}

fn entry_json(entry: &Entry) -> String {
    format!("{{\"id\":{},\"kind\":{},\"preview\":{},\"size\":{},\"timestamp\":{},\"origin\":{}}}",
        entry.id, quote(entry.kind()), quote("[hidden; use Show]"), entry.data.len(), entry.timestamp, quote(&entry.origin))
}

fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""), '\\' => result.push_str("\\\\"),
            ch if ch.is_control() || invisible(ch) => {
                for unit in ch.encode_utf16(&mut [0; 2]) { result.push_str(&format!("\\u{unit:04x}")); }
            }
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn invisible(ch: char) -> bool {
    !ch.is_control() && one_mk_clipboard::history::invisible(ch)
}

fn terminal_text(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if (ch.is_control() && ch != '\n' && ch != '\t') || invisible(ch) { result.extend(ch.escape_unicode()); }
        else { result.push(ch); }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_are_masked_until_explicit_show() {
        let (clipboard, _) = Clipboard::memory();
        clipboard.write(one_mk_clipboard::ClipData { text: Some("dummy-secret".into()), ..Default::default() }).await.unwrap();
        for json in [false, true] {
            assert!(!handle(&clipboard, HistoryAction::List, json).await.unwrap().contains("dummy-secret"));
        }
        assert!(handle(&clipboard, HistoryAction::Show(1), false).await.unwrap().contains("dummy-secret"));
    }

    #[test]
    fn terminal_and_json_never_emit_hostile_controls() {
        let hostile = "safe\u{1b}[2J\r\0\u{202e}evil\n日本語\"\\";
        assert_eq!(terminal_text(hostile), "safe\\u{1b}[2J\\u{d}\\u{0}\\u{202e}evil\n日本語\"\\");
        assert_eq!(quote(hostile), "\"safe\\u001b[2J\\u000d\\u0000\\u202eevil\\u000a日本語\\\"\\\\\"");
        for scalar in 0..=0x10ffff {
            if let Some(ch) = char::from_u32(scalar) {
                let encoded = quote(&ch.to_string());
                assert!(!encoded.chars().any(|ch| ch.is_control() || invisible(ch)));
            }
        }
    }

    #[tokio::test]
    async fn copying_is_local_and_errors_are_actionable() {
        let (clipboard, backend) = Clipboard::memory();
        clipboard.write(one_mk_clipboard::ClipData { text: Some("first".into()), ..Default::default() }).await.unwrap();
        let id = clipboard.history().lock().unwrap().list()[0].id;
        clipboard.write(one_mk_clipboard::ClipData { text: Some("second".into()), ..Default::default() }).await.unwrap();
        assert_eq!(handle(&clipboard, HistoryAction::Copy(id), true).await.unwrap(), "{\"copied\":true}\n");
        assert_eq!(backend.text().as_deref(), Some("first"));
        assert_eq!(clipboard.history().lock().unwrap().len(), 2);
        assert!(handle(&clipboard, HistoryAction::Show(99), false).await.unwrap_err().to_string().contains("run one-mk clipboard history list"));
        assert_eq!(handle(&clipboard, HistoryAction::Clear, true).await.unwrap(), "{\"cleared\":true}\n");
        assert_eq!(handle(&clipboard, HistoryAction::List, true).await.unwrap(), "[]\n");
    }
}
