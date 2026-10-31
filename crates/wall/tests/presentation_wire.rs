use one_mk_wall_sync::presentation::{Error as AdmissionError, Gate, Owner, PresentationPlan, Rect, Slice};
use one_mk_wall_sync::presentation_wire::{self as wire, Error};

fn owner() -> Owner { Owner { unique_name: ":1.42".into(), uid: 1000, pid: 1234 } }

fn plan() -> PresentationPlan {
    PresentationPlan { generation: [1; 16], consent_revision: 7, sequence: 1, layout_serial: 3,
        media_id: [2; 32], canvas: Rect { x: -100, y: -50, width: 500, height: 200 },
        slices: vec![Slice { screen: "left".into(),
            canvas: Rect { x: -100, y: -50, width: 100, height: 100 } },
            Slice { screen: "right".into(),
                canvas: Rect { x: 20, y: -50, width: 100, height: 100 } }],
        looping: true, paused: false }
}

fn gate() -> Gate {
    let mut gate = Gate::new(owner(), 7, [1; 16], plan().canvas, 3,
        vec![(0, 0, 100, 100, 1.0), (120, 0, 100, 100, 1.25)]).unwrap();
    gate.pause_hint(0).unwrap();
    gate
}

#[test]
fn round_trip_preserves_reserved_canvas_signed_coordinates_and_all_flags() {
    for looping in [false, true] {
        for paused in [false, true] {
            let original = PresentationPlan { looping, paused, ..plan() };
            let bytes = wire::encode(&original).unwrap();
            assert_eq!(bytes.len(), 139);
            assert_eq!(&bytes[..8], &[b'W', b'P', b'L', b'N', 1,
                u8::from(looping) | (u8::from(paused) << 1), 0, 0]);
            assert_eq!(wire::decode(&bytes), Ok(original));
        }
    }
}

#[test]
fn every_truncation_and_trailing_data_is_rejected() {
    let bytes = wire::encode(&plan()).unwrap();
    for length in 0..bytes.len() { assert!(wire::decode(&bytes[..length]).is_err(), "{length}"); }
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(wire::decode(&trailing), Err(Error::Trailing));
    assert_eq!(wire::decode(&vec![0; wire::MAX_PAYLOAD + 1]), Err(Error::Bounds));
}

#[test]
fn unknown_magic_version_flags_and_reserved_fields_are_rejected() {
    for (index, value, expected) in [(0, b'X', Error::Version), (4, 2, Error::Version),
        (5, 4, Error::Flags), (6, 1, Error::Flags), (7, 1, Error::Flags)] {
        let mut bytes = wire::encode(&plan()).unwrap();
        bytes[index] = value;
        assert_eq!(wire::decode(&bytes), Err(expected));
    }
}

#[test]
fn counts_and_screen_lengths_are_bounded_before_allocation() {
    for (index, value, expected) in [(92, 17_u16, Error::Bounds), (92, u16::MAX, Error::Bounds),
        (94, 0, Error::Screen), (94, 257, Error::Screen), (94, u16::MAX, Error::Screen)] {
        let mut bytes = wire::encode(&plan()).unwrap();
        bytes[index..index + 2].copy_from_slice(&value.to_le_bytes());
        assert_eq!(wire::decode(&bytes), Err(expected));
    }
    let original = plan();
    let mut too_many = original.clone();
    too_many.slices = vec![original.slices[0].clone(); 17];
    assert_eq!(wire::encode(&too_many), Err(Error::Bounds));
}

#[test]
fn screen_names_require_bounded_utf8_without_control_characters() {
    for screen in ["".to_owned(), "a".repeat(257), "\n".into(), "\0".into(), "\u{85}".into()] {
        let mut original = plan();
        original.slices[0].screen = screen;
        assert_eq!(wire::encode(&original), Err(Error::Screen));
    }
    for value in [0xff, 0, b'\n'] {
        let mut bytes = wire::encode(&plan()).unwrap();
        bytes[96] = value;
        assert_eq!(wire::decode(&bytes), Err(Error::Screen));
    }
    let mut original = plan();
    original.slices[0].screen = "é".repeat(128);
    assert_eq!(wire::decode(&wire::encode(&original).unwrap()), Ok(original));
}

#[test]
fn total_size_budget_applies_to_encoder_and_decoder() {
    let mut original = plan();
    original.slices = vec![original.slices[0].clone(); 16];
    for slice in &mut original.slices { slice.screen = "a".repeat(200); }
    let bytes = wire::encode(&original).unwrap();
    assert!(bytes.len() <= wire::MAX_PAYLOAD);
    assert_eq!(wire::decode(&bytes), Ok(original.clone()));
    for slice in &mut original.slices { slice.screen = "a".repeat(256); }
    assert_eq!(wire::encode(&original), Err(Error::Bounds));
}

#[test]
fn malformed_payload_discards_intent_without_resetting_replay_protection() {
    let mut gate = gate();
    let bytes = wire::encode(&plan()).unwrap();
    gate.accept_encoded(&owner(), &bytes, true, Some(7)).unwrap();
    assert!(gate.active().is_some());
    assert_eq!(gate.accept_encoded(&owner(), b"bad", true, Some(7)), Err(AdmissionError::Wire));
    assert!(gate.active().is_none());
    assert_eq!(gate.accept_encoded(&owner(), &bytes, true, Some(7)), Err(AdmissionError::Replay));
    let fresh = wire::encode(&PresentationPlan { sequence: 2, ..plan() }).unwrap();
    gate.accept_encoded(&owner(), &fresh, true, Some(7)).unwrap();
    assert!(gate.active().is_some());
}

#[test]
fn independent_live_consent_and_identity_are_checked_before_payload() {
    let bytes = wire::encode(&plan()).unwrap();
    for (enabled, revision) in [(false, Some(7)), (true, None), (true, Some(8))] {
        let mut gate = gate();
        gate.accept_encoded(&owner(), &bytes, true, Some(7)).unwrap();
        assert_eq!(gate.accept_encoded(&owner(), b"bad", enabled, revision), Err(AdmissionError::Revoked));
        assert!(gate.active().is_none());
        assert_eq!(gate.accept_encoded(&owner(), &bytes, true, Some(7)), Err(AdmissionError::Revoked));
    }
    let mut gate = gate();
    gate.accept_encoded(&owner(), &bytes, true, Some(7)).unwrap();
    let stranger = Owner { pid: 1235, ..owner() };
    assert_eq!(gate.accept_encoded(&stranger, b"bad", false, None), Err(AdmissionError::Identity));
    assert!(gate.active().is_none());
    let fresh = wire::encode(&PresentationPlan { sequence: 2, ..plan() }).unwrap();
    gate.accept_encoded(&owner(), &fresh, true, Some(7)).unwrap();
    assert!(gate.active().is_some());
}

#[test]
fn decoded_semantics_still_require_admission_and_owner_loss_is_terminal() {
    let mut gate = gate();
    let bytes = wire::encode(&PresentationPlan { consent_revision: 8, ..plan() }).unwrap();
    assert_eq!(gate.accept_encoded(&owner(), &bytes, true, Some(7)), Err(AdmissionError::Consent));
    gate.owner_lost();
    assert_eq!(gate.accept_encoded(&owner(), &wire::encode(&plan()).unwrap(), true, Some(7)),
        Err(AdmissionError::Revoked));
    assert!(gate.active().is_none());
}

#[test]
fn arbitrary_bounded_bytes_never_panic_or_produce_noncanonical_payloads() {
    let mut seed = 17_u64;
    for length in 0..=wire::MAX_PAYLOAD + 1 {
        let mut bytes = vec![0; length];
        for byte in &mut bytes {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *byte = (seed >> 32) as u8;
        }
        if let Ok(decoded) = wire::decode(&bytes) { assert_eq!(wire::encode(&decoded).unwrap(), bytes); }
    }
    let bytes = wire::encode(&plan()).unwrap();
    for index in 0..bytes.len() {
        for value in 0..=255 {
            let mut changed = bytes.clone();
            changed[index] = value;
            if let Ok(decoded) = wire::decode(&changed) { assert_eq!(wire::encode(&decoded).unwrap(), changed); }
        }
    }
}
