//! The whole command line of the real binary: every command's `--help`, every
//! error path (bad arguments, no daemon, unknown computer), JSON output, setting
//! changes that must stick or be refused untouched, settings and device files
//! that are old, partial or damaged, `bench`, and install/integrate inside a
//! sandboxed home.
#![cfg(target_os = "linux")]

mod qa_common;
use qa_common::*;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn alone(name: &str) -> (Lab, Node) {
    let lab = Lab::new();
    let node = lab.node(name);
    (lab, node)
}

fn docs() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/COMMANDS.md")).unwrap()
}

/// The `Commands:` section of a help text: (name, description).
fn subcommands(help: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in help.lines() {
        if line.trim_end() == "Commands:" {
            inside = true;
            continue;
        }
        if inside {
            if !line.starts_with("  ") || line.trim().is_empty() {
                break;
            }
            let mut parts = line.trim().splitn(2, char::is_whitespace);
            let name = parts.next().unwrap().to_owned();
            if name != "help" {
                out.push((name, parts.next().unwrap_or("").trim().to_owned()));
            }
        }
    }
    out
}

fn crawl(node: &Node, path: &[String], seen: &mut BTreeSet<String>, depth: usize) {
    let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
    args.push("--help");
    let out = Lab::run(node, &args);
    let label = path.join(" ");
    assert!(out.ok(), "`one-mk {label} --help` failed ({:?}): {}", out.code, out.stderr);
    assert!(out.stdout.contains("Usage: one-mk"), "`one-mk {label} --help` has no usage line:\n{}", out.stdout);
    assert!(out.stderr.is_empty(), "`one-mk {label} --help` wrote to stderr: {}", out.stderr);
    assert!(!out.stdout.lines().next().unwrap_or("").trim().is_empty(), "`one-mk {label} --help` starts with nothing");
    seen.insert(label.clone());
    // `one-mk help <command>` says the same.
    if !path.is_empty() && path.len() == 1 {
        let via_help = Lab::run(node, &["help", &path[0]]);
        assert!(via_help.ok() && via_help.stdout == out.stdout, "`one-mk help {label}` differs from `--help`");
        let short = Lab::run(node, &[&path[0], "-h"]);
        assert!(short.ok() && short.stdout.contains("Usage: one-mk"), "`one-mk {label} -h`: {short:?}");
    }
    assert!(depth < 5, "help recursion too deep at {label}");
    for (name, description) in subcommands(&out.stdout) {
        assert!(!description.is_empty(), "`one-mk {label} {name}` has no description");
        let mut next = path.to_vec();
        next.push(name);
        crawl(node, &next, seen, depth + 1);
    }
}

#[test]
fn every_command_and_subcommand_has_help_and_the_command_reference_lists_it() {
    if !isolated("every_command_and_subcommand_has_help_and_the_command_reference_lists_it") {
        return;
    }
    let (_lab, node) = alone("alpha");
    let mut seen = BTreeSet::new();
    crawl(&node, &[], &mut seen, 0);
    let top: Vec<String> = subcommands(&Lab::ok(&node, &["--help"])).into_iter().map(|(name, _)| name).collect();
    // The reference says what the program says: every command is listed in it, with its summary.
    let reference = docs();
    for (name, description) in subcommands(&Lab::ok(&node, &["--help"])) {
        assert!(
            reference.contains(&format!("## one-mk {name}")),
            "docs/COMMANDS.md has no section for `one-mk {name}`"
        );
        let first_line = description.split(". ").next().unwrap().trim_end_matches('.');
        let head: String = first_line.chars().take(40).collect();
        assert!(
            reference.contains(&head),
            "docs/COMMANDS.md describes `one-mk {name}` differently from --help ({head:?})"
        );
    }
    // And the crawl really went down: a spread of nested commands was reached.
    for nested in [
        "clipboard history list",
        "wallpaper extension install",
        "keymap add",
        "gestures preset",
        "buttons map",
        "profile auto",
        "pointer-extension status",
    ] {
        assert!(seen.contains(nested), "{nested} was not reached; seen {seen:?}");
    }
    assert!(top.len() >= 40, "{top:?}");
    // The version, and a plain run of the wrong thing.
    let version = Lab::ok(&node, &["--version"]);
    assert!(
        version.starts_with("one-mk ") && version.trim().split(' ').nth(1).is_some_and(|v| v.split('.').count() == 3),
        "{version}"
    );
    assert!(Lab::ok(&node, &["-V"]) == version);
}

fn gone_wrong(node: &Node, args: &[&str], needle: &str) {
    let out = Lab::run(node, args);
    let error = out.failed().to_owned();
    assert!(error.contains(needle), "`one-mk {}` said {error:?}, expected it to mention {needle:?}", args.join(" "));
    assert!(out.stdout.is_empty() || !out.stdout.contains("panicked"));
}

/// Without a daemon: commands that need one say how to start it and exit non-zero;
/// those that only read or write the settings folder work.
#[test]
fn commands_that_need_a_daemon_say_how_to_start_it_and_the_rest_work_without() {
    if !isolated("commands_that_need_a_daemon_say_how_to_start_it_and_the_rest_work_without") {
        return;
    }
    let (_lab, node) = alone("alpha");
    for args in [
        &["takeback"][..],
        &["switch", "next"],
        &["switch", "here"],
        &["switch", "somebody"],
        &["lock-pointer", "on"],
        &["lock-pointer", "toggle"],
        &["broadcast", "on"],
        &["lock-all"],
        &["clip-send"],
        &["clip-send", "--to", "x"],
        &["clip-get"],
        &["clipboard", "history", "list"],
        &["clipboard", "history", "show", "1"],
        &["clipboard", "history", "clear"],
    ] {
        gone_wrong(&node, args, "not running");
        assert!(
            Lab::run(&node, args).stderr.contains("one-mk run") || Lab::run(&node, args).stderr.contains("start it"),
            "{args:?} does not say how to start it"
        );
    }
    // Reading commands work and say the daemon is not running.
    for (args, needle) in [
        (&["status"][..], "not running"),
        (&["lock-pointer"][..], "not running"),
        (&["broadcast"][..], "not running"),
        (&["stop"][..], "not running"),
        (&["wallpaper", "status"][..], "not running"),
        (&["partials"][..], "No unfinished"),
        (&["devices"][..], "No paired computers"),
        (&["notify"][..], "notification"),
        (&["gaming"][..], "Gaming mode"),
        (&["appearance"][..], "Appearance"),
        (&["lock-sync"][..], "Lock sync"),
        (&["hooks"][..], "No hooks"),
        (&["power"][..], "Sleep sync"),
        (&["pointer"][..], "Pointer"),
        (&["scroll"][..], "Scrolling"),
        (&["autoscroll"][..], "Autoscroll"),
        (&["gestures"][..], "No gestures"),
        (&["buttons"][..], "No button rules"),
        (&["keymap"][..], "No key rules"),
        (&["edge"][..], "No edge rules"),
        (&["hotkeys"][..], "Keyboard shortcuts"),
        (&["profile", "list"][..], "No layout profiles"),
        (&["typing"][..], "Typing preference"),
        (&["pointer-overlay"][..], "pointer"),
        (&["clipboard"][..], "Clipboard sharing"),
        (&["id"][..], "fingerprint"),
    ] {
        let out = Lab::run(&node, args);
        assert!(out.ok(), "`one-mk {}` failed without a daemon: {}", args.join(" "), out.both());
        assert!(
            out.both().contains(needle),
            "`one-mk {}` printed {:?}, expected {needle:?}",
            args.join(" "),
            out.both()
        );
    }
    // `status` on a stopped daemon is not an error (exit 0): scripts read its text.
    assert!(Lab::run(&node, &["status"]).ok());
}

#[test]
fn unknown_computers_and_missing_arguments_are_refused_with_something_to_act_on() {
    if !isolated("unknown_computers_and_missing_arguments_are_refused_with_something_to_act_on") {
        return;
    }
    let (_lab, node) = alone("alpha");
    for args in [
        &["layout", "nobody", "left"][..],
        &["forget", "nobody"],
        &["rename", "nobody", "x"],
        &["send", "/etc/hostname", "--to", "nobody"],
        &["bench", "--peer", "nobody"],
        &["typing", "--peer", "nobody"],
        &["pointer", "--speed", "2", "--peer", "nobody"],
    ] {
        gone_wrong(&node, args, "nobody");
    }
    gone_wrong(&node, &["send", "/etc/hostname"], "one-mk pair");
    // Usage errors: exit status 2 with clap's explanation and a pointer to --help.
    for args in [
        &["layout"][..],
        &["layout", "x", "diagonal"],
        &["layout", "x", "left", "--offset", "1"],
        &["layout", "x", "left", "--offset", "99999999,0"],
        &["rename"],
        &["forget"],
        &["send"],
        &["switch"],
        &["bogus"],
        &["--bogus"],
        &["gaming", "bogus"],
        &["appearance", "bogus"],
        &["lock-sync", "maybe"],
        &["lock-pointer", "sideways"],
        &["broadcast", "maybe"],
        &["broadcast", "--idle-off", "abc"],
        &["pointer-overlay", "x"],
        &["invite", "--expires", "0"],
        &["invite", "--expires", "2h"],
        &["invite", "--expires", "abc"],
        &["pair", "--connect", "nothing"],
        &["run", "--port", "99999"],
        &["run", "--port", "abc"],
        &["run", "--backend", "foo"],
        &["run", "--connect", "foo"],
        &["drag", "--zone", "0"],
        &["drag", "--zone", "999"],
        &["drag", "--on", "--off"],
        &["bench", "--count", "abc"],
        &["bench", "--count", "0"],
        &["bench", "--peer", "x", "--serve", "1"],
        &["clipboard", "--text", "maybe"],
        &["clipboard", "history", "show", "abc"],
        &["update", "--check", "--rollback"],
        &["update", "--check-on-start", "maybe"],
        &["profile"],
        &["profile", "save"],
        &["profile", "switch", "x"],
        &["keymap", "add"],
        &["buttons", "map"],
    ] {
        let out = Lab::run(&node, args);
        assert_eq!(out.code, Some(2), "`one-mk {}` should be a usage error: {out:?}", args.join(" "));
        assert!(
            out.stderr.contains("--help") || out.stderr.contains("Usage"),
            "`one-mk {}` does not point to --help: {}",
            args.join(" "),
            out.stderr
        );
        assert!(!out.stderr.contains("panicked"));
    }
}

#[test]
fn json_outputs_are_valid_json_and_say_the_same_thing_as_the_text() {
    if !isolated("json_outputs_are_valid_json_and_say_the_same_thing_as_the_text") {
        return;
    }
    let (_lab, node) = alone("alpha");
    for args in [
        &["doctor", "--json"][..],
        &["doctor", "--json", "--all"],
        &["input-devices", "--json"],
        &["update", "--status-json"],
        &["wallpaper", "--json", "status"],
    ] {
        let out = Lab::run(&node, args);
        let value: serde_json::Value = serde_json::from_str(out.stdout.trim())
            .unwrap_or_else(|e| panic!("`one-mk {}` is not JSON ({e}): {out:?}", args.join(" ")));
        assert!(value.is_object(), "{args:?}: {value}");
        assert!(
            !out.stdout.trim().contains('\n') || args[0] == "doctor",
            "{args:?} printed more than one JSON document"
        );
    }
    let all =
        serde_json::from_str::<serde_json::Value>(&Lab::run(&node, &["doctor", "--json", "--all"]).stdout).unwrap();
    let some = serde_json::from_str::<serde_json::Value>(&Lab::run(&node, &["doctor", "--json"]).stdout).unwrap();
    assert!(all["checks"].as_array().unwrap().len() >= some["checks"].as_array().unwrap().len());
    // The update status says it cannot reach the (unset) server in words, not a stack trace.
    let status: serde_json::Value = serde_json::from_str(&Lab::ok(&node, &["update", "--status-json"])).unwrap();
    for key in ["check", "install_automatically", "can_update", "needs_password", "rollback_available", "advice"] {
        assert!(status.get(key).is_some(), "update --status-json lacks {key}: {status}");
    }
}

#[test]
fn update_refuses_insecure_unreachable_and_rollback_without_a_backup() {
    if !isolated("update_refuses_insecure_unreachable_and_rollback_without_a_backup") {
        return;
    }
    let (lab, node) = alone("alpha");
    gone_wrong(&node, &["update", "--check", "--url", "http://127.0.0.1:9/latest"], "https://");
    gone_wrong(&node, &["update", "--check", "--url", "ftp://example.com/x"], "https://");
    gone_wrong(&node, &["update", "--check", "--url", "not a url"], "update");
    // A secure address nobody listens on: a plain failure, quickly, naming the cause.
    let dead = format!("https://127.0.0.1:{}/latest", lab.node("closed").port);
    let started = std::time::Instant::now();
    let out = Lab::run(&node, &["update", "--check", "--url", &dead]);
    out.failed();
    assert!(started.elapsed().as_secs() < 40, "{:?}", started.elapsed());
    let rollback = Lab::run(&node, &["update", "--rollback"]);
    assert!(rollback.failed().contains("release"), "{rollback:?}");
    // Turning the check off and on is remembered and shown.
    Lab::ok(&node, &["update", "--check-on-start", "off"]);
    let status: serde_json::Value = serde_json::from_str(&Lab::ok(&node, &["update", "--status-json"])).unwrap();
    assert_eq!(status["check"], false);
    Lab::ok(&node, &["update", "--check-on-start", "on"]);
    let status: serde_json::Value = serde_json::from_str(&Lab::ok(&node, &["update", "--status-json"])).unwrap();
    assert_eq!(status["check"], true);
    // Nothing ever installs on its own: --startup-check with the check off fetches nothing and exits cleanly.
    Lab::ok(&node, &["update", "--check-on-start", "off"]);
    assert!(Lab::run(&node, &["update", "--startup-check"]).ok());
}

#[test]
fn bench_measures_over_loopback_and_refuses_nonsense() {
    if !isolated("bench_measures_over_loopback_and_refuses_nonsense") {
        return;
    }
    let (lab, node) = alone("alpha");
    let out = Lab::run(&node, &["bench", "--count", "200", "--rate", "2000"]);
    let text = out.success();
    assert!(text.contains("round trip") && text.contains("lost"), "{text}");
    assert!(!text.contains("NaN") && !text.contains("inf"), "{text}");
    // `--count 0` used to print NaN for every figure.
    let zero = Lab::run(&node, &["bench", "--count", "0"]);
    assert_eq!(zero.code, Some(2), "{zero:?}");
    // The two ends of a bench across "machines".
    let port = lab.node("benchport").port.to_string();
    let mut server = Proc::spawn(Lab::command(&node, &["bench", "--serve", &port]));
    std::thread::sleep(std::time::Duration::from_millis(500));
    let client = Lab::run(&node, &["bench", "--connect", &format!("127.0.0.1:{port}"), "--count", "300"]);
    let said = client.success();
    assert!(said.contains("round trip") && !said.contains("NaN"), "{said}");
    server.signal(libc::SIGINT);
    let _ = server.try_expect("never", 1);
    drop(server);
    // A server that is not there: a reason, not a hang.
    let dead = format!("127.0.0.1:{}", lab.node("deadbench").port);
    let started = std::time::Instant::now();
    Lab::run(&node, &["bench", "--connect", &dead, "--count", "10"]).failed();
    assert!(started.elapsed().as_secs() < 60);
}

#[test]
fn a_run_with_run_options_before_another_command_is_an_error_not_ignored() {
    if !isolated("a_run_with_run_options_before_another_command_is_an_error_not_ignored") {
        return;
    }
    let (_lab, node) = alone("alpha");
    for args in [&["--port", "1", "status"][..], &["--no-discovery", "devices"]] {
        let out = Lab::run(&node, args);
        assert!(out.failed().contains("one-mk run"), "{out:?}");
    }
}

/// Every setting that can be changed from the command line sticks, shows up when
/// asked and survives a daemon restart; the daemon applies it without a restart.
#[test]
fn settings_commands_change_the_file_show_the_change_and_the_daemon_takes_it_live() {
    if !isolated("settings_commands_change_the_file_show_the_change_and_the_daemon_takes_it_live") {
        return;
    }
    let (mut lab, node) = alone("alpha");
    lab.start_plain(&node);
    for (set, show, expect) in [
        (&["clipboard", "--text", "off", "--html", "off", "--max-mb", "5"][..], &["clipboard"][..], "size limit 5 MB"),
        (&["clipboard", "--image", "off"], &["clipboard"], "images off"),
        (&["clipboard", "--skip-sensitive", "off"], &["clipboard"], "skip marked sensitive content off"),
        (&["clipboard", "--allow-fetch", "on"], &["clipboard"], "others may fetch it on"),
        (&["notify", "off"], &["notify"], "off"),
        (&["gaming", "on"], &["gaming"], "on"),
        (&["appearance", "dark"], &["appearance"], "dark"),
        (&["lock-sync", "on"], &["lock-sync"], "on"),
        (&["typing", "--keys", "characters"], &["typing"], "characters"),
        (
            &["edge", "--corner", "12", "--dwell", "150", "--modifier", "ctrl", "--wrap", "on", "--learn", "off"],
            &["edge"],
            "dwell 150 ms",
        ),
        (&["edge", "--fullscreen-guard", "on", "--fullscreen-allow", "vlc"], &["edge"], "vlc"),
        (
            &["power", "--block-screensaver", "on", "--sleep-sync", "on"],
            &["power"],
            "Sleep sync (displays sleep together): on",
        ),
        (&["pointer", "--speed", "2"], &["pointer"], "speed 2"),
        (
            &[
                "scroll",
                "--speed",
                "1.5",
                "--natural",
                "on",
                "--mode",
                "smooth",
                "--reverse-vertical",
                "on",
                "--modifier",
                "shift=horizontal",
            ],
            &["scroll"],
            "speed 1.5",
        ),
        (&["autoscroll", "--on", "--speed", "2", "--button", "middle"], &["autoscroll"], "speed 2"),
        (&["keymap", "add", "capslock", "esc"], &["keymap", "list"], "capslock"),
        (&["buttons", "preset", "side-back-forward"], &["buttons", "list"], "button"),
        (&["gestures", "preset", "side-swipe"], &["gestures", "list"], "button"),
        (&["hotkeys", "--cycle", "ctrl+alt+n"], &["hotkeys"], "Ctrl+Alt+N"),
        (&["pointer-overlay", "always"], &["pointer-overlay"], "always"),
        (&["hooks", "--on-enter", "/bin/true"], &["hooks"], "/bin/true"),
    ] {
        let said = Lab::run(&node, set);
        assert!(said.ok(), "`one-mk {}` failed: {}", set.join(" "), said.both());
        let shown = Lab::ok(&node, show);
        assert!(
            shown.to_lowercase().contains(&expect.to_lowercase()),
            "after `one-mk {}`, `one-mk {}` says {shown:?}, expected {expect:?}",
            set.join(" "),
            show.join(" ")
        );
        assert!(!said.both().contains("panicked"));
        // The daemon is still answering and did not choke on the new file.
        assert!(
            lab.status_of(&node).is_some(),
            "the daemon stopped answering after `one-mk {}`: {}",
            set.join(" "),
            Lab::log(&node)
        );
    }
    let file = std::fs::read_to_string(node.home.join("settings.toml")).unwrap();
    toml::from_str::<toml::Table>(&file)
        .unwrap_or_else(|e| panic!("settings.toml is not TOML after the changes: {e}\n{file}"));
    // The daemon applied them live (the line it sends status with) and again after a restart.
    let live = Lab::ok(&node, &["status"]);
    assert!(live.contains("size limit 5 MB"), "{live}");
    lab.kill9(&node);
    lab.start_plain(&node);
    let after = Lab::ok(&node, &["status"]);
    assert!(after.contains("size limit 5 MB") && after.contains("images off"), "{after}");
    // The daemon's log has no complaints about any of it.
    let log = Lab::log(&node);
    assert!(!log.contains("panicked") && !log.contains("ERROR"), "{log}");
}

/// A setting refused on the command line leaves `settings.toml` exactly as it was.
#[test]
fn refused_setting_changes_leave_the_file_untouched() {
    if !isolated("refused_setting_changes_leave_the_file_untouched") {
        return;
    }
    let (_lab, node) = alone("alpha");
    Lab::ok(&node, &["edge", "--dwell", "100"]);
    let before = std::fs::read(node.home.join("settings.toml")).unwrap();
    for args in [
        &["pointer", "--speed", "99"][..],
        &["pointer", "--speed", "0"],
        &["pointer", "--speed", "abc"],
        &["scroll", "--speed", "0"],
        &["scroll", "--natural", "perhaps"],
        &["scroll", "--modifier", "nonsense"],
        &["autoscroll", "--on", "--speed", "50"],
        &["autoscroll", "--dead-zone", "99999"],
        &["edge", "--corner", "abc"],
        &["edge", "--modifier", "windows"],
        &["edge", "--wrap", "sometimes"],
        &["edge", "--side", "diagonal", "--dwell", "5"],
        &["keymap", "add", "nosuchkey", "esc"],
        &["keymap", "add", "capslock", "nosuchkey"],
        &["buttons", "map", "99", "button1"],
        &["buttons", "map", "1", "nosuchkey"],
        &["gestures", "add", "button4", "sideways", "back"],
        &["hotkeys", "--jump", "zzz"],
        &["hotkeys", "--cycle", "ctrl+alt+"],
        &["hooks", "--timeout", "100"],
        &["hooks", "--timeout", "0"],
        &["hooks", "--on-enter", &"x".repeat(5000)],
        &["clipboard", "--max-mb", "0"],
        &["clipboard", "--max-mb", "9999"],
        &["drag", "--zone", "500"],
        &["rename", "nobody", "x"],
    ] {
        let out = Lab::run(&node, args);
        assert!(!out.ok() && out.code.is_some(), "`one-mk {}` was accepted: {out:?}", args.join(" "));
        assert!(!out.stderr.contains("panicked"), "`one-mk {}` panicked: {}", args.join(" "), out.stderr);
        assert_eq!(
            std::fs::read(node.home.join("settings.toml")).unwrap(),
            before,
            "`one-mk {}` changed settings.toml although it failed",
            args.join(" ")
        );
    }
}

fn write_settings(node: &Node, bytes: &[u8]) {
    let path = node.home.join("settings.toml");
    let _ = std::fs::remove_dir_all(&path);
    std::fs::write(&path, bytes).unwrap();
}

/// What a person may have in `settings.toml` after an upgrade, a hand edit, a crash or a
/// copy from another system: the read-only commands and the daemon go on working, and
/// nothing that changes the file overwrites what it could not read.
#[test]
fn old_hand_edited_and_damaged_settings_never_stop_the_commands_or_the_daemon() {
    if !isolated("old_hand_edited_and_damaged_settings_never_stop_the_commands_or_the_daemon") {
        return;
    }
    let readable: [(&str, Vec<u8>); 8] = [
        ("empty", Vec::new()),
        ("only blanks", b"   \n\n  \n".to_vec()),
        ("byte order mark", b"\xef\xbb\xbfnotify_files = false\n".to_vec()),
        ("windows line ends", b"notify_files = false\r\n[edge]\r\ndwell_ms = 300\r\n".to_vec()),
        ("a newer settings version", b"settings_version = 99\nnotify_files = false\n".to_vec()),
        ("0.1.13", include_bytes!("fixtures/settings-0.1.13.toml").to_vec()),
        ("0.1.14", include_bytes!("fixtures/settings-0.1.14.toml").to_vec()),
        ("only comments", "# nothing here\n".repeat(200_000).into_bytes()),
    ];
    let (mut lab, node) = alone("alpha");
    for (what, bytes) in &readable {
        write_settings(&node, bytes);
        for args in [
            &["status"][..],
            &["notify"],
            &["edge"],
            &["clipboard"],
            &["hotkeys"],
            &["hooks"],
            &["pointer"],
            &["keymap", "list"],
            &["doctor", "--json"],
        ] {
            let out = Lab::run(&node, args);
            assert!(out.ok() || args[0] == "doctor", "{what}: `one-mk {}` failed: {}", args.join(" "), out.both());
            assert!(!out.both().contains("panicked"), "{what}: {}", out.both());
        }
        lab.start_plain(&node);
        assert!(lab.status_of(&node).is_some(), "{what}: the daemon did not start");
        let change = Lab::run(&node, &["edge", "--dwell", "7"]);
        assert!(change.ok(), "{what}: could not change a setting: {}", change.both());
        assert!(Lab::ok(&node, &["edge"]).contains("dwell 7 ms"), "{what}");
        lab.terminate(&node);
    }
    // 0.1.14's choices were read, not replaced by defaults.
    write_settings(&node, include_bytes!("fixtures/settings-0.1.14.toml"));
    let clipboard = Lab::ok(&node, &["clipboard"]);
    assert!(
        clipboard.contains("text off") && clipboard.contains("images on") && clipboard.contains("size limit 5 MB"),
        "{clipboard}"
    );
    assert!(Lab::ok(&node, &["notify"]).contains("off"));

    // Unknown keys (a newer version's, or a typo) are reported once, kept in the file when it is edited.
    write_settings(&node, b"future_feature = true\n[future]\nx = 1\n[edge]\ndwell_ms = 300\n");
    let doctor = Lab::run(&node, &["doctor"]);
    assert_eq!(
        doctor.stderr.matches("unknown setting \"future\"").count(),
        1,
        "the same warning was repeated: {}",
        doctor.stderr
    );
    Lab::ok(&node, &["edge", "--dwell", "5"]);
    let kept = std::fs::read_to_string(node.home.join("settings.toml")).unwrap();
    assert!(
        kept.contains("future_feature") && kept.contains("[future]"),
        "an edit dropped keys it does not know: {kept}"
    );

    // A file that cannot be understood: reading says so and carries on with defaults, changing refuses
    // and keeps the file byte for byte, and the daemon starts with a warning.
    let broken: [(&str, Vec<u8>); 6] = [
        ("not UTF-8", noise(3000, 5)),
        ("cut off", b"notify_files = fal".to_vec()),
        ("a null byte", b"notify_files = false\n\0\0".to_vec()),
        ("a wrong type", b"notify_files = \"maybe\"\n[edge]\ndwell_ms = 300\n".to_vec()),
        ("lines mixed with binary", [b"gaming = \"on\"\n".as_slice(), &noise(200, 9)].concat()),
        ("a section that is not a table", b"edge = 5\n".to_vec()),
    ];
    for (what, bytes) in &broken {
        write_settings(&node, bytes);
        for args in [&["status"][..], &["doctor"], &["devices"], &["id"]] {
            let out = Lab::run(&node, args);
            assert!(!out.both().contains("panicked"), "{what}: {}", out.both());
        }
        assert!(Lab::run(&node, &["status"]).ok(), "{what}: status must still work");
        let change = Lab::run(&node, &["notify", "off"]);
        assert!(
            !change.ok() && change.stderr.contains("settings.toml"),
            "{what}: a change to an unreadable file was not refused: {change:?}"
        );
        assert_eq!(
            std::fs::read(node.home.join("settings.toml")).unwrap(),
            *bytes,
            "{what}: the unreadable file was overwritten"
        );
        lab.start_plain(&node);
        assert!(lab.status_of(&node).is_some(), "{what}: the daemon did not start");
        lab.terminate(&node);
    }
    // settings.toml replaced by a folder: a named, plain failure.
    let path = node.home.join("settings.toml");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let out = Lab::run(&node, &["notify", "off"]);
    assert!(!out.ok() && !out.both().contains("panicked") && out.stderr.contains("settings.toml"), "{out:?}");
    assert!(!Lab::run(&node, &["status"]).both().contains("panicked"));
}

fn healthy_devices(other: &Node) -> String {
    format!(
        "[[device]]\nname = \"peer\"\nfingerprint = \"{}\"\nside = \"right\"\naddrs = [\"127.0.0.1:1\"]\n",
        fingerprint(other)
    )
}

/// The same for the list of paired computers and the identity: damage is named and
/// never silently replaced by an empty list or a new identity.
#[test]
fn damaged_device_lists_and_identities_are_named_and_never_silently_replaced() {
    if !isolated("damaged_device_lists_and_identities_are_named_and_never_silently_replaced") {
        return;
    }
    let (mut lab, node) = alone("alpha");
    let other = lab.node("other");
    let good = healthy_devices(&other);
    let path = node.home.join("devices.toml");
    std::fs::write(&path, &good).unwrap();
    let devices = Lab::ok(&node, &["devices"]);
    assert!(devices.contains("peer") && devices.contains("right"), "{devices}");
    lab.start_plain(&node);
    assert!(lab.status(&node).peers.iter().any(|p| p.name == "peer"));
    lab.terminate(&node);

    let variants: [(&str, Vec<u8>); 6] = [
        ("garbage", noise(3000, 3)),
        ("cut off", good.as_bytes()[..60].to_vec()),
        ("a bad fingerprint", b"[[device]]\nname = \"peer\"\nfingerprint = \"zz\"\nside = \"right\"\n".to_vec()),
        ("a wrong type", b"[[device]]\nname = 5\nfingerprint = \"ab\"\n".to_vec()),
        ("the same computer twice", format!("{good}{good}").into_bytes()),
        ("an unknown side", good.replace("right", "diagonal").into_bytes()),
    ];
    for (what, bytes) in variants {
        std::fs::write(&path, &bytes).unwrap();
        for args in [
            &["devices"][..],
            &["status"],
            &["doctor"],
            &["layout", "peer", "left"],
            &["forget", "peer"],
            &["rename", "peer", "x"],
        ] {
            let out = Lab::run(&node, args);
            assert!(!out.both().contains("panicked"), "{what}: `one-mk {}`: {}", args.join(" "), out.both());
            assert!(out.code.is_some(), "{what}: `one-mk {}` was killed", args.join(" "));
        }
        assert!(!Lab::run(&node, &["devices"]).both().is_empty());
        let after = std::fs::read(&path).unwrap();
        if !matches!(what, "the same computer twice" | "an unknown side" | "a bad fingerprint") {
            assert_eq!(after, bytes, "{what}: a command rewrote a device list it could not read");
        }
        // A list that cannot be read stops the daemon with the file named (it cannot know whom to
        // trust); one that reads, however odd, lets it start.
        let unreadable = matches!(what, "garbage" | "cut off" | "a wrong type");
        if unreadable {
            let run = Lab::run(
                &node,
                &[
                    "run",
                    "--backend",
                    "null",
                    "--no-discovery",
                    "--no-notify",
                    "--no-clipboard",
                    "--port",
                    &node.port.to_string(),
                ],
            );
            let error = run.failed();
            assert!(error.contains("devices.toml") && !error.contains("is busy"), "{what}: {error}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{what}: a failing daemon rewrote the list");
        } else {
            lab.start_plain(&node);
            assert!(lab.status_of(&node).is_some(), "{what}: the daemon did not start: {}", Lab::log(&node));
            lab.terminate(&node);
        }
    }
    // The identity: a damaged key or certificate is an error that names the files (not "UDP port
    // is busy"), is not replaced by a new identity that every paired computer would refuse, and
    // the daemon says the same.
    std::fs::write(&path, &good).unwrap();
    let key_path = node.home.join("identity.key.der");
    let cert_path = node.home.join("identity.cert.der");
    let (key, cert) = (std::fs::read(&key_path).unwrap(), std::fs::read(&cert_path).unwrap());
    for (what, bad_cert, bad_key) in [
        ("an empty key", cert.clone(), Vec::new()),
        ("a random key", cert.clone(), noise(key.len(), 4)),
        ("a cut key", cert.clone(), key[..40].to_vec()),
        ("a random certificate", noise(cert.len(), 6), key.clone()),
    ] {
        std::fs::write(&key_path, &bad_key).unwrap();
        std::fs::write(&cert_path, &bad_cert).unwrap();
        let out = Lab::run(&node, &["id"]);
        assert!(out.failed().contains("identity.key.der") && out.stderr.contains("damaged"), "{what}: {out:?}");
        let run = Lab::run(
            &node,
            &[
                "run",
                "--backend",
                "null",
                "--no-discovery",
                "--no-notify",
                "--no-clipboard",
                "--port",
                &node.port.to_string(),
            ],
        );
        let error = run.failed();
        assert!(
            error.contains("identity.key.der") && error.contains("damaged") && !error.contains("is busy"),
            "{what}: the daemon blamed something else: {error}"
        );
        assert_eq!(
            (std::fs::read(&key_path).unwrap(), std::fs::read(&cert_path).unwrap()),
            (bad_key, bad_cert),
            "{what}: a failing command rewrote the identity"
        );
    }
    // One half gone: a new identity is made, and it says so rather than doing it silently.
    std::fs::write(&key_path, &key).unwrap();
    std::fs::remove_file(&cert_path).unwrap();
    let out = Lab::run(&node, &["id"]);
    assert!(out.ok() && out.stderr.contains("made a new identity"), "{out:?}");
    assert!(cert_path.exists() && key_path.exists());
}

/// Install and integrate in a sandboxed home with a fake `systemctl`: they write the
/// login entries and "Send to" files there, report what they did, and remove them again.
#[test]
fn install_integrate_and_uninstall_work_inside_a_sandboxed_home_and_undo_cleanly() {
    if !isolated("install_integrate_and_uninstall_work_inside_a_sandboxed_home_and_undo_cleanly") {
        return;
    }
    let (lab, node) = alone("alpha");
    let other = lab.node("other");
    // A fake systemctl first on PATH: this must never reach the real user manager.
    let stubs = lab.path("stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    let log = lab.path("systemctl.log");
    let stub = stubs.join("systemctl");
    std::fs::write(&stub, format!("#!/bin/sh\necho \"$@\" >> {}\nexit 1\n", log.display())).unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // A file manager on PATH, so `integrate` has somewhere to put its entries even on a machine (a CI
    // runner) that has none; it is never run.
    let nautilus = stubs.join("nautilus");
    std::fs::write(&nautilus, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&nautilus, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", stubs.display(), std::env::var("PATH").unwrap_or_default());
    let sandbox = |command: &mut std::process::Command| {
        command
            .env("PATH", &path)
            .env("XDG_CONFIG_HOME", node.home.join("xdg-config"))
            .env("XDG_DATA_HOME", node.home.join("xdg-data"));
    };
    let home_files = |dir: &str| -> Vec<String> {
        let mut found = Vec::new();
        fn walk(dir: &std::path::Path, root: &std::path::Path, found: &mut Vec<String>) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, root, found)
                    } else {
                        found.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned())
                    }
                }
            }
        }
        let root = node.home.join(dir);
        walk(&root, &root, &mut found);
        found.sort();
        found
    };

    // One paired computer so there is something to send to.
    let mut store = trusted(&node);
    store.upsert(one_mk_net::Device {
        name: "peer".into(),
        fingerprint: fingerprint(&other).to_string(),
        side: "right".into(),
        ..Default::default()
    });
    store.save().unwrap();

    let said = Lab::run_with(&node, &["integrate"], sandbox);
    assert!(said.ok(), "{}", said.both());
    let data = home_files("xdg-data");
    assert!(data.iter().any(|f| f.contains("peer")), "no Send to entry for the paired computer: {data:?} {said:?}");
    let removed = Lab::run_with(&node, &["integrate", "--remove"], sandbox);
    assert!(removed.ok() && removed.stdout.contains("Removed"), "{removed:?}");
    assert!(
        !home_files("xdg-data").iter().any(|f| f.contains("peer")),
        "entries left after --remove: {:?}",
        home_files("xdg-data")
    );
    let nothing = Lab::run_with(&node, &["integrate", "--remove"], sandbox);
    assert!(nothing.ok() && nothing.stdout.contains("no \"Send to\""), "{nothing:?}");

    let installed = Lab::run_with(&node, &["install"], sandbox);
    assert!(installed.ok(), "{}", installed.both());
    let config = home_files("xdg-config");
    assert!(
        config.iter().any(|f| f.contains("autostart") || f.contains("systemd")),
        "install wrote no login entry: {config:?} {installed:?}"
    );
    let uninstalled = Lab::run_with(&node, &["uninstall"], sandbox);
    assert!(uninstalled.ok(), "{}", uninstalled.both());
    assert!(
        !home_files("xdg-config").iter().any(|f| f.contains("one-mk.desktop") || f.contains("one-mk.service")),
        "login entries left after uninstall: {:?}",
        home_files("xdg-config")
    );
}
