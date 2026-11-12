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

/// `default`, or in a debug build the number of milliseconds the environment
/// variable `var` holds: how the tests (and only they) wait milliseconds for a
/// timeout that is half a minute in use. A release build never reads the
/// environment, so no shipped build can be made to give up sooner than its
/// default; everywhere else the defaults are what they were.
pub fn test_timeout(default: Duration, var: &str) -> Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var(var).ok().and_then(|v| v.parse::<u64>().ok()).filter(|ms| *ms > 0) {
        return Duration::from_millis(ms);
    }
    let _ = var;
    default
}

/// How long each step of the code agreement may take now ([`STEP_TIMEOUT`]; in
/// a debug build `ONE_MK_TEST_PAIR_STEP_MS` shortens it).
pub fn step_timeout() -> Duration {
    test_timeout(STEP_TIMEOUT, "ONE_MK_TEST_PAIR_STEP_MS")
}

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
        Self::start_within(peer, me, my_name, step_timeout()).await
    }

    /// [`Pairing::start`] with each step given `limit`: how a test sees a step
    /// run out of time without waiting half a minute for it.
    pub async fn start_within(peer: &Peer, me: Fingerprint, my_name: &str, limit: Duration) -> Result<Self> {
        let my_name = sanitize_name(my_name);
        let (mut send, mut recv) = if peer.initiator {
            step(limit, "opening the pairing stream", peer.open_stream(StreamKind::Pairing)).await?
        } else {
            let (kind, send, recv) = step(limit, "waiting for the pairing stream", peer.accept_stream()).await?;
            ensure!(kind == StreamKind::Pairing, "peer did not start pairing");
            (send, recv)
        };

        // 1. Names, before any nonce.
        let mut frame = Vec::with_capacity(2 + my_name.len());
        put_bytes(&mut frame, my_name.as_bytes());
        step(limit, "sending our name", send.write_all(&frame)).await?;
        let peer_name_raw = step(limit, "receiving the peer's name", read_name(&mut recv)).await?;

        let sas = if peer.initiator {
            let t = Transcript::new(me, peer.fingerprint, my_name.as_bytes(), &peer_name_raw);
            let na: [u8; 32] = rand::random();
            step(limit, "sending the commitment", send.write_all(&t.commit(&na))).await?;
            let mut nb = [0u8; 32];
            step(limit, "receiving the peer's nonce", recv.read_exact(&mut nb)).await?;
            step(limit, "revealing our nonce", send.write_all(&na)).await?;
            t.sas(&na, &nb)
        } else {
            let t = Transcript::new(peer.fingerprint, me, &peer_name_raw, my_name.as_bytes());
            let mut commit = [0u8; 32];
            step(limit, "receiving the commitment", recv.read_exact(&mut commit)).await?;
            let nb: [u8; 32] = rand::random();
            step(limit, "sending our nonce", send.write_all(&nb)).await?;
            let mut na = [0u8; 32];
            step(limit, "receiving the peer's nonce", recv.read_exact(&mut na)).await?;
            ensure!(t.commit(&na) == commit, "pairing commitment mismatch");
            t.sas(&na, &nb)
        };
        let peer_name = sanitize_name(&String::from_utf8_lossy(&peer_name_raw));
        Ok(Self { send, recv, sas, peer_name })
    }

    /// Sends our decision and waits for the peer's. True only if both
    /// users accepted. Waits as long as the other user takes; callers
    /// should bound it.
    pub async fn confirm(self, accept: bool) -> Result<bool> {
        self.confirm_port(accept, None).await.map(|(confirmed, _)| confirmed)
    }

    pub async fn confirm_port(mut self, accept: bool, service_port: Option<u16>) -> Result<(bool, Option<u16>)> {
        let mut answer = vec![u8::from(accept)];
        if let Some(port) = service_port { answer.extend_from_slice(&port.to_le_bytes()); }
        self.send.write_all(&answer).await?;
        self.send.finish()?;
        let mut theirs = [0u8; 1];
        self.recv.read_exact(&mut theirs).await?;
        // Make sure our answer arrived before the caller drops the
        // connection (which would discard it, and the other side would see
        // "went away" after saving nothing). Bounded: a peer that never
        // acknowledges can't hold us here.
        let _ = tokio::time::timeout(Duration::from_secs(3), self.send.stopped()).await;
        let trailing = step(step_timeout(), "receiving the service port", self.recv.read_to_end(2)).await?;
        let port = match trailing.as_slice() {
            [] => None,
            [low, high] => Some(u16::from_le_bytes([*low, *high])),
            _ => bail!("invalid pairing service port"),
        };
        ensure!(port != Some(0), "invalid pairing service port");
        Ok((accept && theirs[0] == 1, port))
    }
}

async fn step<T, E>(limit: Duration, what: &str, fut: impl Future<Output = Result<T, E>>) -> Result<T>
where
    E: Into<anyhow::Error>,
{
    match tokio::time::timeout(limit, fut).await {
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

    #[tokio::test]
    async fn service_port_extension_accepts_a_legacy_confirmation() {
        use crate::{Identity, Node};
        use std::sync::Arc;
        let local = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let remote = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (local_peer, remote_peer) = tokio::join!(local.connect(remote.local_addr().unwrap()), async { remote.accept().await.unwrap() });
        let (local_peer, remote_peer) = (local_peer.unwrap(), remote_peer.unwrap());
        let (local_pair, remote_pair) = tokio::join!(
            Pairing::start(&local_peer, local.identity().fingerprint, "new"),
            Pairing::start(&remote_peer, remote.identity().fingerprint, "old")
        );
        let legacy = async {
            let mut pairing = remote_pair.unwrap();
            pairing.send.write_all(&[1]).await.unwrap();
            pairing.send.finish().unwrap();
            let mut answer = [0];
            pairing.recv.read_exact(&mut answer).await.unwrap();
            let _ = tokio::time::timeout(Duration::from_secs(3), pairing.send.stopped()).await;
            assert_eq!(answer, [1]);
        };
        let (new, ()) = tokio::join!(local_pair.unwrap().confirm_port(true, Some(32123)), legacy);
        assert_eq!(new.unwrap(), (true, None));
    }

    async fn connected_pair() -> (crate::Node, crate::Node, Peer, Peer) {
        use crate::{Identity, Node};
        use std::sync::Arc;
        let local = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let remote = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (local_peer, remote_peer) = tokio::join!(local.connect(remote.local_addr().unwrap()), async { remote.accept().await.unwrap() });
        (local, remote, local_peer.unwrap(), remote_peer.unwrap())
    }

    /// The other computer connects and then says nothing: every step ends by
    /// itself, in the time it was given, and says which step it was.
    #[tokio::test]
    async fn a_silent_peer_times_out_in_the_time_given_and_says_which_step() {
        let (local, _remote, local_peer, _remote_peer) = connected_pair().await;
        let limit = Duration::from_millis(150);
        // The one that dials opens the stream, sends its name and waits for the other's.
        let started = std::time::Instant::now();
        let error = Pairing::start_within(&local_peer, local.identity().fingerprint, "dialer", limit).await.err().expect("it must give up");
        let waited = started.elapsed();
        assert!(format!("{error:#}").contains("pairing: timed out receiving the peer's name"), "{error:#}");
        assert!(waited >= limit && waited < Duration::from_secs(5), "{waited:?}");
        // The one that is dialed waits for a stream that never comes.
        let (_local, remote, _local_peer, remote_peer) = connected_pair().await;
        let started = std::time::Instant::now();
        let error = Pairing::start_within(&remote_peer, remote.identity().fingerprint, "listener", limit).await.err().expect("it must give up");
        assert!(format!("{error:#}").contains("pairing: timed out waiting for the pairing stream"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A peer that stalls half way through (after the names, before its nonce)
    /// is let go the same way, and nothing was ever returned that could be saved.
    #[tokio::test]
    async fn a_peer_that_stalls_after_the_names_is_let_go() {
        let (local, remote, local_peer, remote_peer) = connected_pair().await;
        let limit = Duration::from_millis(200);
        let (dialer, ()) = tokio::join!(
            Pairing::start_within(&local_peer, local.identity().fingerprint, "dialer", limit),
            async {
                // Answers the name step like a real peer, then goes quiet.
                let (_, mut send, mut recv) = remote_peer.accept_stream().await.unwrap();
                let mut frame = Vec::new();
                put_bytes(&mut frame, b"listener");
                send.write_all(&frame).await.unwrap();
                let _ = read_name(&mut recv).await.unwrap();
                tokio::time::sleep(Duration::from_millis(600)).await;
                drop((send, recv, remote));
            }
        );
        let error = dialer.err().expect("the commitment is never answered");
        assert!(format!("{error:#}").contains("pairing: timed out receiving the peer's nonce"), "{error:#}");
    }

    #[test]
    fn the_timeouts_are_what_they_were_unless_a_debug_test_shortens_them() {
        assert_eq!(STEP_TIMEOUT, Duration::from_secs(30));
        // Nothing sets this variable here, and a value that is not a positive
        // number of milliseconds is ignored.
        assert_eq!(test_timeout(Duration::from_secs(120), "ONE_MK_TEST_NOT_SET_ANYWHERE"), Duration::from_secs(120));
        assert_eq!(step_timeout(), STEP_TIMEOUT);
    }

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
