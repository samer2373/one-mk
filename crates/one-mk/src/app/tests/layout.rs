//! Computers placed at an offset (crossing only where their screens
//! touch), walls where nothing touches an edge, crossings between
//! multi-monitor desktops, profiles, and layout sync with peers old and
//! new, between real daemons on loopback.

use super::*;

fn screen(w: i32, h: i32) -> Desktop {
    Desktop::new(vec![Rect::new(0, 0, w, h)])
}

async fn status(dir: &std::path::Path) -> Status {
    Status::decode(&control::request(dir, Request::Status).await.unwrap().expect("daemon running")).unwrap()
}

/// The segment of a `side` edge on the line `at`, from `lo` to `hi`.
fn edge(side: Side, at: i32, lo: i32, hi: i32) -> Edge {
    Edge { side, at, lo, hi }
}

/// What a computer arms when exactly `e` leads to its peer.
fn only(e: Edge) -> CaptureCmd {
    CaptureCmd::SetEdges(vec![e])
}

/// A release at `pos`, x rounded (homecoming lands 1/256 px inside a screen).
fn released_near(cmd: CaptureCmd) -> (u32, Pos) {
    match cmd {
        CaptureCmd::Release { id, pos: Some(p) } => (id, Pos::new(p.x.round(), p.y)),
        other => panic!("expected a release, got {other:?}"),
    }
}

/// `dir`'s record of the device with fingerprint `fp`.
fn record(dir: &std::path::Path, fp: &str) -> Device {
    TrustStore::load(dir).unwrap().find(&fp[..8]).unwrap().clone()
}

/// A crossing at a point of A's edge that the laptop touches maps 1:1 by
/// the offset; above it, and back from the laptop below the end of A's
/// edge, the edge is a wall.
#[tokio::test]
async fn offset_crossing_maps_and_walls() {
    // B: a small laptop to A's right, its top edge 300 px below A's.
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((1920, 300));
        b_sees_a.offset = Some((-1920, -300));
    };
    let mut duo = Duo::start_on("offset", None, setup, screen(1920, 1080), screen(1280, 800)).await;
    // Only where the screens touch is armed: A's edge from 300 down, the
    // laptop's down to A's bottom (780 on its edge).
    duo.settle_as(only(edge(Side::Right, 1920, 300, 1080)), only(edge(Side::Left, 0, 0, 780))).await;

    // Above the laptop: nothing there, the pointer stays on A (a backend
    // armed as asked never reports this; the daemon refuses it anyway).
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 100.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: None });
    assert_eq!(status(&duo.da).await.controlling, None);

    // Facing the laptop: it enters 300 px higher up, 1:1.
    duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 1000.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 700.0)));

    // To the laptop's bottom-left corner, below the end of A's edge (A's
    // 1080 is the laptop's 780): pushing left is a wall.
    duo.ha.events.send(Event::Motion { dx: 0.0, dy: 99.0 }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 799.0)));
    duo.ha.events.send(Event::Motion { dx: -50.0, dy: 0.0 }).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(duo.ha.capture.try_recv().is_err(), "the pointer must not come home there");
    assert!(status(&duo.da).await.controlling.is_some());

    // Higher up it comes home, at the matching point of A's edge.
    duo.ha.events.send(Event::Motion { dx: 0.0, dy: -100.0 }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 699.0)));
    duo.ha.events.send(Event::Motion { dx: -5.0, dy: 0.0 }).unwrap();
    assert_eq!(released_near(next_capture(&mut duo.ha).await), (2, Pos::new(1918.0, 999.0)));
    duo.stop().await;
}

/// A dwell at an edge inside A's bounding box: B faces the part of A's
/// main monitor's right edge below A's shorter second monitor. Pushing
/// outwards there (towards B) keeps the pointer held until the dwell
/// ends; moving back into the desktop hands it back.
#[tokio::test]
async fn dwell_at_an_inner_edge() {
    let a = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 720)]);
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((1920, 720));
        b_sees_a.offset = Some((-1920, -720));
    };
    let mut duo = Duo::start_on("inner-dwell", None, setup, a, screen(1280, 800)).await;
    duo.settle_as(only(edge(Side::Right, 1920, 720, 1080)), only(edge(Side::Left, 0, 0, 360))).await;
    super::set(&duo.da, |s| s.edge.all.dwell_ms = Some(300)).await;

    let started = Instant::now();
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 900.0) }).unwrap();
    duo.ha.events.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
    duo.ha.events.send(Event::Motion { dx: 40.0, dy: 10.0 }).unwrap(); // pushing, sliding down
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 190.0)));
    assert!(started.elapsed() >= Duration::from_millis(300), "{:?}", started.elapsed());
    assert!(duo.ha.capture.try_recv().is_err(), "never handed back");
    duo.leave_b().await;

    duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 900.0) }).unwrap();
    duo.ha.events.send(Event::Motion { dx: -30.0, dy: 0.0 }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 2, pos: Some(Pos::new(1889.0, 900.0)) });
    duo.stop().await;
}

/// From A's second, portrait monitor (not its primary) to B's upper
/// monitor (not B's primary either), and back to the same point.
#[tokio::test]
async fn crossing_between_multi_monitor_desktops() {
    let a = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, -240, 1080, 1920)]);
    let b = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(0, -1080, 1920, 1080)]);
    // B's desktop starts 600 px above A's (its top, -1080 in its pixels,
    // at -840 in A's), right of A's portrait monitor.
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((3000, 240));
        b_sees_a.offset = Some((-3000, -240));
    };
    let mut duo = Duo::start_on("multimon", None, setup, a, b).await;
    // Each of B's monitors touches A's portrait one: two segments on each.
    let a_arms = CaptureCmd::SetEdges(vec![edge(Side::Right, 3000, 240, 1320), edge(Side::Right, 3000, -240, 240)]);
    let b_arms = CaptureCmd::SetEdges(vec![edge(Side::Left, 0, 0, 1080), edge(Side::Left, 0, -480, 0)]);
    duo.settle_as(a_arms, b_arms).await;
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(2999.0, -200.0) }).unwrap();
    // -200 on A is 40 below the top of A's desktop, 640 below the top of
    // B's: -440 on B, on its upper monitor.
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, -440.0)));
    duo.ha.events.send(Event::Motion { dx: -1.0, dy: 0.0 }).unwrap();
    assert_eq!(released_near(next_capture(&mut duo.ha).await), (1, Pos::new(2998.0, -200.0)));
    duo.stop().await;
}

/// `one-mk layout` with an offset applies at once on both computers: the
/// other one stores the mirror image under the same version, and either
/// side can change it again.
#[tokio::test]
async fn offset_change_converges_as_a_mirror_image() {
    let mut duo = Duo::start("offset-sync", None).await;
    let (fa, fb) = (duo.fa.clone(), duo.fb.clone());
    // B below A, its left edge 300 px right of A's.
    crate::manage::set_layout(&duo.da, &fb[..8], Some(Side::Bottom), Some(Some((300, 1080))), None).unwrap();
    assert!(control::request(&duo.da, Request::Reload).await.unwrap().is_some());
    assert_eq!(next_capture(&mut duo.ha).await, only(edge(Side::Bottom, 1080, 300, 1920)));
    assert_eq!(next_capture(&mut duo.hb).await, only(edge(Side::Top, 0, 0, 1620)));
    let (a_sees_b, b_sees_a) = (record(&duo.da, &fb), record(&duo.db, &fa));
    assert_eq!((a_sees_b.side(), a_sees_b.offset), (Side::Bottom, Some((300, 1080))));
    assert_eq!((b_sees_a.side(), b_sees_a.offset), (Side::Top, Some((-300, -1080))));
    assert_eq!(a_sees_b.layout_version, b_sees_a.layout_version);
    let s = status(&duo.db).await;
    assert_eq!((s.peers[0].side, s.offset_of(&s.peers[0].id)), (Side::Top, Some((-300, -1080))));
    assert_eq!((&s.peers[0].screens, &s.screens), (&vec![Rect::new(0, 0, 1920, 1080)], &vec![Rect::new(0, 0, 1920, 1080)]));

    // B puts A straight above it instead (the side stays).
    crate::manage::set_layout(&duo.db, &fa[..8], None, Some(Some((0, -1080))), None).unwrap();
    assert!(control::request(&duo.db, Request::Reload).await.unwrap().is_some());
    let fb2 = fb.clone();
    let da = duo.da.clone();
    eventually("A to adopt B's change", move || record(&da, &fb2).offset == Some((0, 1080))).await;
    assert_eq!(record(&duo.da, &fb).side(), Side::Bottom);
    // Same size, lined up: straight down.
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Bottom, pos: Pos::new(960.0, 1079.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(960.0, 0.0)));
    duo.stop().await;
}

/// A peer from before offsets sends layouts without one: on the same side
/// our offset stands, on a new side it is dropped, and an older version
/// changes nothing. It is sent ours as trailing bytes it skips.
#[tokio::test]
async fn layout_from_a_peer_without_offset() {
    let duo = Duo::start("old-peer", None).await;
    let mut old = RawPeer::connect_as("old-peer-c", &duo.db, duo.pb, |d| {
        d.side = Side::Top.to_string();
        d.offset = Some((50, -1080));
        d.layout_version = 5;
    })
    .await;
    // B told it where it is: below it, B's (0,0) 50 px left of its.
    assert_eq!(old.layout, Control::Layout { side_of_you: Side::Bottom, version: 5, offset: Some((50, -1080)) });
    let fc = Identity::load_or_create(&old.dir).unwrap().fingerprint.to_string();
    let old_layout = |side: u8, version: u64| {
        let mut body = vec![13, side];
        body.extend_from_slice(&version.to_le_bytes());
        body
    };
    // Same side, newer: B keeps the offset.
    old.send_body(&old_layout(2, 10)).await;
    let (db, f) = (duo.db.clone(), fc.clone());
    eventually("version 10", move || record(&db, &f).layout_version == 10).await;
    assert_eq!((record(&duo.db, &fc).side(), record(&duo.db, &fc).offset), (Side::Top, Some((50, -1080))));
    // A new side: anywhere along it.
    old.send_body(&old_layout(0, 11)).await;
    let (db, f) = (duo.db.clone(), fc.clone());
    eventually("version 11", move || record(&db, &f).layout_version == 11).await;
    assert_eq!((record(&duo.db, &fc).side(), record(&duo.db, &fc).offset), (Side::Left, None));
    // Older than what B has: ignored.
    old.send_body(&old_layout(1, 3)).await;
    old.send(Control::Layout { side_of_you: Side::Bottom, version: 4, offset: Some((0, 1080)) }).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let d = record(&duo.db, &fc);
    assert_eq!((d.side(), d.offset, d.layout_version), (Side::Left, None, 11));
    // The link is still fine.
    assert!(status(&duo.db).await.peers.iter().any(|p| p.online && p.id == d.short_id()));
    drop(old);
    duo.stop().await;
}

/// Two computers on B's left, one above the other: each point of B's edge
/// leads to the one facing it.
#[tokio::test]
async fn two_neighbours_share_one_edge() {
    // A covers the top half of B's left edge (A's desktop starts 540 px
    // higher), C the bottom half.
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((1920, 540));
        b_sees_a.offset = Some((-1920, -540));
    };
    let mut duo = Duo::start_with("two-left", None, setup).await;
    duo.settle_as(only(edge(Side::Right, 1920, 540, 1080)), only(edge(Side::Left, 0, 0, 540))).await;
    let mut c = RawPeer::connect_as("two-left-c", &duo.db, duo.pb, |d| {
        d.side = Side::Left.to_string();
        d.offset = Some((-1920, 540));
    })
    .await;
    c.send(Control::Screens(vec![Rect::new(0, 0, 1920, 1080)])).await;
    let db = duo.db.clone();
    for _ in 0..250 {
        if status(&db).await.peers.iter().filter(|p| !p.screens.is_empty()).count() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Between them they cover all of B's left edge.
    match next_capture(&mut duo.hb).await {
        CaptureCmd::SetEdges(mut e) => {
            e.sort_by_key(|e| e.lo);
            assert_eq!(e, [edge(Side::Left, 0, 0, 540), edge(Side::Left, 0, 540, 1080)]);
        }
        other => panic!("expected edges, got {other:?}"),
    }

    // Entering a right edge lands 1/256 px inside it.
    let right = 1920.0 - 1.0 / 256.0;
    duo.hb.events.send(Event::Activated { id: 1, side: Side::Left, pos: Pos::new(0.0, 100.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.ha).await, Inject::MoveTo(Pos::new(right, 640.0)));
    duo.hb.events.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
    assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(2.0, 100.0)) });

    duo.hb.events.send(Event::Activated { id: 2, side: Side::Left, pos: Pos::new(0.0, 900.0) }).unwrap();
    loop {
        match c.next().await {
            Control::Enter { pos, .. } => break assert_eq!(pos, Pos::new(right, 360.0)),
            _ => continue,
        }
    }
    drop(c);
    duo.stop().await;
}

/// A profile that switches by itself goes into effect when the network
/// matches (once per change of network), syncs to the peer like any
/// layout change, and leaves later tweaks alone until the network changes.
#[tokio::test]
async fn profile_switches_with_the_network() {
    use crate::profiles::{NetworkId, ProfileCmd, Profiles};
    let mut duo = Duo::start("auto-profile", None).await;
    let (da, fb, fa) = (duo.da.clone(), duo.fb.clone(), duo.fa.clone());
    // At the desk B is below, 300 px to the right; at home on the right.
    crate::manage::set_layout(&da, &fb[..8], Some(Side::Bottom), Some(Some((300, 1080))), None).unwrap();
    crate::profiles::run(&da, ProfileCmd::Save { name: "desk".into() }).await.unwrap();
    crate::manage::set_layout(&da, &fb[..8], Some(Side::Right), Some(None), None).unwrap();
    crate::profiles::run(&da, ProfileCmd::Save { name: "home".into() }).await.unwrap();
    let auto = ProfileCmd::Auto { name: "desk".into(), ssid: vec!["Desk WiFi".into()], network: vec![], off: false, clear: false };
    let watching = crate::netwatch::fake::watchers();
    crate::profiles::run(&da, auto).await.unwrap(); // reloads A, which starts watching
    eventually("A to watch the network", || crate::netwatch::fake::watchers() > watching).await;
    let s = status(&da).await;
    assert_eq!((s.layout.profile.as_deref(), s.layout.profiles.len()), (Some("home"), 2));

    let desk = NetworkId { ssids: vec!["Desk WiFi".into()], ..NetworkId::default() };
    crate::netwatch::fake::announce(&desk);
    assert_eq!(next_capture(&mut duo.ha).await, only(edge(Side::Bottom, 1080, 300, 1920)));
    assert_eq!(next_capture(&mut duo.hb).await, only(edge(Side::Top, 0, 0, 1620)));
    let b_sees_a = record(&duo.db, &fa);
    assert_eq!((b_sees_a.side(), b_sees_a.offset), (Side::Top, Some((-300, -1080))));
    assert_eq!(Profiles::load(&da).unwrap().active.as_deref(), Some("desk"));

    // A tweak, then the same network again (a reconnect): the tweak stands.
    crate::manage::set_layout(&da, &fb[..8], None, Some(Some((100, 1080))), None).unwrap();
    assert!(control::request(&da, Request::Reload).await.unwrap().is_some());
    crate::netwatch::fake::announce(&desk);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(record(&da, &fb).offset, Some((100, 1080)));
    // Elsewhere, then back at the desk: the profile again.
    crate::netwatch::fake::announce(&NetworkId { ssids: vec!["Café".into()], ..NetworkId::default() });
    crate::netwatch::fake::announce(&desk);
    let (db, fa2) = (duo.db.clone(), fa.clone());
    eventually("B to get the desk layout back", move || record(&db, &fa2).offset == Some((-300, -1080))).await;
    assert_eq!(record(&da, &fb).offset, Some((300, 1080)));
    duo.stop().await;
}

/// An offset past the end of the edge (`one-mk layout B right --offset
/// 1920,5000`): nothing crosses and status says why, but `one-mk switch`
/// still reaches B, and from there all of B's edge facing A leads home
/// (there is no touching part to aim for).
#[tokio::test]
async fn no_overlap_warns_and_a_switch_still_comes_home() {
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((1920, 5000));
        b_sees_a.offset = Some((-1920, -5000));
    };
    let mut duo = Duo::start_on("no-overlap", None, setup, screen(1920, 1080), screen(1280, 800)).await;
    let da = duo.da.clone();
    let warned = || async {
        let st = control::request(&da, Request::Status).await.ok().flatten().and_then(|t| Status::decode(&t));
        st.is_some_and(|s| s.layout.warnings.iter().any(|w| w.contains("does not overlap")))
    };
    eventually_async("the warning", warned).await;
    while duo.ha.capture.try_recv().is_ok() {}

    let fb = duo.fb.clone();
    assert!(ask(&duo.da, switch_to(&fb)).await.starts_with("ok\n"));
    loop {
        match next_capture(&mut duo.ha).await {
            CaptureCmd::Activate { side } => break assert_eq!(side, Side::Right),
            CaptureCmd::SetEdges(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    duo.ha.events.send(Event::Activated { id: 3, side: Side::Right, pos: Pos::new(1919.0, 500.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(640.0, 400.0)));
    duo.ha.events.send(Event::Motion { dx: -2000.0, dy: 0.0 }).unwrap();
    loop {
        match next_capture(&mut duo.ha).await {
            CaptureCmd::Release { id: 3, pos: Some(p) } => break assert_eq!(p.x.round(), 1918.0, "just inside A's right edge: {p:?}"),
            CaptureCmd::SetEdges(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    duo.stop().await;
}

/// Capture at the wall part of an edge (where a backend that can't limit
/// its barriers to the shared part would capture): the pointer is handed
/// straight back, and keys and clicks caught before that took effect are
/// typed here, whole, instead of being lost.
#[tokio::test]
async fn input_caught_at_a_wall_is_typed_here() {
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((1920, 300));
        b_sees_a.offset = Some((-1920, -300));
    };
    let mut duo = Duo::start_on("wall-input", None, setup, screen(1920, 1080), screen(1280, 800)).await;
    duo.settle_as(only(edge(Side::Right, 1920, 300, 1080)), only(edge(Side::Left, 0, 0, 780))).await;
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 100.0) }).unwrap();
    for ev in [
        Event::Key { code: 30, down: true },
        Event::Key { code: 30, down: false },
        Event::Button { button: 0x110, down: true },
        Event::ScrollDiscrete { dx: 0, dy: 1 },
        Event::Button { button: 0x110, down: false },
        // Went down here before the capture: its release is due here.
        Event::Key { code: 31, down: false },
    ] {
        duo.ha.events.send(ev).unwrap();
    }
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: None });
    let mut got = Vec::new();
    for _ in 0..6 {
        got.push(next_inject(&mut duo.ha).await);
    }
    assert_eq!(
        got,
        [
            Inject::Key { code: 30, down: true },
            Inject::Key { code: 30, down: false },
            Inject::Button { button: 0x110, down: true },
            Inject::Button { button: 0x110, down: false },
            Inject::ScrollDiscrete { dx: 0, dy: 1 },
            Inject::Key { code: 31, down: false },
        ]
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(injected(&mut duo.ha).is_empty(), "the captured releases of taps add nothing");
    assert!(injected(&mut duo.hb).is_empty(), "nothing went to B");
    duo.stop().await;
}

/// Both computers change the layout at the same moment (the same
/// millisecond, so the same version) to different, conflicting offsets.
/// The computer with the larger fingerprint wins: both end up holding its
/// layout, as mirror images, and at most one more `Layout` goes out after
/// the two that announced the changes (none if the loser adopted first).
#[tokio::test]
async fn simultaneous_changes_converge() {
    let duo = Duo::start("tie", None).await;
    let (fa, fb) = (duo.fa.clone(), duo.fb.clone());
    let sent = |dir: &std::path::Path| layouts_sent_by(dir).load(std::sync::atomic::Ordering::SeqCst);
    let before = sent(&duo.da) + sent(&duo.db);
    const NOW: u64 = 4_102_444_800_000;
    crate::manage::set_layout_at(&duo.da, &fb[..8], Some(Side::Bottom), Some(Some((50, 1080))), None, NOW).unwrap();
    crate::manage::set_layout_at(&duo.db, &fa[..8], Some(Side::Top), Some(Some((35, -1080))), None, NOW).unwrap();
    assert!(control::request(&duo.da, Request::Reload).await.unwrap().is_some());
    assert!(control::request(&duo.db, Request::Reload).await.unwrap().is_some());

    // A puts B at (50, 1080); B puts A at (35, -1080), so B at (-35, 1080)
    // in A's terms.
    let a_wins = fa > fb;
    let want = if a_wins { (50, 1080) } else { (-35, 1080) };
    let mirror = |(x, y): (i32, i32)| (-x, -y);
    let (da, db, fa2, fb2) = (duo.da.clone(), duo.db.clone(), fa.clone(), fb.clone());
    eventually("both to hold the winner's layout", move || {
        let (x, y) = (record(&da, &fb2), record(&db, &fa2));
        x.offset == Some(want) && y.offset == Some(mirror(want)) && x.layout_version == y.layout_version
    })
    .await;
    // Quiet again: no ping-pong, however long we look.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (a_sees_b, b_sees_a) = (record(&duo.da, &fb), record(&duo.db, &fa));
    assert_eq!((a_sees_b.side(), b_sees_a.side()), (Side::Bottom, Side::Top));
    assert_eq!(b_sees_a.offset, a_sees_b.offset.map(mirror));
    assert_eq!(a_sees_b.layout_version, b_sees_a.layout_version);
    // Either the loser adopted before announcing anything (nothing to
    // settle, so nothing more is sent), or both announced and the winner sent
    // its layout once more under a newer version, so that peers that don't
    // know the rule adopt it too. Never more.
    let resent = match a_sees_b.layout_version {
        NOW => 0,
        v if v == NOW + 1 => 1,
        v => panic!("unexpected version {v}"),
    };
    assert_eq!(sent(&duo.da) + sent(&duo.db), before + 2 + resent);
    duo.stop().await;
}

/// A peer from before the tie rule sends a different layout under the
/// version B holds. Whoever has the larger fingerprint wins: if it is the
/// old peer, B adopts its layout (and says nothing); if it is B, B sends the
/// old peer its own under a newer version, which the old peer adopts.
#[tokio::test]
async fn tie_with_an_old_peer_converges_either_way() {
    for peer_wins in [true, false] {
        let tag = format!("old-tie-{peer_wins}");
        let duo = Duo::start(&tag, None).await;
        let b_fp = Identity::load_or_create(&duo.db).unwrap().fingerprint;
        let dir = (0..)
            .map(|i| temp_dir(&format!("{tag}-c{i}")))
            .find(|d| (Identity::load_or_create(d).unwrap().fingerprint > b_fp) == peer_wins)
            .unwrap();
        let sent = layouts_sent_by(&duo.db);
        let before = sent.load(std::sync::atomic::Ordering::SeqCst);
        let mut old = RawPeer::connect_in(dir, &duo.db, duo.pb, |d| {
            d.side = Side::Top.to_string();
            d.offset = Some((50, -1080));
            d.layout_version = 5;
        })
        .await;
        let fc = Identity::load_or_create(&old.dir).unwrap().fingerprint.to_string();
        // B holds Top, offset (50, -1080), version 5. The peer says it is on B's left instead.
        let mut body = vec![13, 0];
        body.extend_from_slice(&5u64.to_le_bytes());
        old.send_body(&body).await;
        let (db, f) = (duo.db.clone(), fc.clone());
        if peer_wins {
            eventually("B to adopt the old peer's layout", move || record(&db, &f).side() == Side::Left).await;
            assert_eq!(record(&duo.db, &fc).layout_version, 5);
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), before + 1, "only the layout sent on connecting");
        } else {
            // B keeps its layout and sends it again, newer.
            let again = old.next().await;
            assert_eq!(again, Control::Layout { side_of_you: Side::Bottom, version: 6, offset: Some((50, -1080)) });
            assert_eq!(record(&duo.db, &fc).layout_version, 6);
            assert_eq!(record(&duo.db, &fc).side(), Side::Top);
            // The old peer adopts it; that settles it, and B stays quiet.
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), before + 2);
        }
        drop(old);
        duo.stop().await;
    }
}
