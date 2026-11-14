//! Hostile conditions between real daemons: a computer that stalls and wakes (SIGSTOP/SIGCONT, as
//! a laptop lid or a Wi-Fi drop looks from outside), commands fired at the same time, hundreds of
//! files in one send, two senders to one receiver, and a locked screen.
#![cfg(target_os = "linux")]

mod qa_common;
use qa_common::*;
use std::time::{Duration, Instant};

fn two() -> (Lab, Node, Node) {
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    lab.pair(&alpha, &beta);
    (lab, alpha, beta)
}

fn online(lab: &Lab, node: &Node) -> bool {
    lab.status_of(node).is_some_and(|status| !status.peers.is_empty() && status.peers.iter().all(|peer| peer.online))
}

#[test]
fn a_stalled_computer_is_let_go_and_found_again_when_it_wakes() {
    if !isolated("a_stalled_computer_is_let_go_and_found_again_when_it_wakes") {
        return;
    }
    let (lab, alpha, beta) = two();
    let pid = lab.daemon_pid(&beta) as i32;
    // beta freezes (a laptop lid, a Wi-Fi drop) while idle: alpha notices and shows it offline.
    unsafe { libc::kill(pid, libc::SIGSTOP) };
    wait("alpha to see beta offline", 40, || (!online(&lab, &alpha)).then_some(()));
    // A send to a frozen computer gives up with a reason in a bounded time.
    let started = std::time::Instant::now();
    let src = lab.path("f.txt");
    std::fs::write(&src, "x").unwrap();
    let out = Lab::run(&alpha, &["send", "--to", "beta", src.to_str().unwrap()]);
    out.failed();
    assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
    unsafe { libc::kill(pid, libc::SIGCONT) };
    wait("both online again after the wake", 60, || (online(&lab, &alpha) && online(&lab, &beta)).then_some(()));
    let out = Lab::run(&alpha, &["send", "--to", "beta", src.to_str().unwrap()]);
    assert!(out.ok(), "{}", out.both());

    // Frozen while it is being controlled: the controller gets its pointer back by itself,
    // and nothing is stuck after the wake.
    Lab::ok(&alpha, &["switch", "beta"]);
    wait("alpha controls beta", 20, || lab.status(&alpha).controlling.is_some().then_some(()));
    unsafe { libc::kill(pid, libc::SIGSTOP) };
    wait("alpha to take its pointer back from a frozen computer", 60, || {
        lab.status(&alpha).controlling.is_none().then_some(())
    });
    unsafe { libc::kill(pid, libc::SIGCONT) };
    wait("online again", 60, || (online(&lab, &alpha) && online(&lab, &beta)).then_some(()));
    wait("beta is not left controlled", 30, || (lab.status(&beta).controlled_by.is_none()).then_some(()));
    assert!(lab.status(&alpha).controlling.is_none());
}

#[test]
fn commands_fired_at_once_never_corrupt_the_settings_or_the_device_list() {
    if !isolated("commands_fired_at_once_never_corrupt_the_settings_or_the_device_list") {
        return;
    }
    let (lab, alpha, beta) = two();
    let mut procs = Vec::new();
    let sides = ["left", "right", "top", "bottom"];
    // By id: `rename` changes the name another command in the storm would look for.
    let id = short_id(&beta);
    for round in 0..3 {
        for (index, side) in sides.iter().enumerate() {
            procs.push(Proc::spawn(Lab::command(&alpha, &["layout", &id, side])));
            procs.push(Proc::spawn(Lab::command(
                &alpha,
                &["edge", "--dwell", &format!("{}", round * 10 + index as u32 + 1)],
            )));
            procs.push(Proc::spawn(Lab::command(&alpha, &["rename", &id, &format!("name {round}{index}")])));
            procs.push(Proc::spawn(Lab::command(&alpha, &["clipboard", "--max-mb", &format!("{}", 5 + index)])));
            procs.push(Proc::spawn(Lab::command(&alpha, &["pointer", "--speed", "1.5"])));
        }
    }
    let mut failures = Vec::new();
    for proc in procs {
        let out = proc.finish(90);
        // A busy lock is a reason to retry, never a reason to lose or tear the file.
        if !out.ok() {
            failures.push(out.stderr.clone());
        }
        assert!(!out.both().contains("panicked"), "{out:?}");
    }
    assert!(
        failures.iter().all(|f| f.contains("lock") || f.contains("busy") || f.contains("another one-mk")),
        "a command failed for another reason: {failures:?}"
    );
    assert!(failures.len() < 12, "too many commands refused for a lock: {}", failures.len());
    // Both files still read, whatever order things happened in.
    let settings = std::fs::read_to_string(alpha.home.join("settings.toml")).unwrap();
    toml::from_str::<toml::Table>(&settings).unwrap_or_else(|e| panic!("settings.toml is torn: {e}\n{settings}"));
    let devices = std::fs::read_to_string(alpha.home.join("devices.toml")).unwrap();
    toml::from_str::<toml::Table>(&devices).unwrap_or_else(|e| panic!("devices.toml is torn: {e}\n{devices}"));
    assert_eq!(trusted(&alpha).devices().len(), 1);
    assert_eq!(trusted(&alpha).devices()[0].fingerprint, fingerprint(&beta).to_string());
    assert!(lab.status_of(&alpha).is_some() && lab.status_of(&beta).is_some());
    assert!(!Lab::log(&alpha).contains("panicked"));
    // No temp or lock litter that could confuse a later run.
    let litter: Vec<_> = std::fs::read_dir(&alpha.home)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp") || n.ends_with(".new") || n.contains("~"))
        .collect();
    assert!(litter.is_empty(), "{litter:?}");
}

#[test]
fn many_small_files_and_two_senders_at_once_all_arrive_without_mixing() {
    if !isolated("many_small_files_and_two_senders_at_once_all_arrive_without_mixing") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta, gamma) = (lab.node("alpha"), lab.node("beta"), lab.node("gamma"));
    for node in [&alpha, &beta, &gamma] {
        lab.start_plain(node);
    }
    lab.pair(&alpha, &beta);
    lab.pair(&gamma, &beta);
    // 600 files in nested folders from one sender, and a second sender sending files of the
    // same names at the same moment.
    let tree = |who: &str| {
        let root = lab.path(&format!("tree-{who}/shared"));
        for i in 0..600 {
            let dir = root.join(format!("d{}", i % 7));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("f{i}.txt")), format!("{who}-{i}")).unwrap();
        }
        root
    };
    let (from_alpha, from_gamma) = (tree("alpha"), tree("gamma"));
    let first = Proc::spawn(Lab::command(&alpha, &["send", "--to", "beta", from_alpha.to_str().unwrap()]));
    let second = Proc::spawn(Lab::command(&gamma, &["send", "--to", "beta", from_gamma.to_str().unwrap()]));
    let (one, two) = (first.finish(180), second.finish(180));
    assert!(one.ok(), "{}", one.both());
    assert!(two.ok(), "{}", two.both());
    // Every one of the 1200 files is somewhere under the receiver's folder, each with its own content.
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with(".one-mk-partial") {
                continue;
            }
            if path.is_dir() { walk(&path, out) } else { out.push(path) }
        }
    }
    let mut found = Vec::new();
    walk(&beta.downloads, &mut found);
    assert_eq!(found.len(), 1200, "files received: {}", found.len());
    let mut contents: Vec<String> = found.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    contents.sort();
    contents.dedup();
    assert_eq!(contents.len(), 1200, "two files ended up with the same content, or one was lost");
    for who in ["alpha", "gamma"] {
        for i in [0usize, 1, 299, 599] {
            assert!(contents.contains(&format!("{who}-{i}")), "{who}-{i} is missing");
        }
    }
    assert!(Lab::ok(&beta, &["partials"]).contains("No unfinished"));
    assert!(lab.status(&beta).transfers.is_empty());
}

#[test]
fn lock_all_locks_every_screen_and_says_so() {
    if !isolated("lock_all_locks_every_screen_and_says_so") {
        return;
    }
    let (lab, alpha, beta) = two();
    let said = Lab::ok(&alpha, &["lock-all"]);
    assert!(said.contains("Locking"), "{said}");
    wait("both screens locked", 20, || {
        (lab.status(&alpha).screen_locked && lab.status(&beta).screen_locked).then_some(())
    });
    assert!(Lab::log(&beta).contains("asked to lock"), "{}", Lab::log(&beta));
    // While locked, broadcasting is refused with the reason (nothing can type into a locked screen).
    let refused = Lab::run(&alpha, &["broadcast", "on"]);
    assert!(refused.failed().contains("locked"), "{refused:?}");
    assert!(lab.status(&alpha).broadcast.is_empty());
    // So is switching to another computer, in the same words (what is typed at a lock screen
    // must not be captured and sent elsewhere); coming home is not refused.
    for args in [&["switch", "beta"][..], &["switch", "next"]] {
        let switched = Lab::run(&alpha, args);
        assert!(switched.failed().contains("this computer's screen is locked; unlock it first"), "{args:?}: {switched:?}");
    }
    assert!(lab.status(&alpha).controlling.is_none());
    assert!(Lab::run(&alpha, &["switch", "here"]).ok());
    // Settings of lock sync are remembered and shown.
    Lab::ok(&alpha, &["lock-sync", "on"]);
    assert!(Lab::ok(&alpha, &["lock-sync"]).contains("on"));
    assert!(lab.status_of(&alpha).is_some());
}

/// A locked computer is not entered: the pointer that would go onto its lock screen comes home and
/// nothing is typed there (what is typed at a lock screen must not travel between computers, in
/// either direction). The null backend's fake locker only locks, so "beta unlocked, alpha locked"
/// is made by restarting beta (a restart is an unlocked screen).
#[test]
fn a_locked_computer_is_never_entered_over_an_edge() {
    if !isolated("a_locked_computer_is_never_entered_over_an_edge") {
        return;
    }
    let mut lab = Lab::new();
    let (mut alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    // The refusal is an info line: ask for it, so the test sees that the refusal itself happened.
    alpha.env.push(("RUST_LOG".into(), "warn,one_mk=info".into()));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    lab.pair(&alpha, &beta);
    // Online and each knows the other's screens (a switch needs them).
    let ready = |lab: &Lab, node: &Node| {
        lab.status_of(node).is_some_and(|s| !s.peers.is_empty() && s.peers.iter().all(|p| p.online && !p.screens.is_empty()))
    };
    wait("both ready", 40, || (ready(&lab, &alpha) && ready(&lab, &beta)).then_some(()));
    Lab::ok(&alpha, &["lock-all"]);
    wait("both screens locked", 20, || (lab.status(&alpha).screen_locked && lab.status(&beta).screen_locked).then_some(()));
    // Beta comes back unlocked while alpha stays locked.
    lab.kill9(&beta);
    lab.start_plain(&beta);
    wait("beta online and unlocked, alpha locked", 40, || {
        let (a, b) = (lab.status_of(&alpha)?, lab.status_of(&beta)?);
        (a.screen_locked && !b.screen_locked && ready(&lab, &alpha) && ready(&lab, &beta)).then_some(())
    });
    // Beta goes to alpha (the same request as crossing its edge): alpha refuses, the pointer is home.
    Lab::ok(&beta, &["switch", "alpha"]);
    wait("beta to be home again", 20, || lab.status(&beta).controlling.is_none().then_some(()));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(lab.status(&alpha).controlled_by.is_none(), "alpha's lock screen was controlled");
    assert!(lab.status(&beta).controlling.is_none());
    let log = Lab::log(&alpha);
    assert!(log.contains("refused control: this computer's screen is locked"), "{log}");
    assert!(!log.contains("is controlling this computer"), "{log}");
    assert!(lab.status(&alpha).screen_locked);
    // Alpha, locked, cannot go to beta either (nothing typed at its lock screen is sent).
    let refused = Lab::run(&alpha, &["switch", "beta"]);
    assert!(refused.failed().contains("this computer's screen is locked; unlock it first"), "{refused:?}");
    // Control: once alpha is back unlocked, beta can go there.
    lab.kill9(&alpha);
    lab.start_plain(&alpha);
    wait("both ready", 40, || (ready(&lab, &alpha) && ready(&lab, &beta)).then_some(()));
    // Beta can still be holding the link to the alpha that was killed (it notices after 4 s of silence), and
    // a request sent into it is lost: ask again every few seconds until the new alpha is controlled.
    let mut asked: Option<Instant> = None;
    wait("beta controls the unlocked alpha", 40, || {
        if lab.status(&alpha).controlled_by.is_some() {
            return Some(());
        }
        if asked.is_none_or(|at| at.elapsed() >= Duration::from_secs(4)) {
            let _ = Lab::run(&beta, &["switch", "alpha"]);
            asked = Some(Instant::now());
        }
        None
    });
}

#[test]
fn a_full_restart_of_everything_comes_back_with_no_manual_step() {
    if !isolated("a_full_restart_of_everything_comes_back_with_no_manual_step") {
        return;
    }
    let (mut lab, alpha, beta) = two();
    let src = lab.path("again.txt");
    std::fs::write(&src, "before").unwrap();
    assert!(Lab::run(&alpha, &["send", "--to", "beta", src.to_str().unwrap()]).ok());
    for round in 0..3 {
        // Everything dies at once, ungracefully, the way a power cut does.
        lab.kill9(&alpha);
        lab.kill9(&beta);
        assert!(lab.status_of(&alpha).is_none());
        // Restart in the other order each time: no start-up order is required.
        if round % 2 == 0 {
            lab.start_plain(&alpha);
            lab.start_plain(&beta);
        } else {
            lab.start_plain(&beta);
            lab.start_plain(&alpha);
        }
        wait("both online again", 60, || (online(&lab, &alpha) && online(&lab, &beta)).then_some(()));
        std::fs::write(&src, format!("round {round}")).unwrap();
        let out = Lab::run(&alpha, &["send", "--to", "beta", src.to_str().unwrap()]);
        assert!(out.ok(), "round {round}: {}", out.both());
    }
    assert_eq!(read(&beta.downloads.join("again (3).txt")), b"round 2");
    // The trust files were not touched by any of it.
    assert_eq!(trusted(&alpha).devices().len(), 1);
    assert_eq!(trusted(&beta).devices().len(), 1);
    std::thread::sleep(Duration::from_millis(100));
}

/// Anything can connect to the control socket of the account's own folder; whatever it sends, the
/// daemon answers or drops it and goes on, and the socket stays private.
#[test]
fn the_control_socket_survives_garbage_floods_and_slow_clients() {
    if !isolated("the_control_socket_survives_garbage_floods_and_slow_clients") {
        return;
    }
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let (lab, alpha, _beta) = two();
    let socket = alpha.home.join("control.sock");
    let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&socket).unwrap().permissions()) & 0o777;
    assert_eq!(mode & 0o077, 0, "the control socket is reachable by others (mode {mode:o})");
    let ask = |line: &[u8]| -> String {
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        stream.write_all(line).unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
        reply
    };
    for junk in [
        &b"\n"[..],
        b"\0\0\0\0\n",
        b"status extra words that do not belong\n",
        b"STATUS\n",
        b"switch\n",
        b"switch \xff\xfe\n",
        b"clip-send \n",
        b"history show 99999999999999999999999\n",
        b"transfer garbage\n",
        b"settings\r\n",
    ] {
        let reply = ask(junk);
        assert!(!reply.contains("panicked"), "{junk:?} -> {reply}");
    }
    // A megabyte with no newline, then a line of exactly the wrong size.
    let mut big = UnixStream::connect(&socket).unwrap();
    let _ = big.write_all(&vec![b'x'; 1 << 20]);
    drop(big);
    let _ = ask(&vec![b'y'; 5000]);
    // Clients that connect and say nothing, or half a line, many at a time.
    let mut idle: Vec<UnixStream> = (0..150).map(|_| UnixStream::connect(&socket).unwrap()).collect();
    for (index, stream) in idle.iter_mut().enumerate() {
        if index % 2 == 0 {
            let _ = stream.write_all(b"stat");
        }
    }
    // The daemon still answers promptly while they hang around.
    let started = std::time::Instant::now();
    assert!(Lab::ok(&alpha, &["status"]).contains("is running"));
    assert!(started.elapsed().as_secs() < 8, "{:?}", started.elapsed());
    drop(idle);
    std::thread::sleep(Duration::from_millis(300));
    assert!(lab.status_of(&alpha).is_some());
    assert!(!Lab::log(&alpha).contains("panicked"), "{}", Lab::log(&alpha));
}

/// Three computers paired in a mesh: every one sees the other two, `switch next` goes round, and
/// commands that need a choice ask for one.
#[test]
fn three_computers_see_each_other_and_commands_that_need_a_choice_ask_for_one() {
    if !isolated("three_computers_see_each_other_and_commands_that_need_a_choice_ask_for_one") {
        return;
    }
    let mut lab = Lab::new();
    let (alpha, beta, gamma) = (lab.node("alpha"), lab.node("beta"), lab.node("gamma"));
    for node in [&alpha, &beta, &gamma] {
        lab.start_plain(node);
    }
    lab.pair(&alpha, &beta);
    lab.pair(&alpha, &gamma);
    lab.pair(&beta, &gamma);
    wait("a full mesh", 60, || {
        [&alpha, &beta, &gamma]
            .iter()
            .all(|node| lab.status_of(node).is_some_and(|s| s.peers.len() == 2 && s.peers.iter().all(|p| p.online)))
            .then_some(())
    });
    // One side of a computer cannot hold two others: pairing put them on different sides.
    for node in [&alpha, &beta, &gamma] {
        let mut sides: Vec<String> = lab.status(node).peers.iter().map(|p| p.side.to_string()).collect();
        sides.sort();
        sides.dedup();
        assert_eq!(sides.len(), 2, "{} has two computers on one side: {:?}", node.name, lab.status(node).peers);
    }
    // A command that needs to know which one asks, and says how.
    let src = lab.path("f.txt");
    std::fs::write(&src, "x").unwrap();
    let out = Lab::run(&alpha, &["send", src.to_str().unwrap()]);
    assert!(out.failed().contains("several paired computers") && out.stderr.contains("--to"), "{out:?}");
    // `switch next` goes to one computer, then the other, then back here.
    let mut visited = Vec::new();
    for _ in 0..2 {
        Lab::ok(&alpha, &["switch", "next"]);
        wait("control moved", 20, || lab.status(&alpha).controlling.is_some().then_some(()));
        visited.push(lab.status(&alpha).controlling.clone().unwrap());
        // Moving on from a computer that is controlled needs it to have noticed: give it a moment.
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_ne!(visited[0], visited[1], "`switch next` stayed on one computer: {visited:?}");
    Lab::ok(&alpha, &["switch", "next"]);
    wait("back on alpha", 20, || lab.status(&alpha).controlling.is_none().then_some(()));
    // Broadcast reaches both.
    assert!(Lab::ok(&alpha, &["broadcast", "on"]).contains("2 computers"));
    Lab::ok(&alpha, &["broadcast", "off"]);
    // Taking one computer out does not disturb the other.
    Lab::ok(&alpha, &["forget", "gamma"]);
    wait("alpha has one peer", 20, || (lab.status(&alpha).peers.len() == 1).then_some(()));
    assert!(Lab::run(&alpha, &["send", "--to", "beta", src.to_str().unwrap()]).ok());
}
