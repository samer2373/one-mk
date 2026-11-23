//! `one-mk bench`: measures what one-mk adds on top of the network.
//!
//! - protocol cost: CPU time to handle one mouse event end to end in our code
//! - round trip: real QUIC datagrams, either over loopback or to a machine
//!   running `one-mk bench --serve`
//! - CPU: process CPU time while streaming events at mouse rates

use anyhow::{Context, Result};
use one_mk_net::{Identity, Node, TrustStore};
use one_mk_proto::{Datagram, StreamKind, Desktop, MotionReceiver, Rect, RemoteCursor, Side, Step};
use std::hint::black_box;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Options {
    pub count: u32,
    pub rate: u32,
    pub serve: Option<u16>,
    pub connect: Option<SocketAddr>,
    /// A paired device running the daemon.
    pub peer: Option<String>,
}

pub async fn run(dir: &Path, opts: Options) -> Result<()> {
    if let Some(port) = opts.serve {
        return serve(port).await;
    }

    let ns = protocol_cost();

    let (client, conn, label) = if let Some(query) = &opts.peer {
        let identity = Arc::new(Identity::load_or_create(dir)?);
        let trust = TrustStore::load(dir)?;
        let device = trust.find(query).ok_or_else(|| crate::manage::not_found(&trust, query))?.clone();
        let shown = trust.display(&device);
        let fp = device.fingerprint().context("corrupt fingerprint in devices.toml")?;
        let node = Node::bind_any(identity.clone(), 0)?;
        let peer = crate::send::find(&node, &identity, &device, &shown, fp, Vec::new(), "or measure a computer by address with `one-mk bench --connect <ip:port>`, where `one-mk bench --serve <port>` runs").await?;
        // Tells the daemon this connection only echoes probes.
        peer.open_stream(StreamKind::Probe).await?;
        let label = format!("to {shown} ({})", peer.conn.remote_address());
        (node, peer.conn, label)
    } else {
        let client = Node::bind_any(Arc::new(Identity::ephemeral()?), 0)?;
        let (target, label) = match opts.connect {
            Some(addr) => (addr, format!("to {addr}")),
            None => {
                let server = Node::bind(Arc::new(Identity::ephemeral()?), "127.0.0.1:0".parse()?)?;
                let addr = server.local_addr()?;
                tokio::spawn(echo(server));
                (addr, "loopback".to_owned())
            }
        };
        let peer = client.connect_timeout(target, DIAL_TIMEOUT).await?;
        (client, peer.conn, label)
    };

    println!("one-mk bench ({label}, {} events at {} Hz)", opts.count, opts.rate);
    println!("  protocol cost   {ns:.0} ns per mouse event (move, encode, decode, filter)");

    let cpu0 = cpu_time();
    let (mut rtts, wall) = measure(&conn, opts.count, opts.rate).await?;
    let cpu = cpu_time().map(|c| c.saturating_sub(cpu0.unwrap_or_default()));

    rtts.sort_unstable();
    let q = |p: f64| -> f64 {
        if rtts.is_empty() {
            return f64::NAN;
        }
        rtts[((rtts.len() - 1) as f64 * p).round() as usize] as f64 / 1000.0
    };
    println!(
        "  round trip      p50 {:.0} µs   p99 {:.0} µs   p99.9 {:.0} µs   max {:.0} µs",
        q(0.5),
        q(0.99),
        q(0.999),
        q(1.0)
    );
    println!("  one-way         ~{:.0} µs typical (half the round trip)", q(0.5) / 2.0);
    println!("  lost            {} of {}", opts.count as usize - rtts.len(), opts.count);
    if let Some(cpu) = cpu {
        let who = if opts.connect.is_some() || opts.peer.is_some() { "this end" } else { "both ends" };
        println!(
            "  CPU             {:.2}% of one core ({who}) at {} events/s",
            cpu.as_secs_f64() / wall.as_secs_f64() * 100.0,
            opts.rate
        );
    }
    client.close();
    Ok(())
}

/// How long after the last probe was sent a late echo is still waited for.
const GRACE: Duration = Duration::from_secs(1);

/// Sends `count` probes at `rate` per second and collects the round trip of each
/// one that is echoed (nanoseconds), and how long sending took. An echo counts
/// once, for a probe that was sent, with the time that probe carried; anything
/// else (a repeat, an id never sent, a made-up time) is ignored. Echoes are
/// collected until every one is in, or until [`GRACE`] after the last probe left,
/// however slowly the probes go out.
async fn measure(conn: &quinn::Connection, count: u32, rate: u32) -> Result<(Vec<u64>, Duration)> {
    let base = Instant::now();
    let total = count as usize;
    // What each probe carried (+1, so 0 is "not sent yet").
    let sent_ns: Arc<Vec<AtomicU64>> = Arc::new((0..total).map(|_| AtomicU64::new(0)).collect());
    let finished = Arc::new(AtomicBool::new(false));
    let receiver = {
        let (conn, sent_ns, finished) = (conn.clone(), sent_ns.clone(), finished.clone());
        tokio::spawn(async move {
            let mut rtts = Vec::with_capacity(total);
            let mut echoed = vec![false; total];
            let mut finished_at: Option<Instant> = None;
            while rtts.len() < total {
                if finished.load(Ordering::Acquire) {
                    finished_at.get_or_insert_with(Instant::now);
                }
                if finished_at.is_some_and(|at| at.elapsed() >= GRACE) {
                    break;
                }
                let Ok(read) = tokio::time::timeout(Duration::from_millis(100), conn.read_datagram()).await else { continue };
                let Ok(b) = read else { break };
                if let Ok(Datagram::ProbeEcho { id, sent_ns: carried }) = Datagram::decode(&b) {
                    let Some(slot) = sent_ns.get(id as usize) else { continue };
                    if echoed[id as usize] || slot.load(Ordering::Acquire) != carried.wrapping_add(1) { continue }
                    let Some(elapsed) = (base.elapsed().as_nanos() as u64).checked_sub(carried) else { continue };
                    echoed[id as usize] = true;
                    rtts.push(elapsed);
                }
            }
            rtts
        })
    };

    let wall0 = Instant::now();
    let per_tick = rate.div_ceil(1000).max(1);
    let mut tick = tokio::time::interval(Duration::from_secs_f64(f64::from(per_tick) / f64::from(rate)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut sent = 0u32;
    while sent < count {
        tick.tick().await;
        for _ in 0..per_tick.min(count - sent) {
            let at = base.elapsed().as_nanos() as u64;
            sent_ns[sent as usize].store(at.wrapping_add(1), Ordering::Release);
            let d = Datagram::Probe { id: sent, sent_ns: at };
            let (b, n) = d.encode();
            conn.send_datagram(bytes::Bytes::copy_from_slice(&b[..n]))?;
            sent += 1;
        }
    }
    let wall = wall0.elapsed();
    finished.store(true, Ordering::Release);
    Ok((receiver.await?, wall))
}

/// Average time for one pointer event through all of our own code.
fn protocol_cost() -> f64 {
    let desk = Desktop::new(vec![Rect::new(0, 0, 3840, 2160)]);
    let mut cursor = RemoteCursor::enter(desk, Side::Left, 0.5, 1);
    let mut rx = MotionReceiver::default();
    rx.enter(1);
    const N: u32 = 2_000_000;
    let start = Instant::now();
    for i in 0..N {
        let dx = if i % 2000 < 1000 { 1.25 } else { -1.25 };
        if let Step::Move(d) = cursor.motion(black_box(dx), black_box(0.5 - (i % 2) as f64)) {
            let (b, n) = d.encode();
            if let Ok(Datagram::Motion { epoch, seq, pos }) = Datagram::decode(black_box(&b[..n])) {
                black_box(rx.accept(epoch, seq, pos));
            }
        }
    }
    start.elapsed().as_nanos() as f64 / f64::from(N)
}

async fn serve(port: u16) -> Result<()> {
    let node = Node::bind_any(Arc::new(Identity::ephemeral()?), port)?;
    println!("bench server on {}; on the other machine run: one-mk bench --connect <this-ip>:{}", node.local_addr()?, node.local_addr()?.port());
    echo(node).await;
    Ok(())
}

async fn echo(node: Node) {
    while let Some(Ok(peer)) = node.accept().await {
        tokio::spawn(async move {
            while let Ok(b) = peer.conn.read_datagram().await {
                if let Ok(Datagram::Probe { id, sent_ns }) = Datagram::decode(&b) {
                    let (e, n) = Datagram::ProbeEcho { id, sent_ns }.encode();
                    let _ = peer.conn.send_datagram(bytes::Bytes::copy_from_slice(&e[..n]));
                }
            }
        });
    }
}

#[cfg(unix)]
fn cpu_time() -> Option<Duration> {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `u` is a valid, writable rusage.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) } != 0 {
        return None;
    }
    let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    Some(tv(u.ru_utime) + tv(u.ru_stime))
}

#[cfg(not(unix))]
fn cpu_time() -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A responder that echoes what `answer` says for each probe it reads
    /// (`id`, then the probe's own time), sending every listed echo.
    async fn responder(answer: impl Fn(u32, u64) -> Vec<(u32, u64)> + Send + 'static) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let server = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let peer = server.accept().await.unwrap().unwrap();
            while let Ok(bytes) = peer.conn.read_datagram().await {
                if let Ok(Datagram::Probe { id, sent_ns }) = Datagram::decode(&bytes) {
                    for (id, sent_ns) in answer(id, sent_ns) {
                        let (bytes, length) = Datagram::ProbeEcho { id, sent_ns }.encode();
                        let _ = peer.conn.send_datagram(bytes::Bytes::copy_from_slice(&bytes[..length]));
                    }
                }
            }
        });
        (address, task)
    }

    async fn connect(address: SocketAddr) -> (Node, quinn::Connection) {
        let client = Node::bind_any(Arc::new(Identity::ephemeral().unwrap()), 0).unwrap();
        let peer = client.connect_timeout(address, DIAL_TIMEOUT).await.unwrap();
        (client, peer.conn)
    }

    /// At a rate slower than the old one-second cut-off, an early lost echo
    /// must not end the listening: the later probes still count.
    #[tokio::test]
    async fn a_slow_rate_keeps_listening_until_the_sender_is_done() {
        let (address, task) = responder(|id, sent_ns| if id == 0 { vec![] } else { vec![(id, sent_ns)] }).await;
        let (_client, conn) = connect(address).await;
        let (rtts, _) = measure(&conn, 3, 1).await.unwrap();
        task.abort();
        assert_eq!(rtts.len(), 2, "probe 0 was lost, 1 and 2 were echoed");
    }

    /// A repeated echo, or one for a probe that was never sent, is no delivery.
    #[tokio::test]
    async fn duplicate_and_unsent_echoes_are_not_deliveries() {
        let (address, task) = responder(|id, sent_ns| if id == 0 { vec![(0, sent_ns), (0, sent_ns), (1, sent_ns), (99, sent_ns)] } else { vec![] }).await;
        let (_client, conn) = connect(address).await;
        let (rtts, _) = measure(&conn, 2, 1000).await.unwrap();
        task.abort();
        assert_eq!(rtts.len(), 1, "only probe 0 was echoed, and once");
    }

    #[tokio::test]
    async fn hostile_probe_timestamps_do_not_panic_or_replace_valid_samples() {
        let server = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let address = server.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let peer = server.accept().await.unwrap().unwrap();
            let bytes = peer.conn.read_datagram().await.unwrap();
            let Datagram::Probe { id, sent_ns } = Datagram::decode(&bytes).unwrap() else { panic!("expected probe") };
            for timestamp in [u64::MAX, sent_ns] {
                let (bytes, length) = Datagram::ProbeEcho { id, sent_ns: timestamp }.encode();
                peer.conn.send_datagram(bytes::Bytes::copy_from_slice(&bytes[..length])).unwrap();
            }
            let _ = peer.conn.closed().await;
        });
        let directory = tempfile::tempdir().unwrap();
        let result = run(directory.path(), Options { count: 1, rate: 1000, serve: None, connect: Some(address), peer: None }).await;
        responder.abort();
        let _ = responder.await;
        assert!(result.is_ok(), "malformed echo crashed the benchmark: {result:?}");
    }
}
