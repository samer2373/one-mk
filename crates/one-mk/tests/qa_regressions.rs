use std::process::{Command, Output};

fn private_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    home
}

fn run(home: &std::path::Path, args: &[&str]) -> Output {
    let runtime = private_home();
    Command::new(env!("CARGO_BIN_EXE_one-mk"))
        .env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home).env("ONE_MK_HOME", home).env("XDG_RUNTIME_DIR", runtime.path())
        .args(args).output().unwrap()
}

#[test]
fn controls_fail_consistently_without_daemon() {
    let home = private_home();
    for args in [vec!["takeback"], vec!["switch", "next"], vec!["lock-pointer", "on"], vec!["lock-all"], vec!["broadcast", "on"], vec!["clip-get"]] {
        let output = run(home.path(), &args);
        assert!(!output.status.success(), "{args:?} incorrectly succeeded");
        assert!(String::from_utf8_lossy(&output.stderr).contains("one-mk is not running. Start it with `one-mk run`"), "{args:?}: {:?}", output.stderr);
    }
}

#[test]
fn status_does_not_warn_about_expected_update_gate() {
    let home = private_home();
    let output = run(home.path(), &["status"]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("publisher signing key"));
}

#[test]
fn invalid_utf8_settings_names_file() {
    let home = private_home();
    let path = home.path().join("settings.toml");
    std::fs::write(&path, [255]).unwrap();
    let output = run(home.path(), &["notify", "off"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(path.to_str().unwrap()));
}

#[test]
fn history_can_be_enabled_without_daemon_and_unknown_keys_survive() {
    let home = private_home();
    let path = home.path().join("settings.toml");
    std::fs::write(&path, "future = 42\n\"future.key\" = 17\n[clipboard]\nfuture_option = true\n[edge]\n\"future.key\" = 19\n").unwrap();
    for state in ["on", "off"] {
        let output = run(home.path(), &["clipboard", "history", state]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("future = 42") && text.contains("future_option = true"));
        assert!(text.contains("\"future.key\" = 17") && text.contains("\"future.key\" = 19"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("future_option"));
    }
}

#[test]
fn fullscreen_allow_rejects_controls_and_unbounded_names() {
    let home = private_home();
    for name in ["bad\nescape".to_owned(), "x".repeat(100_000)] {
        assert!(!run(home.path(), &["edge", "--fullscreen-allow", &name]).status.success());
    }
}

#[test]
fn missing_send_path_cannot_emit_terminal_controls() {
    let home = private_home();
    let path = home.path().join("missing\u{1b}[2J\u{7}");
    let output = run(home.path(), &["send", path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(!output.stderr.contains(&0x1b) && !output.stderr.contains(&7));
    assert!(String::from_utf8_lossy(&output.stderr).contains("reading file to send"));
}

#[test]
fn invalid_control_targets_are_rejected_before_connecting() {
    let home = private_home();
    for args in [vec!["switch", "bad\nstop"], vec!["clip-send", "--to", "bad\nescape"], vec!["switch", &"x".repeat(100_000)]] {
        let output = run(home.path(), &args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("computer name or id"));
    }
}

#[test]
fn disabling_autoscroll_does_not_tell_user_to_click() {
    let home = private_home();
    let output = run(home.path(), &["autoscroll", "--off"]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Click the toggle"));
    let output = run(home.path(), &["status"]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Autoscroll and gestures:"));
}

#[test]
fn renderer_install_requires_an_explicit_nonempty_file() {
    let home = private_home();
    let output = run(home.path(), &["wallpaper", "extension", "install"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--renderer"));
    let path = home.path().join("empty-renderer");
    std::fs::write(&path, "").unwrap();
    let output = run(home.path(), &["wallpaper", "extension", "install", "--renderer", path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(path.to_str().unwrap()));
}

#[cfg(unix)]
#[test]
fn read_only_settings_folder_names_the_action_and_path() {
    use std::os::unix::fs::PermissionsExt;
    let home = private_home();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let output = run(home.path(), &["notify", "off"]);
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("opening") && error.contains(home.path().to_str().unwrap()), "{error}");
}

struct Process(std::process::Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn process(home: &std::path::Path, args: &[&str], interactive: bool) -> Process {
    use std::process::Stdio;
    Process(Command::new(env!("CARGO_BIN_EXE_one-mk"))
        .env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home).env("ONE_MK_HOME", home).env("XDG_RUNTIME_DIR", home)
        .args(args).stdin(if interactive { Stdio::piped() } else { Stdio::null() })
        .stdout(if interactive { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null()).spawn().unwrap())
}

fn lines(process: &mut Process) -> std::sync::mpsc::Receiver<String> {
    use std::io::BufRead;
    let stdout = process.0.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() { break; }
        }
    });
    receiver
}

fn event(receiver: &std::sync::mpsc::Receiver<String>, word: &str) -> String {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let line = receiver.recv_timeout(end.saturating_duration_since(std::time::Instant::now())).expect("pair event timeout");
        if line.split('\t').next() == Some(word) { return line; }
    }
}

fn daemon_status(home: &std::path::Path) -> Option<one_mk_ctl::status::Status> {
    one_mk_ctl::client::request(home, one_mk_ctl::client::Request::Status, std::time::Duration::from_secs(2)).ok().flatten()
        .and_then(|reply| one_mk_ctl::status::Status::decode(&reply))
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !predicate() {
        assert!(std::time::Instant::now() < end, "daemon state did not converge");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn shortcut_upgrade_notice_is_once_and_piped_logs_have_no_ansi() {
    use std::io::Read;
    use std::process::Stdio;
    let home = private_home();
    std::fs::write(home.path().join("settings.toml"), "settings_version = 2\n[wallpaper]\nversion = 999\n").unwrap();
    assert!(run(home.path(), &["notify", "off"]).status.success());
    std::fs::write(home.path().join("settings.toml"), std::fs::read_to_string(home.path().join("settings.toml")).unwrap().replace("version = 1", "version = 999")).unwrap();
    for first in [true, false] {
        let mut daemon = Process(Command::new(env!("CARGO_BIN_EXE_one-mk"))
            .env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", home.path()).env("ONE_MK_HOME", home.path()).env("XDG_RUNTIME_DIR", home.path())
            .args(["run", "--backend", "null", "--no-discovery", "--no-notify", "--no-clipboard", "--port", "0"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
        wait_until(|| daemon_status(home.path()).is_some());
        assert!(home.path().join(".wave5-shortcuts-noticed").exists());
        daemon.0.kill().unwrap();
        daemon.0.wait().unwrap();
        let mut output = String::new();
        daemon.0.stdout.take().unwrap().read_to_string(&mut output).unwrap();
        assert_eq!(output.contains("New default shortcuts:"), first, "{output}");
        let mut logs = String::new();
        daemon.0.stderr.take().unwrap().read_to_string(&mut logs).unwrap();
        assert!(logs.contains("WARN") && logs.contains("wallpaper"), "expected a real warning: {logs}");
        assert!(!logs.contains('\u{1b}'), "{logs}");
    }
}

#[test]
fn cli_pair_reloads_two_running_daemons_and_keeps_real_ports() {
    use std::io::Write;
    let (home_a, home_b) = (private_home(), private_home());
    let bind_a = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_b = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let (port_a, port_b) = (bind_a.local_addr().unwrap().port(), bind_b.local_addr().unwrap().port());
    drop((bind_a, bind_b));
    let args = |port| vec!["run".to_owned(), "--backend".into(), "null".into(), "--no-discovery".into(), "--no-notify".into(), "--no-clipboard".into(), "--port".into(), port];
    let daemon_a_args = args(port_a.to_string());
    let daemon_b_args = args(port_b.to_string());
    let _daemon_a = process(home_a.path(), &daemon_a_args.iter().map(String::as_str).collect::<Vec<_>>(), false);
    let _daemon_b = process(home_b.path(), &daemon_b_args.iter().map(String::as_str).collect::<Vec<_>>(), false);
    wait_until(|| daemon_status(home_a.path()).is_some() && daemon_status(home_b.path()).is_some());
    let mut pair_a = process(home_a.path(), &["pair", "--machine", "--no-discovery"], true);
    let events_a = lines(&mut pair_a);
    let me = event(&events_a, "me");
    let port = me.split('\t').nth(3).unwrap();
    let mut pair_b = process(home_b.path(), &["pair", "--machine", "--no-discovery", "--connect", &format!("127.0.0.1:{port}")], true);
    let events_b = lines(&mut pair_b);
    event(&events_a, "code");
    event(&events_b, "code");
    pair_a.0.stdin.as_mut().unwrap().write_all(b"y\n").unwrap();
    pair_b.0.stdin.as_mut().unwrap().write_all(b"y\n").unwrap();
    event(&events_a, "paired");
    event(&events_b, "paired");
    for (home, port) in [(home_a.path(), port_b), (home_b.path(), port_a)] {
        let store = one_mk_net::TrustStore::load(home).unwrap();
        assert!(store.devices()[0].addrs.iter().any(|address| address.ends_with(&format!(":{port}"))), "wrong service port: {:?}", store.devices()[0].addrs);
    }
    wait_until(|| [home_a.path(), home_b.path()].iter().all(|home| daemon_status(home).is_some_and(|status| status.peers.len() == 1 && status.peers[0].online)));
}
