//! Networking for one-mk: device identity, mutually-authenticated QUIC,
//! pairing, the trusted-device store and LAN discovery.

mod discovery;
mod files;
mod identity;
pub mod invite;
mod node;
mod pairing;
mod stream;
mod tls;
mod trust;

pub use discovery::{Discovery, Found, is_virtual as is_virtual_interface};
pub use identity::{Fingerprint, Identity};
pub use node::{Node, Peer};
pub use pairing::{Pairing, STEP_TIMEOUT, Sas, step_timeout, test_timeout};
pub use stream::{
    ControlWriter, open_stream, read_clip_header, read_file_msg, read_frame, read_stream_kind, write_file_msg,
};
pub use files::{COMMAND_LOCK_WAIT, DAEMON_LOCK_WAIT, lock_file, sync_dir};
#[doc(hidden)]
#[cfg(all(unix, debug_assertions))]
pub use files::was_synced;
pub use trust::{Device,MAX_OFFSET, PlacedBy, Swap, TrustStore};

pub use quinn;
