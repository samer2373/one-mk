//! Edges that line up with an offset, walls where nothing faces an edge,
//! crossings between multi-monitor desktops, and layout sync with peers
//! old and new, between real daemons on loopback.

use super::*;

fn screen(w: i32, h: i32) -> Desktop {
    Desktop::new(vec![Rect::new(0, 0, w, h)])
}

async fn status(dir: &std::path::Path) -> Status {
    Status::decode(&control::request(dir, Request::Status).await.unwrap().expect("daemon running")).unwrap()
}

/// `dir`'s record of the device with fingerprint `fp`.
fn record(dir: &std::path::Path, fp: &str) -> Device {
    TrustStore::load(dir).unwrap().find(&fp[..8]).unwrap().clone()
}

/// A crossing at a point of A's edge that the laptop faces maps 1:1 with
/// the offset; above it, and back from the laptop below the end of A's
/// edge, the edge is a wall.
#[tokio::test]
async fn offset_crossing_maps_and_walls() {
    // B: a small laptop to A's right, its top edge 300 px below A's.
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.set_align(Align::Offset(300));
        b_sees_a.set_align(Align::Offset(-300));
    };
    let mut duo = Duo::start_on("offset", None, setup, screen(1920, 1080), screen(1280, 800)).await;
    duo.settle().await;

    // Above the laptop: nothing there, the pointer stays on A.
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
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 2, pos: Some(Pos::new(1917.0, 999.0)) });
    duo.stop().await;
}

/// From A's second, portrait monitor (not its primary) to B's upper
/// monitor (not B's primary either), and back to the same point.
#[tokio::test]
async fn crossing_between_multi_monitor_desktops() {
    let a = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, -240, 1080, 1920)]);
    let b = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(0, -1080, 1920, 1080)]);
    // B's desktop starts 600 px above A's.
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.set_align(Align::Offset(-600));
        b_sees_a.set_align(Align::Offset(600));
    };
    let mut duo = Duo::start_on("multimon", None, setup, a, b).await;
    duo.settle().await;
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(2999.0, -200.0) }).unwrap();
    // -200 on A is 40 below the top of A's desktop, 640 below the top of
    // B's: -440 on B, on its upper monitor.
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, -440.0)));
    duo.ha.events.send(Event::Motion { dx: -1.0, dy: 0.0 }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(2997.0, -200.0)) });
    duo.stop().await;
}

/// `one-mk layout` with an offset applies at once on both computers: the
/// other one stores the mirror image under the same version, and either
/// side can change it again.
#[tokio::test]
async fn offset_change_converges_as_a_mirror_image() {
    let mut duo = Duo::start("offset-sync", None).await;
    let (fa, fb) = (duo.fa.clone(), duo.fb.clone());
    crate::manage::set_layout(&duo.da, &fb[..8], Some(Side::Bottom), Some(Align::Offset(300)), None).unwrap();
    assert!(control::request(&duo.da, Request::Reload).await.unwrap().is_some());
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Side::Bottom]));
    assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Side::Top]));
    let (a_sees_b, b_sees_a) = (record(&duo.da, &fb), record(&duo.db, &fa));
    assert_eq!((a_sees_b.side(), a_sees_b.align()), (Side::Bottom, Align::Offset(300)));
    assert_eq!((b_sees_a.side(), b_sees_a.align()), (Side::Top, Align::Offset(-300)));
    assert_eq!(a_sees_b.layout_version, b_sees_a.layout_version);
    let s = status(&duo.db).await;
    let place = s.layout.place(&s.peers[0].id).unwrap();
    assert_eq!((s.peers[0].side, place.align), (Side::Top, Align::Offset(-300)));
    assert_eq!((&place.screens, &s.layout.screens), (&vec![Rect::new(0, 0, 1920, 1080)], &vec![Rect::new(0, 0, 1920, 1080)]));

    // B centres A instead (the side stays, so no edge is re-armed).
    crate::manage::set_layout(&duo.db, &fa[..8], None, Some(Align::Center), None).unwrap();
    assert!(control::request(&duo.db, Request::Reload).await.unwrap().is_some());
    let fb2 = fb.clone();
    let da = duo.da.clone();
    eventually("A to adopt B's change", move || record(&da, &fb2).align() == Align::Center).await;
    assert_eq!(record(&duo.da, &fb).side(), Side::Bottom);
    // Same size, centred: straight down.
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Bottom, pos: Pos::new(960.0, 1079.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(960.0, 0.0)));
    duo.stop().await;
}

/// A peer from before alignment sends layouts without it: on the same side
/// our alignment stands, on a new side it starts stretched, and an older
/// version changes nothing. It is sent ours as trailing bytes it skips.
#[tokio::test]
async fn layout_from_a_peer_without_alignment() {
    let duo = Duo::start("old-peer", None).await;
    let mut old = RawPeer::connect_as("old-peer-c", &duo.db, duo.pb, |d| {
        d.side = Side::Top.to_string();
        d.set_align(Align::Offset(50));
        d.layout_version = 5;
    })
    .await;
    // B told it where it is: below it, B's edge 50 px before its.
    assert_eq!(old.layout, Control::Layout { side_of_you: Side::Bottom, version: 5, align: Some(Align::Offset(-50)) });
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
    assert_eq!((record(&duo.db, &fc).side(), record(&duo.db, &fc).align()), (Side::Top, Align::Offset(50)));
    // A new side: stretched.
    old.send_body(&old_layout(0, 11)).await;
    let (db, f) = (duo.db.clone(), fc.clone());
    eventually("version 11", move || record(&db, &f).layout_version == 11).await;
    assert_eq!((record(&duo.db, &fc).side(), record(&duo.db, &fc).align()), (Side::Left, Align::Stretch));
    // Older than what B has: ignored.
    old.send_body(&old_layout(1, 3)).await;
    old.send(Control::Layout { side_of_you: Side::Bottom, version: 4, align: Some(Align::Center) }).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let d = record(&duo.db, &fc);
    assert_eq!((d.side(), d.align(), d.layout_version), (Side::Left, Align::Stretch, 11));
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
        a_sees_b.set_align(Align::Offset(540));
        b_sees_a.set_align(Align::Offset(-540));
    };
    let mut duo = Duo::start_with("two-left", None, setup).await;
    duo.settle().await;
    let mut c = RawPeer::connect_as("two-left-c", &duo.db, duo.pb, |d| {
        d.side = Side::Left.to_string();
        d.set_align(Align::Offset(540));
    })
    .await;
    c.send(Control::Screens(vec![Rect::new(0, 0, 1920, 1080)])).await;
    let db = duo.db.clone();
    for _ in 0..250 {
        if status(&db).await.layout.places.iter().filter(|p| !p.screens.is_empty()).count() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    duo.hb.events.send(Event::Activated { id: 1, side: Side::Left, pos: Pos::new(0.0, 100.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.ha).await, Inject::MoveTo(Pos::new(1919.0, 640.0)));
    duo.hb.events.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
    assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(2.0, 100.0)) });

    duo.hb.events.send(Event::Activated { id: 2, side: Side::Left, pos: Pos::new(0.0, 900.0) }).unwrap();
    loop {
        match c.next().await {
            Control::Enter { pos, .. } => break assert_eq!(pos, Pos::new(1919.0, 360.0)),
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
    crate::manage::set_layout(&da, &fb[..8], Some(Side::Bottom), Some(Align::Offset(300)), None).unwrap();
    crate::profiles::run(&da, ProfileCmd::Save { name: "desk".into() }).await.unwrap();
    crate::manage::set_layout(&da, &fb[..8], Some(Side::Right), Some(Align::Stretch), None).unwrap();
    crate::profiles::run(&da, ProfileCmd::Save { name: "home".into() }).await.unwrap();
    let auto = ProfileCmd::Auto { name: "desk".into(), ssid: vec!["Desk WiFi".into()], network: vec![], off: false, clear: false };
    let watching = crate::netwatch::fake::watchers();
    crate::profiles::run(&da, auto).await.unwrap(); // reloads A, which starts watching
    eventually("A to watch the network", || crate::netwatch::fake::watchers() > watching).await;
    let s = status(&da).await;
    assert_eq!((s.layout.profile.as_deref(), s.layout.profiles.len()), (Some("home"), 2));

    let desk = NetworkId { ssids: vec!["Desk WiFi".into()], ..NetworkId::default() };
    crate::netwatch::fake::announce(&desk);
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Side::Bottom]));
    assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Side::Top]));
    let b_sees_a = record(&duo.db, &fa);
    assert_eq!((b_sees_a.side(), b_sees_a.align()), (Side::Top, Align::Offset(-300)));
    assert_eq!(Profiles::load(&da).unwrap().active.as_deref(), Some("desk"));

    // A tweak, then the same network again (a reconnect): the tweak stands.
    crate::manage::set_layout(&da, &fb[..8], None, Some(Align::Offset(100)), None).unwrap();
    assert!(control::request(&da, Request::Reload).await.unwrap().is_some());
    crate::netwatch::fake::announce(&desk);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(record(&da, &fb).align(), Align::Offset(100));
    // Elsewhere, then back at the desk: the profile again.
    crate::netwatch::fake::announce(&NetworkId { ssids: vec!["Café".into()], ..NetworkId::default() });
    crate::netwatch::fake::announce(&desk);
    let (db, fa2) = (duo.db.clone(), fa.clone());
    eventually("B to get the desk layout back", move || record(&db, &fa2).align() == Align::Offset(-300)).await;
    assert_eq!(record(&da, &fb).align(), Align::Offset(300));
    duo.stop().await;
}
