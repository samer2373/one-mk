//! Compact little-endian binary encoding. Positions are fixed-point with
//! 1/256 px precision.

use crate::{Pos, Rect};
use std::fmt;

/// First byte of every QUIC stream, saying what the stream carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamKind {
    Control = 1,
    Pairing = 2,
}

impl StreamKind {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Control),
            2 => Some(Self::Pairing),
            _ => None,
        }
    }
}

/// Unreliable, latency-critical messages. Each one is self-contained, so
/// loss or reordering never corrupts state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Datagram {
    /// Absolute pointer position on the receiver's desktop.
    Motion { epoch: u16, seq: u32, pos: Pos },
    /// Sent while the pointer is remote and idle, so Wi-Fi radios stay out
    /// of power-save (which otherwise adds 50-200 ms stalls).
    Keepalive,
    /// Latency measurement; the receiver echoes it back.
    Probe { id: u32, sent_ns: u64 },
    ProbeEcho { id: u32, sent_ns: u64 },
}

impl Datagram {
    pub const MAX_LEN: usize = 15;

    pub fn encode(&self) -> ([u8; Self::MAX_LEN], usize) {
        let mut w = Writer::fixed();
        match *self {
            Datagram::Motion { epoch, seq, pos } => {
                w.u8(1);
                w.u16(epoch);
                w.u32(seq);
                w.pos(pos);
            }
            Datagram::Keepalive => w.u8(2),
            Datagram::Probe { id, sent_ns } => {
                w.u8(3);
                w.u32(id);
                w.u64(sent_ns);
            }
            Datagram::ProbeEcho { id, sent_ns } => {
                w.u8(4);
                w.u32(id);
                w.u64(sent_ns);
            }
        }
        let n = w.buf.len();
        let mut out = [0u8; Self::MAX_LEN];
        out[..n].copy_from_slice(&w.buf);
        (out, n)
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(b);
        let d = match r.u8()? {
            1 => Datagram::Motion { epoch: r.u16()?, seq: r.u32()?, pos: r.pos()? },
            2 => Datagram::Keepalive,
            3 => Datagram::Probe { id: r.u32()?, sent_ns: r.u64()? },
            4 => Datagram::ProbeEcho { id: r.u32()?, sent_ns: r.u64()? },
            t => return Err(DecodeError::UnknownTag(t)),
        };
        Ok(d)
    }
}

/// Reliable, ordered messages on the control stream.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Hello { version: u16, name: String, screens: Vec<Rect> },
    /// The sender's monitors changed.
    Screens(Vec<Rect>),
    /// The pointer enters the receiver's desktop at `pos`.
    Enter { epoch: u16, pos: Pos },
    /// The pointer left; the receiver releases anything still held.
    Leave,
    Key { code: u32, down: bool },
    /// Carries the pointer position at the time of the click, so a click
    /// can never land where the pointer was a few packets ago.
    Button { button: u32, down: bool, seq: u32, pos: Pos },
    /// Smooth scroll in logical pixels.
    Scroll { dx: f32, dy: f32 },
    /// Wheel clicks in 1/120ths of a detent (high-resolution wheels).
    ScrollDiscrete { dx: i32, dy: i32 },
    ScrollStop,
}

impl Control {
    /// Upper bound for a control frame body.
    pub const MAX_LEN: usize = 16 * 1024;

    /// Appends a length-prefixed frame.
    pub fn encode_frame(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0, 0]);
        let mut w = Writer { buf: std::mem::take(out) };
        self.encode_body(&mut w);
        *out = w.buf;
        let len = u16::try_from(out.len() - start - 2).expect("control frame too large");
        out[start..start + 2].copy_from_slice(&len.to_le_bytes());
    }

    fn encode_body(&self, w: &mut Writer) {
        match self {
            Control::Hello { version, name, screens } => {
                w.u8(1);
                w.u16(*version);
                w.str(name);
                w.rects(screens);
            }
            Control::Screens(screens) => {
                w.u8(2);
                w.rects(screens);
            }
            Control::Enter { epoch, pos } => {
                w.u8(3);
                w.u16(*epoch);
                w.pos(*pos);
            }
            Control::Leave => w.u8(4),
            Control::Key { code, down } => {
                w.u8(5);
                w.u32(*code);
                w.u8(u8::from(*down));
            }
            Control::Button { button, down, seq, pos } => {
                w.u8(6);
                w.u32(*button);
                w.u8(u8::from(*down));
                w.u32(*seq);
                w.pos(*pos);
            }
            Control::Scroll { dx, dy } => {
                w.u8(7);
                w.u32(dx.to_bits());
                w.u32(dy.to_bits());
            }
            Control::ScrollDiscrete { dx, dy } => {
                w.u8(8);
                w.i32(*dx);
                w.i32(*dy);
            }
            Control::ScrollStop => w.u8(9),
        }
    }

    /// Decodes a frame body (without the length prefix).
    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(b);
        let c = match r.u8()? {
            1 => Control::Hello { version: r.u16()?, name: r.str()?, screens: r.rects()? },
            2 => Control::Screens(r.rects()?),
            3 => Control::Enter { epoch: r.u16()?, pos: r.pos()? },
            4 => Control::Leave,
            5 => Control::Key { code: r.u32()?, down: r.u8()? != 0 },
            6 => Control::Button {
                button: r.u32()?,
                down: r.u8()? != 0,
                seq: r.u32()?,
                pos: r.pos()?,
            },
            7 => Control::Scroll { dx: f32::from_bits(r.u32()?), dy: f32::from_bits(r.u32()?) },
            8 => Control::ScrollDiscrete { dx: r.i32()?, dy: r.i32()? },
            9 => Control::ScrollStop,
            t => return Err(DecodeError::UnknownTag(t)),
        };
        Ok(c)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    UnknownTag(u8),
    BadString,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("message truncated"),
            DecodeError::UnknownTag(t) => write!(f, "unknown message tag {t}"),
            DecodeError::BadString => f.write_str("invalid UTF-8 string"),
        }
    }
}

impl std::error::Error for DecodeError {}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn fixed() -> Self {
        Self { buf: Vec::with_capacity(Datagram::MAX_LEN) }
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn pos(&mut self, p: Pos) {
        self.i32(to_fixed(p.x));
        self.i32(to_fixed(p.y));
    }
    fn str(&mut self, s: &str) {
        let s = &s[..s.floor_char_boundary(255)];
        self.u8(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
    }
    fn rects(&mut self, rects: &[Rect]) {
        let n = rects.len().min(64);
        self.u8(n as u8);
        for r in &rects[..n] {
            self.i32(r.x);
            self.i32(r.y);
            self.i32(r.w);
            self.i32(r.h);
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(DecodeError::Truncated)?;
        self.0 = rest;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn pos(&mut self) -> Result<Pos, DecodeError> {
        Ok(Pos::new(from_fixed(self.i32()?), from_fixed(self.i32()?)))
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let n = self.u8()? as usize;
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        String::from_utf8(s.to_vec()).map_err(|_| DecodeError::BadString)
    }
    fn rects(&mut self) -> Result<Vec<Rect>, DecodeError> {
        let n = self.u8()?;
        (0..n).map(|_| Ok(Rect::new(self.i32()?, self.i32()?, self.i32()?, self.i32()?))).collect()
    }
}

fn to_fixed(v: f64) -> i32 {
    (v * 256.0).round() as i32
}

fn from_fixed(v: i32) -> f64 {
    f64::from(v) / 256.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_round_trip() {
        for d in [
            Datagram::Motion { epoch: 3, seq: 99, pos: Pos::new(1919.5, -20.25) },
            Datagram::Keepalive,
            Datagram::Probe { id: 1, sent_ns: u64::MAX },
            Datagram::ProbeEcho { id: 2, sent_ns: 5 },
        ] {
            let (buf, n) = d.encode();
            assert_eq!(Datagram::decode(&buf[..n]).unwrap(), d);
        }
        assert_eq!(Datagram::Motion { epoch: 0, seq: 0, pos: Pos::default() }.encode().1, 15);
    }

    #[test]
    fn control_round_trip() {
        let msgs = [
            Control::Hello {
                version: 1,
                name: "desk".into(),
                screens: vec![Rect::new(0, 0, 1920, 1080), Rect::new(-1440, 0, 1440, 900)],
            },
            Control::Screens(vec![]),
            Control::Enter { epoch: 9, pos: Pos::new(0.0, 540.0) },
            Control::Leave,
            Control::Key { code: 30, down: true },
            Control::Button { button: 0x110, down: false, seq: 7, pos: Pos::new(3.5, 4.0) },
            Control::Scroll { dx: 0.0, dy: -12.5 },
            Control::ScrollDiscrete { dx: 0, dy: -120 },
            Control::ScrollStop,
        ];
        let mut buf = Vec::new();
        for m in &msgs {
            m.encode_frame(&mut buf);
        }
        let mut rest = &buf[..];
        for m in &msgs {
            let len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
            assert_eq!(&Control::decode(&rest[2..2 + len]).unwrap(), m);
            rest = &rest[2 + len..];
        }
        assert!(rest.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Datagram::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(Datagram::decode(&[1, 0]), Err(DecodeError::Truncated));
        assert_eq!(Control::decode(&[200]), Err(DecodeError::UnknownTag(200)));
    }
}
