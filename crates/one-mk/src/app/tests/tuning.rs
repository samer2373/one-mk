//! Input tuning between real daemons on loopback (null backends): what B
//! is sent when A's `[input]` remaps keys and buttons and tunes the
//! pointer and scrolling, and that nothing stays held when a visit ends
//! mid-press or the rules change mid-visit.

use super::*;
use crate::tuning::{ButtonFrom, ButtonRule, KeyRule, PointerRule, Scope, ScrollRule, To};

const CAPS: u32 = 58;
const ESC: u32 = 1;
const LCTRL: u32 = 29;
const LSHIFT: u32 = 42;
const LALT: u32 = 56;
const RALT: u32 = 100;
const H: u32 = 35;
const C: u32 = 46;
const LEFT: u32 = 105;
const BACKSPACE: u32 = 14;

fn to(from: &str, to: &str) -> KeyRule {
    KeyRule { from: from.into(), to: Some(To::One(to.into())), ..Default::default() }
}

fn key(code: u32, down: bool) -> Inject {
    Inject::Key { code, down }
}

/// Presses (or releases) keys on A, in order.
fn press(duo: &Duo, keys: &[(u32, bool)]) {
    for &(code, down) in keys {
        duo.ha.events.send(Event::Key { code, down }).unwrap();
    }
}

/// The next `n` key, button and scroll injections on B (pointer moves skipped).
async fn next_n(duo: &mut Duo, n: usize) -> Vec<Inject> {
    let mut out = Vec::new();
    while out.len() < n {
        out.push(next_non_move(&mut duo.hb).await);
    }
    out
}

/// Remaps, swaps and combos: B gets the keys A's rules make, and Ctrl
/// hidden by a combo comes back while it is still held.
#[tokio::test]
async fn remapped_keys_reach_the_peer() {
    let mut duo = Duo::start("tune-keys", None).await;
    set(&duo.da, |s| {
        s.input.keymap.rule = vec![to("capslock", "esc"), to("lalt", "lmeta"), to("ctrl+h", "backspace")];
    })
    .await;
    duo.enter_b().await;
    press(&duo, &[(CAPS, true), (CAPS, false), (LALT, true), (LALT, false)]);
    assert_eq!(next_n(&mut duo, 4).await, [key(ESC, true), key(ESC, false), key(125, true), key(125, false)]);
    press(&duo, &[(LCTRL, true), (H, true), (H, false), (LCTRL, false)]);
    assert_eq!(
        next_n(&mut duo, 6).await,
        [key(LCTRL, true), key(LCTRL, false), key(BACKSPACE, true), key(BACKSPACE, false), key(LCTRL, true), key(LCTRL, false)]
    );
    duo.stop().await;
}

/// Caps Lock as tap Esc / hold Ctrl, and a layer: B sees the hold as soon
/// as another key is pressed, and when the pointer comes back mid-hold, B
/// releases what A's mapper sent (Ctrl and Left), not the keys pressed on
/// A (Caps Lock, AltGr, h).
#[tokio::test]
async fn a_visit_ending_mid_hold_releases_what_was_sent() {
    let mut duo = Duo::start("tune-hold", None).await;
    set(&duo.da, |s| {
        s.input.keymap.rule = vec![
            KeyRule { from: "capslock".into(), tap: Some("esc".into()), hold: Some("lctrl".into()), ..Default::default() },
            to("ralt", "layer:nav"),
            KeyRule { layer: Some("nav".into()), ..to("h", "left") },
        ];
    })
    .await;
    duo.enter_b().await;
    // A tap.
    press(&duo, &[(CAPS, true), (CAPS, false)]);
    assert_eq!(next_n(&mut duo, 2).await, [key(ESC, true), key(ESC, false)]);
    // Held with C: Ctrl+C.
    press(&duo, &[(CAPS, true), (C, true), (C, false)]);
    assert_eq!(next_n(&mut duo, 3).await, [key(LCTRL, true), key(C, true), key(C, false)]);
    // Into the layer, h held as Left.
    press(&duo, &[(RALT, true), (H, true)]);
    assert_eq!(next_n(&mut duo, 1).await, [key(LEFT, true)]);
    // Back home with Caps Lock, AltGr and h still down.
    duo.leave_b().await;
    let mut released = next_n(&mut duo, 2).await;
    released.sort_by_key(|i| if let Inject::Key { code, .. } = i { *code } else { 0 });
    assert_eq!(released, [key(LCTRL, false), key(LEFT, false)]);
    // Their physical releases now go nowhere.
    press(&duo, &[(H, false), (RALT, false), (CAPS, false)]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(injected(&mut duo.hb).into_iter().filter(|i| !matches!(i, Inject::MoveTo(_))).count(), 0);
    duo.stop().await;
}

/// A tap/hold key held past its time is held, without another key: the
/// daemon's timer runs only while it waits.
#[tokio::test]
async fn a_tap_hold_key_held_long_enough_holds() {
    let mut duo = Duo::start("tune-timer", None).await;
    set(&duo.da, |s| {
        s.input.keymap.tap_ms = Some(60);
        s.input.keymap.rule =
            vec![KeyRule { from: "capslock".into(), tap: Some("esc".into()), hold: Some("lctrl".into()), ..Default::default() }];
    })
    .await;
    duo.enter_b().await;
    let at = Instant::now();
    press(&duo, &[(CAPS, true)]);
    assert_eq!(next_n(&mut duo, 1).await, [key(LCTRL, true)]);
    assert!(at.elapsed() >= Duration::from_millis(60), "{:?}", at.elapsed());
    press(&duo, &[(CAPS, false)]);
    assert_eq!(next_n(&mut duo, 1).await, [key(LCTRL, false)]);
    duo.stop().await;
}

/// The keymap changes mid-visit: a key held as Ctrl is still released as
/// Ctrl, new presses follow the new rules.
#[tokio::test]
async fn a_reload_mid_hold_keeps_the_pairing() {
    let mut duo = Duo::start("tune-reload", None).await;
    set(&duo.da, |s| s.input.keymap.rule = vec![to("capslock", "lctrl")]).await;
    duo.enter_b().await;
    press(&duo, &[(CAPS, true)]);
    assert_eq!(next_n(&mut duo, 1).await, [key(LCTRL, true)]);
    set(&duo.da, |s| s.input.keymap.rule = vec![to("capslock", "esc")]).await;
    press(&duo, &[(CAPS, false), (CAPS, true), (CAPS, false)]);
    assert_eq!(next_n(&mut duo, 3).await, [key(LCTRL, false), key(ESC, true), key(ESC, false)]);
    duo.stop().await;
}

/// Buttons: middle becomes left (held through a drag), the side buttons
/// go back as Alt+Left on a PC, and a key can be an action (`switch here`
/// brings the pointer home).
#[tokio::test]
async fn buttons_are_remapped_and_keys_can_switch() {
    let mut duo = Duo::start("tune-buttons", None).await;
    set(&duo.da, |s| {
        s.input.button = vec![
            ButtonRule { from: ButtonFrom::Number(3), to: To::One("button1".into()), scope: Scope::Global, app: None },
            ButtonRule { from: ButtonFrom::Number(4), to: To::One("back".into()), scope: Scope::Global, app: None },
        ];
        s.input.keymap.rule = vec![to("f13", "switch here")];
    })
    .await;
    duo.enter_b().await;
    duo.ha.events.send(Event::Button { button: 0x112, down: true }).unwrap();
    duo.ha.events.send(Event::Motion { dx: 50.0, dy: 0.0 }).unwrap();
    duo.ha.events.send(Event::Button { button: 0x112, down: false }).unwrap();
    assert_eq!(
        next_n(&mut duo, 2).await,
        [Inject::Button { button: 0x110, down: true }, Inject::Button { button: 0x110, down: false }]
    );
    duo.ha.events.send(Event::Button { button: 0x113, down: true }).unwrap();
    duo.ha.events.send(Event::Button { button: 0x113, down: false }).unwrap();
    assert_eq!(next_n(&mut duo, 4).await, [key(LALT, true), key(LEFT, true), key(LEFT, false), key(LALT, false)]);
    press(&duo, &[(183, true)]);
    assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { .. }), "f13 brought the pointer home");
    duo.stop().await;
}

/// Scrolling for this one computer: reversed, and Shift scrolls sideways;
/// Shift reached B when pressed and is lifted (released on B) by the
/// first sideways scroll, so B never sees Shift+scroll nor keeps Shift.
#[tokio::test]
async fn scrolling_is_tuned_and_the_modifier_is_lifted() {
    let mut duo = Duo::start("tune-scroll", None).await;
    let fb = duo.fb[..8].to_owned();
    set(&duo.da, |s| {
        s.input.scroll = vec![
            ScrollRule { reverse_vertical: Some(true), scope: Scope::Peer(fb.clone()), ..Default::default() },
            ScrollRule { modifiers: [("shift".to_owned(), "horizontal".to_owned())].into(), ..Default::default() },
            // Another computer's: not for B.
            ScrollRule { speed: Some(5.0), scope: Scope::Peer("someone else".into()), ..Default::default() },
        ];
    })
    .await;
    duo.enter_b().await;
    duo.ha.events.send(Event::ScrollDiscrete { dx: 0, dy: 120 }).unwrap();
    assert_eq!(next_n(&mut duo, 1).await, [Inject::ScrollDiscrete { dx: 0, dy: -120 }]);
    press(&duo, &[(LSHIFT, true)]);
    assert_eq!(next_n(&mut duo, 1).await, [key(LSHIFT, true)]);
    duo.ha.events.send(Event::ScrollDiscrete { dx: 0, dy: 120 }).unwrap();
    duo.ha.events.send(Event::ScrollDiscrete { dx: 0, dy: 120 }).unwrap();
    assert_eq!(
        next_n(&mut duo, 3).await,
        [key(LSHIFT, false), Inject::ScrollDiscrete { dx: -120, dy: 0 }, Inject::ScrollDiscrete { dx: -120, dy: 0 }]
    );
    // Released here: nothing more for B; pressed again, it goes through.
    press(&duo, &[(LSHIFT, false), (LSHIFT, true), (LSHIFT, false)]);
    assert_eq!(next_n(&mut duo, 2).await, [key(LSHIFT, true), key(LSHIFT, false)]);
    duo.ha.events.send(Event::Scroll { dx: 0.0, dy: 4.0 }).unwrap();
    assert_eq!(next_n(&mut duo, 1).await, [Inject::Scroll { dx: 0.0, dy: -4.0 }]);
    duo.stop().await;
}

/// Pointer speed: B's pointer moves twice as far as A's mouse says.
#[tokio::test]
async fn pointer_speed_scales_the_remote_motion() {
    let mut duo = Duo::start("tune-pointer", None).await;
    set(&duo.da, |s| s.input.pointer = vec![PointerRule { speed: Some(2.0), ..Default::default() }]).await;
    duo.enter_b().await;
    duo.ha.events.send(Event::Motion { dx: 10.0, dy: 5.25 }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(20.0, 550.5)));
    duo.stop().await;
}

/// Sets what B's (fake) focus probe says: a window of the app going by
/// `apps`, or (`None`) a window it can't tell (Wayland-native).
fn focus_b(duo: &Duo, apps: Option<&[&str]>) {
    let focus = apps.map(|a| one_mk_proto::Focus { fullscreen: false, apps: a.iter().map(|s| (*s).to_owned()).collect() });
    *duo.hb.focus.lock().unwrap() = focus;
}

/// A click on B (which makes B look at its focus again, as a click may
/// move it there).
async fn click(duo: &mut Duo) {
    duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
    duo.ha.events.send(Event::Button { button: 0x110, down: false }).unwrap();
    assert_eq!(
        next_n(duo, 2).await,
        [Inject::Button { button: 0x110, down: true }, Inject::Button { button: 0x110, down: false }]
    );
}

/// The next absolute position B's pointer is moved to.
async fn next_pos(duo: &mut Duo) -> Pos {
    loop {
        if let Inject::MoveTo(p) = next_inject(&mut duo.hb).await {
            return p;
        }
    }
}

/// The pointer speed in force for B: how far 10 px of A's mouse moves it
/// (there and back, so it stays put).
async fn speed(duo: &mut Duo) -> f64 {
    duo.ha.events.send(Event::Motion { dx: 10.0, dy: 0.0 }).unwrap();
    let there = next_pos(duo).await;
    duo.ha.events.send(Event::Motion { dx: -10.0, dy: 0.0 }).unwrap();
    let back = next_pos(duo).await;
    (there.x - back.x) / 10.0
}

/// Waits until B's report of its focus has reached A's rules: the
/// pointer speed for the app is in force (or isn't).
async fn until_speed(duo: &mut Duo, want: f64) {
    for _ in 0..200 {
        if speed(duo).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("pointer speed {want} never came into force");
}

/// Rules for an app follow the app with the focus on B: B says which
/// after a click (once the click has acted), A's rules switch on and off
/// mid-visit, a key held through a switch is released as it was pressed,
/// and an app B can't name turns the rules for apps off, which `one-mk
/// status` explains.
#[tokio::test]
async fn per_app_rules_follow_the_peers_focused_app() {
    let mut duo = Duo::start("tune-apps", None).await;
    set(&duo.da, |s| {
        s.input.keymap.rule = vec![to("capslock", "esc"), KeyRule { app: Some("code".into()), ..to("capslock", "lctrl") }];
        s.input.pointer = vec![PointerRule { speed: Some(2.0), app: Some("code".into()), ..Default::default() }];
    })
    .await;
    focus_b(&duo, Some(&["firefox", "Firefox"]));
    duo.enter_b().await;
    eventually("B looked at its focus", || duo.hb.focus_probes.load(std::sync::atomic::Ordering::SeqCst) > 0).await;
    duo.ha.events.send(Event::Motion { dx: 100.0, dy: 0.0 }).unwrap(); // away from the edge
    next_pos(&mut duo).await;
    assert_eq!(speed(&mut duo).await, 1.0);
    press(&duo, &[(CAPS, true), (CAPS, false)]);
    assert_eq!(next_n(&mut duo, 2).await, [key(ESC, true), key(ESC, false)]);

    // Into the editor on B.
    focus_b(&duo, Some(&["code", "Code"]));
    click(&mut duo).await;
    until_speed(&mut duo, 2.0).await;
    press(&duo, &[(CAPS, true), (CAPS, false)]);
    assert_eq!(next_n(&mut duo, 2).await, [key(LCTRL, true), key(LCTRL, false)]);

    // Held as Ctrl while the focus moves on: still released as Ctrl.
    press(&duo, &[(CAPS, true)]);
    assert_eq!(next_n(&mut duo, 1).await, [key(LCTRL, true)]);
    focus_b(&duo, Some(&["firefox", "Firefox"]));
    click(&mut duo).await;
    until_speed(&mut duo, 1.0).await;
    press(&duo, &[(CAPS, false), (CAPS, true), (CAPS, false)]);
    assert_eq!(next_n(&mut duo, 3).await, [key(LCTRL, false), key(ESC, true), key(ESC, false)]);
    assert!(status(&duo.da).await.tuning.is_empty(), "B says which app it is");

    // A window B can't name: rules for apps are off (not left on).
    focus_b(&duo, Some(&["code"]));
    click(&mut duo).await;
    until_speed(&mut duo, 2.0).await;
    focus_b(&duo, None);
    click(&mut duo).await;
    until_speed(&mut duo, 1.0).await;
    press(&duo, &[(CAPS, true), (CAPS, false)]);
    assert_eq!(next_n(&mut duo, 2).await, [key(ESC, true), key(ESC, false)]);
    let notes = status(&duo.da).await.tuning;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("Rule for app code not active: ") && notes[0].contains("does not report its focused app"), "{notes:?}");
    duo.ha.events.send(Event::Motion { dx: -500.0, dy: 0.0 }).unwrap();
    assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { pos: Some(_), .. }), "home");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(injected(&mut duo.hb).iter().all(|i| !matches!(i, Inject::Key { .. })), "nothing was left to release");
    duo.stop().await;
}

/// B says which app has its focus only to the computer controlling it,
/// only once asked for exactly that, and only until told to stop; a
/// controller that asks the old way (fullscreen only) never hears it.
#[tokio::test]
async fn the_focused_app_goes_only_to_a_controller_that_asks() {
    let duo = Duo::start("tune-app-ask", None).await;
    let mut c = RawPeer::connect("tune-app-ask-c", &duo.db, duo.pb).await;
    c.send(Control::Caps(caps::LOCAL)).await;
    *duo.hb.focus.lock().unwrap() = Some(one_mk_proto::Focus { fullscreen: true, apps: vec!["code".into()] });
    c.send(Control::Enter { epoch: 5, pos: Pos::new(5.0, 5.0) }).await;
    loop {
        if c.next().await == (Control::EnterAck { epoch: 5 }) {
            break;
        }
    }
    async fn next_focus(c: &mut RawPeer) -> Control {
        loop {
            let m = c.next().await;
            if matches!(m, Control::Focus { .. }) {
                return m;
            }
        }
    }
    // As an older build asks: no byte saying what to report.
    c.send_body(&[18, 5, 0, 1]).await;
    assert_eq!(next_focus(&mut c).await, Control::Focus { epoch: 5, fullscreen: true, apps: vec![] });
    c.send(Control::WatchFocus { epoch: 5, on: true, app: true, fullscreen: false }).await;
    assert_eq!(next_focus(&mut c).await, Control::Focus { epoch: 5, fullscreen: false, apps: vec!["code".into()] });
    // The focus moves after a click.
    *duo.hb.focus.lock().unwrap() = Some(one_mk_proto::Focus { fullscreen: true, apps: vec!["firefox".into()] });
    c.send(Control::Button { button: 0x110, down: true, seq: 1, pos: Pos::new(5.0, 5.0) }).await;
    assert_eq!(next_focus(&mut c).await, Control::Focus { epoch: 5, fullscreen: false, apps: vec!["firefox".into()] });
    // Asked to stop: nothing more, whatever happens.
    c.send(Control::WatchFocus { epoch: 5, on: false, app: false, fullscreen: true }).await;
    *duo.hb.focus.lock().unwrap() = Some(one_mk_proto::Focus { fullscreen: false, apps: vec!["code".into()] });
    c.send(Control::Button { button: 0x110, down: false, seq: 2, pos: Pos::new(5.0, 5.0) }).await;
    c.send(Control::Button { button: 0x110, down: true, seq: 3, pos: Pos::new(5.0, 5.0) }).await;
    assert!(tokio::time::timeout(FOCUS_SETTLE * 5, next_focus(&mut c)).await.is_err(), "B said nothing more");
    duo.stop().await;
}
