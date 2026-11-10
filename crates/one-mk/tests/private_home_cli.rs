//! Everything the daemon and the command line create in the settings folder
//! is private (`0600` files, `0700` folders) from the first byte, even for a
//! person whose umask is Ubuntu's `002`; and a file an older version made
//! with that umask is tightened when the program starts.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn private(path: &Path) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("one-mk-private-home-").tempdir_in(path).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// `one-mk` run under umask 002, with a clean environment and `home` as its
/// settings folder.
fn one_mk(root: &Path, home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "umask 002; exec \"$0\" \"$@\"", env!("CARGO_BIN_EXE_one-mk"), "--home"]).arg(home).args(args)
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", root).env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data")).env("XDG_RUNTIME_DIR", root.join("run")).env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", root.join("no-bus").display()));
    command
}

/// Every file and folder below `home` that group or others can reach.
fn loose(home: &Path) -> Vec<String> {
    fn walk(path: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(path).unwrap().flatten() {
            let metadata = entry.metadata().unwrap();
            if metadata.permissions().mode() & 0o077 != 0 && !metadata.file_type().is_symlink() {
                found.push(format!("{} {:o}", entry.path().display(), metadata.permissions().mode() & 0o777));
            }
            if metadata.is_dir() { walk(&entry.path(), found); }
        }
    }
    let mut found = Vec::new();
    walk(home, &mut found);
    found
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

#[test]
fn what_the_daemon_and_the_cli_create_is_private_under_a_group_writable_umask() {
    let sandbox = tempfile::Builder::new().prefix("one-mk-private-home-").tempdir().unwrap();
    for folder in ["config", "data", "run"] {
        std::fs::create_dir(sandbox.path().join(folder)).unwrap();
        std::fs::set_permissions(sandbox.path().join(folder), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let holder = private(sandbox.path());
    let home = holder.path().join("home");
    let _daemon = Daemon(one_mk(sandbox.path(), &home, &["run", "--backend", "null", "--no-discovery", "--no-clipboard", "--no-notify", "--port", "0"])
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let mut ready = false;
    for _ in 0..200 {
        if one_mk_ctl::client::request(&home, one_mk_ctl::client::Request::Status, Duration::from_secs(1)).ok().flatten().is_some() { ready = true; break; }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(ready, "the daemon did not start");
    for args in [["status"].as_slice(), ["wallpaper", "status"].as_slice(), ["devices"].as_slice()] {
        let output = one_mk(sandbox.path(), &home, args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
    assert_eq!(loose(&home), Vec::<String>::new(), "files made under umask 002 must not be reachable by the group");
}

#[test]
fn files_an_older_version_made_loose_are_tightened_at_start() {
    let sandbox = tempfile::Builder::new().prefix("one-mk-private-home-").tempdir().unwrap();
    for folder in ["config", "data", "run"] {
        std::fs::create_dir(sandbox.path().join(folder)).unwrap();
        std::fs::set_permissions(sandbox.path().join(folder), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let holder = private(sandbox.path());
    let home = holder.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let names = ["daemon.log", "setup-done", "tray.lock", "update-state.json", "profiles.toml"];
    for name in names {
        std::fs::write(home.join(name), "x").unwrap();
        std::fs::set_permissions(home.join(name), std::fs::Permissions::from_mode(0o664)).unwrap();
    }
    std::fs::write(home.join("notes.txt"), "mine").unwrap();
    std::fs::set_permissions(home.join("notes.txt"), std::fs::Permissions::from_mode(0o664)).unwrap();
    let output = one_mk(sandbox.path(), &home, &["devices"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    for name in names {
        assert_eq!(std::fs::metadata(home.join(name)).unwrap().permissions().mode() & 0o777, 0o600, "{name}");
    }
    assert_eq!(std::fs::metadata(home.join("notes.txt")).unwrap().permissions().mode() & 0o777, 0o664, "a file oneMK did not make is left alone");
}
