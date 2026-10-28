//! Clipboard sharing.
//!
//! - When the pointer moves to a peer, we send our clipboard to it, unless
//!   it already has that content.
//! - When a peer's pointer leaves us, we send our clipboard back to it, but
//!   only if the user copied something here while controlling us.
//!
//! Snapshots never travel on the input connection: each peer gets a
//! separate "bulk" connection, dialed on first use, so a large copy can't
//! hold up mouse datagrams even through QUIC congestion control. All
//! clipboard access happens on the clipboard thread. Echoes are prevented
//! by remembering, per peer, the digest of the content it is known to have
//! (sent to it or received from it).

use anyhow::{Context, bail};
use one_mk_clipboard::{ClipData, Clipboard, Digest, MAX_BYTES, Read};
use one_mk_net::{Fingerprint, Node, open_stream, read_clip_header};
use one_mk_proto::{ClipFormat, ClipHeader, StreamKind};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// The longest a peer may take to deliver one clipboard snapshot.
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(30);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

pub struct ClipSync {
    cb: Clipboard,
    node: Node,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Content each peer is known to have.
    known: HashMap<Fingerprint, Digest>,
    /// Peers with a push in flight, and whether another push was asked for
    /// meanwhile (with the digest to skip besides `known`).
    pushing: HashMap<Fingerprint, Option<Option<Digest>>>,
    /// Peers we are receiving a clipboard from.
    receiving: HashSet<Fingerprint>,
    /// Peers controlling us: resolved with their address when they leave.
    sessions: HashMap<Fingerprint, oneshot::Sender<SocketAddr>>,
    /// Our bulk connection to each peer.
    bulk: HashMap<Fingerprint, quinn::Connection>,
}

impl ClipSync {
    pub fn new(cb: Clipboard, node: Node) -> Arc<Self> {
        Arc::new(Self { cb, node, state: Mutex::default() })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("poisoned")
    }

    /// The pointer moved to `fp` (reachable at `addr`): send our clipboard
    /// if it changed since `fp` last had it.
    pub fn push(self: &Arc<Self>, fp: Fingerprint, name: &str, addr: SocketAddr) {
        self.spawn_push(fp, name.to_owned(), addr, None);
    }

    /// `fp` started controlling us. Remembers what our clipboard holds, so
    /// we can tell on `left` whether the user copied something here.
    pub fn entered(self: &Arc<Self>, fp: Fingerprint, name: &str) {
        let (tx, rx) = oneshot::channel();
        self.state().sessions.insert(fp, tx); // replaces (and ends) any earlier session
        let (this, name) = (self.clone(), name.to_owned());
        tokio::spawn(async move {
            let baseline = this.cb.digest().await.ok().flatten();
            if let Ok(addr) = rx.await {
                this.spawn_push(fp, name, addr, baseline);
            }
        });
    }

    /// `fp` (reachable at `addr`) stopped controlling us.
    pub fn left(&self, fp: Fingerprint, addr: SocketAddr) {
        if let Some(tx) = self.state().sessions.remove(&fp) {
            let _ = tx.send(addr);
        }
    }

    /// The link to `fp` closed; its clipboard may be gone too.
    pub fn forget(&self, fp: Fingerprint) {
        let mut s = self.state();
        s.known.remove(&fp);
        s.sessions.remove(&fp);
        if let Some(conn) = s.bulk.remove(&fp) {
            conn.close(0u32.into(), b"bye");
        }
    }

    /// Starts a push to `fp`, or, if one is in flight, marks it to run
    /// again afterwards so a change made meanwhile is not lost.
    fn spawn_push(self: &Arc<Self>, fp: Fingerprint, name: String, addr: SocketAddr, also_skip: Option<Digest>) {
        {
            let mut s = self.state();
            if let Some(again) = s.pushing.get_mut(&fp) {
                *again = Some(also_skip);
                return;
            }
            s.pushing.insert(fp, None);
        }
        let this = self.clone();
        tokio::spawn(async move {
            let mut also_skip = also_skip;
            loop {
                this.push_once(fp, &name, addr, also_skip).await;
                let mut s = this.state();
                match s.pushing.get_mut(&fp).and_then(Option::take) {
                    Some(next) => also_skip = next,
                    None => {
                        s.pushing.remove(&fp);
                        break;
                    }
                }
            }
        });
    }

    async fn push_once(&self, fp: Fingerprint, name: &str, addr: SocketAddr, also_skip: Option<Digest>) {
        let skip: Vec<Digest> = self.state().known.get(&fp).copied().into_iter().chain(also_skip).collect();
        match self.cb.read(skip).await {
            Ok(Read::Changed { digest, data }) => {
                self.state().known.insert(fp, digest);
                match self.send(fp, addr, &data).await {
                    Ok(()) => tracing::debug!(peer = %name, bytes = data.len(), "sent clipboard"),
                    Err(e) => {
                        tracing::debug!(peer = %name, "sending clipboard: {e:#}");
                        self.state().known.remove(&fp);
                    }
                }
            }
            Ok(Read::TooLarge { digest, size }) => {
                tracing::warn!(peer = %name, "clipboard is too large to share ({} MB, limit {} MB); skipped",
                    size >> 20, MAX_BYTES >> 20);
                self.state().known.insert(fp, digest);
            }
            Ok(Read::Unchanged(_) | Read::Empty) => {}
            Err(e) => tracing::warn!("reading the clipboard: {e:#}"),
        }
    }

    /// Our bulk connection to `fp`, dialed if there is none. The peer tells
    /// it from a link by its first stream (a clipboard stream).
    async fn bulk(&self, fp: Fingerprint, addr: SocketAddr) -> anyhow::Result<quinn::Connection> {
        if let Some(conn) = self.state().bulk.get(&fp).filter(|c| c.close_reason().is_none()) {
            return Ok(conn.clone());
        }
        let peer = self.node.connect_timeout(addr, DIAL_TIMEOUT).await?;
        if peer.fingerprint != fp {
            peer.conn.close(1u32.into(), b"wrong peer");
            bail!("{addr} is a different device now");
        }
        self.state().bulk.insert(fp, peer.conn.clone());
        Ok(peer.conn)
    }

    async fn send(&self, fp: Fingerprint, addr: SocketAddr, data: &ClipData) -> anyhow::Result<()> {
        let conn = self.bulk(fp, addr).await?;
        let (mut send, _recv) = open_stream(&conn, StreamKind::Clipboard).await?;
        let parts: Vec<(ClipFormat, &[u8])> = [
            (ClipFormat::Text, data.text.as_deref().map(str::as_bytes)),
            (ClipFormat::Html, data.html.as_deref().map(str::as_bytes)),
            (ClipFormat::Png, data.png.as_deref()),
        ]
        .into_iter()
        .filter_map(|(f, b)| Some((f, b?)))
        .collect();
        let header = ClipHeader { items: parts.iter().map(|(f, b)| (*f, b.len() as u32)).collect() };
        let mut buf = Vec::new();
        header.encode_frame(&mut buf);
        send.write_all(&buf).await?;
        for (_, b) in parts {
            send.write_all(b).await?;
        }
        send.finish()?;
        Ok(())
    }

    /// Handles an incoming clipboard stream from `fp`. One at a time per
    /// peer; a second one while the first is still arriving is refused.
    pub async fn receive(&self, fp: Fingerprint, name: &str, mut recv: quinn::RecvStream) {
        if !self.state().receiving.insert(fp) {
            tracing::debug!(peer = %name, "clipboard already arriving; refused another");
            let _ = recv.stop(1u32.into());
            return;
        }
        let res = tokio::time::timeout(RECEIVE_TIMEOUT, self.try_receive(fp, &mut recv)).await;
        self.state().receiving.remove(&fp);
        match res.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out"))) {
            Ok(Some(bytes)) => tracing::debug!(peer = %name, bytes, "received clipboard"),
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(peer = %name, "receiving clipboard: {e:#}");
                let _ = recv.stop(1u32.into());
            }
        }
    }

    async fn try_receive(&self, fp: Fingerprint, recv: &mut quinn::RecvStream) -> anyhow::Result<Option<usize>> {
        let header = read_clip_header(recv).await?;
        let total = header.total();
        if total > MAX_BYTES as u64 {
            tracing::warn!("peer sent a {} MB clipboard (limit {} MB); ignored", total >> 20, MAX_BYTES >> 20);
            let _ = recv.stop(0u32.into());
            return Ok(None);
        }
        let mut data = ClipData::default();
        for (format, len) in header.items {
            let buf = read_exact(recv, len as usize).await?;
            match format {
                ClipFormat::Text => data.text = String::from_utf8(buf).ok(),
                ClipFormat::Html => data.html = String::from_utf8(buf).ok(),
                ClipFormat::Png => data.png = Some(buf),
            }
        }
        if data.is_empty() {
            return Ok(None);
        }
        let size = data.len();
        let digest = self.cb.write(data).await?;
        self.state().known.insert(fp, digest);
        Ok(Some(size))
    }
}

/// Reads `len` bytes, growing the buffer only as data actually arrives,
/// so a header that promises much and sends little costs nothing.
async fn read_exact(recv: &mut quinn::RecvStream, len: usize) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    while buf.len() < len {
        let chunk = recv.read_chunk(len - buf.len(), true).await?.context("clipboard stream ended early")?;
        buf.extend_from_slice(&chunk.bytes);
    }
    Ok(buf)
}
