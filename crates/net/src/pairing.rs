//! Pairing by comparing a 6-digit code, like Bluetooth "numeric comparison".
//!
//! Both devices display a code derived from both certificate fingerprints,
//! both device names and fresh random nonces. The steps are:
//!
//! 1. Both send their name.
//! 2. The initiator commits to its nonce: `H(transcript, na)`.
//! 3. The responder sends its nonce `nb`.
//! 4. The initiator reveals `na`; the responder checks the commitment.
//!
//! Everything the user sees (names) and everything that identifies the
//! devices (fingerprints) is fixed before any nonce is revealed, so a
//! man-in-the-middle can neither search for certificates nor for names
//! that produce matching codes: a forged pairing succeeds with probability
//! 1 in 1,000,000, and only if the user confirms it. Names are bound into
//! the code too, so a name shown next to the code is the one the other
//! device actually sent.

use crate::identity::Fingerprint;
use crate::node::Peer;
use anyhow::{Context, Result, bail, ensure};
use one_mk_proto::{StreamKind, sanitize_name};
use sha2::{Digest, Sha256};
use std::fmt;
use std::future::Future;
use std::time::Duration;

/// Bumped whenever the pairing exchange or transcript changes.
const VERSION: &[u8] = b"one-mk pairing v2";
/// Longest name accepted from the peer, in bytes (before cleaning).
const MAX_NAME: usize = 1024;
/// Every automatic step must finish within this.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// The peer's name, cleaned with `sanitize_name` (safe to print).
    pub peer_name: String,
}

impl Pairing {
    /// Runs the code agreement. Afterwards both sides show `sas` and the
    /// user confirms on each that the codes match. Each step times out
    /// after `STEP_TIMEOUT`.
    pub async fn start(peer: &Peer, me: Fingerprint, my_name: &str) -> Result<Self> {
        let my_name = sanitize_name(my_name);
        let (mut send, mut recv) = if peer.initiator {
            step("opening the pairing stream", peer.open_stream(StreamKind::Pairing)).await?
        } else {
            let (kind, send, recv) = step("waiting for the pairing stream", peer.accept_stream()).await?;
            ensure!(kind == StreamKind::Pairing, "peer did not start pairing");
            (send, recv)
        };

        // 1. Names, before any nonce.
        let mut frame = Vec::with_capacity(2 + my_name.len());
        put_bytes(&mut frame, my_name.as_bytes());
        step("sending our name", send.write_all(&frame)).await?;
        let peer_name_raw = step("receiving the peer's name", read_name(&mut recv)).await?;

        let sas = if peer.initiator {
            let t = Transcript::new(me, peer.fingerprint, my_name.as_bytes(), &peer_name_raw);
            let na: [u8; 32] = rand::random();
            step("sending the commitment", send.write_all(&t.commit(&na))).await?;
            let mut nb = [0u8; 32];
            step("receiving the peer's nonce", recv.read_exact(&mut nb)).await?;
            step("revealing our nonce", send.write_all(&na)).await?;
            t.sas(&na, &nb)
        } else {
            let t = Transcript::new(peer.fingerprint, me, &peer_name_raw, my_name.as_bytes());
            let mut commit = [0u8; 32];
            step("receiving the commitment", recv.read_exact(&mut commit)).await?;
            let nb: [u8; 32] = rand::random();
            step("sending our nonce", send.write_all(&nb)).await?;
            let mut na = [0u8; 32];
            step("receiving the peer's nonce", recv.read_exact(&mut na)).await?;
            ensure!(t.commit(&na) == commit, "pairing commitment mismatch");
            t.sas(&na, &nb)
        };
        let peer_name = sanitize_name(&String::from_utf8_lossy(&peer_name_raw));
        Ok(Self { send, recv, sas, peer_name })
    }

    /// Sends our decision and waits for the peer's. True only if both
    /// users accepted. Waits as long as the other user takes; callers
    /// should bound it.
    pub async fn confirm(mut self, accept: bool) -> Result<bool> {
        self.send.write_all(&[u8::from(accept)]).await?;
        self.send.finish()?;
        let mut theirs = [0u8; 1];
        self.recv.read_exact(&mut theirs).await?;
        // Make sure our answer arrived before the caller drops the
        // connection (which would discard it, and the other side would see
        // "went away" after saving nothing). Bounded: a peer that never
        // acknowledges can't hold us here.
        let _ = tokio::time::timeout(Duration::from_secs(3), self.send.stopped()).await;
        Ok(accept && theirs[0] == 1)
    }
}

async fn step<T, E>(what: &str, fut: impl Future<Output = Result<T, E>>) -> Result<T>
where
    E: Into<anyhow::Error>,
{
    match tokio::time::timeout(STEP_TIMEOUT, fut).await {
        Ok(res) => res.map_err(Into::into).with_context(|| format!("pairing: {what}")),
        Err(_) => bail!("pairing: timed out {what}"),
    }
}

async fn read_name(recv: &mut quinn::RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await?;
    let len = u16::from_be_bytes(len) as usize;
    ensure!(len <= MAX_NAME, "peer name too long ({len} bytes)");
    let mut name = vec![0u8; len];
    recv.read_exact(&mut name).await?;
    Ok(name)
}

/// Appends `b` with a 2-byte big-endian length prefix.
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    let len = u16::try_from(b.len()).expect("names are short");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(b);
}

/// The canonical, length-delimited record of who is pairing with whom,
/// as seen identically by both sides.
struct Transcript(Vec<u8>);

impl Transcript {
    fn new(initiator: Fingerprint, responder: Fingerprint, initiator_name: &[u8], responder_name: &[u8]) -> Self {
        let mut t = Vec::new();
        put_bytes(&mut t, VERSION);
        put_bytes(&mut t, &initiator.0);
        put_bytes(&mut t, &responder.0);
        put_bytes(&mut t, initiator_name);
        put_bytes(&mut t, responder_name);
        Self(t)
    }

    fn commit(&self, na: &[u8; 32]) -> [u8; 32] {
        Sha256::new().chain_update(b"commit").chain_update(&self.0).chain_update(na).finalize().into()
    }

    fn sas(&self, na: &[u8; 32], nb: &[u8; 32]) -> Sas {
        let h = Sha256::new()
            .chain_update(b"sas")
            .chain_update(&self.0)
            .chain_update(na)
            .chain_update(nb)
            .finalize();
        Sas(u32::from_le_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sas_format() {
        assert_eq!(Sas(482_913).to_string(), "482 913");
        assert_eq!(Sas(7).to_string(), "000 007");
    }

    #[test]
    fn names_are_bound_into_commitment_and_code() {
        let (a, b) = (Fingerprint([1; 32]), Fingerprint([2; 32]));
        let (na, nb) = ([3u8; 32], [4u8; 32]);
        let base = Transcript::new(a, b, b"alice", b"bob");
        for other in [
            Transcript::new(a, b, b"alice", b"bob2"),
            Transcript::new(a, b, b"alic", b"ebob"), // same concatenation
            Transcript::new(a, b, b"bob", b"alice"),
            Transcript::new(b, a, b"alice", b"bob"),
        ] {
            assert_ne!(base.commit(&na), other.commit(&na));
            assert_ne!(base.0, other.0);
        }
        assert_eq!(base.sas(&na, &nb), Transcript::new(a, b, b"alice", b"bob").sas(&na, &nb));
        assert_ne!(base.commit(&na), base.commit(&nb));
    }

    #[test]
    fn transcript_is_versioned() {
        let t = Transcript::new(Fingerprint([0; 32]), Fingerprint([0; 32]), b"", b"");
        assert!(t.0.windows(VERSION.len()).any(|w| w == VERSION));
        assert_eq!(VERSION, b"one-mk pairing v2");
    }
}
