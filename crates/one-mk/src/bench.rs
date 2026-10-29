//! `one-mk bench`: measures what one-mk adds on top of the network.
//!
//! - protocol cost: CPU time to handle one mouse event end to end in our code
//! - round trip: real QUIC datagrams, either over loopback or to a machine
//!   running `one-mk bench --serve`
//! - CPU: process CPU time while streaming events at mouse rates

use anyhow::{Context, Result};
use one_mk_net::{Identity, Node, TrustStore};
use one_mk_proto::{Align, Datagram, StreamKind, Desktop, MotionReceiver, Pos, Rect, RemoteCursor, Seam, Side, Step};
use std::hint::black_box;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
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
        let device = trust
            .find(query)
            .with_context(|| format!("no paired computer called {query:?} (see `one-mk devices`)"))?
            .clone();
        let fp = device.fingerprint().context("corrupt fingerprint in devices.toml")?;
        let node = Node::bind_any(identity.clone(), 0)?;
        let peer = crate::send::find(&node, &identity, &device, fp, Vec::new()).await?;
        // Tells the daemon this connection only echoes probes.
        peer.open_stream(StreamKind::Probe).await?;
        let label = format!("to {} ({})", device.name, peer.conn.remote_address());
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

    let base = Instant::now();
    let recv_conn = conn.clone();
    let count = opts.count as usize;
    let receiver = tokio::spawn(async move {
        let mut rtts = Vec::with_capacity(count);
        while let Ok(Ok(b)) = tokio::time::timeout(Duration::from_secs(1), recv_conn.read_datagram()).await {
            if let Ok(Datagram::ProbeEcho { sent_ns, .. }) = Datagram::decode(&b) {
                rtts.push(base.elapsed().as_nanos() as u64 - sent_ns);
                if rtts.len() == count {
                    break;
                }
            }
        }
        rtts
    });

    let cpu0 = cpu_time();
    let wall0 = Instant::now();
    let per_tick = opts.rate.div_ceil(1000).max(1);
    let mut tick = tokio::time::interval(Duration::from_secs_f64(f64::from(per_tick) / f64::from(opts.rate)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut sent = 0u32;
    while sent < opts.count {
        tick.tick().await;
        for _ in 0..per_tick.min(opts.count - sent) {
            let d = Datagram::Probe { id: sent, sent_ns: base.elapsed().as_nanos() as u64 };
            let (b, n) = d.encode();
            conn.send_datagram(bytes::Bytes::copy_from_slice(&b[..n]))?;
            sent += 1;
        }
    }
    let wall = wall0.elapsed();
    let cpu = cpu_time().map(|c| c.saturating_sub(cpu0.unwrap_or_default()));
    let mut rtts = receiver.await?;

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

/// Average time for one pointer event through all of our own code.
fn protocol_cost() -> f64 {
    let desk = Desktop::new(vec![Rect::new(0, 0, 3840, 2160)]);
    let seam = Seam::new(desk.clone(), desk, Side::Right, Align::Stretch);
    let mut cursor = RemoteCursor::cross(seam, Pos::new(3839.0, 1080.0), 1).expect("on the edge");
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
