//! `one-mk pair`: find another computer running `one-mk pair`, show the
//! same 6-digit code on both, and remember each other once confirmed.
//!
//! One candidate is handled at a time. If a session fails or stalls, we go
//! back to waiting for the next one; only an explicit "no" ends pairing.

use anyhow::{Context, Result, bail};
use one_mk_net::{Device, Discovery, Fingerprint, Identity, Node, Pairing, Peer, TrustStore};
use one_mk_proto::Side;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Each automatic step (dialing, the code exchange) must finish within this.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// One connection attempt.
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a person has to compare the codes and answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);
/// Show troubleshooting hints if nothing turned up by then.
const HINT_AFTER: Duration = Duration::from_secs(60);

pub async fn run(dir: &Path, connect: Option<SocketAddr>) -> Result<()> {
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let mut trust = TrustStore::load(dir)?;
    let node = Node::bind_any(identity.clone(), 0)?;
    let port = node.local_addr()?.port();
    let me = identity.fingerprint;
    let (_discovery, mut found) = match Discovery::start(&identity, port, true) {
        Ok((d, f)) => (Some(d), f),
        Err(e) if connect.is_some() => {
            tracing::debug!("discovery unavailable: {e:#}");
            (None, mpsc::unbounded_channel().1)
        }
        Err(e) => return Err(e.context("could not start network discovery (try --connect <ip:port>)")),
    };

    println!("This computer: {} ({}), pairing port {port}", identity.name, me.short());
    match connect {
        Some(addr) => println!("Connecting to {addr}..."),
        None => println!("Looking for another computer... run `one-mk pair` on it too."),
    }

    let mut answers = Answers::spawn();
    let (tx, mut peers) = mpsc::channel::<Peer>(4);
    {
        let (node, tx) = (node.clone(), tx.clone());
        tokio::spawn(async move {
            while let Some(res) = node.accept().await {
                if let Ok(p) = res
                    && tx.send(p).await.is_err()
                {
                    break;
                }
            }
        });
    }
    let (dial_err_tx, mut dial_err) = mpsc::unbounded_channel::<String>();
    // Candidates whose every address failed, so a later announcement may
    // be dialed again.
    let (unreachable_tx, mut unreachable) = mpsc::unbounded_channel::<Fingerprint>();
    if let Some(addr) = connect {
        dial_explicit(node.clone(), addr, tx.clone(), dial_err_tx.clone());
    }

    // Pairing candidates seen through discovery (untrusted address hints).
    let mut candidates: HashMap<Fingerprint, Vec<SocketAddr>> = HashMap::new();
    let mut dialed = HashSet::new();
    let hint = tokio::time::sleep(HINT_AFTER);
    tokio::pin!(hint);
    let mut hinted = false;
    loop {
        tokio::select! {
            Some(f) = found.recv() => {
                if !f.pairing || f.fingerprint == me {
                    continue;
                }
                // Announcements can arrive before any usable address does.
                if f.addrs.is_empty() {
                    continue;
                }
                candidates.insert(f.fingerprint, f.addrs.clone());
                // Only the lower id dials, so the two computers never race
                // each other with two separate pairing sessions.
                if connect.is_none() && f.fingerprint > me && dialed.insert(f.fingerprint) {
                    dial(node.clone(), f.fingerprint, f.addrs, Duration::ZERO, tx.clone(), unreachable_tx.clone());
                }
            }
            Some(peer) = peers.recv() => {
                if peer.fingerprint == me || peer.conn.close_reason().is_some() {
                    continue;
                }
                hinted = true;
                let short = peer.fingerprint.short();
                match session(peer, &identity, &mut trust, &mut answers).await {
                    Ok(()) => break,
                    Err(e) => {
                        println!();
                        println!("Pairing with {short} did not complete: {e:#}");
                        println!("Waiting for another try... (Ctrl+C to stop)");
                        answers.clear();
                        // Try the known candidates again, giving the other
                        // side a moment to go back to waiting too.
                        let delay = Duration::from_secs(2);
                        if let Some(addr) = connect {
                            dial_explicit_after(node.clone(), addr, delay, tx.clone(), dial_err_tx.clone());
                        } else {
                            dialed.clear();
                            for (fp, addrs) in &candidates {
                                if *fp > me && dialed.insert(*fp) {
                                    dial(node.clone(), *fp, addrs.clone(), delay, tx.clone(), unreachable_tx.clone());
                                }
                            }
                        }
                    }
                }
            }
            Some(fp) = unreachable.recv() => {
                dialed.remove(&fp);
            }
            Some(e) = dial_err.recv() => bail!(e),
            _ = &mut hint, if !hinted => {
                hinted = true;
                if candidates.is_empty() {
                    print_hints(port);
                }
            }
        }
    }
    node.close();
    tokio::time::sleep(Duration::from_millis(50)).await;
    Ok(())
}

/// One pairing attempt. `Ok` once pairing is over for good (saved, or a
/// person said no); `Err` if it failed and we should keep waiting.
async fn session(peer: Peer, identity: &Identity, trust: &mut TrustStore, answers: &mut Answers) -> Result<()> {
    let fingerprint = peer.fingerprint;
    let remote = peer.conn.remote_address();
    println!();
    println!("Pairing with a computer at {} (id {})...", remote.ip().to_canonical(), fingerprint.short());
    let res = exchange(&peer, identity, trust, answers).await;
    if res.is_err() {
        peer.conn.close(0u32.into(), b"pairing failed");
    }
    res
}

async fn exchange(peer: &Peer, identity: &Identity, trust: &mut TrustStore, answers: &mut Answers) -> Result<()> {
    let fingerprint = peer.fingerprint;
    // Remember the address with the service port (not this temporary one),
    // for networks where discovery is blocked.
    let addr = SocketAddr::new(peer.conn.remote_address().ip().to_canonical(), one_mk_proto::DEFAULT_PORT);
    let pairing = Pairing::start(peer, identity.fingerprint, &identity.name).await?;
    let name = pairing.peer_name.clone();

    // Everything the other computer chose (its name) comes first; the code
    // comes last, on its own line, so nothing printed after it can
    // disguise it.
    println!("  The other computer says its name is: {name}");
    println!("  Check that it shows this same code:");
    println!();
    println!("      {}", pairing.sas);
    println!();
    let ok = match answers.ask("  Do the codes match? [y/N] ", ANSWER_TIMEOUT).await {
        Some(ok) => ok,
        None => {
            let _ = tokio::time::timeout(Duration::from_secs(2), pairing.confirm(false)).await;
            bail!("no answer within {} s", ANSWER_TIMEOUT.as_secs());
        }
    };
    if !ok {
        let _ = tokio::time::timeout(Duration::from_secs(2), pairing.confirm(false)).await;
        println!("Pairing cancelled.");
        return Ok(());
    }
    println!("  Waiting for the other computer to confirm...");
    let confirmed = tokio::time::timeout(ANSWER_TIMEOUT, pairing.confirm(true))
        .await
        .context("the other computer did not answer in time")?
        .context("the other computer went away")?;
    if !confirmed {
        println!("The other computer did not confirm. Nothing was saved.");
        return Ok(());
    }

    // Opposite defaults on each side, so the layout agrees out of the box.
    let side = if peer.initiator { Side::Right } else { Side::Left };
    trust.upsert(Device { name: name.clone(), fingerprint: fingerprint.to_string(), side: side.to_string(), addrs: vec![addr.to_string()] });
    trust.save()?;
    let side = trust.get(&fingerprint).map_or(side, |d| d.side());
    println!();
    println!("Paired with {name}. It is set to the {side} of this computer.");
    println!("If that's wrong:  one-mk layout \"{name}\" {}", side.opposite());
    println!("Start sharing:    one-mk run   (on both computers)");
    Ok(())
}

fn print_hints(port: u16) {
    println!();
    println!("Nothing found yet. If `one-mk pair` is running on the other computer:");
    println!("  - check that both are on the same network (not a guest or isolated Wi-Fi)");
    println!("  - check that a firewall allows one-mk (UDP, and mDNS on port 5353)");
    println!("  - or connect directly; on the other computer run:");
    println!("        one-mk pair --connect <this computer's IP>:{port}");
}

/// Dials a discovered candidate, trying its addresses in order (one
/// connection at a time, so both sides end up handling the same one).
/// Reports the candidate on `unreachable` if every address failed.
fn dial(
    node: Node,
    fp: Fingerprint,
    addrs: Vec<SocketAddr>,
    delay: Duration,
    tx: mpsc::Sender<Peer>,
    unreachable: mpsc::UnboundedSender<Fingerprint>,
) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        for addr in addrs {
            match node.connect_timeout(addr, DIAL_TIMEOUT).await {
                Ok(p) if p.fingerprint == fp => {
                    let _ = tx.send(p).await;
                    return;
                }
                Ok(p) => p.conn.close(0u32.into(), b"wrong device"),
                Err(e) => tracing::debug!("{e:#}"),
            }
        }
        let _ = unreachable.send(fp);
    });
}

fn dial_explicit(node: Node, addr: SocketAddr, tx: mpsc::Sender<Peer>, err: mpsc::UnboundedSender<String>) {
    dial_explicit_after(node, addr, Duration::ZERO, tx, err);
}

/// Dials `--connect`, retrying for `STEP_TIMEOUT`, then reports a clear
/// error.
fn dial_explicit_after(
    node: Node,
    addr: SocketAddr,
    delay: Duration,
    tx: mpsc::Sender<Peer>,
    err: mpsc::UnboundedSender<String>,
) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let deadline = Instant::now() + STEP_TIMEOUT;
        let mut last = String::new();
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now()).min(DIAL_TIMEOUT);
            match node.connect_timeout(addr, left).await {
                Ok(p) => {
                    let _ = tx.send(p).await;
                    return;
                }
                Err(e) => {
                    last = format!("{:#}", e.root_cause());
                    tokio::time::sleep(Duration::from_secs(1).min(deadline.saturating_duration_since(Instant::now())))
                        .await;
                }
            }
        }
        let _ = err.send(format!(
            "could not reach {addr} within {} s ({last}); check that `one-mk pair` is running there \
             and that the address and port match what it prints",
            STEP_TIMEOUT.as_secs()
        ));
    });
}

/// Lines typed on stdin, read on a plain thread (never a runtime blocking
/// thread, which would keep the process alive at exit).
struct Answers {
    rx: mpsc::UnboundedReceiver<String>,
}

impl Answers {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = std::thread::Builder::new().name("one-mk-stdin".into()).spawn(move || {
            let mut line = String::new();
            while matches!(std::io::stdin().read_line(&mut line), Ok(n) if n > 0) {
                if tx.send(std::mem::take(&mut line)).is_err() {
                    break;
                }
            }
        });
        Self { rx }
    }

    /// Asks a yes/no question. `None` if nobody answered in time; end of
    /// input counts as "no".
    async fn ask(&mut self, prompt: &str, timeout: Duration) -> Option<bool> {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        match tokio::time::timeout(timeout, self.rx.recv()).await {
            Ok(Some(line)) => Some(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")),
            Ok(None) => {
                println!();
                Some(false)
            }
            Err(_) => {
                println!();
                None
            }
        }
    }

    /// Forgets anything typed for an earlier session, so a stray "y"
    /// can't confirm the next one.
    fn clear(&mut self) {
        while self.rx.try_recv().is_ok() {}
    }
}
