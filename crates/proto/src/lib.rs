//! one-mk wire protocol and the pure logic that decides where the pointer is.
//!
//! Nothing in this crate does I/O, so all of it is unit-tested and shared by
//! every platform backend.

mod codec;
mod geometry;
mod keys;
mod motion;
mod transfer;

pub use codec::{Control, Datagram, DecodeError, StreamKind};
pub use geometry::{Desktop, Pos, Rect, Side};
pub use keys::HeldKeys;
pub use motion::{MotionReceiver, RemoteCursor, Step};
pub use transfer::{ClipFormat, ClipHeader, FileMsg};

/// Bumped on any incompatible wire change; peers refuse mismatched versions.
pub const PROTOCOL_VERSION: u16 = 1;
/// QUIC ALPN identifier.
pub const ALPN: &[u8] = b"one-mk/1";
/// Default UDP port.
pub const DEFAULT_PORT: u16 = 24900;
