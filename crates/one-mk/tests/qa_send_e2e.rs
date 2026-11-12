//! `one-mk send` between two real daemons: small, empty and large files, folders,
//! name collisions, odd names, cancelling, the receiver dying mid-transfer and
//! resuming, and every way a send can be refused. Receivers' files are compared
//! byte for byte.
#![cfg(target_os = "linux")]

mod qa_common;
use qa_common::*;
use std::path::{Path, PathBuf};

fn two() -> (Lab, Node, Node) {
    let mut lab = Lab::new();
    let (alpha, beta) = (lab.node("alpha"), lab.node("beta"));
    lab.start_plain(&alpha);
    lab.start_plain(&beta);
    lab.pair(&alpha, &beta);
    (lab, alpha, beta)
}

fn source(lab: &Lab) -> PathBuf {
    let dir = lab.path("src");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn files_under(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if rel.starts_with(".one-mk-partial") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(&path, root, out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn send(from: &Node, to: &str, paths: &[&Path]) -> Out {
    let mut args = vec!["send", "--to", to, "--"];
    let paths: Vec<String> = paths.iter().map(|p| p.to_str().unwrap().to_owned()).collect();
    args.extend(paths.iter().map(String::as_str));
    Lab::run(from, &args)
}

#[test]
fn small_empty_and_binary_files_arrive_intact_and_repeats_are_numbered() {
    if !isolated("small_empty_and_binary_files_arrive_intact_and_repeats_are_numbered") {
        return;
    }
    let (lab, alpha, beta) = two();
    let src = source(&lab);
    let small = src.join("small.txt");
    let zero = src.join("zero.bin");
    let binary = src.join("binary.dat");
    std::fs::write(&small, "hello, other computer\n").unwrap();
    std::fs::write(&zero, b"").unwrap();
    std::fs::write(&binary, noise(70_000, 7)).unwrap();

    let out = send(&alpha, "beta", &[&small, &zero, &binary]);
    let said = out.both();
    assert!(out.ok() && said.contains("Sent 3 files"), "{said}");
    assert_eq!(read(&beta.downloads.join("small.txt")), b"hello, other computer\n");
    assert_eq!(std::fs::metadata(beta.downloads.join("zero.bin")).unwrap().len(), 0);
    assert_eq!(read(&beta.downloads.join("binary.dat")), noise(70_000, 7));

    // Again, and again with different content under the same name: nothing is overwritten.
    std::fs::write(&small, "second version\n").unwrap();
    assert!(send(&alpha, "beta", &[&small]).ok());
    std::fs::write(&small, "third version\n").unwrap();
    assert!(send(&alpha, "beta", &[&small]).ok());
    assert!(send(&alpha, "beta", &[&zero]).ok());
    assert_eq!(read(&beta.downloads.join("small.txt")), b"hello, other computer\n");
    assert_eq!(read(&beta.downloads.join("small (1).txt")), b"second version\n");
    assert_eq!(read(&beta.downloads.join("small (2).txt")), b"third version\n");
    assert!(beta.downloads.join("zero (1).bin").exists());

    // The other way, and the sending computer's own downloads stay untouched.
    let back = src.join("from-beta.txt");
    std::fs::write(&back, "reply").unwrap();
    assert!(send(&beta, "alpha", &[&back]).ok());
    assert_eq!(read(&alpha.downloads.join("from-beta.txt")), b"reply");
    assert!(files_under(&alpha.downloads).iter().all(|f| f == "from-beta.txt"));
    // `status` and `partials` leave nothing unfinished behind.
    assert!(Lab::ok(&beta, &["partials"]).contains("No unfinished"));
    assert!(lab.status(&beta).transfers.is_empty() && lab.status(&alpha).transfers.is_empty());
}

#[test]
fn folders_arrive_with_their_structure_and_odd_names_are_kept_or_cleaned_never_escaped() {
    if !isolated("folders_arrive_with_their_structure_and_odd_names_are_kept_or_cleaned_never_escaped") {
        return;
    }
    let (lab, alpha, beta) = two();
    let src = source(&lab);
    let tree = src.join("tree");
    std::fs::create_dir_all(tree.join("a/b/c")).unwrap();
    std::fs::create_dir_all(tree.join("empty")).unwrap();
    std::fs::write(tree.join("top.txt"), "t").unwrap();
    std::fs::write(tree.join("a/mid.txt"), "m").unwrap();
    std::fs::write(tree.join("a/b/c/leaf.txt"), "l").unwrap();
    std::os::unix::fs::symlink("/etc/passwd", tree.join("a/link")).unwrap();
    std::os::unix::fs::symlink(&tree, tree.join("a/b/loop")).unwrap();
    let out = send(&alpha, "beta", &[&tree]);
    assert!(out.ok(), "{}", out.both());
    assert_eq!(files_under(&beta.downloads), ["tree/a/b/c/leaf.txt", "tree/a/mid.txt", "tree/top.txt"]);
    assert!(
        out.stderr.contains("Skipped") && out.stderr.contains("link"),
        "links are skipped silently: {}",
        out.stderr
    );
    assert!(!files_under(&beta.downloads).iter().any(|f| f.contains("passwd")));

    // File names of every kind.
    let odd = src.join("odd");
    std::fs::create_dir_all(&odd).unwrap();
    let names = [
        "with space.txt",
        "-dash.txt",
        "--to=evil",
        "ünï cødé ☃ 🎉.txt",
        "trailing dot.",
        ".hidden",
        "CON",
        "nul.txt",
        "a:b*c?.txt",
        "quote\"s'.txt",
        "back\\slash.txt",
        "$(touch pwned).txt",
        "`touch pwned`.txt",
        "semi;colon && echo.txt",
        &"n".repeat(255),
        &"é".repeat(127),
    ];
    for name in names {
        std::fs::write(odd.join(name), name).unwrap();
    }
    let paths: Vec<PathBuf> = names.iter().map(|name| odd.join(name)).collect();
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let out = send(&alpha, "beta", &refs);
    assert!(out.ok(), "{}", out.both());
    let got = files_under(&beta.downloads);
    for name in [
        "with space.txt",
        "-dash.txt",
        "--to=evil",
        "ünï cødé ☃ 🎉.txt",
        ".hidden",
        "nul_.txt",
        "CON_",
        "$(touch pwned).txt",
        "`touch pwned`.txt",
    ] {
        assert!(got.iter().any(|f| f == name), "{name:?} missing from {got:?}");
    }
    assert!(got.iter().any(|f| f.len() == 255), "{got:?}");
    // Nothing is created outside the downloads folder, and nothing was executed.
    assert!(!beta.downloads.join("pwned").exists());
    assert!(std::fs::read_dir(&beta.home).unwrap().flatten().all(|e| e.file_name() != "pwned"));

    // Names that cannot be sent (a newline, a tab) are left out with a message and the rest goes.
    let bad = odd.join("new\nline.txt");
    std::fs::write(&bad, "x").unwrap();
    let fine = odd.join("fine-after-bad.txt");
    std::fs::write(&fine, "ok").unwrap();
    let out = send(&alpha, "beta", &[&bad, &fine]);
    assert!(out.ok(), "{}", out.both());
    assert!(out.stderr.contains("Skipped") && out.stderr.contains("new\\nline"), "{}", out.stderr);
    assert_eq!(read(&beta.downloads.join("fine-after-bad.txt")), b"ok");
    // Only bad names: a clear failure.
    assert!(send(&alpha, "beta", &[&bad]).failed().contains("nothing to send"));

    // A folder and a file with the same name do not clobber each other.
    std::fs::write(src.join("clash"), "i am a file").unwrap();
    assert!(send(&alpha, "beta", &[&src.join("clash")]).ok());
    let dir = src.join("sub/clash");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("inside.txt"), "i am inside").unwrap();
    let out = send(&alpha, "beta", &[&dir]);
    assert!(out.ok(), "a folder named like a file already received failed: {}", out.both());
    assert_eq!(read(&beta.downloads.join("clash")), b"i am a file");
    let got = files_under(&beta.downloads);
    assert!(got.iter().any(|f| f.ends_with("inside.txt")), "{got:?}");
}

#[test]
fn a_large_file_arrives_bit_for_bit_and_status_shows_it_while_it_moves() {
    if !isolated("a_large_file_arrives_bit_for_bit_and_status_shows_it_while_it_moves") {
        return;
    }
    let (lab, alpha, beta) = two();
    let src = source(&lab);
    let big = src.join("big.bin");
    {
        use std::io::{Read, Write};
        let mut urandom = std::fs::File::open("/dev/urandom").unwrap();
        let mut out = std::fs::File::create(&big).unwrap();
        let mut block = vec![0u8; 1 << 20];
        for _ in 0..200 {
            urandom.read_exact(&mut block).unwrap();
            out.write_all(&block).unwrap();
        }
        // An odd tail, so the size is not a multiple of any block size.
        out.write_all(&block[..12_345]).unwrap();
    }
    let proc = Proc::spawn(Lab::command(&alpha, &["send", "--to", "beta", big.to_str().unwrap()]));
    let mut saw_progress = false;
    for _ in 0..400 {
        if lab.status(&beta).transfers.iter().any(|t| !t.sending && t.done > 0 && t.total > t.done) {
            saw_progress = true;
            assert!(lab.status(&alpha).transfers.iter().all(|t| t.sending));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let out = proc.finish(120);
    assert!(out.ok(), "{}", out.both());
    assert!(saw_progress, "status never showed the transfer in progress");
    assert_eq!(std::fs::metadata(beta.downloads.join("big.bin")).unwrap().len(), 200 * (1 << 20) + 12_345);
    assert!(read(&beta.downloads.join("big.bin")) == read(&big), "the received file differs from the sent one");
    wait("transfer rows to clear", 20, || {
        (lab.status(&beta).transfers.is_empty() && lab.status(&alpha).transfers.is_empty()).then_some(())
    });
}

#[test]
fn cancelling_a_send_keeps_what_arrived_and_the_next_send_resumes_it() {
    if !isolated("cancelling_a_send_keeps_what_arrived_and_the_next_send_resumes_it") {
        return;
    }
    let (lab, alpha, beta) = two();
    let src = source(&lab);
    let big = src.join("huge.bin");
    {
        // A sparse 1 GiB file with a marker in the middle: nothing to write here, plenty to move.
        use std::io::{Seek, Write};
        let mut file = std::fs::File::create(&big).unwrap();
        file.set_len(1 << 30).unwrap();
        file.seek(std::io::SeekFrom::Start(512 << 20)).unwrap();
        file.write_all(b"marker in the middle").unwrap();
    }
    let proc = Proc::spawn(Lab::command(&alpha, &["send", "--to", "beta", big.to_str().unwrap()]));
    wait("bytes arrive", 60, || lab.status(&beta).transfers.iter().any(|t| !t.sending && t.done > 0).then_some(()));
    proc.signal(libc::SIGINT);
    let out = proc.finish(30);
    assert_eq!(out.code, Some(1), "{}", out.both());
    assert!(out.stderr.contains("cancelled; what arrived is kept"), "{}", out.stderr);
    // The receiver learned of it quickly (not by waiting for a timeout) and kept a partial.
    wait("receiver to drop the transfer", 15, || lab.status(&beta).transfers.is_empty().then_some(()));
    assert!(!beta.downloads.join("huge.bin").exists(), "a half file was left under the real name");
    let partials = Lab::ok(&beta, &["partials"]);
    assert!(partials.contains("1 unfinished received file"), "{partials}");

    // Sending again resumes from where it stopped and finishes the whole file.
    let out = send(&alpha, "beta", &[&big]);
    assert!(out.ok() && out.stderr.contains("resumed huge.bin from"), "{}", out.both());
    let received = beta.downloads.join("huge.bin");
    assert_eq!(std::fs::metadata(&received).unwrap().len(), 1 << 30);
    {
        use std::io::{Read, Seek};
        let mut file = std::fs::File::open(&received).unwrap();
        file.seek(std::io::SeekFrom::Start(512 << 20)).unwrap();
        let mut marker = [0u8; 20];
        file.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"marker in the middle");
    }
    assert!(Lab::ok(&beta, &["partials"]).contains("No unfinished"));
}

#[test]
fn the_receiver_dying_mid_transfer_fails_the_send_cleanly_and_a_later_send_finishes_it() {
    if !isolated("the_receiver_dying_mid_transfer_fails_the_send_cleanly_and_a_later_send_finishes_it") {
        return;
    }
    let (mut lab, alpha, beta) = two();
    let src = source(&lab);
    let big = src.join("resume-me.bin");
    {
        // 1 GiB, sparse, with a marker near the start, in the middle and at the very end.
        use std::io::{Seek, Write};
        let mut file = std::fs::File::create(&big).unwrap();
        file.set_len(1 << 30).unwrap();
        for (offset, text) in
            [(3u64 << 20, "near the start"), (512 << 20, "in the middle"), ((1 << 30) - 12, "the very end")]
        {
            file.seek(std::io::SeekFrom::Start(offset)).unwrap();
            file.write_all(text.as_bytes()).unwrap();
        }
    }
    let proc = Proc::spawn(Lab::command(&alpha, &["send", "--to", "beta", big.to_str().unwrap()]));
    wait("bytes arrive", 60, || {
        lab.status(&beta).transfers.iter().any(|t| !t.sending && t.done > 1 << 20).then_some(())
    });
    lab.kill9(&beta);
    let started = std::time::Instant::now();
    let out = proc.finish(60);
    assert!(started.elapsed().as_secs() <= 30, "the sender took {:?} to notice the receiver died", started.elapsed());
    assert_eq!(out.code, Some(1), "{}", out.both());
    assert!(out.stderr.contains("not sent") && out.stderr.contains("send again to resume"), "{}", out.stderr);
    assert!(!beta.downloads.join("resume-me.bin").exists());

    // While it is down, a new send says so and gives up in a bounded time.
    let started = std::time::Instant::now();
    let out = send(&alpha, "beta", &[&big]);
    assert!(out.failed().contains("could not reach beta") && out.stderr.contains("one-mk run"), "{}", out.both());
    assert!(started.elapsed().as_secs() <= 20, "{:?}", started.elapsed());

    // Restart the receiver: the partial survived the kill -9 and the send resumes from it.
    lab.start_plain(&beta);
    wait("alpha sees beta", 40, || lab.status(&alpha).peers.iter().all(|p| p.online).then_some(()));
    let out = send(&alpha, "beta", &[&big]);
    assert!(out.ok(), "{}", out.both());
    assert!(out.stderr.contains("resumed resume-me.bin from"), "the partial was lost: {}", out.stderr);
    {
        use std::io::{Read, Seek};
        let mut file = std::fs::File::open(beta.downloads.join("resume-me.bin")).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 1 << 30);
        for (offset, text) in
            [(3u64 << 20, "near the start"), (512 << 20, "in the middle"), ((1 << 30) - 12, "the very end")]
        {
            file.seek(std::io::SeekFrom::Start(offset)).unwrap();
            let mut got = vec![0u8; text.len()];
            file.read_exact(&mut got).unwrap();
            assert_eq!(got, text.as_bytes(), "the resumed file is wrong at {offset}");
        }
    }
}

#[test]
fn every_way_a_send_can_be_refused_says_what_to_do() {
    if !isolated("every_way_a_send_can_be_refused_says_what_to_do") {
        return;
    }
    let (lab, alpha, beta) = two();
    let src = source(&lab);
    let file = src.join("f.txt");
    std::fs::write(&file, "x").unwrap();

    // No such path, an unreadable file, a folder of only nothing.
    let missing = src.join("missing.txt");
    assert!(send(&alpha, "beta", &[&missing]).failed().contains("missing.txt"));
    let secret = src.join("secret.txt");
    std::fs::write(&secret, "x").unwrap();
    std::fs::set_permissions(&secret, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    // Inside a private namespace the test may hold the capability that overrides file permissions.
    let enforced = std::fs::File::open(&secret).is_err();
    if enforced {
        let out = send(&alpha, "beta", &[&secret]);
        out.failed();
        assert!(!beta.downloads.join("secret.txt").exists());
    }
    std::fs::set_permissions(&secret, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    std::fs::create_dir_all(src.join("hollow")).unwrap();
    assert!(send(&alpha, "beta", &[&src.join("hollow")]).failed().contains("nothing to send"));

    // Unknown, ambiguous and missing recipients.
    assert!(send(&alpha, "nobody", &[&file]).failed().contains("no paired computer called"));
    let ambiguous = Lab::run(&alpha, &["send", file.to_str().unwrap(), "--to", ""]);
    ambiguous.failed();
    let lonely = lab.node("lonely");
    assert!(Lab::run(&lonely, &["send", file.to_str().unwrap()]).failed().contains("one-mk pair"));

    // A receiver whose downloads folder cannot be written says why and keeps nothing.
    if enforced {
        std::fs::set_permissions(&beta.downloads, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();
        let out = send(&alpha, "beta", &[&file]);
        std::fs::set_permissions(&beta.downloads, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        out.failed();
        assert!(out.stderr.contains("not sent"), "{}", out.stderr);
        assert!(files_under(&beta.downloads).is_empty());
    }
    assert!(send(&alpha, "beta", &[&file]).ok(), "the receiver did not recover once the folder was writable");

    // The receiver forgot the sender: said once, with what to do; sending again will not help.
    Lab::ok(&beta, &["forget", "alpha"]);
    let out = send(&alpha, "beta", &[&file]);
    let error = out.failed();
    assert!(error.contains("removed this computer") && error.contains("one-mk forget"), "{error}");
    assert!(!error.contains("panicked"));
}
