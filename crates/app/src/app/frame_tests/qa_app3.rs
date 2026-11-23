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
