//! Wire format conformance: every message round-trips, every truncation is
//! rejected (or is the older, shorter form of the same message), sizes stay
//! inside the frame limits the network layer enforces, and version
//! negotiation refuses what it must. Pseudo-random cases use fixed seeds.

use one_mk_proto::typing::LayoutDescriptor;
use one_mk_proto::wallpaper::{self, Kind};
use one_mk_proto::{
    Batch, ClipFormat, ClipHeader, Control, Datagram, DecodeError, FileMsg, MAX_SCREENS, PROTOCOL_VERSION, Pos, Rect, Side,
    StreamKind, caps, os,
};

/// xorshift64*: deterministic, no dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
    fn flag(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn pos(&mut self) -> Pos {
        // Multiples of 1/256 px are exactly representable on the wire.
        Pos::new(self.range(-4_000_000, 4_000_000) as f64 / 256.0, self.range(-4_000_000, 4_000_000) as f64 / 256.0)
    }
    fn rects(&mut self) -> Vec<Rect> {
        (0..self.below(5)).map(|_| Rect::new(self.range(-20000, 20000) as i32, self.range(-20000, 20000) as i32, self.range(1, 16000) as i32, self.range(1, 16000) as i32)).collect()
    }
    fn string(&mut self, max_chars: u64) -> String {
        const ALPHABET: [char; 12] = ['a', 'Z', '7', ' ', '-', 'é', 'ß', '界', '😀', 'ñ', '_', '.'];
        (0..self.below(max_chars + 1)).map(|_| ALPHABET[self.below(ALPHABET.len() as u64) as usize]).collect()
    }
    fn finite_f32(&mut self) -> f32 {
        loop {
            let v = f32::from_bits(self.next() as u32);
            if v.is_finite() {
                return v;
            }
        }
    }
}

fn body(message: &Control) -> Vec<u8> {
    let mut frame = Vec::new();
    message.encode_frame(&mut frame);
    let len = u16::from_le_bytes([frame[0], frame[1]]) as usize;
    assert_eq!(len, frame.len() - 2, "the length prefix counts exactly the body: {message:?}");
    frame.split_off(2)
}

fn wallpaper_message(kind: Kind) -> wallpaper::Message {
    let mut descriptor = [0; 48];
    match kind {
        Kind::Canvas => {
            descriptor[..4].copy_from_slice(&7680u32.to_le_bytes());
            descriptor[4..8].copy_from_slice(&2160u32.to_le_bytes());
            descriptor[8] = 3;
            descriptor[9] = 5;
        }
        Kind::Renderer => descriptor[0] = 1,
        _ => {}
    }
    wallpaper::Message { kind, coordinator: [0xC0; 32], epoch: [0xE0; 16], sequence: u64::MAX, descriptor }
}

fn layout(name: &str, variant: &str) -> LayoutDescriptor {
    LayoutDescriptor { name: name.into(), variant: variant.into(), rules: "evdev".into(), model: "pc105".into() }
}

/// One of every variant, at their extremes.
fn catalogue() -> Vec<Control> {
    let mut all = vec![
        Control::Hello { version: PROTOCOL_VERSION, name: String::new(), screens: vec![], nonce: 0, os: os::UNKNOWN },
        Control::Hello {
            version: PROTOCOL_VERSION,
            name: "x".repeat(255),
            screens: (0..MAX_SCREENS as i32).map(|i| Rect::new(i * 100, -i, 100, 50)).collect(),
            nonce: u64::MAX,
            os: os::WINDOWS,
        },
        Control::Screens(vec![Rect::new(i32::MIN, i32::MIN, 1, 1)]),
        Control::Screens(vec![Rect::new(0, 0, i32::MAX, i32::MAX)]),
        Control::Enter { epoch: u16::MAX, pos: Pos::new(-8_000_000.0, 8_000_000.0) },
        Control::Leave,
        Control::Key { code: u32::MAX, down: false },
        Control::Button { button: 0x114, down: true, seq: u32::MAX, pos: Pos::new(0.00390625, -0.00390625) },
        Control::Scroll { dx: f32::MIN_POSITIVE, dy: -0.0 },
        Control::Scroll { dx: f32::MAX, dy: f32::MIN },
        Control::ScrollDiscrete { dx: i32::MIN, dy: i32::MAX },
        Control::ScrollStop,
        Control::EnterAck { epoch: 0 },
        Control::Busy { epoch: 1 },
        Control::Revoked { epoch: 2 },
        Control::IdleRevoked { epoch: 3 },
        Control::Caps(u32::MAX),
        Control::PointerMode { epoch: 4, relative: true },
        Control::Lock { id: 0, all: false },
        Control::Inhibit { epoch: 5, on: false },
        Control::Sleep { id: 1 },
        Control::Text { text: "é".into() },
        Control::Text { text: "a".into() },
        Control::Text { text: "👍".into() },
        Control::CharacterKey { code: 30, down: true, character: 'ß' },
        Control::CharacterKey { code: 767, down: false, character: '界' },
        Control::KeyboardLayout(Box::new(layout("de", "nodeadkeys"))),
        Control::KeyboardLayout(Box::new(LayoutDescriptor { name: "x".repeat(128), variant: String::new(), rules: String::new(), model: String::new() })),
        Control::LiveWallpaper(String::new()),
        Control::LiveWallpaper("{\"action\":\"on\"}".into()),
        Control::LiveWallpaper("z".repeat(12000)),
        Control::Focus { epoch: 6, fullscreen: true, apps: vec!["a".repeat(255), "b".into(), "c".into(), "d".into()] },
        Control::WatchFocus { epoch: 7, on: true, app: true, fullscreen: false },
    ];
    for side in Side::ALL {
        all.push(Control::Layout { side_of_you: side, version: u64::MAX, offset: Some((1 << 24, -(1 << 24))), clear_offset: false });
        all.push(Control::Layout { side_of_you: side, version: 0, offset: None, clear_offset: true });
        all.push(Control::Layout { side_of_you: side, version: 5, offset: None, clear_offset: false });
    }
    for kind in [Kind::Version, Kind::Renderer, Kind::Canvas, Kind::ConsentRequest, Kind::ConsentGrant, Kind::ConsentRevoke] {
        all.push(Control::Wallpaper(wallpaper_message(kind)));
    }
    all
}

/// A random, canonical (so exactly round-trippable) message.
fn random_control(rng: &mut Rng) -> Control {
    match rng.below(31) {
        0 => Control::Hello { version: PROTOCOL_VERSION, name: rng.string(60), screens: rng.rects(), nonce: rng.next(), os: rng.below(4) as u8 },
        1 => Control::Screens(rng.rects()),
        2 => Control::Enter { epoch: rng.next() as u16, pos: rng.pos() },
        3 => Control::Leave,
        4 => Control::Key { code: rng.next() as u32, down: rng.flag() },
        5 => Control::Button { button: rng.next() as u32, down: rng.flag(), seq: rng.next() as u32, pos: rng.pos() },
        6 => Control::Scroll { dx: rng.finite_f32(), dy: rng.finite_f32() },
        7 => Control::ScrollDiscrete { dx: rng.next() as i32, dy: rng.next() as i32 },
        8 => Control::ScrollStop,
        9 => Control::EnterAck { epoch: rng.next() as u16 },
        10 => Control::Busy { epoch: rng.next() as u16 },
        11 => Control::Revoked { epoch: rng.next() as u16 },
        12 => Control::IdleRevoked { epoch: rng.next() as u16 },
        13 => {
            let offset = rng.flag().then(|| (rng.range(-(1 << 24), 1 << 24) as i32, rng.range(-(1 << 24), 1 << 24) as i32));
            let clear_offset = offset.is_none() && rng.flag();
            Control::Layout { side_of_you: Side::ALL[rng.below(4) as usize], version: rng.next(), offset, clear_offset }
        }
        14 => Control::Caps(rng.next() as u32),
        15 => Control::PointerMode { epoch: rng.next() as u16, relative: rng.flag() },
        16 => Control::Lock { id: rng.next(), all: rng.flag() },
        17 => Control::Focus { epoch: rng.next() as u16, fullscreen: rng.flag(), apps: Vec::new() },
        18 => Control::WatchFocus { epoch: rng.next() as u16, on: rng.flag(), app: rng.flag(), fullscreen: rng.flag() },
        19 => Control::Inhibit { epoch: rng.next() as u16, on: rng.flag() },
        20 => Control::Sleep { id: rng.next() },
        21 => Control::Text { text: ["a", "Z", "é", "界", "😀", "9", "~"][rng.below(7) as usize].into() },
        22 => Control::CharacterKey {
            code: rng.range(1, 767) as u32,
            down: rng.flag(),
            character: ['a', 'é', '界', '😀', '~', ' '][rng.below(6) as usize],
        },
        23 => Control::KeyboardLayout(Box::new(layout(&format!("l{}", rng.below(99)), &rng.string(10).replace('\u{1f600}', "x")))),
        24 => Control::LiveWallpaper(rng.string(200)),
        25 => Control::Wallpaper(wallpaper_message(Kind::Version)),
        26 => Control::Wallpaper(wallpaper_message(Kind::Canvas)),
        _ => Control::Caps(caps::LOCAL),
    }
}

#[test]
fn every_variant_round_trips_and_its_length_prefix_is_exact() {
    for message in catalogue() {
        let encoded = body(&message);
        assert_eq!(Control::decode(&encoded).as_ref(), Ok(&message), "{message:?}");
        assert!(encoded.len() <= Control::MAX_LEN, "{message:?} is {} bytes", encoded.len());
        assert!(encoded.len() <= u16::MAX as usize);
    }
}

/// A cut-off message is an error, never a panic and never a different
/// message of the same tag. The only exceptions are the messages that grew
/// a tail within protocol 2: their older, shorter form is the same message
/// with the default for the missing part, and that is what is returned.
#[test]
fn every_truncation_is_rejected_or_the_older_shorter_form() {
    for message in catalogue() {
        let encoded = body(&message);
        for cut in 0..encoded.len() {
            let result = Control::decode(&encoded[..cut]);
            match result {
                Err(_) => {}
                Ok(shorter) => {
                    let tag = encoded[0];
                    assert!(
                        matches!(tag, 13 | 16 | 17 | 18),
                        "{message:?} cut to {cut} bytes decoded as {shorter:?}"
                    );
                    // Never a message of another kind.
                    assert_eq!(std::mem::discriminant(&shorter), std::mem::discriminant(&message), "{message:?} at {cut}");
                }
            }
        }
        assert_eq!(Control::decode(&[]), Err(DecodeError::Truncated));
    }
}

#[test]
fn tails_after_a_fixed_message_are_ignored_but_typing_messages_are_strict() {
    for message in catalogue() {
        let mut long = body(&message);
        long.extend_from_slice(&[0xA5; 7]);
        let strict = matches!(
            message,
            Control::Text { .. } | Control::CharacterKey { .. } | Control::KeyboardLayout(_) | Control::Wallpaper(_)
        );
        // Layout, Lock, Focus and WatchFocus read their optional tail from
        // those bytes, so the result may legitimately differ; they must not panic.
        let optional_tail = matches!(
            message,
            Control::Layout { .. } | Control::Lock { .. } | Control::Focus { .. } | Control::WatchFocus { .. }
        );
        match Control::decode(&long) {
            Ok(same) if !strict && !optional_tail => assert_eq!(same, message),
            Ok(_) => assert!(optional_tail, "{message:?} accepted trailing bytes"),
            Err(_) => assert!(strict || optional_tail, "{message:?} rejected a harmless tail"),
        }
    }
}

#[test]
fn random_messages_round_trip_alone_and_back_to_back() {
    let mut rng = Rng(0x00C0_FFEE_1234_5678);
    let mut stream = Vec::new();
    let mut sent = Vec::new();
    for round in 0..20_000 {
        let message = random_control(&mut rng);
        let encoded = body(&message);
        assert_eq!(Control::decode(&encoded).as_ref(), Ok(&message), "round {round}: {message:?}");
        message.encode_frame(&mut stream);
        sent.push(message);
        if sent.len() == 64 {
            let mut rest = &stream[..];
            for expected in sent.drain(..) {
                let len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
                assert_eq!(Control::decode(&rest[2..2 + len]), Ok(expected));
                rest = &rest[2 + len..];
            }
            assert!(rest.is_empty());
            stream.clear();
        }
    }
}

/// Re-encoding what a hostile peer sent never produces a frame that is
/// itself rejected or grows (so a relaying peer cannot be made to emit
/// garbage).
#[test]
fn decode_encode_is_stable_for_arbitrary_bytes() {
    let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
    let mut accepted = 0;
    for round in 0..200_000u32 {
        let len = rng.below(48) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if let Some(tag) = bytes.first_mut() {
            // Mostly real tags, so the interesting branches run.
            *tag = (round % 32) as u8;
        }
        if let Ok(message) = Control::decode(&bytes) {
            accepted += 1;
            let once = body(&message);
            let again = Control::decode(&once).unwrap_or_else(|e| panic!("{bytes:?} -> {message:?} -> {once:?}: {e}"));
            assert_eq!(body(&again), once, "{message:?}");
        }
    }
    assert!(accepted > 5_000, "the generator reaches the decoders ({accepted})");
}

#[test]
fn only_a_hello_carries_a_version_and_only_ours_is_accepted() {
    let hello = |version: u16| {
        let mut b = body(&Control::Hello { version: PROTOCOL_VERSION, name: "peer".into(), screens: vec![Rect::new(0, 0, 10, 10)], nonce: 9, os: os::LINUX });
        b[1..3].copy_from_slice(&version.to_le_bytes());
        b
    };
    for version in 0..=u16::MAX {
        let result = Control::decode(&hello(version));
        if version == PROTOCOL_VERSION {
            assert!(matches!(result, Ok(Control::Hello { version: v, .. }) if v == PROTOCOL_VERSION));
        } else {
            assert_eq!(result, Err(DecodeError::Version(version)));
        }
    }
    // The version is checked before the rest is parsed: a future layout we
    // cannot read still says "wrong version", not "garbage".
    for tail in [&[][..], &[0xFF; 3], &[0xFF; 200]] {
        let mut future = vec![1];
        future.extend_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        future.extend_from_slice(tail);
        assert_eq!(Control::decode(&future), Err(DecodeError::Version(PROTOCOL_VERSION + 1)));
    }
    // A hello cut inside the version is truncation.
    assert_eq!(Control::decode(&[1, 2]), Err(DecodeError::Truncated));
    assert_eq!(Control::decode(&[1]), Err(DecodeError::Truncated));
    // Errors read well enough to show a person.
    assert!(DecodeError::Version(7).to_string().contains('7'));
    assert!(DecodeError::UnknownTag(200).to_string().contains("200"));
}

#[test]
fn hello_names_and_screens_are_cut_not_corrupted() {
    // 300 bytes of two-byte characters: cut on a character boundary.
    let name = "é".repeat(150);
    let b = body(&Control::Hello { version: PROTOCOL_VERSION, name, screens: vec![], nonce: 1, os: os::MACOS });
    let Ok(Control::Hello { name, .. }) = Control::decode(&b) else { panic!("a long name must not break the hello") };
    assert_eq!(name.len(), 254);
    assert!(name.chars().all(|c| c == 'é'));
    // Four-byte characters: 255 / 4 = 63 whole ones.
    let b = body(&Control::Hello { version: PROTOCOL_VERSION, name: "😀".repeat(100), screens: vec![], nonce: 1, os: os::MACOS });
    let Ok(Control::Hello { name, .. }) = Control::decode(&b) else { panic!() };
    assert_eq!(name.chars().count(), 63);
    // More monitors than the limit are cut to the limit, and the rest of the hello still reads.
    let many: Vec<Rect> = (0..200).map(|i| Rect::new(i * 10, 0, 10, 10)).collect();
    let b = body(&Control::Hello { version: PROTOCOL_VERSION, name: "n".into(), screens: many.clone(), nonce: 77, os: os::LINUX });
    let Ok(Control::Hello { screens, nonce, os: o, .. }) = Control::decode(&b) else { panic!() };
    assert_eq!((screens.len(), nonce, o), (MAX_SCREENS, 77, os::LINUX));
    assert_eq!(screens[..], many[..MAX_SCREENS]);
}

#[test]
fn bad_strings_in_a_hello_are_rejected() {
    // name length 2, invalid UTF-8.
    let mut b = vec![1];
    b.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    b.extend_from_slice(&[2, 0xC0, 0xAF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(Control::decode(&b), Err(DecodeError::BadString));
    // Name length runs past the end.
    let mut b = vec![1];
    b.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    b.extend_from_slice(&[200, b'a', b'b']);
    assert_eq!(Control::decode(&b), Err(DecodeError::Truncated));
}

#[test]
fn typing_messages_refuse_what_a_hostile_peer_could_send() {
    let invalid = |bytes: &[u8]| matches!(Control::decode(bytes), Err(DecodeError::Invalid(_)));
    let text = |s: &[u8]| {
        let mut b = vec![21];
        b.extend_from_slice(&(s.len() as u16).to_le_bytes());
        b.extend_from_slice(s);
        b
    };
    assert!(invalid(&text(b"")), "empty text");
    assert!(invalid(&text(b"ab")), "two characters");
    assert!(invalid(&text("\u{7}".as_bytes())), "a control character");
    assert!(invalid(&text("\u{200d}".as_bytes())), "a zero-width joiner");
    assert!(invalid(&text("\u{e000}".as_bytes())), "private use");
    assert!(invalid(&text("\u{a0}".as_bytes())), "a no-break space is not a typeable character");
    assert!(invalid(&text("\n".as_bytes())), "a newline goes through Enter, not as text");
    assert!(invalid(&text(&[b'a'; 9])), "over the limit");
    assert_eq!(Control::decode(&text(&[0xFF])), Err(DecodeError::BadString));
    assert_eq!(Control::decode(&text(" ".as_bytes())), Ok(Control::Text { text: " ".into() }), "a space is a character");
    // An invalid text is sent as an empty one, which the receiver rejects: callers (the typing
    // `Sender`) validate first, so nothing invalid is ever built.
    let b = body(&Control::Text { text: "two".into() });
    assert!(Control::decode(&b).is_err());

    let key = |code: u32, down: u8, ch: u32| {
        let mut b = vec![22];
        b.extend_from_slice(&code.to_le_bytes());
        b.push(down);
        b.extend_from_slice(&ch.to_le_bytes());
        b
    };
    assert!(Control::decode(&key(30, 1, 'a' as u32)).is_ok());
    assert!(invalid(&key(0, 1, 'a' as u32)), "key code 0");
    assert!(invalid(&key(768, 1, 'a' as u32)), "key code above the evdev range");
    assert!(invalid(&key(30, 2, 'a' as u32)), "direction other than 0 and 1");
    assert!(invalid(&key(30, 1, 0xD800)), "a surrogate");
    assert!(invalid(&key(30, 1, 0x110000)), "beyond Unicode");
    assert!(invalid(&key(30, 1, 7)), "a control character");
    let mut long = key(30, 1, 'a' as u32);
    long.push(0);
    assert!(invalid(&long), "trailing bytes");

    let layout_frame = |fields: [&str; 4]| {
        let mut b = vec![23];
        for f in fields {
            b.extend_from_slice(&(f.len() as u16).to_le_bytes());
            b.extend_from_slice(f.as_bytes());
        }
        b
    };
    assert!(Control::decode(&layout_frame(["us", "", "evdev", "pc105"])).is_ok());
    assert!(invalid(&layout_frame(["", "", "", ""])), "a layout needs a name");
    assert!(invalid(&layout_frame(["us\n", "", "", ""])), "control characters");
    assert!(invalid(&layout_frame([&"x".repeat(129), "", "", ""])), "a name over 128 bytes");
}

#[test]
fn live_wallpaper_and_wallpaper_frames_are_bounded() {
    // 12000 bytes pass, 12001 are refused, and sending more cuts at the limit.
    for (len, ok) in [(0usize, true), (11999, true), (12000, true), (12001, false), (60000, false)] {
        let mut b = vec![28];
        b.extend_from_slice(&(len as u16).to_le_bytes());
        b.extend(std::iter::repeat_n(b'q', len));
        assert_eq!(Control::decode(&b).is_ok(), ok, "{len}");
    }
    let cut = body(&Control::LiveWallpaper("é".repeat(9000)));
    let Ok(Control::LiveWallpaper(s)) = Control::decode(&cut) else { panic!() };
    assert_eq!(s.len(), 12000);
    assert!(cut.len() <= Control::MAX_LEN);
    // The wallpaper envelope is fixed-size: nothing else is accepted.
    for len in [0usize, 1, wallpaper::BODY_LEN - 1, wallpaper::BODY_LEN + 1, 4096] {
        let mut b = vec![wallpaper::CONTROL_TAG];
        b.extend(std::iter::repeat_n(0u8, len));
        assert!(matches!(Control::decode(&b), Err(DecodeError::Invalid(_))), "{len}");
    }
}

#[test]
fn wallpaper_messages_validate_every_field() {
    let good = wallpaper_message(Kind::Canvas);
    let bytes = good.encode();
    assert_eq!(wallpaper::Message::decode(&bytes), Ok(good));
    let mutate = |index: usize, value: u8| {
        let mut b = bytes;
        b[index] = value;
        wallpaper::Message::decode(&b)
    };
    assert!(mutate(0, 2).is_err(), "another envelope version");
    assert!(mutate(2, 9).is_err(), "unknown kind");
    assert!(mutate(5, 1).is_err(), "reserved bytes must be zero");
    assert!(mutate(40, 0).is_ok(), "epoch only has to be non-zero as a whole");
    // Zero epoch, zero sequence.
    let mut zero_epoch = bytes;
    zero_epoch[40..56].fill(0);
    assert!(wallpaper::Message::decode(&zero_epoch).is_err());
    let mut zero_seq = bytes;
    zero_seq[56..64].fill(0);
    assert!(wallpaper::Message::decode(&zero_seq).is_err());
    // Canvas bounds: width/height 1..=2_000_000, 1..=8 devices, outputs devices..=16.
    let canvas = |w: u32, h: u32, devices: u8, outputs: u8| {
        let mut m = good;
        m.descriptor[..4].copy_from_slice(&w.to_le_bytes());
        m.descriptor[4..8].copy_from_slice(&h.to_le_bytes());
        m.descriptor[8] = devices;
        m.descriptor[9] = outputs;
        wallpaper::Message::decode(&m.encode()).is_ok()
    };
    assert!(canvas(1, 1, 1, 1));
    assert!(canvas(2_000_000, 2_000_000, 8, 16));
    assert!(!canvas(0, 1, 1, 1));
    assert!(!canvas(1, 0, 1, 1));
    assert!(!canvas(2_000_001, 1, 1, 1));
    assert!(!canvas(1, 2_000_001, 1, 1));
    assert!(!canvas(1, 1, 0, 1));
    assert!(!canvas(1, 1, 9, 9));
    assert!(!canvas(1, 1, 3, 2), "fewer outputs than devices");
    assert!(!canvas(1, 1, 1, 17));
    // Descriptors of the other kinds are all zero (renderer: one flag byte 0/1).
    for kind in [Kind::Version, Kind::ConsentRequest, Kind::ConsentGrant, Kind::ConsentRevoke] {
        let mut m = wallpaper_message(kind);
        m.descriptor[7] = 1;
        assert!(wallpaper::Message::decode(&m.encode()).is_err(), "{kind:?}");
    }
    let mut renderer = wallpaper_message(Kind::Renderer);
    renderer.descriptor[0] = 2;
    assert!(wallpaper::Message::decode(&renderer.encode()).is_err());
}

#[test]
fn datagrams_round_trip_and_reject_cuts_and_unknown_tags() {
    let mut rng = Rng(42);
    for _ in 0..5000 {
        let datagram = match rng.below(4) {
            0 => Datagram::Motion { epoch: rng.next() as u16, seq: rng.next() as u32, pos: rng.pos() },
            1 => Datagram::MotionBy { epoch: rng.next() as u16, seq: rng.next() as u32, sum_x: rng.next() as i32, sum_y: rng.next() as i32 },
            2 => Datagram::Probe { id: rng.next() as u32, sent_ns: rng.next() },
            _ => Datagram::ProbeEcho { id: rng.next() as u32, sent_ns: rng.next() },
        };
        let (buf, n) = datagram.encode();
        assert!(n <= Datagram::MAX_LEN);
        assert_eq!(Datagram::decode(&buf[..n]), Ok(datagram));
        for cut in 0..n {
            assert_eq!(Datagram::decode(&buf[..cut]), Err(DecodeError::Truncated), "{datagram:?} at {cut}");
        }
        // UDP padding or a longer future form is read by its known prefix.
        let mut padded = buf[..n].to_vec();
        padded.extend_from_slice(&[0xEE; 9]);
        assert_eq!(Datagram::decode(&padded), Ok(datagram));
    }
    for tag in [0u8, 2, 6, 7, 100, 255] {
        assert_eq!(Datagram::decode(&[tag; 20]), Err(DecodeError::UnknownTag(tag)));
    }
}

#[test]
fn stream_kinds_cover_every_byte_exactly_once() {
    let known: Vec<_> = (0..=255u8).filter_map(|b| StreamKind::from_u8(b).map(|k| (b, k))).collect();
    assert_eq!(known.len(), 6);
    for (byte, kind) in known {
        assert_eq!(kind as u8, byte);
    }
}

#[test]
fn clipboard_headers_obey_their_limits() {
    // More than 16 items are cut to 16 when sending.
    let items: Vec<_> = (0..40).map(|i| (if i % 2 == 0 { ClipFormat::Text } else { ClipFormat::Png }, i as u32)).collect();
    let header = ClipHeader { items: items.clone(), pull: false, concealed: false };
    let mut frame = Vec::new();
    header.encode_frame(&mut frame);
    let decoded = ClipHeader::decode(&frame[2..]).unwrap();
    assert_eq!(decoded.items, items[..16]);
    // The total of many huge items cannot overflow.
    let huge = ClipHeader { items: vec![(ClipFormat::Png, u32::MAX); 255], pull: false, concealed: false };
    assert_eq!(huge.total(), 255 * u64::from(u32::MAX));
    // 255 items decode (a peer may send them); the payload sizes stay the caller's to bound.
    let mut raw = vec![255u8];
    for _ in 0..255 {
        raw.extend_from_slice(&[ClipFormat::Html as u8, 1, 0, 0, 0]);
    }
    assert_eq!(ClipHeader::decode(&raw).unwrap().items.len(), 255);
    // A pull and a private probe carry no items and are distinct.
    for (header, pull, concealed) in [(ClipHeader::pull_request(), true, false), (ClipHeader::private_probe(), false, true)] {
        let mut f = Vec::new();
        header.encode_frame(&mut f);
        let back = ClipHeader::decode(&f[2..]).unwrap();
        assert_eq!((back.items.len(), back.pull, back.concealed), (0, pull, concealed));
    }
    // Every prefix of an item list is truncation, except exactly at an item boundary after n items.
    let mut frame = Vec::new();
    ClipHeader { items: vec![(ClipFormat::Text, 5), (ClipFormat::Png, 9)], pull: true, concealed: true }.encode_frame(&mut frame);
    for cut in 0..frame.len() - 3 {
        assert!(ClipHeader::decode(&frame[2..2 + cut]).is_err(), "cut {cut}");
    }
    // Unknown format tags are refused, not skipped (the payload layout depends on them).
    for tag in [0u8, 4, 5, 200, 255] {
        assert_eq!(ClipHeader::decode(&[1, tag, 1, 0, 0, 0]), Err(DecodeError::UnknownTag(tag)));
    }
    for (b, f) in [(1u8, ClipFormat::Text), (2, ClipFormat::Html), (3, ClipFormat::Png)] {
        assert_eq!(ClipFormat::from_u8(b), Some(f));
    }
}

#[test]
fn file_messages_fit_their_frame_and_round_trip_at_the_extremes() {
    let batch = Batch { id: u64::MAX, files: u32::MAX, bytes: u64::MAX };
    let worst = [
        FileMsg::Offer { path: "p".repeat(FileMsg::MAX_PATH), size: u64::MAX, mtime: u64::MAX, batch },
        FileMsg::Offer { path: String::new(), size: 0, mtime: 0, batch: Batch::default() },
        FileMsg::Accept { offset: u64::MAX },
        FileMsg::Reject { reason: "r".repeat(1024) },
        FileMsg::End { hash: [0xFF; 32] },
        FileMsg::Saved { name: "n".repeat(FileMsg::MAX_PATH) },
        FileMsg::Failed { reason: "é".repeat(512) },
    ];
    for message in worst {
        let mut frame = Vec::new();
        message.encode_frame(&mut frame);
        assert!(frame.len() - 2 <= FileMsg::MAX_LEN, "{} bytes", frame.len());
        assert_eq!(FileMsg::decode(&frame[2..]).as_ref(), Ok(&message));
    }
    // Over-long reasons and names are cut at the limit on a character boundary, never rejected.
    let mut frame = Vec::new();
    FileMsg::Reject { reason: "é".repeat(2000) }.encode_frame(&mut frame);
    let Ok(FileMsg::Reject { reason }) = FileMsg::decode(&frame[2..]) else { panic!() };
    assert_eq!(reason.len(), 1024);
    // A hostile Offer whose declared string is longer than the frame is truncation.
    let mut offer = vec![1];
    offer.extend_from_slice(&5000u16.to_le_bytes());
    offer.extend_from_slice(&[b'a'; 10]);
    assert_eq!(FileMsg::decode(&offer), Err(DecodeError::Truncated));
}

#[test]
fn random_file_and_clipboard_headers_round_trip() {
    let mut rng = Rng(0xF11E);
    for _ in 0..5000 {
        let message = match rng.below(6) {
            0 => FileMsg::Offer {
                path: rng.string(100),
                size: rng.next(),
                mtime: rng.next(),
                batch: if rng.flag() { Batch::default() } else { Batch { id: rng.next() | 1, files: rng.next() as u32, bytes: rng.next() } },
            },
            1 => FileMsg::Accept { offset: rng.next() },
            2 => FileMsg::Reject { reason: rng.string(100) },
            3 => FileMsg::End { hash: std::array::from_fn(|_| rng.next() as u8) },
            4 => FileMsg::Saved { name: rng.string(100) },
            _ => FileMsg::Failed { reason: rng.string(100) },
        };
        let mut frame = Vec::new();
        message.encode_frame(&mut frame);
        assert_eq!(FileMsg::decode(&frame[2..]), Ok(message));
        let header = ClipHeader {
            items: (0..rng.below(17)).map(|_| ([ClipFormat::Text, ClipFormat::Html, ClipFormat::Png][rng.below(3) as usize], rng.next() as u32)).collect(),
            pull: rng.flag(),
            concealed: rng.flag(),
        };
        let mut frame = Vec::new();
        header.encode_frame(&mut frame);
        assert_eq!(ClipHeader::decode(&frame[2..]), Ok(header));
    }
}

#[test]
fn capability_bits_are_stable_and_local_ones_are_a_subset() {
    // The numeric values are the wire contract with already released builds.
    assert_eq!(
        [caps::RELATIVE, caps::LOCK, caps::FOCUS, caps::POWER, caps::FOCUS_BEAT, caps::FOCUS_APP, caps::TEXT, caps::IDLE_REVOKE, caps::WALLPAPER, caps::CLIPBOARD_FETCH, caps::TEXT_SOURCE, caps::TEXT_HOLD, caps::LIVE_WALLPAPER],
        [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]
    );
    // An unknown bit from a newer peer survives decoding untouched.
    let b = body(&Control::Caps(caps::LOCAL | 1 << 31));
    assert_eq!(Control::decode(&b), Ok(Control::Caps(caps::LOCAL | 1 << 31)));
    assert_eq!(os::local(), if cfg!(target_os = "linux") { os::LINUX } else if cfg!(windows) { os::WINDOWS } else { os::MACOS });
}
