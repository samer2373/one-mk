//! Compact little-endian binary encoding. Positions are fixed-point with
//! 1/256 px precision.

use crate::{Align, PROTOCOL_VERSION, Pos, Rect, Side};
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
    /// Relative motion (gaming mode): the sum of every delta sent during
    /// visit `epoch`, in 1/256 px, wrapping. Carrying the running sum, not
    /// the step, makes it self-contained like `Motion`: a lost datagram is
    /// made up by the next, so the pointer never drifts, and a resend (same
    /// `seq`) moves nothing. Only sent to peers with [`caps::RELATIVE`].
    MotionBy { epoch: u16, seq: u32, sum_x: i32, sum_y: i32 },
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
            Datagram::MotionBy { epoch, seq, sum_x, sum_y } => {
                w.u8(5);
                w.u16(epoch);
                w.u32(seq);
                w.i32(sum_x);
                w.i32(sum_y);
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
            5 => Datagram::MotionBy { epoch: r.u16()?, seq: r.u32()?, sum_x: r.i32()?, sum_y: r.i32()? },
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

/// Optional features, announced with [`Control::Caps`]. Each is additive
/// within protocol 2: a peer that doesn't announce one never gets sent it
/// (or, for messages that are safe to ignore, skips them as unknown tags).
pub mod caps {
    /// Understands [`Datagram::MotionBy`](crate::Datagram::MotionBy) and
    /// [`Control::PointerMode`](crate::Control::PointerMode).
    pub const RELATIVE: u32 = 1;
    /// Understands [`Control::Lock`](crate::Control::Lock).
    pub const LOCK: u32 = 2;
    /// Answers [`Control::WatchFocus`](crate::Control::WatchFocus) with
    /// [`Control::Focus`](crate::Control::Focus).
    pub const FOCUS: u32 = 4;
    /// Understands [`Control::Inhibit`](crate::Control::Inhibit) and
    /// [`Control::Sleep`](crate::Control::Sleep).
    pub const POWER: u32 = 8;
    /// While watched and fullscreen, says [`Control::Focus`] again at least
    /// every [`FOCUS_REPEAT`](crate::FOCUS_REPEAT), so a watcher that hears
    /// nothing for a while can stop trusting its last "fullscreen".
    pub const FOCUS_BEAT: u32 = 16;
    /// What this build supports.
    pub const LOCAL: u32 = RELATIVE | LOCK | FOCUS | POWER | FOCUS_BEAT;
}

/// How often a computer with [`caps::FOCUS_BEAT`] repeats `Focus` while
/// a fullscreen window has the focus. Its controller drops relative motion
/// (where no edge leads home) after [`FOCUS_SILENCE`] without one.
pub const FOCUS_REPEAT: std::time::Duration = std::time::Duration::from_secs(2);
pub const FOCUS_SILENCE: std::time::Duration = std::time::Duration::from_secs(6);

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
    /// Layout sync: the sender sits on `side_of_you` of the receiver,
    /// lined up with it as `align` says (from the receiver's point of view:
    /// the sender mirrors its own setting). The receiver adopts it if
    /// `version` is newer than the one it has stored.
    ///
    /// `align` is trailing (a kind byte and an `i32`): `None` when the
    /// sender predates alignment, or sent a kind this build doesn't know.
    /// Builds that predate it read the side and version and skip the rest.
    Layout { side_of_you: Side, version: u64, align: Option<Align> },
    // Additive messages. Peers that predate them skip unknown tags (see
    // `link.rs`), so they need no new protocol version.
    /// The optional features the sender supports (bits of [`caps`]), sent
    /// right after `Hello`.
    Caps(u32),
    /// The controller switches visit `epoch` to relative motion
    /// ([`Datagram::MotionBy`]) or back to absolute positions. While
    /// relative, a `Button`'s `pos` means nothing and is not applied.
    PointerMode { epoch: u16, relative: bool },
    /// Lock your screen. `id` names the lock event, so a computer that
    /// passes it on never gets it back as a new one. `all`: someone ran
    /// `one-mk lock-all`, which every computer obeys; otherwise it is lock
    /// sync, which only computers with lock sync on follow. (`all` is a
    /// trailing byte: a frame without it, from a build before it, counts
    /// as `all`, as every computer obeyed those.)
    Lock { id: u64, all: bool },
    /// From the controlled computer: its focused window is (or is no
    /// longer) fullscreen, during visit `epoch`. Gaming mode `auto` uses it.
    /// With [`caps::FOCUS_BEAT`], "fullscreen" is said again at least every
    /// [`FOCUS_REPEAT`] while it lasts.
    Focus { epoch: u16, fullscreen: bool },
    /// From the controller: send `Focus` now and whenever it changes during
    /// visit `epoch` (`on`), or stop (gaming mode left `auto`). Only gaming
    /// mode `auto` asks, so nobody else pays for looking at windows. (`on`
    /// is a trailing byte: a frame without it, from a build before it,
    /// means on; such a build reads an "off" as "on", which it already is.)
    WatchFocus { epoch: u16, on: bool },
    /// From the controller, during visit `epoch`: keep the screensaver,
    /// display blanking and idle sleep off while it lasts (`on`), or stop.
    /// The receiver also lets go when the visit ends or the link drops, so
    /// a controller that vanishes never leaves the screen awake.
    Inhibit { epoch: u16, on: bool },
    /// Put your display to sleep (sleep sync). `id` names the event, so a
    /// computer that passes it on never gets it back as a new one.
    Sleep { id: u64 },
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
            Control::Layout { side_of_you, version, align } => {
                w.u8(13);
                w.u8(Side::ALL.iter().position(|s| s == side_of_you).unwrap_or(0) as u8);
                w.u64(*version);
                if let Some(a) = align {
                    let (kind, px) = match a {
                        Align::Stretch => (0, 0),
                        Align::Offset(o) => (1, *o),
                        Align::Center => (2, 0),
                    };
                    w.u8(kind);
                    w.i32(px);
                }
            }
            Control::Caps(bits) => {
                w.u8(14);
                w.u32(*bits);
            }
            Control::PointerMode { epoch, relative } => {
                w.u8(15);
                w.u16(*epoch);
                w.u8(u8::from(*relative));
            }
            Control::Lock { id, all } => {
                w.u8(16);
                w.u64(*id);
                w.u8(u8::from(*all));
            }
            Control::Focus { epoch, fullscreen } => {
                w.u8(17);
                w.u16(*epoch);
                w.u8(u8::from(*fullscreen));
            }
            Control::WatchFocus { epoch, on } => {
                w.u8(18);
                w.u16(*epoch);
                w.u8(u8::from(*on));
            }
            Control::Inhibit { epoch, on } => {
                w.u8(19);
                w.u16(*epoch);
                w.u8(u8::from(*on));
            }
            Control::Sleep { id } => {
                w.u8(20);
                w.u64(*id);
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
                align: r.align()?,
            },
            14 => Control::Caps(r.u32()?),
            15 => Control::PointerMode { epoch: r.u16()?, relative: r.u8()? != 0 },
            16 => Control::Lock { id: r.u64()?, all: r.flag_or(true)? },
            17 => Control::Focus { epoch: r.u16()?, fullscreen: r.u8()? != 0 },
            18 => Control::WatchFocus { epoch: r.u16()?, on: r.flag_or(true)? },
            19 => Control::Inhibit { epoch: r.u16()?, on: r.u8()? != 0 },
            20 => Control::Sleep { id: r.u64()? },
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
    /// A flag added at the end of a message: `default` when the sender's
    /// build predates it and the message ends here.
    fn flag_or(&mut self, default: bool) -> Result<bool, DecodeError> {
        if self.0.is_empty() { Ok(default) } else { Ok(self.u8()? != 0) }
    }
    /// [`Control::Layout`]'s trailing alignment.
    fn align(&mut self) -> Result<Option<Align>, DecodeError> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let (kind, px) = (self.u8()?, self.i32()?);
        Ok(match kind {
            0 => Some(Align::Stretch),
            1 => Some(Align::Offset(px)),
            2 => Some(Align::Center),
            _ => None,
        })
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
            Datagram::MotionBy { epoch: 1, seq: 2, sum_x: i32::MIN, sum_y: -3 },
        ] {
            let (buf, n) = d.encode();
            assert_eq!(Datagram::decode(&buf[..n]).unwrap(), d);
        }
        assert_eq!(Datagram::Motion { epoch: 0, seq: 0, pos: Pos::default() }.encode().1, 15);
        assert_eq!(Datagram::MotionBy { epoch: 0, seq: 0, sum_x: 0, sum_y: 0 }.encode().1, 15);
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
            Control::Layout { side_of_you: Side::Bottom, version: 1_700_000_000_123, align: None },
            Control::Layout { side_of_you: Side::Left, version: 1, align: Some(Align::Offset(-300)) },
            Control::Layout { side_of_you: Side::Top, version: 2, align: Some(Align::Center) },
            Control::Layout { side_of_you: Side::Right, version: 3, align: Some(Align::Stretch) },
            Control::Caps(caps::LOCAL),
            Control::PointerMode { epoch: 3, relative: true },
            Control::PointerMode { epoch: 3, relative: false },
            Control::Lock { id: u64::MAX, all: true },
            Control::Lock { id: 7, all: false },
            Control::Focus { epoch: 4, fullscreen: true },
            Control::WatchFocus { epoch: 5, on: true },
            Control::WatchFocus { epoch: 5, on: false },
            Control::Inhibit { epoch: 6, on: true },
            Control::Inhibit { epoch: 6, on: false },
            Control::Sleep { id: u64::MAX },
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

    /// The messages added within protocol 2 use tags that protocol-2 peers
    /// built before them don't know, so they skip them (`UnknownTag`) and
    /// keep the link: no version bump is needed. This pins the tag numbers
    /// those older decoders saw as unknown.
    #[test]
    fn additive_messages_use_fresh_tags() {
        let tag = |m: &Control| {
            let mut b = Vec::new();
            m.encode_frame(&mut b);
            b[2]
        };
        let new = [
            Control::Caps(0),
            Control::PointerMode { epoch: 0, relative: true },
            Control::Lock { id: 0, all: true },
            Control::Focus { epoch: 0, fullscreen: false },
            Control::WatchFocus { epoch: 0, on: true },
            Control::Inhibit { epoch: 0, on: true },
            Control::Sleep { id: 0 },
        ];
        assert_eq!(new.iter().map(tag).collect::<Vec<_>>(), [14, 15, 16, 17, 18, 19, 20]);
        // Flags added later at the end of `Lock` and `WatchFocus`: frames
        // from builds before them decode as they meant (lock-all, watch on),
        // and those builds read the new frames' fields they know, ignoring
        // the extra byte.
        let mut id = vec![16];
        id.extend_from_slice(&9u64.to_le_bytes());
        assert_eq!(Control::decode(&id), Ok(Control::Lock { id: 9, all: true }));
        assert_eq!(Control::decode(&[18, 3, 0]), Ok(Control::WatchFocus { epoch: 3, on: true }));
        let mut b = Vec::new();
        Control::WatchFocus { epoch: 3, on: false }.encode_frame(&mut b);
        assert_eq!(&b[2..5], &[18, 3, 0], "old fields first, at the same place");
        b.clear();
        Control::Lock { id: 9, all: false }.encode_frame(&mut b);
        assert_eq!(&b[2..11], &id[..]);
        let (b, _) = Datagram::MotionBy { epoch: 0, seq: 0, sum_x: 0, sum_y: 0 }.encode();
        assert_eq!(b[0], 5);
        // A Hello is still the same bytes, so old peers still connect.
        let mut hello = Vec::new();
        Control::Hello { version: PROTOCOL_VERSION, name: "x".into(), screens: vec![], nonce: 0, os: 0 }
            .encode_frame(&mut hello);
        assert_eq!(hello.len(), 2 + 1 + 2 + 2 + 1 + 8 + 1);
    }

    #[test]
    fn layout_rejects_unknown_side() {
        let mut body = vec![13, 9];
        body.extend_from_slice(&1u64.to_le_bytes());
        assert_eq!(Control::decode(&body), Err(DecodeError::Invalid("side")));
    }

    /// Alignment is additive: a frame from a build before it has none, a
    /// build before it reads ours as the same side and version, and a kind
    /// from a newer build is ignored rather than refusing the layout.
    #[test]
    fn layout_alignment_is_compatible() {
        let mut old = vec![13, 3];
        old.extend_from_slice(&7u64.to_le_bytes());
        assert_eq!(Control::decode(&old), Ok(Control::Layout { side_of_you: Side::Bottom, version: 7, align: None }));
        let mut new = Vec::new();
        Control::Layout { side_of_you: Side::Bottom, version: 7, align: Some(Align::Offset(300)) }.encode_frame(&mut new);
        assert_eq!(&new[2..2 + old.len()], &old[..], "an older build reads the same side and version first");
        assert_eq!(new.len(), 2 + old.len() + 5);
        let mut future = old.clone();
        future.push(9);
        future.extend_from_slice(&5i32.to_le_bytes());
        assert_eq!(Control::decode(&future), Ok(Control::Layout { side_of_you: Side::Bottom, version: 7, align: None }));
        let mut cut = old.clone();
        cut.push(1);
        assert_eq!(Control::decode(&cut), Err(DecodeError::Truncated));
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
    fn hello_from_another_version() {
        let mut buf = Vec::new();
        Control::Hello { version: PROTOCOL_VERSION, name: "x".into(), screens: vec![], nonce: 1, os: 0 }
            .encode_frame(&mut buf);
        buf[3..5].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(Control::decode(&buf[2..]), Err(DecodeError::Version(1)));
    }
}
