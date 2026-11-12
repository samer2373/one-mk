//! QA click-through flows over the whole window: Keys, network profiles,
//! clipboard history, the wallpaper page, the update pill, Settings and the
//! first-run setup. Every flow presses what a person presses and then reads
//! two things back: what the window shows, and what it asked `one-mk` (or the
//! running daemon) to do.
//!
//! Neither is the real thing. `one-mk` is a small shell script in a private
//! folder that writes down every call and can be told to fail or to print
//! something; the daemon is a socket in the window's private folder that
//! writes down every request and answers from a table. Nothing here can touch
//! a real oneMK, login item or desktop.

use super::*;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The stand-in for `one-mk`. The window always passes `--home <dir>`; the
/// script keeps its notes in `<dir>/fake`: `calls.log` (one call a line, the
/// arguments separated by tabs, `--home` left out), and it fails (message on
/// stderr, exit 1) or prints what the files `fail-<first>-<second>`,
/// `fail-<first>`, `out-<first>-<second>` and `out-<first>` say, the most
/// specific first. `<second>` is `_` for a call of one word (`update` alone).
const SCRIPT: &str = r#"#!/bin/sh
home=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--home" ]; then home="$a"; fi
  prev="$a"
done
[ -n "$home" ] || { echo "fake one-mk: no --home" >&2; exit 64; }
state="$home/fake"
mkdir -p "$state"
line=""
first=""
second=""
skip=0
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  if [ "$a" = "--home" ]; then skip=1; continue; fi
  if [ -z "$first" ]; then first="$a"; elif [ -z "$second" ]; then second="$a"; fi
  line="$line$a	"
done
printf '%s\n' "$line" >> "$state/calls.log"
[ -n "$second" ] || second="_"
if [ -f "$state/fail-$first-$second" ]; then cat "$state/fail-$first-$second" >&2; exit 1; fi
if [ -f "$state/fail-$first" ]; then cat "$state/fail-$first" >&2; exit 1; fi
if [ -f "$state/out-$first-$second" ]; then cat "$state/out-$first-$second"; elif [ -f "$state/out-$first" ]; then cat "$state/out-$first"; fi
exit 0
"#;

/// The script is shared by every test of the process and removed with the
/// last of them (writing one per test could meet another test's fork while
/// the file is still open for writing: "text file busy").
static SHARED: Mutex<(usize, Option<PathBuf>)> = Mutex::new((0, None));

/// A `one-mk` that only writes down what it was asked.
pub(super) struct Fake {
    state: PathBuf,
}

impl Fake {
    /// Makes this window's `one-mk` the stand-in.
    pub fn install(win: &mut Win) -> Fake {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        if shared.1.is_none() {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("omk-fake-onemk-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let script = dir.join("one-mk");
            std::fs::write(&script, SCRIPT).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            shared.1 = Some(script);
        }
        shared.0 += 1;
        win.app.link.daemon = shared.1.clone().unwrap();
        let state = win.dir.join("fake");
        std::fs::create_dir_all(&state).unwrap();
        Fake { state }
    }

    /// Every call so far, each as its arguments.
    pub fn calls(&self) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(self.state.join("calls.log")).unwrap_or_default();
        text.lines().map(|l| l.split('\t').filter(|a| !a.is_empty()).map(str::to_owned).collect()).collect()
    }

    /// The calls whose first arguments are these.
    pub fn calls_with(&self, start: &[&str]) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|c| c.len() >= start.len() && c.iter().zip(start).all(|(a, b)| a == b))
            .collect()
    }

    /// Calls starting with `first` (or `first-second`) fail with this message.
    pub fn fail(&self, which: &str, message: &str) {
        std::fs::write(self.state.join(format!("fail-{which}")), message).unwrap();
    }

    pub fn mend(&self, which: &str) {
        let _ = std::fs::remove_file(self.state.join(format!("fail-{which}")));
    }

    /// Calls starting with `first` print this.
    pub fn prints(&self, first: &str, text: &str) {
        std::fs::write(self.state.join(format!("out-{first}")), text).unwrap();
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        shared.0 -= 1;
        if shared.0 == 0
            && let Some(script) = shared.1.take()
        {
            let _ = std::fs::remove_dir_all(script.parent().unwrap());
        }
    }
}

/// A daemon that answers on the window's control socket: it writes down each
/// request line and replies with what `reply` makes of it.
pub(super) struct Socket {
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Socket {
    pub fn listen(win: &Win, reply: impl Fn(&str) -> String + Send + 'static) -> Socket {
        let path = one_mk_ctl::client::unix_socket_path(&win.dir);
        let listener =
            std::os::unix::net::UnixListener::bind(&path).expect("the window's private folder takes a socket");
        listener.set_nonblocking(true).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, off) = (seen.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !off.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_ok() {
                    let line = line.trim().to_owned();
                    let answer = reply(&line);
                    log.lock().unwrap().push(line);
                    let _ = stream.write_all(answer.as_bytes());
                }
            }
        });
        Socket { seen, stop, thread: Some(thread) }
    }

    /// Every request line so far.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// The requests that start with `word`.
    pub fn seen_with(&self, word: &str) -> Vec<String> {
        self.seen().into_iter().filter(|l| l.starts_with(word)).collect()
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Win {
    /// Frames (with a short sleep between, for the threads the window
    /// started) until `done` says so; panics with what is on screen if not.
    pub fn wait(&mut self, what: &str, done: impl Fn(&Win) -> bool) {
        for _ in 0..600 {
            self.frames(2);
            if done(self) {
                self.settle();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!(
            "waited 6 s for {what}; on screen: {:#?}; toast: {:?}",
            self.names(),
            self.app.toast.as_ref().map(|t| &t.text)
        );
    }

    /// Waits until the window has nothing running.
    pub fn quiet(&mut self) {
        self.wait("every command to finish", |w| w.app.busy.is_empty());
    }

    /// Tells the window what the settings files say now.
    pub fn prefs_changed(&mut self) {
        let prefs = self.app.link.prefs();
        self.send(Event::Prefs(Box::new(prefs)));
    }

    /// Writes `settings.toml` and tells the window.
    pub fn settings(&mut self, text: &str) {
        std::fs::write(self.dir.join("settings.toml"), text).unwrap();
        self.prefs_changed();
    }

    pub fn texts_joined(&self) -> String {
        self.texts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" | ")
    }

    pub fn toast_text(&self) -> String {
        self.app.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
    }
}

/// The keys of `settings.toml`'s `[input]` the window reads for a rule.
pub(super) fn rule_toml(rows: &[&str]) -> String {
    let mut text = String::new();
    for row in rows {
        text.push_str("[[input.keymap.rule]]\n");
        text.push_str(row);
        text.push('\n');
    }
    text
}

// ---------- Keys & mouse ----------

const CAPS: &str = "Caps Lock: unchanged. Arrows move between keys, Enter zooms in to change it";
const KB: &str = "usb/046d:c31c//Travel Keyboard";

fn keys_window() -> (Win, Fake) {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.go(Tab::Keys);
    (win, fake)
}

fn select_caps(win: &mut Win) {
    win.click(CAPS);
    win.settle();
    assert_eq!(win.app.keys.selected, Some(58), "{:#?}", win.names());
}

pub(super) fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn clicking_a_key_opens_its_panel_and_done_closes_it() {
    let (mut win, _fake) = keys_window();
    assert!(win.has_text("No keys are changed yet"), "{}", win.texts_joined());
    select_caps(&mut win);
    for name in [
        "Press the key Caps Lock should become",
        "Choose what Caps Lock becomes from a list",
        "Done with Caps Lock",
        "More for Caps Lock: no output, reset",
    ] {
        assert!(win.find(name).is_some(), "{name}: {:#?}", win.names());
    }
    assert!(win.find_containing("Caps Lock: Types Caps Lock, as it is.").is_some(), "{:#?}", win.names());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    win.click("Done with Caps Lock");
    assert_eq!(win.app.keys.selected, None);
    assert!(win.find("Press the key Caps Lock should become").is_none());
}

#[test]
fn zoomed_in_on_a_key_every_key_still_has_an_area_a_screen_reader_can_place() {
    let (mut win, _fake) = keys_window();
    select_caps(&mut win);
    let keys: Vec<_> = win.items().into_iter().filter(|i| i.name.contains("Arrows move between keys")).collect();
    assert!(keys.len() > 80, "{}", keys.len());
    let inverted: Vec<_> = keys
        .iter()
        .filter(|k| k.rect.min.x > k.rect.max.x || k.rect.min.y > k.rect.max.y)
        .map(|k| (&k.name, k.rect))
        .collect();
    assert!(inverted.is_empty(), "keys with an inverted area: {inverted:#?}");
}

#[test]
fn pressing_a_new_key_saves_the_rule_and_escape_cancels() {
    let (mut win, fake) = keys_window();
    select_caps(&mut win);
    win.click("Press the key Caps Lock should become");
    assert!(win.app.keys.capture.is_some());
    assert!(win.has_text("Press the key Caps Lock should become…"), "{}", win.texts_joined());
    assert!(win.app.keys_grab(&win.ctx), "the window's own shortcuts wait while a key is pressed for a rule");
    win.press(Key::Escape);
    assert!(win.app.keys.capture.is_none(), "Esc cancels");
    assert!(fake.calls_with(&["keymap", "add"]).is_empty(), "nothing was saved");
    win.click("Press the key Caps Lock should become");
    win.press(Key::A);
    assert!(win.app.keys.capture.is_none());
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "add"]), vec![args(&["keymap", "add", "--", "capslock", "a"])]);
    // The rules are checked after a change.
    assert!(!fake.calls_with(&["keymap", "check"]).is_empty());
    assert!(win.app.keys.error.is_none());
}

#[test]
fn a_rule_the_command_refuses_says_so_next_to_the_key() {
    let (mut win, fake) = keys_window();
    fake.fail("keymap-add", "one-mk: no such key: bogus");
    select_caps(&mut win);
    win.click("Press the key Caps Lock should become");
    win.press(Key::A);
    win.wait("the refusal", |w| w.app.keys.error.is_some());
    let said = win.app.keys.error.clone().unwrap();
    assert_eq!(said.0, "keyboard");
    assert_eq!(said.1, "Couldn't change Caps Lock: no such key: bogus");
    assert!(win.find_containing("Couldn't change Caps Lock: no such key: bogus").is_some(), "{:#?}", win.names());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    // Choosing again clears it.
    fake.mend("keymap-add");
    win.click("Press the key Caps Lock should become");
    win.press(Key::A);
    win.quiet();
    assert!(win.app.keys.error.is_none());
}

#[test]
fn the_list_finds_a_key_by_name_and_enter_takes_the_first() {
    let (mut win, fake) = keys_window();
    select_caps(&mut win);
    win.click("Choose what Caps Lock becomes from a list");
    assert!(win.app.keys.picker.is_some());
    win.type_text("escape");
    win.press(Key::Enter);
    win.quiet();
    let adds = fake.calls_with(&["keymap", "add"]);
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0][..3], ["keymap", "add", "--"]);
    assert_eq!(adds[0][3], "capslock");
    assert!(win.app.keys.picker.is_none());
}

#[test]
fn a_list_with_nothing_found_says_so_and_cancel_closes_it() {
    let (mut win, fake) = keys_window();
    select_caps(&mut win);
    win.click("Choose what Caps Lock becomes from a list");
    win.type_text("zzzzqqq");
    assert!(win.has_text("Nothing is called that."), "{}", win.texts_joined());
    win.press(Key::Enter);
    assert!(fake.calls_with(&["keymap", "add"]).is_empty());
    win.click("Close the list");
    assert!(win.app.keys.picker.is_none());
}

#[test]
fn no_output_makes_the_key_do_nothing_and_reset_puts_it_back() {
    let (mut win, fake) = keys_window();
    select_caps(&mut win);
    win.click("More for Caps Lock: no output, reset");
    win.click("Make Caps Lock do nothing");
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "add"]), vec![args(&["keymap", "add", "--", "capslock", "none"])]);
    // The file now has the rule: the panel offers Reset.
    win.settings(&rule_toml(&["from = \"capslock\"\nto = \"none\""]));
    win.click("More for Caps Lock: no output, reset");
    win.click("Make Caps Lock type itself again");
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "remove"]), vec![args(&["keymap", "remove", "--", "capslock"])]);
}

#[test]
fn the_changed_keys_list_reads_the_file_and_edit_and_remove_work() {
    let (mut win, fake) = keys_window();
    win.settings(&rule_toml(&[
        "from = \"capslock\"\nto = \"esc\"",
        "from = \"rightalt\"\nto = \"lctrl\"\nscope = \"mac\"",
    ]));
    assert!(win.find("Changed keys (2)").is_some() || win.has_text("Changed keys (2)"), "{:#?}", win.names());
    let rows: Vec<_> =
        win.items().into_iter().filter(|i| i.role == accesskit::Role::ListItem).map(|i| i.name).collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    // The edit pencil opens the key.
    let edit = win
        .items()
        .into_iter()
        .find(|i| i.name.starts_with("Change: ") && i.name.contains("Caps Lock"))
        .unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(edit.rect.center());
    assert_eq!(win.app.keys.selected, Some(58));
    let remove =
        win.items().into_iter().find(|i| i.name.starts_with("Remove: ") && i.name.contains("Caps Lock")).unwrap();
    win.click_at(remove.rect.center());
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "remove"]), vec![args(&["keymap", "remove", "--", "capslock"])]);
    // A refused removal says so in the list, and the rule is still there.
    fake.fail("keymap-remove", "one-mk: settings.toml is read-only");
    let remove =
        win.items().into_iter().find(|i| i.name.starts_with("Remove: ") && i.name.contains("Caps Lock")).unwrap();
    win.click_at(remove.rect.center());
    win.wait("the refusal", |w| w.app.keys.error.as_ref().is_some_and(|e| e.0 == "list"));
    assert!(win.has_text("settings.toml is read-only"), "{}", win.texts_joined());
}

#[test]
fn problems_from_the_rule_check_are_listed_in_plain_words() {
    let (mut win, _fake) = keys_window();
    win.settings(&rule_toml(&["from = \"capslock\"\nto = \"esc\""]));
    win.send(Event::KeymapCheck(Ok(vec!["rule 1: two rules for capslock".into()])));
    assert!(win.has_text("capslock"), "{}", win.texts_joined());
    win.send(Event::KeymapCheck(Err("the check did not run".into())));
    assert!(win.has_text("The rules couldn't be checked: the check did not run"), "{}", win.texts_joined());
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

fn two_keyboards() -> String {
    format!(
        r#"{{"available": true, "note": "per-device rules are untested here", "devices": [
        {{"fingerprint": "{KB}", "name": "Travel Keyboard", "bus": "usb", "connection": "USB", "keyboard": true, "pointer": false, "connected": true}},
        {{"fingerprint": "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Desk Mouse", "name": "Desk Mouse", "bus": "bluetooth", "connection": "Bluetooth", "keyboard": false, "pointer": true, "connected": false}},
        {{"fingerprint": "virtual/0000:0000//XTEST", "name": "XTEST", "bus": "virtual", "connection": "", "keyboard": true, "pointer": true, "connected": true}}]}}"#
    )
}

#[test]
fn a_rule_for_one_keyboard_is_added_and_removed_for_that_keyboard() {
    let (mut win, fake) = keys_window();
    fake.prints("input-devices", &two_keyboards());
    win.click("For: All computers. Choose which computers the changes are for");
    win.wait("the keyboards", |w| w.app.keys.devices.is_some());
    assert!(win.find_containing("Travel Keyboard").is_some(), "{:#?}", win.names());
    assert!(win.find_containing("Desk Mouse").is_some());
    assert!(
        win.find_containing("XTEST").is_none(),
        "a device made by software is no one's keyboard: {:#?}",
        win.names()
    );
    assert!(
        win.has_text("Try your rule in the test box before using it on another computer."),
        "{}",
        win.texts_joined()
    );
    win.click("For: Travel Keyboard, Connected · USB keyboard");
    win.settle();
    assert_eq!(win.app.keys.scope, crate::tune::Scope::Device(KB.into(), Box::new(crate::tune::Scope::All)));
    assert!(
        win.find("For: Only Travel Keyboard. Choose which computers the changes are for").is_some(),
        "{:#?}",
        win.names()
    );
    select_caps(&mut win);
    assert!(win.has_text("Changes here are only for keys typed on Travel Keyboard."), "{}", win.texts_joined());
    win.click("Choose what Caps Lock becomes from a list");
    win.type_text("escape");
    win.press(Key::Enter);
    win.quiet();
    let adds = fake.calls_with(&["keymap", "add"]);
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0][2], format!("--device={KB}"), "{adds:?}");
    // The file now has it, and the list names the rule.
    win.settings(&format!(
        "[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"esc\"\n\n[input.keymap.rule.scope]\ndevice = \"{KB}\"\n"
    ));
    let remove = win
        .items()
        .into_iter()
        .find(|i| i.name.starts_with("Remove: "))
        .unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(remove.rect.center());
    win.quiet();
    let gone = fake.calls_with(&["keymap", "remove"]);
    assert_eq!(
        gone,
        vec![vec!["keymap".to_owned(), "remove".into(), format!("--device={KB}"), "--".into(), "capslock".into()]]
    );
}

#[test]
fn the_keyboard_list_says_why_it_is_empty_or_failed() {
    let (mut win, fake) = keys_window();
    let open = |win: &mut Win| win.click("For: All computers. Choose which computers the changes are for");
    // A desktop that mixes keyboards together (Wayland).
    fake.prints("input-devices", r#"{"available": false, "note": "per-device rules are not available on this desktop: the desktop merges all keyboards and mice before oneMK sees them", "devices": []}"#);
    open(&mut win);
    win.wait("the answer", |w| w.app.keys.devices.is_some());
    assert!(win.has_text("Not on this desktop: it mixes all keyboards and mice together"), "{}", win.texts_joined());
    win.press(Key::Escape);
    // Nothing seen yet.
    fake.prints("input-devices", r#"{"available": true, "devices": []}"#);
    open(&mut win);
    win.wait("the answer", |w| matches!(&w.app.keys.devices, Some(Ok(d)) if d.available));
    assert!(win.has_text("None seen yet."), "{}", win.texts_joined());
    win.press(Key::Escape);
    // The command fails.
    fake.fail("input-devices", "one-mk: cannot read /dev/input");
    open(&mut win);
    win.wait("the failure", |w| matches!(&w.app.keys.devices, Some(Err(_))));
    assert!(win.has_text("Couldn't list the keyboards and mice: cannot read /dev/input"), "{}", win.texts_joined());
    // The scope stays all computers throughout.
    assert_eq!(win.app.keys.scope, crate::tune::Scope::All);
}

#[test]
#[cfg(target_os = "linux")]
fn on_a_desktop_that_cannot_change_its_own_keys_the_switch_says_so() {
    // Linux: nobody paired, so the page explains why the rule has no effect here.
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(alone()));
    win.go(Tab::Keys);
    assert!(win.has_text("On this computer"), "{}", win.texts_joined());
    assert!(win.has_text("Not available on this desktop yet"), "{}", win.texts_joined());
    let switch = win.find("Also on this computer").unwrap_or_else(|| panic!("{:#?}", win.names()));
    assert!(switch.disabled, "it cannot be switched on here");
    // A first rule on Linux with nobody paired does not also ask for local rules.
    win.click(CAPS);
    win.click("Press the key Caps Lock should become");
    win.press(Key::A);
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "local"]), Vec::<Vec<String>>::new());
    assert_eq!(fake.calls_with(&["keymap", "add"]).len(), 1);
}

fn mouse_window() -> (Win, Fake) {
    let (mut win, fake) = keys_window();
    win.click("Mouse: buttons, pointer and scrolling");
    win.settle();
    assert_eq!(win.app.keys.view, keys::View::Mouse);
    (win, fake)
}

#[test]
fn a_mouse_button_becomes_a_key_from_the_list_a_computer_switch_or_a_command() {
    let (mut win, fake) = mouse_window();
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
    win.click("Back (side): Unchanged. Change it");
    win.settle();
    assert!(win.find("Done with Back (side)").is_some(), "{:#?}", win.names());
    // A key, from the list.
    win.click("Back (side): A key or shortcut");
    win.click("Back (side): choose what it does from a list");
    win.type_text("escape");
    win.press(Key::Enter);
    win.quiet();
    let maps = fake.calls_with(&["buttons", "map"]);
    assert_eq!(maps, vec![args(&["buttons", "map", "--", "4", "esc"])], "{:?}", fake.calls());
    // Switching computers.
    win.click("Back (side): Switch computer");
    win.click("Office PC");
    win.quiet();
    assert_eq!(
        fake.calls_with(&["buttons", "map"]).last(),
        Some(&args(&["buttons", "map", "--", "4", "switch Office PC"]))
    );
    // A command.
    win.click("Back (side): Run a command");
    win.click_at(win.find_containing("Back (side): a command to run").unwrap().rect.center());
    win.type_text("notify-send hi");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(
        fake.calls_with(&["buttons", "map"]).last(),
        Some(&args(&["buttons", "map", "--", "4", "run: notify-send hi"]))
    );
    assert_eq!(fake.calls_with(&["buttons", "map"]).len(), 3);
}

#[test]
fn the_pointer_and_scrolling_controls_run_their_commands() {
    let (mut win, fake) = mouse_window();
    win.click("Shift scrolls sideways");
    win.quiet();
    win.click("Reverse up and down");
    win.quiet();
    win.click_at(win.find("Pointer speed, from 0.1 to 10 times").unwrap().rect.center());
    win.shortcut(Key::A);
    win.type_text("2");
    win.press(Key::Enter);
    win.quiet();
    let calls: Vec<Vec<String>> = fake.calls().into_iter().filter(|c| c[0] != "keymap").collect();
    assert_eq!(
        calls,
        vec![
            args(&["scroll", "--modifier=shift=horizontal"]),
            args(&["scroll", "--reverse-vertical=on"]),
            args(&["pointer", "--speed=2"])
        ]
    );
}

#[test]
fn the_first_rule_with_nobody_paired_switches_local_rules_on_where_that_works() {
    // The decision itself, for the systems that can (macOS, Windows): the
    // first rule of a computer on its own also applies on it; never after
    // the first, never with a computer paired, never when already on.
    use keys::solo_rules_start_with as start;
    assert!(start(true, Some(0), false, 0));
    assert!(!start(false, Some(0), false, 0), "Linux cannot");
    assert!(!start(true, Some(1), false, 0), "a computer is paired: the rule goes there");
    assert!(!start(true, None, false, 0), "oneMK is not running");
    assert!(!start(true, Some(0), true, 0), "already on");
    assert!(!start(true, Some(0), false, 1), "not the first rule");
}

// ---------- layout profiles and the Wi-Fi they come on by ----------

fn profiles_window(profiles: &[&str], current: Option<&str>) -> (Win, Fake) {
    let mut win = Win::light(1400.0, 1000.0);
    let fake = Fake::install(&mut win);
    let mut status = two();
    status.layout.profiles = profiles.iter().map(|p| (*p).to_owned()).collect();
    status.layout.profile = current.map(str::to_owned);
    win.status(Some(status));
    win.go(Tab::Arrange);
    (win, fake)
}

fn type_in(win: &mut Win, field: &str, text: &str) {
    let item = win.find(field).unwrap_or_else(|| panic!("no field {field:?}: {:#?}", win.names()));
    win.click_at(item.rect.center());
    win.type_text(text);
}

#[test]
fn a_profile_is_saved_under_a_name_and_the_name_stays_when_saving_fails() {
    let (mut win, fake) = profiles_window(&[], None);
    assert!(win.has_text("Save where every computer is now, to switch back in one click."), "{}", win.texts_joined());
    assert!(win.find("Save this layout as a profile").unwrap().disabled, "no name, no save");
    type_in(&mut win, "Name for a new profile of this layout", "  Desk and Laptop ");
    win.click("Save this layout as a profile");
    win.quiet();
    assert_eq!(fake.calls_with(&["profile"]), vec![args(&["profile", "save", "Desk and Laptop"])]);
    assert_eq!(win.toast_text(), "Saved this layout as “Desk and Laptop”");
    assert!(win.app.drafts.get("profile-new").is_none_or(String::is_empty), "the field is empty again");
    // It fails: the toast says so and the name is back in the field to be fixed.
    fake.fail("profile-save", "one-mk: a profile needs at least one paired computer");
    type_in(&mut win, "Name for a new profile of this layout", "Second");
    win.press(Key::Enter);
    win.wait("the failure", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't save this layout as “Second”.");
    assert_eq!(win.app.drafts.get("profile-new").map(String::as_str), Some("Second"));
}

#[test]
fn saving_needs_a_paired_computer() {
    let mut win = Win::light(1400.0, 1000.0);
    let fake = Fake::install(&mut win);
    win.status(Some(alone()));
    win.go(Tab::Arrange);
    assert!(win.has_text("Once computers are paired") || win.has_text("Pair"), "{}", win.texts_joined());
    if let Some(button) = win.find("Save this layout as a profile") {
        assert!(button.disabled);
    }
    assert!(fake.calls().is_empty());
}

#[test]
fn a_profile_is_used_and_the_one_in_use_says_so() {
    let (mut win, fake) = profiles_window(&["Desk", "Laptop"], Some("Desk"));
    assert!(win.has_text("In use"), "{}", win.texts_joined());
    assert!(win.find("Use profile Desk").is_none(), "the one in use has no Use button");
    win.click("Use profile Laptop");
    win.quiet();
    assert_eq!(fake.calls_with(&["profile"]), vec![args(&["profile", "use", "Laptop"])]);
    assert_eq!(win.toast_text(), "Using the layout “Laptop”");
    fake.fail("profile-use", "one-mk: Office PC is not connected");
    win.click("Use profile Laptop");
    win.wait("the failure", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't switch to “Laptop”.");
    assert!(problems(&win).is_empty(), "{:#?}", problems(&win));
}

#[test]
fn deleting_a_profile_asks_first_and_cancel_deletes_nothing() {
    let (mut win, fake) = profiles_window(&["Desk", "Laptop"], None);
    win.click("More for profile Laptop");
    win.click("Delete profile Laptop");
    assert!(win.app.sheet_open());
    assert!(win.has_text("Delete the layout “Laptop”?"), "{}", win.texts_joined());
    assert!(fake.calls().is_empty(), "nothing yet");
    win.click("Cancel");
    assert!(!win.app.sheet_open());
    assert!(fake.calls_with(&["profile"]).is_empty());
    win.click("Delete profile Laptop");
    win.click("Delete");
    win.quiet();
    assert_eq!(fake.calls_with(&["profile"]), vec![args(&["profile", "delete", "Laptop"])]);
    assert_eq!(win.toast_text(), "Deleted the layout “Laptop”");
    fake.fail("profile-delete", "one-mk: no such profile");
    win.settings("");
    win.click("More for profile Desk");
    win.click("Delete profile Desk");
    win.click("Delete");
    win.wait("the failure", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't delete “Desk”.");
}

#[test]
fn a_profile_comes_on_by_itself_on_a_wifi_network_and_that_can_be_turned_off() {
    let (mut win, fake) = profiles_window(&["Desk", "Laptop"], None);
    win.click("More for profile Laptop");
    assert!(win.find("Switch to Laptop by itself on this Wi-Fi").unwrap().disabled, "no network name yet");
    type_in(&mut win, "Wi-Fi network that switches to Laptop by itself", "Home WiFi");
    win.click("Switch to Laptop by itself on this Wi-Fi");
    win.quiet();
    assert_eq!(fake.calls_with(&["profile"]), vec![args(&["profile", "auto", "Laptop", "--ssid", "Home WiFi"])]);
    assert_eq!(win.toast_text(), "“Laptop” comes on by itself on Wi-Fi “Home WiFi”");
    // The file now says so: the row says when, and Turn off is offered.
    std::fs::write(win.dir.join("profiles.toml"), "[[profile]]\nname = \"Laptop\"\nssid = [\"Home WiFi\"]\n").unwrap();
    win.prefs_changed();
    assert_eq!(win.app.prefs.auto.get("Laptop").map(String::as_str), Some("Wi-Fi “Home WiFi”"));
    assert!(win.has_text("Comes on by itself on Wi-Fi “Home WiFi”"), "{}", win.texts_joined());
    win.click("Stop switching to Laptop by itself");
    win.quiet();
    assert_eq!(fake.calls_with(&["profile", "auto"]).last(), Some(&args(&["profile", "auto", "Laptop", "--off"])));
    assert_eq!(win.toast_text(), "“Laptop” no longer comes on by itself");
    fake.fail("profile-auto", "one-mk: that network name is too long");
    type_in(&mut win, "Wi-Fi network that switches to Laptop by itself", "Cafe");
    win.press(Key::Enter);
    win.wait("the failure", |w| w.app.toast.as_ref().is_some_and(|t| t.error));
    assert_eq!(win.toast_text(), "Couldn't change when “Laptop” comes on by itself.");
}
