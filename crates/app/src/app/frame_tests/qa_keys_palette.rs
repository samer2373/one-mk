//! Flows that went wrong in the command palette and on the Keys & mouse
//! page: each presses what a person presses and reads back what the window
//! shows and what it asked `one-mk` to do (`qa.rs` has the stand-ins).

use super::qa::{Fake, args};
use super::*;
use crate::tune::Scope;
use one_mk_ctl::update_logic::UpdateStatus;

fn window(status: Status) -> (Win, Fake) {
    window_of(status, 900.0)
}

fn window_of(status: Status, height: f32) -> (Win, Fake) {
    let mut win = Win::light(1200.0, height);
    let fake = Fake::install(&mut win);
    win.status(Some(status));
    (win, fake)
}

fn open_palette(win: &mut Win) {
    win.shortcut(Key::K);
    assert!(win.app.palette.open);
}

// ---------- the palette ----------

#[test]
fn the_palette_offers_no_page_the_window_would_turn_away_while_one_mk_is_off() {
    let mut win = Win::light(1200.0, 900.0);
    win.status(None);
    open_palette(&mut win);
    win.type_text("arrange");
    assert!(win.has_text("Nothing found"), "{:#?}", win.names());
    assert!(win.find("Arrange computers").is_none());
    // Pages that work while it is off are still there; running, Arrange is too.
    win.app.palette.query = "wallpaper".into();
    win.frames(4);
    assert!(win.find("Wallpaper").is_some(), "{:#?}", win.names());
    win.shortcut(Key::K);
    win.status(Some(two()));
    open_palette(&mut win);
    win.type_text("arrange");
    assert!(win.find("Arrange computers").is_some(), "{:#?}", win.names());
}

#[test]
fn the_palette_offers_the_clipboard_only_where_sharing_is_on() {
    let mut status = two();
    status.clipboard_off = true;
    let mut win = Win::light(1200.0, 900.0);
    win.status(Some(status));
    open_palette(&mut win);
    win.type_text("clipboard");
    assert!(win.find_containing("Send the clipboard to").is_none() && win.find_containing("Get the clipboard from").is_none(), "{:#?}", win.names());
    assert!(win.find("Switch to Office PC").is_none(), "only clipboard words were typed");
    win.shortcut(Key::K);
    win.status(Some(two()));
    open_palette(&mut win);
    win.type_text("clipboard");
    assert!(win.find("Send the clipboard to Office PC").is_some(), "{:#?}", win.names());
    assert!(win.find("Get the clipboard from Office PC").is_some());
}

#[test]
fn the_palette_offers_an_update_only_where_it_can_be_installed_from_here() {
    let status = |can_update: bool, move_first: bool| UpdateStatus {
        check: true, can_update, move_to_applications: move_first, available: Some("9.9.9".into()), ..Default::default()
    };
    let mut win = Win::light(1200.0, 900.0);
    win.status(Some(two()));
    for (can, move_first, offered) in [(true, false, true), (false, false, false), (true, true, false)] {
        win.send(Event::UpdateStatus(status(can, move_first)));
        open_palette(&mut win);
        win.type_text("Update oneMK");
        assert_eq!(win.find("Update oneMK to 9.9.9").is_some(), offered, "can_update {can}, move first {move_first}: {:#?}", win.names());
        win.press(Key::Escape);
    }
}

#[test]
fn check_health_again_starts_one_check() {
    let (mut win, fake) = window(two());
    assert_eq!(win.tab(), Tab::Desk);
    open_palette(&mut win);
    win.type_text("Check health again");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(win.tab(), Tab::Health);
    assert_eq!(fake.calls_with(&["doctor"]).len(), 1, "{:?}", fake.calls());
    // From Health itself, with a fresh check at hand, it still checks again, once.
    open_palette(&mut win);
    win.type_text("Check health again");
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(fake.calls_with(&["doctor"]).len(), 2, "{:?}", fake.calls());
}

#[test]
fn a_click_that_closes_the_palette_does_not_also_press_what_lies_under_it() {
    let (mut win, _fake) = window(two());
    open_palette(&mut win);
    win.frames(12);
    let tab = win.find("Arrange").unwrap_or_else(|| panic!("{:#?}", win.names()));
    win.click_at(tab.rect.center());
    assert!(!win.app.palette.open, "a click on the dimmed window closes it");
    assert_eq!(win.tab(), Tab::Desk, "and goes no further");
}

#[test]
fn return_does_what_the_letters_typed_in_the_same_frame_found() {
    let (mut win, _fake) = window(two());
    open_palette(&mut win);
    win.frames(4);
    // The letters and Return reach the window together (a fast typist, a paste).
    win.push(egui::Event::Text("Settings".into()));
    win.push(egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    win.frames(2);
    win.settle();
    assert!(!win.app.palette.open);
    assert_eq!(win.tab(), Tab::Settings);
}

#[test]
fn the_choice_stays_on_its_entry_when_the_computers_change_under_it() {
    let online = |ids: &[(&str, &str)]| with(ids.iter().map(|(id, name)| peer(id, name, Side::Right, os::WINDOWS, true)).collect());
    let (mut win, _fake) = window(online(&[("p1", "Desk 1"), ("p2", "Desk 2"), ("p3", "Desk 3")]));
    open_palette(&mut win);
    win.type_text("Switch to Desk");
    win.press(Key::ArrowDown);
    assert_eq!(win.app.palette_chosen().as_deref(), Some("Computers / Switch to Desk 2"));
    // The first computer goes away: the choice does not slide to the next one.
    win.status(Some(online(&[("p2", "Desk 2"), ("p3", "Desk 3")])));
    win.frames(4);
    assert_eq!(win.app.palette_chosen().as_deref(), Some("Computers / Switch to Desk 2"));
}

/// Whether the frame asked the input method to break off its composition.
fn breaks_composition(win: &mut Win) -> bool {
    win.t += 1.0 / 60.0;
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, win.size)),
        time: Some(win.t),
        predicted_dt: 1.0 / 60.0,
        focused: true,
        ..Default::default()
    };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(win.ppp);
    let (app, frame) = (&mut win.app, &mut win.frame);
    let out = win.ctx.run_ui(input, |ui| eframe::App::ui(app, ui, frame));
    out.platform_output.ime.is_some_and(|ime| ime.should_interrupt_composition)
}

#[test]
fn the_palette_does_not_break_off_an_input_methods_composition_every_frame() {
    let (mut win, _fake) = window(two());
    open_palette(&mut win);
    win.frames(6);
    for _ in 0..3 {
        assert!(!breaks_composition(&mut win));
    }
}

// ---------- Keys & mouse ----------

const CAPS: &str = "Caps Lock: unchanged. Arrows move between keys, Enter zooms in to change it";

fn keys_window(status: Status, settings: &str) -> (Win, Fake) {
    keys_window_of(status, settings, 900.0)
}

fn keys_window_of(status: Status, settings: &str, height: f32) -> (Win, Fake) {
    let (mut win, fake) = window_of(status, height);
    win.settings(settings);
    win.go(Tab::Keys);
    (win, fake)
}

/// The mouse page, in a window tall enough to show all of it (autoscroll and gestures are far down).
fn mouse_window(status: Status, settings: &str) -> (Win, Fake) {
    let (mut win, fake) = keys_window_of(status, settings, 1600.0);
    win.click("Mouse: buttons, pointer and scrolling");
    win.settle();
    (win, fake)
}

#[test]
fn closing_when_held_cancels_its_recorder() {
    let (mut win, fake) = keys_window(two(), "");
    win.click(CAPS);
    win.click("More: a different key when held");
    win.click("Press the key Caps Lock gives when held");
    assert!(matches!(win.app.keys.capture, Some((keys::Want::Hold, _))));
    // Collapsed without recording anything.
    win.click("More: a different key when held");
    assert!(win.app.keys.capture.is_none(), "nothing is waiting for a key any more");
    win.press(Key::A);
    win.quiet();
    assert!(fake.calls_with(&["keymap", "add"]).is_empty(), "{:?}", fake.calls());
    // Even when it is left running some other way, a hidden recorder takes nothing.
    win.app.keys.capture = Some((keys::Want::Hold, crate::tune::Capture::default()));
    win.app.keys.more = false;
    win.press(Key::A);
    win.quiet();
    assert!(win.app.keys.capture.is_none());
    assert!(fake.calls_with(&["keymap", "add"]).is_empty(), "{:?}", fake.calls());
}

const SEQUENCE: &str = "[[input.keymap.rule]]\nfrom = \"a\"\nto = [\"ctrl+a\", \"ctrl+c\"]\n";

#[test]
fn a_hold_is_not_added_to_a_sequence_without_saying_so() {
    let (mut win, fake) = keys_window(two(), SEQUENCE);
    win.app.select_key(30, None);
    win.settle();
    win.app.choose(keys::Want::Hold, "shift".into());
    win.quiet();
    assert!(fake.calls_with(&["keymap", "add"]).is_empty(), "the chain is not replaced: {:?}", fake.calls());
    let said = win.app.keys.error.clone().expect("it says why");
    assert!(said.1.contains("sequence"), "{said:?}");
}

// (F18 to F20 are not on the drawn keyboard, so a chip that says them is the panel's.)
const APP_RULE: &str = "[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"f20\"\n\n[[input.keymap.rule]]\nfrom = \"capslock\"\ntap = \"f19\"\nhold = \"f18\"\napp = \"Code\"\n";

#[test]
fn the_editor_shows_the_tap_and_hold_of_the_app_rule_being_changed() {
    let (mut win, _fake) = keys_window(two(), APP_RULE);
    win.app.select_key(58, None);
    win.app.keys.app = Some("Code".into());
    win.settle();
    let chips: Vec<&str> = win.texts.iter().map(|t| t.text.as_str()).collect();
    assert!(chips.contains(&"F19") && chips.contains(&"F18"), "{chips:?}");
    assert!(!chips.contains(&"F20"), "{chips:?}");
}

#[test]
fn adding_a_hold_keeps_the_tap_the_key_inherits() {
    let (mut win, fake) = keys_window(two(), "[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"f1\"\n");
    win.app.keys.scope = Scope::Os(one_mk_proto::os::LINUX);
    win.app.select_key(58, None);
    win.settle();
    win.app.choose(keys::Want::Hold, "shift".into());
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "add"]), vec![args(&["keymap", "add", "--tap=f1", "--hold=shift", "--os=linux", "--", "capslock"])], "{:?}", fake.calls());
}

#[test]
fn escape_closes_the_list_and_it_takes_typing_each_time_it_opens() {
    let (mut win, _fake) = keys_window(two(), "");
    win.click(CAPS);
    win.click("Choose what Caps Lock becomes from a list");
    assert!(win.app.keys.picker.is_some());
    win.press(Key::Escape);
    assert!(win.app.keys.picker.is_none(), "Esc closes the list");
    // Closed by choosing another key, then opened again: typing searches at once.
    win.click("Choose what Caps Lock becomes from a list");
    win.app.select_key(30, None);
    win.settle();
    win.click("Choose what A becomes from a list");
    win.type_text("esc");
    assert_eq!(win.app.keys.picker.as_ref().map(|p| p.1.as_str()), Some("esc"));
}

#[test]
fn the_picker_gives_search_focus_then_tabs_to_categories_and_applies_the_highlight() {
    let (mut win, fake) = keys_window(two(), "");
    win.app.keys.scope = Scope::Os(one_mk_proto::os::MACOS);
    win.app.select_key(58, None);
    win.settle();
    let open = "Choose what Caps Lock becomes from a list";
    let search = "Search for a key or an action";
    win.click(open);
    assert!(win.find(search).is_some_and(|item| item.focused));
    assert_eq!(win.find("Matching keys and actions").unwrap().role, egui::accesskit::Role::List);
    assert_eq!(win.find("Mission Control").unwrap().role, egui::accesskit::Role::ListItem);
    win.press(Key::Tab);
    assert!(win.find("Suggested").is_some_and(|item| item.focused), "{:#?}", win.names());
    win.press(Key::Escape);
    assert!(fake.calls_with(&["keymap", "add"]).is_empty());
    win.click(open);
    win.type_text("mision control");
    assert!(win.find("Mission Control").is_some());
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "add"]), vec![args(&["keymap", "add", "--os=mac", "--", "capslock", "os:overview"])]);

    win.click(open);
    win.type_text("workspace");
    win.press(Key::ArrowDown);
    win.press(Key::Enter);
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "add"]).last().and_then(|call| call.last()).map(String::as_str), Some("os:previous-desktop"));
}

#[test]
fn a_held_click_on_remove_goes_to_the_rule_it_was_pressed_on() {
    let rules = ["from = \"a\"\nto = \"b\"", "from = \"capslock\"\nto = \"esc\"", "from = \"f1\"\nto = \"f2\""];
    let (mut win, fake) = keys_window(two(), &super::qa::rule_toml(&rules));
    let remove = "Remove: Caps Lock → Esc";
    let row = win.find("Change: Caps Lock → Esc").unwrap_or_else(|| panic!("{:#?}", win.names())).rect;
    win.move_to(row.center());
    let at = win.find(remove).unwrap_or_else(|| panic!("{:#?}", win.names())).rect.center();
    win.move_to(at);
    win.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    win.frames(2);
    // Meanwhile the first rule goes (another window, the command line): the next one moves up.
    win.settings(&super::qa::rule_toml(&rules[1..]));
    win.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    win.frames(4);
    win.quiet();
    assert!(fake.calls_with(&["keymap", "remove"]).is_empty(), "{:?}", fake.calls());
}

#[test]
fn rules_with_alike_scopes_or_written_twice_each_have_their_own_controls() {
    // A computer named Macs and all Macs both read "Only Macs"; the third rule is written twice.
    let rules = [
        "from = \"a\"\nto = \"b\"\nscope = { peer = \"Macs\" }",
        "from = \"a\"\nto = \"b\"\nscope = { os = \"macos\" }",
        "from = \"f1\"\nto = \"f2\"",
        "from = \"f1\"\nto = \"f2\"",
    ];
    let status = with(vec![peer("p1", "Macs", Side::Right, one_mk_proto::os::MACOS, true)]);
    let (win, _fake) = keys_window(status, &super::qa::rule_toml(&rules));
    let removes = win.items().into_iter().filter(|i| i.name.starts_with("Remove: ")).count();
    assert_eq!(removes, rules.len(), "{:#?}", win.names());
}

#[test]
fn moving_a_rule_to_an_app_adds_the_new_rule_before_removing_the_old_one() {
    let (mut win, fake) = keys_window(two(), "[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"esc\"\n");
    fake.fail("keymap-add", "one-mk: refused");
    win.app.select_key(58, None);
    win.settle();
    win.click_at(win.find_containing("Only in app").unwrap_or_else(|| panic!("{:#?}", win.names())).rect.center());
    win.type_text("Code");
    win.press(Key::Enter);
    win.wait("the add", |_| !fake.calls_with(&["keymap", "add"]).is_empty());
    win.quiet();
    assert!(fake.calls_with(&["keymap", "remove"]).is_empty(), "the rule stays when its replacement is refused: {:?}", fake.calls());
}

#[test]
fn recording_a_gesture_stops_holding_the_shortcuts_once_it_is_left() {
    let (mut win, _fake) = mouse_window(two(), "");
    win.click("Press a key or shortcut for this gesture");
    assert!(win.ctx.data(|d| d.get_temp::<bool>(Id::unique("gesture-recording"))).unwrap_or(false), "recording");
    win.click("Desk");
    assert_eq!(win.tab(), Tab::Desk);
    win.shortcut(Key::K);
    assert!(win.app.palette.open, "the shortcuts work again on the page it went to");
}

#[test]
fn as_it_is_on_a_button_a_broader_rule_changes_sends_it_as_it_is_for_this_computer() {
    let (mut win, fake) = mouse_window(two(), "[[input.button]]\nfrom = 4\nto = \"esc\"\n");
    win.app.keys.scope = Scope::Peer("Office PC".into());
    win.settle();
    win.click("Back (side): Esc. Change it");
    win.click("Back (side): As it is");
    win.quiet();
    assert!(fake.calls_with(&["buttons", "unmap"]).is_empty(), "there is no rule of its own to remove: {:?}", fake.calls());
    assert_eq!(fake.calls_with(&["buttons", "map"]), vec![args(&["buttons", "map", "--peer=Office PC", "--", "4", "button4"])], "{:?}", fake.calls());
    assert!(win.app.keys.error.is_none());
}

#[test]
fn a_computer_named_here_is_switched_to_by_its_id() {
    let status = with(vec![peer("ab12-cd34", "Here", Side::Right, os::WINDOWS, true)]);
    let (mut win, fake) = mouse_window(status, "");
    win.click("Back (side): Unchanged. Change it");
    win.click("Back (side): Switch computer");
    win.click("Here");
    win.quiet();
    assert_eq!(fake.calls_with(&["buttons", "map"]), vec![args(&["buttons", "map", "--", "4", "switch ab12-cd34"])], "{:?}", fake.calls());
}

#[test]
fn computers_with_one_name_are_chosen_and_saved_by_their_ids() {
    let status = with(vec![peer("ab12-cd34", "Studio (ab12-cd34)", Side::Right, os::WINDOWS, true), peer("ee00-ff11", "Studio (ee00-ff11)", Side::Left, os::WINDOWS, true)]);
    let (mut win, fake) = keys_window(status, "");
    win.click("For: All computers. Choose which computers the changes are for");
    win.click("For: Only Studio (ab12-cd34)");
    assert_eq!(win.app.keys.scope, Scope::Peer("ab12-cd34".into()));
    assert!(win.has_text("For: Only Studio (ab12-cd34)"), "{}", win.texts_joined());
    win.click(CAPS);
    win.click("Press the key Caps Lock should become");
    win.press(Key::A);
    win.quiet();
    assert_eq!(fake.calls_with(&["keymap", "add"]), vec![args(&["keymap", "add", "--peer=ab12-cd34", "--", "capslock", "a"])], "{:?}", fake.calls());
}

#[test]
fn a_speed_being_set_is_kept_for_its_own_computers_and_the_scrolling_kind() {
    let (mut win, _fake) = mouse_window(two(), "[[input.pointer]]\nspeed = 0.5\nscope = { peer = \"Office PC\" }\n");
    let all = win.app.speed_target("pointer-speed");
    win.app.keys.scope = Scope::Peer("Office PC".into());
    let peer = win.app.speed_target("pointer-speed");
    assert_ne!(all, peer, "writes queued for two computers are two changes");
    win.app.keys.scope = Scope::All;
    let wheel = {
        win.app.keys.device = Some(crate::tune::Device::Wheel);
        win.app.speed_target("scroll-speed")
    };
    win.app.keys.device = Some(crate::tune::Device::Touchpad);
    assert_ne!(wheel, win.app.speed_target("scroll-speed"), "nor are a wheel's and a touchpad's");
    win.app.keys.device = None;
    // A speed just set for all computers does not stay on show for the next computer.
    let key = win.app.speed_target("pointer-speed");
    win.app.keys.drafts.insert(key, 2.0);
    win.app.keys.scope = Scope::Peer("Office PC".into());
    win.settle();
    assert!(win.texts.iter().any(|t| t.text == "0.5"), "{}", win.texts_joined());
    assert!(!win.texts.iter().any(|t| t.text == "2"), "{}", win.texts_joined());
}

#[test]
fn nothing_is_offered_for_the_left_and_right_buttons() {
    let (mut win, _fake) = mouse_window(two(), "");
    win.click("Left button: Left click. Change it");
    win.click("Left button: A key or shortcut");
    win.click("Left button: choose what it does from a list");
    win.type_text("nothing");
    assert!(win.find("Nothing").is_none(), "{:#?}", win.names());
    // The back button may be turned off.
    win.click("Done with Left button");
    win.click("Back (side): Unchanged. Change it");
    win.click("Back (side): A key or shortcut");
    win.click("Back (side): choose what it does from a list");
    win.type_text("nothing");
    assert!(win.find("Nothing").is_some(), "{:#?}", win.names());
}

#[test]
fn the_side_swipe_setup_is_refused_where_autoscroll_takes_the_back_button() {
    let (mut win, fake) = mouse_window(two(), "[input.autoscroll]\nenabled = true\nbutton = \"button4\"\n");
    win.click("Set up side-button desktop swipes");
    win.quiet();
    assert!(fake.calls_with(&["gestures", "preset"]).is_empty(), "{:?}", fake.calls());
    let said = win.app.keys.error.clone().expect("it says why");
    assert!(said.1.contains("reserved by autoscroll"), "{said:?}");
}

#[test]
fn the_test_box_swaps_ctrl_and_cmd_for_a_mac_as_a_visit_does() {
    let status = with(vec![peer("m1", "MacBook", Side::Right, os::MACOS, true)]);
    let (mut win, _fake) = keys_window(status, "[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"lctrl\"\n");
    win.app.keys.scope = Scope::Peer("MacBook".into());
    assert_eq!(win.app.facts().swap, cfg!(not(target_os = "macos")), "Auto swaps between a Mac and the rest");
    // The peer's own setting decides.
    let mut off = with(vec![peer("m1", "MacBook", Side::Right, os::MACOS, true)]);
    off.peers[0].swap = "off".into();
    win.status(Some(off));
    assert!(!win.app.facts().swap);
}

#[test]
fn the_list_of_keyboards_is_asked_again_while_it_stays_open() {
    let (mut win, fake) = keys_window(two(), "");
    win.click("For: All computers. Choose which computers the changes are for");
    win.wait("the first list", |_| !fake.calls_with(&["input-devices"]).is_empty());
    let first = fake.calls_with(&["input-devices"]).len();
    // Left open, it looks again every couple of seconds (a keyboard may have been plugged in).
    win.wait("another look", |_| fake.calls_with(&["input-devices"]).len() > first);
}
