//! QA click-through flows: Clipboard & files, the clipboard history.
//! (The stand-ins for `one-mk` and the daemon are in `qa.rs`.)

use super::qa::{Fake, Socket, args};
use super::*;

fn items() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    format!(
        r#"[{{"id":7,"kind":"text","preview":"[hidden; use Show]","size":12,"timestamp":{},"origin":"Studio"}},
{{"id":8,"kind":"image","preview":"[hidden; use Show]","size":2048,"timestamp":{},"origin":"Office PC"}}]"#,
        now - 30,
        now - 7200
    )
}

/// A daemon with a clipboard history: `list` gives `items`, `show 7` gives its text.
fn history_daemon(win: &Win, items: String) -> Socket {
    Socket::listen(win, move |line| match line {
        "clip-history list --json" => format!("ok\n{items}"),
        "clip-history show 7" => "ok\nthe secret line\nand a second".to_owned(),
        "clip-history show 8" => "error this item is gone".to_owned(),
        l if l.starts_with("clip-history copy") || l == "clip-history clear" => "ok".to_owned(),
        _ => String::new(),
    })
}

fn files_with_history() -> (Win, Fake, Socket) {
    let mut win = Win::light(1200.0, 900.0);
    let fake = Fake::install(&mut win);
    let daemon = history_daemon(&win, items());
    win.settings("[clipboard]\nhistory_max = 50\n");
    let mut status = two();
    status.clipboard_history_enabled = true;
    status.clipboard_history = 2;
    win.status(Some(status));
    win.go(Tab::Files);
    win.wait("the history", |w| w.app.clipboard_history.as_ref().is_some_and(Result::is_ok));
    (win, fake, daemon)
}

const CLEAR: &str = "Clear deletes all memory and saved history. Click again to confirm; Refresh cancels.";
const SURE: &str = "Sure? Click again to delete all memory and saved history; Refresh cancels.";
const SHOW7: &str = "Show the preview of clipboard item 7 on screen";
const HIDE7: &str = "Hide the preview of clipboard item 7 on screen";
const SHOW8: &str = "Show the preview of clipboard item 8 on screen";

#[test]
fn history_is_off_by_default_and_the_switch_asks_for_it() {
    let mut win = Win::light(1200.0, 900.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.go(Tab::Files);
    win.wait("the history note", |w| w.has_text("Nothing you copy is kept"));
    assert!(win.find(CLEAR).unwrap().disabled && win.find("Refresh clipboard history").unwrap().disabled);
    win.click("Keep clipboard history");
    win.quiet();
    assert_eq!(fake.calls_with(&["clipboard"]), vec![args(&["clipboard", "history", "on"])]);
    assert!(win.app.prefs.history_enabled, "shown at once");
    // The command is refused: the switch goes back to what is saved.
    fake.fail("clipboard", "one-mk: settings.toml is read-only");
    win.click("Keep clipboard history");
    win.wait("the refusal", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't change clipboard history.");
    assert!(!win.app.prefs.history_enabled, "back to what the file says");
}

#[test]
fn the_items_are_hidden_until_show_and_hide_hides_them_again() {
    let (mut win, _fake, daemon) = files_with_history();
    assert_eq!(daemon.seen_with("clip-history list"), vec!["clip-history list --json"]);
    assert!(
        win.find_containing(
            "Copy clipboard item 7 here: Hidden · choose Show to see it. text · 12 B · from Studio · just now"
        )
        .is_some(),
        "{:#?}",
        win.names()
    );
    assert!(win.has_text("image · 2 KB · from Office PC"), "{}", win.texts_joined());
    assert!(!win.has_text("the secret line"));
    win.click(SHOW7);
    win.wait("the revealed text", |w| w.app.clipboard_reveal.is_some());
    assert_eq!(daemon.seen_with("clip-history show"), vec!["clip-history show 7"]);
    assert!(win.has_text("the secret line and a second"), "separators are flattened: {}", win.texts_joined());
    assert!(win.find_containing("Copy clipboard item 7 here: the secret line and a second").is_some());
    win.click(HIDE7);
    assert!(win.app.clipboard_reveal.is_none());
    assert!(!win.has_text("the secret line"));
    // Leaving and coming back never shows it again.
    win.click(SHOW7);
    win.wait("the revealed text", |w| w.app.clipboard_reveal.is_some());
    win.go(Tab::Desk);
    win.go(Tab::Files);
    win.wait("the history again", |w| w.app.clipboard_history.as_ref().is_some_and(Result::is_ok));
    assert!(!win.has_text("the secret line"), "{}", win.texts_joined());
    assert!(win.find(SHOW7).is_some());
}

#[test]
fn every_button_says_what_it_shows_in_its_name_even_armed() {
    let (mut win, _fake, _daemon) = files_with_history();
    assert!(label_not_in_name(&win).is_empty(), "{:#?}", label_not_in_name(&win));
    win.click(CLEAR);
    assert!(win.find(SURE).is_some());
    assert!(label_not_in_name(&win).is_empty(), "{:#?}", label_not_in_name(&win));
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn an_item_that_is_gone_reveals_nothing() {
    let (mut win, _fake, daemon) = files_with_history();
    win.click(SHOW8);
    win.wait("the request", |_| !daemon.seen_with("clip-history show").is_empty());
    win.frames(30);
    assert_eq!(daemon.seen_with("clip-history show"), vec!["clip-history show 8"]);
    assert!(win.app.clipboard_reveal.is_none());
    assert!(win.find(SHOW8).is_some(), "still hidden");
}

#[test]
fn clicking_an_item_copies_it_here() {
    let (mut win, _fake, daemon) = files_with_history();
    let row = win.find_containing("Copy clipboard item 7 here").unwrap();
    win.click_at(row.rect.center());
    win.wait("the copy", |w| {
        !w.app.busy.iter().any(|j| matches!(j, Job::Set { .. })) && !daemon.seen_with("clip-history copy").is_empty()
    });
    assert_eq!(daemon.seen_with("clip-history copy"), vec!["clip-history copy 7"]);
    // The list is read again after it.
    win.wait("the list again", |_| daemon.seen_with("clip-history list").len() >= 2);
}

#[test]
fn clear_asks_twice_refresh_cancels_and_leaving_the_page_cancels() {
    let (mut win, _fake, daemon) = files_with_history();
    win.click(CLEAR);
    assert!(win.app.clipboard_clear_confirm);
    assert!(win.find(SURE).is_some(), "{:#?}", win.names());
    assert!(win.has_text("Sure?"), "{}", win.texts_joined());
    assert!(daemon.seen_with("clip-history clear").is_empty(), "one click clears nothing");
    // Refresh cancels.
    win.click("Refresh clipboard history");
    win.wait("the history", |w| w.app.clipboard_history.as_ref().is_some_and(Result::is_ok));
    assert!(!win.app.clipboard_clear_confirm);
    assert!(win.find(CLEAR).is_some() && win.find(SURE).is_none());
    // Armed, then the page is left and opened again: it must not still be
    // armed, or one stray click would delete the whole history.
    win.click(CLEAR);
    assert!(win.find(SURE).is_some());
    win.go(Tab::Desk);
    win.go(Tab::Files);
    win.wait("the history", |w| w.app.clipboard_history.as_ref().is_some_and(Result::is_ok));
    assert!(win.find(SURE).is_none(), "the second click is asked for again");
    assert!(daemon.seen_with("clip-history clear").is_empty());
    // Switching history off and on again does not leave it armed either.
    win.click(CLEAR);
    win.click("Keep clipboard history");
    win.settle();
    assert!(!win.app.clipboard_clear_confirm);
    win.click("Keep clipboard history");
    win.settle();
    assert!(win.find(SURE).is_none());
    // Two clicks clear it.
    win.click(CLEAR);
    win.click(SURE);
    win.wait("the clear", |w| {
        w.app.busy.iter().all(|j| !matches!(j, Job::Set { .. })) && !daemon.seen_with("clip-history clear").is_empty()
    });
    assert_eq!(daemon.seen_with("clip-history clear"), vec!["clip-history clear"]);
}

#[test]
fn history_that_cannot_be_read_says_why_and_an_empty_one_says_so() {
    let mut win = Win::light(1200.0, 900.0);
    let _fake = Fake::install(&mut win);
    win.settings("[clipboard]\nhistory_max = 50\n");
    let mut status = two();
    status.clipboard_history_enabled = true;
    win.status(Some(status));
    win.go(Tab::Files);
    // No daemon answers on the socket: the window says how to fix it.
    win.wait("the note", |w| w.app.clipboard_history.is_some());
    assert!(
        win.has_text("Start oneMK to see clipboard history.") || win.has_text("Could not read history"),
        "{}",
        win.texts_joined()
    );
    win.send(Event::ClipboardHistory(Ok(vec![])));
    assert!(win.has_text("No clipboard items yet. Copy something, then choose Refresh."), "{}", win.texts_joined());
    win.send(Event::ClipboardHistory(Err("Could not read history. Try Refresh when oneMK is running.".into())));
    assert!(win.has_text("Could not read history. Try Refresh when oneMK is running."), "{}", win.texts_joined());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

/// A daemon that knows its history instance, which a test can change to
/// stand for a restart: it lists item 7 under the current instance, and
/// refuses a preview or copy sent with another one, as the real daemon does.
fn instance_daemon(win: &Win, instance: std::sync::Arc<std::sync::atomic::AtomicU64>, preview_delay_ms: u64) -> Socket {
    use std::sync::atomic::Ordering;
    let stale = "error The clipboard history was reloaded since this list was read (oneMK restarted); refresh the list and choose again";
    Socket::listen(win, move |line| {
        let now = instance.load(Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        match line {
            "clip-history list --json" => format!(
                r#"ok
[{{"id":7,"instance":{now},"kind":"text","preview":"[hidden; use Show]","size":12,"timestamp":{timestamp},"origin":"Studio"}}]"#
            ),
            l if l.starts_with("clip-history preview 7 ") || l.starts_with("clip-history copy 7 ") => {
                if l.starts_with("clip-history preview") { std::thread::sleep(std::time::Duration::from_millis(preview_delay_ms)); }
                if l.ends_with(&format!(" {now}")) { if l.contains("preview") { "ok\nthe bounded preview".to_owned() } else { "ok".to_owned() } } else { stale.to_owned() }
            }
            _ => String::new(),
        }
    })
}

fn files_with_instance(delay: u64) -> (Win, Fake, Socket, std::sync::Arc<std::sync::atomic::AtomicU64>) {
    let mut win = Win::light(1200.0, 900.0);
    let fake = Fake::install(&mut win);
    let instance = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(42));
    let daemon = instance_daemon(&win, instance.clone(), delay);
    win.settings("[clipboard]\nhistory_max = 50\n");
    let mut status = two();
    status.clipboard_history_enabled = true;
    status.clipboard_history = 1;
    win.status(Some(status));
    win.go(Tab::Files);
    win.wait("the history", |w| w.app.clipboard_history.as_ref().is_some_and(Result::is_ok));
    (win, fake, daemon, instance)
}

#[test]
fn show_and_copy_send_the_history_instance_the_row_was_listed_under() {
    let (mut win, _fake, daemon, _instance) = files_with_instance(0);
    win.click(SHOW7);
    win.wait("the preview", |w| w.app.clipboard_reveal.is_some());
    // The bounded preview, not the CLI's whole-item show.
    assert_eq!(daemon.seen_with("clip-history preview"), vec!["clip-history preview 7 42"]);
    assert!(daemon.seen_with("clip-history show").is_empty());
    assert!(win.has_text("the bounded preview"), "{}", win.texts_joined());
    let row = win.find_containing("Copy clipboard item 7 here").unwrap();
    win.click_at(row.rect.center());
    win.wait("the copy", |w| !w.app.busy.iter().any(|j| matches!(j, Job::Set { .. })) && !daemon.seen_with("clip-history copy").is_empty());
    assert_eq!(daemon.seen_with("clip-history copy"), vec!["clip-history copy 7 42"]);
}

#[test]
fn a_row_from_before_a_daemon_restart_is_refused_says_so_and_the_list_is_read_again() {
    let (mut win, _fake, daemon, instance) = files_with_instance(0);
    // The daemon restarted between two looks: its history is another instance now.
    instance.store(43, std::sync::atomic::Ordering::SeqCst);
    let lists = daemon.seen_with("clip-history list").len();
    win.click(SHOW7);
    win.wait("the refusal", |w| w.app.clipboard_reveal_error.is_some());
    assert!(win.app.clipboard_reveal.is_none(), "nothing from the new history is shown for the old row");
    assert!(win.has_text("The clipboard history was reloaded since this list was read (oneMK restarted); refresh the list and choose again"), "{}", win.texts_joined());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    // The list is read again, so the next click goes out with the new instance.
    win.wait("the list again", |_| daemon.seen_with("clip-history list").len() > lists);
    win.wait("the new rows", |w| w.app.clipboard_history.as_ref().is_some_and(|h| h.as_ref().is_ok_and(|items| items.iter().all(|item| item.instance == Some(43)))));
    win.click(SHOW7);
    win.wait("the preview", |w| w.app.clipboard_reveal.is_some());
    assert_eq!(daemon.seen_with("clip-history preview").last().map(String::as_str), Some("clip-history preview 7 43"));
    assert!(win.app.clipboard_reveal_error.is_none(), "the next try clears it");
    // Copy of a stale row: the daemon refuses, the window says so, and reads the list again.
    instance.store(44, std::sync::atomic::Ordering::SeqCst);
    let lists = daemon.seen_with("clip-history list").len();
    let row = win.find_containing("Copy clipboard item 7 here").unwrap();
    win.click_at(row.rect.center());
    win.wait("the refusal", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    win.wait("the list again", |_| daemon.seen_with("clip-history list").len() > lists);
    assert_eq!(daemon.seen_with("clip-history copy"), vec!["clip-history copy 7 43"]);
}

#[test]
fn a_preview_still_on_its_way_is_not_asked_for_twice() {
    let (mut win, _fake, daemon, _instance) = files_with_instance(700);
    win.click_now(SHOW7);
    win.frames(3);
    let show = win.find(SHOW7).expect("the row keeps its Show button");
    assert!(show.disabled, "Show waits for the answer");
    // Presses meanwhile go nowhere.
    for _ in 0..3 { win.click_at(show.rect.center()); }
    win.wait("the preview", |w| w.app.clipboard_reveal.is_some());
    assert_eq!(daemon.seen_with("clip-history preview"), vec!["clip-history preview 7 42"]);
    assert!(win.find(HIDE7).is_some_and(|button| !button.disabled));
}
