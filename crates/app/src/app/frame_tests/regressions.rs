//! Whole-window regressions for the Desk's drag, the file drop and the sheets:
//! each presses what a person presses (with the headless harness) and reads back
//! what the window did and what it asked `one-mk` to do.

use super::qa::Fake;
use super::*;

const OFFICE: &str = "Office PC, Windows, on your right";
const MAC: &str = "MacBook Pro, macOS, on your left";

/// Office PC on the right, a MacBook on the left: the MacBook is last in the scene's list.
fn two_sided() -> Status {
    with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, true), peer("p2", "MacBook Pro", Side::Left, os::MACOS, true)])
}

fn button(win: &mut Win, at: Pos2, pressed: bool) {
    win.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// Presses on the named computer and carries it `by` away in steps, button still down.
fn pick_up(win: &mut Win, name: &str, by: egui::Vec2) -> Pos2 {
    let from = win.find(name).unwrap_or_else(|| panic!("no {name:?}: {:#?}", win.names())).rect.center();
    win.move_to(from);
    button(win, from, true);
    win.frames(2);
    for k in 1..=8 {
        win.move_to(from + by * (k as f32 / 8.0));
    }
    from + by
}

fn release(win: &mut Win, at: Pos2) {
    button(win, at, false);
    win.frames(3);
}

fn dragged_id(win: &Win) -> Option<(String, usize)> {
    match &win.app.drag {
        Some(Drag::Device { id, i, .. }) => Some((id.clone(), *i)),
        _ => None,
    }
}

fn layouts(fake: &Fake) -> Vec<Vec<String>> {
    fake.calls_with(&["layout"])
}

#[test]
fn retrying_a_move_asks_for_the_same_exact_place() {
    let mut win = Win::light(1200.0, 780.0);
    let _fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.choose_place("p1", Side::Top, Some((100, 0)), false);
    let job = win.app.busy.iter().find(|j| matches!(j, Job::Layout { .. })).cloned().expect("the move is running");
    win.app.pending_side.clear();
    win.app.retry(job);
    assert_eq!(win.app.pending_side.get("p1").map(|(side, offset, _)| (*side, *offset)), Some((Side::Top, Some((100, 0)))), "the offset was dropped");
}

#[test]
fn removing_a_computer_during_a_drag_neither_crashes_nor_moves_another() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two_sided()));
    let at = pick_up(&mut win, MAC, vec2(-40.0, 0.0));
    assert_eq!(dragged_id(&win).map(|(id, _)| id), Some("p2".into()), "the MacBook is in hand");
    // The earlier computer is removed (and fades out) while the MacBook is held.
    win.status(Some(with(vec![peer("p2", "MacBook Pro", Side::Left, os::MACOS, true)])));
    assert_eq!(dragged_id(&win), Some(("p2".into(), 1)), "it is found again where it stands now");
    release(&mut win, at);
    win.quiet();
    assert!(layouts(&fake).iter().all(|call| call.contains(&"p2".to_owned())), "{:?}", layouts(&fake));
}

#[test]
fn the_computer_in_hand_going_away_puts_the_drag_down() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two_sided()));
    let at = pick_up(&mut win, MAC, vec2(-40.0, 0.0));
    win.status(Some(with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, true)])));
    assert!(win.app.drag.is_none() && win.app.drag_side.is_none(), "the drag is over");
    release(&mut win, at);
    win.quiet();
    assert!(layouts(&fake).is_empty(), "nothing is placed: {:?}", layouts(&fake));
}

#[test]
fn leaving_the_desk_while_dragging_leaves_no_drag_behind() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    let at = pick_up(&mut win, OFFICE, vec2(-300.0, 0.0));
    assert!(win.app.drag.is_some());
    win.go(Tab::Health);
    assert!(win.app.drag.is_none() && win.app.drag_side.is_none(), "the drag went with the page");
    release(&mut win, at);
    win.go(Tab::Desk);
    assert!(win.app.drag.is_none(), "still dragging after coming back");
    // And a drag that nothing holds (the release was never seen) is put down by the desk itself.
    win.app.drag = Some(Drag::Orbit);
    win.frames(2);
    assert!(win.app.drag.is_none());
}

#[test]
fn escape_while_dragging_cancels_the_move() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    let at = pick_up(&mut win, OFFICE, vec2(-400.0, 0.0));
    win.push(egui::Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    win.frames(3);
    release(&mut win, at);
    win.quiet();
    assert!(win.app.layout_asked.is_empty() && layouts(&fake).is_empty(), "Escape placed it: {:?}", layouts(&fake));
    assert!(win.app.drag.is_none() && win.app.drag_side.is_none());
    // The same drag, let go, does place it.
    let at = pick_up(&mut win, OFFICE, vec2(-400.0, 0.0));
    release(&mut win, at);
    win.quiet();
    assert!(!win.app.layout_asked.is_empty(), "the control drag placed nothing");
}

#[test]
fn a_drop_lands_where_the_pointer_was_let_go() {
    let mut win = Win::light(1200.0, 780.0);
    let _fake = Fake::install(&mut win);
    win.status(Some(two()));
    let from = win.find(OFFICE).expect("computer").rect.center();
    let _ = pick_up(&mut win, OFFICE, vec2(-400.0, 0.0));
    // In the frame of the release the pointer is back at the start: that is where it lands.
    win.push(egui::Event::PointerMoved(from));
    release(&mut win, from);
    win.quiet();
    assert_eq!(win.app.layout_asked.get("p1").map(|(side, _)| *side), Some(Side::Right), "{:?}", win.app.layout_asked);
}

#[test]
fn a_computer_that_drops_off_mid_drag_keeps_its_exact_place() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    // Placed exactly: its (0,0) is at 2560,0 in our pixels.
    let mut status = two();
    status.offsets = vec![("p1".into(), (2560, 0))];
    win.status(Some(status));
    let at = pick_up(&mut win, OFFICE, vec2(-40.0, 0.0));
    // It goes offline (no screens reported) while held; the saved place stays.
    let mut gone = with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, false)]);
    gone.offsets = vec![("p1".into(), (2560, 0))];
    win.status(Some(gone));
    release(&mut win, at);
    win.quiet();
    assert!(layouts(&fake).is_empty(), "its offset was cleared: {:?}", layouts(&fake));
}

#[test]
fn the_context_menu_closes_when_its_computer_leaves() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two_sided()));
    let at = win.find(OFFICE).expect("computer").rect.center();
    win.right_click_at(at);
    assert!(win.find_containing("On the left").is_some(), "{:#?}", win.names());
    // The one it was opened on is removed; the other stays and takes its place in the list.
    win.status(Some(with(vec![peer("p2", "MacBook Pro", Side::Left, os::MACOS, true)])));
    win.settle();
    assert!(win.find_containing("On the left").is_none(), "the menu is still open, for another computer: {:#?}", win.names());
    assert!(layouts(&fake).is_empty());
}

#[test]
fn the_drop_chooser_starts_on_cancel_even_when_its_buttons_stack() {
    let mut win = Win::light(700.0, 700.0);
    win.status(Some(crowded()));
    win.go(Tab::Settings);
    let file = win.dir.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    win.move_to(pos2(300.0, 300.0));
    win.drop_files(std::slice::from_ref(&file));
    win.settle();
    assert!(win.has_text("Send 1 item to which computer?"), "{:#?}", win.names());
    let cancel = win.find("Cancel").expect("Cancel");
    let recipient = win.find("Office PC").expect("a recipient");
    assert!(cancel.rect.top() > recipient.rect.top(), "the buttons are stacked, Cancel last: {:?} {:?}", cancel.rect, recipient.rect);
    let focused = win.items().into_iter().find(|item| item.focused).map(|item| item.name);
    assert_eq!(focused.as_deref(), Some("Cancel"), "Enter would send the files");
}

#[test]
fn files_dropped_on_another_page_are_sent_once() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.go(Tab::Health);
    win.quiet();
    let file = win.dir.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    win.move_to(pos2(300.0, 300.0));
    win.drop_files(std::slice::from_ref(&file));
    win.quiet();
    assert_eq!(win.tab(), Tab::Files);
    assert_eq!(fake.calls_with(&["send"]).len(), 1, "{:?}", fake.calls());
}

#[test]
fn files_let_go_over_a_sheet_are_not_sent() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.removing = Some(("p1".into(), "Office PC".into(), win.t));
    win.settle();
    assert!(win.app.sheet_open());
    let file = win.dir.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    win.move_to(pos2(300.0, 300.0));
    win.drop_files(std::slice::from_ref(&file));
    win.settle();
    win.quiet();
    assert!(fake.calls_with(&["send"]).is_empty(), "{:?}", fake.calls());
    assert!(win.app.pending_drop.is_none());
}

#[test]
fn files_dropped_on_a_row_need_a_pointer_that_moved() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    win.status(Some(crowded()));
    win.go(Tab::Files);
    let row = win.find_containing("Send files to MacBook Pro").expect("the recipient's row").rect.center();
    win.move_to(row);
    let file = win.dir.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    // The pointer has been resting there: where it is says nothing about where the files were let go.
    win.drop_files(std::slice::from_ref(&file));
    win.quiet();
    assert!(fake.calls_with(&["send"]).is_empty(), "sent without asking: {:?}", fake.calls());
    assert!(win.app.pending_drop.is_some(), "it asks which computer");
    // A pointer that moved during the drag is trusted: the row it is on.
    win.app.pending_drop = None;
    win.settle();
    win.app.drop_pos_fresh = true;
    win.drop_files(std::slice::from_ref(&file));
    win.quiet();
    let sends = fake.calls_with(&["send"]);
    assert_eq!(sends.len(), 1, "{:?}", fake.calls());
    assert!(sends[0].contains(&"p2".to_owned()), "{sends:?}");
}

#[test]
fn a_check_that_ends_after_a_change_runs_again() {
    let mut win = Win::light(1200.0, 780.0);
    let _fake = Fake::install(&mut win);
    win.status(Some(two()));
    win.app.doctor = Some((Ok(vec![]), win.t));
    let before = win.app.health_basis.clone();
    assert_eq!(before, Some((true, vec!["p1".to_owned()])));
    // A check is running when the other computer goes offline, and no status follows.
    win.app.busy.push(Job::Doctor);
    win.status(Some(with(vec![peer("p1", "Office PC", Side::Right, os::WINDOWS, false)])));
    assert_eq!(win.app.health_basis, before, "the change waits for the check to end");
    win.send(Event::Done { what: Job::Doctor, error: None });
    assert_eq!(win.app.health_basis, Some((true, vec![])), "no new check was started for what changed");
}

#[test]
fn leaving_settings_stops_a_shortcut_being_recorded() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.go(Tab::Settings);
    win.app.settings_cat = settings_page::Cat::Shortcuts;
    win.settle();
    win.ctx.data_mut(|data| data.insert_temp(Id::unique("shortcut-recorder"), 0usize));
    win.click("Wallpaper");
    win.settle();
    assert_eq!(win.tab(), Tab::Wallpaper);
    win.shortcut(Key::K);
    assert!(win.app.palette.open, "the hidden recorder swallowed the page keys");
}

#[test]
fn the_update_pill_opens_the_updates_whatever_settings_showed_last() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.go(Tab::Settings);
    win.app.settings_cat = settings_page::Cat::Shortcuts;
    win.go(Tab::Desk);
    win.send(Event::UpdateStatus(one_mk_ctl::update_logic::UpdateStatus { available: Some("9.9.9".into()), ..Default::default() }));
    win.settle();
    let pill = win.find_containing("open Settings").unwrap_or_else(|| panic!("no update pill: {:#?}", win.names()));
    win.click(&pill.name);
    win.settle();
    assert_eq!(win.tab(), Tab::Settings);
    assert_eq!(win.app.settings_cat, settings_page::Cat::General, "the Updates group is in General");
}

/// A `one-mk pair` that says "paired" and then needs a moment for what it does after.
#[test]
fn the_window_lets_a_finished_pairing_end_by_itself() {
    use std::os::unix::fs::PermissionsExt;
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(alone()));
    let marker = win.dir.join("integrated");
    let script = win.dir.join("one-mk-pairing");
    std::fs::write(&script, format!("#!/bin/sh\nprintf 'paired\\t0123456789abcdef\\tStudio PC\\tright\\n'\nsleep 0.4\ntouch '{}'\n", marker.display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut proc = None;
    for _ in 0..50 {
        match Pairing::with_method(&script, &[], win.ctx.clone(), crate::pairing::Method::Search, 1) {
            Ok(p) => {
                proc = Some(p);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(e) => panic!("{e}"),
        }
    }
    let now = win.t;
    win.app.pairing = Some(PairUi { proc, step: Step::Searching { me: "Studio".into(), port: None }, since: now, opened: now, hint: false, retry: None, me: "Studio".into(), port: None });
    win.wait("the paired step", |w| w.app.pairing.as_ref().is_some_and(|p| matches!(p.step, Step::Paired { .. })));
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "the window ended one-mk before it finished");
}

#[test]
fn a_drop_that_changes_nothing_puts_the_computer_back_where_it_is() {
    let mut win = Win::light(1200.0, 780.0);
    let fake = Fake::install(&mut win);
    let mut status = two();
    status.offsets = vec![("p1".into(), (2560, 1170))];
    win.status(Some(status));
    win.settle();
    let rest = win.app.scene.devices[1].target;
    // Placed as low as it goes along the right edge; carried straight out to the right, past where
    // it would snap, and let go: its place is the same.
    let at = pick_up(&mut win, OFFICE, vec2(250.0, 0.0));
    assert!(win.app.scene.devices[1].target != rest || win.app.drag.is_some());
    release(&mut win, at);
    win.settle();
    win.quiet();
    let d = &win.app.scene.devices[1];
    assert!(layouts(&fake).is_empty(), "the test needs a drop that changes nothing: {:?}", layouts(&fake));
    assert!((d.target.x - rest.x).abs() < 0.01 && (d.pos.x - rest.x).abs() < 0.5, "the desk stayed at the dropped spot: {:?} vs {:?}", d.target, rest);
}

#[test]
fn a_drag_picks_up_the_computer_that_was_pressed_not_what_is_under_the_pointer_later() {
    let mut win = Win::light(1200.0, 780.0);
    win.status(Some(two()));
    win.settle();
    let bounds = win.app.scene.bounds(1).expect("the computer's box");
    // The outermost point of its screen along the middle row.
    let y = bounds.center().y;
    let edge = (0..400).map(|k| bounds.right() + 2.0 - k as f32 * 0.25).find(|&x| win.app.scene.hit(pos2(x, y)) == Some(1)).expect("a point on it");
    let press = pos2(edge - 1.0, y);
    assert_eq!(win.app.scene.hit(press), Some(1));
    assert_eq!(win.app.scene.hit(press + vec2(10.0, 0.0)), None, "ten points on is off the computer");
    win.move_to(press);
    button(&mut win, press, true);
    win.frames(2);
    // The first move is already past the click distance, outward over the background.
    win.push(egui::Event::PointerMoved(press + vec2(10.0, 0.0)));
    win.frames(2);
    assert_eq!(dragged_id(&win).map(|(id, _)| id), Some("p1".into()), "{:?}", win.app.drag.is_some());
}

#[test]
fn a_recipient_scrolled_out_of_sight_takes_no_drop_over_the_bar() {
    let mut win = Win::light(1200.0, 420.0);
    let fake = Fake::install(&mut win);
    win.status(Some(crowded()));
    win.go(Tab::Files);
    let body_top = shell::frame(win.window()).content.top();
    win.move_to(pos2(600.0, 300.0));
    // Scroll a little at a time until a recipient's row has gone under the bar.
    let mut hidden = None;
    for _ in 0..60 {
        win.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -12.0), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE, source: egui::MouseWheelSource::Wheel });
        win.frames(2);
        hidden = win.controls().into_iter().find(|item| item.name.starts_with("Send files to ") && item.rect.center().y > 0.0 && item.rect.center().y < body_top - 4.0);
        if hidden.is_some() {
            break;
        }
    }
    let hidden = hidden.unwrap_or_else(|| panic!("no recipient went under the bar: {:#?}", win.controls().into_iter().filter(|i| i.name.starts_with("Send files to ")).map(|i| i.rect).collect::<Vec<_>>()));
    let file = win.dir.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    win.move_to(hidden.rect.center());
    win.app.drop_pos_fresh = true;
    win.drop_files(std::slice::from_ref(&file));
    win.quiet();
    assert!(fake.calls_with(&["send"]).is_empty(), "sent to a computer that cannot be seen: {:?}", fake.calls());
}
