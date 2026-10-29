//! The X11 backend against a private `Xvfb` (skipped where it isn't
//! installed): injection observed from a second X client, capture driven
//! through XTest the way a real mouse would push against an edge, and the
//! grab ending with the process, even on `kill -9`.

use super::*;
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use x11rb::protocol::xproto::GrabStatus;

const W: i16 = 1280;
const H: i16 = 800;

/// A private X server on a free display, killed when dropped.
struct Xvfb {
    child: Child,
    display: String,
}

impl Xvfb {
    fn start() -> Option<Xvfb> {
        // Xvfb picks a free display itself and tells us which.
        let child = Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0", &format!("{W}x{H}x24"), "-nolisten", "tcp", "-noreset"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            eprintln!("Xvfb not installed: skipping");
            return None;
        };
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().expect("piped")).read_line(&mut line).unwrap();
        let display = format!(":{}", line.trim());
        assert!(line.trim().parse::<u32>().is_ok(), "Xvfb did not start: {line:?}");
        Some(Xvfb { child, display })
    }

    fn observer(&self) -> Observer {
        let (conn, screen) = x11rb::connect(Some(&self.display)).expect("observer connects");
        let root = conn.setup().roots[screen].root;
        conn.xtest_get_version(2, 2).unwrap().reply().unwrap();
        Observer { conn, root }
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        // SIGTERM, so it removes its socket and lock file.
        // SAFETY: plain kill(2) on our own child.
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        let _ = self.child.wait();
    }
}

/// A second X client: looks at what the backend did, and plays the local
/// mouse and keyboard through XTest.
struct Observer {
    conn: RustConnection,
    root: xproto::Window,
}

impl Observer {
    fn pointer(&self) -> (i16, i16, u16) {
        let p = self.conn.query_pointer(self.root).unwrap().reply().unwrap();
        (p.root_x, p.root_y, u16::from(p.mask))
    }

    fn key_down(&self, keycode: u8) -> bool {
        let keys = self.conn.query_keymap().unwrap().reply().unwrap().keys;
        keys[usize::from(keycode / 8)] & (1 << (keycode % 8)) != 0
    }

    /// Whether this client could grab the keyboard (and lets go again).
    fn can_grab(&self) -> bool {
        let m = xproto::GrabMode::ASYNC;
        let s = self.conn.grab_keyboard(false, self.root, CURRENT_TIME, m, m).unwrap().reply().unwrap().status;
        self.conn.ungrab_keyboard(CURRENT_TIME).unwrap();
        self.conn.flush().unwrap();
        s == GrabStatus::SUCCESS
    }

    fn fake(&self, type_: u8, detail: u8, root: xproto::Window, x: i16, y: i16) {
        self.conn.xtest_fake_input(type_, detail, CURRENT_TIME, root, x, y, 0).unwrap();
        self.conn.sync().unwrap();
    }

    fn move_to(&self, x: i16, y: i16) {
        self.fake(xproto::MOTION_NOTIFY_EVENT, 0, self.root, x, y);
    }

    /// Relative motion, as a mouse makes it (barriers only stop that).
    fn move_by(&self, dx: i16, dy: i16) {
        self.fake(xproto::MOTION_NOTIFY_EVENT, 1, NONE, dx, dy);
    }

    fn sync(&self) {
        self.conn.sync().unwrap();
    }
}

trait Sync_ {
    fn sync(&self) -> Result<()>;
}

impl Sync_ for RustConnection {
    fn sync(&self) -> Result<()> {
        self.get_input_focus()?.reply()?;
        Ok(())
    }
}

/// Polls `f` until it holds (the backend works on its own thread).
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// The next event that is not a status report.
async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("an event within 3 s")
            .expect("backend alive");
        if !matches!(ev, Event::Status { .. }) {
            return ev;
        }
    }
}

async fn activated(events: &mut mpsc::UnboundedReceiver<Event>) -> (u32, Side, Pos) {
    loop {
        match next(events).await {
            Event::Activated { id, side, pos } => return (id, side, pos),
            Event::Modifiers(_) => {}
            ev => panic!("expected Activated, got {ev:?}"),
        }
    }
}

#[tokio::test]
async fn injection_reaches_the_x_server() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, _locked) = open_on(&x.display).unwrap();
    let screen = Desktop::new(vec![Rect::new(0, 0, W.into(), H.into())]);
    assert_eq!(next(&mut b.events).await, Event::Desktop(screen));

    b.inject.send(Inject::MoveTo(Pos::new(100.0, 200.0))).unwrap();
    eventually("the absolute move", || o.pointer().0 == 100 && o.pointer().1 == 200);
    b.inject.send(Inject::MoveBy { dx: 3, dy: -2 }).unwrap();
    eventually("the relative move", || o.pointer().0 == 103 && o.pointer().1 == 198);
    b.inject.send(Inject::MoveTo(Pos::new(99999.0, -5.0))).unwrap();
    eventually("a move clamped onto the screen", || o.pointer().0 == W - 1 && o.pointer().1 == 0);

    b.inject.send(Inject::Key { code: 30, down: true }).unwrap();
    eventually("A down (keycode 38)", || o.key_down(38));
    b.inject.send(Inject::Key { code: 30, down: false }).unwrap();
    eventually("A up", || !o.key_down(38));

    let button1 = u16::from(xproto::KeyButMask::BUTTON1);
    let button3 = u16::from(xproto::KeyButMask::BUTTON3);
    b.inject.send(Inject::Button { button: 0x110, down: true }).unwrap();
    eventually("left button down", || o.pointer().2 & button1 != 0);
    b.inject.send(Inject::Button { button: 0x110, down: false }).unwrap();
    b.inject.send(Inject::Button { button: 0x111, down: true }).unwrap();
    eventually("left up, right down", || o.pointer().2 & (button1 | button3) == button3);
    b.inject.send(Inject::Button { button: 0x111, down: false }).unwrap();
    eventually("right up", || o.pointer().2 & button3 == 0);

    // Wheel clicks arrive as buttons 4-7 in the window under the pointer.
    let win = o.conn.generate_id().unwrap();
    let aux = xproto::CreateWindowAux::new().override_redirect(1).event_mask(xproto::EventMask::BUTTON_PRESS);
    o.conn
        .create_window(0, win, o.root, 0, 0, W as u16, H as u16, 0, xproto::WindowClass::INPUT_ONLY, 0, &aux)
        .unwrap();
    o.conn.map_window(win).unwrap();
    o.sync();
    b.inject.send(Inject::MoveTo(Pos::new(500.0, 500.0))).unwrap();
    for m in [
        Inject::ScrollDiscrete { dx: 0, dy: 120 },
        Inject::ScrollDiscrete { dx: 0, dy: -60 },
        Inject::ScrollDiscrete { dx: 0, dy: -60 },
        Inject::ScrollDiscrete { dx: 120, dy: 0 },
        Inject::Scroll { dx: -30.0, dy: 0.0 },
        Inject::Scroll { dx: -30.0, dy: 0.0 },
    ] {
        b.inject.send(m).unwrap();
        // One at a time, so they are not coalesced.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut buttons = Vec::new();
    eventually("four wheel clicks", || {
        while let Some(ev) = o.conn.poll_for_event().unwrap() {
            if let x11rb::protocol::Event::ButtonPress(e) = ev {
                buttons.push(e.detail);
            }
        }
        buttons.len() >= 4
    });
    assert_eq!(buttons, vec![5, 4, 7, 6]);

    // A key still held when the daemon goes away is let go of.
    b.inject.send(Inject::Key { code: 42, down: true }).unwrap();
    eventually("Shift down", || o.key_down(50));
    drop(b);
    eventually("Shift let go of when the daemon went away", || !o.key_down(50));
}

#[tokio::test]
async fn pushing_against_an_edge_captures_until_released() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, locked) = open_on(&x.display).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    b.capture.send(CaptureCmd::SetSides(vec![crate::Armed::whole(Side::Right)])).unwrap();

    // Along the edge without pushing out: nothing.
    o.move_to(W - 1, 400);
    o.move_by(0, 5);
    // The left edge has no peer.
    o.move_to(0, 400);
    o.move_by(-10, 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(b.events.try_recv().is_err(), "nothing captured yet");
    assert!(o.can_grab());

    o.move_to(W - 1, 400);
    o.move_by(10, 0);
    let (id, side, pos) = activated(&mut b.events).await;
    assert_eq!((side, pos), (Side::Right, Pos::new(f64::from(W - 1), 400.0)));
    assert!(!o.can_grab(), "the keyboard is ours");

    // Local input now goes to the peer, and nowhere here.
    o.move_by(-3, 2);
    assert_eq!(next(&mut b.events).await, Event::Motion { dx: -3.0, dy: 2.0 });
    assert_eq!((o.pointer().0, o.pointer().1), (W - 1, 400), "the pointer stays put");
    o.fake(2, 38, NONE, 0, 0);
    o.fake(3, 38, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: true });
    assert_eq!(next(&mut b.events).await, Event::Key { code: 30, down: false });
    o.fake(4, 1, NONE, 0, 0);
    o.fake(5, 1, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::Button { button: 0x110, down: true });
    assert_eq!(next(&mut b.events).await, Event::Button { button: 0x110, down: false });
    o.fake(4, 5, NONE, 0, 0);
    o.fake(5, 5, NONE, 0, 0);
    assert_eq!(next(&mut b.events).await, Event::ScrollDiscrete { dx: 0, dy: 120 });

    // A release for an older activation is ignored; ours lets go.
    b.capture.send(CaptureCmd::Release { id: id.wrapping_sub(1), pos: None }).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!o.can_grab());
    b.capture.send(CaptureCmd::Release { id, pos: Some(Pos::new(1270.0, 300.0)) }).unwrap();
    eventually("the pointer put back", || (o.pointer().0, o.pointer().1) == (1270, 300));
    assert!(o.can_grab(), "the keyboard is free again");
    o.move_by(-5, 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ev = b.events.try_recv();
    assert!(ev.is_err(), "local input stays local: {ev:?}");

    // `one-mk switch`, then the screen locks: capture ends by itself.
    b.capture.send(CaptureCmd::Activate { side: Side::Left }).unwrap();
    let (id, side, _) = activated(&mut b.events).await;
    assert_eq!(side, Side::Left);
    assert!(!o.can_grab());
    locked.send(true).unwrap();
    assert_eq!(next(&mut b.events).await, Event::Deactivated { id });
    eventually("the grab released on lock", || o.can_grab());
    o.move_to(W - 1, 100);
    o.move_by(10, 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(b.events.try_recv().is_err(), "no capture while locked");
    b.capture.send(CaptureCmd::Activate { side: Side::Right }).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Error(_)));
    locked.send(false).unwrap();

    // While a peer controls this machine, local input is reported, and
    // our own injected input is not.
    b.inject.send(Inject::Controlled(true)).unwrap();
    b.inject.send(Inject::MoveBy { dx: -2, dy: 0 }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(b.events.try_recv().is_err(), "injected motion is not local input");
    // (XTest is all this X server has: the observer's own XTest device
    // counts as ours, so this is as far as Xvfb lets us check.)

    // Another program holding the keyboard: no capture, and no retry
    // until the pointer leaves the edge.
    b.inject.send(Inject::Controlled(false)).unwrap();
    let m = xproto::GrabMode::ASYNC;
    o.conn.grab_keyboard(false, o.root, CURRENT_TIME, m, m).unwrap().reply().unwrap();
    o.move_to(W - 1, 200);
    o.move_by(10, 0);
    o.move_by(10, 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(b.events.try_recv().is_err(), "no capture while another client has the keyboard");
    o.conn.ungrab_keyboard(CURRENT_TIME).unwrap();
    o.move_by(-20, 0);
    o.move_by(30, 0);
    let (id, _, _) = activated(&mut b.events).await;

    // The daemon going away ends capture.
    drop(b.capture);
    drop(b.inject);
    eventually("the grab released when the daemon went away", || o.can_grab());
    let _ = id;
}

#[tokio::test]
async fn without_barriers_an_edge_window_captures() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let (mut b, _locked) = open_target(Target { display: Some(x.display.clone()), barriers: false }).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    o.move_to(W - 1, 400);
    b.capture.send(CaptureCmd::SetSides(vec![crate::Armed::whole(Side::Right)])).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(b.events.try_recv().is_err(), "the pointer was there before: no capture");
    o.move_to(600, 400);
    o.move_to(W - 1, 400);
    let (id, side, pos) = activated(&mut b.events).await;
    assert_eq!((side, pos), (Side::Right, Pos::new(f64::from(W - 1), 400.0)));
    assert!(!o.can_grab());
    o.move_by(-4, 1);
    assert_eq!(next(&mut b.events).await, Event::Motion { dx: -4.0, dy: 1.0 });

    // Put back onto the edge: no capture until it has left the edge.
    b.capture.send(CaptureCmd::Release { id, pos: Some(Pos::new(f64::from(W - 1), 300.0)) }).unwrap();
    eventually("the pointer put back", || (o.pointer().0, o.pointer().1) == (W - 1, 300));
    assert!(o.can_grab());
    o.move_to(W - 1, 310);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(b.events.try_recv().is_err(), "no capture while still on the edge");
    o.move_to(W - 10, 310);
    o.move_to(W - 1, 320);
    let (_, side, _) = activated(&mut b.events).await;
    assert_eq!(side, Side::Right);
}

#[tokio::test]
async fn monitor_changes_are_reported() {
    let Some(x) = Xvfb::start() else { return };
    let (mut b, _locked) = open_on(&x.display).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    // Xvfb can only shrink its screen (which also turns its one output
    // off); that is a monitor change all the same.
    let xrandr = |args: &[&str]| {
        Command::new("xrandr").arg("-d").arg(&x.display).args(args).stderr(Stdio::null()).status().is_ok()
    };
    b.capture.send(CaptureCmd::Activate { side: Side::Top }).unwrap();
    let (id, ..) = activated(&mut b.events).await;
    if !xrandr(&["--fb", "1024x768"]) {
        eprintln!("xrandr not installed: skipping");
        return;
    }
    // Capture in progress ends: the screen it started on may be gone.
    assert_eq!(next(&mut b.events).await, Event::Deactivated { id });
    assert_eq!(next(&mut b.events).await, Event::Desktop(Desktop::new(vec![Rect::new(0, 0, 1024, 768)])));
    assert!(x.observer().can_grab());
}

/// Run by `kill_9_releases_the_grab` in a process of its own: captures,
/// presses a key, and waits to be killed.
#[tokio::test]
#[ignore = "run by kill_9_releases_the_grab"]
async fn child_captures_and_waits() {
    let Ok(display) = std::env::var("ONE_MK_TEST_DISPLAY") else { return };
    let (mut b, _locked) = open_on(&display).unwrap();
    assert!(matches!(next(&mut b.events).await, Event::Desktop(_)));
    b.inject.send(Inject::Key { code: 30, down: true }).unwrap();
    b.capture.send(CaptureCmd::Activate { side: Side::Right }).unwrap();
    activated(&mut b.events).await;
    println!("CAPTURED");
    tokio::time::sleep(Duration::from_secs(60)).await;
}

#[test]
fn kill_9_releases_the_grab() {
    let Some(x) = Xvfb::start() else { return };
    let o = x.observer();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "x11::tests::child_captures_and_waits", "--ignored", "--nocapture", "--test-threads=1"])
        .env("ONE_MK_TEST_DISPLAY", &x.display)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let out = std::io::BufReader::new(child.stdout.take().unwrap());
    let captured = out.lines().map_while(Result::ok).any(|l| l.contains("CAPTURED"));
    assert!(captured, "the child captured");
    assert!(!o.can_grab(), "the child holds the keyboard");
    assert!(o.key_down(38), "the child holds A down");
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
    child.wait().unwrap();
    eventually("the grab gone with the killed process", || o.can_grab());
    // XTest keeps keys pressed after the client that pressed them is gone;
    // the next start lets go of them.
    assert!(o.key_down(38), "XTest still holds A after kill -9");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (_b, _locked) = rt.block_on(async { open_on(&x.display) }).unwrap();
    eventually("A let go of by the next start", || !o.key_down(38));
}
