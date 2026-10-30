//! Broadcast between real daemons on loopback (null backends): one
//! controller, A, and two computers, "bee" on its right (1920 x 1080) and
//! "sea" on its left (1280 x 800). Keys, clicks, scrolling and movement
//! reach both; nothing stays held when broadcast ends, when one drops out,
//! or by any random mix of presses, drops and returns.

use super::*;
use crate::app::broadcast::testing;
use crate::tuning::{KeyRule, Scope, To};

const LCTRL: u32 = 29;
const LALT: u32 = 56;
const LSHIFT: u32 = 42;
const ESC: u32 = 1;
const KEY_A: u32 = 30;
const KEY_B: u32 = 48;
const KEY_X: u32 = 45;
const CAPS: u32 = 58;
const LEFT_BUTTON: u32 = 0x110;
const CENTER_B: Pos = Pos::new(960.0, 540.0);
const CENTER_C: Pos = Pos::new(640.0, 400.0);

struct Daemon {
    dir: PathBuf,
    h: NullHandles,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<()>>,
    port: u16,
    fp: String,
}

impl Daemon {
    fn spawn(dir: &std::path::Path, port: u16, connect: Vec<SocketAddr>, desk: Desktop) -> Daemon {
        let fp = Identity::load_or_create(dir).unwrap().fingerprint.to_string();
        let (backend, h) = one_mk_input::null(desk);
        let (stop, mut rx) = watch::channel(false);
        let (power, _) = one_mk_input::fake_power();
        let opts = Options {
            backend: one_mk_input::Kind::Null,
            port,
            connect,
            discovery: false,
            clipboard: None,
            downloads: dir.join("downloads"),
            start_locked: false,
            power: Some(power),
        };
        let task = tokio::spawn(serve(dir.to_owned(), opts, backend, async move {
            let _ = rx.wait_for(|s| *s).await;
        }));
        Daemon { dir: dir.to_owned(), h, stop, task, port, fp }
    }

    async fn stop(self) {
        self.stop.send(true).unwrap();
        self.task.await.unwrap().unwrap();
    }

    fn send(&self, ev: Event) {
        self.h.events.send(ev).unwrap();
    }

    fn keys(&self, keys: &[(u32, bool)]) {
        for &(code, down) in keys {
            self.send(Event::Key { code, down, device: None });
        }
    }
}

/// `dir` trusts the computer with identity `other` under `name`, on `side`.
fn pair(dir: &std::path::Path, other: &Identity, name: &str, side: Side, swap: Swap) {
    let mut t = TrustStore::load(dir).unwrap();
    t.upsert(Device {
        name: name.into(),
        fingerprint: other.fingerprint.to_string(),
        side: side.to_string(),
        addrs: vec![],
        swap_ctrl_cmd: swap,
        ..Device::default()
    });
    t.save().unwrap();
}

struct Trio {
    a: Daemon,
    b: Daemon,
    c: Daemon,
}

fn desk_b() -> Desktop {
    Desktop::new(vec![Rect::new(0, 0, 1920, 1080)])
}

fn desk_c() -> Desktop {
    Desktop::new(vec![Rect::new(0, 0, 1280, 800)])
}

impl Trio {
    /// A, B and C running, B and C connected to A; `setup` edits A's
    /// settings folder first.
    async fn start(tag: &str, swap_c: Swap, setup: impl FnOnce(&std::path::Path)) -> Trio {
        let dirs = ["a", "b", "c"].map(|x| temp_dir(&format!("bc-{tag}-{x}")));
        let ids = dirs.each_ref().map(|d| Identity::load_or_create(d).unwrap());
        pair(&dirs[0], &ids[1], "bee", Side::Right, Swap::Auto);
        pair(&dirs[0], &ids[2], "sea", Side::Left, swap_c);
        pair(&dirs[1], &ids[0], "ay", Side::Left, Swap::Auto);
        pair(&dirs[2], &ids[0], "ay", Side::Right, Swap::Auto);
        setup(&dirs[0]);
        let pa = free_port();
        let a = Daemon::spawn(&dirs[0], pa, vec![], Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]));
        let to_a = vec![SocketAddr::from(([127, 0, 0, 1], pa))];
        let b = Daemon::spawn(&dirs[1], free_port(), to_a.clone(), desk_b());
        let c = Daemon::spawn(&dirs[2], free_port(), to_a, desk_c());
        let mut trio = Trio { a, b, c };
        trio.settle(2).await;
        trio
    }

    /// Waits until A has `n` computers online with their screens, then
    /// forgets the edges it armed meanwhile.
    async fn settle(&mut self, n: usize) {
        let da = self.a.dir.clone();
        eventually_async("computers to connect", || async {
            let s = control::request(&da, Request::Status).await.ok().flatten().and_then(|r| Status::decode(&r));
            s.is_some_and(|s| s.peers.iter().filter(|p| p.online && !p.screens.is_empty()).count() == n)
        })
        .await;
        while self.a.h.capture.try_recv().is_ok() {}
    }

    /// `one-mk broadcast on` on A, answered by the backend capturing as
    /// capture `id`; both computers get their pointer at their centre.
    async fn on(&mut self, id: u32) -> String {
        let reply = self.ask_on(id).await;
        assert_eq!(next_inject(&mut self.b.h).await, Inject::MoveTo(CENTER_B));
        assert_eq!(next_inject(&mut self.c.h).await, Inject::MoveTo(CENTER_C));
        reply
    }

    async fn ask_on(&mut self, id: u32) -> String {
        let da = self.a.dir.clone();
        let asked = tokio::spawn(async move { ask(&da, Request::Broadcast(Toggle::On)).await });
        let side = loop {
            match next_capture(&mut self.a.h).await {
                CaptureCmd::Activate { side } => break side,
                CaptureCmd::SetEdges(_) => {}
                other => panic!("expected Activate, got {other:?}"),
            }
        };
        self.a.send(Event::Activated { id, side, pos: Pos::new(1919.0, 3.0) });
        asked.await.unwrap()
    }

    /// The next key, button or scroll on B and on C.
    async fn both(&mut self) -> (Inject, Inject) {
        (next_non_move(&mut self.b.h).await, next_non_move(&mut self.c.h).await)
    }

    async fn stop(self) {
        let dirs = [self.a.dir.clone(), self.b.dir.clone(), self.c.dir.clone()];
        for d in [self.c, self.b, self.a] {
            d.stop().await;
        }
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

/// A's next capture command other than arming edges (computers come and go).
async fn next_capture_skipping_edges(h: &mut NullHandles) -> CaptureCmd {
    loop {
        match next_capture(h).await {
            CaptureCmd::SetEdges(_) => {}
            other => return other,
        }
    }
}

fn key(code: u32, down: bool) -> Inject {
    Inject::Key { code, down }
}

/// Only pointer moves (and `Controlled`) arrived on `h` since last looked.
fn nothing_but_moves(h: &mut NullHandles) {
    let rest: Vec<Inject> = injected(h).into_iter().filter(|i| !matches!(i, Inject::MoveTo(_))).collect();
    assert!(rest.is_empty(), "{rest:?}");
}

/// Keys, a click, scrolling and movement reach both computers, each
/// pointer moves on its own screen, status says so, and stopping releases
/// what was held on both and brings the pointer back to A's centre.
#[tokio::test]
async fn broadcast_types_clicks_and_scrolls_on_every_computer() {
    let mut t = Trio::start("all", Swap::Auto, |_| {}).await;
    let s = status(&t.a.dir).await;
    assert!(s.broadcast.is_empty());
    assert_eq!(s.broadcast_hotkey, if cfg!(windows) { "Ctrl+Alt+Shift+B" } else { "Ctrl+Alt+B" });

    assert_eq!(t.on(7).await, "ok\non\nbee\tsea\n");
    let s = status(&t.a.dir).await;
    assert_eq!((s.broadcast, s.controlling), (vec!["bee".to_owned(), "sea".to_owned()], None));
    assert!(status(&t.b.dir).await.controlled_by.is_some(), "B sees a normal controller");
    assert_eq!(ask(&t.a.dir, Request::Broadcast(Toggle::On)).await, "already\non\nbee\tsea\n");

    // Movement: the same amount from each centre.
    t.a.send(Event::Motion { dx: 100.0, dy: -40.0, device: None });
    assert_eq!(next_inject(&mut t.b.h).await, Inject::MoveTo(Pos::new(1060.0, 500.0)));
    assert_eq!(next_inject(&mut t.c.h).await, Inject::MoveTo(Pos::new(740.0, 360.0)));
    // Keys.
    t.a.keys(&[(LSHIFT, true), (KEY_A, true), (KEY_A, false)]);
    for want in [key(LSHIFT, true), key(KEY_A, true), key(KEY_A, false)] {
        assert_eq!(t.both().await, (want, want));
    }
    // A click, where each pointer is.
    t.a.send(Event::Button { button: LEFT_BUTTON, down: true, device: None });
    assert_eq!(next_inject(&mut t.b.h).await, Inject::MoveTo(Pos::new(1060.0, 500.0)));
    assert_eq!(next_inject(&mut t.c.h).await, Inject::MoveTo(Pos::new(740.0, 360.0)));
    let click = Inject::Button { button: LEFT_BUTTON, down: true };
    assert_eq!(t.both().await, (click, click));
    t.a.send(Event::Button { button: LEFT_BUTTON, down: false, device: None });
    let up = Inject::Button { button: LEFT_BUTTON, down: false };
    assert_eq!(t.both().await, (up, up));
    // Scrolling.
    t.a.send(Event::ScrollDiscrete { dx: 0, dy: -1, device: None });
    let wheel = Inject::ScrollDiscrete { dx: 0, dy: -1 };
    assert_eq!(t.both().await, (wheel, wheel));
    // A long push right: each stops at its own edge; neither leaves.
    t.a.send(Event::Motion { dx: 99_999.0, dy: 0.0, device: None });
    for (h, right, y) in [(&mut t.b.h, 1920.0, 500.0), (&mut t.c.h, 1280.0, 360.0)] {
        match next_inject(h).await {
            Inject::MoveTo(p) => assert!((right - 1.0..right).contains(&p.x) && p.y == y, "{p:?}"),
            other => panic!("expected a move, got {other:?}"),
        }
    }
    assert!(!status(&t.a.dir).await.broadcast.is_empty());

    // Off with Shift still held: both let go of it (told to, well before
    // their lease would have), A gets its pointer back.
    let asked = Instant::now();
    assert_eq!(ask(&t.a.dir, Request::Broadcast(Toggle::Off)).await, "ok\noff\n");
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 7, pos: Some(CENTER_B) });
    assert_eq!(t.both().await, (key(LSHIFT, false), key(LSHIFT, false)));
    assert!(asked.elapsed() < LEASE / 2, "released by the end of broadcast, not by a lease: {:?}", asked.elapsed());
    assert!(status(&t.a.dir).await.broadcast.is_empty());
    eventually_async("B to be let go", || async { status(&t.b.dir).await.controlled_by.is_none() }).await;
    // Shift's real release, after, goes nowhere.
    t.a.send(Event::Key { code: LSHIFT, down: false, device: None });
    assert_eq!(ask(&t.a.dir, Request::Broadcast(Toggle::Off)).await, "already\noff\n");
    tokio::time::sleep(Duration::from_millis(100)).await;
    nothing_but_moves(&mut t.b.h);
    nothing_but_moves(&mut t.c.h);
    t.stop().await;
}

/// Nobody to broadcast to: said, and nothing captured. A visit in
/// progress hands its capture over (letting go on the computer it was on);
/// the shortcut starts and stops it (its key never reaching anyone), and so
/// do `switch here` and a jump.
#[tokio::test]
async fn broadcast_starts_and_stops_every_way() {
    use one_mk_input::hotkeys::Action;
    // Alone.
    let alone = temp_dir("bc-alone");
    let mut a = Daemon::spawn(&alone, free_port(), vec![], Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]));
    eventually_async("A", || async { control::request(&alone, Request::Status).await.ok().flatten().is_some() }).await;
    let reply = ask(&alone, Request::Broadcast(Toggle::Toggle)).await;
    assert!(reply.starts_with("error no other computer is connected"), "{reply}");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(a.h.capture.try_recv().is_err(), "nothing was captured");
    a.stop().await;
    let _ = std::fs::remove_dir_all(alone);

    let mut t = Trio::start("ways", Swap::Auto, |_| {}).await;
    // A visit to B, Shift held there: broadcast takes over at once.
    let fb = t.b.fp.clone();
    assert!(ask(&t.a.dir, switch_to(&fb)).await.starts_with("ok\n"));
    assert_eq!(next_capture(&mut t.a.h).await, CaptureCmd::Activate { side: Side::Right });
    t.a.send(Event::Activated { id: 3, side: Side::Right, pos: Pos::new(1919.0, 3.0) });
    assert_eq!(next_inject(&mut t.b.h).await, Inject::MoveTo(CENTER_B));
    t.a.send(Event::Key { code: LSHIFT, down: true, device: None });
    assert_eq!(next_non_move(&mut t.b.h).await, key(LSHIFT, true));
    assert_eq!(ask(&t.a.dir, Request::Broadcast(Toggle::On)).await, "ok\non\nbee\tsea\n");
    assert_eq!(next_non_move(&mut t.b.h).await, key(LSHIFT, false), "the visit let go first");
    assert_eq!(next_inject(&mut t.c.h).await, Inject::MoveTo(CENTER_C));
    // Shift's release now is for nobody (B never saw it pressed in this
    // broadcast); a key is for both.
    t.a.keys(&[(LSHIFT, false), (KEY_A, true), (KEY_A, false)]);
    assert_eq!(t.both().await, (key(KEY_A, true), key(KEY_A, true)));
    assert_eq!(t.both().await, (key(KEY_A, false), key(KEY_A, false)));

    // Ctrl+Alt+B while broadcasting stops it; B is never typed anywhere.
    t.a.keys(&[(LCTRL, true), (LALT, true)]);
    assert_eq!(t.both().await, (key(LCTRL, true), key(LCTRL, true)));
    assert_eq!(t.both().await, (key(LALT, true), key(LALT, true)));
    t.a.send(Event::Key { code: KEY_B, down: true, device: None });
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 3, pos: Some(CENTER_B) });
    assert_eq!(t.both().await, (key(LCTRL, false), key(LCTRL, false)));
    assert_eq!(t.both().await, (key(LALT, false), key(LALT, false)));
    t.a.keys(&[(KEY_B, false), (LCTRL, false), (LALT, false)]);

    // The shortcut at home (from the system's listener) starts it. (Not
    // within 300 ms of the same shortcut from captured keys: that would be
    // the other report of the same press.)
    tokio::time::sleep(one_mk_input::hotkeys::DEBOUNCE).await;
    t.a.send(Event::Hotkey(Action::Broadcast));
    let side = loop {
        match next_capture(&mut t.a.h).await {
            CaptureCmd::Activate { side } => break side,
            CaptureCmd::SetEdges(_) => {}
            other => panic!("{other:?}"),
        }
    };
    t.a.send(Event::Activated { id: 4, side, pos: Pos::new(1919.0, 3.0) });
    assert_eq!(next_inject(&mut t.b.h).await, Inject::MoveTo(CENTER_B));
    assert_eq!(next_inject(&mut t.c.h).await, Inject::MoveTo(CENTER_C));
    eventually_async("broadcast on", || async { !status(&t.a.dir).await.broadcast.is_empty() }).await;
    // `switch here` stops it.
    assert_eq!(ask(&t.a.dir, Request::Switch(SwitchTo::Here)).await, "here\nstopped broadcasting\n");
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 4, pos: Some(CENTER_B) });

    // A jump to C stops it, then goes to C as usual.
    t.on(5).await;
    let fc = t.c.fp.clone();
    assert!(ask(&t.a.dir, switch_to(&fc)).await.starts_with("ok\n"));
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 5, pos: Some(CENTER_B) });
    assert_eq!(next_capture(&mut t.a.h).await, CaptureCmd::Activate { side: Side::Left });
    t.a.send(Event::Activated { id: 6, side: Side::Left, pos: Pos::new(0.0, 3.0) });
    assert_eq!(next_inject(&mut t.c.h).await, Inject::MoveTo(CENTER_C));
    assert!(status(&t.a.dir).await.broadcast.is_empty());
    assert!(status(&t.a.dir).await.controlling.is_some());
    t.stop().await;
}

/// A computer whose user takes control back drops out (letting go of what
/// it held, by itself) and the rest carry on; one that reconnects is not
/// added; when the last one leaves, broadcast ends.
#[tokio::test]
async fn a_computer_that_drops_out_lets_go_and_the_rest_carry_on() {
    let mut t = Trio::start("drop", Swap::Auto, |_| {}).await;
    t.on(1).await;
    t.a.send(Event::Key { code: KEY_A, down: true, device: None });
    t.a.send(Event::Button { button: LEFT_BUTTON, down: true, device: None });
    assert_eq!(t.both().await, (key(KEY_A, true), key(KEY_A, true)));
    let click = Inject::Button { button: LEFT_BUTTON, down: true };
    assert_eq!(t.both().await, (click, click));

    // C takes control back: it releases both, A carries on with B.
    assert!(ask(&t.c.dir, Request::TakeBack).await.starts_with("ok\n"));
    let mut released = vec![next_non_move(&mut t.c.h).await, next_non_move(&mut t.c.h).await];
    released.sort_by_key(|i| format!("{i:?}"));
    assert_eq!(released, [Inject::Button { button: LEFT_BUTTON, down: false }, key(KEY_A, false)]);
    eventually_async("C to drop out", || async { status(&t.a.dir).await.broadcast == ["bee"] }).await;
    t.a.send(Event::Key { code: KEY_X, down: true, device: None });
    assert_eq!(next_non_move(&mut t.b.h).await, key(KEY_X, true));
    t.a.send(Event::Key { code: KEY_X, down: false, device: None });
    assert_eq!(next_non_move(&mut t.b.h).await, key(KEY_X, false));
    tokio::time::sleep(Duration::from_millis(50)).await;
    nothing_but_moves(&mut t.c.h);

    // C restarts and reconnects: not added.
    // (On a fresh port: in this process the first one's tasks may still
    // hold its socket for a moment.)
    let dc = t.c.dir.clone();
    t.c.stop().await;
    t.c = Daemon::spawn(&dc, free_port(), vec![SocketAddr::from(([127, 0, 0, 1], t.a.port))], desk_c());
    eventually_async("C back online", || async { status(&t.a.dir).await.peers.iter().filter(|p| p.online).count() == 2 }).await;
    assert_eq!(status(&t.a.dir).await.broadcast, ["bee"], "a computer that connects later is not added");
    t.a.send(Event::Key { code: KEY_X, down: true, device: None });
    assert_eq!(next_non_move(&mut t.b.h).await, key(KEY_X, true));

    // B goes away (its daemon stops, letting go of what it held), and with
    // it the last computer: broadcast ends and A gets its pointer back.
    let db = t.b.dir.clone();
    t.b.stop().await;
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 1, pos: Some(CENTER_B) });
    assert!(status(&t.a.dir).await.broadcast.is_empty());
    tokio::time::sleep(Duration::from_millis(50)).await;
    nothing_but_moves(&mut t.c.h);
    t.b = Daemon::spawn(&db, free_port(), vec![SocketAddr::from(([127, 0, 0, 1], t.a.port))], desk_b());
    t.settle(2).await;
    t.stop().await;
}

/// A computer that lets go by its idle rule (and says so) is entered again
/// with the next input, where it was, and that input reaches it. A key held
/// since before is not released there (it never saw it pressed this time).
#[tokio::test]
async fn idle_let_go_enters_again_with_the_next_input() {
    let mut t = Trio::start("rest", Swap::Auto, |_| {}).await;
    // B's idle rule fires soon after its last input (A's other tests keep the
    // real minute).
    testing::set(&t.b.dir, None, Some(Duration::from_millis(300)));
    t.on(1).await;
    t.a.send(Event::Motion { dx: 10.0, dy: 10.0, device: None });
    assert_eq!(next_inject(&mut t.b.h).await, Inject::MoveTo(Pos::new(970.0, 550.0)));
    t.a.send(Event::Key { code: KEY_X, down: true, device: None });
    assert_eq!(t.both().await, (key(KEY_X, true), key(KEY_X, true)));
    // B lets go by its own idle rule: no input for 300 ms.
    assert_eq!(next_non_move(&mut t.b.h).await, key(KEY_X, false));
    assert_eq!(status(&t.a.dir).await.broadcast, ["bee", "sea"], "resting, still in");
    // The next key enters B again where its pointer was, then types there.
    t.a.send(Event::Key { code: KEY_A, down: true, device: None });
    assert_eq!(next_inject(&mut t.b.h).await, Inject::MoveTo(Pos::new(970.0, 550.0)));
    assert_eq!(t.both().await, (key(KEY_A, true), key(KEY_A, true)));
    t.a.keys(&[(KEY_A, false), (KEY_X, false)]);
    assert_eq!(t.both().await, (key(KEY_A, false), key(KEY_A, false)));
    assert_eq!(next_non_move(&mut t.c.h).await, key(KEY_X, false));
    tokio::time::sleep(Duration::from_millis(100)).await;
    nothing_but_moves(&mut t.b.h);
    t.stop().await;
}

/// A user who takes control back stays out, whenever they do it: the next
/// input of the broadcast does not enter that computer again.
#[tokio::test]
async fn taking_control_back_is_never_undone_by_the_next_input() {
    let mut t = Trio::start("takeback", Swap::Auto, |_| {}).await;
    t.on(1).await;
    t.a.send(Event::Key { code: KEY_X, down: true, device: None });
    assert_eq!(t.both().await, (key(KEY_X, true), key(KEY_X, true)));
    assert!(ask(&t.b.dir, Request::TakeBack).await.starts_with("ok\n"));
    assert_eq!(next_non_move(&mut t.b.h).await, key(KEY_X, false));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(status(&t.a.dir).await.broadcast, ["sea"], "B is out");
    t.a.send(Event::Key { code: KEY_A, down: true, device: None });
    assert_eq!(next_non_move(&mut t.c.h).await, key(KEY_A, true));
    tokio::time::sleep(Duration::from_millis(100)).await;
    nothing_but_moves(&mut t.b.h);
    t.a.keys(&[(KEY_A, false), (KEY_X, false)]);
    t.stop().await;
}

/// No input for the configured time ends it (and input keeps it going);
/// the emergency chord and a screen lock end it too, releasing everything.
#[tokio::test]
async fn idle_emergency_and_lock_end_it_without_stuck_keys() {
    let idle = Duration::from_millis(600);
    let mut t = Trio::start("end", Swap::Auto, |da| testing::set(da, Some(Some(idle)), None)).await;
    t.on(1).await;
    t.a.send(Event::Key { code: LSHIFT, down: true, device: None });
    assert_eq!(t.both().await, (key(LSHIFT, true), key(LSHIFT, true)));
    // Input every 200 ms for a second keeps it on.
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        t.a.send(Event::Motion { dx: 1.0, dy: 0.0, device: None });
    }
    assert!(!status(&t.a.dir).await.broadcast.is_empty());
    let quiet = Instant::now();
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 1, pos: Some(CENTER_B) });
    assert!(quiet.elapsed() >= idle - Duration::from_millis(250), "{:?}", quiet.elapsed());
    assert_eq!(t.both().await, (key(LSHIFT, false), key(LSHIFT, false)));
    t.a.send(Event::Key { code: LSHIFT, down: false, device: None });

    // Ctrl+Alt+Shift+Esc: everything held is released, Esc reaches no one.
    t.on(2).await;
    t.a.keys(&[(LCTRL, true), (LALT, true), (LSHIFT, true)]);
    for code in [LCTRL, LALT, LSHIFT] {
        assert_eq!(t.both().await, (key(code, true), key(code, true)));
    }
    let asked = Instant::now();
    t.a.send(Event::Key { code: ESC, down: true, device: None });
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 2, pos: Some(CENTER_B) });
    let mut on_b: Vec<Inject> = Vec::new();
    for _ in 0..3 {
        let (b, c) = t.both().await;
        assert_eq!(b, c);
        on_b.push(b);
    }
    assert!(asked.elapsed() < LEASE / 2, "{:?}", asked.elapsed());
    on_b.sort_by_key(|i| format!("{i:?}"));
    let mut want = vec![key(LCTRL, false), key(LALT, false), key(LSHIFT, false)];
    want.sort_by_key(|i| format!("{i:?}"));
    assert_eq!(on_b, want);
    t.a.keys(&[(ESC, false), (LCTRL, false), (LALT, false), (LSHIFT, false)]);

    // The screen locking here.
    t.on(3).await;
    t.a.send(Event::Key { code: KEY_A, down: true, device: None });
    assert_eq!(t.both().await, (key(KEY_A, true), key(KEY_A, true)));
    t.a.send(Event::ScreenLocked(true));
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 3, pos: Some(CENTER_B) });
    assert_eq!(t.both().await, (key(KEY_A, false), key(KEY_A, false)));
    let reply = ask(&t.a.dir, Request::Broadcast(Toggle::On)).await;
    assert!(reply.starts_with("error this computer's screen is locked"), "{reply}");
    t.a.send(Event::ScreenLocked(false));
    tokio::time::sleep(Duration::from_millis(100)).await;
    nothing_but_moves(&mut t.b.h);
    nothing_but_moves(&mut t.c.h);
    t.stop().await;
}

/// Each computer gets its own Ctrl/Cmd swap and key mapper: C swaps and
/// remaps Caps Lock to Esc, B does neither; each releases what it was
/// sent.
#[tokio::test]
async fn swap_and_key_rules_apply_per_computer() {
    let mut t = Trio::start("tune", Swap::On, |_| {}).await;
    let fc = t.c.fp.clone();
    set(&t.a.dir, |s| {
        s.input.keymap.rule = vec![KeyRule { from: "capslock".into(), to: Some(To::One("esc".into())), scope: Scope::Peer(fc), ..Default::default() }];
    })
    .await;
    t.on(1).await;
    t.a.keys(&[(LCTRL, true), (CAPS, true), (CAPS, false)]);
    assert_eq!(t.both().await, (key(LCTRL, true), key(125, true)), "C swaps Ctrl for Cmd");
    assert_eq!(t.both().await, (key(CAPS, true), key(ESC, true)), "only C remaps Caps Lock");
    assert_eq!(t.both().await, (key(CAPS, false), key(ESC, false)));
    assert_eq!(ask(&t.a.dir, Request::Broadcast(Toggle::Toggle)).await, "ok\noff\n");
    assert_eq!(t.both().await, (key(LCTRL, false), key(125, false)), "each releases the code it was sent");
    t.stop().await;
}

/// A key mapped to an action asks for it once per press, not once per
/// computer: toggling the pointer lock twice would undo it.
#[tokio::test]
async fn a_key_action_runs_once_for_all_computers() {
    const F12: u32 = 88;
    let mut t = Trio::start("action", Swap::Auto, |_| {}).await;
    set(&t.a.dir, |s| s.input.keymap.rule = vec![KeyRule { from: "f12".into(), to: Some(To::One("lock-pointer".into())), ..Default::default() }]).await;
    t.on(1).await;
    t.a.keys(&[(F12, true), (F12, false)]);
    eventually_async("the lock", || async { status(&t.a.dir).await.pointer_locked }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(status(&t.a.dir).await.pointer_locked, "toggled once, not once per computer");
    assert_eq!(status(&t.a.dir).await.broadcast, ["bee", "sea"], "and broadcast carries on");
    nothing_but_moves(&mut t.b.h);
    nothing_but_moves(&mut t.c.h);
    t.stop().await;
}

/// Starting while capture doesn't start gives up after a second, says
/// why, and a capture that starts after all is handed straight back.
#[tokio::test]
async fn a_capture_that_never_starts_gives_up() {
    let mut t = Trio::start("nocap", Swap::Auto, |_| {}).await;
    let reply = ask(&t.a.dir, Request::Broadcast(Toggle::On)).await;
    assert!(reply.starts_with("error this computer's input capture did not start"), "{reply}");
    assert!(matches!(next_capture(&mut t.a.h).await, CaptureCmd::Activate { .. }));
    t.a.send(Event::Activated { id: 9, side: Side::Right, pos: Pos::new(1919.0, 500.0) });
    assert_eq!(next_capture_skipping_edges(&mut t.a.h).await, CaptureCmd::Release { id: 9, pos: None });
    assert!(status(&t.a.dir).await.broadcast.is_empty());
    tokio::time::sleep(Duration::from_millis(100)).await;
    nothing_but_moves(&mut t.b.h);
    t.stop().await;
}

/// The key-state invariant of a single visit, for every computer in a
/// broadcast, under random presses, releases, clicks, scrolls, movement,
/// computers dropping out and coming back: no computer is ever sent the
/// release of something it doesn't hold, or a press of something it
/// already holds, and when broadcast ends nothing is held anywhere,
/// before any lease could have tidied up.
#[tokio::test]
async fn random_presses_drops_and_returns_never_leave_a_key_held() {
    for (seed, rest) in [(0x5eed_u64, true), (0xb0b, false), (0xfeed_f00d, true)] {
        let mut t = Trio::start(&format!("prop-{seed:x}"), Swap::Auto, |_| {}).await;
        if rest {
            // The receivers' idle rules fire at random, as the sleeps allow.
            for dir in [&t.b.dir, &t.c.dir] {
                testing::set(dir, None, Some(Duration::from_millis(40)));
            }
        }
        t.on(1).await;
        let mut rng = seed;
        let mut next = move |n: u64| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % n
        };
        let pool = [KEY_A, KEY_X, LSHIFT, LCTRL, LEFT_BUTTON, 0x111];
        let mut held: Vec<u32> = Vec::new();
        for _ in 0..160 {
            match next(10) {
                0..=3 => {
                    let code = pool[next(pool.len() as u64) as usize];
                    let down = !held.contains(&code);
                    if down { held.push(code) } else { held.retain(|&c| c != code) }
                    t.a.send(if code >= LEFT_BUTTON { Event::Button { button: code, down, device: None } } else { Event::Key { code, down, device: None } });
                }
                4 => t.a.send(Event::Motion { dx: next(41) as f64 - 20.0, dy: next(41) as f64 - 20.0, device: None }),
                5 => t.a.send(Event::ScrollDiscrete { dx: 0, dy: 1, device: None }),
                6 if next(4) == 0 => {
                    let peer = if next(2) == 0 { &t.b.dir } else { &t.c.dir };
                    let _ = ask(peer, Request::TakeBack).await;
                }
                _ => tokio::time::sleep(Duration::from_millis(next(8))).await,
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let reply = ask(&t.a.dir, Request::Broadcast(Toggle::Off)).await;
        assert!(reply == "ok\noff\n" || reply == "already\noff\n", "{reply}");
        // Well inside the 750 ms lease: what is released by now was released
        // by the end of the broadcast, not by a timeout.
        tokio::time::sleep(Duration::from_millis(250)).await;
        for (who, h) in [("B", &mut t.b.h), ("C", &mut t.c.h)] {
            let mut down: Vec<u32> = Vec::new();
            for i in injected(h) {
                let (code, press) = match i {
                    Inject::Key { code, down } => (code, down),
                    Inject::Button { button, down } => (button, down),
                    _ => continue,
                };
                if press {
                    assert!(!down.contains(&code), "seed {seed:x}: {who} pressed {code:#x} twice");
                    down.push(code);
                } else {
                    assert!(down.contains(&code), "seed {seed:x}: {who} released {code:#x}, not held");
                    down.retain(|&c| c != code);
                }
            }
            assert!(down.is_empty(), "seed {seed:x}: {who} still holds {down:?}");
        }
        for code in held {
            t.a.send(if code >= LEFT_BUTTON { Event::Button { button: code, down: false, device: None } } else { Event::Key { code, down: false, device: None } });
        }
        t.stop().await;
    }
}

/// A keyboard that goes away (unplugged, Bluetooth asleep) while it holds a
/// key: every computer of the broadcast lets go of it at once, even though
/// no visit exists between events.
#[tokio::test]
async fn an_unplugged_keyboard_is_let_go_of_by_every_computer_of_a_broadcast() {
    use one_mk_input::{DeviceRef, fake_keyboard};
    let mut t = Trio::start("unplug", Swap::Auto, |_| {}).await;
    t.a.h.devices(vec![fake_keyboard(7, "Keyboard")]);
    t.on(871).await;
    t.a.h.key_from(KEY_A, true, Some(DeviceRef(7)));
    assert_eq!(t.both().await, (key(KEY_A, true), key(KEY_A, true)));
    t.a.h.devices(vec![]);
    let released = tokio::time::timeout(Duration::from_millis(500), t.both()).await;
    t.stop().await;
    assert_eq!(released.ok(), Some((key(KEY_A, false), key(KEY_A, false))));
}
