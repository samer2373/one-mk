//! Compact little-endian binary encoding. Positions are fixed-point with
//! 1/256 px precision.

use crate::{PROTOCOL_VERSION, Pos, Rect, Side};
use std::fmt;

/// First byte of every QUIC stream, saying what the stream carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamKind {
    Control = 1,
    Pairing = 2,
    /// One clipboard snapshot: a [`ClipHeader`](crate::ClipHeader) frame,
    /// then the payloads, then end of stream.
    Clipboard = 3,
    /// A sequence of files; see [`FileMsg`](crate::FileMsg).
    File = 4,
    /// A paired device measuring latency: the peer echoes its `Probe`
    /// datagrams on this connection. Never a link, never replaces one.
    Probe = 5,
}

impl StreamKind {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Control),
            2 => Some(Self::Pairing),
            3 => Some(Self::Clipboard),
            4 => Some(Self::File),
            5 => Some(Self::Probe),
            _ => None,
        }
    }
}

/// Unreliable, latency-critical messages. Each one is self-contained, so
/// loss or reordering never corrupts state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Datagram {
    /// Absolute pointer position on the receiver's desktop. While the
    /// pointer is remote and idle, the latest one is resent (same `seq`) as
    /// a keepalive: it keeps Wi-Fi radios out of power save (which adds
    /// 50-200 ms stalls) and tells the receiver the controller is alive.
    Motion { epoch: u16, seq: u32, pos: Pos },
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
            3 => Datagram::Probe { id: r.u32()?, sent_ns: r.u64()? },
            4 => Datagram::ProbeEcho { id: r.u32()?, sent_ns: r.u64()? },
            t => return Err(DecodeError::UnknownTag(t)),
        };
        Ok(d)
    }
}

/// Operating system codes for [`Control::Hello`].
pub mod os {
    pub const UNKNOWN: u8 = 0;
    pub const LINUX: u8 = 1;
    pub const WINDOWS: u8 = 2;
    pub const MACOS: u8 = 3;

    /// This machine's OS.
    pub const fn local() -> u8 {
        if cfg!(target_os = "linux") {
            LINUX
        } else if cfg!(windows) {
            WINDOWS
        } else if cfg!(target_os = "macos") {
            MACOS
        } else {
            UNKNOWN
        }
    }
}

/// Reliable, ordered messages on the control stream.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// First message on a control stream. `nonce` is random per connection
    /// and breaks ties between duplicate connections; `os` is one of [`os`].
    Hello { version: u16, name: String, screens: Vec<Rect>, nonce: u64, os: u8 },
    /// The sender's monitors changed.
    Screens(Vec<Rect>),
    /// The pointer enters the receiver's desktop at `pos`. The receiver
    /// answers `EnterAck` or, if another machine controls it, `Busy`.
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
    /// The receiver accepted `Enter { epoch }`; input may follow.
    EnterAck { epoch: u16 },
    /// The receiver is controlled by another machine and refused
    /// `Enter { epoch }`; the sender takes its pointer back.
    Busy { epoch: u16 },
    /// The receiver ended visit `epoch` (someone used its own mouse or
    /// keyboard, or the sender went silent); the sender takes its pointer back.
    Revoked { epoch: u16 },
    /// Layout sync: the sender sits on `side_of_you` of the receiver. The
    /// receiver adopts it if `version` is newer than the one it has stored.
    /// `offset` is where the sender put the receiver's (0,0) in its own
    /// pixels; it travels as optional trailing bytes, which older peers
    /// neither send nor read.
    Layout { side_of_you: Side, version: u64, offset: Option<(i32, i32)> },
}

impl Control {
    /// Upper bound for a control frame body.
    pub const MAX_LEN: usize = 16 * 1024;

    /// Appends a length-prefixed frame.
    pub fn encode_frame(&self, out: &mut Vec<u8>) {
        frame(out, |w| self.encode_body(w));
    }

    fn encode_body(&self, w: &mut Writer) {
        match self {
            Control::Hello { version, name, screens, nonce, os } => {
                w.u8(1);
                w.u16(*version);
                w.str(name);
                w.rects(screens);
                w.u64(*nonce);
                w.u8(*os);
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
            Control::EnterAck { epoch } => {
                w.u8(10);
                w.u16(*epoch);
            }
            Control::Busy { epoch } => {
                w.u8(11);
                w.u16(*epoch);
            }
            Control::Revoked { epoch } => {
                w.u8(12);
                w.u16(*epoch);
            }
            Control::Layout { side_of_you, version, offset } => {
                w.u8(13);
                w.u8(Side::ALL.iter().position(|s| s == side_of_you).unwrap_or(0) as u8);
                w.u64(*version);
                if let Some((x, y)) = offset {
                    w.i32(*x);
                    w.i32(*y);
                }
            }
        }
    }

    /// Decodes a frame body (without the length prefix). Everything a peer
    /// sends is validated here, so nothing downstream can overflow or see
    /// NaN. A `Hello` from another protocol version is
    /// [`DecodeError::Version`], since the rest may be laid out differently.
    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(b);
        let c = match r.u8()? {
            1 => match r.u16()? {
                PROTOCOL_VERSION => Control::Hello {
                    version: PROTOCOL_VERSION,
                    name: r.str()?,
                    screens: r.rects()?,
                    nonce: r.u64()?,
                    os: r.u8()?,
                },
                v => return Err(DecodeError::Version(v)),
            },
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
            7 => Control::Scroll { dx: r.f32()?, dy: r.f32()? },
            8 => Control::ScrollDiscrete { dx: r.i32()?, dy: r.i32()? },
            9 => Control::ScrollStop,
            10 => Control::EnterAck { epoch: r.u16()? },
            11 => Control::Busy { epoch: r.u16()? },
            12 => Control::Revoked { epoch: r.u16()? },
            13 => Control::Layout {
                side_of_you: *Side::ALL.get(usize::from(r.u8()?)).ok_or(DecodeError::Invalid("side"))?,
                version: r.u64()?,
                offset: match r.0.len() {
                    0..8 => None,
                    _ => {
                        let o = (r.i32()?, r.i32()?);
                        // Far beyond any desktop: it would only overflow the geometry.
                        if o.0.unsigned_abs().max(o.1.unsigned_abs()) > MAX_OFFSET {
                            return Err(DecodeError::Invalid("offset"));
                        }
                        Some(o)
                    }
                },
            },
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
    /// Well-formed but out of range (bad rectangle, NaN, ...).
    Invalid(&'static str),
    /// A `Hello` from another protocol version.
    Version(u16),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("message truncated"),
            DecodeError::UnknownTag(t) => write!(f, "unknown message tag {t}"),
            DecodeError::BadString => f.write_str("invalid UTF-8 string"),
            DecodeError::Invalid(what) => write!(f, "invalid {what}"),
            DecodeError::Version(v) => write!(f, "protocol version {v} (we speak {PROTOCOL_VERSION})"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Appends a frame: a u16 little-endian body length, then the body.
pub(crate) fn frame(out: &mut Vec<u8>, body: impl FnOnce(&mut Writer)) {
    let start = out.len();
    out.extend_from_slice(&[0, 0]);
    let mut w = Writer { buf: std::mem::take(out) };
    body(&mut w);
    *out = w.buf;
    let len = u16::try_from(out.len() - start - 2).expect("frame too large");
    out[start..start + 2].copy_from_slice(&len.to_le_bytes());
}

pub(crate) struct Writer {
    pub(crate) buf: Vec<u8>,
}

impl Writer {
    fn fixed() -> Self {
        Self { buf: Vec::with_capacity(Datagram::MAX_LEN) }
    }
    pub(crate) fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub(crate) fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u64(&mut self, v: u64) {
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
    /// A string of up to `max` bytes (truncated at a character boundary).
    pub(crate) fn long_str(&mut self, s: &str, max: usize) {
        let s = &s[..s.floor_char_boundary(max.min(u16::MAX as usize))];
        self.u16(s.len() as u16);
        self.buf.extend_from_slice(s.as_bytes());
    }
    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn rects(&mut self, rects: &[Rect]) {
        let n = rects.len().min(MAX_SCREENS);
        self.u8(n as u8);
        for r in &rects[..n] {
            self.i32(r.x);
            self.i32(r.y);
            self.i32(r.w);
            self.i32(r.h);
        }
    }
}

pub(crate) struct Reader<'a>(pub(crate) &'a [u8]);

impl Reader<'_> {
    pub(crate) fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(DecodeError::Truncated)?;
        self.0 = rest;
        Ok(*head)
    }
    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take::<1>()?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    pub(crate) fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    /// Fixed-point, so always finite.
    fn pos(&mut self) -> Result<Pos, DecodeError> {
        Ok(Pos::new(from_fixed(self.i32()?), from_fixed(self.i32()?)))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        let v = f32::from_bits(self.u32()?);
        if !v.is_finite() {
            return Err(DecodeError::Invalid("number"));
        }
        Ok(v)
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
    pub(crate) fn long_str(&mut self) -> Result<String, DecodeError> {
        let n = self.u16()? as usize;
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        String::from_utf8(s.to_vec()).map_err(|_| DecodeError::BadString)
    }
    /// Monitors: positive sizes, at most [`MAX_SCREENS`], and every edge
    /// and the overall span representable in `i32`.
    fn rects(&mut self) -> Result<Vec<Rect>, DecodeError> {
        let n = usize::from(self.u8()?);
        if n > MAX_SCREENS {
            return Err(DecodeError::Invalid("screen count"));
        }
        let rects = (0..n)
            .map(|_| Ok(Rect::new(self.i32()?, self.i32()?, self.i32()?, self.i32()?)))
            .collect::<Result<Vec<_>, _>>()?;
        let bad = |r: &Rect| r.w <= 0 || r.h <= 0 || r.x.checked_add(r.w).is_none() || r.y.checked_add(r.h).is_none();
        if rects.iter().any(bad) {
            return Err(DecodeError::Invalid("screen"));
        }
        if let (Some(x), Some(y)) = (rects.iter().map(|r| r.x).min(), rects.iter().map(|r| r.y).min()) {
            let right = rects.iter().map(Rect::right).max().unwrap_or(x);
            let bottom = rects.iter().map(Rect::bottom).max().unwrap_or(y);
            if right.checked_sub(x).is_none() || bottom.checked_sub(y).is_none() {
                return Err(DecodeError::Invalid("screen layout"));
            }
        }
        Ok(rects)
    }
}

/// Most monitors a desktop may have on the wire.
pub const MAX_SCREENS: usize = 64;

/// Largest offset (either axis) a `Layout` may carry, in pixels.
const MAX_OFFSET: u32 = 1 << 24;

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
                version: PROTOCOL_VERSION,
                name: "desk".into(),
                screens: vec![Rect::new(0, 0, 1920, 1080), Rect::new(-1440, 0, 1440, 900)],
                nonce: 0x0123_4567_89ab_cdef,
                os: os::MACOS,
            },
            Control::Screens(vec![]),
            Control::Enter { epoch: 9, pos: Pos::new(0.0, 540.0) },
            Control::Leave,
            Control::Key { code: 30, down: true },
            Control::Button { button: 0x110, down: false, seq: 7, pos: Pos::new(3.5, 4.0) },
            Control::Scroll { dx: 0.0, dy: -12.5 },
            Control::ScrollDiscrete { dx: 0, dy: -120 },
            Control::ScrollStop,
            Control::EnterAck { epoch: 1 },
            Control::Busy { epoch: 2 },
            Control::Revoked { epoch: u16::MAX },
            Control::Layout { side_of_you: Side::Bottom, version: 1_700_000_000_123, offset: None },
            Control::Layout { side_of_you: Side::Left, version: 7, offset: Some((-3432, 98)) },
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
    fn stream_kind_round_trip() {
        for k in [StreamKind::Control, StreamKind::Pairing, StreamKind::Clipboard, StreamKind::File, StreamKind::Probe] {
            assert_eq!(StreamKind::from_u8(k as u8), Some(k));
        }
        assert_eq!(StreamKind::Clipboard as u8, 3);
        assert_eq!(StreamKind::File as u8, 4);
        assert_eq!(StreamKind::from_u8(0), None);
        assert_eq!(StreamKind::from_u8(6), None);
    }

    #[test]
    fn layout_rejects_unknown_side() {
        let mut body = vec![13, 9];
        body.extend_from_slice(&1u64.to_le_bytes());
        assert_eq!(Control::decode(&body), Err(DecodeError::Invalid("side")));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Datagram::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(Datagram::decode(&[1, 0]), Err(DecodeError::Truncated));
        assert_eq!(Control::decode(&[200]), Err(DecodeError::UnknownTag(200)));
        assert_eq!(Datagram::decode(&[2]), Err(DecodeError::UnknownTag(2))); // the old Keepalive
    }

    fn screens_body(rects: &[[i32; 4]]) -> Vec<u8> {
        let mut b = vec![2, rects.len() as u8];
        for r in rects {
            for v in r {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn rejects_out_of_range_values() {
        let invalid = |b: &[u8]| matches!(Control::decode(b), Err(DecodeError::Invalid(_)));
        assert!(Control::decode(&screens_body(&[[0, 0, 1920, 1080], [-1440, 0, 1440, 900]])).is_ok());
        assert!(invalid(&screens_body(&[[0, 0, 0, 1080]])), "zero width");
        assert!(invalid(&screens_body(&[[0, 0, 1920, -5]])), "negative height");
        assert!(invalid(&screens_body(&[[i32::MAX - 10, 0, 1920, 1080]])), "right edge overflows");
        assert!(invalid(&screens_body(&[[0, i32::MAX, 10, 10]])), "bottom edge overflows");
        assert!(invalid(&screens_body(&[[i32::MIN, 0, 10, 10], [i32::MAX - 20, 0, 10, 10]])), "span overflows");
        assert!(invalid(&screens_body(&vec![[0, 0, 10, 10]; MAX_SCREENS + 1])), "too many screens");
        assert!(Control::decode(&screens_body(&vec![[0, 0, 10, 10]; MAX_SCREENS])).is_ok());

        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut b = vec![7];
            b.extend_from_slice(&0f32.to_bits().to_le_bytes());
            b.extend_from_slice(&bad.to_bits().to_le_bytes());
            assert!(invalid(&b), "{bad}");
        }
        // Positions are fixed-point, so even extreme ones decode as finite.
        let mut b = vec![3, 0, 0];
        b.extend_from_slice(&i32::MIN.to_le_bytes());
        b.extend_from_slice(&i32::MAX.to_le_bytes());
        let Ok(Control::Enter { pos, .. }) = Control::decode(&b) else { panic!() };
        assert!(pos.x.is_finite() && pos.y.is_finite());
    }

    #[test]
    fn layout_offset_is_optional_trailing_bytes() {
        // What an older peer sends: no offset after the version.
        let mut old = vec![13, 1];
        old.extend_from_slice(&42u64.to_le_bytes());
        assert_eq!(Control::decode(&old), Ok(Control::Layout { side_of_you: Side::Right, version: 42, offset: None }));
        // And a newer one; an older reader stops before the trailing bytes.
        let mut new = old.clone();
        new.extend_from_slice(&(-1920i32).to_le_bytes());
        new.extend_from_slice(&98i32.to_le_bytes());
        assert_eq!(Control::decode(&new), Ok(Control::Layout { side_of_you: Side::Right, version: 42, offset: Some((-1920, 98)) }));
        let mut buf = Vec::new();
        Control::Layout { side_of_you: Side::Right, version: 42, offset: Some((-1920, 98)) }.encode_frame(&mut buf);
        assert_eq!(&buf[2..], &new[..]);
        let mut bad = old;
        bad.extend_from_slice(&i32::MIN.to_le_bytes());
        bad.extend_from_slice(&0i32.to_le_bytes());
        assert_eq!(Control::decode(&bad), Err(DecodeError::Invalid("offset")));
    }

    #[test]
    fn hello_from_another_version() {
        let mut buf = Vec::new();
        Control::Hello { version: PROTOCOL_VERSION, name: "x".into(), screens: vec![], nonce: 1, os: 0 }
            .encode_frame(&mut buf);
        buf[3..5].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(Control::decode(&buf[2..]), Err(DecodeError::Version(1)));
    }
}
