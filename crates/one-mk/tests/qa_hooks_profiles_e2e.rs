//! Hooks, layout profiles and `bench --peer` between two real daemons.
#![cfg(target_os = "linux")]

mod qa_common;
use one_mk_proto::Side;
use qa_common::*;
use std::process::{Command, Stdio};
use std::time::Duration;

fn two() -> (Lab, Node, Node) {
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    lab.pair(&alpha, &beta);
    (lab, alpha, beta)
}

fn peer_side(lab: &Lab, node: &Node) -> Side {
    lab.status(node).peers[0].side
}

fn script(lab: &Lab, name: &str, body: &str) -> std::path::PathBuf {
    let path = lab.path(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    path
}

fn lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path).unwrap_or_default().lines().map(str::to_owned).collect()
}

fn pgrep(marker: &str) -> bool {
    Command::new("pgrep")
        .args(["-f", &format!("sleep {marker}")])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Inside the private user namespace the folders above the test appear to belong to nobody, which
/// hooks refuse; the documented override (`--trust-permissions`, removed again by `--off`) says it is fine.
fn trust(node: &Node) {
    Lab::ok(node, &["hooks", "--trust-permissions"]);
}

/// Hooks run on enter, leave, connect and disconnect with the documented environment, never
/// through a shell unless asked, and are killed at their timeout.
#[test]
fn hooks_run_on_every_event_with_their_environment_and_time_out() {
    if !isolated("hooks_run_on_every_event_with_their_environment_and_time_out") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    let record = lab.path("events.log");
    let hook = script(
        &lab,
        "hook.sh",
        &format!("echo \"$ONE_MK_EVENT|$ONE_MK_PEER|$ONE_MK_PEER_ID|$ONE_MK_SIDE\" >> {}", record.display()),
    );
    for event in ["--on-enter", "--on-leave", "--on-peer-connect", "--on-peer-disconnect"] {
        Lab::ok(&alpha, &["hooks", event, hook.to_str().unwrap()]);
    }
    trust(&alpha);
    assert!(Lab::ok(&alpha, &["hooks"]).contains("hook.sh"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    lab.pair(&alpha, &beta);
    let id = short_id(&beta);
    let has = |event: &str| {
        lines(&record).iter().any(|line| line.starts_with(&format!("{event}|beta|")) && line.contains(&id))
    };
    wait("peer_connect hook", 20, || has("peer_connect").then_some(()));
    let connect = lines(&record).into_iter().find(|line| line.starts_with("peer_connect")).unwrap();
    assert_eq!(connect.split('|').count(), 4, "{connect}");

    std::thread::sleep(Duration::from_millis(700));
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("leave hook", 20, || has("leave").then_some(()));
    std::thread::sleep(Duration::from_millis(700));
    Lab::ok(&alpha, &["switch", "here"]);
    wait("enter hook", 20, || has("enter").then_some(()));
    lab.kill9(&beta);
    wait("peer_disconnect hook", 30, || has("peer_disconnect").then_some(()));

    // A hook that hangs is killed at its timeout, and nothing is left running.
    let marker = "31.0417";
    let slow = script(&lab, "slow.sh", &format!("exec sleep {marker}"));
    Lab::ok(&alpha, &["hooks", "--off"]);
    Lab::ok(&alpha, &["hooks", "--timeout", "1", "--on-peer-connect", slow.to_str().unwrap()]);
    trust(&alpha);
    lab.start_plain(&beta);
    wait("the slow hook to start", 30, || pgrep(marker).then_some(()));
    wait("the slow hook to be killed at its timeout", 15, || (!pgrep(marker)).then_some(()));
    assert!(lab.status_of(&alpha).is_some(), "{}", Lab::log(&alpha));

    // Shell syntax in a command is text, not code, while shell mode is off.
    let pwned = lab.path("pwned");
    Lab::ok(&alpha, &["hooks", "--off"]);
    Lab::ok(
        &alpha,
        &["hooks", "--on-enter", &format!("/bin/echo $(touch {0}) `touch {0}` ; touch {0}", pwned.display())],
    );
    trust(&alpha);
    wait("beta connected", 30, || lab.status(&alpha).peers.iter().all(|p| p.online).then_some(()));
    Lab::ok(&alpha, &["switch", "beta"]);
    Lab::ok(&alpha, &["switch", "here"]);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!pwned.exists(), "a hook was run through a shell although shell mode is off");
    Lab::ok(&alpha, &["hooks", "--off"]);
}

/// A computer that names itself with shell syntax is made harmless before a shell-mode hook sees it.
#[test]
fn a_hostile_computer_name_cannot_inject_commands_into_a_shell_mode_hook() {
    if !isolated("a_hostile_computer_name_cannot_inject_commands_into_a_shell_mode_hook") {
        return;
    }
    if !names_work() {
        eprintln!("SKIP: host names cannot be set here");
        return;
    }
    let mut lab = Lab::new();
    let alpha = lab.node("alpha");
    let canary = lab.path("PWNED");
    let evil = "x;touch PWNED;y$(id)`id`";
    let beta = lab.node(evil);
    let record = lab.path("shell.log");
    Lab::ok(
        &alpha,
        &[
            "hooks",
            "--shell",
            "on",
            "--on-peer-connect",
            &format!("cd {} && echo \"$ONE_MK_PEER\" >> {}", lab.root.path().display(), record.display()),
        ],
    );
    trust(&alpha);
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    lab.pair(&alpha, &beta);
    wait("the hook to run", 20, || (!lines(&record).is_empty()).then_some(()));
    let name = lines(&record)[0].clone();
    assert!(
        !name.contains(';') && !name.contains('$') && !name.contains('`'),
        "the peer name reached the shell unclean: {name}"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert!(!canary.exists(), "a computer name ran a command");
}

/// Named layouts: save where everything is, change it, switch back, and the switch reaches the other computer.
#[test]
fn layout_profiles_save_switch_list_and_delete_and_the_switch_reaches_both_computers() {
    if !isolated("layout_profiles_save_switch_list_and_delete_and_the_switch_reaches_both_computers") {
        return;
    }
    let (lab, alpha, beta) = two();
    Lab::ok(&alpha, &["layout", "beta", "right"]);
    wait("settled right", 20, || (peer_side(&lab, &beta) == Side::Left).then_some(()));
    assert!(Lab::ok(&alpha, &["profile", "save", "desk"]).contains("desk"));
    Lab::ok(&alpha, &["layout", "beta", "top"]);
    wait("settled top", 20, || (peer_side(&lab, &beta) == Side::Bottom).then_some(()));
    Lab::ok(&alpha, &["profile", "save", "couch"]);
    let list = Lab::ok(&alpha, &["profile", "list"]);
    assert!(list.contains("desk") && list.contains("couch"), "{list}");
    let used = Lab::ok(&alpha, &["profile", "use", "desk"]);
    assert!(used.contains("desk"), "{used}");
    wait("desk layout in place on both", 20, || {
        (peer_side(&lab, &alpha) == Side::Right && peer_side(&lab, &beta) == Side::Left).then_some(())
    });
    Lab::ok(&alpha, &["profile", "use", "couch"]);
    wait("couch layout in place on both", 20, || {
        (peer_side(&lab, &alpha) == Side::Top && peer_side(&lab, &beta) == Side::Bottom).then_some(())
    });
    // Automatic switching rules are stored and shown.
    Lab::ok(&alpha, &["profile", "auto", "desk", "--ssid", "Home Wi-Fi", "--network", "192.168.1.0/24"]);
    assert!(Lab::ok(&alpha, &["profile", "list"]).contains("Home Wi-Fi"));
    Lab::run(&alpha, &["profile", "auto", "desk", "--network", "not-a-network"]).failed();
    Lab::run(&alpha, &["profile", "auto", "nope", "--ssid", "x"]).failed();
    Lab::ok(&alpha, &["profile", "auto", "desk", "--off"]);
    // Unknown names and hostile names are refused; nothing changes.
    assert!(Lab::run(&alpha, &["profile", "use", "nope"]).failed().contains("nope"));
    assert!(Lab::run(&alpha, &["profile", "delete", "nope"]).failed().contains("nope"));
    for bad in ["", "   ", "a\nb"] {
        Lab::run(&alpha, &["profile", "save", bad]).failed();
    }
    assert_eq!(peer_side(&lab, &alpha), Side::Top);
    assert!(Lab::ok(&alpha, &["profile", "delete", "couch"]).contains("couch"));
    assert!(!Lab::ok(&alpha, &["profile", "list"]).contains("couch"));
    // A profile naming a computer that was forgotten since still applies without a crash.
    Lab::ok(&alpha, &["forget", "beta"]);
    let after = Lab::run(&alpha, &["profile", "use", "desk"]);
    assert!(!after.both().contains("panicked"), "{after:?}");
}

/// `bench --peer` measures the paired computer through the running daemons.
#[test]
fn bench_against_a_paired_computer_measures_the_real_link() {
    if !isolated("bench_against_a_paired_computer_measures_the_real_link") {
        return;
    }
    let (_lab, alpha, _beta) = two();
    let out = Lab::run(&alpha, &["bench", "--peer", "beta", "--count", "200", "--rate", "1000"]);
    let text = out.success();
    assert!(text.contains("round trip") && !text.contains("NaN"), "{text}");
    Lab::run(&alpha, &["bench", "--peer", "nobody"]).failed();
}
