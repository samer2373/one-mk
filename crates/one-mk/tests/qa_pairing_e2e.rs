//! End-to-end pairing and invitations with real `one-mk` processes over
//! loopback in a private network namespace: the happy path, a wrong code, a
//! refusal, no answer, an invitation that expires, is reused or is wrong.
#![cfg(target_os = "linux")]

mod qa_common;
use qa_common::*;
use std::time::Duration;

#[test]
fn pairing_with_matching_codes_saves_both_sides_and_the_daemons_connect() {
    if !isolated("pairing_with_matching_codes_saves_both_sides_and_the_daemons_connect") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);

    let (mut host, mut joiner) = lab.pair_start(&alpha, &beta);
    let (host_code, join_code) = (host.expect("code\t", 20), joiner.expect("code\t", 20));
    // `code <name> <six digits>`: the same six digits on both screens, each names the other.
    let (host_fields, join_fields): (Vec<_>, Vec<_>) =
        (host_code.split('\t').collect(), join_code.split('\t').collect());
    assert_eq!(host_fields[2], join_fields[2], "the two computers show different codes");
    assert_eq!(host_fields[2].replace(' ', "").len(), 6, "{host_fields:?}");
    assert!(host_fields[2].chars().all(|c| c.is_ascii_digit() || c == ' '), "{host_fields:?}");
    // Nothing is saved before both say yes.
    assert!(trusted(&alpha).devices().is_empty() && trusted(&beta).devices().is_empty());
    host.say("yes");
    assert!(host.try_expect("paired", 3).is_none(), "paired before the other computer answered");
    assert!(trusted(&alpha).devices().is_empty());
    joiner.say("y");
    let (host_done, join_done) = (host.expect("paired", 20), joiner.expect("paired", 20));
    assert!(
        host_done.contains(&fingerprint(&beta).to_string()) && join_done.contains(&fingerprint(&alpha).to_string()),
        "{host_done} {join_done}"
    );
    assert!(host.finish(20).ok() && joiner.finish(20).ok());

    for (me, other) in [(&alpha, &beta), (&beta, &alpha)] {
        let store = trusted(me);
        assert_eq!(store.devices().len(), 1);
        let device = &store.devices()[0];
        assert_eq!(device.fingerprint, fingerprint(other).to_string());
        assert!(
            device.addrs.iter().any(|a| a.ends_with(&format!(":{}", other.port))),
            "the saved address is not the other's service port: {:?}",
            device.addrs
        );
    }
    // Both running daemons were told to reload and now see each other.
    wait("both online", 30, || {
        let a = lab.status_of(&alpha)?;
        let b = lab.status_of(&beta)?;
        (a.peers.len() == 1 && a.peers[0].online && b.peers.len() == 1 && b.peers[0].online).then_some(())
    });
    let devices = Lab::ok(&alpha, &["devices"]);
    assert!(devices.contains("beta") && devices.contains(&short_id(&beta)), "{devices}");
    // The saved keys and trust file are private to the account.
    use std::os::unix::fs::PermissionsExt;
    for home in [&alpha.home, &beta.home] {
        let mode = |name: &str| std::fs::metadata(home.join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("identity.key.der") & 0o077, 0, "the private key is readable by others");
        assert_eq!(mode("devices.toml") & 0o022, 0, "devices.toml is writable by others");
    }
}

/// Both computers save a pairing at about the same moment and the first to do
/// so dials the other, which may not have saved yet and refuses it. That must
/// not be reported as "it removed this computer" (found by QA: about half of
/// all pairings on loopback logged it), and the two must still end up connected.
#[test]
fn a_fresh_pairing_never_claims_the_other_computer_removed_this_one() {
    if !isolated("a_fresh_pairing_never_claims_the_other_computer_removed_this_one") {
        return;
    }
    for round in 0..6 {
        let mut lab = Lab::new();
        let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
        lab.start_plain(&alpha);
        lab.start_plain(&beta);
        lab.pair(&alpha, &beta);
        wait("both online", 30, || {
            let (a, b) = (lab.status_of(&alpha)?, lab.status_of(&beta)?);
            (a.peers.iter().all(|p| p.online)
                && b.peers.iter().all(|p| p.online)
                && !a.peers.is_empty()
                && !b.peers.is_empty())
            .then_some(())
        });
        std::thread::sleep(Duration::from_millis(700));
        for node in [&alpha, &beta] {
            let (log, status) = (Lab::log(node), Lab::ok(node, &["status"]));
            for text in [&log, &status] {
                assert!(
                    !text.contains("removed this computer") && !text.contains("refuses this computer"),
                    "round {round}, {}: {text}",
                    node.name
                );
            }
            assert!(lab.status(node).removed.is_empty());
        }
    }
}

#[test]
fn a_wrong_code_or_a_refusal_saves_nothing_and_pairing_works_afterwards() {
    if !isolated("a_wrong_code_or_a_refusal_saves_nothing_and_pairing_works_afterwards") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);

    // The person on beta sees a different code and says no; alpha says yes.
    let (mut host, mut joiner) = lab.pair_start(&alpha, &beta);
    host.expect("code\t", 20);
    joiner.expect("code\t", 20);
    host.say("yes");
    joiner.say("no");
    let hosted = host.try_expect("declined", 20).or_else(|| host.try_expect("cancelled", 1));
    assert!(hosted.is_some(), "alpha was not told the other side refused: {:?} {}", host.seen(), host.stderr());
    joiner.expect("cancelled", 20);
    let (one, two) = (host.finish(20), joiner.finish(20));
    assert!(one.ok() && two.ok(), "{one:?} {two:?}");
    for node in [&alpha, &beta] {
        assert!(trusted(node).devices().is_empty(), "{} saved the other after a refusal", node.name);
        assert!(lab.status(node).peers.is_empty());
    }

    // Anything but yes is a no, including garbage and an end of input.
    for answer in ["maybe", "yess", "", "no"] {
        let (mut host, mut joiner) = lab.pair_start(&alpha, &beta);
        host.expect("code\t", 20);
        joiner.expect("code\t", 20);
        host.say(answer);
        joiner.say("yes");
        host.expect("cancelled", 20);
        joiner.expect("declined", 20);
        host.finish(20);
        joiner.finish(20);
        assert!(
            trusted(&alpha).devices().is_empty() && trusted(&beta).devices().is_empty(),
            "answer {answer:?} paired"
        );
    }

    // Closing the answer channel is a no as well.
    let (mut host, mut joiner) = lab.pair_start(&alpha, &beta);
    host.expect("code\t", 20);
    joiner.expect("code\t", 20);
    drop(host.child.stdin.take());
    joiner.say("yes");
    host.expect("cancelled", 20);
    joiner.expect("declined", 20);
    assert!(trusted(&alpha).devices().is_empty());
    host.finish(20);
    joiner.finish(20);

    // And a good pairing right after all that still works.
    lab.pair(&alpha, &beta);
    assert_eq!(trusted(&alpha).devices().len(), 1);
    // Pairing again with someone already paired does not duplicate them.
    lab.pair(&alpha, &beta);
    assert_eq!(trusted(&alpha).devices().len(), 1, "{:?}", trusted(&alpha).devices());
    assert_eq!(trusted(&beta).devices().len(), 1);
}

#[test]
fn pairing_with_nothing_listening_says_how_to_connect_and_fails() {
    if !isolated("pairing_with_nothing_listening_says_how_to_connect_and_fails") {
        return;
    }
    let lab = Lab::new();
    let alpha = lab.node("alpha");
    let dead = format!("127.0.0.1:{}", lab.node("nobody").port);
    let out = Lab::run(&alpha, &["pair", "--no-discovery", "--connect", &dead]);
    let error = out.failed();
    assert!(error.contains("could not reach") && error.contains(&dead) && error.contains("one-mk pair"), "{error}");
    assert!(trusted(&alpha).devices().is_empty());
}

/// Nobody answers for two minutes: pairing gives up, saves nothing and keeps waiting.
#[test]
#[ignore = "takes over two minutes (the answer timeout is 120 s); run with --ignored"]
fn no_answer_to_the_codes_times_out_and_saves_nothing() {
    if !isolated("no_answer_to_the_codes_times_out_and_saves_nothing") {
        return;
    }
    let lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    let (mut host, mut joiner) = lab.pair_start(&alpha, &beta);
    host.expect("code\t", 20);
    joiner.expect("code\t", 20);
    let retry = host.expect("retry", 130);
    assert!(retry.contains("no answer within 120 s"), "{retry}");
    assert!(trusted(&alpha).devices().is_empty() && trusted(&beta).devices().is_empty());
}

fn invite_link(host: &mut Proc) -> String {
    host.expect("invitation\t", 20).trim_start_matches("invitation\t").to_owned()
}

fn invite(node: &Node, extra: &[&str]) -> Proc {
    let listen = format!("{}:0", reachable_address().expect("an invitation needs a non-loopback address"));
    let mut args = vec!["invite", "--machine", "--listen", &listen, "--no-discovery", "--service-port"];
    let port = node.port.to_string();
    args.push(&port);
    args.extend_from_slice(extra);
    Proc::spawn(Lab::command(node, &args))
}

fn join(node: &Node, link: &str) -> Proc {
    let port = node.port.to_string();
    let mut joiner = Proc::spawn(Lab::command(node, &["join", "--machine", "--service-port", &port]));
    joiner.say(link);
    joiner
}

#[test]
fn an_invitation_pairs_once_and_a_second_joiner_is_refused() {
    if !isolated("an_invitation_pairs_once_and_a_second_joiner_is_refused") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta, gamma) = (lab.node("alpha"), lab.node("beta"), lab.node("gamma"));
    for node in [&alpha, &beta, &gamma] {
        lab.start_plain(node);
    }
    let mut host = invite(&alpha, &[]);
    let link = invite_link(&mut host);
    assert!(link.starts_with("onemk://"), "{link}");
    let mut first = join(&beta, &link);
    host.expect("code\t", 20);
    first.expect("code\t", 20);
    assert!(trusted(&alpha).devices().is_empty() && trusted(&beta).devices().is_empty(), "trusted before approval");
    host.say("yes");
    first.say("yes");
    host.expect("paired", 20);
    first.expect("paired", 20);
    assert!(host.finish(20).ok() && first.finish(20).ok());
    assert_eq!(trusted(&alpha).devices().len(), 1);
    assert_eq!(trusted(&beta).devices().len(), 1);
    wait("alpha and beta online", 30, || {
        let status = lab.status_of(&alpha)?;
        (status.peers.len() == 1 && status.peers[0].online).then_some(())
    });

    // The link is burned: a third computer using it gets nothing, and nobody is trusted.
    let second = join(&gamma, &link);
    let out = second.finish(30);
    assert!(!out.ok(), "a burned invitation worked a second time: {}", out.both());
    assert!(!out.stderr.trim().is_empty(), "no reason given");
    assert!(trusted(&gamma).devices().is_empty() && trusted(&alpha).devices().len() == 1);
    // The host process is gone too: its listener closed with the link.
    let again = join(&gamma, &link).finish(30);
    assert!(!again.ok());
}

#[test]
fn an_expired_invitation_is_refused_and_the_host_says_so() {
    if !isolated("an_expired_invitation_is_refused_and_the_host_says_so") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    let mut host = invite(&alpha, &["--expires", "2s"]);
    let link = invite_link(&mut host);
    let out = host.finish(30);
    assert!(!out.ok() && out.stderr.contains("expired"), "{out:?}");
    // Dialing the dead link fails quickly with a reason and trusts nobody.
    let late = join(&beta, &link).finish(60);
    assert!(!late.ok() && !late.stderr.trim().is_empty(), "{late:?}");
    assert!(trusted(&alpha).devices().is_empty() && trusted(&beta).devices().is_empty());
}

#[test]
fn a_declined_or_tampered_invitation_saves_nothing() {
    if !isolated("a_declined_or_tampered_invitation_saves_nothing") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);

    // Garbage is refused before anything is dialed, with something to act on.
    for bad in ["", "hello", "onemk://", "onemk://zz", "https://example.com/invite", "onemk://v1/AAAA"] {
        let port = beta.port.to_string();
        let mut joiner = Proc::spawn(Lab::command(&beta, &["join", "--machine", "--service-port", &port]));
        joiner.say(bad);
        let out = joiner.finish(20);
        out.failed();
        assert!(trusted(&beta).devices().is_empty());
    }

    // A flipped character in a good link no longer opens the invitation.
    let mut host = invite(&alpha, &[]);
    let link = invite_link(&mut host);
    let mut broken = link.clone().into_bytes();
    let last = broken.len() - 3;
    broken[last] = if broken[last] == b'A' { b'B' } else { b'A' };
    let out = join(&beta, &String::from_utf8(broken).unwrap()).finish(30);
    assert!(!out.ok(), "{out:?}");
    assert!(trusted(&beta).devices().is_empty() && trusted(&alpha).devices().is_empty());

    // The host can still refuse the joiner at the code step.
    let mut joiner = join(&beta, &link);
    // A rejected attempt does not burn the invitation: the right link still reaches the code step.
    assert!(host.stderr().contains("Invitation attempt rejected"), "{}", host.stderr());
    host.expect("code\t", 20);
    joiner.expect("code\t", 20);
    host.say("no");
    joiner.say("yes");
    let _ = host.finish(30);
    let _ = joiner.finish(30);
    assert!(
        trusted(&alpha).devices().is_empty() && trusted(&beta).devices().is_empty(),
        "paired although the host said no"
    );
}

#[test]
fn a_second_invitation_cannot_be_opened_on_the_same_computer() {
    if !isolated("a_second_invitation_cannot_be_opened_on_the_same_computer") {
        return;
    }
    let lab = Lab::new();
    let alpha = lab.node("alpha");
    let mut first = invite(&alpha, &[]);
    invite_link(&mut first);
    let second = invite(&alpha, &[]).finish(20);
    assert!(second.failed().contains("already open"), "{second:?}");
    first.signal(libc::SIGINT);
    let out = first.finish(20);
    assert!(!out.ok() && out.stderr.contains("cancelled"), "{out:?}");
    // After cancelling, a new invitation can be made.
    let mut third = invite(&alpha, &[]);
    invite_link(&mut third);
    std::thread::sleep(Duration::from_millis(100));
}
