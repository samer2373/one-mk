//! QA click-through flows: the update pill and install, Settings, first run.
//! (The stand-in for `one-mk` is in `qa.rs`: `update --status-json` prints
//! what a test tells it to, and `update` itself prints its progress lines.)

use super::qa::{Fake, args};
use super::*;
use one_mk_ctl::update_logic::UpdateStatus;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn status_json(status: &UpdateStatus) -> String {
    serde_json::to_string(status).unwrap()
}

fn settings_window() -> (Win, Fake) {
    let mut win = Win::light(1200.0, 900.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.go(Tab::Settings);
    (win, fake)
}

fn on_this_version() -> UpdateStatus {
    UpdateStatus { check: true, can_update: true, advice: "Download the new version.".into(), ..Default::default() }
}

fn newer(version: &str) -> UpdateStatus {
    UpdateStatus { available: Some(version.into()), ..on_this_version() }
}

#[test]
fn until_the_update_state_is_known_settings_says_it_is_looking_and_a_failure_is_said() {
    let (mut win, fake) = settings_window();
    assert!(win.has_text("Checking how updates work here…"), "{}", win.texts_joined());
    // The program that tells it can't be asked: that is said, not left looking forever.
    fake.fail("update---status-json", "one-mk: unknown option");
    win.app.link.update_status();
    win.wait("the answer", |w| w.app.update_output.is_some());
    assert!(win.has_text("Couldn't read update preferences"), "{}", win.texts_joined());
    assert!(!win.has_text("Checking how updates work here…"), "{}", win.texts_joined());
}

#[test]
fn check_for_updates_says_up_to_date_and_a_failed_check_says_so() {
    let (mut win, fake) = settings_window();
    fake.prints("update---status-json", &status_json(&on_this_version()));
    win.send(Event::UpdateStatus(on_this_version()));
    assert!(win.find("Check for updates").is_some(), "{:#?}", win.names());
    assert!(win.has_text(&format!("oneMK {VERSION}")), "{}", win.texts_joined());
    assert!(!win.has_text("You're up to date"));
    win.click("Check for updates");
    win.quiet();
    assert_eq!(fake.calls_with(&["update", "--check"]).len(), 1, "{:?}", fake.calls());
    assert!(win.has_text(&format!("You're up to date. oneMK {VERSION}.")), "{}", win.texts_joined());
    // The check itself fails (offline).
    fake.fail("update---check", "one-mk: could not reach the release server");
    win.click("Check for updates");
    win.wait("the failure", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't check for updates.");
}

#[test]
fn a_newer_version_shows_the_pill_the_button_and_the_sentence() {
    let (mut win, fake) = settings_window();
    fake.prints("update---status-json", &status_json(&newer("9.9.9")));
    win.send(Event::UpdateStatus(newer("9.9.9")));
    assert!(win.find("Update to 9.9.9").is_some(), "{:#?}", win.names());
    assert!(win.has_text(&format!("oneMK 9.9.9 is available. You have {VERSION}.")), "{}", win.texts_joined());
    assert!(win.has_text("Update ready"), "the pill in the top bar: {}", win.texts_joined());
    assert!(win.find_containing("open Settings").is_some());
    assert!(
        problems(&win).is_empty() && label_not_in_name(&win).is_empty(),
        "{:#?} {:#?}",
        problems(&win),
        label_not_in_name(&win)
    );
}

#[test]
fn installing_shows_progress_then_the_new_state_and_leaves_no_failure() {
    let (mut win, fake) = settings_window();
    fake.prints("update---status-json", &status_json(&on_this_version()));
    fake.prints("update-_", "Downloading… 40%\nDownloading… 100%\nInstalled 9.9.9.\n");
    win.send(Event::UpdateStatus(newer("9.9.9")));
    win.click_now("Update to 9.9.9");
    assert!(win.app.busy.contains(&Job::Update));
    assert!(
        matches!(win.app.update_box(), Some(super::super::updates::UpdateBox::Working { .. })),
        "the pill and the button say it is working"
    );
    win.wait("the update", |w| !w.app.busy.contains(&Job::Update));
    assert_eq!(
        fake.calls_with(&["update"]).iter().filter(|c| c.len() == 1).count(),
        1,
        "one install: {:?}",
        fake.calls()
    );
    assert!(!win.app.update_ui.failed);
    // The program then reports there is nothing newer.
    assert!(win.find("Check for updates").is_some() || win.find("Update to 9.9.9").is_some(), "{:#?}", win.names());
}

#[test]
fn the_install_shows_how_far_it_is_and_who_is_working() {
    let (mut win, _fake) = settings_window();
    win.send(Event::UpdateStatus(newer("9.9.9")));
    win.app.busy.push(Job::Update);
    win.send(Event::UpdateOutput("Downloading… 40%".into()));
    assert!(win.has_text("Updating…"), "{}", win.texts_joined());
    let bar = win
        .items()
        .into_iter()
        .find(|i| i.role == accesskit::Role::ProgressIndicator)
        .unwrap_or_else(|| panic!("{:#?}", win.names()));
    assert_eq!(bar.name, "Downloading… 40%");
    assert!(win.find("Updating…").is_none_or(|b| b.disabled), "the button waits");
    // Without a percentage: a bead that travels, named by what is said.
    win.send(Event::UpdateOutput("Verifying the download…".into()));
    let bar = win.items().into_iter().find(|i| i.role == accesskit::Role::ProgressIndicator).unwrap();
    assert_eq!(bar.name, "Verifying the download…");
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    win.app.busy.retain(|j| j != &Job::Update);
}

#[test]
fn a_failed_install_says_what_happened_and_offers_download_and_trying_again() {
    let (mut win, fake) = settings_window();
    fake.fail("update-_", "one-mk: the download did not match its signature");
    win.send(Event::UpdateStatus(newer("9.9.9")));
    win.click("Update to 9.9.9");
    win.wait("the failure", |w| w.app.update_ui.failed);
    assert!(!win.app.busy.contains(&Job::Update));
    assert!(win.has_text("The update didn't finish"), "{}", win.texts_joined());
    assert!(win.has_text("the download did not match its signature"), "{}", win.texts_joined());
    assert!(win.has_text("Update didn't finish"), "the pill says it too: {}", win.texts_joined());
    assert!(win.find("Open the download page").is_some(), "a way to get it by hand: {:#?}", win.names());
    assert!(
        problems(&win).is_empty() && label_not_in_name(&win).is_empty(),
        "{:#?} {:#?}",
        problems(&win),
        label_not_in_name(&win)
    );
    // Trying again clears the failure while it runs.
    fake.mend("update-_");
    fake.prints("update-_", "Installed 9.9.9.\n");
    win.click("Update to 9.9.9");
    assert!(!win.app.update_ui.failed);
    win.wait("the second try", |w| !w.app.busy.contains(&Job::Update));
    assert!(!win.app.update_ui.failed);
}

#[test]
fn a_failed_install_with_nothing_said_still_offers_a_way_out() {
    let (mut win, _fake) = settings_window();
    win.send(Event::UpdateStatus(newer("9.9.9")));
    win.send(Event::Done { what: Job::Update, error: Some(String::new()) });
    assert!(win.app.update_ui.failed);
    assert!(win.has_text("Get the new version from the download page."), "{}", win.texts_joined());
}

#[test]
fn where_updating_cannot_be_done_here_the_button_downloads() {
    let (mut win, _fake) = settings_window();
    let status =
        UpdateStatus { can_update: false, advice: "Update with your package manager.".into(), ..newer("9.9.9") };
    win.send(Event::UpdateStatus(status));
    assert!(win.find("Download").is_some(), "{:#?}", win.names());
    assert!(win.find("Update to 9.9.9").is_none());
    assert!(win.has_text("Update with your package manager."), "{}", win.texts_joined());
    assert!(win.find("Install them without asking").unwrap().disabled);
}

#[test]
fn an_install_that_needs_a_password_is_never_automatic() {
    let (mut win, fake) = settings_window();
    let status = UpdateStatus { needs_password: true, install_automatically: true, ..newer("9.9.9") };
    win.send(Event::UpdateStatus(status));
    win.frames(30);
    assert!(fake.calls_with(&["update"]).is_empty(), "{:?}", fake.calls());
    assert!(win.has_text("Installing asks for your password"), "{}", win.texts_joined());
    assert!(win.find("Install them without asking").unwrap().disabled);
}

#[test]
fn an_automatic_install_starts_by_itself_once_per_version() {
    let (mut win, fake) = settings_window();
    fake.prints("update---status-json", &status_json(&UpdateStatus { install_automatically: true, ..newer("9.9.9") }));
    fake.prints("update-_", "Installed 9.9.9.\n");
    win.send(Event::UpdateStatus(UpdateStatus { install_automatically: true, ..newer("9.9.9") }));
    win.wait("the install", |w| {
        !fake.calls_with(&["update"]).iter().all(|c| c.len() > 1) && !w.app.busy.contains(&Job::Update)
    });
    win.frames(60);
    let installs = fake.calls_with(&["update"]).into_iter().filter(|c| c.len() == 1).count();
    assert_eq!(installs, 1, "{:?}", fake.calls());
    // A version the person went back from is not installed again by itself.
    win.send(Event::UpdateStatus(UpdateStatus {
        install_automatically: true,
        skipped: Some("9.9.10".into()),
        ..newer("9.9.10")
    }));
    win.frames(30);
    let installs = fake.calls_with(&["update"]).into_iter().filter(|c| c.len() == 1).count();
    assert_eq!(installs, 1, "{:?}", fake.calls());
}

#[test]
fn the_two_update_switches_run_their_commands_and_a_refusal_says_so() {
    let (mut win, fake) = settings_window();
    fake.prints("update---status-json", &status_json(&on_this_version()));
    win.send(Event::UpdateStatus(on_this_version()));
    win.click("Look for updates");
    win.quiet();
    assert_eq!(fake.calls_with(&["update", "--check-on-start"]), vec![args(&["update", "--check-on-start", "off"])]);
    win.click("Install them without asking");
    win.quiet();
    assert_eq!(
        fake.calls_with(&["update", "--install-automatically"]),
        vec![args(&["update", "--install-automatically", "on"])]
    );
    fake.fail("update---check-on-start", "one-mk: settings.toml is read-only");
    win.click("Look for updates");
    win.wait("the refusal", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't save update preferences.");
}

#[test]
fn going_back_to_the_previous_version_is_one_command_and_moving_the_app_another() {
    let (mut win, fake) = settings_window();
    fake.prints("update---status-json", &status_json(&on_this_version()));
    win.send(Event::UpdateStatus(UpdateStatus { rollback_available: true, ..on_this_version() }));
    win.click("Go back to the previous version");
    win.wait("the command", |_| !fake.calls_with(&["update", "--rollback"]).is_empty());
    win.quiet();
    assert_eq!(fake.calls_with(&["update", "--rollback"]), vec![args(&["update", "--rollback"])]);
    win.send(Event::UpdateStatus(UpdateStatus { move_to_applications: true, ..newer("9.9.9") }));
    assert!(win.has_text("It's running from the disk image"), "{}", win.texts_joined());
    win.click("Move to Applications");
    win.wait("the command", |_| !fake.calls_with(&["update", "--move-to-applications"]).is_empty());
    win.quiet();
}

// ---------- first run ----------
//
// `Link::first_run` changes the real login items, so no test calls it: the
// window is told what setup did (`Event::Setup`), as it is when it runs.

use one_mk_ctl::setup::{Report, Step};

fn setup(login: Step, integrations: Step, service: Step) -> Event {
    Event::Setup(Report { login, integrations, service })
}

#[test]
fn first_run_that_set_things_up_says_so_once_and_one_that_changed_nothing_says_nothing() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    win.send(setup(Step::Done, Step::Done, Step::Done));
    assert_eq!(win.toast_text(), "oneMK is set up and starts when you log in.");
    assert!(!win.app.toast.as_ref().unwrap().error);
    // It goes by itself.
    win.frames(60 * 5);
    assert!(win.app.toast.is_none(), "{:?}", win.app.toast.as_ref().map(|t| &t.text));
    // Nothing changed (it was set up already, or there is no file manager).
    win.send(setup(Step::Already, Step::Skipped("no supported file manager".into()), Step::Already));
    assert!(win.app.toast.is_none());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn first_run_that_could_not_finish_says_which_step_and_why() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    win.send(setup(
        Step::Failed("The sharing service could not be found next to this app.".into()),
        Step::Done,
        Step::Already,
    ));
    let toast = win.app.toast.as_ref().expect("a toast");
    assert!(toast.error);
    assert_eq!(
        toast.text,
        "Setup didn't finish. Start at login: The sharing service could not be found next to this app."
    );
    win.send(setup(Step::Done, Step::Failed("no write access to ~/.local/share/file-manager".into()), Step::Done));
    assert!(win.toast_text().starts_with("Setup didn't finish. File manager menu: "), "{}", win.toast_text());
}

#[test]
fn first_run_whose_service_would_not_start_offers_try_again_and_the_log() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(None);
    win.send(setup(Step::Done, Step::Done, Step::Failed("Network port 24800 is in use by another program".into())));
    assert_eq!(win.toast_text(), "oneMK couldn't start.");
    let toast = win.app.toast.as_ref().unwrap();
    assert_eq!(toast.retry, Some(Job::Start));
    assert!(
        win.find_containing("Try again").is_some() && win.find_containing("Show log").is_some(),
        "{:#?}",
        win.names()
    );
}

#[test]
fn the_start_at_login_switch_waits_until_it_is_known_and_then_tells_the_truth() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    win.go(Tab::Settings);
    assert!(win.app.autostart.is_none());
    assert!(win.find("Start at login").unwrap().disabled, "not known yet: pressing it would guess");
    win.send(Event::Autostart(true));
    let on = win.find("Start at login").unwrap();
    assert!(!on.disabled);
    win.send(Event::Autostart(false));
    assert_eq!(win.app.autostart, Some(false));
}

#[test]
fn a_computer_alone_is_offered_its_first_pairing_and_what_works_without_one() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    assert!(win.has_text("Pair your first computer"), "{}", win.texts_joined());
    assert!(win.has_text("Or use oneMK on this computer alone"), "{}", win.texts_joined());
    for what in ["Clipboard history, on this computer", "Wallpaper"] {
        assert!(win.find_containing(what).is_some(), "{what}: {:#?}", win.names());
    }
    assert!(problems(&win).is_empty() && label_not_in_name(&win).is_empty());
}

// ---------- the permission guide ----------

fn missing(which: &[&str]) -> Status {
    let mut status = two();
    status.input = which.iter().map(|w| (*w).to_owned()).collect();
    status
}

const NEEDS_ACCESSIBILITY: &str = "one-mk needs Accessibility permission to share this Mac's mouse and keyboard.";
const NEEDS_INPUT_MONITORING: &str = "one-mk needs Input Monitoring permission to send this Mac's mouse and keyboard.";

#[test]
fn a_missing_permission_is_said_on_the_desk_and_in_health_and_goes_away_when_given() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(missing(&[NEEDS_ACCESSIBILITY])));
    assert!(win.has_text("Waiting for this computer's permission"), "{}", win.texts_joined());
    assert!(win.app.health_trouble(), "the bar points at Health");
    win.go(Tab::Health);
    win.send(Event::Doctor(Ok(vec![])));
    assert!(
        win.has_text("Allow Accessibility in System Settings to share this Mac's mouse and keyboard."),
        "{}",
        win.texts_joined()
    );
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    win.status(Some(missing(&[])));
    assert!(!win.has_text("Allow Accessibility in System Settings"), "{}", win.texts_joined());
    assert!(!win.app.health_trouble());
}

#[test]
fn the_permission_guide_walks_both_permissions_and_leaves_when_both_are_given() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(missing(&[NEEDS_ACCESSIBILITY])));
    win.go(Tab::Health);
    let check = crate::view::Check {
        id: "input".into(),
        title: "Permissions".into(),
        verdict: crate::view::Verdict::Warn,
        detail: "Allow Accessibility.".into(),
        fix: Some("Allow it in System Settings.".into()),
        action: Some("OpenPermissionGuide".into()),
    };
    win.send(Event::Doctor(Ok(vec![check])));
    assert!(!win.app.guide.active());
    win.click("Allow sharing");
    assert!(win.app.guide.active(), "{:#?}", win.names());
    win.frames(30);
    assert!(win.app.guide.active(), "waiting for the switch in System Settings");
    // Accessibility is on; Input Monitoring is the next one asked for.
    win.status(Some(missing(&[NEEDS_INPUT_MONITORING])));
    win.frames(120);
    assert!(win.app.guide.active());
    // Both on: the guide ticks, holds a moment and leaves.
    win.status(Some(missing(&[])));
    win.frames(60 * 8);
    assert!(!win.app.guide.active(), "it left once everything was allowed");
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn off_macos_a_missing_permission_never_pops_the_guide_up_by_itself() {
    // The guide opens by itself only on a Mac: elsewhere the permission is the desktop's own dialog.
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(missing(&[NEEDS_ACCESSIBILITY, NEEDS_INPUT_MONITORING])));
    win.frames(60);
    assert_eq!(win.app.guide.active(), cfg!(target_os = "macos"));
}

#[test]
fn a_live_missing_permission_line_has_the_same_open_guide_button_as_a_check() {
    // No doctor check reports anything: the daemon's own line is all Health has, and it must still say
    // where to go, with the button a doctor-reported "Permissions" row gets.
    for line in [NEEDS_ACCESSIBILITY, NEEDS_INPUT_MONITORING] {
        let mut win = Win::light(1200.0, 780.0);
        win.status(Some(missing(&[line])));
        win.go(Tab::Health);
        win.send(Event::Doctor(Ok(vec![])));
        win.frames(2);
        assert!(win.has_text("Open guide"), "{line}: {}", win.texts_joined());
        assert!(!win.app.guide.active());
        win.click("Open guide");
        win.frames(2);
        if cfg!(target_os = "macos") {
            assert!(win.app.guide.active(), "the same guide as the check's button");
        } else {
            // Elsewhere the permission is the desktop's own dialog: the same steps beside the button.
            assert!(win.has_text("Look for your desktop's sharing window"), "{}", win.texts_joined());
        }
        assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
        // Allowed: the line and its button go away.
        win.status(Some(missing(&[])));
        win.frames(2);
        assert!(!win.has_text("Open guide"), "{}", win.texts_joined());
    }
}
