use one_mk_wall_sync::{Action, Controller, Estimator, MonotonicClock, Probe, Timeline, frame_for_target, wire::Message};

struct Clock(f64, u64);
impl MonotonicClock for Clock {
    fn now(&self) -> f64 { self.0 }
    fn generation(&self) -> u64 { self.1 }
}

fn burst(start: f64, offset: f64, drift: f64, generation: u64, sequence: u64) -> Vec<Probe> {
    (0..8).map(|index| {
        let t1 = start + index as f64 * 0.01;
        let remote = |local: f64| local + offset + local * drift;
        Probe { session: 1, generation, sequence: sequence + index, t1, t2: remote(t1 + 0.001), t3: remote(t1 + 0.0011), t4: t1 + 0.0021 }
    }).collect()
}

#[test]
fn offset_sign_drift_and_prediction() {
    for drift in [-200.0e-6, -50.0e-6, 0.0, 50.0e-6, 200.0e-6] {
        let mut estimator = Estimator::new(1, 7);
        for step in 0..80 {
            estimator.observe(&burst(100.0 + step as f64 * 10.0, 3.0, drift, 7, step * 8)).expect("valid burst");
        }
        let mapped = estimator.coordinator_time(910.0).expect("map");
        assert!((mapped - (913.0 + 910.0 * drift)).abs() < 0.001, "{mapped}, {drift}");
        assert!((estimator.drift_ppm() - drift * 1.0e6).abs() < 1.0);
        assert!(estimator.uncertainty(1000.0).expect("uncertainty") > estimator.uncertainty(910.0).expect("uncertainty"));
    }
}

#[test]
fn robust_reorder_duplicate_outlier_replay_and_generation() {
    let mut estimator = Estimator::new(1, 1);
    let mut probes = burst(100.0, 2.0, 0.0, 1, 0);
    probes[0].t2 += 0.5;
    probes[0].t3 += 0.5;
    probes.reverse();
    probes.push(probes[1]);
    estimator.observe(&probes).expect("outlier filtered");
    assert!((estimator.coordinator_time(101.0).expect("map") - 103.0).abs() < 0.0001);
    assert!(estimator.observe(&probes).is_err());
    assert!(estimator.observe(&burst(102.0, 2.0, 0.0, 2, 20)).is_err());
    let mut foreign = burst(102.0, 2.0, 0.0, 1, 20);
    for probe in &mut foreign { probe.session = 2; }
    assert!(estimator.observe(&foreign).is_err());
    assert!(estimator.observe(&burst(99.0, 2.0, 0.0, 1, 20)).is_err());
    estimator.reset(1, 2);
    assert!(estimator.coordinator_time(102.0).is_err());
    estimator.observe(&burst(102.0, 4.0, 0.0, 2, 0)).expect("new authority");
    assert!((estimator.coordinator_time(103.0).expect("map") - 107.0).abs() < 0.0001);
}

#[test]
fn high_rtt_majority_cannot_bias_low_rtt_selection() {
    let mut probes = burst(100.0, 2.0, 0.0, 1, 0);
    for probe in &mut probes[3..] {
        probe.t2 += 0.080;
        probe.t3 += 0.080;
        probe.t4 += 0.100;
    }
    let mut estimator = Estimator::new(1, 1);
    estimator.observe(&probes).expect("low RTT half");
    assert!((estimator.coordinator_time(101.0).expect("map") - 103.0).abs() < 1e-6);
    assert!(Estimator::with_window(1, 1, 3).is_err());
    assert!(Estimator::with_window(1, 1, 33).is_err());
}

#[test]
fn slow_decoder_drops_backlog_instead_of_accumulating_lag() {
    for elapsed_frames in 1..600 {
        let target = elapsed_frames as f64 / 60.0;
        let newest = (target * 24.0).floor() / 24.0;
        let queue: Vec<_> = (0..8).map(|index| newest - (7 - index) as f64 / 24.0).filter(|pts| *pts >= 0.0).collect();
        let selected = frame_for_target(&queue, target).expect("bounded queue").expect("eligible frame");
        assert_eq!(selected, queue.len() - 1);
        assert!((target - queue[selected]) < 1.0 / 24.0 + 1e-9);
    }
}

#[test]
fn bounded_slew_and_consistent_liar_limitation() {
    let mut estimator = Estimator::new(1, 1);
    estimator.observe(&burst(100.0, 2.0, 0.0, 1, 0)).expect("valid");
    estimator.observe(&burst(110.0, 1000.0, 0.0, 1, 8)).expect("bounded liar");
    assert!((estimator.coordinator_time(111.0).expect("map") - 113.0).abs() < 0.006);
    assert!(estimator.drift_ppm().abs() <= 20.0001);
    let mut liar = Estimator::new(1, 1);
    liar.observe(&burst(100.0, 1000.0, 0.0, 1, 0)).expect("undetectable initial lie");
    assert!((liar.coordinator_time(101.0).expect("map") - 1101.0).abs() < 0.0001);
}

#[test]
fn asymmetry_is_not_offset_identifiable() {
    let probes: Vec<_> = (0..8).map(|sequence| Probe { session: 1, generation: 1, sequence, t1: 100.0, t2: 102.010, t3: 102.010, t4: 100.010 }).collect();
    let mut estimator = Estimator::new(1, 1);
    estimator.observe(&probes).expect("valid asymmetric delay");
    assert!((estimator.coordinator_time(101.0).expect("map") - 103.005).abs() < 1.0e-9);
}

#[test]
fn hostile_float_values_never_enter_state() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 1.0e300] {
        for field in 0..4 {
            let mut probes = burst(100.0, 2.0, 0.0, 1, 0);
            for probe in &mut probes {
                match field { 0 => probe.t1 = value, 1 => probe.t2 = value, 2 => probe.t3 = value, _ => probe.t4 = value }
            }
            assert!(Estimator::new(1, 1).observe(&probes).is_err());
        }
        assert!(Timeline::new(value, 60.0, 0.0).is_err());
        assert!(Timeline::new(1.0, value, 0.0).is_err());
        assert!(Controller::new(value, 0.01).is_err());
        assert!(Controller::new(0.1, value).is_err());
        assert!(frame_for_target(&[value], 1.0).is_err());
    }
    assert!(Estimator::new(1, 1).observe(&vec![burst(100.0, 2.0, 0.0, 1, 0)[0]; 65]).is_err());
}

#[test]
fn timeline_loop_future_epoch_and_frame_dropping() {
    let timeline = Timeline::new(100.0, 60.0, 5.0).expect("timeline");
    assert_eq!(timeline.position(99.0), Ok(None));
    assert_eq!(timeline.position(100.0), Ok(Some(5.0)));
    assert_eq!(timeline.position(155.0), Ok(Some(0.0)));
    assert!((timeline.error(0.01, 59.99).expect("wrap") - 0.02).abs() < 1.0e-10);
    assert_eq!(frame_for_target(&[1.0, 1.1, 1.2, 1.3], 1.25), Ok(Some(2)));
    assert_eq!(frame_for_target(&[1.0], 0.0), Ok(None));
    assert!(frame_for_target(&[2.0, 1.0], 3.0).is_err());
    assert!(frame_for_target(&[1.0; 257], 3.0).is_err());
}

#[test]
fn controller_late_join_hysteresis_boundary_sleep_and_clock_jump() {
    let mut estimator = Estimator::new(1, 1);
    estimator.observe(&burst(10.0, 0.0, 0.0, 1, 0)).expect("map");
    let timeline = Timeline::new(0.0, 60.0, 0.0).expect("timeline");
    let mut controller = Controller::new(0.1, 0.02).expect("controller");
    assert_eq!(controller.tick(&Clock(20.0, 1), &estimator, &timeline, 0.0), Ok(Action::HardResync(20.0)));
    assert!(matches!(controller.tick(&Clock(20.2, 1), &estimator, &timeline, 20.18), Ok(Action::Rate(rate)) if rate > 1.0 && rate <= 1.02));
    assert!(matches!(controller.tick(&Clock(20.4, 1), &estimator, &timeline, 20.394), Ok(Action::Rate(_))));
    assert_eq!(controller.tick(&Clock(20.6, 1), &estimator, &timeline, 20.599), Ok(Action::OnTime));
    assert_eq!(controller.tick(&Clock(20.8, 1), &estimator, &timeline, 20.794), Ok(Action::OnTime));
    assert_eq!(controller.tick(&Clock(20.81, 1), &estimator, &timeline, 20.8), Ok(Action::Wait));
    assert!(controller.tick(&Clock(19.0, 1), &estimator, &timeline, 19.0).is_err());
    assert!(controller.tick(&Clock(21.0, 2), &estimator, &timeline, 21.0).is_err());
    assert_eq!(controller.tick(&Clock(26.0, 1), &estimator, &timeline, 26.0), Ok(Action::HardResync(26.0)));
    for second in 27..60 {
        controller.tick(&Clock(second as f64, 1), &estimator, &timeline, second as f64).expect("regular cadence");
    }
    controller.tick(&Clock(59.9, 1), &estimator, &timeline, 59.9).expect("time gap");
    assert!(matches!(controller.tick(&Clock(60.05, 1), &estimator, &timeline, 59.90), Ok(Action::Seek(_))));
    controller.reset();
    assert!(matches!(controller.tick(&Clock(61.0, 1), &estimator, &timeline, 1.0), Ok(Action::HardResync(_))));
}

#[test]
fn deterministic_arbitrary_bits_property_and_wire_roundtrip() {
    let mut state = 0x90d5_e8c9_1234_5678u64;
    for iteration in 0..100_000 {
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        let value = f64::from_bits(state);
        let probe = Probe { session: 1, generation: 1, sequence: iteration, t1: value, t2: value, t3: value, t4: value };
        if let Ok((local, offset, rtt)) = probe.measurement() {
            assert!(local.is_finite() && offset.is_finite() && rtt.is_finite());
        }
        let message = Message { reply: false, probe: Probe { t2: 0.0, t3: 0.0, t4: 0.0, ..probe } };
        if let Ok(encoded) = message.encode() { assert_eq!(Message::decode(&encoded), Ok(message)); }
        let mut bytes = [0u8; 64];
        for (index, byte) in bytes.iter_mut().enumerate() { *byte = state.rotate_left(index as u32) as u8; }
        let _ = Message::decode(&bytes[..iteration as usize % 65]);
    }
    let message = Message { reply: true, probe: Probe { session: 1, generation: 4, sequence: 9, t1: 1.0, t2: 2.0, t3: 2.1, t4: 0.0 } };
    let bytes = message.encode().expect("encode");
    assert_eq!(Message::decode(&bytes), Ok(message));
    for index in [0, 4, 5, 6, 7] {
        let mut corrupt = bytes;
        corrupt[index] = 255;
        assert!(Message::decode(&corrupt).is_err(), "byte {index}");
    }
    assert!(Message::decode(&bytes[..63]).is_err());
    assert!(Message::decode(&[bytes.as_slice(), &[0]].concat()).is_err());
    for index in 0..64 {
        for bit in 0..8 {
            let mut altered = bytes;
            altered[index] ^= 1 << bit;
            if let Ok(decoded) = Message::decode(&altered) {
                assert_eq!(decoded.encode().expect("valid encoding"), altered);
            }
        }
    }
}

#[test]
fn stalled_presenter_does_not_cause_seek_storm() {
    let mut estimator = Estimator::new(1, 1);
    estimator.observe(&burst(10.0, 0.0, 0.0, 1, 0)).expect("map");
    let timeline = Timeline::new(0.0, 60.0, 0.0).expect("timeline");
    let mut controller = Controller::new(0.1, 0.02).expect("controller");
    for step in 0..40 {
        let action = controller.tick(&Clock(20.0 + step as f64 * 0.11, 1), &estimator, &timeline, 0.0).expect("tick");
        if step == 0 { assert!(matches!(action, Action::HardResync(_))); }
        else { assert!(!matches!(action, Action::HardResync(_) | Action::Seek(_))); }
    }
}

#[test]
fn arbitrary_state_updates_are_finite_and_errors_are_atomic() {
    let mut state = 17u64;
    let mut estimator = Estimator::new(1, 1);
    estimator.observe(&burst(100.0, 3.0, 0.0, 1, 0)).expect("map");
    for step in 1..10_000u64 {
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        let now = 101.0 + step as f64;
        let before = estimator.coordinator_time(now);
        let mut probes = burst(now - 0.5, 3.0, 0.0, 1, step * 8);
        for probe in &mut probes {
            match state % 7 {
                0 => probe.t1 = f64::from_bits(state),
                1 => probe.t2 = f64::from_bits(state),
                2 => probe.t3 = f64::from_bits(state),
                3 => probe.t4 = f64::from_bits(state),
                4 => probe.generation = state,
                5 => probe.sequence = 0,
                _ => (),
            }
        }
        probes.reverse();
        probes.push(probes[0]);
        let result = estimator.observe(&probes);
        let after = estimator.coordinator_time(now).expect("finite map");
        assert!(after.is_finite());
        if result.is_err() { assert_eq!(before, Ok(after)); }
        assert!(estimator.drift_ppm().abs() <= 500.0001);
    }
}
