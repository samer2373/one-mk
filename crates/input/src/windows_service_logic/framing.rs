use crate::typing::Text;
use crate::{Event, Inject};
use one_mk_proto::Pos;

pub const IPC_VERSION: u16 = 2;
pub const MAX_FRAME: usize = 192;
pub const MAX_BODY: usize = MAX_FRAME - 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicError {
    Truncated, Oversized, Version, Invalid, Trailing, Unauthorized, Expired,
    Exhausted, State, Clock, Path,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Challenge { serial: u64, nonce: [u8; 32] },
    Proof { nonce: [u8; 32], signature: [u8; 32] },
    Accepted([u8; 32]),
    Input(Inject),
    ReleaseAll,
    LocalInput,
    ScreenLocked(bool),
}

impl Message {
    pub fn from_event(event: &Event) -> Result<Self, LogicError> {
        match event {
            Event::LocalInput => Ok(Self::LocalInput),
            Event::ScreenLocked(locked) => Ok(Self::ScreenLocked(*locked)),
            _ => Err(LogicError::Invalid),
        }
    }

    /// The message for a peer from before character keys had a kind of their own (it reads kind 24 as unknown and
    /// closes the channel): a character key goes as the plain key it was then, which is how the Windows side
    /// injects it anyway (by key code). Everything else is as it is.
    pub fn for_legacy_peer(&self) -> Self {
        match self {
            Self::Input(Inject::CharacterKey { code, down, .. }) => Self::Input(Inject::Key { code: *code, down: *down }),
            other => other.clone(),
        }
    }
}

/// The same rules as the network encoding of a character key (`one_mk_proto`): a known key code and a printable
/// character.
fn character_key_valid(code: u32, character: char) -> bool {
    one_mk_proto::typing::valid_key_code(code) && one_mk_proto::typing::printable(character)
}

/// Fixed storage, including the four-byte little-endian body length.
pub struct Frame {
    bytes: [u8; MAX_FRAME],
    len: usize,
}

impl Frame {
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
}

/// Validate the prefix before a transport reserves storage or reads the body.
pub fn body_length(prefix: [u8; 4]) -> Result<usize, LogicError> {
    let length = u32::from_le_bytes(prefix) as usize;
    if length > MAX_BODY { return Err(LogicError::Oversized); }
    if length < 4 { return Err(LogicError::Invalid); }
    Ok(length)
}

pub fn encode(message: &Message) -> Result<Frame, LogicError> {
    let mut frame = Frame { bytes: [0; MAX_FRAME], len: 8 };
    frame.bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
    let mut payload = [0u8; 64];
    let (kind, length) = match message {
        Message::Challenge { serial, nonce } => {
            payload[..8].copy_from_slice(&serial.to_le_bytes());
            payload[8..40].copy_from_slice(nonce);
            (1, 40)
        }
        Message::Proof { nonce, signature } => {
            payload[..32].copy_from_slice(nonce);
            payload[32..].copy_from_slice(signature);
            (2, 64)
        }
        Message::Accepted(tag) => {
            payload[..32].copy_from_slice(tag);
            (3, 32)
        }
        Message::ReleaseAll => (4, 0),
        Message::LocalInput => (5, 0),
        Message::ScreenLocked(locked) => { payload[0] = u8::from(*locked); (6, 1) }
        Message::Input(input) => match input {
            Inject::MoveTo(pos) => {
                finite(pos.x, pos.y)?;
                payload[..8].copy_from_slice(&pos.x.to_le_bytes());
                payload[8..16].copy_from_slice(&pos.y.to_le_bytes());
                (16, 16)
            }
            Inject::MoveBy { dx, dy } | Inject::ScrollDiscrete { dx, dy } => {
                payload[..4].copy_from_slice(&dx.to_le_bytes());
                payload[4..8].copy_from_slice(&dy.to_le_bytes());
                (if matches!(input, Inject::MoveBy { .. }) { 17 } else { 20 }, 8)
            }
            Inject::Button { button, down } | Inject::Key { code: button, down } => {
                payload[..4].copy_from_slice(&button.to_le_bytes());
                payload[4] = u8::from(*down);
                (if matches!(input, Inject::Button { .. }) { 18 } else { 22 }, 5)
            }
            // Its own kind: kind 22 is a plain key, and an older peer reads 24 as unknown and refuses it (see
            // `Message::for_legacy_peer`).
            Inject::CharacterKey { code, down, character } => {
                if !character_key_valid(*code, *character) { return Err(LogicError::Invalid); }
                payload[..4].copy_from_slice(&code.to_le_bytes());
                payload[4] = u8::from(*down);
                payload[5..9].copy_from_slice(&u32::from(*character).to_le_bytes());
                (24, 9)
            }
            Inject::Text { text } => {
                let text = text.as_str().as_bytes();
                payload[0] = text.len() as u8;
                payload[1..=text.len()].copy_from_slice(text);
                (25, 1 + text.len())
            }
            Inject::Scroll { dx, dy } => {
                finite(*dx, *dy)?;
                payload[..8].copy_from_slice(&dx.to_le_bytes());
                payload[8..16].copy_from_slice(&dy.to_le_bytes());
                (19, 16)
            }
            Inject::ScrollStop => (21, 0),
            Inject::Controlled(controlled) => { payload[0] = u8::from(*controlled); (23, 1) }
        },
    };
    frame.bytes[6] = kind;
    frame.bytes[8..8 + length].copy_from_slice(&payload[..length]);
    frame.len += length;
    frame.bytes[..4].copy_from_slice(&((frame.len - 4) as u32).to_le_bytes());
    Ok(frame)
}

fn finite(first: f64, second: f64) -> Result<(), LogicError> {
    if first.is_finite() && second.is_finite() { Ok(()) } else { Err(LogicError::Invalid) }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], LogicError> {
        let bytes = self.0.get(..N).ok_or(LogicError::Truncated)?;
        let mut result = [0; N];
        result.copy_from_slice(bytes);
        self.0 = &self.0[N..];
        Ok(result)
    }

    fn boolean(&mut self) -> Result<bool, LogicError> {
        match self.array::<1>()? { [0] => Ok(false), [1] => Ok(true), _ => Err(LogicError::Invalid) }
    }
}

/// Exactly one complete frame; no allocation and no ignored suffix.
pub fn decode(bytes: &[u8]) -> Result<Message, LogicError> {
    let mut reader = Reader(bytes);
    let length = body_length(reader.array()?)?;
    if bytes.len() < length + 4 { return Err(LogicError::Truncated); }
    if bytes.len() > length + 4 { return Err(LogicError::Trailing); }
    if u16::from_le_bytes(reader.array()?) != IPC_VERSION { return Err(LogicError::Version); }
    let [kind, reserved] = reader.array()?;
    if reserved != 0 { return Err(LogicError::Invalid); }
    let message = match kind {
        1 => Message::Challenge { serial: u64::from_le_bytes(reader.array()?), nonce: reader.array()? },
        2 => Message::Proof { nonce: reader.array()?, signature: reader.array()? },
        3 => Message::Accepted(reader.array()?),
        4 => Message::ReleaseAll,
        5 => Message::LocalInput,
        6 => Message::ScreenLocked(reader.boolean()?),
        16 | 19 => {
            let first = f64::from_le_bytes(reader.array()?);
            let second = f64::from_le_bytes(reader.array()?);
            finite(first, second)?;
            Message::Input(if kind == 16 { Inject::MoveTo(Pos { x: first, y: second }) }
                else { Inject::Scroll { dx: first, dy: second } })
        }
        17 | 20 => {
            let dx = i32::from_le_bytes(reader.array()?);
            let dy = i32::from_le_bytes(reader.array()?);
            Message::Input(if kind == 17 { Inject::MoveBy { dx, dy } } else { Inject::ScrollDiscrete { dx, dy } })
        }
        18 | 22 => {
            let code = u32::from_le_bytes(reader.array()?);
            let down = reader.boolean()?;
            Message::Input(if kind == 18 { Inject::Button { button: code, down } } else { Inject::Key { code, down } })
        }
        24 => {
            let code = u32::from_le_bytes(reader.array()?);
            let down = reader.boolean()?;
            let character = char::from_u32(u32::from_le_bytes(reader.array()?)).ok_or(LogicError::Invalid)?;
            if !character_key_valid(code, character) { return Err(LogicError::Invalid); }
            Message::Input(Inject::CharacterKey { code, down, character })
        }
        25 => {
            let [length] = reader.array()?;
            let length = usize::from(length);
            let bytes = reader.0.get(..length).ok_or(LogicError::Truncated)?;
            let text = std::str::from_utf8(bytes).map_err(|_| LogicError::Invalid)?;
            let text = Text::new(text).ok_or(LogicError::Invalid)?;
            reader.0 = &reader.0[length..];
            Message::Input(Inject::Text { text })
        }
        21 => Message::Input(Inject::ScrollStop),
        23 => Message::Input(Inject::Controlled(reader.boolean()?)),
        _ => return Err(LogicError::Invalid),
    };
    if !reader.0.is_empty() { return Err(LogicError::Trailing); }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages() -> Vec<Message> {
        vec![
            Message::Challenge { serial: u64::MAX, nonce: [42; 32] }, Message::Proof { nonce: [2; 32], signature: [3; 32] },
            Message::Accepted([4; 32]), Message::ReleaseAll, Message::LocalInput,
            Message::ScreenLocked(false), Message::ScreenLocked(true),
            Message::Input(Inject::MoveTo(Pos { x: -2.5, y: 100.125 })),
            Message::Input(Inject::MoveBy { dx: i32::MIN, dy: i32::MAX }),
            Message::Input(Inject::Scroll { dx: -0.5, dy: 2.0 }),
            Message::Input(Inject::ScrollDiscrete { dx: -1, dy: 1 }),
            Message::Input(Inject::ScrollStop), Message::Input(Inject::Controlled(false)),
            Message::Input(Inject::Controlled(true)),
        ].into_iter().chain([false, true].into_iter().flat_map(|down| [
            Message::Input(Inject::Key { code: u32::MAX, down }),
            Message::Input(Inject::Button { button: 272, down }),
        ])).collect()
    }

    #[test]
    fn every_message_roundtrips_and_every_truncation_fails() {
        for message in messages() {
            let frame = encode(&message).unwrap();
            assert_eq!(decode(frame.as_bytes()), Ok(message));
            for end in 0..frame.len { assert!(decode(&frame.as_bytes()[..end]).is_err()); }
            let mut bytes = frame.as_bytes().to_vec();
            bytes.push(0);
            assert_eq!(decode(&bytes), Err(LogicError::Trailing));
            let length = (bytes.len() - 4) as u32;
            bytes[..4].copy_from_slice(&length.to_le_bytes());
            assert_eq!(decode(&bytes), Err(LogicError::Trailing));
        }
    }

    /// Every `Inject` the helper may be asked to perform crosses the pipe as what it is. (The match is
    /// exhaustive on purpose: a new variant must be put here, and given a wire form, before this compiles.)
    #[test]
    fn every_inject_variant_crosses_the_pipe_unchanged() {
        let samples = [
            Inject::MoveTo(Pos { x: 1.5, y: -3.0 }), Inject::MoveBy { dx: -4, dy: 5 }, Inject::Button { button: 272, down: true },
            Inject::Scroll { dx: 0.25, dy: -1.0 }, Inject::ScrollDiscrete { dx: 1, dy: -1 }, Inject::ScrollStop,
            Inject::Key { code: 30, down: true }, Inject::Key { code: 30, down: false },
            Inject::Text { text: "@".into() }, Inject::Text { text: "é".into() }, Inject::Text { text: "e\u{301}".into() },
            Inject::CharacterKey { code: 30, down: true, character: 'ß' },
            Inject::CharacterKey { code: 30, down: false, character: 'ß' },
            Inject::CharacterKey { code: 21, down: true, character: '😀' },
            Inject::CharacterKey { code: 767, down: false, character: '界' },
            Inject::Controlled(true),
        ];
        for sample in samples {
            match sample {
                Inject::MoveTo(_) | Inject::MoveBy { .. } | Inject::Button { .. } | Inject::Scroll { .. }
                | Inject::ScrollDiscrete { .. } | Inject::ScrollStop | Inject::Key { .. } | Inject::Text { .. }
                | Inject::CharacterKey { .. } | Inject::Controlled(_) => {}
            }
            let frame = encode(&Message::Input(sample)).unwrap_or_else(|error| panic!("{sample:?}: {error:?}"));
            assert_eq!(decode(frame.as_bytes()), Ok(Message::Input(sample)), "{sample:?}");
        }
    }

    #[test]
    fn a_character_key_is_not_a_plain_key_on_the_wire() {
        let character = Message::Input(Inject::CharacterKey { code: 30, down: true, character: 'ß' });
        let plain = Message::Input(Inject::Key { code: 30, down: true });
        let (character_frame, plain_frame) = (encode(&character).unwrap(), encode(&plain).unwrap());
        assert_ne!(character_frame.as_bytes(), plain_frame.as_bytes());
        assert_eq!(decode(character_frame.as_bytes()), Ok(character.clone()));
        assert_ne!(decode(character_frame.as_bytes()), Ok(plain.clone()), "the character was lost");
        // A peer from before character keys had a kind of their own gets the plain key it always got, and
        // frames it sent are still read as they were.
        assert_eq!(character.for_legacy_peer(), plain);
        assert_eq!(plain.for_legacy_peer(), plain);
        assert_eq!(Message::ReleaseAll.for_legacy_peer(), Message::ReleaseAll);
        assert_eq!(Message::Input(Inject::Text { text: "@".into() }).for_legacy_peer(), Message::Input(Inject::Text { text: "@".into() }));
        let old_frame = [9, 0, 0, 0, IPC_VERSION as u8, 0, 22, 0, 30, 0, 0, 0, 1];
        assert_eq!(&old_frame[..], plain_frame.as_bytes());
        assert_eq!(decode(&old_frame), Ok(plain));
    }

    #[test]
    fn character_keys_and_text_are_validated_like_the_network_encoding() {
        let ok = |code, character| Message::Input(Inject::CharacterKey { code, down: true, character });
        for (code, character) in [(0, 'a'), (768, 'a'), (u32::MAX, 'a'), (30, '\n'), (30, '\u{0}'), (30, '\u{7f}'), (30, '\u{e000}')] {
            assert_eq!(encode(&ok(code, character)).err(), Some(LogicError::Invalid), "{code} {character:?}");
        }
        let frame = encode(&ok(30, 'a')).unwrap();
        // Frame: 4 length, 2 version, kind, reserved, then code(4) down(1) character(4).
        for (offset, value, what) in [(8, 0u32, "code"), (8, 768, "code"), (13, 0xD800, "surrogate"), (13, 0x11_0000, "beyond Unicode"),
            (13, 10, "control"), (13, 0xE000, "private use"), (13, 0x200B, "format")] {
            let mut bytes = frame.as_bytes().to_vec();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(decode(&bytes), Err(LogicError::Invalid), "{what}");
        }
        let mut bytes = frame.as_bytes().to_vec();
        bytes[12] = 2;
        assert_eq!(decode(&bytes), Err(LogicError::Invalid), "direction");

        // Text: one length byte and as many bytes, one printable grapheme.
        let text = encode(&Message::Input(Inject::Text { text: "é".into() })).unwrap();
        assert_eq!(&text.as_bytes()[6..], &[25, 0, 2, 0xC3, 0xA9]);
        let body = |bytes: &[u8]| {
            let mut frame = vec![0; 4];
            frame.extend_from_slice(&IPC_VERSION.to_le_bytes());
            frame.extend_from_slice(&[25, 0]);
            frame.extend_from_slice(bytes);
            let length = (frame.len() - 4) as u32;
            frame[..4].copy_from_slice(&length.to_le_bytes());
            decode(&frame)
        };
        assert_eq!(body(&[1, b'a']), Ok(Message::Input(Inject::Text { text: "a".into() })));
        assert_eq!(body(&[0]), Err(LogicError::Invalid), "empty");
        assert_eq!(body(&[9, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i']), Err(LogicError::Invalid), "too long");
        assert_eq!(body(&[2, b'a']), Err(LogicError::Truncated), "length beyond the frame");
        assert_eq!(body(&[1, b'a', b'b']), Err(LogicError::Trailing));
        assert_eq!(body(&[2, b'a', b'b']), Err(LogicError::Invalid), "two graphemes");
        assert_eq!(body(&[1, 0xFF]), Err(LogicError::Invalid), "not UTF-8");
        assert_eq!(body(&[1, b'\n']), Err(LogicError::Invalid), "not printable");
    }

    #[test]
    fn lengths_versions_flags_kinds_and_booleans_are_strict() {
        assert_eq!(body_length((MAX_BODY as u32).to_le_bytes()), Ok(MAX_BODY));
        for length in [MAX_BODY as u32 + 1, u32::MAX] {
            assert_eq!(decode(&length.to_le_bytes()), Err(LogicError::Oversized));
        }
        for length in 0u32..4 { assert_eq!(body_length(length.to_le_bytes()), Err(LogicError::Invalid)); }
        let original = encode(&Message::ScreenLocked(true)).unwrap();
        for (index, value, error) in [(4, 3, LogicError::Version), (6, 255, LogicError::Invalid),
            (7, 1, LogicError::Invalid), (8, 2, LogicError::Invalid)] {
            let mut bytes = original.as_bytes().to_vec();
            bytes[index] = value;
            assert_eq!(decode(&bytes), Err(error));
        }
    }

    #[test]
    fn floats_and_event_allowlist() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(encode(&Message::Input(Inject::Scroll { dx: value, dy: 0.0 })).is_err());
            assert!(encode(&Message::Input(Inject::MoveTo(Pos { x: 0.0, y: value }))).is_err());
            let mut frame = encode(&Message::Input(Inject::MoveTo(Pos { x: 0.0, y: 0.0 }))).unwrap();
            frame.bytes[8..16].copy_from_slice(&value.to_le_bytes());
            assert_eq!(decode(frame.as_bytes()), Err(LogicError::Invalid));
        }
        assert_eq!(Message::from_event(&Event::LocalInput), Ok(Message::LocalInput));
        assert_eq!(Message::from_event(&Event::ScreenLocked(true)), Ok(Message::ScreenLocked(true)));
        assert_eq!(Message::from_event(&Event::Error("untrusted".into())), Err(LogicError::Invalid));
    }

    #[test]
    fn hostile_bytes_never_panic() {
        let mut bytes = [0; MAX_FRAME + 1];
        for length in 0..=MAX_FRAME + 1 {
            for value in 0..=255u8 {
                bytes.fill(value);
                let _ = decode(&bytes[..length]);
                if length >= 4 {
                    bytes[..4].copy_from_slice(&((length - 4) as u32).to_le_bytes());
                    let _ = decode(&bytes[..length]);
                }
            }
        }
    }
}
