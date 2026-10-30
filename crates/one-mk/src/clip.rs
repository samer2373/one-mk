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
//!
//! The user's settings (`settings.rs`) limit it: formats that are switched
//! off are neither sent nor accepted, a clipboard over the size limit is
//! not sent, and content a password manager marked as sensitive is not sent
//! by the automatic push. Two things go around the pointer, both asked for
//! on the command line: [`ClipSync::send_now`] pushes our clipboard to one
//! computer, and [`ClipSync::pull`] fetches one computer's clipboard. They
//! use the same clipboard stream; a pull is a header with the pull flag
//! that the peer answers on the same stream (see `ClipHeader`).

use anyhow::{Context, bail};
use one_mk_clipboard::{ClipData, Clipboard, Digest, Read};
use one_mk_net::{Fingerprint, Node, open_stream, read_clip_header};
use one_mk_proto::{ClipAck, ClipFormat, ClipHeader, StreamKind};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

const FETCH_DISABLED: u32 = 0x434c4950;
pub const FETCH_DISABLED_MESSAGE: &str = "does not allow fetching its clipboard";

/// The longest a peer may take to deliver one clipboard snapshot.
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(30);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a sender waits to hear what the receiver did with a clipboard.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ClipSync {
    cb: Clipboard,
    node: Node,
    state: Mutex<State>,
    /// Whether peers may fetch our clipboard.
    allow_pull: AtomicBool,
    history_dir: Mutex<Option<std::path::PathBuf>>,
}

/// What an explicit send or fetch moved.
#[derive(Debug, PartialEq, Eq)]
pub struct Moved {
    pub bytes: usize,
    /// "text", "text and html", "image"...
    pub what: String,
    /// What the receiver said it did; `None` if it didn't say (an older version).
    pub ack: Option<ClipAck>,
}

fn describe(data: &ClipData) -> String {
    let kinds: Vec<&str> = [(data.text.is_some(), "text"), (data.html.is_some(), "html"), (data.png.is_some(), "image")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    match kinds.as_slice() {
        [] => "nothing".to_owned(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// "12 B", "3 KB", "1.9 MB": binary units, like the size limit.
pub fn mb(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / f64::from(1 << 20))
    } else if bytes >= 1 << 10 {
        format!("{} KB", bytes.div_ceil(1 << 10))
    } else {
        format!("{bytes} B")
    }
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
        Arc::new(Self { cb, node, state: Mutex::default(), allow_pull: AtomicBool::new(false), history_dir: Mutex::default() })
    }

    pub fn set_history_dir(&self, dir: &std::path::Path) {
        *self.history_dir.lock().unwrap_or_else(|error| error.into_inner()) = Some(dir.to_owned());
        if one_mk_clipboard::history::clean_unfinished(dir).is_err() {
            self.cb.history().lock().unwrap_or_else(|error| error.into_inner()).error = Some(
                "Could not remove an unfinished clipboard-history save; check the settings folder is writable, then restart one-mk.".into()
            );
        }
    }

    pub fn clipboard(&self) -> &Clipboard { &self.cb }

    pub fn local_computer(&self) -> (&str, Fingerprint) { (&self.node.identity().name, self.node.identity().fingerprint) }

    #[cfg(test)]
    pub fn filter_for_test(&self) -> one_mk_clipboard::Filter {
        self.cb.filter()
    }

    /// Applies the user's clipboard settings, from now on.
    pub fn apply(&self, s: &crate::settings::ClipSettings) {
        self.cb.set_filter(s.filter());
        self.cb.history_skip_apps(s.history_skip_apps.clone());
        let path = self.history_dir.lock().unwrap_or_else(|error| error.into_inner()).as_ref().map(|dir| dir.join("clipboard-history.bin"));
        if !s.history_persist && let Some(path) = &path {
            match std::fs::remove_file(path) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => self.cb.history().lock().unwrap_or_else(|error| error.into_inner()).error = Some(
                    "Could not remove saved clipboard history; remove clipboard-history.bin from the settings folder.".into()
                ),
            }
        }
        self.cb.history().lock().unwrap_or_else(|error| error.into_inner()).configure(
            s.history_max, (s.history_max_mb.min(256) as usize) << 20, s.filter().max_bytes(), s.history_persist.then_some(path).flatten()
        );
        self.allow_pull.store(s.allow_pull, Ordering::Relaxed);
        // What was skipped (too large, sensitive) may be sendable now.
        self.state().known.clear();
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
                match self.send(fp, addr, &data, true).await {
                    Ok(None | Some(ClipAck::Stored)) => {
                        self.state().known.insert(fp, digest);
                        tracing::debug!(peer = %name, bytes = data.len(), "sent clipboard");
                    }
                    Ok(Some(ack)) => {
                        self.state().known.remove(&fp);
                        tracing::debug!(peer = %name, ?ack, "clipboard rejected; will retry on the next crossing");
                    }
                    Err(e) => {
                        tracing::debug!(peer = %name, "sending clipboard: {e:#}");
                        self.state().known.remove(&fp);
                    }
                }
            }
            Ok(Read::TooLarge { digest, size }) => {
                tracing::warn!(peer = %name, "clipboard is too large to share ({}, limit {}); skipped \
                    (raise the limit with `one-mk clipboard --max-mb`)", mb(size), mb(self.cb.filter().max_bytes()));
                self.state().known.insert(fp, digest);
            }
            Ok(Read::Concealed(digest)) => {
                tracing::debug!(peer = %name, "clipboard is marked sensitive; not shared");
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

    /// Sends `data` on a new clipboard stream. With `want_ack`, waits for
    /// the receiver's answer (`None`: it gave none, as older versions don't).
    async fn send(&self, fp: Fingerprint, addr: SocketAddr, data: &ClipData, want_ack: bool) -> anyhow::Result<Option<ClipAck>> {
        let conn = self.bulk(fp, addr).await?;
        let (mut send, mut recv) = open_stream(&conn, StreamKind::Clipboard).await?;
        let wrote = write_snapshot(&mut send, data).await;
        if !want_ack {
            return wrote.map(|()| None);
        }
        // A receiver that refuses early stops our stream, and still answers.
        let mut ack = [0u8; 1];
        // A closed stream with nothing on it is an older receiver, which never answers.
        match tokio::time::timeout(ACK_TIMEOUT, recv.read_exact(&mut ack)).await {
            Ok(Ok(())) => ClipAck::from_u8(ack[0]).map(Some).ok_or_else(|| anyhow::anyhow!("unknown clipboard acknowledgement {}", ack[0])),
            Ok(Err(_)) | Err(_) => wrote.map(|()| None),
        }
    }

    /// Pushes our clipboard to `fp` now, whether or not it changed since
    /// we last did and without the pointer crossing. The user asked for
    /// this copy to be sent, so it goes even if it is marked sensitive;
    /// the formats and size limit still apply.
    pub async fn send_now(self: &Arc<Self>, fp: Fingerprint, addr: SocketAddr) -> anyhow::Result<Moved> {
        match self.cb.read_explicit(Vec::new()).await.context("reading the clipboard")? {
            Read::Changed { digest, data } => {
                let ack = self.send(fp, addr, &data, true).await?;
                if matches!(ack, None | Some(ClipAck::Stored)) {
                    self.state().known.insert(fp, digest);
                }
                Ok(Moved { bytes: data.len(), what: describe(&data), ack })
            }
            Read::TooLarge { size, .. } => bail!(
                "your clipboard is {}, over the limit of {}; raise it with `one-mk clipboard --max-mb N`",
                mb(size),
                mb(self.cb.filter().max_bytes())
            ),
            Read::Empty | Read::Unchanged(_) | Read::Concealed(_) => bail!(
                "your clipboard is empty, or holds only formats that are switched off (see `one-mk clipboard`)"
            ),
        }
    }

    /// Fetches the clipboard of `fp` into ours.
    pub async fn pull(self: &Arc<Self>, fp: Fingerprint, addr: SocketAddr) -> anyhow::Result<Moved> {
        let conn = self.bulk(fp, addr).await?;
        let (mut send, mut recv) = open_stream(&conn, StreamKind::Clipboard).await?;
        let mut buf = Vec::new();
        ClipHeader::pull_request().encode_frame(&mut buf);
        send.write_all(&buf).await?;
        send.finish()?;
        let got = tokio::time::timeout(RECEIVE_TIMEOUT, async {
            let header = read_clip_header(&mut recv).await.map_err(|error| {
                if error.chain().any(|cause| cause.downcast_ref::<quinn::ReadExactError>().is_some_and(|error| matches!(error, quinn::ReadExactError::ReadError(quinn::ReadError::Reset(code)) if *code == quinn::VarInt::from_u32(FETCH_DISABLED)))) {
                    anyhow::anyhow!(FETCH_DISABLED_MESSAGE)
                } else {
                    anyhow::anyhow!("it did not answer: it may be running an older one-mk, or not sharing its clipboard ({error:#})")
                }
            })?;
            self.read_snapshot(fp, &self.peer_name(fp), &header, &mut recv).await
        })
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")))?;
        match got {
            Received::Stored { bytes, what } => Ok(Moved { bytes, what, ack: Some(ClipAck::Stored) }),
            Received::Nothing => bail!("its clipboard is empty, or it does not share it (`one-mk clipboard` there)"),
            Received::FormatsOff => bail!("what it has is switched off here (see `one-mk clipboard`)"),
            Received::TooLarge(size) => bail!(
                "its clipboard is {}, over the limit of {} here; raise it with `one-mk clipboard --max-mb N`",
                mb(size),
                mb(self.cb.filter().max_bytes())
            ),
        }
    }

    /// Handles an incoming clipboard stream from `fp`: a snapshot to store
    /// (answered with a [`ClipAck`]) or a pull (answered with our clipboard).
    /// One at a time per peer; a second one while the first is still
    /// arriving is refused.
    pub async fn receive(&self, fp: Fingerprint, name: &str, mut send: quinn::SendStream, mut recv: quinn::RecvStream) {
        if !self.state().receiving.insert(fp) {
            tracing::debug!(peer = %name, "clipboard already arriving; refused another");
            let _ = recv.stop(1u32.into());
            return;
        }
        let res = tokio::time::timeout(RECEIVE_TIMEOUT, self.try_receive(fp, name, &mut send, &mut recv)).await;
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

    async fn try_receive(
        &self,
        fp: Fingerprint,
        name: &str,
        send: &mut quinn::SendStream,
        recv: &mut quinn::RecvStream,
    ) -> anyhow::Result<Option<usize>> {
        let header = read_clip_header(recv).await?;
        if header.pull {
            self.answer_pull(name, send).await?;
            return Ok(None);
        }
        let (ack, stored) = match self.read_snapshot(fp, name, &header, recv).await? {
            Received::Stored { bytes, .. } => (Some(ClipAck::Stored), Some(bytes)),
            Received::FormatsOff => (Some(ClipAck::FormatsOff), None),
            Received::TooLarge(_) => (Some(ClipAck::TooLarge), None),
            Received::Nothing => (None, None),
        };
        // Tell the sender what became of it; it may have stopped listening, which is fine.
        if let Some(ack) = ack
            && send.write_all(&[ack as u8]).await.is_ok()
        {
            let _ = send.finish();
        }
        Ok(stored)
    }

    /// `name` asked for our clipboard: send it back on this stream, or an
    /// empty snapshot if there is nothing we may share (or sharing is off).
    /// Sensitive content is never handed out this way: only we can decide to
    /// send that.
    async fn answer_pull(&self, name: &str, send: &mut quinn::SendStream) -> anyhow::Result<()> {
        if !self.allow_pull.load(Ordering::Relaxed) {
            send.reset(FETCH_DISABLED.into())?;
            return Ok(());
        }
        let data = if self.allow_pull.load(Ordering::Relaxed) {
            match self.cb.read(Vec::new()).await? {
                Read::Changed { data, .. } => Some(data),
                _ => None,
            }
        } else {
            None
        };
        match &data {
            Some(d) => println!("{name} fetched this computer's clipboard ({}).", describe(d)),
            None => tracing::debug!(peer = %name, "pull answered with nothing"),
        }
        write_snapshot(send, data.as_deref().unwrap_or(&ClipData::default())).await
    }

    /// Reads the payloads `header` announces and puts what the settings
    /// allow on the clipboard, remembering that `fp` has it.
    async fn read_snapshot(&self, fp: Fingerprint, name: &str, header: &ClipHeader, recv: &mut quinn::RecvStream) -> anyhow::Result<Received> {
        let filter = self.cb.filter();
        let wanted = |f: ClipFormat| match f {
            ClipFormat::Text => filter.text,
            ClipFormat::Html => filter.html,
            ClipFormat::Png => filter.image,
        };
        if header.items.is_empty() {
            return Ok(Received::Nothing);
        }
        let allowed: u64 = header.items.iter().filter(|(f, _)| wanted(*f)).map(|(_, n)| u64::from(*n)).sum();
        if allowed == 0 {
            let _ = recv.stop(0u32.into());
            return Ok(Received::FormatsOff);
        }
        let max = filter.max_bytes();
        if allowed > max as u64 {
            tracing::warn!("peer sent a {} clipboard (limit {}); ignored", mb(allowed as usize), mb(max));
            let _ = recv.stop(0u32.into());
            return Ok(Received::TooLarge(allowed as usize));
        }
        let last = header.items.iter().rposition(|(f, _)| wanted(*f)).expect("allowed > 0");
        let mut data = ClipData::default();
        for (format, len) in &header.items[..=last] {
            if !wanted(*format) {
                skip_exact(recv, *len as usize).await?;
                continue;
            }
            let buf = read_exact(recv, *len as usize).await?;
            match format {
                ClipFormat::Text => data.text = String::from_utf8(buf).ok(),
                ClipFormat::Html => data.html = String::from_utf8(buf).ok(),
                ClipFormat::Png => data.png = Some(buf),
            }
        }
        // Whatever is left is formats we don't take: not worth reading.
        let _ = recv.stop(0u32.into());
        if data.is_empty() {
            return Ok(Received::Nothing);
        }
        let (bytes, what) = (data.len(), describe(&data));
        let digest = self.cb.write_from(data, name.into(), true).await?;
        self.state().known.insert(fp, digest);
        Ok(Received::Stored { bytes, what })
    }

    fn peer_name(&self, fp: Fingerprint) -> String {
        self.history_dir.lock().unwrap_or_else(|error| error.into_inner()).as_ref()
            .and_then(|dir| one_mk_net::TrustStore::load(dir).ok())
            .and_then(|trust| trust.devices().iter().find(|device| device.fingerprint() == Some(fp)).map(|device| device.name.clone()))
            .unwrap_or_else(|| fp.short())
    }
}

/// What came of a snapshot read off a stream.
enum Received {
    Stored { bytes: usize, what: String },
    /// An empty snapshot.
    Nothing,
    /// Every format in it is switched off.
    FormatsOff,
    TooLarge(usize),
}

/// A snapshot as one header and the payloads, then the end of the stream.
async fn write_snapshot(send: &mut quinn::SendStream, data: &ClipData) -> anyhow::Result<()> {
    let parts: Vec<(ClipFormat, &[u8])> = [
        (ClipFormat::Text, data.text.as_deref().map(str::as_bytes)),
        (ClipFormat::Html, data.html.as_deref().map(str::as_bytes)),
        (ClipFormat::Png, data.png.as_deref()),
    ]
    .into_iter()
    .filter_map(|(f, b)| Some((f, b?)))
    .collect();
    let header = ClipHeader { items: parts.iter().map(|(f, b)| (*f, b.len() as u32)).collect(), pull: false };
    let mut buf = Vec::new();
    header.encode_frame(&mut buf);
    send.write_all(&buf).await?;
    for (_, b) in parts {
        send.write_all(b).await?;
    }
    send.finish()?;
    Ok(())
}

/// Reads and drops `len` bytes.
async fn skip_exact(recv: &mut quinn::RecvStream, len: usize) -> anyhow::Result<()> {
    let mut left = len;
    while left > 0 {
        let chunk = recv.read_chunk(left, true).await?.context("clipboard stream ended early")?;
        left -= chunk.bytes.len();
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejected_automatic_push_retries_the_same_clipboard() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve();
        text(&a.mem, "unchanged clipboard");
        b.settings(ClipSettings { text: false, ..Default::default() });
        a.clip.push_once(b.fp(), "b", b.addr(), None).await;
        assert!(!a.clip.state().known.contains_key(&b.fp()));
        b.settings(ClipSettings::default());
        a.clip.push_once(b.fp(), "b", b.addr(), None).await;
        eventually("retry accepted", || b.mem.text().as_deref() == Some("unchanged clipboard")).await;
    }

    #[tokio::test]
    async fn unknown_acknowledgement_is_not_cached_as_delivered() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve_ack(Some(255));
        text(&a.mem, "unchanged clipboard");
        a.clip.push_once(b.fp(), "b", b.addr(), None).await;
        assert!(!a.clip.state().known.contains_key(&b.fp()));
    }
    use crate::app::Services;
    use crate::settings::ClipSettings;
    use crate::transfer::Inbox;
    use one_mk_clipboard::{MemoryClipboard, Raw};
    use one_mk_net::Identity;

    /// A computer: a node on loopback, a clipboard the "user" works on, and
    /// the services that answer a peer's bulk connection.
    struct Pc {
        node: Node,
        clip: Arc<ClipSync>,
        mem: MemoryClipboard,
    }

    impl Pc {
        fn new() -> Pc {
            let (cb, mem) = Clipboard::memory();
            let node = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
            Pc { clip: ClipSync::new(cb, node.clone()), node, mem }
        }

        fn fp(&self) -> Fingerprint {
            self.node.identity().fingerprint
        }

        fn addr(&self) -> SocketAddr {
            self.node.local_addr().unwrap()
        }

        fn settings(&self, s: ClipSettings) {
            self.clip.apply(&s);
        }

        /// Serves bulk connections like the daemon does (`app::serve_streams`).
        fn serve(&self) {
            let (node, clip) = (self.node.clone(), self.clip.clone());
            let inbox = Inbox::new(std::env::temp_dir().join("one-mk-clip-test-unused"));
            tokio::spawn(async move {
                while let Some(Ok(peer)) = node.accept().await {
                    // Every peer of the test counts as paired.
                    let trusted = tokio::sync::watch::channel(std::collections::HashSet::from([peer.fingerprint])).0;
                    let services = Services { clip: Some(clip.clone()), inbox: inbox.clone(), trusted };
                    tokio::spawn(async move {
                        let fp = peer.fingerprint;
                        if let Ok(first) = peer.accept_stream().await {
                            crate::app::serve_streams(peer.conn, fp, "peer".into(), services, first).await;
                        }
                    });
                }
            });
        }

        /// Serves like a version from before acks and pulls: reads a snapshot,
        /// answers nothing.
        fn serve_old(&self) {
            self.serve_ack(None);
        }

        fn serve_ack(&self, acknowledgement: Option<u8>) {
            let node = self.node.clone();
            tokio::spawn(async move {
                while let Some(Ok(peer)) = node.accept().await {
                    tokio::spawn(async move {
                        while let Ok((_, mut send, mut recv)) = peer.accept_stream().await {
                            if let Ok(h) = read_clip_header(&mut recv).await {
                                let _ = read_exact(&mut recv, h.total() as usize).await;
                                if let Some(acknowledgement) = acknowledgement {
                                    let _ = send.write_all(&[acknowledgement]).await;
                                    let _ = send.finish();
                                }
                            }
                            drop(send); // never answers
                        }
                    });
                }
            });
        }
    }

    async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn text(mem: &MemoryClipboard, t: &str) {
        mem.set(Raw { text: Some(t.into()), ..Raw::default() });
    }

    #[tokio::test]
    async fn clip_send_reaches_only_the_chosen_computer_without_the_pointer() {
        let (a, b, c) = (Pc::new(), Pc::new(), Pc::new());
        b.serve();
        c.serve();
        text(&a.mem, "for b only");
        let moved = a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        assert_eq!(moved, Moved { bytes: 10, what: "text".into(), ack: Some(ClipAck::Stored) });
        assert_eq!(b.mem.text().as_deref(), Some("for b only"));
        assert_eq!(c.mem.writes(), 0, "the other computer is left alone");

        // Even if it is the same content as last time: asking again sends again.
        b.mem.set(Raw::default());
        a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        assert_eq!(b.mem.text().as_deref(), Some("for b only"));
        assert_eq!(b.mem.writes(), 2);
        // ... and it is remembered as B's, so the pointer crossing doesn't send it a third time.
        a.clip.push(b.fp(), "b", b.addr());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(b.mem.writes(), 2);
    }

    #[tokio::test]
    async fn clip_send_says_why_nothing_was_sent_or_taken() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve();
        let err = |e: anyhow::Error| format!("{e:#}");

        assert!(err(a.clip.send_now(b.fp(), b.addr()).await.unwrap_err()).contains("empty"));

        // Over our own limit: not even sent.
        text(&a.mem, &"x".repeat(2 << 20));
        a.settings(ClipSettings { max_mb: 1, ..Default::default() });
        let e = err(a.clip.send_now(b.fp(), b.addr()).await.unwrap_err());
        assert!(e.contains("2.0 MB") && e.contains("1.0 MB") && e.contains("--max-mb"), "{e}");
        assert_eq!(b.mem.writes(), 0);

        // Over the receiver's limit: sent, refused, and we are told.
        a.settings(ClipSettings::default());
        b.settings(ClipSettings { max_mb: 1, ..Default::default() });
        let m = a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        assert_eq!(m.ack, Some(ClipAck::TooLarge));
        assert_eq!(b.mem.writes(), 0);

        // A format the receiver has switched off.
        b.settings(ClipSettings { image: false, ..Default::default() });
        a.mem.set(Raw {
            image: Some(one_mk_clipboard::Image { width: 1, height: 1, rgba: vec![1, 2, 3, 255] }),
            ..Raw::default()
        });
        let m = a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        assert_eq!(m.ack, Some(ClipAck::FormatsOff));
        assert_eq!(b.mem.writes(), 0);

        // We have it switched off: nothing to send.
        a.settings(ClipSettings { image: false, ..Default::default() });
        assert!(err(a.clip.send_now(b.fp(), b.addr()).await.unwrap_err()).contains("switched off"));
    }

    #[tokio::test]
    async fn the_receiver_keeps_only_the_formats_it_allows() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve();
        a.mem.set(Raw { text: Some("plain".into()), html: Some("<b>rich</b>".into()), ..Raw::default() });
        b.settings(ClipSettings { html: false, ..Default::default() });
        let m = a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        assert_eq!(m.ack, Some(ClipAck::Stored));
        let got = b.mem.get();
        assert_eq!((got.text.as_deref(), got.html.as_deref()), (Some("plain"), None));

        // And a sender doesn't offer what it has switched off.
        a.settings(ClipSettings { text: false, ..Default::default() });
        b.settings(ClipSettings::default());
        a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        let got = b.mem.get();
        assert_eq!((got.text.as_deref(), got.html.as_deref()), (None, Some("<b>rich</b>")));
    }

    #[tokio::test]
    async fn clip_get_fetches_that_computers_clipboard() {
        let (a, b) = (Pc::new(), Pc::new());
        b.settings(ClipSettings { allow_pull: true, ..Default::default() });
        b.serve();
        text(&b.mem, "from b");
        let m = a.clip.pull(b.fp(), b.addr()).await.unwrap();
        assert_eq!((m.what.as_str(), m.bytes), ("text", 6));
        assert_eq!(a.mem.text().as_deref(), Some("from b"));
        assert_eq!(a.clip.clipboard().history().lock().unwrap().list()[0].data.text.as_deref(), Some("from b"));
        assert_eq!(b.mem.writes(), 0, "B was only read");
        // A's pointer crossing to B later does not send it back.
        b.clip.push(a.fp(), "a", a.addr());
        a.clip.push(b.fp(), "b", b.addr());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(b.mem.writes(), 0, "A knows B has this content");
    }

    #[tokio::test]
    async fn clip_get_respects_the_owners_and_the_askers_settings() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve();
        let err = |e: anyhow::Error| format!("{e:#}");
        text(&b.mem, "secret-ish");

        // The owner doesn't let it be fetched.
        b.settings(ClipSettings { allow_pull: false, ..Default::default() });
        assert!(err(a.clip.pull(b.fp(), b.addr()).await.unwrap_err()).contains(FETCH_DISABLED_MESSAGE));
        assert_eq!(a.mem.writes(), 0);

        // A password manager's copy is never handed out on request.
        b.settings(ClipSettings { allow_pull: true, ..Default::default() });
        b.mem.set_concealed(true);
        assert!(err(a.clip.pull(b.fp(), b.addr()).await.unwrap_err()).contains("empty, or it does not share"));
        b.mem.set_concealed(false);

        // Over the owner's size limit: nothing comes.
        text(&b.mem, &"y".repeat(2 << 20));
        b.settings(ClipSettings { max_mb: 1, allow_pull: true, ..Default::default() });
        assert!(a.clip.pull(b.fp(), b.addr()).await.is_err());
        b.settings(ClipSettings { allow_pull: true, ..Default::default() });

        // The asker has text switched off: what comes is refused here.
        a.settings(ClipSettings { text: false, ..Default::default() });
        text(&b.mem, "plain");
        assert!(err(a.clip.pull(b.fp(), b.addr()).await.unwrap_err()).contains("switched off here"));
        assert_eq!(a.mem.writes(), 0);

        // The asker's size limit.
        a.settings(ClipSettings { max_mb: 1, ..Default::default() });
        text(&b.mem, &"z".repeat(2 << 20));
        assert!(err(a.clip.pull(b.fp(), b.addr()).await.unwrap_err()).contains("over the limit"));
        assert_eq!(a.mem.writes(), 0);
    }

    #[tokio::test]
    async fn an_older_peer_neither_confirms_a_send_nor_answers_a_pull() {
        let (a, old) = (Pc::new(), Pc::new());
        old.serve_old();
        text(&a.mem, "hi");
        let m = a.clip.send_now(old.fp(), old.addr()).await.unwrap();
        assert_eq!(m.ack, None, "sent, but it can't say what it did");

        let e = format!("{:#}", a.clip.pull(old.fp(), old.addr()).await.unwrap_err());
        assert!(e.contains("older one-mk"), "{e}");
    }

    #[tokio::test]
    async fn the_automatic_push_honours_formats_and_sensitive_content() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve();
        a.mem.set(Raw { text: Some("t".into()), html: Some("<i>t</i>".into()), ..Raw::default() });
        a.settings(ClipSettings { html: false, ..Default::default() });
        a.clip.push(b.fp(), "b", b.addr());
        eventually("B to get the text", || b.mem.text().as_deref() == Some("t")).await;
        assert_eq!(b.mem.get().html, None, "html is switched off at the sender");

        // A password manager's copy stays put when the pointer crosses...
        text(&a.mem, "hunter2");
        a.mem.set_concealed(true);
        a.clip.push(b.fp(), "b", b.addr());
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(b.mem.text().as_deref(), Some("t"));
        // ... unless the user turns the skip off.
        a.settings(ClipSettings { skip_sensitive: false, ..Default::default() });
        a.clip.push(b.fp(), "b", b.addr());
        eventually("B to get it once skipping is off", || b.mem.text().as_deref() == Some("hunter2")).await;
    }

    #[tokio::test]
    async fn explicit_send_overrides_the_sensitive_skip() {
        let (a, b) = (Pc::new(), Pc::new());
        b.serve();
        text(&a.mem, "my wifi password");
        a.mem.set_concealed(true);
        a.clip.send_now(b.fp(), b.addr()).await.unwrap();
        assert_eq!(b.mem.text().as_deref(), Some("my wifi password"));
    }

    #[test]
    fn what_was_moved_reads_well() {
        let d = |t: bool, h: bool, i: bool| ClipData {
            text: t.then(|| "x".into()),
            html: h.then(|| "x".into()),
            png: i.then(|| vec![1]),
        };
        assert_eq!(describe(&d(true, false, false)), "text");
        assert_eq!(describe(&d(true, true, false)), "text and html");
        assert_eq!(describe(&d(true, true, true)), "text, html and image");
        assert_eq!(describe(&d(false, false, false)), "nothing");
        assert_eq!(mb(5 << 20), "5.0 MB");
        assert_eq!(mb(1500), "2 KB");
        assert_eq!(mb(12), "12 B");
    }
}
