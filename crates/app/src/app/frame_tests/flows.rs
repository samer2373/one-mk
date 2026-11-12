//! Flows: press things, type, and watch the window's own state change.
//! Nothing here presses a control that opens a file dialog (Choose a video,
//! Send files) or changes the login items: those reach the desktop.

use super::*;

fn pairing(step: Step, win: &Win) -> PairUi {
    PairUi { proc: None, step, since: win.t, opened: win.t - 1.0, hint: false, retry: None, me: "Studio".into(), port: Some(24800) }
}

fn show_pairing(win: &mut Win, step: Step) {
    win.app.pairing = Some(pairing(step, win));
    win.settle();
}

// ---------- pages and shortcuts ----------

#[test]
fn every_page_has_its_shortcut_and_its_tab_in_the_bar() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    for (tab, name) in [(Tab::Arrange, "Arrange"), (Tab::Wallpaper, "Wallpaper"), (Tab::Files, "Clipboard & files"), (Tab::Keys, "Keys & mouse"), (Tab::Health, "Health"), (Tab::Settings, "Settings"), (Tab::Desk, "Desk")] {
        win.go(tab);
        let bar = win.find(name).unwrap_or_else(|| panic!("no tab {name}"));
        assert_eq!(bar.role, accesskit::Role::Tab);
    }
    // Ctrl+, is Settings as everywhere.
    win.go(Tab::Desk);
    win.shortcut(Key::Comma);
    win.settle();
    assert_eq!(win.tab(), Tab::Settings);
}

#[test]
fn clicking_a_tab_in_the_bar_opens_its_page() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    for (name, tab) in [("Arrange", Tab::Arrange), ("Wallpaper", Tab::Wallpaper), ("Clipboard & files", Tab::Files), ("Keys & mouse", Tab::Keys), ("Health", Tab::Health), ("Settings", Tab::Settings), ("Desk", Tab::Desk)] {
        win.click(name);
        win.settle();
        assert_eq!(win.tab(), tab, "{name}");
    }
}

#[test]
fn a_narrow_window_shortens_the_tabs_and_the_action() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    assert!(win.find("Clipboard & files").is_some() && win.find("Keys & mouse").is_some());
    win.resize(900.0, 700.0);
    // The accessible names stay whole; the painted words shorten.
    assert!(win.find("Clipboard & files").is_some(), "{:#?}", win.names());
    assert!(win.has_text("Files") && win.has_text("Keys"));
    assert!(!win.texts.iter().any(|t| t.text == "Clipboard & files"), "the long name is painted only in a wide bar");
    win.resize(1200.0, 780.0);
    assert!(win.texts.iter().any(|t| t.text == "Clipboard & files"));
}

#[test]
fn resizing_through_every_width_never_breaks_a_page() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    for tab in [Tab::Desk, Tab::Arrange, Tab::Files, Tab::Settings] {
        win.go(tab);
        for (w, h) in [(2000.0, 1200.0), (1281.0, 800.0), (1100.0, 700.0), (999.0, 640.0), (900.0, 600.0), (820.0, 540.0), (1500.0, 900.0)] {
            win.size = vec2(w, h);
            win.frames(12);
            let bad = problems(&win);
            assert!(bad.is_empty(), "{tab:?} at {w}x{h}: {bad:#?}");
        }
    }
}

// ---------- states of the daemon ----------

#[test]
fn looking_then_off_then_running_then_off_again() {
    let mut win = Win::light(1200.0, 780.0);
    assert!(win.has_text("Looking for oneMK"));
    assert!(win.app.status.is_none());
    win.status(None);
    assert!(win.app.off());
    assert!(win.has_text("oneMK isn't running"), "{:#?}", win.names());
    assert!(win.find("Start oneMK").is_some());
    win.status(Some(alone()));
    assert!(!win.app.off());
    assert!(win.find("Start oneMK").is_none());
    assert!(win.has_text("No computers paired yet") || win.has_text("Pair your first computer"));
    win.status(Some(two()));
    assert!(win.has_text("Office PC"));
    win.status(None);
    assert!(win.has_text("oneMK isn't running"));
}

#[test]
fn off_takes_the_window_back_to_a_page_that_means_something() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.go(Tab::Arrange);
    win.status(None);
    // Arrange is nothing without a running oneMK: it goes to the Desk.
    assert_eq!(win.tab(), Tab::Desk);
    let arrange = win.find("Arrange").expect("tab");
    assert!(arrange.disabled, "Arrange stays dim while oneMK is off");
    win.shortcut(Key::Num2);
    assert_eq!(win.tab(), Tab::Desk, "its shortcut does nothing while off");
    win.shortcut(Key::Num6);
    win.settle();
    assert_eq!(win.tab(), Tab::Health, "Health still opens");
}

#[test]
fn start_onemk_tries_and_a_failure_says_so_with_try_again_and_the_log() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(None);
    win.click_now("Start oneMK");
    assert!(win.app.busy.contains(&Job::Start));
    // The service program is missing: the start fails and says what to do.
    for _ in 0..200 {
        win.frames(3);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if win.app.toast.is_some() {
            break;
        }
    }
    win.settle();
    let toast = win.app.toast.as_ref().expect("an error toast");
    assert!(toast.error);
    assert!(toast.text.contains("couldn't start"), "{}", toast.text);
    assert!(!win.app.busy.contains(&Job::Start), "the button is free again");
    assert!(win.find_containing("Try again").is_some() && win.find_containing("Show log").is_some(), "{:#?}", win.names());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn the_top_bar_action_follows_what_is_happening() {
    let mut win = Win::light(1200.0, 780.0);
    let action = |win: &Win| win.app.action().0;
    assert_eq!(action(&win), "Pair a computer");
    win.status(None);
    assert_eq!(action(&win), "Start oneMK");
    win.status(Some(alone()));
    assert_eq!(action(&win), "Pair a computer");
    win.status(Some(two()));
    assert_eq!(action(&win), "Pair another computer");
    let mut s = two();
    s.controlled_by = Some("Office PC".into());
    win.status(Some(s));
    assert_eq!(action(&win), "Take back");
    assert!(win.has_text("Office PC is controlling this computer"));
    let mut s = two();
    s.broadcast = vec!["Office PC".into()];
    win.status(Some(s));
    assert_eq!(action(&win), "Use keyboard to stop");
    assert!(!win.app.action().2, "it is a sentence, not a button");
    assert!(win.has_text("Broadcasting"));
}

#[test]
fn the_headline_names_the_situation_in_every_state() {
    let mut win = Win::light(1200.0, 780.0);
    let headline = |win: &mut Win, s: Status| {
        win.status(Some(s));
        win.app.headline_text().0
    };
    assert_eq!(headline(&mut win, alone()), "No computers paired yet");
    assert_eq!(headline(&mut win, two()), "Office PC is on your right");
    let mut s = with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, false)]);
    assert_eq!(headline(&mut win, s.clone()), "Waiting for your computers");
    s.removed = vec!["p1".into()];
    assert_eq!(headline(&mut win, s), "Office PC removed this computer");
    let mut s = two();
    s.controlling = Some("Office PC".into());
    assert_eq!(headline(&mut win, s), "Controlling Office PC");
    let mut s = two();
    s.input = vec!["Waiting for your approval in the desktop's dialog.".into()];
    assert_eq!(headline(&mut win, s), "Waiting for this computer's permission");
    let mut s = crowded();
    s.peers[3].online = true;
    assert_eq!(headline(&mut win, s), "Connected to 5 computers");
    let mut s = alone();
    s.unrecognised = vec!["A computer keeps trying; pair it first.".into()];
    assert_eq!(headline(&mut win, s), "A computer can't get in");
}

#[test]
fn the_status_line_in_the_bar_says_it_in_a_word() {
    let mut win = Win::light(1400.0, 800.0);
    assert_eq!(win.app.status_line().1, "Looking for oneMK…");
    win.status(None);
    assert_eq!(win.app.status_line().1, "Sharing is off");
    win.status(Some(alone()));
    assert_eq!(win.app.status_line().1, "On · not paired");
    win.status(Some(with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, false)])));
    assert_eq!(win.app.status_line().1, "On · none connected");
    win.status(Some(crowded()));
    assert_eq!(win.app.status_line().1, "On · some offline");
    win.status(Some(two()));
    assert_eq!(win.app.status_line().1, "On · connected");
    assert!(win.find_containing("On · connected").is_some(), "the status is a link to Health for a screen reader");
}

// ---------- the palette ----------

#[test]
fn the_palette_opens_filters_goes_and_closes() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.shortcut(Key::K);
    assert!(win.app.palette.open);
    assert!(win.find("Settings").is_some());
    win.type_text("wallp");
    assert!(win.find("Wallpaper").is_some());
    win.press(Key::Enter);
    win.settle();
    assert!(!win.app.palette.open);
    assert_eq!(win.tab(), Tab::Wallpaper);
    // Ctrl+K again closes it; so does Esc; so does a click away.
    win.shortcut(Key::K);
    assert!(win.app.palette.open);
    win.shortcut(Key::K);
    assert!(!win.app.palette.open);
    win.shortcut(Key::K);
    win.press(Key::Escape);
    assert!(!win.app.palette.open);
    win.shortcut(Key::K);
    win.frames(10);
    win.click_at(pos2(5.0, 700.0));
    assert!(!win.app.palette.open, "a click on the dimmed window closes it");
}

#[test]
fn the_palette_finds_nothing_politely_and_survives_odd_text() {
    let mut win = Win::light(820.0, 540.0);
    win.status(Some(hard_names()));
    win.shortcut(Key::K);
    for query in ["zzzzzz", "ß", "İ", "שלום", "مكتب", "日本", "🖥️", "  ", "a very long query that is longer than the box could ever show in one line, so it scrolls"] {
        // Typed, as a person does (the field scrolls to what was typed last).
        win.app.palette.query.clear();
        win.type_text(query);
        win.frames(6);
        // (A text field scrolls what is typed in it: its own text is cut at the side on purpose.)
        let bad: Vec<String> = problems(&win).into_iter().filter(|p| !(p.starts_with("text cut off") && p.contains(query))).collect();
        assert!(bad.is_empty(), "query {query:?}: {bad:#?}");
    }
    win.app.palette.query = "zzzzzz".into();
    win.frames(4);
    assert!(win.has_text("Nothing found"));
    // The computers' own names are found, in any script.
    win.app.palette.query = "日本".into();
    win.frames(4);
    assert!(win.find("Switch to 日本語のコンピュータ名前").is_some(), "{:#?}", win.names());
    win.app.palette.query = "مكتب".into();
    win.frames(4);
    assert!(win.find("Switch to مكتب العمل الرئيسي").is_some());
}

#[test]
fn the_palette_changes_the_look_and_the_window_follows() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    assert_eq!(pal().theme, crate::style::Theme::Light);
    win.shortcut(Key::K);
    win.type_text("Appearance: Dark");
    win.press(Key::Enter);
    win.settle();
    assert_eq!(pal().theme, crate::style::Theme::Dark);
    assert_eq!(win.app.appearance(), crate::style::Appearance::Dark);
    win.shortcut(Key::K);
    win.type_text("Appearance: Light");
    win.press(Key::Enter);
    win.settle();
    assert_eq!(pal().theme, crate::style::Theme::Light);
}

// ---------- pairing ----------

#[test]
fn pair_a_computer_without_a_service_says_why_and_leaves_no_sheet() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    win.click("Pair a computer");
    win.settle();
    // `one-mk pair` can't be started (this is a test): a toast, no half-open sheet.
    assert!(win.app.pairing.is_none());
    let toast = win.app.toast.as_ref().expect("a toast");
    assert!(toast.error && toast.text.contains("Couldn't start pairing"), "{}", toast.text);
    assert!(!win.app.sheet_open());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn each_pairing_step_has_its_sheet_and_its_buttons() {
    let mut win = Win::light(820.0, 540.0);
    win.status(Some(alone()));
    let steps: Vec<(Step, &str, Vec<&str>)> = vec![
        (Step::Searching { me: "Studio".into(), port: Some(24800) }, "Looking for another computer", vec!["Cancel"]),
        (Step::Found, "Found a computer", vec!["Cancel"]),
        (Step::Code { name: "Office PC".into(), code: "123456".into() }, "Found “Office PC”", vec!["They don't match", "Codes match, pair"]),
        (Step::Code { name: "Studio".into(), code: "123456".into() }, "Found another computer named “Studio”", vec!["They don't match", "Codes match, pair"]),
        (Step::Waiting { name: "Office PC".into() }, "Waiting for Office PC to confirm", vec!["Cancel"]),
        (Step::Ended("nobody confirmed the code in 2 minutes".into()), "Not paired", vec!["Close", "Try again"]),
        (Step::Paired { id: "p9".into(), name: "Office PC".into() }, "Paired with Office PC", vec![]),
    ];
    for (step, title, buttons) in steps {
        let label = format!("{step:?}");
        show_pairing(&mut win, step);
        assert!(win.app.sheet_open(), "{label}");
        assert!(win.has_text(title), "{label}: {:#?}", win.names());
        for b in buttons {
            assert!(win.find(b).is_some(), "{label}: button {b:?} in {:#?}", win.names());
        }
        let bad = problems(&win);
        assert!(bad.is_empty(), "{label}: {bad:#?}");
        // The sheet sits inside the window, and the dialog is named for a screen reader.
        let rect = win.app.sheet_rect.expect("sheet rect");
        assert!(win.window().contains_rect(rect), "{label}: {rect:?}");
        assert!(win.items().iter().any(|i| i.role == accesskit::Role::Dialog && i.name.starts_with(title)), "{label}");
    }
}

#[test]
fn the_codes_screen_confirms_and_waits() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    show_pairing(&mut win, Step::Code { name: "Office PC".into(), code: "482915".into() });
    // The keyboard starts on the safe button, never the one that confirms.
    let focused = win.items().into_iter().find(|i| i.focused).map(|i| i.name);
    assert_eq!(focused.as_deref(), Some("They don't match"));
    win.click("Codes match, pair");
    assert!(matches!(win.app.pairing.as_ref().unwrap().step, Step::Waiting { .. }));
    assert!(win.has_text("Waiting for Office PC to confirm"));
}

#[test]
fn escape_says_they_dont_match_on_the_codes_and_cancels_elsewhere() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    show_pairing(&mut win, Step::Searching { me: "Studio".into(), port: None });
    win.press(Key::Escape);
    win.settle();
    assert!(win.app.pairing.is_none(), "Esc cancels a search");
    // On the codes it is a "no" (the other side hears it), not a silent close:
    // with no service behind it here the sheet stays for the answer to come.
    show_pairing(&mut win, Step::Code { name: "Office PC".into(), code: "111111".into() });
    win.press(Key::Escape);
    assert!(win.app.pairing.is_some());
    // Once confirmed, Esc waits for Cancel.
    show_pairing(&mut win, Step::Waiting { name: "Office PC".into() });
    win.press(Key::Escape);
    assert!(win.app.pairing.is_some());
    win.click("Cancel");
    win.settle();
    assert!(win.app.pairing.is_none());
}

#[test]
fn the_keyboard_cannot_leave_the_pairing_sheet() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    show_pairing(&mut win, Step::Code { name: "Studio 2".into(), code: "654321".into() });
    let sheet = win.app.sheet_rect.unwrap();
    for _ in 0..12 {
        win.press(Key::Tab);
        let focused = win.items().into_iter().find(|i| i.focused);
        if let Some(f) = focused {
            assert!(sheet.expand(2.0).contains_rect(f.rect), "Tab reached {:?} outside the sheet {sheet:?}", f.name);
        }
    }
}

#[test]
fn a_paired_computer_is_selected_and_the_sheet_goes_by_itself() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    show_pairing(&mut win, Step::Waiting { name: "Office PC".into() });
    win.app.pairing.as_mut().unwrap().step = Step::Paired { id: "p1".into(), name: "Office PC".into() };
    win.app.pairing.as_mut().unwrap().since = win.t;
    // A paired sheet holds a moment (the tick), then leaves.
    win.frames(30);
    assert!(win.app.pairing.is_some());
    win.frames(120);
    assert!(win.app.pairing.is_none());
    assert!(!win.app.sheet_open());
}

#[test]
fn ended_then_try_again_starts_again_and_reports_a_failure() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    show_pairing(&mut win, Step::Ended("Pairing stopped".into()));
    win.click("Try again");
    win.settle();
    assert!(win.app.pairing.is_none());
    assert!(win.app.toast.as_ref().is_some_and(|t| t.text.contains("Couldn't start pairing")));
}

#[test]
fn a_slow_search_adds_the_firewall_hint_and_a_failed_attempt_is_said() {
    let mut win = Win::light(820.0, 540.0);
    win.status(Some(alone()));
    show_pairing(&mut win, Step::Hint);
    win.app.pairing.as_mut().unwrap().hint = true;
    win.settle();
    assert!(win.has_text("firewall"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    assert!(win.has_text("pairing port is 24800"));
    let p = win.app.pairing.as_mut().unwrap();
    p.retry = Some("the codes did not match".into());
    win.settle();
    assert!(win.has_text("That attempt didn't finish"));
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

// ---------- removing, dropping, errors ----------

#[test]
fn removing_a_computer_asks_first_and_cancel_changes_nothing() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.click("Remove this computer: Office PC (asks first)");
    win.settle();
    assert!(win.has_text("Remove Office PC?"));
    assert_eq!(win.app.sheet_view().unwrap().focus, 0);
    win.click("Cancel");
    win.settle();
    assert!(win.app.removing.is_none());
    assert!(win.app.busy.is_empty());
    win.click("Remove this computer: Office PC (asks first)");
    win.settle();
    win.press(Key::Escape);
    win.settle();
    assert!(win.app.removing.is_none(), "Esc is Cancel");
    assert!(win.app.busy.is_empty());
}

#[test]
fn removing_then_a_failure_says_so() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.click("Remove this computer: Office PC (asks first)");
    win.settle();
    win.click("Remove");
    assert!(win.app.removing.is_none());
    for _ in 0..200 {
        win.frames(3);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if win.app.toast.is_some() {
            break;
        }
    }
    win.settle();
    let toast = win.app.toast.as_ref().expect("the failure is said");
    assert!(toast.text.contains("Couldn't remove Office PC"), "{}", toast.text);
    assert!(win.app.busy.is_empty());
}

#[test]
fn files_dropped_where_it_is_unclear_ask_which_computer() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(crowded()));
    win.go(Tab::Settings);
    let file = win.dir.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    win.move_to(pos2(300.0, 300.0));
    // A file let go over a page without the desk opens Clipboard & files and asks.
    win.drop_files(&[file.clone()]);
    win.settle();
    assert_eq!(win.tab(), Tab::Files);
    assert!(win.app.pending_drop.is_some() || win.app.sheet_open(), "{:#?}", win.names());
    assert!(win.has_text("Send 1 item to which computer?"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    // Cancel, the online computers only (the offline one is not offered).
    for name in ["Cancel", "Office PC", "MacBook Pro", "Living room", "Build server"] {
        assert!(win.find(name).is_some(), "{name}: {:#?}", win.names());
    }
    assert!(win.find("Old laptop").is_none(), "the offline computer is not offered");
    // The sheet fits the window and its buttons stand apart (the page behind it is another matter).
    let sheet = win.app.sheet_rect.expect("sheet");
    assert!(win.window().contains_rect(sheet), "{sheet:?}");
    let buttons: Vec<Item> = ["Cancel", "Office PC", "MacBook Pro", "Living room", "Build server"].iter().filter_map(|n| win.find(n)).collect();
    for (i, a) in buttons.iter().enumerate() {
        assert!(sheet.expand(1.0).contains_rect(a.rect), "{:?} {:?} outside {sheet:?}", a.name, a.rect);
        for b in &buttons[i + 1..] {
            assert!(a.rect.intersect(b.rect).area() < 1.0, "{:?} lies on {:?}", a.name, b.name);
        }
    }
    win.click("Cancel");
    win.settle();
    assert!(win.app.pending_drop.is_none());
}

#[test]
fn an_error_toast_says_what_to_do_and_goes_away() {
    let mut win = Win::light(820.0, 540.0);
    win.status(Some(two()));
    let now = win.t;
    win.app.toast_error("Couldn't move Office PC to the left. The desk went back to where it was; try again.", Some("one-mk: the layout file is read-only".into()), now);
    win.frames(30);
    assert!(win.items().iter().any(|i| i.role == accesskit::Role::Alert && i.name.contains("Couldn't move Office PC")), "{:#?}", win.names());
    assert!(win.find("Open Health").is_some() || win.find_containing("Health").is_some(), "{:#?}", win.names());
    let bad = problems(&win);
    assert!(bad.is_empty(), "{bad:#?}");
    win.press(Key::Escape);
    win.frames(40);
    assert!(win.app.toast.is_none(), "Esc dismisses an error");
}

#[test]
fn a_very_long_error_stays_inside_the_window() {
    for (w, h) in [(820.0, 540.0), (1200.0, 780.0)] {
        let mut win = Win::light(w, h);
        win.status(Some(two()));
        let long = "The reason is very long indeed and keeps going. ".repeat(30);
        let now = win.t;
        win.app.toast_error("Couldn't change the setting that you asked for just now.", Some(long), now);
        win.frames(40);
        let alert = win.items().into_iter().find(|i| i.role == accesskit::Role::Alert).expect("alert");
        assert!(win.window().expand(1.0).contains_rect(alert.rect), "{w}x{h}: {:?}", alert.rect);
        let bad = problems(&win);
        assert!(bad.is_empty(), "{w}x{h}: {bad:#?}");
    }
}

#[test]
fn a_plain_toast_leaves_by_itself() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    let now = win.t;
    win.app.toast("Sent 2 items to Office PC", now);
    win.frames(20);
    assert!(win.has_text("Sent 2 items to Office PC"));
    assert!(win.items().iter().any(|i| i.role == accesskit::Role::Status && i.name.contains("Sent 2 items")));
    win.frames(60 * 5);
    assert!(win.app.toast.is_none());
}

// ---------- the desk and what it can do ----------

#[test]
fn choosing_a_side_changes_the_desk_at_once_and_a_failure_puts_it_back() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    assert_eq!(win.app.side_shown("p1", Side::Right), Side::Right);
    win.click_now("Position of Office PC: Left");
    assert_eq!(win.app.side_shown("p1", Side::Right), Side::Left, "shown at once, before the daemon says");
    assert!(win.app.busy.iter().any(|j| matches!(j, Job::Layout { .. })));
    for _ in 0..200 {
        win.frames(3);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if win.app.toast.is_some() {
            break;
        }
    }
    win.settle();
    // `one-mk layout` is missing: the desk goes back and says so.
    let toast = win.app.toast.as_ref().expect("toast");
    assert!(toast.text.contains("Couldn't move Office PC to the left"), "{}", toast.text);
    assert_eq!(win.app.side_shown("p1", Side::Right), Side::Right);
}

#[test]
fn selecting_a_computer_changes_the_panel() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.click("Studio, this computer");
    win.settle();
    assert_eq!(win.app.selected_id().as_deref(), Some("me"));
    assert!(win.find("Remove this computer: Office PC (asks first)").is_none());
    win.click("Office PC, Windows, on your right");
    win.settle();
    assert_eq!(win.app.selected_id().as_deref(), Some("p1"));
    assert!(win.find("Remove this computer: Office PC (asks first)").is_some());
}

#[test]
fn the_computer_that_goes_away_while_selected_falls_back_to_this_one() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.click("Office PC, Windows, on your right");
    win.status(Some(alone()));
    assert!(win.find("Remove this computer: Office PC (asks first)").is_none());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn an_offline_computer_says_so_and_cannot_be_switched_to() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, false)])));
    win.click("Office PC, Offline, on your right");
    win.settle();
    assert!(win.has_text("Offline"), "{:#?}", win.names());
    if let Some(switch) = win.find("Move the mouse and keyboard to Office PC") {
        assert!(switch.disabled, "an offline computer can't take the keyboard");
    }
    let mut s = with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, false)]);
    s.removed = vec!["p1".into()];
    win.status(Some(s));
    assert!(win.has_text("Removed you"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
}

#[test]
fn a_transfer_shows_its_progress_and_never_overflows() {
    let mut win = Win::light(820.0, 540.0);
    let mut s = two();
    s.transfers = vec![
        TransferStatus { id: 1, sending: true, peer: "Office PC".into(), files: 3, done: 450_000_000, total: 1_000_000_000, rate: 8_000_000 },
        TransferStatus { id: 2, sending: false, peer: "Office PC".into(), files: 1, done: 0, total: 0, rate: 0 },
    ];
    win.status(Some(s));
    for tab in [Tab::Desk, Tab::Files] {
        win.go(tab);
        assert!(win.has_text("45%") || win.has_text("Sending 3 files"), "{tab:?}: {:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
        let bad = problems(&win);
        assert!(bad.is_empty(), "{tab:?}: {bad:#?}");
    }
}

// ---------- settings, health ----------

#[test]
fn the_settings_pills_open_one_category_each() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.go(Tab::Settings);
    for cat in settings_page::Cat::ALL {
        let item = win.items().into_iter().find(|i| i.name == cat.label()).unwrap_or_else(|| panic!("no pill {}: {:#?}", cat.label(), win.names()));
        win.click_at(item.rect.center());
        win.settle();
        assert_eq!(win.app.settings_cat, cat);
        let bad = problems(&win);
        assert!(bad.is_empty(), "{cat:?}: {bad:#?}");
    }
}

#[test]
fn appearance_in_settings_switches_the_whole_window() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.go(Tab::Settings);
    let dark = win.find_containing("Dark").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(dark.rect.center());
    win.frames(60);
    assert_eq!(pal().theme, crate::style::Theme::Dark);
    assert!(problems(&win).is_empty());
}

fn check(id: &str, verdict: crate::view::Verdict, detail: &str, fix: Option<&str>) -> crate::view::Check {
    crate::view::Check { id: id.into(), title: id.into(), verdict, detail: detail.into(), fix: fix.map(str::to_owned), action: None }
}

#[test]
fn health_reads_the_checks_and_check_again_runs_them() {
    use crate::view::Verdict::*;
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.go(Tab::Health);
    // The first run (against a program that is missing) says it didn't run.
    for _ in 0..100 {
        win.frames(3);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if win.app.doctor.is_some() {
            break;
        }
    }
    win.settle();
    assert!(win.has_text("The checks didn't run") || win.has_text("Not checked yet"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    win.send(Event::Doctor(Ok(vec![
        check("daemon", Pass, "oneMK is running.", None),
        check("firewall", Warn, "Port 24800 may be blocked.", Some("Allow oneMK through the firewall.")),
        check("clipboard", Fail, "No clipboard tool.", Some("`sudo apt install wl-clipboard`")),
        check("hotkeys", Skip, "Not set up.", None),
    ])));
    assert!(win.has_text("Port 24800 may be blocked."), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    assert!(win.has_text("2 things need you") || win.has_text("need you"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    assert!(win.find_containing("Copy").is_some(), "{:#?}", win.names());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    // Everything passes: one line says so.
    win.send(Event::Doctor(Ok(vec![check("daemon", Pass, "oneMK is running.", None)])));
    assert!(win.has_text("Checks pass"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    win.click("Check again: run the checks again");
    assert!(win.app.busy.contains(&Job::Doctor) || win.app.doctor.is_some());
}

#[test]
fn the_bar_warns_when_health_needs_you() {
    use crate::view::Verdict::*;
    let mut win = Win::light(1400.0, 800.0);
    win.status(Some(two()));
    win.go(Tab::Health);
    win.send(Event::Doctor(Ok(vec![check("clipboard", Fail, "No clipboard tool.", None)])));
    assert!(win.app.health_trouble());
    assert_eq!(win.app.status_line().1, "On · check Health");
}

// ---------- one computer, an update, a long name ----------

#[test]
fn one_computer_offers_what_works_alone() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    assert!(win.has_text("Pair your first computer"), "{:#?}", win.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    assert!(win.has_text("Or use oneMK on this computer alone"));
    let button = win.find("Clipboard history, on this computer").expect("a button for what works alone");
    win.click_at(button.rect.center());
    win.settle();
    assert_eq!(win.tab(), Tab::Files);
    // Every other page opens with nobody paired, too.
    for tab in [Tab::Wallpaper, Tab::Keys, Tab::Health, Tab::Settings, Tab::Arrange, Tab::Desk] {
        win.go(tab);
        assert!(problems(&win).is_empty(), "{tab:?}: {:#?}", problems(&win));
    }
}

#[test]
fn an_update_shows_a_pill_that_opens_settings() {
    let mut win = Win::light(1400.0, 800.0);
    win.status(Some(two()));
    assert!(win.find_containing("Update ready").is_none());
    win.send(Event::UpdateStatus(one_mk_ctl::update_logic::UpdateStatus { available: Some("9.9.9".into()), ..Default::default() }));
    let pill = win.find_containing("open Settings").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(pill.rect.center());
    win.settle();
    assert_eq!(win.tab(), Tab::Settings);
}

#[test]
fn a_computer_with_a_huge_name_is_cut_to_fit_and_its_full_name_stays_for_a_screen_reader() {
    let mut win = Win::light(820.0, 540.0);
    let name = "The very long name of the computer in the corner of the big office upstairs";
    win.status(Some(with(vec![peer("p1", name, Side::Right, os::WINDOWS, true)])));
    assert!(win.find_containing(name).is_some(), "{:#?}", win.names());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

// ---------- the keyboard alone ----------

#[test]
fn tab_walks_the_bar_in_reading_order_and_every_stop_is_named() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    let mut stops: Vec<String> = vec![];
    for _ in 0..14 {
        win.press(Key::Tab);
        let f = win.items().into_iter().find(|i| i.focused);
        let Some(f) = f else { continue };
        assert!(!f.name.is_empty(), "a focus stop with no name at {:?}", f.rect);
        assert!(win.window().contains_rect(f.rect.shrink(1.0)), "focus left the window: {:?} {:?}", f.name, f.rect);
        stops.push(f.name);
    }
    let first_tabs: Vec<&str> = stops.iter().map(String::as_str).take(7).collect();
    assert_eq!(first_tabs, ["Desk", "Arrange", "Wallpaper", "Clipboard & files", "Keys & mouse", "Health", "Settings"], "{stops:#?}");
    assert!(stops.iter().any(|s| s.starts_with("Search")), "{stops:#?}");
}

#[test]
fn enter_on_a_focused_tab_opens_it() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    for _ in 0..3 {
        win.press(Key::Tab);
    }
    let f = win.items().into_iter().find(|i| i.focused).expect("focus");
    assert_eq!(f.name, "Wallpaper");
    win.press(Key::Enter);
    win.settle();
    assert_eq!(win.tab(), Tab::Wallpaper);
}

#[test]
fn shift_tab_goes_back() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.press(Key::Tab);
    win.press(Key::Tab);
    win.key(Key::Tab, Modifiers::SHIFT);
    let f = win.items().into_iter().find(|i| i.focused).expect("focus");
    assert_eq!(f.name, "Desk");
}
