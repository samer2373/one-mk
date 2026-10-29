//! `one-mk send`: send files or folders to a paired computer running
//! `one-mk run`.
//!
//! This uses the same identity as the service on this machine, so the
//! receiver knows it is trusted. It opens its own connection whose first
//! stream is a file stream, which the receiver treats as transfer-only:
//! the service's mouse/keyboard link is not touched.

use crate::control::{self, Request};
use crate::track::Rate;
use crate::transfer::{self, Outcome, Outgoing, Progress};
use one_mk_ctl::status::TransferStatus;
use anyhow::{Context, Result, bail};
use one_mk_net::{Device, Discovery, Fingerprint, Identity, Node, Peer, TrustStore};
use std::collections::HashSet;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// How long to look for the device before giving up.
const FIND_TIMEOUT: Duration = Duration::from_secs(6);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long to wait before asking again when the receiver is still counting our last connection.
const BUSY_RETRY: Duration = Duration::from_millis(1500);
/// How long to wait for the close notice to leave when finishing.
const CLOSE_WAIT: Duration = Duration::from_millis(500);

pub async fn run(dir: &Path, paths: Vec<PathBuf>, to: Option<String>, connect: Vec<SocketAddr>) -> Result<()> {
    for p in &paths {
        if std::fs::symlink_metadata(p).is_err() {
            bail!("{}: no such file or folder", p.display());
        }
    }
    let files = transfer::collect(&paths)?;
    if files.is_empty() {
        bail!("nothing to send (the folders are empty)");
    }
    let total = files.iter().fold(0u64, |total, file| total.saturating_add(file.size));
    if files.len() > 10_000 || total > 1 << 40 {
        bail!("file batch exceeds the limit of 10000 files or 1 TiB");
    }
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let trust = TrustStore::load(dir)?;
    let device = pick(&trust, to.as_deref())?.clone();
    let fp = device.fingerprint().context("corrupt fingerprint in devices.toml")?;

    let start = Instant::now();
    let tried = Attempt { dir, identity: &identity, device: &device, fp, connect: &connect };
    let (first, mut sent) = tried.send(files, false).await?;
    // The receiver counts a connection until it hears it closed, and the one
    // before this may still be on its way out: ask again once.
    let (busy, mut results): (Vec<_>, Vec<_>) =
        first.into_iter().partition(|(_, o)| matches!(o, Outcome::Failed(why) if transfer::is_busy(why)));
    if !busy.is_empty() {
        tokio::time::sleep(BUSY_RETRY).await;
        let (again, more) = tried.send(busy.into_iter().map(|(f, _)| f).collect(), true).await?;
        results.extend(again);
        sent = sent.saturating_add(more);
    }

    let elapsed = start.elapsed().as_secs_f64();
    eprint!("\r\x1b[2K");
    let mut failed = 0;
    let (mut saved, mut saved_bytes) = (0, 0u64);
    for (file, outcome) in &results {
        match outcome {
            Outcome::Saved { resumed_from, .. } => {
                saved += 1;
                saved_bytes = saved_bytes.saturating_add(file.size);
                if *resumed_from > 0 {
                    eprintln!("  resumed {} from {}", file.rel, bytes(*resumed_from));
                }
            }
            Outcome::Rejected(why) | Outcome::Failed(why) => {
                failed += 1;
                eprintln!("  not sent: {}: {why}", file.rel);
            }
        }
    }
    eprintln!(
        "Sent {} to {} in {:.1} s ({}/s)",
        summary(saved, saved_bytes),
        device.name,
        elapsed,
        bytes((sent as f64 / elapsed.max(1e-6)) as u64)
    );
    if failed > 0 {
        bail!("{failed} file(s) were not sent");
    }
    Ok(())
}

/// What one try at sending needs.
struct Attempt<'a> {
    dir: &'a Path,
    identity: &'a Arc<Identity>,
    device: &'a Device,
    fp: Fingerprint,
    connect: &'a [SocketAddr],
}

impl Attempt<'_> {
    /// Connects and sends `files`. Returns each file's outcome and the bytes
    /// that crossed the network.
    async fn send(&self, files: Vec<Outgoing>, retry: bool) -> Result<(Vec<(Outgoing, Outcome)>, u64)> {
        let node = Node::bind_any(self.identity.clone(), 0)?;
        let peer = find(&node, self.identity, self.device, self.fp, self.connect.to_vec()).await?;
        let conn = peer.conn;

        let total = files.iter().fold(0u64, |total, file| total.saturating_add(file.size));
        let addr = conn.remote_address();
        let addr = SocketAddr::new(addr.ip().to_canonical(), addr.port());
        let name = &self.device.name;
        if retry {
            eprintln!("Trying again ({})", summary(files.len(), total));
        } else {
            eprintln!("Sending {} to {name} ({addr})", summary(files.len(), total));
        }
        let progress = Arc::new(Progress::default());
        let start = Instant::now();
        // The daemon on this computer shows this send in `status` and the window.
        let (reports, reporter) = Reporter::start(
            self.dir.to_owned(),
            TransferStatus {
                id: transfer::new_batch_id(),
                sending: true,
                peer: one_mk_proto::sanitize_name(name),
                files: u32::try_from(files.len()).unwrap_or(u32::MAX),
                total,
                ..Default::default()
            },
        );
        let id = reports.borrow().id;
        let ticker = tokio::spawn(show_progress(progress.clone(), start, reports.clone()));
        let results = transfer::send_files_as(&conn, files, progress.clone(), id).await;
        ticker.abort();
        // One task tells the daemon, in order, so nothing can arrive after the end.
        reports.send_modify(|u| u.finished = true);
        let _ = reporter.await;
        conn.close(0u32.into(), b"done");
        // Wait for the close notice to leave: the receiver holds on to this
        // connection until it hears that, or times out.
        node.shutdown(CLOSE_WAIT).await;
        let sent = progress.done.load(Ordering::Relaxed).saturating_sub(progress.skipped.load(Ordering::Relaxed));
        Ok((results, sent))
    }
}

fn pick<'a>(trust: &'a TrustStore, to: Option<&str>) -> Result<&'a Device> {
    if let Some(q) = to {
        return trust.find(q).with_context(|| format!("no paired computer called {q:?} (see `one-mk devices`)"));
    }
    match trust.devices() {
        [] => bail!("no paired computers; run `one-mk pair` first"),
        [only] => Ok(only),
        many => {
            let names: Vec<&str> = many.iter().map(|d| d.name.as_str()).collect();
            bail!("several paired computers ({}); choose one with --to", names.join(", "))
        }
    }
}

/// Dials the device at its saved addresses, `--connect` addresses, and
/// wherever mDNS finds it; the first connection with the right
/// certificate wins.
pub(crate) async fn find(node: &Node, identity: &Identity, device: &Device, fp: Fingerprint, connect: Vec<SocketAddr>) -> Result<Peer> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Peer>();
    let mut dialed = HashSet::new();
    let mut dial = |addr: SocketAddr| {
        if !dialed.insert(addr) {
            return;
        }
        let (node, tx) = (node.clone(), tx.clone());
        tokio::spawn(async move {
            match node.connect_timeout(addr, DIAL_TIMEOUT).await {
                Ok(p) if p.fingerprint == fp => {
                    let _ = tx.send(p);
                }
                Ok(p) => p.conn.close(0u32.into(), b"wrong device"),
                Err(e) => tracing::debug!(%addr, "dial failed: {e:#}"),
            }
        });
    };
    connect.into_iter().for_each(&mut dial);
    device.addrs.iter().filter_map(|a| a.parse().ok()).for_each(&mut dial);
    let (_discovery, mut found) = match Discovery::browse(identity) {
        Ok((d, f)) => (Some(d), f),
        Err(e) => {
            tracing::debug!("discovery unavailable: {e:#}");
            (None, mpsc::unbounded_channel().1)
        }
    };
    let deadline = tokio::time::sleep(FIND_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            Some(p) = rx.recv() => return Ok(p),
            Some(f) = found.recv() => {
                if f.fingerprint == fp && !f.pairing {
                    f.addrs.into_iter().for_each(&mut dial);
                }
            }
            _ = &mut deadline => bail!(
                "could not reach {}; is `one-mk run` running there? (try --connect <ip:port>)",
                device.name
            ),
        }
    }
}

/// What the daemon is told about a send, as it changes.
#[derive(Clone, Default)]
struct Update {
    status: TransferStatus,
    finished: bool,
}

impl std::ops::Deref for Update {
    type Target = TransferStatus;
    fn deref(&self) -> &TransferStatus {
        &self.status
    }
}

/// Tells the daemon on this computer how far a send is.
struct Reporter;

impl Reporter {
    /// Starts the one task that tells the daemon about a send. Progress goes
    /// in through the returned channel; only the latest is told, one at a
    /// time and in order, so a slow daemon never sees an older figure after
    /// a newer one, and `finished` is always last. The task ends after it.
    fn start(dir: PathBuf, status: TransferStatus) -> (watch::Sender<Update>, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = watch::channel(Update { status, finished: false });
        let task = tokio::spawn(async move {
            loop {
                if rx.changed().await.is_err() {
                    return;
                }
                let u = rx.borrow_and_update().clone();
                // Best effort: no daemon running, or one that doesn't answer, is fine.
                let req = Request::Transfer { t: u.status, finished: u.finished };
                let _ = tokio::time::timeout(Duration::from_secs(1), control::request(&dir, req)).await;
                if u.finished {
                    return;
                }
            }
        });
        (tx, task)
    }
}

async fn show_progress(p: Arc<Progress>, start: Instant, reports: watch::Sender<Update>) {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut speed = Rate::new(start);
    let mut ticks = 0u32;
    loop {
        tick.tick().await;
        let (total, done, skipped) =
            (p.total.load(Ordering::Relaxed), p.done.load(Ordering::Relaxed), p.skipped.load(Ordering::Relaxed));
        let moved = done.saturating_sub(skipped);
        let now = Instant::now();
        speed.update(now, moved);
        // Twice a second is plenty for a status line, and costs the daemon nothing between sends.
        ticks += 1;
        if ticks % 2 == 1 {
            let rate = speed.get(now);
            reports.send_modify(|u| {
                u.status.done = done;
                u.status.total = u.status.total.max(done);
                u.status.rate = rate;
            });
        }
        let rate = moved as f64 / start.elapsed().as_secs_f64().max(1e-3);
        let eta = if rate > 0.0 { format!("{:.0} s", (total - done.min(total)) as f64 / rate) } else { "-".into() };
        let pct = if total > 0 { done as f64 * 100.0 / total as f64 } else { 100.0 };
        eprint!("\r\x1b[2K  {pct:5.1}%  {} / {}  {}/s  ETA {eta}", bytes(done), bytes(total), bytes(rate as u64));
        let _ = std::io::stderr().flush();
    }
}

fn summary(files: usize, size: u64) -> String {
    format!("{files} file{} ({})", if files == 1 { "" } else { "s" }, bytes(size))
}

fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::{Inbox, MAX_CONNS_PER_PEER};

    /// A receiver on loopback that serves transfer connections the way the
    /// daemon does (`app::serve_streams`): a permit per connection, held
    /// until the connection is over.
    struct Receiver {
        addr: SocketAddr,
        fp: Fingerprint,
        inbox: Arc<Inbox>,
        root: PathBuf,
        _node: Node,
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-send-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn receiver(tag: &str) -> Receiver {
        let id = Arc::new(Identity::ephemeral().unwrap());
        let node = Node::bind(id.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let root = temp(&format!("{tag}-inbox"));
        let inbox = Inbox::new(root.clone());
        let (accepting, n) = (inbox.clone(), node.clone());
        tokio::spawn(async move {
            while let Some(Ok(peer)) = n.accept().await {
                let inbox = accepting.clone();
                tokio::spawn(async move {
                    let permit = match inbox.admit(peer.fingerprint) {
                        Ok(p) => Arc::new(p),
                        Err(why) => return peer.conn.close(3u32.into(), why.as_bytes()),
                    };
                    while let Ok((send, mut recv)) = peer.conn.accept_bi().await {
                        let (inbox, permit) = (inbox.clone(), permit.clone());
                        tokio::spawn(async move {
                            let _ = one_mk_net::read_stream_kind(&mut recv).await;
                            inbox.receive_on(&permit, "sender", send, recv).await;
                        });
                    }
                });
            }
        });
        Receiver { addr: node.local_addr().unwrap(), fp: id.fingerprint, inbox, root, _node: node }
    }

    /// A home folder for `one-mk send` that has paired with `rx`.
    fn sender_home(tag: &str, rx: &Receiver) -> (PathBuf, Fingerprint) {
        let home = temp(&format!("{tag}-home"));
        let me = Identity::load_or_create(&home).unwrap().fingerprint;
        let devices = format!("[[device]]\nname = \"laptop\"\nfingerprint = \"{}\"\naddrs = [\"{}\"]\n", rx.fp, rx.addr);
        std::fs::write(home.join("devices.toml"), devices).unwrap();
        (home, me)
    }

    /// A finished send must free the receiver's slot at once: the receiver
    /// only counts the connection as over once it has heard it close (or
    /// 4 s of silence), and a send that exited without the close notice
    /// leaving made every second send in a row fail.
    #[tokio::test]
    async fn back_to_back_sends_all_get_through() {
        let rx = receiver("back");
        let (home, _) = sender_home("back", &rx);
        let file = temp("back-src").join("f.txt");
        std::fs::write(&file, "hi\n").unwrap();
        for i in 0..5 {
            // Each send gets a runtime of its own that is gone when it ends,
            // like the `one-mk send` process: nothing it leaves unfinished
            // gets to finish later.
            let (home, file) = (home.clone(), file.clone());
            let sent = tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(run(&home, vec![file], None, vec![]))
            })
            .await
            .unwrap();
            assert!(sent.is_ok(), "send {i}: {sent:?}");
        }
        assert_eq!(std::fs::read_dir(&rx.root).unwrap().flatten().filter(|e| e.path().is_file()).count(), 5);
    }

    /// If the receiver is at its connection limit when we ask (say the
    /// previous send hasn't been seen leaving yet), we ask again once.
    #[tokio::test]
    async fn a_busy_receiver_is_asked_again_once() {
        let rx = receiver("busy");
        let (home, me) = sender_home("busy", &rx);
        let file = temp("busy-src").join("g.txt");
        std::fs::write(&file, "hi\n").unwrap();
        let mut held: Vec<_> = (0..MAX_CONNS_PER_PEER).map(|_| rx.inbox.admit(me).unwrap()).collect();
        // Frees up while the sender waits to try again.
        let free = tokio::spawn(async move {
            tokio::time::sleep(BUSY_RETRY / 3).await;
            held.pop();
            held
        });
        let sent = run(&home, vec![file.clone()], None, vec![]).await;
        assert!(sent.is_ok(), "{sent:?}");
        assert!(rx.root.join("g.txt").exists());
        let mut held = free.await.unwrap();

        // Still full for good: tried twice, then it says so.
        held.push(rx.inbox.admit(me).unwrap());
        let sent = run(&home, vec![file], None, vec![]).await;
        let why = format!("{:#}", sent.unwrap_err());
        assert!(why.contains("1 file(s) were not sent"), "{why}");
    }
}
