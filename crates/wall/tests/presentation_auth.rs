use one_mk_wall_sync::bridge::Monitor;
use one_mk_wall_sync::presentation::{Owner, Rect};
use one_mk_wall_sync::presentation_auth::{self as auth, Outcome, Publisher, Record, RecordError, Refusal,
    Reply, Session, Teardown};
use one_mk_wall_sync::presentation_wire;

const DAEMON: &str = ":1.7";
const RENDERER: &str = ":1.9";

fn record() -> Record { Record { key: [5; 32], generation: [6; 16], pid: 4321, unique_name: DAEMON.into() } }
fn owner() -> Owner { Owner { unique_name: DAEMON.into(), uid: 1000, pid: 4321 } }
fn monitors() -> Vec<Monitor> { vec![(0, 0, 1920, 1080, 1.0), (1920, 0, 1920, 1080, 1.0)] }
fn canvas() -> Rect { Rect { x: -1920, y: 0, width: 5760, height: 1080 } }
fn publisher() -> Publisher { Publisher::new([6; 16], [9; 32], canvas(), (0, 0)).unwrap() }
fn session() -> Session { Session::new(owner(), record(), RENDERER.into(), 1, monitors(), 0).unwrap() }

/// What the genuine daemon would answer to `nonce`.
fn answer(key: &[u8; 32], nonce: &[u8; 32], enabled: bool, revision: u64, payload: &[u8]) -> [u8; 32] {
    auth::reply_tag(key, RENDERER, DAEMON, nonce, enabled, revision, payload)
}

fn deliver(session: &mut Session, nonce: [u8; 32], enabled: bool, revision: u64, payload: &[u8]) -> Outcome {
    let tag = answer(&record().key, &nonce, enabled, revision, payload);
    session.reply(&owner(), &nonce, Reply { enabled, revision, payload, tag: &tag })
}

#[test]
fn record_round_trips_and_rejects_every_truncation_extension_and_invalid_field() {
    let bytes = record().encode().unwrap();
    assert_eq!(Record::decode(&bytes), Ok(record()));
    for length in 0..bytes.len() { assert!(Record::decode(&bytes[..length]).is_err(), "{length}"); }
    let mut longer = bytes.clone();
    longer.push(b'1');
    assert!(Record::decode(&longer).is_err());
    assert!(Record::decode(&vec![0; auth::MAX_RECORD + 1]).is_err());
    for (index, value) in [(0, b'X'), (4, 2)] {
        let mut bad = bytes.clone();
        bad[index] = value;
        assert_eq!(Record::decode(&bad), Err(RecordError::Version));
    }
    for (bad, error) in [
        (Record { key: [0; 32], ..record() }, RecordError::Key),
        (Record { generation: [0; 16], ..record() }, RecordError::Generation),
        (Record { pid: 0, ..record() }, RecordError::Pid),
        (Record { unique_name: "org.onemk.Daemon".into(), ..record() }, RecordError::Name),
        (Record { unique_name: ":1".into(), ..record() }, RecordError::Name),
        (Record { unique_name: ":1.2/3".into(), ..record() }, RecordError::Name),
    ] {
        assert_eq!(bad.encode(), Err(error));
    }
    let debug = format!("{:?}", record());
    assert!(debug.contains("redacted") && !debug.contains("[5, 5"), "{debug}");
}

#[test]
fn tags_bind_key_direction_connections_nonce_and_every_field() {
    let key = record().key;
    let nonce = [1; 32];
    let request = auth::request_tag(&key, RENDERER, DAEMON, &nonce, 1, &monitors());
    for other in [
        auth::request_tag(&[4; 32], RENDERER, DAEMON, &nonce, 1, &monitors()),
        auth::request_tag(&key, ":1.10", DAEMON, &nonce, 1, &monitors()),
        auth::request_tag(&key, RENDERER, ":1.8", &nonce, 1, &monitors()),
        auth::request_tag(&key, DAEMON, RENDERER, &nonce, 1, &monitors()),
        auth::request_tag(&key, RENDERER, DAEMON, &[2; 32], 1, &monitors()),
        auth::request_tag(&key, RENDERER, DAEMON, &nonce, 2, &monitors()),
        auth::request_tag(&key, RENDERER, DAEMON, &nonce, 1, &monitors()[..1]),
        auth::request_tag(&key, RENDERER, DAEMON, &nonce, 1, &[(0, 0, 1920, 1080, 1.25), (1920, 0, 1920, 1080, 1.0)]),
    ] {
        assert_ne!(request, other);
    }
    let reply = auth::reply_tag(&key, RENDERER, DAEMON, &nonce, true, 7, b"plan");
    assert_ne!(request, reply);
    for other in [
        auth::reply_tag(&key, RENDERER, DAEMON, &nonce, false, 7, b"plan"),
        auth::reply_tag(&key, RENDERER, DAEMON, &nonce, true, 8, b"plan"),
        auth::reply_tag(&key, RENDERER, DAEMON, &nonce, true, 7, b"plan2"),
        auth::reply_tag(&key, RENDERER, DAEMON, &[2; 32], true, 7, b"plan"),
        auth::reply_tag(&key, ":1.10", DAEMON, &nonce, true, 7, b"plan"),
    ] {
        assert_ne!(reply, other);
    }
    assert!(auth::tag_matches(&reply, &reply));
    assert!(!auth::tag_matches(&reply, &reply[..31]));
    assert!(!auth::tag_matches(&reply, &[reply.as_slice(), &[0]].concat()));
    assert!(!auth::tag_matches(&reply, &[0; 32]));
}

#[test]
fn publisher_advances_sequence_only_on_change_and_slices_on_the_shared_canvas() {
    let mut publisher = Publisher::new([6; 16], [9; 32], canvas(), (-1920, 0)).unwrap();
    let first = publisher.plan(7, 1, &monitors(), false).unwrap();
    assert_eq!(publisher.plan(7, 1, &monitors(), false).unwrap(), first);
    assert_eq!(publisher.sequence(), 1);
    let plan = presentation_wire::decode(&first).unwrap();
    assert_eq!((plan.sequence, plan.consent_revision, plan.layout_serial, plan.media_id), (1, 7, 1, [9; 32]));
    assert_eq!(plan.canvas, canvas());
    assert_eq!(plan.slices.len(), 2);
    assert_eq!(plan.slices[0].canvas, Rect { x: -1920, y: 0, width: 1920, height: 1080 });
    assert_eq!(plan.slices[1].canvas, Rect { x: 0, y: 0, width: 1920, height: 1080 });
    let paused = publisher.plan(7, 1, &monitors(), true).unwrap();
    assert_eq!(presentation_wire::decode(&paused).unwrap().sequence, 2);
    assert!(presentation_wire::decode(&paused).unwrap().paused);
    assert_eq!(publisher.previous(), Some(first.as_slice()));
    assert!(publisher.plan(0, 1, &monitors(), false).is_err());
    assert!(publisher.plan(7, 0, &monitors(), false).is_err());
    assert!(Publisher::new([0; 16], [9; 32], canvas(), (0, 0)).is_err());
    assert!(Publisher::new([6; 16], [0; 32], canvas(), (0, 0)).is_err());
    assert!(Publisher::new([6; 16], [9; 32], Rect { width: 0, ..canvas() }, (0, 0)).is_err());
    let mut overflow = Publisher::new([6; 16], [9; 32], canvas(), (i32::MAX, 0)).unwrap();
    assert!(overflow.plan(7, 1, &monitors(), false).is_err());
}

#[test]
fn genuine_plans_are_admitted_and_identical_bytes_are_idempotent() {
    let mut publisher = publisher();
    let mut session = session();
    let first = publisher.plan(7, 1, &monitors(), false).unwrap();
    assert_eq!(deliver(&mut session, [1; 32], true, 7, &first), Outcome::Accepted { sequence: 1, active: true });
    assert_eq!(deliver(&mut session, [2; 32], true, 7, &first), Outcome::Unchanged);
    let paused = publisher.plan(7, 1, &monitors(), true).unwrap();
    assert_eq!(deliver(&mut session, [3; 32], true, 7, &paused), Outcome::Accepted { sequence: 2, active: false });
    assert!(session.admitted().unwrap().paused);
}

#[test]
fn wrong_absent_forged_or_replayed_credentials_tear_down() {
    let payload = publisher().plan(7, 1, &monitors(), false).unwrap();
    let wrong_key = auth::reply_tag(&[4; 32], RENDERER, DAEMON, &[1; 32], true, 7, &payload);
    let other_nonce = answer(&record().key, &[2; 32], true, 7, &payload);
    let other_pair = auth::reply_tag(&record().key, ":1.10", DAEMON, &[1; 32], true, 7, &payload);
    for tag in [wrong_key.to_vec(), other_nonce.to_vec(), other_pair.to_vec(), Vec::new(), vec![0; 32]] {
        let mut session = session();
        let outcome = session.reply(&owner(), &[1; 32], Reply { enabled: true, revision: 7, payload: &payload, tag: &tag });
        assert_eq!(outcome, Outcome::Teardown(Teardown::Authentication));
        assert_eq!(deliver(&mut session, [3; 32], true, 7, &payload), Outcome::Teardown(Teardown::Authentication));
        assert!(session.admitted().is_none() && !session.active());
    }
    // A correctly tagged answer from any other connection is still refused.
    for caller in [Owner { unique_name: ":1.8".into(), ..owner() }, Owner { pid: 1, ..owner() }, Owner { uid: 0, ..owner() }] {
        let mut session = session();
        let tag = answer(&record().key, &[1; 32], true, 7, &payload);
        assert_eq!(session.reply(&caller, &[1; 32], Reply { enabled: true, revision: 7, payload: &payload, tag: &tag }),
            Outcome::Teardown(Teardown::Authentication));
    }
    // A mid-session forgery ends an admitted presentation too.
    let mut session = session();
    assert!(matches!(deliver(&mut session, [1; 32], true, 7, &payload), Outcome::Accepted { .. }));
    assert_eq!(session.reply(&owner(), &[2; 32], Reply { enabled: true, revision: 7, payload: &payload, tag: &[0; 32] }),
        Outcome::Teardown(Teardown::Authentication));
    assert!(session.admitted().is_none() && !session.active());
}

#[test]
fn session_identity_must_match_the_record() {
    assert!(Session::new(Owner { unique_name: ":1.8".into(), ..owner() }, record(), RENDERER.into(), 1, monitors(), 0).is_err());
    assert!(Session::new(Owner { pid: 1, ..owner() }, record(), RENDERER.into(), 1, monitors(), 0).is_err());
    assert!(Session::new(owner(), record(), DAEMON.into(), 1, monitors(), 0).is_err());
    assert!(Session::new(owner(), record(), "org.onemk.Renderer".into(), 1, monitors(), 0).is_err());
    assert!(Session::new(owner(), record(), RENDERER.into(), 0, monitors(), 0).is_err());
    assert!(Session::new(owner(), record(), RENDERER.into(), 1, monitors(), 16).is_err());
}

#[test]
fn stale_replayed_oversize_and_malformed_plans_are_refused_and_keep_the_admitted_plan() {
    let mut publisher = publisher();
    let mut session = session();
    let first = publisher.plan(7, 1, &monitors(), false).unwrap();
    let second = publisher.plan(7, 1, &monitors(), true).unwrap();
    assert!(matches!(deliver(&mut session, [1; 32], true, 7, &first), Outcome::Accepted { sequence: 1, .. }));
    assert!(matches!(deliver(&mut session, [2; 32], true, 7, &second), Outcome::Accepted { sequence: 2, .. }));
    let admitted = session.admitted().cloned();
    let mut stale_sequence = presentation_wire::decode(&second).unwrap();
    stale_sequence.sequence = 1;
    stale_sequence.paused = false;
    stale_sequence.media_id = [3; 32];
    let mut stale_revision = presentation_wire::decode(&second).unwrap();
    stale_revision.sequence = 3;
    stale_revision.consent_revision = 6;
    let mut other_generation = presentation_wire::decode(&second).unwrap();
    other_generation.sequence = 3;
    other_generation.generation = [8; 16];
    let mut oversize = second.clone();
    oversize.resize(presentation_wire::MAX_PAYLOAD + 1, 0);
    let mut reserved = second.clone();
    reserved[6] = 1;
    for (payload, refusal) in [
        (first.clone(), Refusal::Replay),
        (presentation_wire::encode(&stale_sequence).unwrap(), Refusal::Replay),
        (presentation_wire::encode(&stale_revision).unwrap(), Refusal::Consent),
        (presentation_wire::encode(&other_generation).unwrap(), Refusal::Generation),
        (oversize, Refusal::Wire),
        (reserved, Refusal::Wire),
        (b"bad".to_vec(), Refusal::Wire),
        (Vec::new(), Refusal::Wire),
    ] {
        assert_eq!(deliver(&mut session, [4; 32], true, 7, &payload), Outcome::Refused(refusal));
        assert_eq!(session.admitted().cloned(), admitted);
    }
    assert_eq!(session.invalid_envelope(), Outcome::Refused(Refusal::Envelope));
    assert_eq!(session.admitted().cloned(), admitted);
    assert_eq!(deliver(&mut session, [5; 32], true, 7, &second), Outcome::Unchanged);
    let third = publisher.plan(7, 1, &monitors(), false).unwrap();
    assert_eq!(deliver(&mut session, [6; 32], true, 7, &third), Outcome::Accepted { sequence: 3, active: true });
}

#[test]
fn a_refused_first_plan_leaves_no_pinned_canvas() {
    let mut session = session();
    let mut publisher = Publisher::new([6; 16], [9; 32], Rect { x: 0, y: 0, width: 100, height: 100 }, (0, 0)).unwrap();
    let small = publisher.plan(7, 1, &monitors(), false).unwrap();
    assert_eq!(deliver(&mut session, [1; 32], true, 7, &small), Outcome::Refused(Refusal::Geometry));
    let mut wrong_generation = Publisher::new([8; 16], [9; 32], canvas(), (0, 0)).unwrap();
    let foreign = wrong_generation.plan(7, 1, &monitors(), false).unwrap();
    assert_eq!(deliver(&mut session, [2; 32], true, 7, &foreign), Outcome::Refused(Refusal::Generation));
    let good = self::publisher().plan(7, 1, &monitors(), false).unwrap();
    assert!(matches!(deliver(&mut session, [3; 32], true, 7, &good), Outcome::Accepted { sequence: 1, active: true }));
}

#[test]
fn consent_revocation_or_replacement_and_owner_loss_tear_down() {
    let payload = publisher().plan(7, 1, &monitors(), false).unwrap();
    for (enabled, revision) in [(false, 7), (true, 8), (true, 0), (false, 0)] {
        let mut session = session();
        assert!(matches!(deliver(&mut session, [1; 32], true, 7, &payload), Outcome::Accepted { .. }));
        assert_eq!(deliver(&mut session, [2; 32], enabled, revision, &[]), Outcome::Teardown(Teardown::Revoked));
        assert!(session.admitted().is_none() && !session.active());
        assert_eq!(deliver(&mut session, [3; 32], true, 7, &payload), Outcome::Teardown(Teardown::Revoked));
    }
    let mut session = session();
    assert_eq!(deliver(&mut session, [1; 32], false, 0, &[]), Outcome::Teardown(Teardown::Revoked));
    let mut session = self::session();
    assert!(matches!(deliver(&mut session, [1; 32], true, 7, &payload), Outcome::Accepted { .. }));
    assert_eq!(session.owner_lost(), Teardown::OwnerLost);
    assert!(session.admitted().is_none() && !session.active());
    assert_eq!(deliver(&mut session, [2; 32], true, 7, &payload), Outcome::Teardown(Teardown::OwnerLost));
    assert_eq!(session.ended(), Some(Teardown::OwnerLost));
}

#[test]
fn repeated_refusals_end_the_session() {
    let mut session = session();
    assert!(matches!(deliver(&mut session, [1; 32], true, 7, &publisher().plan(7, 1, &monitors(), false).unwrap()),
        Outcome::Accepted { .. }));
    for _ in 1..auth::MAX_CONSECUTIVE_REFUSALS {
        assert_eq!(deliver(&mut session, [2; 32], true, 7, b"bad"), Outcome::Refused(Refusal::Wire));
    }
    assert_eq!(deliver(&mut session, [2; 32], true, 7, b"bad"), Outcome::Teardown(Teardown::Faults));
    assert!(session.admitted().is_none());
}

#[test]
fn layout_and_pause_changes_require_fresh_plans() {
    let mut publisher = publisher();
    let mut session = session();
    assert!(matches!(deliver(&mut session, [1; 32], true, 7, &publisher.plan(7, 1, &monitors(), false).unwrap()),
        Outcome::Accepted { active: true, .. }));
    session.pause_hint(2).unwrap();
    assert!(!session.active());
    session.pause_hint(0).unwrap();
    assert!(session.active());
    let single = vec![(0, 0, 1920, 1080, 1.0)];
    session.layout(2, single.clone()).unwrap();
    assert!(session.admitted().is_none() && !session.active());
    let old = publisher.plan(7, 1, &monitors(), true).unwrap();
    assert_eq!(deliver(&mut session, [2; 32], true, 7, &old), Outcome::Refused(Refusal::Layout));
    let fresh = publisher.plan(7, 2, &single, false).unwrap();
    assert!(matches!(deliver(&mut session, [3; 32], true, 7, &fresh), Outcome::Accepted { sequence: 3, active: true }));
    assert_eq!(session.layout(2, single.clone()), Err(Teardown::Protocol));
    assert_eq!(session.pause_hint(0), Err(Teardown::Protocol));
    let mut session = self::session();
    assert_eq!(session.pause_hint(16), Err(Teardown::Protocol));
}
