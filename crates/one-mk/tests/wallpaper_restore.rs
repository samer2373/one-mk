//! Picture wallpapers end to end through the real daemon and helper: the
//! person's own GNOME wallpaper comes back exactly, after a crash and after
//! `one-mk uninstall`, and other desktops are told plainly that pictures
//! can't be set there. GNOME's settings are a fake `gsettings` (a shell
//! script keeping values in files) and every process runs with an empty
//! environment whose PATH holds only that fake: the real desktop settings
//! can't be reached. Build `one-mk-wallpaper-helper` first when running this
//! file alone.
#![cfg(target_os = "linux")]

use one_mk_ctl::client::{self, Request};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const FAKE: &str = r#"#!/bin/sh
store="$FAKE_GSETTINGS"
[ "$2" = org.gnome.desktop.background ] || { printf 'No such schema “%s”\n' "$2" >&2; exit 1; }
default() {
    case "$1" in
    picture-uri) printf "'file:///usr/share/backgrounds/default.png'\n" ;;
    picture-uri-dark) printf "'file:///usr/share/backgrounds/default-dark.png'\n" ;;
    picture-options) printf "'zoom'\n" ;;
    esac
}
case "$1" in
get) if [ "$GSETTINGS_BACKEND" != memory ] && [ -s "$store/$3" ]; then IFS= read -r value < "$store/$3"; printf '%s\n' "$value"; else default "$3"; fi ;;
set) case "$4" in "'"*) value="$4" ;; *) value="'$4'" ;; esac
    printf '%s\n' "$value" > "$store/$3"; printf 'set %s %s\n' "$3" "$value" >> "$store/log" ;;
reset) : > "$store/$3"; printf 'reset %s\n' "$3" >> "$store/log" ;;
*) exit 1 ;;
esac
"#;

struct Desktop { root: tempfile::TempDir, name: &'static str }

struct Daemon(Child);
impl Drop for Daemon { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

impl Desktop {
    fn new(name: &'static str) -> Self {
        let root = tempfile::tempdir().unwrap();
        for folder in ["bin", "store", "home", "data", "config", "run"] { std::fs::create_dir_all(root.path().join(folder)).unwrap(); }
        let fake = root.path().join("bin/gsettings");
        std::fs::write(&fake, FAKE).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // `extension install` asks the shell's version (it refuses other GNOME series and no GNOME at all).
        let shell = root.path().join("bin/gnome-shell");
        std::fs::write(&shell, "#!/bin/sh\necho 'GNOME Shell 50.1'\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(root.path().join("home/settings.toml"), "[update]\ncheck = false\n").unwrap();
        Self { root, name }
    }
    fn dir(&self) -> PathBuf { self.root.path().join("home") }
    fn store(&self) -> PathBuf { self.root.path().join("store") }
    fn set(&self, key: &str, value: &str) { std::fs::write(self.store().join(key), format!("{value}\n")).unwrap(); }
    fn get(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.store().join(key)).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
    }
    fn log(&self) -> String { std::fs::read_to_string(self.store().join("log")).unwrap_or_default() }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk"));
        command.env_clear().env("PATH", self.root.path().join("bin")).env("HOME", self.dir())
            .env("XDG_CONFIG_HOME", self.root.path().join("config")).env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_CURRENT_DESKTOP", self.name).env("FAKE_GSETTINGS", self.store())
            // Never the real session bus: zbus falls back to /run/user/<uid>/bus
            // when neither of these is set.
            .env("XDG_RUNTIME_DIR", self.root.path().join("run")).env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", self.root.path().join("run/no-bus").display()))
            .arg("--home").arg(self.dir());
        command
    }
    fn start(&self) -> Daemon {
        let daemon = Daemon(self.command().args(["run", "--backend", "null", "--no-discovery", "--no-clipboard", "--no-notify", "--port", "0"])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        for _ in 0..200 {
            if client::request(&self.dir(), Request::Status, Duration::from_secs(1)).ok().flatten().is_some() { return daemon; }
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("daemon failed to start");
    }
    fn ask(&self, command: one_mk_wallpaper::Command) -> String {
        client::request(&self.dir(), one_mk_ctl::wallpaper::request(&command), Duration::from_secs(60)).unwrap().unwrap()
    }
    fn picture(&self) -> PathBuf {
        let path = self.root.path().join("lake.png");
        image::RgbaImage::from_pixel(64, 36, image::Rgba([20, 90, 160, 255])).save(&path).unwrap();
        path
    }
    fn wait_for(&self, what: &str, done: impl Fn() -> bool) {
        for _ in 0..300 { if done() { return; } std::thread::sleep(Duration::from_millis(20)); }
        panic!("{what}: {}", self.log());
    }
}

fn ours(value: Option<String>, desktop: &Desktop) -> bool { value.is_some_and(|value| value.contains(desktop.dir().canonicalize().unwrap().to_str().unwrap())) }

fn own_wallpaper_is_back(desktop: &Desktop) {
    assert_eq!(desktop.get("picture-uri").as_deref(), Some("'file:///home/me/Pictures/lake.jpg'"), "{}", desktop.log());
    assert_eq!(desktop.get("picture-options").as_deref(), Some("'scaled'"));
    assert_eq!(desktop.get("picture-uri-dark"), None, "a default stays a default: {}", desktop.log());
    assert!(!desktop.dir().join("wallpaper-video/pictures/previous.json").exists());
}

#[test]
fn the_persons_own_gnome_wallpaper_comes_back_after_off_a_crash_and_uninstall() {
    let desktop = Desktop::new("ubuntu:GNOME");
    desktop.set("picture-uri", "'file:///home/me/Pictures/lake.jpg'");
    desktop.set("picture-options", "'scaled'");
    let picture = desktop.picture();

    // Set, crash (no chance to clean up), start again, turn off.
    let mut daemon = desktop.start();
    assert_eq!(desktop.ask(one_mk_wallpaper::Command::Set { path: picture.clone() }), "ok\n");
    assert!(ours(desktop.get("picture-uri"), &desktop), "{}", desktop.log());
    let _ = daemon.0.kill(); let _ = daemon.0.wait();
    let _daemon = desktop.start();
    assert_eq!(desktop.ask(one_mk_wallpaper::Command::Off), "ok\n");
    own_wallpaper_is_back(&desktop);
    drop(_daemon);

    // Set, crash, and uninstall while nothing runs.
    let mut daemon = desktop.start();
    assert_eq!(desktop.ask(one_mk_wallpaper::Command::Set { path: picture }), "ok\n");
    let _ = daemon.0.kill(); let _ = daemon.0.wait();
    let output = desktop.command().arg("uninstall").output().unwrap();
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{said}{}", String::from_utf8_lossy(&output.stderr));
    assert!(said.contains("your own wallpaper is in place") && said.contains("GNOME extension is not installed"), "{said}");
    own_wallpaper_is_back(&desktop);
    // Starting oneMK again doesn't bring the picture back.
    let log = desktop.log();
    let _daemon = desktop.start();
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(desktop.log(), log);
}

#[test]
fn an_original_left_behind_by_a_crash_is_put_back_when_onemk_starts() {
    let desktop = Desktop::new("GNOME");
    desktop.set("picture-uri", "'file:///home/me/Pictures/lake.jpg'");
    desktop.set("picture-options", "'scaled'");
    let mut daemon = desktop.start();
    assert_eq!(desktop.ask(one_mk_wallpaper::Command::Set { path: desktop.picture() }), "ok\n");
    let _ = daemon.0.kill(); let _ = daemon.0.wait();
    // As if Off had been saved but oneMK stopped before restoring.
    std::fs::remove_file(desktop.dir().join("wallpaper-video/selection.json")).unwrap();
    let _daemon = desktop.start();
    desktop.wait_for("restored at start", || !desktop.dir().join("wallpaper-video/pictures/previous.json").exists());
    own_wallpaper_is_back(&desktop);
}

#[test]
fn uninstall_removes_the_gnome_extension_onemk_installed() {
    let desktop = Desktop::new("GNOME");
    // This stand-in GNOME has no `org.gnome.shell` settings, so the extension cannot be
    // switched on for the next login: the files are put in place and the command says so.
    let installed = desktop.command().args(["wallpaper", "extension", "install"]).output().unwrap();
    assert!(!installed.status.success() && String::from_utf8_lossy(&installed.stderr).contains("Installed in") && String::from_utf8_lossy(&installed.stderr).contains("No such schema"),
        "{}", String::from_utf8_lossy(&installed.stderr));
    let extension = desktop.root.path().join("data/gnome-shell/extensions/one-mk-wallpaper@one-mk.org");
    assert!(extension.join("extension.js").is_file());
    let output = desktop.command().arg("uninstall").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("removed oneMK's GNOME extension"));
    assert!(!extension.exists());
}

#[test]
fn other_linux_desktops_and_bad_choices_are_told_plainly() {
    let desktop = Desktop::new("KDE");
    let _daemon = desktop.start();
    let reply = desktop.ask(one_mk_wallpaper::Command::Set { path: desktop.picture() });
    assert!(reply.starts_with("error ") && reply.contains("need GNOME") && reply.contains("leaves it unchanged"), "{reply}");
    assert_eq!(desktop.log(), "", "no GNOME setting is written on KDE");
    let missing = desktop.root.path().join("missing.mp4");
    let reply = desktop.ask(one_mk_wallpaper::Command::Set { path: missing });
    assert!(reply.contains("missing.mp4 does not exist"), "{reply}");
    let reply = desktop.ask(one_mk_wallpaper::Command::Set { path: desktop.root.path().join("bin") });
    assert!(reply.contains("is a folder") && !reply.contains("changed before sending"), "{reply}");
    // The command line says the same, naming the file.
    let output = desktop.command().args(["wallpaper", "set"]).arg(desktop.root.path().join("nope.webm")).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nope.webm does not exist"), "{}", String::from_utf8_lossy(&output.stderr));
}
