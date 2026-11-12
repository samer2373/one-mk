//! QA click-through flows: Settings changes reach `settings.toml`.
//!
//! The appearance is written by the window itself. Everything else is a
//! `one-mk` command, so these flows run the real `one-mk` (built beside the
//! window's test binary; `cargo test --workspace` makes it) against the
//! window's private settings folder: the control is pressed, the file is read
//! back the way the window reads it, and the window is told. Without a built
//! `one-mk` they say so and stop (the same commands are proven by
//! `qa_cli_surface` in the `one-mk` crate).

use super::qa::Fake;
use super::*;
use crate::view::Prefs;

/// Makes this window's `one-mk` the real one, if it is built.
fn use_real_onemk(win: &mut Win) -> bool {
    let me = std::env::current_exe().unwrap();
    let real = me
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join(if cfg!(windows) { "one-mk.exe" } else { "one-mk" }));
    match real.filter(|path| path.is_file()) {
        Some(path) => {
            win.app.link.daemon = path;
            true
        }
        None => {
            eprintln!("SKIPPED: no built one-mk beside the test binary (cargo build -p one-mk)");
            false
        }
    }
}

fn settings_text(win: &Win) -> String {
    std::fs::read_to_string(win.dir.join("settings.toml")).unwrap_or_default()
}

#[test]
fn the_appearance_is_saved_in_the_file_and_the_file_wins_over_the_window() {
    let mut win = Win::light(1200.0, 900.0);
    win.status(Some(two()));
    win.go(Tab::Settings);
    win.click("Appearance: Dark");
    win.quiet();
    let text = settings_text(&win);
    assert!(text.contains("[window]") && text.contains("appearance = \"dark\""), "{text}");
    assert_eq!(win.app.link.prefs().appearance, crate::style::Appearance::Dark);
    win.click("Appearance: Light");
    win.quiet();
    assert!(settings_text(&win).contains("appearance = \"light\""), "{}", settings_text(&win));
    win.click("Appearance: System");
    win.quiet();
    assert!(!settings_text(&win).contains("appearance = \"light\""), "{}", settings_text(&win));
    assert_eq!(win.app.link.prefs().appearance, crate::style::Appearance::System);
    // Somebody changes the file by hand: the window follows when it is told.
    win.settings("[window]\nappearance = \"dark\"\n");
    // The look cross-fades for 200 ms (on a desktop whose own look is dark there is nothing to fade
    // from, which is why this once passed only there), so give it its frames.
    win.wait("the dark look", |_| pal().theme == crate::style::Theme::Dark);
    assert_eq!(pal().theme, crate::style::Theme::Dark);
    win.frames(60);
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn an_appearance_that_cannot_be_saved_still_shows_and_says_so() {
    let mut win = Win::light(1200.0, 900.0);
    win.status(Some(two()));
    win.go(Tab::Settings);
    // The settings folder is a file: nothing can be written there.
    let _ = std::fs::remove_dir_all(&win.dir);
    std::fs::write(&win.dir, b"in the way").unwrap();
    win.click("Appearance: Dark");
    win.wait("the refusal", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't save the appearance. This window keeps it until it closes.");
    assert_eq!(pal().theme, crate::style::Theme::Dark, "shown all the same");
    let _ = std::fs::remove_file(&win.dir);
    std::fs::create_dir_all(&win.dir).unwrap();
}

/// A control to press, the page it is on, and what it changes.
struct Case {
    cat: Option<settings_page::Cat>,
    tab: Tab,
    control: &'static str,
    /// What the setting reads as: before and after must differ.
    read: fn(&Prefs) -> String,
}

fn cases() -> Vec<Case> {
    use settings_page::Cat::*;
    let case = |tab, cat, control, read| Case { tab, cat, control, read };
    vec![
        case(Tab::Files, None, "Text", |p| p.clip_text.to_string()),
        case(Tab::Files, None, "Rich text (HTML)", |p| p.clip_html.to_string()),
        case(Tab::Files, None, "Images", |p| p.clip_image.to_string()),
        case(Tab::Files, None, "Others can get this clipboard", |p| p.clip_allow_fetch.to_string()),
        case(Tab::Files, None, "Skip sensitive copies", |p| p.clip_skip_sensitive.to_string()),
        case(Tab::Files, None, "Notify when files arrive", |p| p.notify_files.to_string()),
        case(Tab::Files, None, "Keep clipboard history", |p| p.history_enabled.to_string()),
        case(Tab::Settings, Some(Edges), "Stay in fullscreen windows", |p| p.edge_fullscreen.to_string()),
        case(Tab::Settings, Some(Edges), "Hold a key to cross: Shift", |p| p.edge_modifier.clone()),
        case(Tab::Settings, Some(Edges), "Learn computer positions: Off", |p| p.edge_learn.clone()),
        case(Tab::Settings, Some(Power), "Keep computers I control awake", |p| p.block_screensaver.to_string()),
        case(Tab::Settings, Some(Power), "Sleep displays together", |p| p.sleep_sync.to_string()),
        case(Tab::Settings, Some(Power), "Lock every screen with this one", |p| p.lock_sync.to_string()),
        case(Tab::Settings, Some(Games), "Gaming mode: On", |p| p.gaming.clone()),
        case(Tab::Settings, Some(Shortcuts), "Use keyboard shortcuts", |p| p.hotkeys_on.to_string()),
        case(Tab::Settings, Some(Typing), "Typing preference: Letters (experimental)", |p| p.typing.to_owned()),
        case(Tab::Settings, Some(Pointer), "Pointer wrap: Left / right", |p| p.edge_wrap.clone()),
        case(Tab::Settings, Some(Pointer), "Draw a pointer on computers with no mouse: Never", |p| {
            p.pointer_draw.clone()
        }),
    ]
}

#[test]
fn every_switch_and_choice_in_settings_reaches_the_file_and_comes_back() {
    let mut win = Win::light(1200.0, 900.0);
    if !use_real_onemk(&mut win) {
        return;
    }
    win.status(Some(two()));
    let mut bad = vec![];
    for case in cases() {
        win.go(case.tab);
        if let Some(cat) = case.cat {
            win.app.settings_cat = cat;
            win.settle();
        }
        let before = (case.read)(&win.app.link.prefs());
        win.click(case.control);
        win.quiet();
        let after = (case.read)(&win.app.link.prefs());
        if before == after {
            bad.push(format!(
                "{}: pressing it left the file as it was ({before}); toast: {}",
                case.control,
                win.toast_text()
            ));
            continue;
        }
        // The window shows what the file says once it is told.
        win.prefs_changed();
        if (case.read)(&win.app.prefs) != after {
            bad.push(format!(
                "{}: the window shows {} but the file says {after}",
                case.control,
                (case.read)(&win.app.prefs)
            ));
        }
        if win.app.toast.as_ref().is_some_and(|t| t.error) {
            bad.push(format!("{}: {}", case.control, win.toast_text()));
        }
    }
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
    // Switches go back when pressed again.
    win.go(Tab::Settings);
    win.app.settings_cat = settings_page::Cat::Power;
    win.settle();
    let first = win.app.link.prefs().sleep_sync;
    win.click("Sleep displays together");
    win.quiet();
    win.click("Sleep displays together");
    win.quiet();
    assert_eq!(win.app.link.prefs().sleep_sync, first);
}

#[test]
fn numbers_typed_into_settings_are_saved_and_a_bad_one_is_refused_and_put_back() {
    let mut win = Win::light(1200.0, 900.0);
    if !use_real_onemk(&mut win) {
        return;
    }
    win.status(Some(two()));
    win.go(Tab::Settings);
    win.app.settings_cat = settings_page::Cat::Edges;
    win.settle();
    let field = win.find("Don't cross near corners, in pixels (0: off)").unwrap();
    win.click_at(field.rect.center());
    win.shortcut(Key::A);
    win.type_text("30");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(win.app.link.prefs().edge_corner, 30, "{}", settings_text(&win));
    win.prefs_changed();
    let field = win.find("Wait at the edge, in milliseconds (0: off)").unwrap();
    win.click_at(field.rect.center());
    win.shortcut(Key::A);
    win.type_text("250");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(win.app.link.prefs().edge_dwell_ms, 250, "{}", settings_text(&win));
    // Typing mode's broadcast timeout, on its own page.
    win.app.settings_cat = settings_page::Cat::Typing;
    win.settle();
    let field = win.find("Broadcast idle timeout in minutes; zero means never").unwrap();
    win.click_at(field.rect.center());
    win.shortcut(Key::A);
    win.type_text("5");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(win.app.link.prefs().broadcast_idle, 5, "{}", settings_text(&win));
    // A number that makes no sense is refused, said, and the field shows what is saved.
    let field = win.find("Broadcast idle timeout in minutes; zero means never").unwrap();
    win.click_at(field.rect.center());
    win.shortcut(Key::A);
    win.type_text("many");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(win.app.link.prefs().broadcast_idle, 5, "{}", settings_text(&win));
    win.prefs_changed();
    assert_eq!(win.app.prefs.broadcast_idle, 5);
}

#[test]
fn a_command_that_fails_puts_the_control_back_to_what_is_saved_and_says_so() {
    let mut win = Win::light(1200.0, 900.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.go(Tab::Settings);
    win.app.settings_cat = settings_page::Cat::Power;
    win.settle();
    let saved = win.app.prefs.sleep_sync;
    fake.fail("power", "one-mk: settings.toml is read-only");
    win.click("Sleep displays together");
    win.wait("the refusal", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't change sleep sync.");
    assert_eq!(win.app.prefs.sleep_sync, saved, "back to what is saved");
    assert_eq!(fake.calls_with(&["power"]).len(), 1);
}

#[test]
fn a_settings_file_the_window_cannot_read_is_said_and_changes_wait() {
    let mut win = Win::light(1200.0, 900.0);
    win.status(Some(two()));
    win.go(Tab::Keys);
    // A file over the size limit is refused as the daemon refuses it.
    let big = vec![b'#'; (one_mk_ctl::MAX_SETTINGS_BYTES + 10) as usize];
    std::fs::write(win.dir.join("settings.toml"), big).unwrap();
    win.prefs_changed();
    assert!(win.app.prefs.unreadable);
    assert!(win.has_text("Preferences couldn't be read. Changes are paused"), "{}", win.texts_joined());
}
