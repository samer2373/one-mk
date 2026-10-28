//! Networking for one-mk: device identity, mutually-authenticated QUIC,
//! pairing, the trusted-device store and LAN discovery.

mod discovery;
mod identity;
mod node;
mod pairing;
mod stream;
mod tls;
mod trust;

pub use discovery::{Discovery, Found};
pub use identity::{Fingerprint, Identity};
pub use node::{Node, Peer};
pub use pairing::{Pairing, Sas};
pub use stream::{read_frame, ControlWriter};
pub use trust::{Device, TrustStore};

pub use quinn;
