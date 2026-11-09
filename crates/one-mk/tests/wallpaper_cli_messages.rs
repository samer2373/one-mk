//! What `one-mk wallpaper ...` tells a person when something is missing or
//! fails: the words and the exit code, with a throwaway home, no desktop
//! session and a PATH holding only stand-in programs (never the real
//! `gsettings`, `gnome-shell` or `gnome-extensions`).
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::Builder::new().prefix("one-mk-wallpaper-cli-").tempdir().unwrap();
        for folder in ["home", "bin", "run", "data"] {
            std::fs::create_dir(root.path().join(folder)).unwrap();
            std::fs::set_permissions(root.path().join(folder), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self { root }
    }

    fn path(&self, name: &str) -> std::path::PathBuf { self.root.path().join(name) }

    /// A program on the stand-in PATH that prints `stdout` and exits with `code`.
    fn program(&self, name: &str, stdout: &str, code: i32) {
        let file = self.path("bin").join(name);
        std::fs::write(&file, format!("#!/bin/sh\necho '{stdout}'\nexit {code}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_one-mk"))
            .env_clear().env("PATH", self.path("bin")).env("HOME", self.path("home")).env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_RUNTIME_DIR", self.path("run")).env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", self.path("run").join("no-bus").display()))
            .arg("--home").arg(self.path("home")).args(args).output().unwrap()
    }

    fn renderer(&self) -> std::path::PathBuf {
        let renderer = self.path("renderer");
        std::fs::write(&renderer, "renderer").unwrap();
        renderer
    }
}

fn stdout(output: &Output) -> String { String::from_utf8_lossy(&output.stdout).into_owned() }
fn stderr(output: &Output) -> String { String::from_utf8_lossy(&output.stderr).into_owned() }
fn extension(sandbox: &Sandbox) -> std::path::PathBuf { sandbox.path("data").join("gnome-shell/extensions/one-mk-wallpaper@one-mk.org") }

#[test]
fn status_without_the_service_says_off_and_why_and_changes_fail_with_a_fix() {
    let sandbox = Sandbox::new();
    let status = sandbox.run(&["wallpaper", "status"]);
    assert!(status.status.success(), "{}", stderr(&status));
    let text = stdout(&status);
    assert!(text.contains("off") && text.contains("oneMK is not running") && text.contains("one-mk run"), "{text}");
    assert!(!text.contains("Start oneMK first"), "{text}");
    let json = sandbox.run(&["wallpaper", "--json", "status"]);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!((value["ok"].clone(), value["running"].clone()), (true.into(), false.into()));
    let picture = sandbox.path("picture.png");
    std::fs::write(&picture, "x").unwrap();
    for args in [vec!["wallpaper", "set", picture.to_str().unwrap()], vec!["wallpaper", "off"], vec!["wallpaper", "pause"], vec!["wallpaper", "play"]] {
        let failed = sandbox.run(&args);
        assert!(!failed.status.success(), "{args:?}");
        assert!(stderr(&failed).contains("oneMK is not running. Start it with `one-mk run`"), "{args:?}: {}", stderr(&failed));
    }
    // --json: the failure is JSON too, and the exit code still says so.
    let failed = sandbox.run(&["wallpaper", "--json", "off"]);
    assert!(!failed.status.success());
    let value: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert!(value["ok"] == false && value["error"].as_str().unwrap().contains("not running"), "{value}");
}

#[test]
fn install_that_cannot_switch_the_extension_on_fails_and_says_why() {
    let sandbox = Sandbox::new();
    sandbox.program("gnome-shell", "GNOME Shell 50.1", 0);
    // gsettings that fails and says nothing: the message used to end "(GNOME settings said: )".
    sandbox.program("gsettings", "", 1);
    // (a bare `exit 1`: no output at all)
    std::fs::write(sandbox.path("bin").join("gsettings"), "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(sandbox.path("bin").join("gsettings"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let renderer = sandbox.renderer();
    let output = sandbox.run(&["wallpaper", "extension", "install", "--renderer", renderer.to_str().unwrap()]);
    assert!(!output.status.success(), "a half-done install must not exit 0: {}", stdout(&output));
    let error = stderr(&output);
    assert!(error.contains("Installed in") && error.contains("could not switch it on") && error.contains("One step left"), "{error}");
    assert!(error.contains("without saying why") && !error.contains("said: )") && !error.contains("said: ."), "{error}");
    assert!(extension(&sandbox).join("metadata.json").is_file(), "the files are in place");
}

#[test]
fn install_refuses_other_gnome_versions_and_computers_without_gnome() {
    let sandbox = Sandbox::new();
    let renderer = sandbox.renderer();
    let args = ["wallpaper", "extension", "install", "--renderer", renderer.to_str().unwrap()];
    let none = sandbox.run(&args);
    assert!(!none.status.success() && stderr(&none).contains("GNOME Shell was not found"), "{}", stderr(&none));
    sandbox.program("gnome-shell", "GNOME Shell 49.2", 0);
    let other = sandbox.run(&args);
    assert!(!other.status.success() && stderr(&other).contains("GNOME Shell 49.2 is not supported") && stderr(&other).contains("Nothing was installed"), "{}", stderr(&other));
    assert!(!extension(&sandbox).exists(), "nothing is installed for an unsupported GNOME");
}

#[test]
fn uninstall_says_when_gnome_could_not_be_told_and_succeeds_when_there_is_nothing_to_tell() {
    let sandbox = Sandbox::new();
    sandbox.program("gnome-shell", "GNOME Shell 50.1", 0);
    let renderer = sandbox.renderer();
    // gsettings is not installed at all: no GNOME settings to keep tidy, so it is a clean removal.
    let installed = sandbox.run(&["wallpaper", "extension", "install", "--renderer", renderer.to_str().unwrap()]);
    assert!(!installed.status.success());
    let clean = sandbox.run(&["wallpaper", "extension", "uninstall"]);
    assert!(clean.status.success(), "{}", stderr(&clean));
    assert!(stdout(&clean).contains("Switched off and removed") && !extension(&sandbox).exists(), "{}", stdout(&clean));
    // gsettings is there but refuses: the files go, the command says what is left.
    std::fs::write(sandbox.path("bin").join("gsettings"), "#!/bin/sh\necho 'dconf is not running' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(sandbox.path("bin").join("gsettings"), std::fs::Permissions::from_mode(0o755)).unwrap();
    sandbox.run(&["wallpaper", "extension", "install", "--renderer", renderer.to_str().unwrap()]);
    let failed = sandbox.run(&["wallpaper", "extension", "uninstall"]);
    assert!(!failed.status.success(), "{}", stdout(&failed));
    let error = stderr(&failed);
    assert!(error.contains("Removed oneMK's GNOME extension") && error.contains("could not switch it off in GNOME") && error.contains("gnome-extensions disable one-mk-wallpaper@one-mk.org") && error.contains("dconf is not running"), "{error}");
    assert!(!stdout(&failed).contains("Switched off and removed"));
    assert!(!extension(&sandbox).exists(), "the files are removed all the same");
}

#[test]
fn json_is_offered_only_where_it_is_implemented() {
    let sandbox = Sandbox::new();
    for command in ["set", "off", "status", "pause", "play"] {
        let help = sandbox.run(&["wallpaper", command, "--help"]);
        assert!(stdout(&help).contains("--json"), "{command}");
    }
    let help = sandbox.run(&["wallpaper", "--help"]);
    assert!(stdout(&help).contains("JSON") && !stdout(&help).contains("only `extension`"), "{}", stdout(&help));
}

/// `pointer-extension install` also refuses a GNOME it was not tried on, or one
/// whose version cannot be read, and installs nothing (docs/INPUT.md says so).
#[test]
fn the_pointer_extension_is_not_installed_for_an_unknown_or_other_gnome() {
    let sandbox = Sandbox::new();
    let installed = sandbox.path("data").join("gnome-shell/extensions/one-mk-pointer@one-mk.org");
    let none = sandbox.run(&["pointer-extension", "install"]);
    assert!(!none.status.success() && stderr(&none).contains("GNOME Shell was not found") && stderr(&none).contains("nothing was installed"), "{}", stderr(&none));
    for said in ["GNOME Shell 49.2", "GNOME Shell 51.0", "GNOME Shell", "unknown", "", "GNOME Shell 5.0"] {
        sandbox.program("gnome-shell", said, 0);
        let refused = sandbox.run(&["pointer-extension", "install"]);
        assert!(!refused.status.success() && stderr(&refused).contains("nothing was installed"), "{said:?}: {}", stderr(&refused));
        assert!(!installed.exists(), "{said:?}");
    }
    // A shell that cannot even say its version.
    sandbox.program("gnome-shell", "GNOME Shell 50.1", 1);
    assert!(!sandbox.run(&["pointer-extension", "install"]).status.success() && !installed.exists());
    sandbox.program("gnome-shell", "GNOME Shell 50.1", 0);
    let fine = sandbox.run(&["pointer-extension", "install"]);
    assert!(fine.status.success() && installed.join("metadata.json").is_file(), "{}", stderr(&fine));
}
