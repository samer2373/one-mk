//! QA click-through flows: the Wallpaper page, with the wallpaper helper
//! unavailable and available. The daemon is a socket that writes down every
//! wallpaper command it is sent (Choose… opens a file dialog, which a test
//! must not: a file let go over the page is the same choice).

use super::qa::Socket;
use super::*;
use one_mk_wallpaper::{Command, Fit, Media, Plan, Playback, Spread};

fn wallpaper_status(available: bool, plan: Option<Plan>, message: &str) -> Status {
    let mut status = two();
    status.wallpaper =
        Some(one_mk_wallpaper::Status { plan, message: message.into(), available, ..Default::default() });
    status
}

fn plan(status: &Status, picture: bool, paused: bool) -> Plan {
    Plan {
        hash: "a".repeat(64),
        title: "Across the horizon.mp4".into(),
        media: Media { width: 3840, height: 1080, duration_ms: 65_000, frame_rate: 30 },
        canvas: one_mk_ctl::wallpaper::canvas(status),
        start_ms: 1,
        fit: Fit::Fill,
        computers: vec!["me".into(), "p1".into()],
        pause_on_battery: true,
        paused,
        picture,
        asset_hash: None,
        spread: Spread::Span,
    }
}

/// The wallpaper commands the daemon was sent, in order.
fn commands(daemon: &Socket) -> Vec<Command> {
    daemon
        .seen_with("wallpaper-live ")
        .iter()
        .map(|line| serde_json::from_str(line.trim_start_matches("wallpaper-live ")).expect("a command"))
        .collect()
}

fn window(status: Status) -> (Win, Socket) {
    let mut win = Win::light(1200.0, 900.0);
    let daemon =
        Socket::listen(&win, |line| if line.starts_with("wallpaper-live ") { "ok".to_owned() } else { String::new() });
    win.status(Some(status));
    win.go(Tab::Wallpaper);
    (win, daemon)
}

fn drop_video(win: &mut Win) -> PathBuf {
    let path = win.dir.join("loop.mp4");
    std::fs::write(&path, b"not really a video").unwrap();
    win.drop_files(std::slice::from_ref(&path));
    path
}

#[test]
fn without_the_helper_nothing_can_be_chosen_and_the_page_says_why() {
    let (mut win, daemon) = window(wallpaper_status(false, None, "The wallpaper helper is not installed"));
    assert!(win.has_text("Wallpaper isn't available on this computer"), "{}", win.texts_joined());
    assert!(
        win.has_text("The wallpaper helper is not installed"),
        "the daemon's reason is shown: {}",
        win.texts_joined()
    );
    assert!(win.find("Choose…").unwrap().disabled);
    assert!(win.find("Fill the screens").unwrap().disabled, "nothing to size yet");
    // A file let go over the page is the same as Choose…: refused too.
    drop_video(&mut win);
    win.frames(20);
    assert!(commands(&daemon).is_empty(), "{:?}", daemon.seen());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn while_onemk_is_off_the_page_asks_to_start_it() {
    let mut win = Win::light(1200.0, 900.0);
    let daemon = Socket::listen(&win, |_| String::new());
    win.status(None);
    win.go(Tab::Wallpaper);
    assert!(win.has_text("Start oneMK to choose a wallpaper"), "{}", win.texts_joined());
    assert!(win.find("Choose…").unwrap().disabled);
    drop_video(&mut win);
    assert!(daemon.seen().is_empty());
}

#[test]
fn a_picture_or_video_let_go_on_the_page_is_chosen() {
    let (mut win, daemon) = window(wallpaper_status(true, None, ""));
    assert!(win.has_text("Choose a video or picture"), "{}", win.texts_joined());
    assert!(!win.find("Choose…").unwrap().disabled);
    assert!(win.find("Fill the screens").unwrap().disabled, "sizing waits for a choice");
    let path = drop_video(&mut win);
    win.wait("the command", |_| !daemon.seen().is_empty());
    assert_eq!(commands(&daemon), vec![Command::Set { path }]);
}

#[test]
fn a_refused_choice_says_so_and_try_again_sends_the_same_command() {
    let mut win = Win::light(1200.0, 900.0);
    let daemon = Socket::listen(&win, |line| {
        if line.starts_with("wallpaper-live ") {
            "error The video has no picture track".to_owned()
        } else {
            String::new()
        }
    });
    win.status(Some(wallpaper_status(true, None, "")));
    win.go(Tab::Wallpaper);
    let path = drop_video(&mut win);
    win.wait("the refusal", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't change wallpaper.");
    assert!(
        win.app.toast.as_ref().unwrap().detail.as_deref().is_some_and(|d| d.contains("The video has no picture track")),
        "{:?}",
        win.app.toast.as_ref().map(|t| &t.detail)
    );
    assert!(win.app.toast.as_ref().unwrap().retry.is_some(), "Try again is offered");
    let again = win.find_containing("Try again").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(again.rect.center());
    win.wait("the second command", |_| daemon.seen_with("wallpaper-live ").len() >= 2);
    assert_eq!(commands(&daemon), vec![Command::Set { path: path.clone() }, Command::Set { path }]);
}

#[test]
fn a_playing_video_can_be_paused_played_and_turned_off() {
    let status = wallpaper_status(true, None, "");
    let playing = Status {
        wallpaper: Some(one_mk_wallpaper::Status {
            plan: Some(plan(&status, false, false)),
            available: true,
            ..Default::default()
        }),
        ..status.clone()
    };
    let (mut win, daemon) = window(playing.clone());
    assert!(win.has_text("Across the horizon.mp4"), "{}", win.texts_joined());
    assert!(win.has_text("1:05 loop"), "the loop's length: {}", win.texts_joined());
    win.click("Pause");
    win.wait("the command", |_| !daemon.seen().is_empty());
    assert_eq!(commands(&daemon), vec![Command::Pause { paused: true }]);
    // The daemon says it is paused: the button turns into Play and the line says so.
    let paused = Status {
        wallpaper: Some(one_mk_wallpaper::Status {
            plan: Some(plan(&status, false, true)),
            available: true,
            ..Default::default()
        }),
        ..status.clone()
    };
    win.status(Some(paused));
    assert!(win.find("Pause").is_none() && win.find("Play").is_some(), "{:#?}", win.names());
    assert!(win.has_text("paused"), "{}", win.texts_joined());
    win.click("Play");
    win.wait("the command", |_| daemon.seen().len() >= 2);
    assert_eq!(commands(&daemon).last(), Some(&Command::Pause { paused: false }));
    win.click("Turn off");
    win.wait("the command", |_| daemon.seen().len() >= 3);
    assert_eq!(commands(&daemon).last(), Some(&Command::Off));
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn a_picture_has_no_pause_but_can_be_turned_off() {
    let status = wallpaper_status(true, None, "");
    let picture = Status {
        wallpaper: Some(one_mk_wallpaper::Status {
            plan: Some(plan(&status, true, false)),
            available: true,
            ..Default::default()
        }),
        ..status
    };
    let (mut win, daemon) = window(picture);
    assert!(win.find("Pause").is_none() && win.find("Play").is_none(), "{:#?}", win.names());
    assert!(win.find("Pause on battery").is_none_or(|b| b.disabled), "a picture never pauses");
    win.click("Turn off");
    win.wait("the command", |_| !daemon.seen().is_empty());
    assert_eq!(commands(&daemon), vec![Command::Off]);
}

#[test]
fn how_it_fills_where_it_plays_and_the_battery_are_each_one_command() {
    let status = wallpaper_status(true, None, "");
    let playing = Status {
        wallpaper: Some(one_mk_wallpaper::Status {
            plan: Some(plan(&status, false, false)),
            available: true,
            ..Default::default()
        }),
        ..status
    };
    let (mut win, daemon) = window(playing);
    win.click("Fit inside the screens");
    win.click("Stretch across the screens");
    win.click("The whole picture on each screen");
    win.click("Pause on battery");
    win.click("Office PC");
    win.wait("the commands", |_| daemon.seen().len() >= 5);
    assert_eq!(
        commands(&daemon),
        vec![
            Command::Fit { fit: Fit::Fit },
            Command::Fit { fit: Fit::Stretch },
            Command::Spread { spread: Spread::Each },
            Command::Battery { enabled: false },
            Command::Computer { id: "p1".into(), enabled: false },
        ]
    );
}

#[test]
fn a_computer_that_cannot_play_says_so_with_its_name_and_the_whole_reason() {
    let status = wallpaper_status(true, None, "");
    let mut wallpaper = one_mk_wallpaper::Status {
        plan: Some(plan(&status, false, false)),
        available: true,
        message: "Playing on 1 monitor".into(),
        ..Default::default()
    };
    wallpaper.computers.insert("p1".into(), Playback::Error("No video decoder is installed".into()));
    let (mut win, _daemon) = window(Status { wallpaper: Some(wallpaper), ..status });
    assert!(win.has_text("Office PC: No video decoder is installed"), "{}", win.texts_joined());
    assert!(win.has_text("can't play it here"), "{}", win.texts_joined());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    win.settle();
}

#[test]
fn the_gnome_extension_step_is_one_button_that_sends_one_command() {
    let mut wallpaper = one_mk_wallpaper::Status { available: true, ..Default::default() };
    wallpaper.gnome = Some(one_mk_wallpaper::GnomeSetup {
        stage: one_mk_wallpaper::GnomeStage::NotInstalled,
        message: one_mk_wallpaper::GNOME_PLAYBACK_UNAVAILABLE.into(),
        can_install: true,
        can_enable: false,
    });
    let (mut win, daemon) = window(Status { wallpaper: Some(wallpaper), ..two() });
    assert!(win.has_text("GNOME video setup"), "{}", win.texts_joined());
    win.click("Install GNOME extension");
    win.wait("the command", |_| !daemon.seen().is_empty());
    assert_eq!(
        commands(&daemon),
        vec![Command::GnomeExtension { operation: one_mk_wallpaper::ExtensionAction::Install }]
    );
}
