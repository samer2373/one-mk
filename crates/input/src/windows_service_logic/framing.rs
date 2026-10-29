use crate::{Event, Inject};
use one_mk_proto::Pos;

pub const IPC_VERSION: u16 = 1;
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
    Proof([u8; 32]),
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
    let mut payload = [0u8; 40];
    let (kind, length) = match message {
        Message::Challenge { serial, nonce } => {
            payload[..8].copy_from_slice(&serial.to_le_bytes());
            payload[8..].copy_from_slice(nonce);
            (1, 40)
        }
        Message::Proof(tag) | Message::Accepted(tag) => {
            payload[..32].copy_from_slice(tag);
            (if matches!(message, Message::Proof(_)) { 2 } else { 3 }, 32)
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
        2 => Message::Proof(reader.array()?),
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
            Message::Challenge { serial: u64::MAX, nonce: [42; 32] }, Message::Proof([3; 32]),
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

    #[test]
    fn lengths_versions_flags_kinds_and_booleans_are_strict() {
        assert_eq!(body_length((MAX_BODY as u32).to_le_bytes()), Ok(MAX_BODY));
        for length in [MAX_BODY as u32 + 1, u32::MAX] {
            assert_eq!(decode(&length.to_le_bytes()), Err(LogicError::Oversized));
        }
        for length in 0u32..4 { assert_eq!(body_length(length.to_le_bytes()), Err(LogicError::Invalid)); }
        let original = encode(&Message::ScreenLocked(true)).unwrap();
        for (index, value, error) in [(4, 2, LogicError::Version), (6, 255, LogicError::Invalid),
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
