//! One connection to a trusted peer: the handshake, and the tasks that
//! read what the peer sends. Everything read is handed to the app, which
//! alone decides what to do with it.

use anyhow::{Context, Result, bail};
use one_mk_net::{ControlWriter, Fingerprint, Identity, Peer, read_frame};
use one_mk_proto::{Control, Datagram, DecodeError, PROTOCOL_VERSION, Pos, Rect, os, sanitize_name};
use std::hash::{BuildHasher, RandomState};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;

pub struct Link {
    pub fp: Fingerprint,
    pub id: usize,
    /// Display-safe name the peer gave itself.
    pub name: String,
    /// The peer's OS (see [`one_mk_proto::os`]).
    pub os: u8,
    pub rank: Rank,
    pub conn: quinn::Connection,
    pub ctl: ControlWriter,
    /// The peer's monitors.
    pub screens: Vec<Rect>,
    /// Our monitors as we announced them in our `Hello`.
    pub sent_screens: Vec<Rect>,
    recv: quinn::RecvStream,
}

/// What a link task tells the app, tagged with the peer and link id.
pub enum LinkEvent {
    Control(Control),
    Motion { epoch: u16, seq: u32, pos: Pos },
    Closed,
}

pub type LinkTx = mpsc::UnboundedSender<(Fingerprint, usize, LinkEvent)>;

/// Orders duplicate connections between the same two machines (both may
/// dial, and each may dial several addresses). Both sides compute the same
/// rank for a connection, and both keep the lowest, whatever order the
/// connections arrive in: those dialed by the lower fingerprint first,
/// then by the dialer's random nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank {
    dialed_by_higher: bool,
    dialer_nonce: u64,
}

impl Rank {
    pub fn new(me: Fingerprint, peer: Fingerprint, initiator: bool, dialer_nonce: u64) -> Self {
        let dialer = if initiator { me } else { peer };
        Self { dialed_by_higher: dialer != me.min(peer), dialer_nonce }
    }
}

fn random_u64() -> u64 {
    // std seeds RandomState from the OS and changes its keys on every call.
    RandomState::new().hash_one(SystemTime::now())
}

/// Exchanges `Hello`s on the control stream (opened by the dialer).
/// `device` is our own name for the peer, for error messages.
pub async fn handshake(
    peer: Peer,
    (send, mut recv): (quinn::SendStream, quinn::RecvStream),
    me: &Identity,
    screens: Vec<Rect>,
    device: &str,
) -> Result<Link> {
    let ctl = ControlWriter::spawn(send);
    let nonce = random_u64();
    ctl.send(Control::Hello {
        version: PROTOCOL_VERSION,
        name: me.name.clone(),
        screens: screens.clone(),
        nonce,
        os: os::local(),
    });

    let mut buf = Vec::new();
    let hello = match tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv, &mut buf))
        .await
        .context("peer did not say hello")?
    {
        Err(e) => match e.downcast_ref::<DecodeError>() {
            Some(DecodeError::Version(v)) => bail!(
                "{device} runs an incompatible version of one-mk (protocol {v}, we speak {PROTOCOL_VERSION}); update both"
            ),
            _ => return Err(e),
        },
        Ok(hello) => hello,
    };
    let Some(Control::Hello { name, screens: peer_screens, nonce: peer_nonce, os, .. }) = hello else {
        bail!("peer sent an unexpected first message");
    };
    let dialer_nonce = if peer.initiator { nonce } else { peer_nonce };
    Ok(Link {
        fp: peer.fingerprint,
        id: peer.conn.stable_id(),
        name: sanitize_name(&name),
        os,
        rank: Rank::new(me.fingerprint, peer.fingerprint, peer.initiator, dialer_nonce),
        conn: peer.conn,
        ctl,
        screens: peer_screens,
        sent_screens: screens,
        recv,
    })
}

impl Link {
    /// Spawns the tasks that read the peer's control stream and datagrams.
    /// Each sends what it reads to `events` as soon as it arrives, so
    /// neither can starve the other, and no read is ever cancelled halfway
    /// through a frame. Exactly one `Closed` follows the last event.
    pub fn spawn(self, events: LinkTx) -> (quinn::Connection, ControlWriter) {
        let Link { fp, id, name, conn, ctl, mut recv, .. } = self;
        let (ctl_conn, ctl_events) = (conn.clone(), events.clone());
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(256);
            loop {
                match read_frame(&mut recv, &mut buf).await {
                    Ok(Some(msg)) => {
                        if ctl_events.send((fp, id, LinkEvent::Control(msg))).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    // The whole frame was read, so the stream is still in
                    // step: skip it. Unknown tags come from newer peers.
                    Err(e) => match e.downcast_ref::<DecodeError>() {
                        Some(DecodeError::UnknownTag(t)) => tracing::debug!(peer = %name, "skipping unknown message {t}"),
                        Some(d) => tracing::warn!(peer = %name, "ignoring a malformed message: {d}"),
                        None => {
                            tracing::debug!(peer = %name, "control stream: {e:#}");
                            break;
                        }
                    },
                }
            }
            // Ends the datagram task below, which reports `Closed`.
            ctl_conn.close(0u32.into(), b"control stream ended");
        });
        let task_conn = conn.clone();
        tokio::spawn(async move {
            while let Ok(bytes) = task_conn.read_datagram().await {
                match Datagram::decode(&bytes) {
                    Ok(Datagram::Motion { epoch, seq, pos }) => {
                        let _ = events.send((fp, id, LinkEvent::Motion { epoch, seq, pos }));
                    }
                    Ok(Datagram::Probe { id, sent_ns }) => {
                        let (b, n) = Datagram::ProbeEcho { id, sent_ns }.encode();
                        let _ = task_conn.send_datagram(bytes::Bytes::copy_from_slice(&b[..n]));
                    }
                    Ok(Datagram::ProbeEcho { .. }) => {}
                    Err(e) => tracing::trace!("bad datagram: {e}"),
                }
            }
            let _ = events.send((fp, id, LinkEvent::Closed));
        });
        (conn, ctl)
    }
}

/// evdev BTN_* codes for mice live in 0x110..0x120.
pub fn is_button(code: u32) -> bool {
    (0x110..0x120).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both machines rank every connection between them identically, so
    /// they keep the same one whatever order the duplicates arrive in.
    #[test]
    fn both_sides_keep_the_same_duplicate() {
        let a = Identity::ephemeral().unwrap().fingerprint;
        let b = Identity::ephemeral().unwrap().fingerprint;
        // (dialed by a?, dialer's nonce) for four duplicate connections.
        let conns = [(true, 7u64), (true, 3), (false, 1), (false, 9)];
        let ranks = |me, peer| -> Vec<Rank> {
            conns.iter().map(|&(by_a, n)| Rank::new(me, peer, by_a == (me == a), n)).collect()
        };
        let (on_a, on_b) = (ranks(a, b), ranks(b, a));
        assert_eq!(on_a, on_b);
        // Arrival order doesn't matter: each side keeps the minimum.
        let keep = |order: &[usize], r: &[Rank]| order.iter().map(|&i| (r[i], i)).min().unwrap().1;
        let winner = keep(&[0, 1, 2, 3], &on_a);
        assert_eq!(keep(&[3, 2, 1, 0], &on_b), winner);
        assert_eq!(winner, if a < b { 1 } else { 2 });
    }
}
