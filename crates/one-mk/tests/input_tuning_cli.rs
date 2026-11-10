//! The real binary, a throwaway home: what the input tuning commands refuse,
//! what they say, and that one bad line in `settings.toml` costs only itself.

use std::path::Path;
use std::process::{Command, Output};

fn command(home: &Path, args: &[&str]) -> Output {
    // Nothing here may reach the real desktop, settings or daemon.
    Command::new(env!("CARGO_BIN_EXE_one-mk"))
        .args(args)
        .arg("--home")
        .arg(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("c"))
        .env("XDG_DATA_HOME", home.join("d"))
        .env("XDG_CACHE_HOME", home.join("k"))
        .env("XDG_RUNTIME_DIR", home.join("r"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ok(home: &Path, args: &[&str]) -> String {
    let out = command(home, args);
    assert!(out.status.success(), "{args:?}: {}{}", text(&out.stdout), text(&out.stderr));
    text(&out.stdout)
}

fn refused(home: &Path, args: &[&str]) -> String {
    let out = command(home, args);
    assert!(!out.status.success(), "{args:?} should have been refused: {}", text(&out.stdout));
    text(&out.stderr)
}

fn settings(home: &Path) -> String {
    std::fs::read_to_string(home.join("settings.toml")).unwrap_or_default()
}

fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["c", "d", "k", "r"] {
        std::fs::create_dir(dir.path().join(sub)).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(dir.path().join("r"), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

#[test]
fn the_left_and_right_buttons_cannot_be_made_dead() {
    let dir = home();
    let home = dir.path();
    for (button, to) in [("1", "none"), ("2", "none"), ("button1", "none"), ("1", "layer:nav"), ("button2", "layer:nav")] {
        let error = refused(home, &["buttons", "map", button, to]);
        assert!(error.contains("must keep clicking"), "{button} {to}: {error}");
    }
    assert!(settings(home).is_empty(), "a refused rule is not saved: {}", settings(home));
    // Other buttons, and a button with a modifier held, may do nothing.
    ok(home, &["buttons", "map", "4", "none"]);
    ok(home, &["buttons", "map", "ctrl+button1", "none"]);
    // A hand-written rule is skipped, and `keymap check` says so.
    std::fs::write(home.join("settings.toml"), "[[input.button]]\nfrom = 2\nto = \"none\"\n").unwrap();
    let check = command(home, &["keymap", "check"]);
    assert!(!check.status.success());
    assert!(text(&check.stdout).contains("must keep clicking"), "{}", text(&check.stdout));
}

#[test]
fn the_emergency_chord_cannot_be_remapped() {
    let dir = home();
    let error = refused(dir.path(), &["keymap", "add", "ctrl+alt+shift+esc", "a"]);
    assert!(error.contains("emergency chord"), "{error}");
    ok(dir.path(), &["keymap", "add", "ctrl+shift+esc", "a"]);
}

#[test]
fn a_gesture_on_the_autoscroll_button_is_refused_in_either_order() {
    let dir = home();
    let home = dir.path();
    ok(home, &["autoscroll", "--on", "--button", "button4"]);
    let error = refused(home, &["gestures", "add", "button4", "up", "back"]);
    assert!(error.contains("reserved by autoscroll") && error.contains("not saved"), "{error}");
    assert!(!settings(home).contains("gesture"), "{}", settings(home));
    // A different button is fine; turning autoscroll onto its button is not.
    ok(home, &["gestures", "add", "button5", "up", "back"]);
    let error = refused(home, &["autoscroll", "--button", "button5"]);
    assert!(error.contains("reserved by autoscroll"), "{error}");
    assert!(settings(home).contains("button4"), "the refused change left the old button: {}", settings(home));
    // The default (middle) is the toggle too.
    let dir = home_again();
    ok(dir.path(), &["autoscroll", "--on"]);
    let error = refused(dir.path(), &["gestures", "add", "middle", "left", "back"]);
    assert!(error.contains("reserved by autoscroll"), "{error}");
}

fn home_again() -> tempfile::TempDir {
    home()
}

#[test]
fn autoscroll_overrides_can_be_reset() {
    let dir = home();
    let home = dir.path();
    ok(home, &["autoscroll", "--on"]);
    ok(home, &["autoscroll", "--off", "--app", "code*"]);
    ok(home, &["autoscroll", "--speed", "2", "--os", "windows"]);
    assert!(text(&command(home, &["autoscroll"]).stdout).lines().count() >= 3);
    assert!(ok(home, &["autoscroll", "--reset", "--app", "code*"]).contains("forgotten"));
    assert!(ok(home, &["autoscroll", "--reset", "--os", "windows"]).contains("forgotten"));
    let left = settings(home);
    assert!(!left.contains("code") && !left.contains("windows"), "{left}");
    assert!(left.contains("enabled = true"), "the base setting stays: {left}");
    assert!(ok(home, &["autoscroll", "--reset", "--app", "nothing"]).contains("no autoscroll settings"));
    ok(home, &["autoscroll", "--reset"]);
    let all: toml::Value = toml::from_str(&settings(home)).unwrap();
    assert!(all.get("input").is_none(), "{}", settings(home));
    // It is a command of its own.
    assert!(!refused(home, &["autoscroll", "--reset", "--on"]).is_empty());
}

#[test]
fn autoscroll_messages_say_what_happened() {
    let dir = home();
    let home = dir.path();
    assert!(ok(home, &["autoscroll", "--on"]).contains("Autoscroll is on"));
    let saved = ok(home, &["autoscroll", "--speed", "2"]);
    assert!(saved.contains("settings saved") && !saved.contains("is on"), "{saved}");
    let error = refused(home, &["autoscroll", "--speed", "0"]);
    assert!(error.contains("between 0.1 and 10") && !error.contains("choose middle"), "{error}");
    let error = refused(home, &["autoscroll", "--button", "left"]);
    assert_eq!(error.matches("choose").count(), 1, "{error}");
    // Negative numbers reach the range check.
    let error = refused(home, &["autoscroll", "--dead-zone", "-5"]);
    assert!(error.contains("dead zone must be between"), "{error}");
}

#[test]
fn pointer_messages_and_the_hidden_acceleration_flag() {
    let dir = home();
    let home = dir.path();
    let error = refused(home, &["pointer", "--speed", "-1"]);
    assert!(error.contains("say a number from 0.1 to 10"), "{error}");
    let error = refused(home, &["pointer", "--speed", "1e308"]);
    assert!(error.len() < 200 && error.contains("say a number"), "{error}");
    let error = refused(home, &["pointer", "--accel", "curve"]);
    assert!(error.contains("not available on this system yet"), "{error}");
    let help = text(&command(home, &["pointer", "--help"]).stdout);
    assert!(!help.contains("--accel"), "{help}");
    ok(home, &["pointer", "--accel", "system"]);
}

#[test]
fn scroll_output_reads_cleanly() {
    let dir = home();
    let home = dir.path();
    ok(home, &["scroll", "--kind", "wheel", "--speed", "2", "--os", "windows"]);
    ok(home, &["scroll", "--kind", "touchpad", "--natural", "on"]);
    let all = ok(home, &["scroll"]);
    assert!(all.contains("Scrolling, wheels only (for windows computers)"), "{all}");
    assert!(!all.contains(") ("), "{all}");
    let only = ok(home, &["scroll", "--kind", "touchpad"]);
    assert!(only.contains("touchpads only") && !only.contains("wheels"), "{only}");
}

#[test]
fn a_layer_nobody_switches_on_is_pointed_out_when_the_rule_is_added() {
    let dir = home();
    let out = ok(dir.path(), &["keymap", "add", "h", "left", "--layer", "nav"]);
    assert!(out.contains("no key switches the layer"), "{out}");
    ok(dir.path(), &["keymap", "add", "capslock", "layer:nav"]);
    let out = ok(dir.path(), &["keymap", "add", "j", "down", "--layer", "nav"]);
    assert!(!out.contains("no key switches"), "{out}");
}

#[test]
fn local_key_rules_are_only_offered_where_they_can_work() {
    let dir = home();
    let home = dir.path();
    let help = text(&command(home, &["keymap", "--help"]).stdout);
    if cfg!(any(windows, target_os = "macos")) {
        assert!(help.contains("Experimental"), "{help}");
        return;
    }
    assert!(!help.contains("local"), "{help}");
    let error = refused(home, &["keymap", "local", "on"]);
    assert!(error.contains("Windows and macOS only"), "{error}");
    let error = refused(home, &["keymap", "add", "a", "b", "--local"]);
    assert!(error.contains("--local") && error.contains("Windows and macOS only"), "{error}");
    assert!(settings(home).is_empty());
    // Turning it off is always possible.
    ok(home, &["keymap", "local", "off"]);
}

#[test]
fn typing_says_what_each_receiver_will_do() {
    let dir = home();
    let home = dir.path();
    let help = text(&command(home, &["typing", "--help"]).stdout);
    assert!(help.contains("xperimental") && !help.contains("not implemented"), "{help}");
    let out = ok(home, &["typing", "--keys", "characters"]);
    assert!(out.contains("Typing preference: characters") && out.contains("experimental"), "{out}");
    assert!(out.contains("one-mk is not running"), "{out}");
    assert!(!out.contains("Keys stays physical"), "no window names in the CLI: {out}");
}

#[test]
fn one_bad_value_does_not_break_the_rest_of_settings_toml() {
    let dir = home();
    let home = dir.path();
    let file = "[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"esc\"\n\n[[input.scroll]]\nspeed = 2.0\n\n[[input.scroll]]\nnatural = \"maybe\"\n\n[[input.pointer]]\nspeed = 1.5\n\n[clipboard]\ntext = false\n";
    std::fs::write(home.join("settings.toml"), file).unwrap();
    // Listings keep working, and say what is left out and where.
    for (args, shows) in [(&["keymap"][..], "capslock"), (&["pointer"][..], "speed 1.5"), (&["scroll"][..], "speed 2")] {
        let out = command(home, args);
        assert!(out.status.success(), "{args:?}: {}", text(&out.stderr));
        assert!(text(&out.stdout).contains(shows), "{args:?}: {}", text(&out.stdout));
        let err = text(&out.stderr);
        assert!(err.contains("line 8") && err.contains("[[input.scroll]]") && err.contains("maybe"), "{args:?}: {err}");
    }
    // `keymap check` reports it (the file needs fixing) but checks the rest.
    let check = command(home, &["keymap", "check"]);
    assert!(!check.status.success());
    assert!(text(&check.stdout).contains("line 8"), "{}", text(&check.stdout));
    // Changing settings refuses, naming the line, and never saves a copy with
    // the broken part missing.
    let error = refused(home, &["pointer", "--speed", "2"]);
    assert!(error.contains("line 8") || error.contains("maybe"), "{error}");
    assert_eq!(settings(home), file);
}

#[cfg(unix)]
#[test]
fn run_rules_blocked_by_a_writable_folder_are_visible_in_check() {
    use std::os::unix::fs::PermissionsExt;
    let dir = home();
    let home = dir.path();
    ok(home, &["keymap", "add", "f13", "run: echo hi"]);
    assert!(text(&command(home, &["keymap", "check"]).stdout).contains("The input tuning is fine"));
    std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o770)).unwrap();
    let check = command(home, &["keymap", "check"]);
    assert!(!check.status.success(), "{}", text(&check.stdout));
    let out = text(&check.stdout);
    assert!(out.contains("`run: ...`") && out.contains("refused"), "{out}");
    // And adding one says so at once.
    let added = ok(home, &["keymap", "add", "f14", "run: echo hi"]);
    assert!(added.contains("refuses to run commands"), "{added}");
}

#[cfg(unix)]
#[test]
fn a_relative_home_works_like_an_absolute_one() {
    let dir = home();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_one-mk"))
            .args(args)
            .current_dir(&work)
            .env("HOME", dir.path())
            .env("XDG_RUNTIME_DIR", dir.path().join("r"))
            .output()
            .unwrap()
    };
    // Made private first: a folder the umask leaves group-writable would
    // (rightly) refuse `run:` rules.
    std::fs::create_dir(work.join("rel")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(work.join("rel"), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let out = run(&["keymap", "add", "f13", "run: echo hi", "--home", "rel"]);
    assert!(out.status.success(), "{}{}", text(&out.stdout), text(&out.stderr));
    let out = run(&["keymap", "check", "--home", "rel"]);
    assert!(out.status.success(), "a private relative folder is fine: {}{}", text(&out.stdout), text(&out.stderr));
    assert!(work.join("rel/settings.toml").is_file());
}
