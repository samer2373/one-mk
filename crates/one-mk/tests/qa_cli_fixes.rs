//! End-to-end regressions for what an independent QA found in the command
//! line and the daemon (real binaries, `--backend null`, private homes):
//! computers that share a name, wording that was wrong, read-only commands
//! and a broken settings file, hooks refused for permissions, `partials`
//! without a daemon, and the files a send leaves out.
#![cfg(target_os = "linux")]

use one_mk_net::{Device, Identity, TrustStore};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Lab {
    root: tempfile::TempDir,
    children: Vec<Child>,
}

impl Lab {
    fn new() -> Self {
        Self { root: tempfile::Builder::new().prefix("one-mk-qa-fixes-").tempdir().unwrap(), children: Vec::new() }
    }

    /// A private settings folder with an identity, as a computer would have.
    fn home(&self, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let home = self.root.path().join(name);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        Identity::load_or_create(&home).unwrap();
        home
    }

    fn command(home: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", home)
            .env("ONE_MK_HOME", home)
            .env("XDG_RUNTIME_DIR", home)
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/dead");
        command
    }

    /// Runs a command to the end: (succeeded, stdout, stderr).
    fn run(home: &Path, args: &[&str]) -> (bool, String, String) {
        let output: Output = Self::command(home).args(args).stdin(Stdio::null()).output().unwrap();
        (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
    }

    #[track_caller]
    fn ok(home: &Path, args: &[&str]) -> String {
        let (ok, out, err) = Self::run(home, args);
        assert!(ok, "{args:?} failed: {err}{out}");
        out
    }

    /// Starts a daemon (null backend, nothing shared with the desktop) and
    /// returns its UDP port.
    fn daemon(&mut self, home: &Path) -> u16 {
        let log = std::fs::File::create(home.join("daemon.log")).unwrap();
        let child = Self::command(home)
            .args(["run", "--backend", "null", "--no-discovery", "--no-notify", "--no-clipboard", "--port", "0"])
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        self.children.push(child);
        let index = self.children.len() - 1;
        wait(|| {
            assert!(self.children[index].try_wait().unwrap().is_none(), "{}", std::fs::read_to_string(home.join("daemon.log")).unwrap());
            let reply = one_mk_ctl::client::request(home, one_mk_ctl::client::Request::Status, Duration::from_secs(1)).ok().flatten()?;
            one_mk_ctl::status::Status::decode(&reply).map(|status| status.port)
        })
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[track_caller]
fn wait<T>(mut condition: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = condition() {
            return value;
        }
        assert!(Instant::now() < deadline, "did not settle within 20 seconds");
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn fingerprint(home: &Path) -> String {
    Identity::load_or_create(home).unwrap().fingerprint.to_string()
}

fn short(home: &Path) -> String {
    Identity::load_or_create(home).unwrap().fingerprint.short()
}

fn pair(home: &Path, other: &Path, name: &str, side: &str, port: Option<u16>) {
    let mut trust = TrustStore::load(home).unwrap();
    trust.upsert(Device {
        name: name.into(),
        fingerprint: fingerprint(other),
        side: side.into(),
        addrs: port.map(|port| format!("127.0.0.1:{port}")).into_iter().collect(),
        layout_version: 1_700_000_000_000,
        placed_by: one_mk_net::PlacedBy::User,
        ..Device::default()
    });
    trust.save().unwrap();
}

/// Two paired computers that call themselves the same, both running and
/// both placed on this computer's left.
fn twins(lab: &mut Lab) -> (PathBuf, PathBuf, PathBuf) {
    let (me, first, second) = (lab.home("me"), lab.home("first"), lab.home("second"));
    pair(&first, &me, "Me", "right", None);
    pair(&second, &me, "Me", "right", None);
    let (first_port, second_port) = (lab.daemon(&first), lab.daemon(&second));
    pair(&me, &first, "Twin", "left", Some(first_port));
    pair(&me, &second, "Twin", "left", Some(second_port));
    lab.daemon(&me);
    wait(|| {
        let status = Lab::ok(&me, &["status"]);
        (status.matches("online").count() >= 2).then_some(())
    });
    (me, first, second)
}

#[test]
fn computers_with_the_same_name_are_told_apart_and_every_hint_works() {
    let mut lab = Lab::new();
    let (me, first, second) = twins(&mut lab);
    let (id_first, id_second) = (short(&first), short(&second));

    // Everything that lists them shows each by name and id.
    let status = wait(|| {
        let status = Lab::ok(&me, &["status"]);
        status.contains("edges overlap").then_some(status)
    });
    for id in [&id_first, &id_second] {
        assert!(status.contains(&format!("Twin ({id})")), "{status}");
    }
    // The advice names the one to move by its id, and that command works.
    let command = status.split("move the other with `one-mk layout ").nth(1).and_then(|rest| rest.split(' ').next()).expect("a layout hint");
    assert!(command == id_first || command == id_second, "{command} in {status}");
    Lab::ok(&me, &["layout", command, "top"]);

    // A name that fits two computers says so, with the ids to use instead.
    let file = lab.root.path().join("hello.txt");
    std::fs::write(&file, "hi").unwrap();
    let file = file.to_str().unwrap().to_owned();
    for args in [vec!["switch", "Twin"], vec!["layout", "Twin", "left"], vec!["send", "--to", "Twin", &file], vec!["forget", "Twin"]] {
        let (ok, _, err) = Lab::run(&me, &args);
        assert!(!ok, "{args:?}");
        assert!(err.contains("2 paired computers are called") && err.contains(&id_first) && err.contains(&id_second), "{args:?}: {err}");
    }
    let devices = Lab::ok(&me, &["devices"]);
    assert!(devices.contains(&id_first) && devices.contains(&id_second), "{devices}");
    let hotkeys = Lab::ok(&me, &["hotkeys"]);
    assert!(hotkeys.contains(&format!("Twin ({id_first})")) && hotkeys.contains(&format!("Twin ({id_second})")), "{hotkeys}");
    let broadcast = Lab::ok(&me, &["broadcast", "on"]);
    assert!(broadcast.contains(&format!("Twin ({id_first})")) && broadcast.contains(&format!("Twin ({id_second})")), "{broadcast}");
    Lab::ok(&me, &["broadcast", "off"]);
    let profiles = {
        Lab::ok(&me, &["profile", "save", "both"]);
        Lab::ok(&me, &["profile", "list"])
    };
    assert!(profiles.contains(&id_first) && profiles.contains(&id_second), "{profiles}");
    // The id always works.
    assert!(Lab::ok(&me, &["switch", &id_first]).contains("going to"), "an id names one computer");
    Lab::ok(&me, &["switch", "here"]);

    // A name of your own tells them apart; the other computer keeps its own.
    let renamed = Lab::ok(&me, &["rename", &id_first, "Office", "PC"]);
    assert!(renamed.contains("\"Office PC\"") && renamed.contains("still calls itself"), "{renamed}");
    let status = Lab::ok(&me, &["status"]);
    assert!(status.contains("Office PC") && !status.contains(&format!("Twin ({id_second})")), "the other is a plain Twin now: {status}");
    assert!(Lab::ok(&me, &["switch", "Office PC"]).contains("Office PC"));
    Lab::ok(&me, &["switch", "here"]);
    assert_eq!(TrustStore::load(&first).unwrap().devices()[0].name, "Me", "the other computer's own name is untouched");
    assert_eq!(TrustStore::load(&me).unwrap().find(&id_first).unwrap().name, "Twin", "the name it gave itself is kept");
    // A name another computer already has is refused; `--reset` goes back.
    let (ok, _, err) = Lab::run(&me, &["rename", &id_second, "Office PC"]);
    assert!(!ok && err.contains("already shown as"), "{err}");
    Lab::ok(&me, &["rename", "Office PC", "--reset"]);
    assert!(Lab::ok(&me, &["status"]).contains(&format!("Twin ({id_first})")));
}

#[test]
fn typing_reports_what_is_true_after_the_change() {
    let mut lab = Lab::new();
    let me = lab.home("me");
    lab.daemon(&me);
    Lab::ok(&me, &["typing", "--keys", "auto"]);
    let after = Lab::ok(&me, &["typing", "--keys", "physical"]);
    let right_now = after.lines().find(|line| line.starts_with("Right now:")).expect("a Right now line");
    assert!(right_now.contains("physical") && !right_now.contains("auto"), "{after}");
}

#[test]
fn lock_pointer_alone_shows_the_state() {
    let mut lab = Lab::new();
    let me = lab.home("me");
    assert!(Lab::ok(&me, &["lock-pointer"]).contains("not running, so the pointer is not locked"));
    lab.daemon(&me);
    assert!(Lab::ok(&me, &["lock-pointer"]).contains("not locked"));
    Lab::ok(&me, &["lock-pointer", "on"]);
    assert!(Lab::ok(&me, &["lock-pointer"]).contains("is locked to this computer"));
    Lab::ok(&me, &["lock-pointer", "off"]);
    assert!(Lab::ok(&me, &["lock-pointer"]).contains("not locked"));
}

#[test]
fn partials_work_without_a_daemon_and_ask_the_daemon_when_there_is_one() {
    let mut lab = Lab::new();
    let me = lab.home("me");
    let downloads = lab.root.path().join("received");
    std::fs::create_dir_all(&downloads).unwrap();
    let downloads = downloads.to_string_lossy().into_owned();
    let shown = Lab::ok(&me, &["partials", "--downloads", &downloads]);
    assert!(shown.contains("not running") && shown.contains("looking in") && shown.contains("No unfinished received files"), "{shown}");
    assert!(Lab::ok(&me, &["partials", "--clear", "--downloads", &downloads]).contains("Deleted 0 unfinished received files"));
    lab.daemon(&me);
    let shown = Lab::ok(&me, &["partials"]);
    assert!(!shown.contains("looking in"), "a running one-mk answers for itself: {shown}");
}

#[test]
fn hooks_refused_for_permissions_still_list_what_is_set() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new();
    let me = lab.home("me");
    Lab::ok(&me, &["hooks", "--on-enter", "/bin/true"]);
    std::fs::set_permissions(&me, std::fs::Permissions::from_mode(0o770)).unwrap();
    let shown = Lab::ok(&me, &["hooks"]);
    assert!(shown.contains("REFUSED") && shown.contains("on_enter: /bin/true"), "the list is shown with the refusal: {shown}");
    let set = Lab::ok(&me, &["hooks", "--on-leave", "/bin/true"]);
    assert!(set.contains("REFUSED") && set.contains("on_leave: /bin/true"), "{set}");
    assert!(!set.contains("Applied right away"), "nothing runs, so it must not say it applies: {set}");
    assert!(set.contains("do not run until"), "{set}");
}

#[test]
fn a_settings_file_that_is_partly_or_wholly_bad_keeps_read_only_commands_working() {
    let lab = Lab::new();
    let me = lab.home("me");
    let file = me.join("settings.toml");
    // Partly bad: one value of the wrong type.
    std::fs::write(&file, "[edge]\ncorner = \"x\"\n[clipboard]\ntext = false\n").unwrap();
    for args in [vec!["status"], vec!["clipboard"], vec!["edge"], vec!["keymap", "list"], vec!["broadcast"], vec!["hotkeys"], vec!["power"], vec!["hooks"], vec!["gaming"], vec!["lock-sync"], vec!["pointer-overlay"], vec!["notify"]] {
        let (ok, out, err) = Lab::run(&me, &args);
        assert!(ok, "{args:?} must read what it can: {err}{out}");
        assert!(err.contains("settings.toml line 1: [edge] is ignored"), "{args:?} says what it left out: {err}");
    }
    // Not TOML at all: `status` still prints the whole report.
    std::fs::write(&file, "this is = = not toml\n").unwrap();
    let (ok, out, err) = Lab::run(&me, &["status"]);
    assert!(ok, "{err}");
    assert!(out.contains("one-mk is not running") && out.contains("settings.toml can't be read") && out.contains("Clipboard:"), "{out}");
}

#[test]
fn wording_and_numbers_are_reported_as_what_they_are() {
    let lab = Lab::new();
    let me = lab.home("me");
    let (ok, _, err) = Lab::run(&me, &["edge", "--dwell", "-5"]);
    assert!(!ok && err.contains("negative") && !err.contains("unexpected argument"), "{err}");
    let (ok, _, err) = Lab::run(&me, &["edge", "--corner", "99999"]);
    assert!(!ok && err.contains("at most"), "{err}");
    for (value, shown) in [("nan", "NaN"), ("inf", "infinity"), ("1e300", "1e300")] {
        let (ok, _, err) = Lab::run(&me, &["pointer", "--speed", value]);
        assert!(!ok && err.contains(&format!("--speed {shown}: say a number")), "{err}");
    }
    // The spelling of a layer is kept in what is said about it.
    Lab::ok(&me, &["keymap", "add", "a", "b", "--layer", "L1"]);
    let (_, out, _) = Lab::run(&me, &["keymap", "check"]);
    assert!(out.contains("\"L1\"") && !out.contains("\"l1\""), "{out}");
}

#[test]
fn keys_that_cannot_work_are_refused_or_flagged() {
    let lab = Lab::new();
    let me = lab.home("me");
    let (ok, _, err) = Lab::run(&me, &["keymap", "add", "esc", "ctrl+alt+shift+esc"]);
    assert!(!ok && err.contains("emergency chord"), "{err}");
    Lab::ok(&me, &["keymap", "add", "esc", "ctrl+shift+esc"]);
    let added = Lab::ok(&me, &["keymap", "add", "f15", "switch nobody"]);
    assert!(added.contains("no paired computer is called \"nobody\""), "{added}");
    let (ok, out, _) = Lab::run(&me, &["keymap", "check"]);
    assert!(!ok && out.contains("`switch nobody` names no paired computer"), "{out}");
}

#[test]
fn the_new_shortcut_banner_is_for_upgrades_not_new_users() {
    use std::io::Read;
    let lab = Lab::new();
    let me = lab.home("me");
    std::fs::write(me.join("settings.toml"), "[edge]\ncorner = 5\n").unwrap();
    let mut child = Lab::command(&me)
        .args(["run", "--backend", "null", "--no-discovery", "--no-notify", "--no-clipboard", "--port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait(|| one_mk_ctl::client::request(&me, one_mk_ctl::client::Request::Status, Duration::from_secs(1)).ok().flatten());
    child.kill().unwrap();
    child.wait().unwrap();
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    assert!(!out.contains("New default shortcuts"), "a hand-written file with no settings_version is a new user's: {out}");
}

#[test]
fn join_and_invite_say_what_is_wrong() {
    let lab = Lab::new();
    let me = lab.home("me");
    let (ok, _, err) = Lab::run(&me, &["invite", "--listen", "127.0.0.1:5555"]);
    assert!(!ok && err.contains("127.0.0.1:5555") && err.contains("loopback") && err.contains("192.168"), "{err}");
    // A link fed through a pipe needs no prompt to answer.
    let (ok, _, err) = Lab::run(&me, &["join", "--yes"]);
    assert!(!ok && !err.contains("Paste the invitation link"), "{err}");
}

#[test]
fn pair_without_discovery_says_how_to_connect() {
    let lab = Lab::new();
    let me = lab.home("me");
    let log = lab.root.path().join("pair.log");
    let mut child = Lab::command(&me)
        .args(["pair", "--no-discovery"])
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&log).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let text = wait(|| std::fs::read_to_string(&log).ok().filter(|text| text.contains("Discovery is off")));
    let _ = child.kill();
    let _ = child.wait();
    assert!(text.contains("one-mk pair --connect <this computer's IP>:") && !text.contains("Looking for another computer"), "{text}");
}

#[test]
fn a_send_to_a_computer_that_forgot_this_one_says_so() {
    let mut lab = Lab::new();
    let (sender, receiver) = (lab.home("sender"), lab.home("receiver"));
    let port = lab.daemon(&receiver); // it has never paired with the sender
    pair(&sender, &receiver, "Desk", "right", Some(port));
    let file = lab.root.path().join("hello.txt");
    std::fs::write(&file, "hi").unwrap();
    let (ok, _, err) = Lab::run(&sender, &["send", "--to", "Desk", file.to_str().unwrap()]);
    assert!(!ok);
    assert!(err.contains("removed this computer from its paired computers") && err.contains("one-mk pair"), "{err}");
    assert!(!err.contains("send again to resume") && !err.contains("Sent 0 files"), "{err}");
}

#[test]
fn a_pipe_named_on_the_command_line_is_skipped_and_reported() {
    let lab = Lab::new();
    let me = lab.home("me");
    let pipe = lab.root.path().join("pipe");
    let name = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (ok, _, err) = Lab::run(&me, &["send", pipe.to_str().unwrap()]);
    assert!(!ok && err.contains("pipe (a pipe, socket or device)") && err.contains("nothing to send"), "{err}");
}
