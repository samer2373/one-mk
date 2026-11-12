//! End-to-end behaviour of paired daemons: layout, status, doctor, control
//! hand-over, takeback, restarts after `kill -9`, settings and device files that
//! are old, partial or damaged. Real binaries, private homes, `--backend null`,
//! one private network namespace per test where `unshare` allows it.
#![cfg(target_os = "linux")]

mod qa_common;
use one_mk_proto::Side;
use qa_common::*;
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

#[test]
fn layout_changes_apply_to_both_computers_at_once_and_survive_a_restart() {
    if !isolated("layout_changes_apply_to_both_computers_at_once_and_survive_a_restart") {
        return;
    }
    let (mut lab, alpha, beta) = two();
    // Opposite sides on the two computers, whichever one says it.
    for (side, mirrored) in [("right", Side::Left), ("top", Side::Bottom), ("left", Side::Right), ("bottom", Side::Top)]
    {
        let said = Lab::ok(&alpha, &["layout", "beta", side]);
        assert!(said.contains("beta") && said.contains("now"), "{said}");
        wait(&format!("beta to be {mirrored:?} of alpha ({side})"), 20, || {
            (peer_side(&lab, &beta) == mirrored && peer_side(&lab, &alpha).to_string() == side).then_some(())
        });
    }
    // The other computer can change it as well, and then it is alpha that follows.
    Lab::ok(&beta, &["layout", "alpha", "left"]);
    wait("alpha to follow beta's choice", 20, || {
        (peer_side(&lab, &alpha) == Side::Right && peer_side(&lab, &beta) == Side::Left).then_some(())
    });

    // Offsets, and back to anywhere along the edge.
    Lab::ok(&alpha, &["layout", "beta", "--offset", "-1920,100"]);
    let devices = Lab::ok(&alpha, &["devices"]);
    assert!(devices.contains("-1920,100"), "{devices}");
    Lab::ok(&alpha, &["layout", "beta", "--offset", "none"]);
    assert!(!Lab::ok(&alpha, &["devices"]).contains("-1920"));
    Lab::ok(&alpha, &["layout", "beta", "--swap-ctrl-cmd", "on"]);
    assert_eq!(lab.status(&alpha).peers[0].swap, "on");
    Lab::ok(&alpha, &["layout", "beta", "--swap-ctrl-cmd", "auto"]);

    // Kill -9 both, start them again: the arrangement is the same and they reconnect.
    Lab::ok(&alpha, &["layout", "beta", "top"]);
    wait("settled", 20, || (peer_side(&lab, &beta) == Side::Bottom).then_some(()));
    lab.kill9(&alpha);
    lab.kill9(&beta);
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    wait("both connected again", 40, || {
        let (a, b) = (lab.status_of(&alpha)?, lab.status_of(&beta)?);
        (a.peers.iter().all(|p| p.online) && b.peers.iter().all(|p| p.online)).then_some(())
    });
    assert_eq!(peer_side(&lab, &alpha), Side::Top);
    assert_eq!(peer_side(&lab, &beta), Side::Bottom);
}

#[test]
fn rename_and_forget_take_effect_in_the_running_daemon_and_forget_is_one_sided() {
    if !isolated("rename_and_forget_take_effect_in_the_running_daemon_and_forget_is_one_sided") {
        return;
    }
    let (lab, alpha, beta) = two();
    let said = Lab::ok(&alpha, &["rename", "beta", "the", "big", "box"]);
    assert!(said.contains("the big box"), "{said}");
    wait("status shows the alias", 20, || {
        lab.status(&alpha).peers.iter().any(|p| p.name == "the big box").then_some(())
    });
    // Only this computer shows the alias.
    assert!(Lab::ok(&beta, &["devices"]).contains("alpha"));
    assert!(Lab::ok(&alpha, &["devices"]).contains("the big box"));
    Lab::ok(&alpha, &["layout", "the big box", "left"]);
    Lab::ok(&alpha, &["rename", "the big box", "--reset"]);
    assert!(Lab::ok(&alpha, &["devices"]).contains("beta"));
    // An empty or hostile name is refused with a reason, the old one stays.
    for bad in ["", "   "] {
        let out = Lab::run(&alpha, &["rename", "beta", bad]);
        out.failed();
        assert!(Lab::ok(&alpha, &["devices"]).contains("beta"), "{bad:?}");
    }
    // Control characters are taken out of a name, not kept (they would break lines and terminals).
    Lab::ok(&alpha, &["rename", "beta", "line\nbreak\ttab\u{1b}[31m"]);
    let listed = Lab::ok(&alpha, &["devices"]);
    assert!(listed.lines().count() == 2 && !listed.contains('\u{1b}') && !listed.contains('\t'), "{listed:?}");
    assert!(Lab::ok(&alpha, &["rename", &short_id(&beta), "--reset"]).contains("beta"));

    // Forgetting: gone on this side at once and disconnected; the other side is not asked.
    let said = Lab::ok(&alpha, &["forget", "beta"]);
    assert!(said.contains("Forgot beta"), "{said}");
    wait("alpha has no peers", 20, || lab.status(&alpha).peers.is_empty().then_some(()));
    assert_eq!(trusted(&beta).devices().len(), 1, "forget on one side removed it from the other's file");
    wait("beta shows alpha removed it", 40, || (!lab.status(&beta).removed.is_empty()).then_some(()));
    let status = Lab::ok(&beta, &["status"]);
    assert!(status.contains("removed this computer") && status.contains("one-mk forget"), "{status}");
    // The advice works: forget on beta too, and pair again.
    Lab::ok(&beta, &["forget", &short_id(&alpha)]);
    assert!(Lab::run(&beta, &["forget", "alpha"]).failed().contains("no paired computer"));
    lab.pair(&alpha, &beta);
    wait("online again", 30, || {
        (lab.status(&alpha).peers.iter().all(|p| p.online) && !lab.status(&alpha).peers.is_empty()).then_some(())
    });
}

#[test]
fn control_can_be_handed_over_and_taken_back_every_way_and_nothing_sticks() {
    if !isolated("control_can_be_handed_over_and_taken_back_every_way_and_nothing_sticks") {
        return;
    }
    let (lab, alpha, beta) = two();
    let controlling = |node: &Node| lab.status(node).controlling.is_some();
    let controlled = |node: &Node| lab.status(node).controlled_by.is_some();
    let free = || {
        wait("nothing is controlled", 20, || {
            (!controlling(&alpha) && !controlled(&beta) && !controlled(&alpha) && !controlling(&beta)).then_some(())
        });
    };

    // switch to a computer, then `switch here`.
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("alpha controls beta", 20, || (controlling(&alpha) && controlled(&beta)).then_some(()));
    assert!(
        Lab::ok(&beta, &["status"]).contains("alpha is controlling this computer"),
        "{}",
        Lab::ok(&beta, &["status"])
    );
    Lab::ok(&alpha, &["switch", "here"]);
    free();

    // The controlled computer takes itself back (emergency release).
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("alpha controls beta", 20, || controlled(&beta).then_some(()));
    let said = Lab::ok(&beta, &["takeback"]);
    assert!(said.contains("Took control back"), "{said}");
    free();
    // Nothing to take back: a plain answer, not an error or a hang.
    let again = Lab::run(&beta, &["takeback"]);
    assert!(again.ok() || !again.stderr.trim().is_empty(), "{again:?}");

    // `next` goes round and round: beta, then back here.
    Lab::ok(&alpha, &["switch", "next"]);
    wait("alpha controls beta via next", 20, || controlling(&alpha).then_some(()));
    Lab::ok(&alpha, &["switch", "next"]);
    free();
    // Switching to where we already are is not an error.
    Lab::ok(&alpha, &["switch", "here"]);
    // Unknown and hostile targets are refused with a reason; nothing moves.
    for target in ["nobody", "", "bad\nname"] {
        Lab::run(&alpha, &["switch", target]).failed();
    }
    assert!(!controlling(&alpha));

    // Locking the pointer: it says so, shows in status, and unlocks.
    assert!(Lab::ok(&alpha, &["lock-pointer", "on"]).contains("locked"));
    assert!(lab.status(&alpha).pointer_locked);
    assert!(Lab::ok(&alpha, &["lock-pointer"]).contains("locked"));
    Lab::ok(&alpha, &["lock-pointer", "toggle"]);
    assert!(!lab.status(&alpha).pointer_locked);
    Lab::run(&alpha, &["lock-pointer", "sideways"]).failed();

    // Broadcast on, shown, off; its idle timeout is changed and refused when silly.
    assert!(Lab::ok(&alpha, &["broadcast", "on"]).contains("Broadcasting to 1 computer"));
    assert_eq!(lab.status(&alpha).broadcast.len(), 1);
    Lab::ok(&alpha, &["broadcast", "off"]);
    assert!(lab.status(&alpha).broadcast.is_empty());
    Lab::ok(&alpha, &["broadcast", "--idle-off", "5"]);
    Lab::run(&alpha, &["broadcast", "--idle-off", "-1"]).failed();
    free();
}

#[test]
fn the_controller_going_away_never_leaves_the_other_computer_stuck() {
    if !isolated("the_controller_going_away_never_leaves_the_other_computer_stuck") {
        return;
    }
    let (mut lab, alpha, beta) = two();
    // beta is controlled by alpha, then alpha dies without a goodbye (kill -9).
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("alpha controls beta", 20, || lab.status(&beta).controlled_by.is_some().then_some(()));
    lab.kill9(&alpha);
    wait("beta is free again after its controller died", 30, || {
        lab.status(&beta).controlled_by.is_none().then_some(())
    });
    wait("beta sees alpha offline", 30, || lab.status(&beta).peers.iter().all(|p| !p.online).then_some(()));

    // alpha restarts: control is back on alpha, nothing remembers the old hand-over.
    lab.start_plain(&alpha);
    wait("reconnected", 40, || {
        let (a, b) = (lab.status_of(&alpha)?, lab.status_of(&beta)?);
        (a.peers.iter().all(|p| p.online) && b.peers.iter().all(|p| p.online)).then_some(())
    });
    assert!(lab.status(&alpha).controlling.is_none() && lab.status(&beta).controlled_by.is_none());

    // The other way: the computer being controlled dies; the controller gets its pointer back.
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("alpha controls beta", 20, || lab.status(&alpha).controlling.is_some().then_some(()));
    lab.kill9(&beta);
    wait("alpha is back after beta died", 40, || lab.status(&alpha).controlling.is_none().then_some(()));
    // The control socket belongs to a dead process; a new daemon takes over the folder.
    lab.start_plain(&beta);
    wait("beta back", 40, || (lab.status_of(&alpha)?.peers.iter().all(|p| p.online)).then_some(()));
    assert!(Lab::log(&alpha).matches("panicked").count() == 0 && Lab::log(&beta).matches("panicked").count() == 0);
}

#[test]
fn a_second_daemon_for_the_same_folder_is_refused_and_stop_and_sigterm_clean_up() {
    if !isolated("a_second_daemon_for_the_same_folder_is_refused_and_stop_and_sigterm_clean_up") {
        return;
    }
    let mut lab = Lab::new();
    let alpha = lab.node("alpha");
    lab.start_plain(&alpha);
    let mut second = Node { port: lab.node("spare").port, ..alpha.clone() };
    second.name = "alpha".into();
    let out = Lab::run(
        &second,
        &[
            "run",
            "--backend",
            "null",
            "--no-discovery",
            "--no-notify",
            "--no-clipboard",
            "--port",
            &second.port.to_string(),
        ],
    );
    assert!(out.failed().contains("already running"), "{out:?}");
    assert!(lab.status_of(&alpha).is_some(), "the refused second daemon broke the first");

    // The same UDP port in use by another folder's daemon: a reason that names the port.
    let other = lab.node("other");
    let busy = Lab::run(
        &other,
        &[
            "run",
            "--backend",
            "null",
            "--no-discovery",
            "--no-notify",
            "--no-clipboard",
            "--port",
            &alpha.port.to_string(),
        ],
    );
    assert!(!busy.ok() && !busy.stderr.trim().is_empty() && !busy.stderr.contains("panicked"), "{busy:?}");

    // `stop` ends it, the socket goes, a new one starts.
    let said = Lab::ok(&alpha, &["stop"]);
    assert!(said.to_lowercase().contains("stop"), "{said}");
    wait("daemon to exit", 20, || (!lab.daemon_alive(&alpha)).then_some(()));
    assert!(!alpha.home.join("control.sock").exists(), "control.sock left behind by a clean stop");
    assert!(Lab::run(&alpha, &["stop"]).ok(), "stopping what is not running must not fail");

    lab.start_plain(&alpha);
    lab.terminate(&alpha);
    assert!(!alpha.home.join("control.sock").exists(), "control.sock left behind by SIGTERM");
    // After kill -9 the stale socket and lock file are left; the next start must not be blocked by them.
    lab.start_plain(&alpha);
    lab.kill9(&alpha);
    assert!(Lab::run(&alpha, &["status"]).ok(), "a stale socket made status fail");
    lab.start_plain(&alpha);
    assert!(lab.status_of(&alpha).is_some());
}

/// `one-mk status` and `doctor` tell the truth about who is online, in text and JSON.
#[test]
fn status_and_doctor_describe_the_running_pair_and_their_json_is_valid() {
    if !isolated("status_and_doctor_describe_the_running_pair_and_their_json_is_valid") {
        return;
    }
    let (lab, alpha, beta) = two();
    let status = Lab::ok(&alpha, &["status"]);
    assert!(status.contains("is running on alpha") && status.contains("beta") && status.contains("online"), "{status}");
    assert!(status.contains("The mouse and keyboard are on this computer"), "{status}");
    let decoded = lab.status(&alpha);
    assert_eq!((decoded.name.as_str(), decoded.peers.len()), ("alpha", 1));
    assert_eq!(decoded.id, short_id(&alpha));
    assert_eq!(decoded.port, alpha.port);

    // doctor: valid JSON with the same shape every time, a non-zero exit only when something failed.
    for node in [&alpha, &beta] {
        let out = Lab::run(node, &["doctor", "--json"]);
        let json: serde_json::Value =
            serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("doctor --json is not JSON ({e}): {out:?}"));
        let checks = json["checks"].as_array().expect("checks array");
        assert!(!checks.is_empty());
        for check in checks {
            for key in ["id", "title", "status", "detail"] {
                assert!(check.get(key).is_some_and(|v| v.is_string()), "check without {key}: {check}");
            }
            assert!(check["visible"].is_boolean() && (check["fix"].is_null() || check["fix"].is_string()), "{check}");
            assert!(["pass", "warn", "fail", "skip"].contains(&check["status"].as_str().unwrap()), "{check}");
        }
        let failed = checks.iter().any(|check| check["status"] == "fail" && check["visible"] == true);
        assert_eq!(out.ok(), !failed, "exit status {:?} disagrees with the checks: {}", out.code, out.stdout);
        assert_eq!(json["ok"], !failed, "{json}");
        let summary = &json["summary"];
        assert!(["pass", "skip", "warn", "fail"].iter().all(|key| summary[key].is_u64()), "{json}");
        let all = Lab::run(node, &["doctor", "--json", "--all"]);
        let all_json: serde_json::Value = serde_json::from_str(&all.stdout).unwrap();
        assert!(all_json["checks"].as_array().unwrap().len() >= checks.len());
        let text = Lab::run(node, &["doctor"]).stdout;
        assert!(text.contains("one-mk is running") && text.contains("Paired with 1 computer"), "{text}");
    }
    // While nobody listens at all, doctor says how to start it.
    let stopped = Lab::ok(&beta, &["stop"]);
    drop(stopped);
    wait("beta stopped", 20, || (lab.status_of(&beta).is_none()).then_some(()));
    let text = Lab::run(&beta, &["doctor"]).stdout;
    assert!(text.contains("not running") && text.contains("one-mk run"), "{text}");
    wait("alpha notices beta is gone", 20, || (lab.status(&alpha).peers.iter().all(|p| !p.online)).then_some(()));
    let after = Lab::run(&alpha, &["doctor"]);
    assert!(after.stdout.contains("not connected") || after.stdout.contains("offline"), "{}", after.stdout);
    std::thread::sleep(Duration::from_millis(50));
}
