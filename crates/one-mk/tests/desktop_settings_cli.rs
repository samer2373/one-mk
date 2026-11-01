use std::path::Path;
use std::process::Command;

fn command(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_one-mk")).args(args).arg("--home").arg(home).output().unwrap()
}

fn change(home: &Path, args: &[&str]) -> toml::Value {
    let output = command(home, args);
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    toml::from_str(&std::fs::read_to_string(home.join("settings.toml")).unwrap()).unwrap()
}

#[test]
fn desktop_controls_round_trip_through_real_settings_commands() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    for (word, expected) in [("on", true), ("off", false)] {
        assert_eq!(change(home, &["clipboard", "history", "persist", word])["clipboard"]["history_persist"].as_bool().unwrap_or(false), expected);
        assert_eq!(change(home, &["clipboard", "--skip-sensitive", word])["clipboard"]["skip_sensitive"].as_bool(), Some(expected));
    }
    assert_eq!(change(home, &["broadcast", "--idle-off", "7"])["broadcast"]["idle_off_minutes"].as_integer(), Some(7));
    let edge = change(home, &["edge", "--side", "left", "--corner", "12", "--dwell", "250", "--modifier", "ctrl", "--fullscreen-guard", "on"]);
    assert_eq!(edge["edge"]["left"]["corner"].as_integer(), Some(12));
    assert_eq!(edge["edge"]["left"]["dwell_ms"].as_integer(), Some(250));
    let edge = change(home, &["edge", "--wrap", "all", "--learn", "auto"]);
    assert_eq!(edge["edge"]["wrap"].as_str(), Some("all"));
    assert_eq!(edge["edge"]["learn"].as_str(), Some("auto"));
    assert_eq!(change(home, &["pointer-overlay", "always"])["pointer"]["draw_when_no_mouse"].as_str(), Some("always"));
    assert_eq!(change(home, &["drag", "--on"])["drag"]["enabled"].as_bool(), Some(true));
    assert_eq!(change(home, &["hotkeys", "--fetch-clipboard", "ctrl+alt+k"])["hotkeys"]["fetch_clipboard"].as_str(), Some("ctrl+alt+k"));
    let before = std::fs::read_to_string(home.join("settings.toml")).unwrap();
    let conflict = command(home, &["hotkeys", "--home-key", "ctrl+alt+k"]);
    assert!(!conflict.status.success());
    assert_eq!(std::fs::read_to_string(home.join("settings.toml")).unwrap(), before);
    let hooks = change(home, &["hooks", "--on-enter", "echo 'hello world'", "--computer", "", "--event-timeout", "17", "--event-shell", "off"]);
    assert_eq!(hooks["hooks"]["on_enter"].as_str(), Some("echo 'hello world'"));
    assert_eq!(hooks["hooks"]["options"]["enter"]["timeout_secs"].as_integer(), Some(17));
    assert_eq!(hooks["hooks"]["options"]["enter"]["shell"].as_bool(), Some(false));
    let cleared = change(home, &["hooks", "--on-enter", ""]);
    assert!(cleared["hooks"].get("on_enter").is_none());
    assert!(!command(home, &["hooks", "--on-enter", "echo ok", "--event-timeout", "0"]).status.success());
    change(home, &["gestures", "add", "button4", "left", "ctrl+alt+k"]);
    change(home, &["gestures", "remove", "button4", "left"]);
}
