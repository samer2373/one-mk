//! Pairing by comparing a 6-digit code, like Bluetooth "numeric comparison".
//!
//! Both devices display a code derived from both certificate fingerprints
//! and fresh random nonces. The initiator commits to its nonce before
//! seeing the responder's, so a man-in-the-middle cannot search for
//! certificates that produce matching codes: a forged pairing succeeds with
//! probability 1 in 1,000,000, and only if the user confirms it.

use crate::identity::Fingerprint;
use crate::node::Peer;
use anyhow::{Result, ensure};
use one_mk_proto::StreamKind;
use sha2::{Digest, Sha256};
use std::fmt;

/// Short authentication string shown to the user on both devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sas(pub u32);

impl fmt::Display for Sas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03} {:03}", self.0 / 1000, self.0 % 1000)
    }
}

pub struct Pairing {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    pub sas: Sas,
    pub peer_name: String,
}

impl Pairing {
    /// Runs the code agreement. Afterwards both sides show `sas` and the
    /// user confirms on each that the codes match.
    pub async fn start(peer: &Peer, me: Fingerprint, my_name: &str) -> Result<Self> {
        let (mut send, mut recv, sas) = if peer.initiator {
            let (mut send, mut recv) = peer.open_stream(StreamKind::Pairing).await?;
            let na: [u8; 32] = rand::random();
            send.write_all(&Sha256::digest(na)).await?;
            let mut nb = [0u8; 32];
            recv.read_exact(&mut nb).await?;
            send.write_all(&na).await?;
            (send, recv, sas(me, peer.fingerprint, &na, &nb))
        } else {
            let (kind, mut send, mut recv) = peer.accept_stream().await?;
            ensure!(kind == StreamKind::Pairing, "peer did not start pairing");
            let mut commit = [0u8; 32];
            recv.read_exact(&mut commit).await?;
            let nb: [u8; 32] = rand::random();
            send.write_all(&nb).await?;
            let mut na = [0u8; 32];
            recv.read_exact(&mut na).await?;
            ensure!(Sha256::digest(na)[..] == commit, "pairing commitment mismatch");
            (send, recv, sas(peer.fingerprint, me, &na, &nb))
        };
        let name = &my_name.as_bytes()[..my_name.floor_char_boundary(255)];
        send.write_all(&[name.len() as u8]).await?;
        send.write_all(name).await?;
        let mut len = [0u8; 1];
        recv.read_exact(&mut len).await?;
        let mut peer_name = vec![0u8; len[0] as usize];
        recv.read_exact(&mut peer_name).await?;
        let peer_name = String::from_utf8_lossy(&peer_name).into_owned();
        Ok(Self { send, recv, sas, peer_name })
    }

    /// Sends our decision and waits for the peer's. True only if both
    /// users accepted.
    pub async fn confirm(mut self, accept: bool) -> Result<bool> {
        self.send.write_all(&[u8::from(accept)]).await?;
        self.send.finish()?;
        let mut theirs = [0u8; 1];
        self.recv.read_exact(&mut theirs).await?;
        Ok(accept && theirs[0] == 1)
    }
}

fn sas(initiator: Fingerprint, responder: Fingerprint, na: &[u8; 32], nb: &[u8; 32]) -> Sas {
    let h = Sha256::new()
        .chain_update(b"one-mk sas v1")
        .chain_update(initiator.0)
        .chain_update(responder.0)
        .chain_update(na)
        .chain_update(nb)
        .finalize();
    Sas(u32::from_le_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sas_format() {
        assert_eq!(Sas(482_913).to_string(), "482 913");
        assert_eq!(Sas(7).to_string(), "000 007");
    }
}
