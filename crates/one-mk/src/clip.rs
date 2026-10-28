//! Clipboard sharing.
//!
//! - When the pointer moves to a peer, we send our clipboard to it, unless
//!   it already has that content.
//! - When a peer's pointer leaves us, we send our clipboard back to it, but
//!   only if the user copied something here while controlling us.
//!
//! Every clipboard snapshot goes on its own low-priority stream, and all
//! clipboard access happens on the clipboard thread, so neither ever
//! delays input. Echoes are prevented by remembering, per peer, the digest
//! of the content it is known to have (sent to it or received from it).

use one_mk_clipboard::{ClipData, Clipboard, Digest, MAX_BYTES, Read};
use one_mk_net::{Fingerprint, open_stream, read_clip_header};
use one_mk_proto::{ClipFormat, ClipHeader, StreamKind};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

pub struct ClipSync {
    cb: Clipboard,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Content each peer is known to have.
    known: HashMap<Fingerprint, Digest>,
    /// Peers we are reading the clipboard for, to send it.
    pushing: HashSet<Fingerprint>,
    /// Peers controlling us: resolved with their connection when they leave.
    sessions: HashMap<Fingerprint, oneshot::Sender<quinn::Connection>>,
}

impl ClipSync {
    pub fn new(cb: Clipboard) -> Arc<Self> {
        Arc::new(Self { cb, state: Mutex::default() })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("poisoned")
    }

    /// The pointer moved to `fp`: send our clipboard if it changed since
    /// `fp` last had it.
    pub fn push(self: &Arc<Self>, fp: Fingerprint, name: &str, conn: quinn::Connection) {
        self.spawn_push(fp, name.to_owned(), conn, None);
    }

    /// `fp` started controlling us. Remembers what our clipboard holds, so
    /// we can tell on `left` whether the user copied something here.
    pub fn entered(self: &Arc<Self>, fp: Fingerprint, name: &str) {
        let (tx, rx) = oneshot::channel();
        self.state().sessions.insert(fp, tx); // replaces (and ends) any earlier session
        let (this, name) = (self.clone(), name.to_owned());
        tokio::spawn(async move {
            let baseline = this.cb.digest().await.ok().flatten();
            if let Ok(conn) = rx.await {
                this.spawn_push(fp, name, conn, baseline);
            }
        });
    }

    /// `fp` stopped controlling us.
    pub fn left(&self, fp: Fingerprint, conn: quinn::Connection) {
        if let Some(tx) = self.state().sessions.remove(&fp) {
            let _ = tx.send(conn);
        }
    }

    /// The link to `fp` closed; its clipboard may be gone too.
    pub fn forget(&self, fp: Fingerprint) {
        let mut s = self.state();
        s.known.remove(&fp);
        s.sessions.remove(&fp);
    }

    fn spawn_push(self: &Arc<Self>, fp: Fingerprint, name: String, conn: quinn::Connection, also_skip: Option<Digest>) {
        let skip: Vec<Digest> = {
            let mut s = self.state();
            if !s.pushing.insert(fp) {
                return;
            }
            s.known.get(&fp).copied().into_iter().chain(also_skip).collect()
        };
        let this = self.clone();
        tokio::spawn(async move {
            match this.cb.read(skip).await {
                Ok(Read::Changed { digest, data }) => {
                    this.state().known.insert(fp, digest);
                    match send(&conn, &data).await {
                        Ok(()) => tracing::debug!(peer = %name, bytes = data.len(), "sent clipboard"),
                        Err(e) => {
                            tracing::debug!(peer = %name, "sending clipboard: {e:#}");
                            this.state().known.remove(&fp);
                        }
                    }
                }
                Ok(Read::TooLarge { digest, size }) => {
                    tracing::info!(peer = %name, "clipboard is too large to share ({} MB, limit {} MB); skipped",
                        size >> 20, MAX_BYTES >> 20);
                    this.state().known.insert(fp, digest);
                }
                Ok(Read::Unchanged(_) | Read::Empty) => {}
                Err(e) => tracing::warn!("reading the clipboard: {e:#}"),
            }
            this.state().pushing.remove(&fp);
        });
    }

    /// Handles an incoming clipboard stream from `fp`.
    pub async fn receive(&self, fp: Fingerprint, name: &str, mut recv: quinn::RecvStream) {
        match self.try_receive(fp, &mut recv).await {
            Ok(Some(bytes)) => tracing::debug!(peer = %name, bytes, "received clipboard"),
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(peer = %name, "receiving clipboard: {e:#}");
                let _ = recv.stop(1u32.into());
            }
        }
    }

    async fn try_receive(&self, fp: Fingerprint, recv: &mut quinn::RecvStream) -> anyhow::Result<Option<usize>> {
        let header = tokio::time::timeout(Duration::from_secs(10), read_clip_header(recv)).await??;
        let total = header.total();
        if total > MAX_BYTES as u64 {
            tracing::info!("peer sent a {} MB clipboard (limit {} MB); ignored", total >> 20, MAX_BYTES >> 20);
            let _ = recv.stop(0u32.into());
            return Ok(None);
        }
        let mut data = ClipData::default();
        for (format, len) in header.items {
            let mut buf = vec![0u8; len as usize];
            recv.read_exact(&mut buf).await?;
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

async fn send(conn: &quinn::Connection, data: &ClipData) -> anyhow::Result<()> {
    let (mut send, _recv) = open_stream(conn, StreamKind::Clipboard).await?;
    // Below the control stream (priority 0), so input never waits behind it.
    let _ = send.set_priority(-1);
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
