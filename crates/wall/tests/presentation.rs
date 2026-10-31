use one_mk_wall_sync::presentation::{Error, Gate, Owner, PresentationPlan, Rect, Slice};

fn owner() -> Owner { Owner { unique_name: ":1.42".into(), uid: 1000, pid: 1234 } }

fn canvas() -> Rect { Rect { x: -100, y: -50, width: 500, height: 200 } }

fn gate() -> Gate {
    Gate::new(owner(), 7, [1; 16], canvas(), 3,
        vec![(0, 0, 100, 100, 1.0), (120, 0, 100, 100, 1.25)]).unwrap()
}

fn plan() -> PresentationPlan {
    PresentationPlan { generation: [1; 16], consent_revision: 7, sequence: 1, layout_serial: 3,
        media_id: [2; 32], canvas: canvas(), slices: vec![
            Slice { screen: "left".into(), canvas: Rect { x: -100, y: -50, width: 100, height: 100 } },
            Slice { screen: "right".into(), canvas: Rect { x: 20, y: -50, width: 100, height: 100 } },
        ], looping: true, paused: false }
}

#[test]
fn cold_start_waits_for_snapshot_and_preserves_reserved_space() {
    let mut gate = gate();
    assert!(gate.active().is_none());
    gate.accept(&owner(), plan()).unwrap();
    assert!(gate.active().is_none());
    gate.pause_hint(0).unwrap();
    assert_eq!(gate.active().unwrap().canvas, canvas());
    assert_eq!(gate.active().unwrap().slices[1].canvas.x, 20);
    assert!(gate.active().unwrap().looping);
}

#[test]
fn invalid_initial_authority_consent_generation_canvas_and_layout_refuse() {
    for name in ["", "org.onemk.Daemon", ":", ":1", ":1.", ":1.a/b"] {
        let mut caller = owner();
        caller.unique_name = name.into();
        assert!(matches!(Gate::new(caller, 7, [1; 16], canvas(), 3, vec![]), Err(Error::Identity)));
    }
    let mut caller = owner();
    caller.pid = 0;
    assert!(Gate::new(caller, 7, [1; 16], canvas(), 3, vec![]).is_err());
    assert!(matches!(Gate::new(owner(), 0, [1; 16], canvas(), 3, vec![]), Err(Error::Consent)));
    assert!(matches!(Gate::new(owner(), 7, [0; 16], canvas(), 3, vec![]), Err(Error::Generation)));
    assert!(Gate::new(owner(), 7, [1; 16], canvas(), 0, vec![]).is_err());
    assert!(Gate::new(owner(), 7, [1; 16], canvas(), 3, vec![(0, 0, 1, 1, f64::NAN)]).is_err());
    for bounds in [Rect { width: 0, ..canvas() }, Rect { x: i32::MAX, ..canvas() },
        Rect { width: u32::MAX, ..canvas() }, Rect { y: -1_000_001, ..canvas() }] {
        assert!(matches!(Gate::new(owner(), 7, [1; 16], bounds, 3, vec![]), Err(Error::Geometry)));
    }
}

#[test]
fn authenticated_identity_requires_unique_name_uid_and_pid() {
    for caller in [Owner { unique_name: ":1.43".into(), ..owner() },
        Owner { uid: 1001, ..owner() }, Owner { pid: 1235, ..owner() }] {
        let mut gate = gate();
        gate.pause_hint(0).unwrap();
        gate.accept(&owner(), plan()).unwrap();
        let update = PresentationPlan { sequence: 2, ..plan() };
        assert_eq!(gate.accept(&caller, update), Err(Error::Identity));
        assert_eq!(gate.active(), Some(&plan()));
    }
}

#[test]
fn every_rejected_plan_preserves_current_presentation() {
    let cases = [
        (PresentationPlan { consent_revision: 8, ..plan() }, Error::Consent),
        (PresentationPlan { generation: [3; 16], ..plan() }, Error::Generation),
        (PresentationPlan { sequence: 0, ..plan() }, Error::Replay),
        (PresentationPlan { layout_serial: 2, ..plan() }, Error::Layout),
        (PresentationPlan { media_id: [0; 32], ..plan() }, Error::Media),
        (PresentationPlan { canvas: Rect { width: 220, ..canvas() }, ..plan() }, Error::Geometry),
        (PresentationPlan { slices: vec![], ..plan() }, Error::Layout),
    ];
    for (mut invalid, expected) in cases {
        let mut gate = gate();
        gate.pause_hint(0).unwrap();
        gate.accept(&owner(), plan()).unwrap();
        if invalid.sequence != 0 { invalid.sequence = 2; }
        assert_eq!(gate.accept(&owner(), invalid), Err(expected));
        assert_eq!(gate.active(), Some(&plan()));
        assert_eq!(gate.accept(&owner(), plan()), Err(Error::Replay));
    }
}

#[test]
fn slices_cannot_overflow_reflow_duplicate_or_exceed_canvas() {
    for slice in [
        Slice { screen: "".into(), ..plan().slices[1].clone() },
        Slice { screen: "left".into(), ..plan().slices[1].clone() },
        Slice { screen: "a".repeat(257), ..plan().slices[1].clone() },
        Slice { screen: "bad\nname".into(), ..plan().slices[1].clone() },
        Slice { canvas: Rect { x: i32::MAX, ..plan().slices[1].canvas }, ..plan().slices[1].clone() },
        Slice { canvas: Rect { x: 21, ..plan().slices[1].canvas }, ..plan().slices[1].clone() },
        Slice { canvas: Rect { width: 101, ..plan().slices[1].canvas }, ..plan().slices[1].clone() },
        Slice { canvas: Rect { x: 350, ..plan().slices[1].canvas }, ..plan().slices[1].clone() },
    ] {
        let mut gate = gate();
        let mut invalid = plan();
        invalid.slices[1] = slice;
        assert_eq!(gate.accept(&owner(), invalid), Err(Error::Geometry));
        assert!(gate.active().is_none());
    }
    let mut gate = Gate::new(owner(), 7, [1; 16], canvas(), 3,
        vec![(0, 0, 100, 100, 1.0), (50, 0, 100, 100, 1.0)]).unwrap();
    let mut overlapping = plan();
    overlapping.slices[1].canvas.x = -50;
    assert_eq!(gate.accept(&owner(), overlapping), Err(Error::Geometry));
}

#[test]
fn lock_coverage_and_manual_pause_compose_without_overriding_each_other() {
    let mut gate = gate();
    gate.accept(&owner(), plan()).unwrap();
    for reasons in 1..=15 {
        gate.pause_hint(reasons).unwrap();
        assert!(gate.active().is_none());
    }
    gate.pause_hint(0).unwrap();
    assert!(gate.active().is_some());
    gate.accept(&owner(), PresentationPlan { sequence: 2, paused: true, ..plan() }).unwrap();
    gate.pause_hint(2).unwrap();
    gate.pause_hint(0).unwrap();
    assert!(gate.active().is_none());
    gate.accept(&owner(), PresentationPlan { sequence: 3, looping: false, ..plan() }).unwrap();
    assert!(!gate.active().unwrap().looping);
}

#[test]
fn suspend_resume_never_replays_pre_suspend_or_during_suspend_plans() {
    let mut gate = gate();
    gate.pause_hint(0).unwrap();
    gate.accept(&owner(), plan()).unwrap();
    gate.suspend();
    assert!(gate.active().is_none());
    gate.accept(&owner(), PresentationPlan { sequence: 2, ..plan() }).unwrap();
    assert!(gate.active().is_none());
    gate.resume();
    assert!(gate.active().is_none());
    assert_eq!(gate.accept(&owner(), plan()), Err(Error::Replay));
    gate.accept(&owner(), PresentationPlan { sequence: 3, ..plan() }).unwrap();
    assert!(gate.active().is_some());
}

#[test]
fn a_paused_plan_may_leave_the_canvas_but_never_presents() {
    // The right monitor moved to local x=420: its slice (x=320..420) leaves
    // the 500-wide canvas that ends at x=400.
    let mut gate = Gate::new(owner(), 7, [1; 16], canvas(), 3,
        vec![(0, 0, 100, 100, 1.0), (420, 0, 100, 100, 1.0)]).unwrap();
    let mut outside = plan();
    outside.slices[1].canvas.x = 320;
    gate.pause_hint(0).unwrap();
    assert_eq!(gate.accept(&owner(), outside.clone()), Err(Error::Geometry));
    gate.accept(&owner(), PresentationPlan { paused: true, ..outside.clone() }).unwrap();
    assert!(gate.active().is_none());
    // Everything else about the slices is still checked.
    let mut wrong_size = PresentationPlan { sequence: 2, paused: true, ..outside.clone() };
    wrong_size.slices[1].canvas.width = 101;
    assert_eq!(gate.accept(&owner(), wrong_size), Err(Error::Geometry));
    let mut overflow = PresentationPlan { sequence: 2, paused: true, ..outside };
    overflow.slices[1].canvas.x = i32::MAX;
    assert_eq!(gate.accept(&owner(), overflow), Err(Error::Geometry));
}

#[test]
fn hotplug_requires_fresh_serial_and_plan_without_shrinking_canvas() {
    let mut gate = gate();
    gate.pause_hint(0).unwrap();
    gate.accept(&owner(), plan()).unwrap();
    gate.layout_changed(4, vec![(0, 0, 100, 100, 1.0)]).unwrap();
    assert!(gate.active().is_none());
    assert_eq!(gate.accept(&owner(), PresentationPlan { sequence: 2, ..plan() }), Err(Error::Layout));
    let mut replacement = PresentationPlan { sequence: 3, layout_serial: 4, ..plan() };
    replacement.slices.truncate(1);
    gate.accept(&owner(), replacement).unwrap();
    assert_eq!(gate.active().unwrap().canvas, canvas());
    gate.layout_changed(5, vec![]).unwrap();
    assert!(gate.active().is_none());
    assert_eq!(gate.accept(&owner(), PresentationPlan { sequence: 4, layout_serial: 5, ..plan() }), Err(Error::Layout));
}

#[test]
fn consent_revocation_owner_loss_and_protocol_faults_are_terminal() {
    for operation in 0..5 {
        let mut gate = gate();
        gate.pause_hint(0).unwrap();
        gate.accept(&owner(), plan()).unwrap();
        match operation {
            0 => gate.revoke(),
            1 => gate.owner_lost(),
            2 => assert_eq!(gate.pause_hint(16), Err(Error::Layout)),
            3 => assert_eq!(gate.layout_changed(3, vec![]), Err(Error::Layout)),
            _ => assert_eq!(gate.layout_changed(4, vec![(0, 0, 0, 1, 1.0)]), Err(Error::Layout)),
        }
        gate.resume();
        gate.pause_hint(0).unwrap();
        assert!(gate.active().is_none());
        assert_eq!(gate.accept(&owner(), PresentationPlan { sequence: 2, ..plan() }), Err(Error::Revoked));
    }
}

#[test]
fn sequence_exhaustion_cannot_wrap_into_replay() {
    let mut gate = gate();
    gate.accept(&owner(), PresentationPlan { sequence: u64::MAX, ..plan() }).unwrap();
    assert_eq!(gate.accept(&owner(), plan()), Err(Error::Replay));
    assert_eq!(gate.accept(&owner(), PresentationPlan { sequence: u64::MAX, ..plan() }), Err(Error::Replay));
}

#[test]
fn policy_disable_removal_and_revision_replacement_revoke_old_session() {
    for (enabled, revision) in [(false, Some(7)), (true, None), (true, Some(0)), (true, Some(8))] {
        let mut gate = gate();
        gate.pause_hint(0).unwrap();
        gate.accept(&owner(), plan()).unwrap();
        gate.reconcile_consent(true, Some(7));
        assert!(gate.active().is_some());
        gate.reconcile_consent(enabled, revision);
        assert!(gate.active().is_none());
        gate.reconcile_consent(true, Some(7));
        assert_eq!(gate.accept(&owner(), PresentationPlan { sequence: 2, ..plan() }), Err(Error::Revoked));
    }
}
