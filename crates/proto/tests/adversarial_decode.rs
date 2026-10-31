use one_mk_proto::{ClipHeader, Control, Datagram, FileMsg, PROTOCOL_VERSION};

fn exercise(bytes: &[u8]) {
    let _ = one_mk_proto::wallpaper::Message::decode(bytes);
    if let Ok(message) = Datagram::decode(bytes) {
        let (encoded, length) = message.encode();
        assert_eq!(Datagram::decode(&encoded[..length]), Ok(message));
    }
    if let Ok(message) = Control::decode(bytes) {
        let mut encoded = Vec::new();
        message.encode_frame(&mut encoded);
        assert_eq!(Control::decode(&encoded[2..]), Ok(message));
    }
    if let Ok(message) = ClipHeader::decode(bytes) {
        let mut encoded = Vec::new();
        message.encode_frame(&mut encoded);
        assert_eq!(ClipHeader::decode(&encoded[2..]), Ok(message));
    }
    if let Ok(message) = FileMsg::decode(bytes) {
        let mut encoded = Vec::new();
        message.encode_frame(&mut encoded);
        assert_eq!(FileMsg::decode(&encoded[2..]), Ok(message));
    }
}

#[test]
fn deterministic_wire_mutation_stress() {
    let mut state = 0x7761_7665_365f_7161_u64;
    for iteration in 0..100_000 {
        let mut bytes = vec![0; iteration % 513];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
        if let Some(tag) = bytes.first_mut() { *tag = (iteration % 256) as u8; }
        exercise(&bytes);
    }
    for byte in [0, 1, 25, 127, 255] { exercise(&vec![byte; 65535]); }
    for message in [
        Control::Hello { version: PROTOCOL_VERSION, name: "hostile\u{202e}name".into(), screens: vec![], nonce: u64::MAX, os: 0 },
        Control::Key { code: u32::MAX, down: true },
        Control::Scroll { dx: f32::MAX, dy: f32::MIN },
    ] {
        let mut encoded = Vec::new();
        message.encode_frame(&mut encoded);
        for length in 0..=encoded.len() - 2 { exercise(&encoded[2..2 + length]); }
        for index in 2..encoded.len() {
            for byte in [0, 1, 127, 255] {
                let mut mutated = encoded.clone();
                mutated[index] = byte;
                exercise(&mutated[2..]);
            }
        }
    }
}
