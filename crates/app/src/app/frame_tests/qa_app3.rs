//! QA flows for the window's own bookkeeping: Run commands and edge numbers
//! in Settings, the Wallpaper page while a choice is being prepared, and the
//! update row. Same stand-ins as the other flows (`qa.rs`).

use super::qa::{Fake, Socket};
use super::*;
use one_mk_ctl::update_logic::UpdateStatus;
use one_mk_wallpaper::{Command, Fit, Media, Plan, Spread};

fn settings_on(cat: settings_page::Cat) -> (Win, Fake) {
    let mut win = Win::light(1200.0, 1400.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.go(Tab::Settings);
    win.app.settings_cat = cat;
    win.settle();
    (win, fake)
}

#[test]
fn run_commands_follow_the_file_until_something_is_typed() {
    let (mut win, fake) = settings_on(settings_page::Cat::Commands);
    win.settings("[hooks]\non_enter = \"echo old\"\n");
    win.settle();
    assert!(win.has_text("echo old"), "{}", win.texts_joined());
    assert!(win.app.drafts.is_empty(), "nothing typed, nothing kept: {:?}", win.app.drafts);
    // The command is changed from outside (the command line) while the window is open.
    win.settings("[hooks]\non_enter = \"echo new\"\n");
    win.settle();
    assert!(win.has_text("echo new") && !win.has_text("echo old"), "{}", win.texts_joined());
    // Only the time limit is changed here: Save sends the command the file has now.
    let seconds = win.find("Time limit in seconds, 1 to 60").unwrap();
    win.click_at(seconds.rect.center());
    win.key(Key::A, Modifiers::COMMAND);
    win.type_text("9");
    win.click("Save command: Pointer arrives");
    win.quiet();
    let calls = fake.calls_with(&["hooks"]);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0][1..3], ["--on-enter", "echo new"], "{calls:?}");
    assert!(calls[0].contains(&"9".to_owned()), "{calls:?}");
}

#[test]
fn a_removed_command_leaves_no_choices_behind() {
    let (mut win, fake) = settings_on(settings_page::Cat::Commands);
    win.settings("[hooks]\non_enter = \"echo old\"\n[hooks.options.enter]\ntimeout_secs = 20\n");
    win.settle();
    let field = win.find("Command when pointer arrives").unwrap();
    win.click_at(field.rect.center());
    win.key(Key::A, Modifiers::COMMAND);
    win.press(Key::Backspace);
    win.click("Remove command: Pointer arrives");
    win.quiet();
    assert_eq!(fake.calls_with(&["hooks"]).len(), 1);
    win.settings("");
    win.settle();
    assert!(win.app.drafts.keys().all(|key| !key.starts_with("on_enter")), "{:?}", win.app.drafts);
}

#[test]
fn clicking_another_edge_does_not_save_the_number_typed_for_the_last_one() {
    let (mut win, fake) = settings_on(settings_page::Cat::Edges);
    let field = win.find("Wait at the edge, in milliseconds (0: off)").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(field.rect.center());
    win.type_text("700");
    let right = win.find("Which edges cross: Right").unwrap().rect.center();
    win.move_to(right);
    // Pressed and let go in the one frame: the side changes before the field is drawn.
    for pressed in [true, false] {
        win.push(egui::Event::PointerButton { pos: right, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    win.frame();
    win.frames(10);
    win.quiet();
    assert!(fake.calls_with(&["edge"]).is_empty(), "{:?}", fake.calls());
    assert!(!win.app.drafts.contains_key("edge-dwell"));
}

// ---------- Wallpaper ----------

fn wallpaper_status() -> Status {
    let mut status = two();
    let plan = Plan {
        hash: "a".repeat(64),
        title: "Across the horizon.mp4".into(),
        media: Media { width: 3840, height: 1080, duration_ms: 65_000, frame_rate: 30 },
        canvas: one_mk_ctl::wallpaper::canvas(&status),
        start_ms: 1,
        fit: Fit::Fill,
        computers: vec!["me".into(), "p1".into()],
        pause_on_battery: true,
        paused: false,
        picture: false,
        asset_hash: None,
        spread: Spread::Span,
    };
    status.wallpaper = Some(one_mk_wallpaper::Status { plan: Some(plan), message: "Playing".into(), available: true, ..Default::default() });
    status
}

fn wallpaper_window(reply: impl Fn(&str) -> String + Send + 'static) -> (Win, Socket) {
    let mut win = Win::light(1200.0, 900.0);
    let daemon = Socket::listen(&win, reply);
    win.status(Some(wallpaper_status()));
    win.go(Tab::Wallpaper);
    (win, daemon)
}

fn sent_wallpaper(daemon: &Socket) -> usize {
    daemon.seen_with("wallpaper-live ").len()
}

#[test]
fn while_a_new_wallpaper_is_prepared_the_other_changes_wait() {
    // The service takes its time over the one choice this test sends.
    let (mut win, daemon) = wallpaper_window(|line| {
        if !line.starts_with("wallpaper-live ") { return String::new(); }
        std::thread::sleep(std::time::Duration::from_millis(700));
        "ok".to_owned()
    });
    assert!(!win.find("Fit inside the screens").unwrap().disabled);
    let path = win.dir.join("loop.mp4");
    std::fs::write(&path, b"not really a video").unwrap();
    win.drop_files(std::slice::from_ref(&path));
    assert!(win.app.busy.iter().any(|job| matches!(job, Job::Wallpaper(Command::Set { .. }))), "{:?}", win.app.busy);
    assert!(win.find("Fit inside the screens").unwrap().disabled, "it would end the preparation");
    assert!(win.find("Choose…").unwrap().disabled);
    assert!(win.has_text("preparing the new one…"), "{}", win.texts_joined());
    // A second file let go meanwhile is not a second choice.
    win.drop_files(std::slice::from_ref(&path));
    win.wait("the preparation to end", |w| w.app.busy.is_empty());
    assert_eq!(sent_wallpaper(&daemon), 1, "{:?}", daemon.seen());
    win.frames(5);
    assert!(!win.find("Fit inside the screens").unwrap().disabled);
}

#[test]
fn no_control_can_end_a_wallpaper_that_is_being_prepared_except_turning_it_off() {
    let (mut win, daemon) = wallpaper_window(|line| {
        if !line.starts_with("wallpaper-live ") { return String::new(); }
        std::thread::sleep(std::time::Duration::from_millis(700));
        "ok".to_owned()
    });
    let path = win.dir.join("loop.mp4");
    std::fs::write(&path, b"not really a video").unwrap();
    win.drop_files(std::slice::from_ref(&path));
    assert!(win.app.busy.iter().any(|job| matches!(job, Job::Wallpaper(Command::Set { .. }))), "{:?}", win.app.busy);
    // Search's pause, a computer's switch, a second choice: none reach the service.
    win.app.wallpaper_command(Command::Pause { paused: true });
    win.app.wallpaper_command(Command::Computer { id: "p1".into(), enabled: false });
    win.app.wallpaper_command(Command::Set { path: path.clone() });
    assert!(win.toast_text().contains("still being prepared"), "{}", win.toast_text());
    // Turning it off is the way to cancel.
    win.app.wallpaper_command(Command::Off);
    win.wait("both requests to be answered", |_| sent_wallpaper(&daemon) >= 2);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let seen = daemon.seen_with("wallpaper-live ");
    assert_eq!(seen.len(), 2, "only the choice and the cancelling reached the service: {seen:?}");
    assert!(seen[1].contains("off"), "{seen:?}");
}

#[test]
fn a_wallpaper_change_the_service_dropped_says_so() {
    // The service answers a change it dropped for a newer one with nothing at all.
    let (mut win, _daemon) = wallpaper_window(|_| String::new());
    let path = win.dir.join("loop.mp4");
    std::fs::write(&path, b"not really a video").unwrap();
    win.drop_files(std::slice::from_ref(&path));
    win.wait("the failure", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't change wallpaper.");
}

#[test]
fn tabbing_through_the_wallpaper_choices_changes_nothing() {
    let (mut win, daemon) = wallpaper_window(|line| if line.starts_with("wallpaper-live ") { "ok".to_owned() } else { String::new() });
    for _ in 0..40 {
        win.press(Key::Tab);
    }
    assert_eq!(sent_wallpaper(&daemon), 0, "{:?}", daemon.seen());
}

// ---------- updates ----------

fn updates_window(status: UpdateStatus) -> (Win, Fake) {
    let (mut win, fake) = settings_on(settings_page::Cat::General);
    win.send(Event::UpdateStatus(status));
    (win, fake)
}

fn can_update() -> UpdateStatus {
    UpdateStatus { check: true, can_update: true, advice: "Download the new version.".into(), ..Default::default() }
}

#[test]
fn the_manual_check_is_busy_until_it_ends_and_cannot_be_started_twice() {
    let (mut win, fake) = updates_window(can_update());
    win.click_now("Check for updates");
    assert!(win.app.busy.contains(&Job::UpdateCheck), "{:?}", win.app.busy);
    win.frame();
    assert!(win.find("Checking…").unwrap().disabled);
    win.quiet();
    assert_eq!(fake.calls_with(&["update", "--check"]).len(), 1, "{:?}", fake.calls());
}

#[test]
fn a_failed_move_to_applications_can_be_tried_again() {
    let (mut win, _fake) = updates_window(UpdateStatus { move_to_applications: true, can_update: false, ..can_update() });
    assert!(win.find("Move to Applications").is_some_and(|b| !b.disabled), "{:#?}", win.names());
    // The move failed (the other copy was in the way): the failure shows, and so does the way to try again.
    win.send(Event::Done { what: Job::Update, error: Some("An app is already in Applications".into()) });
    win.settle();
    assert!(win.has_text("The update didn't finish"), "{}", win.texts_joined());
    assert!(win.find("Move to Applications").is_some_and(|b| !b.disabled), "{:#?}", win.names());
    assert!(win.find("Open the download page").is_some(), "{:#?}", win.names());
}

#[test]
fn search_and_health_do_not_install_where_the_window_may_not() {
    // Running from the disk image: the way in is to move it first, in Settings.
    let (mut win, fake) = updates_window(UpdateStatus { move_to_applications: true, can_update: false, ..can_update() });
    win.go(Tab::Health);
    win.app.begin_update();
    win.frames(3);
    assert!(!win.app.busy.contains(&Job::Update));
    assert_eq!(win.app.tab, Tab::Settings);
    assert!(fake.calls_with(&["update"]).is_empty(), "{:?}", fake.calls());
}

#[test]
fn turning_off_automatic_updates_is_not_raced_by_the_install() {
    let status = UpdateStatus { install_automatically: true, ..can_update() };
    let (mut win, fake) = updates_window(status);
    win.app.update_available = Some("99.0.0".into());
    // The opt-out is on its way: its write has not come back.
    win.app.busy.push(Job::UpdateSettings);
    win.frames(5);
    assert!(!win.app.busy.contains(&Job::Update), "the old setting must not act");
    assert!(fake.calls_with(&["update"]).is_empty(), "{:?}", fake.calls());
    // Had it stayed on, it installs once the write is done.
    win.app.busy.retain(|job| job != &Job::UpdateSettings);
    win.frames(3);
    win.quiet();
    assert!(!fake.calls_with(&["update"]).is_empty(), "{:?}", fake.calls());
}

#[test]
fn a_stopped_onemk_does_not_say_no_computer_is_paired() {
    let mut win = Win::light(1200.0, 1400.0);
    win.status(None);
    win.go(Tab::Settings);
    win.app.settings_cat = settings_page::Cat::Typing;
    win.settle();
    assert!(win.has_text("Start oneMK to choose how Ctrl and Cmd work"), "{}", win.texts_joined());
    assert!(!win.has_text("Pair a computer to choose how Ctrl and Cmd work"));
    win.status(Some(alone()));
    assert!(win.has_text("Pair a computer to choose how Ctrl and Cmd work"), "{}", win.texts_joined());
}

#[test]
fn keys_for_a_computer_with_a_shown_id_name_it_by_its_id() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(with(vec![
        peer("ab12-cd34", "Studio (ab12-cd34)", Side::Right, os::WINDOWS, true),
        peer("ee00-ff11", "Studio (ee00-ff11)", Side::Left, os::LINUX, true),
    ])));
    let tile = win.find_containing("Studio (ab12-cd34), Windows").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(tile.rect.center());
    win.settle();
    let keys = win.find_containing("Keys for this PC").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(keys.rect.center());
    assert_eq!(win.app.tab, Tab::Keys);
    assert_eq!(win.app.keys.scope, crate::tune::Scope::Peer("ab12-cd34".into()));
}

#[test]
fn health_names_what_its_update_button_does_where_it_cannot_install() {
    let check = crate::view::Check {
        id: "updates".into(),
        title: "Updates".into(),
        verdict: crate::view::Verdict::Warn,
        detail: "A new version is out.".into(),
        fix: Some("update-now".into()),
        action: None,
    };
    for (can, move_first, label) in [(true, false, "Update now"), (false, false, "Download"), (false, true, "Open Settings")] {
        let (mut win, _fake) = updates_window(UpdateStatus { can_update: can, move_to_applications: move_first, ..can_update() });
        win.go(Tab::Health);
        win.health_check_ends();
        win.send(Event::Doctor(Ok(vec![check.clone()])));
        assert!(win.find(label).is_some(), "can_update {can}, move first {move_first}: {:#?}", win.names());
        assert!(win.find("Update now").is_some() == (label == "Update now"));
    }
}

#[test]
fn layout_changes_reach_one_mk_in_the_order_they_were_asked() {
    let (win, fake) = settings_on(settings_page::Cat::General);
    let sides = [Side::Left, Side::Right, Side::Top, Side::Bottom];
    for n in 0..24 {
        win.app.link.set_side("p1", "Office PC", sides[n % 4], None, 2 * n as u64);
        win.app.link.set_swap("p1", "Office PC", if n % 2 == 0 { "on" } else { "off" }, 2 * n as u64 + 1);
    }
    for _ in 0..400 {
        if fake.calls_with(&["layout"]).len() >= 48 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let calls = fake.calls_with(&["layout"]);
    assert_eq!(calls.len(), 48, "{calls:?}");
    for (n, pair) in calls.chunks(2).enumerate() {
        assert_eq!(pair[0][2], sides[n % 4].to_string(), "call {n}: {calls:?}");
        assert_eq!(pair[1][2], "--swap-ctrl-cmd", "call {n}: {calls:?}");
        assert_eq!(pair[1][3], if n % 2 == 0 { "on" } else { "off" }, "call {n}: {calls:?}");
    }
}

#[test]
fn connecting_to_a_name_looks_it_up_beside_the_window() {
    let (mut win, fake) = settings_on(settings_page::Cat::General);
    win.app.pairing = Some(PairUi { proc: None, step: Step::Searching { me: "Studio".into(), port: None }, since: win.t, opened: win.t - 1.0, hint: false, retry: None, me: "Studio".into(), port: None });
    let now = win.t;
    win.app.begin_pairing(crate::pairing::Method::Connect("localhost:32123".into()), now);
    assert!(win.app.pair_lookup.is_some(), "the name is being looked up, not waited for");
    assert!(win.app.pairing.as_ref().is_some_and(|pair| pair.proc.is_none()));
    for _ in 0..300 {
        win.frames(2);
        if !fake.calls_with(&["pair"]).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let calls = fake.calls_with(&["pair"]);
    assert_eq!(calls.len(), 1, "{:?}", fake.calls());
    assert_eq!(calls[0][..3], ["pair", "--machine", "--connect"], "{calls:?}");
    assert!(calls[0][3].ends_with(":32123") && !calls[0][3].contains("localhost"), "the address it found: {calls:?}");
    assert!(win.app.pair_lookup.is_none());
}

#[test]
fn a_name_that_cannot_be_found_ends_pairing_with_a_message() {
    let (mut win, fake) = settings_on(settings_page::Cat::General);
    win.app.pairing = Some(PairUi { proc: None, step: Step::Searching { me: "Studio".into(), port: None }, since: win.t, opened: win.t - 1.0, hint: false, retry: None, me: "Studio".into(), port: None });
    let now = win.t;
    win.app.begin_pairing(crate::pairing::Method::Connect("no-such-host.invalid:32123".into()), now);
    for _ in 0..400 {
        win.frames(2);
        if matches!(win.app.pairing.as_ref().map(|pair| &pair.step), Some(Step::Ended(_))) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(matches!(win.app.pairing.as_ref().map(|pair| &pair.step), Some(Step::Ended(_))), "{:?}", win.app.pairing.as_ref().map(|pair| &pair.step));
    assert!(fake.calls_with(&["pair"]).is_empty(), "{:?}", fake.calls());
    assert!(win.app.toast.as_ref().is_some_and(|toast| toast.error));
}

#[test]
fn an_older_layout_failure_keeps_the_newer_place_on_the_desk() {
    let mut win = Win::light(1200.0, 780.0);
    let _fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.choose_place("p1", Side::Left, None, false);
    win.app.choose_place("p1", Side::Top, None, false);
    let jobs: Vec<Job> = win.app.busy.iter().filter(|j| matches!(j, Job::Layout { .. })).cloned().collect();
    let [left, top] = jobs.as_slice() else { panic!("{jobs:?}") };
    win.send(Event::Done { what: left.clone(), error: Some("busy".into()) });
    assert_eq!(win.app.side_shown("p1", Side::Right), Side::Top, "the newer change is still on its way");
    win.send(Event::Done { what: top.clone(), error: Some("busy".into()) });
    assert_eq!(win.app.side_shown("p1", Side::Right), Side::Right, "the last one failed: the desk goes back");
}

#[test]
fn an_older_exact_move_failing_keeps_a_newer_exact_move_on_the_same_side() {
    let mut win = Win::light(1200.0, 780.0);
    let _fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.choose_place("p1", Side::Left, Some((0, 100)), false);
    win.app.choose_place("p1", Side::Left, Some((0, 300)), false);
    let jobs: Vec<Job> = win.app.busy.iter().filter(|j| matches!(j, Job::Layout { .. })).cloned().collect();
    let [first, second] = jobs.as_slice() else { panic!("{jobs:?}") };
    assert_ne!(first, second, "two moves to the same side are told apart");
    win.send(Event::Done { what: first.clone(), error: Some("busy".into()) });
    assert_eq!(win.app.place_of("p1"), Some((Side::Left, Some((0, 300)))), "the newer exact place stays");
    assert!(win.app.toast.as_ref().is_some_and(|t| t.retry.is_none()), "an older move is not offered again");
}

#[test]
fn trying_a_failed_move_again_never_undoes_a_newer_one() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.choose_place("p1", Side::Left, Some((0, 100)), false);
    let failed = win.app.busy.iter().find(|j| matches!(j, Job::Layout { .. })).cloned().expect("running");
    win.send(Event::Done { what: failed.clone(), error: Some("busy".into()) });
    assert!(win.app.toast.as_ref().is_some_and(|t| t.retry.as_ref() == Some(&failed)), "the exact move can be tried again");
    win.app.choose_place("p1", Side::Top, None, false);
    for _ in 0..400 {
        if fake.calls_with(&["layout"]).len() >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    win.app.retry(failed);
    win.quiet();
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert_eq!(win.app.place_of("p1"), Some((Side::Top, None)), "the newer choice stands");
    assert_eq!(fake.calls_with(&["layout"]).len(), 2, "nothing was asked again: {:?}", fake.calls_with(&["layout"]));
}

#[test]
fn a_failed_exact_move_is_tried_again_with_its_own_offset() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.choose_place("p1", Side::Left, Some((0, 100)), false);
    let failed = win.app.busy.iter().find(|j| matches!(j, Job::Layout { .. })).cloned().expect("running");
    win.send(Event::Done { what: failed.clone(), error: Some("busy".into()) });
    win.app.retry(failed);
    assert_eq!(win.app.place_of("p1"), Some((Side::Left, Some((0, 100)))));
    win.quiet();
    assert!(fake.calls_with(&["layout"]).iter().any(|call| call.contains(&"--offset=0,100".to_owned())), "{:?}", fake.calls_with(&["layout"]));
}

#[test]
fn an_older_swap_failure_keeps_a_newer_choice_even_when_the_same_value_repeats() {
    let mut win = Win::light(1200.0, 780.0);
    let _fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.ask_swap("p1", "Office PC", "on");
    win.app.ask_swap("p1", "Office PC", "off");
    win.app.ask_swap("p1", "Office PC", "on");
    let jobs: Vec<Job> = win.app.busy.iter().filter(|j| matches!(j, Job::Swap { .. })).cloned().collect();
    assert_eq!(jobs.len(), 3, "{jobs:?}");
    win.send(Event::Done { what: jobs[0].clone(), error: Some("busy".into()) });
    assert_eq!(win.app.pending_swap.get("p1").map(String::as_str), Some("on"), "the last choice is still on its way");
    win.send(Event::Done { what: jobs[2].clone(), error: Some("busy".into()) });
    assert!(win.app.pending_swap.is_empty(), "the last one failed: the row goes back");
}
