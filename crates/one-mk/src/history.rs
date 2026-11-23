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
    /// Keep history across restarts, or remove its saved copy.
    Persist { #[arg(value_parser = crate::settings::parse_on_off, action = clap::ArgAction::Set)] enabled: bool },
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
        if let Some(Operation::Persist { enabled }) = operation {
            let history_on = crate::settings::Settings::update(dir, |settings| {
                settings.clipboard.history_persist = enabled;
                Ok(settings.clipboard.history_max > 0)
            })?;
            if enabled {
                crate::settings::apply(dir).await?;
                println!("History after restart: on.");
                println!("Warning: the saved history is plain text, not encrypted: {} (readable only by you, but by every program you run).", saved_file(dir).display());
                println!("Copies a password manager marked as sensitive are never saved. Turn this off again with `one-mk clipboard history persist off`.");
                if !history_on { println!("History itself is off, so nothing is kept yet; turn it on with `one-mk clipboard history on`."); }
            } else {
                apply_and_wait_for_delete(dir).await?;
                println!("History after restart: off. No saved history is left on disk.");
            }
            return Ok(());
        }
        if matches!(operation, Some(Operation::On | Operation::Off)) {
            let enabled = matches!(operation, Some(Operation::On));
            let note = crate::settings::Settings::update(dir, |settings| {
                turn(&mut settings.clipboard, enabled);
                Ok(no_room(&settings.clipboard))
            })?;
            if enabled { crate::settings::apply(dir).await?; } else { apply_and_wait_for_delete(dir).await?; }
            if json { println!("{{\"enabled\":{enabled}}}"); }
            else { println!("Clipboard history is {}.", if enabled { "on; retained in memory unless persistence is explicitly enabled" } else { "off" }); }
            if let Some(note) = note.filter(|_| enabled) { eprintln!("{note}"); }
            return Ok(());
        }
        let action = match operation.unwrap_or(Operation::List) {
            Operation::On | Operation::Off | Operation::Persist { .. } => unreachable!(),
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

/// What `history on` adds when the size cap in `settings.toml` is 0: nothing
/// could be kept.
const NO_ROOM: &str = "Warning: history_max_mb is 0 in settings.toml, so nothing will be kept; set it above 0 (16 is the default).";

fn no_room(clipboard: &crate::settings::ClipSettings) -> Option<&'static str> {
    (clipboard.history_max_mb == 0).then_some(NO_ROOM)
}

/// `history on` or `off`. The number of items is the user's own setting
/// (`history_max`, up to 1000): `on` only gives it a value (20) when it is 0.
fn turn(clipboard: &mut crate::settings::ClipSettings, on: bool) {
    if !on {
        clipboard.history_max = 0;
    } else if clipboard.history_max == 0 {
        clipboard.history_max = 20;
    }
}

/// Where saved history lives (see `ClipSync::apply`).
fn saved_file(dir: &Path) -> std::path::PathBuf {
    dir.join("clipboard-history.bin")
}

/// Asks a running daemon to apply settings that stop saving history, and does
/// not return until the saved file is gone: the daemon deletes it a moment
/// later on another thread, and "Applied" before that would be a promise
/// about a secret still sitting on disk. The daemon confirms with a barrier
/// (`flush`) that answers only once every save it had started, including a
/// first one that has not made the file yet, is finished and removed. With
/// no daemon running the file, and any unfinished save a crash left behind,
/// is deleted here, holding the daemon's own lock so none starts meanwhile.
async fn apply_and_wait_for_delete(dir: &Path) -> Result<()> {
    let file = saved_file(dir);
    // `Settings`, not `Reload`: a reload answers ok even when the settings file
    // could not be read and the old settings stay in force, and then nothing
    // below would have switched anything off.
    let applied = control::request(dir, Request::Settings).await?;
    if let Some(reply) = applied.as_deref() && !reply.starts_with("ok") {
        bail!("one-mk could not apply it, so history may still be kept: {}", reply.trim().trim_start_matches("error "));
    }
    let running = applied.is_some();
    if !running {
        let Some(_instance) = control::InstanceLock::try_acquire(dir)? else {
            bail!("one-mk is starting; run this command again in a moment");
        };
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => bail!("Could not delete the saved clipboard history {}: {error}", file.display()),
        }
        if dir.exists() && let Err(error) = one_mk_clipboard::history::clean_unfinished(dir) {
            bail!("Could not delete an unfinished saved clipboard history in {}: {error}", dir.display());
        }
        // The deletions themselves must survive a power loss, as the daemon's own do.
        if dir.exists() {
            one_mk_net::sync_dir(dir).map_err(|error| anyhow::anyhow!("Could not flush the deletion of the saved clipboard history in {}: {error:#}", dir.display()))?;
        }
        println!("one-mk is not running; the change takes effect the next time it starts.");
        return Ok(());
    }
    if crate::settings::Settings::load_or_default(dir).clipboard.history_persist {
        // History was switched off but the file stays (now empty): clearing makes the daemon write it out before answering.
        let cleared = control::request(dir, Request::ClipHistory { action: HistoryAction::Clear, json: false }).await?;
        if cleared.as_deref().is_some_and(|reply| reply.starts_with("error ")) { bail!("Could not clear the saved clipboard history; run `one-mk clipboard history clear`"); }
        return Ok(());
    }
    let flushed = control::request(dir, Request::ClipHistory { action: HistoryAction::Flush, json: false }).await?;
    match flushed.as_deref() {
        Some(reply) if reply.starts_with("ok\n") => {
            if file.exists() { bail!("The saved clipboard history is still on disk at {}; delete it by hand, or run `one-mk clipboard history clear`", file.display()); }
            return Ok(());
        }
        Some(reply) if reply.starts_with("error ") && !reply.contains("unknown request") => bail!("{}", reply.trim_start_matches("error ").trim()),
        // An older one-mk without the barrier: watch for the file to go.
        _ => {}
    }
    for _ in 0..100 {
        if !file.exists() { return Ok(()); }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    bail!("The saved clipboard history is still on disk at {}; delete it by hand, or run `one-mk clipboard history clear`", file.display())
}

pub async fn handle(cb: &Clipboard, action: HistoryAction, json: bool) -> Result<String> {
    let durable = matches!(action, HistoryAction::Delete(_) | HistoryAction::Clear | HistoryAction::Flush);
    if let HistoryAction::Copy(id) = action {
        let entry = cb.history().lock().unwrap_or_else(|error| error.into_inner()).get(id)
            .ok_or_else(|| anyhow::anyhow!("Clipboard item {id} is no longer in history; run one-mk clipboard history list"))?;
        if entry.data.len() > cb.filter().max_bytes() { bail!("This item is over the clipboard size limit; raise it with one-mk clipboard --max-mb N"); }
        // More than this computer's clipboard can hand over to a program that pastes
        // (X11): set, it would fail every paste. Received copies are refused the same way.
        if let Some(limit) = cb.write_limit() {
            let d = &entry.data;
            let largest = [d.text.as_ref().map(String::len), d.html.as_ref().map(String::len), d.png.as_ref().map(Vec::len)].into_iter().flatten().max().unwrap_or(0);
            if largest > limit {
                let (size, limit) = crate::clip::over(largest, limit);
                bail!("This item is {size}, more than this computer's clipboard can hold ({limit} at most on X11), so it would not paste anywhere");
            }
        }
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
            HistoryAction::Flush => Ok(if json { "{\"flushed\":true}\n".into() } else { String::new() }),
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
    use std::sync::Arc;

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

    /// Turning saving off while the first save is still being written (the
    /// file not there yet) must not answer before that save has finished and
    /// been removed again: the barrier waits for both.
    #[tokio::test]
    async fn the_flush_barrier_waits_for_an_unfinished_first_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = saved_file(dir.path());
        let (clipboard, _) = Clipboard::memory();
        clipboard.history().lock().unwrap().configure(20, one_mk_clipboard::history::DEFAULT_BYTES, one_mk_clipboard::CEILING_BYTES, Some(file.clone()));
        // Large enough that writing it takes a while: saving is turned off
        // while the saver is (or may be) still writing this first copy.
        clipboard.write(one_mk_clipboard::ClipData { text: Some("dummy-secret ".repeat(1 << 20)), ..Default::default() }).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(170)).await;
        {
            let mut history = clipboard.history().lock().unwrap();
            history.remove_saved_file(file.clone());
            history.configure(20, one_mk_clipboard::history::DEFAULT_BYTES, one_mk_clipboard::CEILING_BYTES, None);
        }
        handle(&clipboard, HistoryAction::Flush, false).await.unwrap();
        assert!(!file.exists(), "nothing on disk once the barrier answers");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(!file.exists(), "and nothing appears afterwards");
    }

    /// With no daemon, turning saving off also deletes what a crash in the
    /// middle of a save left behind, and nothing that isn't oneMK's.
    #[tokio::test]
    async fn persist_off_offline_removes_unfinished_saves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".clipboard-history-AbC123"), b"secret history").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"mine").unwrap();
        ClipboardCommand::History { operation: Some(Operation::Persist { enabled: false }), json: false }.run(dir.path()).await.unwrap();
        assert!(!dir.path().join(".clipboard-history-AbC123").exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    /// `history on` keeps the user's own number of items (and `off` is the
    /// one thing that sets it to 0); with no room for anything it says so.
    #[test]
    fn history_on_keeps_the_users_own_item_count() {
        let mut c = crate::settings::ClipSettings { history_max: 200, ..Default::default() };
        turn(&mut c, true);
        assert_eq!(c.history_max, 200);
        turn(&mut c, false);
        assert_eq!(c.history_max, 0);
        turn(&mut c, true);
        assert_eq!(c.history_max, 20);
    }

    #[tokio::test]
    async fn history_on_keeps_the_count_and_notes_a_zero_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        crate::settings::Settings::update(dir.path(), |s| { s.clipboard.history_max_mb = 0; s.clipboard.history_max = 150; Ok(()) }).unwrap();
        ClipboardCommand::History { operation: Some(Operation::On), json: false }.run(dir.path()).await.unwrap();
        let saved = crate::settings::Settings::load_or_default(dir.path()).clipboard;
        assert_eq!(saved.history_max, 150, "the user's own count survives `on`");
        assert_eq!(saved.history_max_mb, 0);
        assert_eq!(no_room(&saved), Some(NO_ROOM));
        assert_eq!(no_room(&crate::settings::ClipSettings::default()), None);
    }

    /// Restoring an item that no paste could read on this computer's clipboard
    /// (X11: one request) is refused, as a received copy of that size is.
    #[tokio::test]
    async fn copy_refuses_what_the_clipboard_cannot_hand_over() {
        let (clipboard, _) = Clipboard::memory();
        let clipboard = clipboard.with_write_limit(1000);
        let history = clipboard.history().clone();
        history.lock().unwrap().configure(20, one_mk_clipboard::history::DEFAULT_BYTES, one_mk_clipboard::CEILING_BYTES, None);
        history.lock().unwrap().record(Arc::new(one_mk_clipboard::ClipData { text: Some("x".repeat(2000)), ..Default::default() }), false, "peer");
        history.lock().unwrap().record(Arc::new(one_mk_clipboard::ClipData { text: Some("small".into()), ..Default::default() }), false, "peer");
        let list = history.lock().unwrap().list();
        let id_of = |text: usize| list.iter().find(|e| e.data.text.as_ref().is_some_and(|t| t.len() == text)).unwrap().id;
        let error = handle(&clipboard, HistoryAction::Copy(id_of(2000)), false).await.unwrap_err().to_string();
        assert!(error.contains("more than this computer's clipboard can hold"), "{error}");
        assert!(handle(&clipboard, HistoryAction::Copy(id_of(5)), false).await.is_ok());
    }

    /// With no daemon the deletion is made durable: the folder is flushed.
    #[cfg(all(unix, debug_assertions))]
    #[tokio::test]
    async fn offline_deletion_flushes_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(saved_file(dir.path()), b"secret history").unwrap();
        ClipboardCommand::History { operation: Some(Operation::Persist { enabled: false }), json: false }.run(dir.path()).await.unwrap();
        assert!(one_mk_net::was_synced(dir.path()));
    }

    /// A daemon that cannot read the new settings keeps the old ones: the command
    /// must say history may still be kept, not that it is off.
    #[tokio::test]
    async fn history_off_reports_a_daemon_that_could_not_apply_it() {
        let dir = crate::testdir::scratch("history-off");
        let mut server = control::Server::start(&dir).await.unwrap();
        let task = tokio::spawn(async move {
            while let Some((_, reply)) = server.recv().await {
                let _ = reply.send("error settings.toml is not valid\n".into());
            }
        });
        let error = ClipboardCommand::History { operation: Some(Operation::Off), json: false }.run(&dir).await.unwrap_err().to_string();
        assert!(error.contains("could not apply") && error.contains("settings.toml is not valid"), "{error}");
        task.abort();
    }

    /// "Persist off" must not report success while a plain-text copy of the
    /// history still sits on disk: with no daemon to delete it, the command does.
    #[tokio::test]
    async fn persist_off_deletes_the_saved_file_before_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let file = saved_file(dir.path());
        std::fs::write(&file, b"secret history").unwrap();
        ClipboardCommand::History { operation: Some(Operation::Persist { enabled: false }), json: false }.run(dir.path()).await.unwrap();
        assert!(!file.exists());
        assert!(!crate::settings::Settings::load_or_default(dir.path()).clipboard.history_persist);
        // Turning it on is saved; the plain-text warning is printed by the command itself.
        ClipboardCommand::History { operation: Some(Operation::Persist { enabled: true }), json: false }.run(dir.path()).await.unwrap();
        assert!(crate::settings::Settings::load_or_default(dir.path()).clipboard.history_persist);
    }
}
