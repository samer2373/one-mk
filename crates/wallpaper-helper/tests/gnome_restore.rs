//! The GNOME picture path end to end through the real helper binary, against
//! a fake `gsettings` (a shell script keeping values in files). The helper
//! runs with an empty environment whose PATH holds only the fake, so the
//! real desktop settings can never be read or written from here.
#![cfg(target_os = "linux")]

use one_mk_wallpaper::{Canvas, Fit, Media, Monitor, Plan, Rect};
use std::path::{Path, PathBuf};
use std::process::Command;

const FAKE: &str = r#"#!/bin/sh
store="$FAKE_GSETTINGS"
[ "$2" = org.gnome.desktop.background ] || { printf 'No such schema\n' >&2; exit 1; }
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

struct Desktop { root: tempfile::TempDir }

impl Desktop {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for folder in ["bin", "store", "home", "config/wallpaper-video/pictures"] { std::fs::create_dir_all(root.path().join(folder)).unwrap(); }
        let fake = root.path().join("bin/gsettings");
        std::fs::write(&fake, FAKE).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { root }
    }
    fn store(&self) -> PathBuf { self.root.path().join("store") }
    fn pictures(&self) -> PathBuf { self.root.path().join("config/wallpaper-video/pictures") }
    fn set(&self, key: &str, value: &str) { std::fs::write(self.store().join(key), format!("{value}\n")).unwrap(); }
    fn get(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.store().join(key)).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
    }
    fn log(&self) -> String { std::fs::read_to_string(self.store().join("log")).unwrap_or_default() }
    fn helper(&self, desktop: &str, arguments: &[&std::ffi::OsStr]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_one-mk-wallpaper")).args(arguments).env_clear()
            .env("PATH", self.root.path().join("bin")).env("HOME", self.root.path().join("home"))
            .env("XDG_CONFIG_HOME", self.root.path().join("home/.config")).env("XDG_CURRENT_DESKTOP", desktop)
            .env("FAKE_GSETTINGS", self.store()).env("XDG_RUNTIME_DIR", self.root.path().join("home"))
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", self.root.path().join("home/no-bus").display())).output().unwrap()
    }
    fn apply(&self, desktop: &str, hash: char) -> std::process::Output {
        let mut plan = plan(); plan.hash = hash.to_string().repeat(64);
        let bundle = self.root.path().join("bundle.video");
        std::fs::write(&bundle, bundle_bytes()).unwrap();
        let plan = serde_json::to_string(&plan).unwrap();
        self.helper(desktop, &["--picture-apply".as_ref(), bundle.as_os_str(), self.pictures().as_os_str(), plan.as_ref(), "host".as_ref()])
    }
    fn restore(&self) -> std::process::Output { self.helper("GNOME", &["--picture-restore".as_ref(), self.pictures().as_os_str()]) }
}

fn plan() -> Plan {
    Plan { hash: "a".repeat(64), title: "Picture".into(), media: Media { width: 8, height: 4, duration_ms: 1, frame_rate: 0 },
        canvas: Canvas { monitors: [0, 4].into_iter().map(|horizontal| Monitor { computer: "host".into(), name: horizontal.to_string(),
            rect: Rect { x: f64::from(horizontal), y: 0.0, width: 4.0, height: 4.0 }, local: None, scale: 1.0, online: true }).collect() },
        start_ms: 1, fit: Fit::Stretch, computers: vec!["host".into()], pause_on_battery: false, paused: false, picture: true, asset_hash: None, spread: Default::default() }
}

fn bundle_bytes() -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();
    let mut bundle = b"oneMKpic".to_vec();
    bundle.extend_from_slice(&2u32.to_le_bytes());
    for _ in 0..2 { bundle.extend_from_slice(&(png.len() as u64).to_le_bytes()); bundle.extend_from_slice(&png); }
    bundle
}

fn ok(output: &std::process::Output) {
    assert!(output.status.success(), "helper failed: {}", String::from_utf8_lossy(&output.stderr));
}

fn ours(value: &str, pictures: &Path) -> bool { value.contains(pictures.canonicalize().unwrap().to_str().unwrap()) }

#[test]
fn gnome_pictures_restore_exactly_the_persons_own_wallpaper() {
    // The person's own picture and options; dark is at the desktop default.
    let desktop = Desktop::new();
    desktop.set("picture-uri", "'file:///home/me/Pictures/lake.jpg'");
    desktop.set("picture-options", "'scaled'");
    ok(&desktop.apply("ubuntu:GNOME", 'a'));
    assert!(ours(&desktop.get("picture-uri").unwrap(), &desktop.pictures()));
    assert!(ours(&desktop.get("picture-uri-dark").unwrap(), &desktop.pictures()));
    assert_eq!(desktop.get("picture-options").unwrap(), "'spanned'");
    // Saved to disk before the first change, so a crash or a restart of the
    // daemon (a new helper process every time) restores the same original.
    let saved = std::fs::read_to_string(desktop.pictures().join("previous.json")).unwrap();
    assert!(saved.contains("lake.jpg") && saved.contains("scaled") && saved.contains("null"), "{saved}");
    // A second picture never replaces that original with oneMK's own.
    ok(&desktop.apply("GNOME", 'b'));
    assert_eq!(std::fs::read_to_string(desktop.pictures().join("previous.json")).unwrap(), saved);
    ok(&desktop.restore());
    assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///home/me/Pictures/lake.jpg'");
    assert_eq!(desktop.get("picture-options").unwrap(), "'scaled'");
    // At its default before: reset, not pinned to today's default value.
    assert_eq!(desktop.get("picture-uri-dark"), None);
    assert!(desktop.log().contains("reset picture-uri-dark"), "{}", desktop.log());
    assert!(!desktop.pictures().join("previous.json").exists());
    // Nothing left to restore: a second Off changes nothing.
    let before = desktop.log();
    ok(&desktop.restore());
    assert_eq!(desktop.log(), before);
}

#[test]
fn a_lost_snapshot_never_saves_onemks_own_picture_as_the_original() {
    let desktop = Desktop::new();
    ok(&desktop.apply("GNOME", 'a'));
    // The snapshot is lost (deleted by hand, an old bug), the picture stays.
    std::fs::remove_file(desktop.pictures().join("previous.json")).unwrap();
    ok(&desktop.apply("GNOME", 'b'));
    ok(&desktop.restore());
    for key in ["picture-uri", "picture-uri-dark", "picture-options"] {
        assert!(desktop.get(key).is_none_or(|value| !ours(&value, &desktop.pictures())), "{key}");
    }
}

#[test]
fn snapshots_from_older_versions_still_restore() {
    let desktop = Desktop::new();
    let old = serde_json::json!([["picture-uri", "'file:///old.png'\n"], ["picture-uri-dark", "'file:///old-dark.png'\n"], ["picture-options", "'zoom'\n"]]);
    std::fs::write(desktop.pictures().join("previous.json"), old.to_string()).unwrap();
    ok(&desktop.restore());
    assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///old.png'");
    assert_eq!(desktop.get("picture-uri-dark").unwrap(), "'file:///old-dark.png'");
}

#[test]
fn other_linux_desktops_are_refused_without_touching_any_setting() {
    for name in ["KDE", "XFCE", "X-Cinnamon", ""] {
        let desktop = Desktop::new();
        let output = desktop.apply(name, 'a');
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("need GNOME") && error.contains("leaves it unchanged"), "{error}");
        assert_eq!(desktop.log(), "");
        assert!(!desktop.pictures().join("previous.json").exists());
    }
}
