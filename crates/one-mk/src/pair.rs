//! `one-mk pair`: find another computer running `one-mk pair`, show the
//! same 6-digit code on both, and remember each other once confirmed.
//!
//! One candidate is handled at a time. If a session fails or stalls, we go
//! back to waiting for the next one; only an explicit "no" ends pairing.
//!
//! With `--machine` (what the app window uses) the output is one event per
//! line, tab-separated, and the answer is `yes` or `no` on stdin:
//!
//! ```text
//! me <name> <id> <port>       searching          hint
//! peer <id> <ip>              code <name> <code> waiting
//! paired <id> <name> <side>   cancelled          declined
//! retry <reason>
//! ```
//!
//! Names are sanitized and never contain tabs or newlines.

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

/// How pairing talks to whoever started it.
#[derive(Clone, Copy)]
pub struct Out {
    machine: bool,
}

impl Out {
    pub fn new(machine: bool) -> Self {
        Self { machine }
    }

    /// Prints the sentence for a person, or the event line for a program.
    fn say(self, human: impl FnOnce() -> String, event: impl FnOnce() -> String) {
        let line = if self.machine { event() } else { human() };
        println!("{line}");
        let _ = std::io::stdout().flush();
    }
}

pub async fn run(dir: &Path, connect: Option<SocketAddr>, out: Out) -> Result<()> {
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

    out.say(
        || format!("This computer: {} ({}), pairing port {port}", identity.name, me.short()),
        || format!("me\t{}\t{}\t{port}", identity.name, me.short()),
    );
    out.say(
        || match connect {
            Some(addr) => format!("Connecting to {addr}..."),
            None => "Looking for another computer... run `one-mk pair` on it too.".into(),
        },
        || "searching".into(),
    );

    let mut answers = Answers::spawn(out);
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
                match session(peer, &identity, &mut trust, &mut answers, out).await {
                    Ok(()) => break,
                    Err(e) => {
                        out.say(
                            || format!("\nPairing with {short} did not complete: {e:#}\nWaiting for another try... (Ctrl+C to stop)"),
                            || format!("retry\t{}", one_line(&format!("{e:#}"))),
                        );
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
                    out.say(|| hints(port), || "hint".into());
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
async fn session(peer: Peer, identity: &Identity, trust: &mut TrustStore, answers: &mut Answers, out: Out) -> Result<()> {
    let fingerprint = peer.fingerprint;
    let ip = peer.conn.remote_address().ip().to_canonical();
    out.say(
        || format!("\nPairing with a computer at {ip} (id {})...", fingerprint.short()),
        || format!("peer\t{}\t{ip}", fingerprint.short()),
    );
    let res = exchange(&peer, identity, trust, answers, out).await;
    if res.is_err() {
        peer.conn.close(0u32.into(), b"pairing failed");
    }
    res
}

async fn exchange(peer: &Peer, identity: &Identity, trust: &mut TrustStore, answers: &mut Answers, out: Out) -> Result<()> {
    let fingerprint = peer.fingerprint;
    // Remember the address with the service port (not this temporary one),
    // for networks where discovery is blocked.
    let addr = SocketAddr::new(peer.conn.remote_address().ip().to_canonical(), one_mk_proto::DEFAULT_PORT);
    let pairing = Pairing::start(peer, identity.fingerprint, &identity.name).await?;
    let name = pairing.peer_name.clone();

    // Everything the other computer chose (its name) comes first; the code
    // comes last, on its own line, so nothing printed after it can
    // disguise it.
    out.say(
        || format!("  The other computer says its name is: {name}\n  Check that it shows this same code:\n\n      {}\n", pairing.sas),
        || format!("code\t{name}\t{}", pairing.sas),
    );
    let ok = match answers.ask("  Do the codes match? [y/N] ", ANSWER_TIMEOUT).await {
        Some(ok) => ok,
        None => {
            let _ = tokio::time::timeout(Duration::from_secs(2), pairing.confirm(false)).await;
            bail!("no answer within {} s", ANSWER_TIMEOUT.as_secs());
        }
    };
    if !ok {
        let _ = tokio::time::timeout(Duration::from_secs(2), pairing.confirm(false)).await;
        out.say(|| "Pairing cancelled.".into(), || "cancelled".into());
        return Ok(());
    }
    out.say(|| "  Waiting for the other computer to confirm...".into(), || "waiting".into());
    let confirmed = tokio::time::timeout(ANSWER_TIMEOUT, pairing.confirm(true))
        .await
        .context("the other computer did not answer in time")?
        .context("the other computer went away")?;
    if !confirmed {
        out.say(|| "The other computer did not confirm. Nothing was saved.".into(), || "declined".into());
        return Ok(());
    }

    // Opposite defaults on each side, so the layout agrees out of the box.
    let side = if peer.initiator { Side::Right } else { Side::Left };
    trust.upsert(Device { name: name.clone(), fingerprint: fingerprint.to_string(), side: side.to_string(), addrs: vec![addr.to_string()], ..Device::default() });
    trust.save()?;
    let side = trust.get(&fingerprint).map_or(side, |d| d.side());
    out.say(
        || {
            format!(
                "\nPaired with {name}. It is set to the {side} of this computer.\n\
                 If that's wrong:  one-mk layout \"{name}\" {}\n\
                 Start sharing:    one-mk run   (on both computers)",
                side.opposite()
            )
        },
        || format!("paired\t{fingerprint}\t{name}\t{side}"),
    );
    Ok(())
}

fn hints(port: u16) -> String {
    format!(
        "\nNothing found yet. If `one-mk pair` is running on the other computer:\n\
         \x20 - check that both are on the same network (not a guest or isolated Wi-Fi)\n\
         \x20 - check that a firewall allows one-mk (UDP, and mDNS on port 5353)\n\
         \x20 - or connect directly; on the other computer run:\n\
         \x20       one-mk pair --connect <this computer's IP>:{port}"
    )
}

/// Error text for a machine event: one line, no tabs.
fn one_line(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
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
    out: Out,
}

impl Answers {
    fn spawn(out: Out) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = std::thread::Builder::new().name("one-mk-stdin".into()).spawn(move || {
            let mut line = String::new();
            while matches!(std::io::stdin().read_line(&mut line), Ok(n) if n > 0) {
                if tx.send(std::mem::take(&mut line)).is_err() {
                    break;
                }
            }
        });
        Self { rx, out }
    }

    /// Asks a yes/no question. `None` if nobody answered in time; end of
    /// input counts as "no".
    async fn ask(&mut self, prompt: &str, timeout: Duration) -> Option<bool> {
        if !self.out.machine {
            print!("{prompt}");
            let _ = std::io::stdout().flush();
        }
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
