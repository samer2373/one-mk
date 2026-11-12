//! The helper after crashes and kills, and against wrong or hostile command lines, through the real
//! binary with a fake `gsettings` (see gnome_restore.rs): the person's own wallpaper must come back,
//! half-written files must not pile up, and every wrong invocation is a one-line refusal, not a panic.
#![cfg(target_os = "linux")]

use one_mk_wallpaper::{Canvas, Fit, Media, Monitor, Plan, Rect};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

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
    fn helper(&self, arguments: &[&std::ffi::OsStr]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_one-mk-wallpaper")).args(arguments).env_clear()
            .env("PATH", self.root.path().join("bin")).env("HOME", self.root.path().join("home"))
            .env("XDG_CONFIG_HOME", self.root.path().join("home/.config")).env("XDG_CURRENT_DESKTOP", "GNOME")
            .env("FAKE_GSETTINGS", self.store()).env("XDG_RUNTIME_DIR", self.root.path().join("home"))
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", self.root.path().join("home/no-bus").display())).output().unwrap()
    }
    fn apply(&self, hash: char) -> Output {
        let mut plan = plan(); plan.hash = hash.to_string().repeat(64);
        let bundle = self.root.path().join("bundle.video");
        std::fs::write(&bundle, bundle_bytes()).unwrap();
        let plan = serde_json::to_string(&plan).unwrap();
        self.helper(&["--picture-apply".as_ref(), bundle.as_os_str(), self.pictures().as_os_str(), plan.as_ref(), "host".as_ref()])
    }
    fn restore(&self) -> Output { self.helper(&["--picture-restore".as_ref(), self.pictures().as_os_str()]) }
}

fn plan() -> Plan {
    Plan { hash: "a".repeat(64), title: "Picture".into(), media: Media { width: 8, height: 4, duration_ms: 1, frame_rate: 0 },
        canvas: Canvas { monitors: [0, 4].into_iter().map(|horizontal| Monitor { computer: "host".into(), name: horizontal.to_string(),
            rect: Rect { x: f64::from(horizontal), y: 0.0, width: 4.0, height: 4.0 }, local: None, scale: 1.0, online: true }).collect() },
        start_ms: 1, fit: Fit::Stretch, computers: vec!["host".into()], pause_on_battery: false, paused: false, picture: true, asset_hash: None, spread: Default::default() }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

fn bundle_bytes() -> Vec<u8> {
    let png = png(4, 4);
    let mut bundle = b"oneMKpic".to_vec();
    bundle.extend_from_slice(&2u32.to_le_bytes());
    for _ in 0..2 { bundle.extend_from_slice(&(png.len() as u64).to_le_bytes()); bundle.extend_from_slice(&png); }
    bundle
}

fn ok(output: &Output) { assert!(output.status.success(), "helper failed: {}", String::from_utf8_lossy(&output.stderr)); }

/// A refusal is exit status 1 with one plain message, never a panic (101) or a signal.
fn refused(output: &Output, what: &str) -> String {
    let error = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(1), "{what}: {error}");
    assert!(!error.contains("panicked") && !error.contains("RUST_BACKTRACE"), "{what}: {error}");
    assert!(!error.trim().is_empty(), "{what}: refused without a message");
    error
}

// ---- crash recovery ----------------------------------------------------------------------

#[test]
fn a_crash_after_the_snapshot_and_before_any_change_restores_to_the_same_wallpaper() {
    let desktop = Desktop::new();
    desktop.set("picture-uri", "'file:///home/me/lake.jpg'");
    desktop.set("picture-options", "'scaled'");
    // oneMK saved the original, then died before changing a setting: previous.json and nothing else.
    std::fs::write(desktop.pictures().join("previous.json"),
        serde_json::json!([["picture-uri", "'file:///home/me/lake.jpg'"], ["picture-uri-dark", null], ["picture-options", "'scaled'"]]).to_string()).unwrap();
    ok(&desktop.restore());
    assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///home/me/lake.jpg'");
    assert_eq!(desktop.get("picture-options").unwrap(), "'scaled'");
    assert!(!desktop.pictures().join("previous.json").exists());
    // And the wallpaper can be chosen again afterwards, saving the same original anew.
    ok(&desktop.apply('c'));
    let saved = std::fs::read_to_string(desktop.pictures().join("previous.json")).unwrap();
    assert!(saved.contains("lake.jpg") && saved.contains("scaled"), "{saved}");
    ok(&desktop.restore());
    assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///home/me/lake.jpg'");
}

#[test]
fn a_crash_after_the_change_and_before_the_state_file_still_restores_everything() {
    let desktop = Desktop::new();
    desktop.set("picture-uri", "'file:///home/me/lake.jpg'");
    ok(&desktop.apply('a'));
    // Killed before current.json was written (or deleted by hand): the pictures are still ours and are cleaned up.
    std::fs::remove_file(desktop.pictures().join("current.json")).unwrap();
    ok(&desktop.restore());
    assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///home/me/lake.jpg'");
    let left: Vec<_> = std::fs::read_dir(desktop.pictures()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn restoring_with_nothing_saved_or_no_folder_changes_nothing_and_creates_nothing() {
    let desktop = Desktop::new();
    ok(&desktop.restore());
    let missing = desktop.root.path().join("never-made");
    ok(&desktop.helper(&["--picture-restore".as_ref(), missing.as_os_str()]));
    assert!(!missing.exists());
    assert_eq!(desktop.log(), "");
}

#[test]
fn a_damaged_snapshot_is_refused_without_touching_a_setting_and_is_kept_for_a_person_to_look_at() {
    for (what, content) in [("empty", ""), ("cut off", "[[\"picture-uri\", \"'file:///a.png'\""), ("null", "null"), ("an object", "{}"), ("wrong shape", "[[\"picture-uri\"]]"),
        ("numbers", "[[\"picture-uri\", 5]]"), ("not JSON", "\u{0}\u{0}garbage")] {
        let desktop = Desktop::new();
        desktop.set("picture-uri", "'file:///ours.png'");
        std::fs::write(desktop.pictures().join("previous.json"), content).unwrap();
        let error = refused(&desktop.restore(), what);
        assert!(error.len() < 600, "{what}: a one-line message, not a dump ({} bytes)", error.len());
        assert_eq!(desktop.log(), "", "{what}: a setting was touched");
        assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///ours.png'");
        assert!(desktop.pictures().join("previous.json").exists(), "{what}: the snapshot was thrown away");
    }
}

#[test]
fn a_snapshot_naming_another_setting_is_refused_before_any_setting_is_restored() {
    let desktop = Desktop::new();
    std::fs::write(desktop.pictures().join("previous.json"),
        serde_json::json!([["picture-uri", "'file:///a.png'"], ["primary-color", "'#000000'"], ["picture-options", "'zoom'"]]).to_string()).unwrap();
    let error = refused(&desktop.restore(), "foreign key");
    assert!(error.contains("not recognised"), "{error}");
    assert_eq!(desktop.log(), "", "the valid first entry must not be applied when a later one is refused");
    // A key written as an option for gsettings is just refused, never passed on.
    std::fs::write(desktop.pictures().join("previous.json"), serde_json::json!([["--recursively", null]]).to_string()).unwrap();
    refused(&desktop.restore(), "option-like key");
    assert_eq!(desktop.log(), "");
}

// ---- half-written files -------------------------------------------------------------------

fn age(path: &std::path::Path, seconds: u64) {
    std::fs::File::options().write(true).open(path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(seconds)).unwrap();
}

#[test]
fn files_a_killed_write_left_behind_are_cleaned_up_and_never_pile_up() {
    let desktop = Desktop::new();
    let pictures = desktop.pictures();
    // What `atomic_write` leaves when its process is killed between creating and renaming.
    let old_picture = pictures.join(format!("{}-0.{}-0.tmp", "d".repeat(64), 4242));
    let old_state = pictures.join("previous.4242-1.tmp");
    let fresh = pictures.join("current.4343-0.tmp");
    for path in [&old_picture, &old_state, &fresh] { std::fs::write(path, vec![0u8; 100_000]).unwrap(); }
    age(&old_picture, 3 * 3600);
    age(&old_state, 3 * 3600);
    ok(&desktop.apply('a'));
    assert!(!old_picture.exists() && !old_state.exists(), "hours-old partial files must be removed by the next apply");
    assert!(fresh.exists(), "a partial file that may belong to a live writer must be left alone");
    // Turning off removes every partial file too, and leaves a clean folder.
    ok(&desktop.restore());
    let left: Vec<_> = std::fs::read_dir(&pictures).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn files_that_are_not_pictures_or_partials_are_never_deleted_by_a_cleanup() {
    let desktop = Desktop::new();
    let pictures = desktop.pictures();
    let notes = pictures.join("notes.txt");
    let backup = pictures.join("backup.tmpx");
    let nested = pictures.join("sub");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("old.tmp"), b"not ours to touch").unwrap();
    std::fs::write(&notes, b"keep").unwrap();
    std::fs::write(&backup, b"keep").unwrap();
    for path in [&notes, &backup] { age(path, 3 * 3600); }
    ok(&desktop.apply('a'));
    assert!(notes.exists() && backup.exists() && nested.join("old.tmp").exists());
}

// ---- wrong and hostile command lines ----------------------------------------------------------

#[test]
fn picture_commands_with_the_wrong_arguments_are_refused_plainly() {
    let desktop = Desktop::new();
    let plan = serde_json::to_string(&plan()).unwrap();
    let path = desktop.root.path().join("x");
    let (path, plan, folder) = (path.as_os_str(), plan.as_ref(), desktop.pictures());
    let cases: Vec<(&str, Vec<&std::ffi::OsStr>)> = vec![
        ("unknown operation", vec!["--picture-explode".as_ref(), path, path, plan, "host".as_ref()]),
        ("no operands", vec!["--picture-prepare".as_ref()]),
        ("too few", vec!["--picture-prepare".as_ref(), path, path, plan]),
        ("too many", vec!["--picture-prepare".as_ref(), path, path, plan, "host".as_ref(), "extra".as_ref()]),
        ("restore without a folder", vec!["--picture-restore".as_ref()]),
        ("restore with two folders", vec!["--picture-restore".as_ref(), folder.as_os_str(), folder.as_os_str()]),
        ("inspect with one operand", vec!["--picture-inspect".as_ref(), path]),
        ("not a plan", vec!["--picture-prepare".as_ref(), path, path, "{".as_ref(), "host".as_ref()]),
        ("null", vec!["--picture-apply".as_ref(), path, path, "null".as_ref(), "host".as_ref()]),
        ("empty", vec!["--picture-apply".as_ref(), path, path, "".as_ref(), "host".as_ref()]),
        ("a computer name of nothing", vec!["--picture-apply".as_ref(), path, path, plan, "".as_ref()]),
    ];
    for (what, arguments) in cases { refused(&desktop.helper(&arguments), what); }
    // A plan that is JSON and a plan, but not a picture plan or not a valid one.
    let mut video = self::plan(); video.picture = false;
    let video = serde_json::to_string(&video).unwrap();
    let error = refused(&desktop.helper(&["--picture-apply".as_ref(), path, path, video.as_ref(), "host".as_ref()]), "a video plan");
    assert!(error.contains("Invalid picture wallpaper plan"), "{error}");
    let mut short = self::plan(); short.hash = "abc".into();
    let short = serde_json::to_string(&short).unwrap();
    refused(&desktop.helper(&["--picture-apply".as_ref(), path, path, short.as_ref(), "host".as_ref()]), "a short hash");
    // Over-long input is refused by size before it is parsed.
    let huge = format!("{{\"title\":\"{}\"}}", "x".repeat(13_000));
    let error = refused(&desktop.helper(&["--picture-apply".as_ref(), path, path, huge.as_ref(), "host".as_ref()]), "a 13 KB plan");
    assert!(error.contains("too large"), "{error}");
    let long_name = "n".repeat(129);
    let error = refused(&desktop.helper(&["--picture-apply".as_ref(), path, path, plan, long_name.as_ref()]), "a 129-byte computer name");
    assert!(error.contains("Invalid picture computer"), "{error}");
    assert_eq!(desktop.log(), "", "no refused command may touch a setting");
}

#[test]
fn arguments_that_are_not_text_are_refused_not_crashed_on() {
    let desktop = Desktop::new();
    let not_text = std::ffi::OsStr::from_bytes(b"\xff\xfe\xfd");
    let plan = serde_json::to_string(&plan()).unwrap();
    let path = desktop.root.path().join("x");
    for arguments in [
        vec!["--picture-prepare".as_ref(), path.as_os_str(), path.as_os_str(), not_text, "host".as_ref()],
        vec!["--picture-prepare".as_ref(), path.as_os_str(), path.as_os_str(), plan.as_ref(), not_text],
        vec![not_text],
        vec!["--picture-\u{ff}".as_ref(), path.as_os_str()],
    ] {
        let output = desktop.helper(&arguments);
        assert!(output.status.code().is_some_and(|code| code != 101), "{arguments:?} panicked or was killed: {output:?}");
        assert!(!output.status.success());
    }
}

#[test]
fn a_missing_or_garbage_picture_never_replaces_the_current_wallpaper_state() {
    let desktop = Desktop::new();
    desktop.set("picture-uri", "'file:///home/me/lake.jpg'");
    let plan = serde_json::to_string(&plan()).unwrap();
    // Bundles of every wrong shape.
    let good = bundle_bytes();
    let mut wrong_count = good.clone(); wrong_count[8..12].copy_from_slice(&3u32.to_le_bytes());
    let mut huge_length = good.clone(); huge_length[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut over_limit = good.clone(); over_limit[12..20].copy_from_slice(&(256u64 * 1024 * 1024 + 1).to_le_bytes());
    let mut trailing = good.clone(); trailing.extend_from_slice(b"extra");
    let mut wrong_size = b"oneMKpic".to_vec();
    wrong_size.extend_from_slice(&2u32.to_le_bytes());
    for _ in 0..2 { let slice = png(5, 5); wrong_size.extend_from_slice(&(slice.len() as u64).to_le_bytes()); wrong_size.extend_from_slice(&slice); }
    let bundles: Vec<(&str, Vec<u8>)> = vec![
        ("empty file", vec![]), ("only the header", b"oneMKpic".to_vec()), ("other magic", b"notAPicture-at-all".to_vec()),
        ("a count that is not the screens", wrong_count), ("a length of u64::MAX", huge_length), ("a length over the limit", over_limit),
        ("trailing bytes", trailing), ("a slice of the wrong size", wrong_size), ("cut in the middle", good[..good.len() / 2].to_vec()),
        ("a text file", b"hello hello hello hello".repeat(10)),
    ];
    for (what, bytes) in bundles {
        let bundle = desktop.root.path().join("bundle.video");
        std::fs::write(&bundle, bytes).unwrap();
        let output = desktop.helper(&["--picture-apply".as_ref(), bundle.as_os_str(), desktop.pictures().as_os_str(), plan.as_ref(), "host".as_ref()]);
        refused(&output, what);
        assert_eq!(desktop.log(), "", "{what}: a setting was touched");
        assert!(!desktop.pictures().join("previous.json").exists() && !desktop.pictures().join("current.json").exists(), "{what}: state was written");
        let pictures: Vec<_> = std::fs::read_dir(desktop.pictures()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert!(pictures.is_empty(), "{what}: left {pictures:?}");
    }
    let missing = desktop.root.path().join("no-such-bundle");
    refused(&desktop.helper(&["--picture-apply".as_ref(), missing.as_os_str(), desktop.pictures().as_os_str(), plan.as_ref(), "host".as_ref()]), "a missing bundle");
    assert_eq!(desktop.get("picture-uri").unwrap(), "'file:///home/me/lake.jpg'");
}

#[test]
fn the_player_and_media_modes_refuse_wrong_invocations_before_using_any_media_library() {
    let desktop = Desktop::new();
    let scratch = desktop.root.path();
    let plan_file = scratch.join("plan.json");
    for (what, arguments) in [
        ("no arguments", vec![]),
        ("a plan file that is not there", vec![plan_file.as_os_str()]),
        ("--prepare with nothing", vec!["--prepare".as_ref()]),
        ("--prepare with no size", vec!["--prepare".as_ref(), scratch.as_os_str(), scratch.as_os_str()]),
        ("--prepare with a size that is not JSON", vec!["--prepare".as_ref(), scratch.as_os_str(), scratch.as_os_str(), "wide".as_ref()]),
        ("--prepare with a 1 pixel size", vec!["--prepare".as_ref(), scratch.as_os_str(), scratch.as_os_str(), "{\"width\":1,\"height\":1}".as_ref()]),
        ("--prepare with a 16385 pixel size", vec!["--prepare".as_ref(), scratch.as_os_str(), scratch.as_os_str(), "{\"width\":16385,\"height\":2}".as_ref()]),
        ("--probe with nothing", vec!["--probe".as_ref()]),
        ("--still with nothing", vec!["--still".as_ref()]),
        ("--still with no width", vec!["--still".as_ref(), scratch.as_os_str(), scratch.as_os_str()]),
        ("--still with a width that is not a number", vec!["--still".as_ref(), scratch.as_os_str(), scratch.as_os_str(), "wide".as_ref()]),
    ] {
        let output = desktop.helper(&arguments);
        refused(&output, what);
    }
    for (what, content) in [("an empty plan file", ""), ("a plan file that is not JSON", "{"), ("a plan file of the wrong shape", "[1,2,3]"), ("a binary plan file", "\u{0}\u{1}\u{2}")] {
        std::fs::write(&plan_file, content).unwrap();
        refused(&desktop.helper(&[plan_file.as_os_str()]), what);
    }
    // A plan file that parses as a player config but whose plan is invalid.
    let config = serde_json::json!({ "path": scratch.join("v"), "plan": { "hash": "x" } }).to_string();
    std::fs::write(&plan_file, config).unwrap();
    refused(&desktop.helper(&[plan_file.as_os_str()]), "an incomplete plan");
    assert_eq!(desktop.log(), "");
}
