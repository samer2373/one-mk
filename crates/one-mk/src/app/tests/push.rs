//! Edges where no computer sits (`docs/EDGE-PUSH.md`), between real
//! daemons on loopback: pointer wrap and layout learning.

use super::*;
use crate::settings::{Learn, Rules, Wrap};
use one_mk_ctl::status::Suggestion;
use one_mk_net::PlacedBy;

const W: i32 = 1920;
const H: i32 = 1080;

fn whole(side: Side) -> Edge {
    Edge::of(side, &Rect::new(0, 0, W, H))
}

fn edges(of: &[Edge]) -> CaptureCmd {
    CaptureCmd::SetEdges(of.to_vec())
}

/// `dir`'s record of the device with fingerprint `fp`.
fn record(dir: &std::path::Path, fp: &str) -> Device {
    TrustStore::load(dir).unwrap().find(&fp[..8]).unwrap().clone()
}

/// A starts with its record of B a guess (as pairing leaves it); B's
/// record of A is the user's.
async fn guessed(tag: &str) -> Duo {
    let mut duo = Duo::start_with(tag, None, |a_sees_b, _| a_sees_b.placed_by = PlacedBy::Default).await;
    // A probes its free edges while B (a guess) is online.
    duo.settle_as(edges(&[whole(Side::Right), whole(Side::Left), whole(Side::Top), whole(Side::Bottom)]), armed(Side::Left)).await;
    duo
}

/// Pushes the pointer against A's `side` edge twice, as two pushes (with
/// a pause between them longer than one push lasts). Each is handed back
/// where it is, like a wall.
async fn push_twice(duo: &mut Duo, side: Side, at: Pos, first_id: u32) {
    for id in [first_id, first_id + 1] {
        duo.ha.events.send(Event::Activated { id, side, pos: at }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id, pos: None });
        if id == first_id {
            tokio::time::sleep(crate::learn::DEBOUNCE + Duration::from_millis(50)).await;
        }
    }
}

/// Wrap on: the left edge (no computer there) leads round to the right,
/// with what was typed or clicked meanwhile kept here, nothing left held,
/// while the right edge still leads to B. Off disarms it again.
#[tokio::test]
async fn wrap_comes_in_at_the_opposite_edge_and_keeps_input_here() {
    let mut duo = Duo::start("push-wrap", None).await;
    set(&duo.da, |s| s.edge.wrap = Wrap::On).await;
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Right), whole(Side::Left)]), "B's edge stays first and whole");
    assert!(status(&duo.da).await.edges.contains("wraps round at left and right"));

    duo.ha.events.send(Event::Activated { id: 5, side: Side::Left, pos: Pos::new(0.0, 300.0) }).unwrap();
    // Caught by that capture before the release took effect: a key and a click.
    for ev in [
        Event::Key { code: 30, down: true, device: None },
        Event::Key { code: 30, down: false, device: None },
        Event::Button { button: 272, down: true, device: None },
        Event::Button { button: 272, down: false, device: None },
    ] {
        duo.ha.events.send(ev).unwrap();
    }
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 5, pos: Some(Pos::new(1917.0, 300.0)) });
    // Typed here as a tap and a whole click; the real releases add nothing.
    let typed = [
        Inject::Key { code: 30, down: true },
        Inject::Key { code: 30, down: false },
        Inject::Button { button: 272, down: true },
        Inject::Button { button: 272, down: false },
    ];
    for want in typed {
        assert_eq!(next_inject(&mut duo.ha).await, want);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(injected(&mut duo.ha).is_empty(), "nothing more, so nothing held");
    assert!(injected(&mut duo.hb).is_empty(), "nothing went to B");
    assert_eq!(status(&duo.da).await.controlling, None);

    // The right edge is B's: it crosses, it does not wrap.
    duo.ha.events.send(Event::Activated { id: 6, side: Side::Right, pos: Pos::new(1919.0, 500.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 500.0)));
    duo.leave_b().await;

    set(&duo.da, |s| s.edge.wrap = Wrap::All).await;
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Right), whole(Side::Left), whole(Side::Top), whole(Side::Bottom)]));
    duo.ha.events.send(Event::Activated { id: 7, side: Side::Top, pos: Pos::new(800.0, 0.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 7, pos: Some(Pos::new(800.0, 1077.0)) });
    set(&duo.da, |s| s.edge.wrap = Wrap::Off).await;
    assert_eq!(next_capture(&mut duo.ha).await, armed(Side::Right));
    duo.stop().await;
}

/// B touches only the lower part of A's right edge: the part above it
/// wraps, the part facing B crosses, and where they meet B wins.
#[tokio::test]
async fn a_real_neighbour_edge_is_never_replaced_by_a_wrap_edge() {
    let setup = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.offset = Some((1920, 300));
        b_sees_a.offset = Some((-1920, -300));
    };
    let laptop = Desktop::new(vec![Rect::new(0, 0, 1280, 800)]);
    let mut duo = Duo::start_on("push-neighbour", None, setup, Desktop::new(vec![Rect::new(0, 0, W, H)]), laptop).await;
    let b_edge = Edge { side: Side::Right, at: W, lo: 300, hi: H };
    duo.settle_as(edges(&[b_edge]), edges(&[Edge { side: Side::Left, at: 0, lo: 0, hi: 780 }])).await;
    set(&duo.da, |s| s.edge.wrap = Wrap::On).await;
    let above = Edge { side: Side::Right, at: W, lo: 0, hi: 300 };
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[b_edge, whole(Side::Left), above]));

    duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 100.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(2.0, 100.0)) });
    // Exactly where the two meet, and a pixel into B's: B.
    for (id, y) in [(2, 300.0), (3, 301.0)] {
        duo.ha.events.send(Event::Activated { id, side: Side::Right, pos: Pos::new(1919.0, y) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, y - 300.0)), "{y}");
        duo.leave_b().await;
    }
    duo.stop().await;
}

/// The edge rules hold for wrapping as for crossing: a dead corner and the
/// fullscreen guard refuse, a dwell waits. Locking the pointer, or B
/// controlling A, disarms the wrap edge; it comes back after.
#[tokio::test]
async fn guards_lock_and_control_apply_to_wrap() {
    let mut duo = Duo::start("push-guards", None).await;
    set(&duo.da, |s| {
        s.edge.wrap = Wrap::On;
        s.edge.left = Some(Rules { corner: Some(20), dwell_ms: Some(200), ..Default::default() });
        s.edge.all.fullscreen_guard = Some(true);
    })
    .await;
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Right), whole(Side::Left)]));
    // A dead corner: handed back where it is.
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Left, pos: Pos::new(0.0, 5.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: None });
    // A dwell: held at the edge, then round.
    let started = Instant::now();
    duo.ha.events.send(Event::Activated { id: 2, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    duo.ha.events.send(Event::Motion { dx: -30.0, dy: 10.0, device: None }).unwrap(); // pushing, sliding down
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 2, pos: Some(Pos::new(1917.0, 510.0)) });
    assert!(started.elapsed() >= Duration::from_millis(200), "{:?}", started.elapsed());
    // Pulled back during the dwell: no wrap.
    duo.ha.events.send(Event::Activated { id: 3, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    duo.ha.events.send(Event::Motion { dx: 30.0, dy: 0.0, device: None }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 3, pos: Some(Pos::new(30.0, 500.0)) });
    // A fullscreen window has the focus.
    *duo.ha.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec!["game".into()] });
    duo.ha.events.send(Event::Activated { id: 4, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 4, pos: None });
    *duo.ha.focus.lock().unwrap() = None;

    // Locked: nothing armed; a capture already on its way is handed back.
    assert_eq!(ask(&duo.da, Request::LockPointer(one_mk_ctl::client::Toggle::On)).await, "ok\non\n");
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[]));
    duo.ha.events.send(Event::Activated { id: 5, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 5, pos: None });
    assert_eq!(ask(&duo.da, Request::LockPointer(one_mk_ctl::client::Toggle::Off)).await, "ok\noff\n");
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Right), whole(Side::Left)]));

    // B takes control of A: A's wrap edge goes while it lasts.
    duo.hb.events.send(Event::Activated { id: 1, side: Side::Left, pos: Pos::new(0.0, 540.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, armed(Side::Right));
    duo.ha.events.send(Event::Activated { id: 6, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 6, pos: None }), "no wrap while controlled");
    assert_eq!(ask(&duo.da, Request::TakeBack).await.lines().next(), Some("ok"));
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Right), whole(Side::Left)]));
    duo.stop().await;
}

/// A paired computer that is offline still sits on its side: nothing
/// wraps there.
#[tokio::test]
async fn no_wrap_where_an_offline_computer_sits() {
    let mut duo = Duo::start("push-offline", None).await;
    let other = Identity::load_or_create(&temp_dir("push-offline-c")).unwrap();
    trust(&duo.da, &other, Side::Left);
    set(&duo.da, |s| s.edge.wrap = Wrap::All).await;
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Right), whole(Side::Top), whole(Side::Bottom)]));
    let _ = std::fs::remove_dir_all(temp_dir("push-offline-c"));
    duo.stop().await;
}

/// The only computer online is where pairing guessed, on the right; the
/// pointer pushed twice against the left edge suggests it is there, in
/// `one-mk status`, without moving anything. Dismissed, it goes, and that
/// computer isn't suggested again for a while.
#[tokio::test]
async fn pushing_twice_suggests_the_computer_online() {
    let mut duo = guessed("push-suggest").await;
    // One push is nothing.
    duo.ha.events.send(Event::Activated { id: 1, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: None });
    assert_eq!(status(&duo.da).await.suggestion, None);
    tokio::time::sleep(crate::learn::PUSH_WINDOW + Duration::from_millis(100)).await;

    push_twice(&mut duo, Side::Left, Pos::new(0.0, 500.0), 2).await;
    let s = status(&duo.da).await;
    let name = s.peers[0].name.clone();
    let id = s.peers[0].id.clone();
    let arg = one_mk_ctl::status::device_arg(&name, &id);
    assert_eq!(s.suggestion, Some(Suggestion { id, name: name.clone(), side: Side::Left, arg }));
    let text = s.render();
    assert!(text.contains(&format!("Is {name} on your left?")) && text.contains("one-mk layout "), "{text}");
    // Nothing moved, here or there.
    assert_eq!(record(&duo.da, &duo.fb).side(), Side::Right);
    assert_eq!(Duo::peer_status(&duo.db).await.side, Side::Left);
    assert!(injected(&mut duo.hb).is_empty());

    assert_eq!(ask(&duo.da, Request::DismissSuggestion).await, "ok\n");
    assert_eq!(status(&duo.da).await.suggestion, None);
    assert_eq!(ask(&duo.da, Request::DismissSuggestion).await, "none\n");
    // Pushed twice again: the cooldown holds.
    push_twice(&mut duo, Side::Left, Pos::new(0.0, 500.0), 4).await;
    assert_eq!(status(&duo.da).await.suggestion, None);
    duo.stop().await;
}

/// The suggestion's own command places the computer, which ends the
/// suggestion and the probes (its place is the user's now).
#[tokio::test]
async fn placing_the_suggested_computer_ends_the_suggestion() {
    let mut duo = guessed("push-accept").await;
    push_twice(&mut duo, Side::Top, Pos::new(900.0, 0.0), 1).await;
    assert_eq!(status(&duo.da).await.suggestion.map(|s| s.side), Some(Side::Top));
    Duo::layout(&duo.da, &duo.fb, Some(Side::Top), None).await;
    assert_eq!(next_capture(&mut duo.ha).await, armed(Side::Top), "B above; no more probes");
    assert_eq!(status(&duo.da).await.suggestion, None);
    assert_eq!(record(&duo.da, &duo.fb).placed_by, PlacedBy::User);
    duo.stop().await;
}

/// `learn = auto`: pushed twice to the left, the guessed computer is put
/// there through the layout path, so the other computer takes the mirror
/// image; crossing to it confirms the place and the probes go. Pushing
/// above is only ever suggested.
#[tokio::test]
async fn auto_places_it_and_the_mirror_syncs() {
    // Both places are pairing's guesses: each probes its free edges.
    let both = |a_sees_b: &mut Device, b_sees_a: &mut Device| {
        a_sees_b.placed_by = PlacedBy::Default;
        b_sees_a.placed_by = PlacedBy::Default;
    };
    let mut duo = Duo::start_with("push-auto", None, both).await;
    let all_but = |taken: Side| {
        let rest = Side::ALL.into_iter().filter(move |s| *s != taken).map(whole);
        edges(&std::iter::once(whole(taken)).chain(rest).collect::<Vec<_>>())
    };
    duo.settle_as(all_but(Side::Right), all_but(Side::Left)).await;
    set(&duo.da, |s| s.edge.learn = Learn::Auto).await;
    assert!(status(&duo.da).await.edges.contains("layout learning places computers"));
    push_twice(&mut duo, Side::Left, Pos::new(0.0, 500.0), 1).await;
    // B now on the left: its edge there; probes on the rest (still a guess, learned).
    assert_eq!(next_capture(&mut duo.ha).await, edges(&[whole(Side::Left), whole(Side::Right), whole(Side::Top), whole(Side::Bottom)]));
    let d = record(&duo.da, &duo.fb);
    assert_eq!((d.side(), d.offset, d.placed_by), (Side::Left, None, PlacedBy::Learned));
    assert_eq!(status(&duo.da).await.suggestion, None);
    // B takes the mirror image: A is on its right now, placed there by
    // A's choice, so B watches no edge for it any more.
    assert_eq!(next_capture(&mut duo.hb).await, armed(Side::Right));
    assert_eq!(Duo::peer_status(&duo.db).await.side, Side::Right);
    let mirror = record(&duo.db, &duo.fa);
    assert_eq!((mirror.layout_version, mirror.placed_by), (d.layout_version, PlacedBy::User));

    // Crossing to B there: right, so learning leaves it be from now on.
    duo.ha.events.send(Event::Activated { id: 9, side: Side::Left, pos: Pos::new(0.0, 500.0) }).unwrap();
    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(1919.0, 500.0)));
    assert_eq!(next_capture(&mut duo.ha).await, armed(Side::Left), "the probes go");
    assert_eq!(record(&duo.da, &duo.fb).placed_by, PlacedBy::Confirmed);
    duo.stop().await;
}

#[tokio::test]
async fn auto_only_suggests_above_and_below() {
    let mut duo = guessed("push-auto-top").await;
    set(&duo.da, |s| s.edge.learn = Learn::Auto).await;
    push_twice(&mut duo, Side::Bottom, Pos::new(900.0, 1079.0), 1).await;
    assert_eq!(status(&duo.da).await.suggestion.map(|s| s.side), Some(Side::Bottom));
    assert_eq!(record(&duo.da, &duo.fb).side(), Side::Right, "not moved");
    // Learning off: no probes at all.
    set(&duo.da, |s| s.edge.learn = Learn::Off).await;
    assert_eq!(next_capture(&mut duo.ha).await, armed(Side::Right));
    duo.stop().await;
}
