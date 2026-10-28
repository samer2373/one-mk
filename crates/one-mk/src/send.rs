//! `one-mk send`: send files or folders to a paired computer running
//! `one-mk run`.
//!
//! This uses the same identity as the service on this machine, so the
//! receiver knows it is trusted. It opens its own connection whose first
//! stream is a file stream, which the receiver treats as transfer-only:
//! the service's mouse/keyboard link is not touched.

use crate::transfer::{self, Outcome, Progress};
use anyhow::{Context, Result, bail};
use one_mk_net::{Device, Discovery, Fingerprint, Identity, Node, Peer, TrustStore};
use std::collections::HashSet;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How long to look for the device before giving up.
const FIND_TIMEOUT: Duration = Duration::from_secs(6);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

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
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let trust = TrustStore::load(dir)?;
    let device = pick(&trust, to.as_deref())?.clone();
    let fp = device.fingerprint().context("corrupt fingerprint in devices.toml")?;

    let node = Node::bind_any(identity.clone(), 0)?;
    let peer = find(&node, &identity, &device, fp, connect).await?;
    let conn = peer.conn;

    let total: u64 = files.iter().map(|f| f.size).sum();
    let addr = conn.remote_address();
    let addr = SocketAddr::new(addr.ip().to_canonical(), addr.port());
    eprintln!("Sending {} to {} ({addr})", summary(files.len(), total), device.name);
    let progress = Arc::new(Progress::default());
    let start = Instant::now();
    let ticker = tokio::spawn(show_progress(progress.clone(), start));
    let results = transfer::send_files(&conn, files, progress.clone()).await;
    ticker.abort();
    conn.close(0u32.into(), b"done");
    node.close();

    let elapsed = start.elapsed().as_secs_f64();
    let sent = progress.done.load(Ordering::Relaxed) - progress.skipped.load(Ordering::Relaxed);
    eprint!("\r\x1b[2K");
    let mut failed = 0;
    let (mut saved, mut saved_bytes) = (0, 0);
    for (file, outcome) in &results {
        match outcome {
            Outcome::Saved { resumed_from, .. } => {
                saved += 1;
                saved_bytes += file.size;
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
async fn find(node: &Node, identity: &Identity, device: &Device, fp: Fingerprint, connect: Vec<SocketAddr>) -> Result<Peer> {
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

async fn show_progress(p: Arc<Progress>, start: Instant) {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tick.tick().await;
        let (total, done, skipped) =
            (p.total.load(Ordering::Relaxed), p.done.load(Ordering::Relaxed), p.skipped.load(Ordering::Relaxed));
        let rate = (done - skipped) as f64 / start.elapsed().as_secs_f64().max(1e-3);
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
