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
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// One connection attempt.
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a person has to compare the codes and answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// Each automatic step (dialing, the code exchange) must finish within this:
/// 30 s. (A debug build's tests shorten it, see `one_mk_net::test_timeout`.)
fn step_timeout() -> Duration {
    one_mk_net::step_timeout()
}

/// [`ANSWER_TIMEOUT`], or what a debug build's test shortened it to.
fn answer_timeout() -> Duration {
    one_mk_net::test_timeout(ANSWER_TIMEOUT, "ONE_MK_TEST_PAIR_ANSWER_MS")
}

/// "30 s", "120 s", "1.5 s": a time out for a message.
fn seconds(time: Duration) -> String {
    let s = time.as_secs_f64();
    if (s - s.round()).abs() < 1e-6 { format!("{} s", s.round() as u64) } else { format!("{s:.1} s") }
}
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

/// The port the other computer should reach this one's one-mk at: the
/// running daemon's, or the default one if none runs (the address is
/// corrected the first time the two connect).
pub async fn service_port(dir: &Path) -> Result<u16> {
    let running = crate::control::request(dir, crate::control::Request::Status).await?
        .and_then(|reply| one_mk_ctl::status::Status::decode(&reply));
    Ok(running.map_or(one_mk_proto::DEFAULT_PORT, |status| status.port))
}

pub async fn run(dir: &Path, connect: Option<SocketAddr>, out: Out, discovery: bool) -> Result<()> {
    let result = run_inner(dir, connect, out, discovery).await;
    if result.is_err() { crate::analytics::result(dir, crate::analytics::Event::PairingFinished, crate::analytics::Origin::Cli, &result); }
    result
}

async fn run_inner(dir: &Path, connect: Option<SocketAddr>, out: Out, discovery: bool) -> Result<()> {
    // Asked once before anything starts: a control socket that can't be
    // reached (a settings folder too deep for one) is an error to report,
    // not a reason to retry pairing for ever.
    let running = crate::control::request(dir, crate::control::Request::Status).await?.is_some();
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let mut trust = TrustStore::load(dir)?;
    let node = Node::bind_any(identity.clone(), 0)?;
    let port = node.local_addr()?.port();
    let me = identity.fingerprint;
    let (_discovery, mut found) = match discovery.then(|| Discovery::start(&identity, port, true)) {
        None => (None, mpsc::unbounded_channel().1),
        Some(Ok((d, f))) => (Some(d), f),
        Some(Err(e)) if connect.is_some() => {
            tracing::debug!("discovery unavailable: {e:#}");
            (None, mpsc::unbounded_channel().1)
        }
        Some(Err(e)) => return Err(e.context("could not start network discovery (try --connect <ip:port>)")),
    };

    out.say(
        || format!("This computer: {} ({}), pairing port {port}", identity.name, me.short()),
        || format!("me\t{}\t{}\t{port}", identity.name, me.short()),
    );
    if !running && !out.machine {
        println!(
            "one-mk isn't running here, so the other computer will look for it on port {} until they first connect.",
            one_mk_proto::DEFAULT_PORT
        );
    }
    out.say(
        || match connect {
            Some(addr) => format!("Connecting to {addr}..."),
            None if !discovery => format!("Discovery is off, so nothing is searched for. Waiting for the other computer to connect here; on it run:  one-mk pair --connect <this computer's IP>:{port}"),
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
                match session(dir, peer, &identity, &mut trust, &mut answers, out).await {
                    Ok(()) => break,
                    Err(e) => {
                        crate::analytics::record(dir, crate::analytics::Event::PairingFinished, crate::analytics::Origin::Cli,
                            crate::analytics::Outcome::Failed, crate::analytics::error_code(&e));
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
async fn session(dir: &Path, peer: Peer, identity: &Identity, trust: &mut TrustStore, answers: &mut Answers, out: Out) -> Result<()> {
    let fingerprint = peer.fingerprint;
    let ip = peer.conn.remote_address().ip().to_canonical();
    out.say(
        || format!("\nPairing with a computer at {ip} (id {})...", fingerprint.short()),
        || format!("peer\t{}\t{ip}", fingerprint.short()),
    );
    let res = exchange(dir, &peer, identity, trust, answers, out).await;
    if res.is_err() {
        peer.conn.close(0u32.into(), b"pairing failed");
    }
    res
}

async fn exchange(dir: &Path, peer: &Peer, identity: &Identity, trust: &mut TrustStore, answers: &mut Answers, out: Out) -> Result<()> {
    let fingerprint = peer.fingerprint;
    let port = service_port(dir).await?;
    let pairing = Pairing::start(peer, identity.fingerprint, &identity.name).await?;
    let name = pairing.peer_name.clone();

    // Everything the other computer chose (its name) comes first; the code
    // comes last, on its own line, so nothing printed after it can
    // disguise it.
    let twin = trust.devices().iter().any(|d| d.label().trim().eq_ignore_ascii_case(name.trim()));
    let twin_note = if twin { "  (A computer you already paired is called the same; once paired they are told apart by id, and `one-mk rename` gives either a name of your own.)\n" } else { "" };
    out.say(
        || format!("  The other computer says its name is: {name}\n{twin_note}  Check that it shows this same code:\n\n      {}\n", pairing.sas),
        || format!("code\t{name}\t{}", pairing.sas),
    );
    let answer_within = answer_timeout();
    let ok = match answers.ask("  Do the codes match? [y/N] ", answer_within).await {
        Some(ok) => ok,
        None => {
            let _ = tokio::time::timeout(Duration::from_secs(2), pairing.confirm(false)).await;
            bail!("no answer within {}", seconds(answer_within));
        }
    };
    if !ok {
        let _ = tokio::time::timeout(Duration::from_secs(2), pairing.confirm(false)).await;
        crate::analytics::record(dir, crate::analytics::Event::PairingFinished, crate::analytics::Origin::Cli,
            crate::analytics::Outcome::Cancelled, crate::analytics::ErrorCode::None);
        out.say(|| "Pairing cancelled.".into(), || "cancelled".into());
        return Ok(());
    }
    out.say(|| "  Waiting for the other computer to confirm...".into(), || "waiting".into());
    let (confirmed, peer_port) = tokio::time::timeout(answer_within, pairing.confirm_port(true, Some(port)))
        .await
        .context("the other computer did not answer in time")?
        .context("the other computer went away")?;
    if !confirmed {
        crate::analytics::record(dir, crate::analytics::Event::PairingFinished, crate::analytics::Origin::Cli,
            crate::analytics::Outcome::Rejected, crate::analytics::ErrorCode::None);
        out.say(|| "The other computer did not confirm. Nothing was saved.".into(), || "declined".into());
        return Ok(());
    }

    let addr = SocketAddr::new(peer.conn.remote_address().ip().to_canonical(), peer_port.unwrap_or(one_mk_proto::DEFAULT_PORT));
    let (placement, fresh) = TrustStore::update(dir, |t| {
        let placement = crate::pairing_layout::remember(t, fingerprint, &name, addr, peer.initiator);
        Ok((placement, t.clone()))
    })?;
    *trust = fresh;
    crate::analytics::record(dir, crate::analytics::Event::PairingFinished, crate::analytics::Origin::Cli,
        crate::analytics::Outcome::Success, crate::analytics::ErrorCode::None);
    let side = placement.side();
    let _ = crate::control::request(dir, crate::control::Request::Reload).await?;
    let (shown, arg) = trust.get(&fingerprint).map_or_else(|| (name.clone(), format!("{:?}", name)), |d| (trust.display(d), trust.arg(d)));
    if placement.placed_by == one_mk_net::PlacedBy::Pending {
        let warning = crate::pairing_layout::warning(&shown, &arg);
        out.say(|| warning.clone(), || format!("placement-needed\t{fingerprint}\t{warning}"));
    }
    let side = trust.get(&fingerprint).map_or(side, |d| d.side());
    let twin_id = trust.get(&fingerprint).filter(|d| trust.label_is_shared(d)).map(Device::short_id);
    out.say(
        || paired_text(&shown, &arg, side, placement.placed_by == one_mk_net::PlacedBy::Pending, twin_id.as_deref()),
        || format!("paired\t{fingerprint}\t{name}\t{}", if placement.placed_by == one_mk_net::PlacedBy::Pending { "unplaced".to_owned() } else { side.to_string() }),
    );
    Ok(())
}

/// What a person is told once paired. `arg` names the computer in a
/// command (its id where its name would fit another computer too);
/// `twin_id`: set when another paired computer is shown under the same
/// name, to say how to tell them apart.
fn paired_text(shown: &str, arg: &str, side: one_mk_proto::Side, pending: bool, twin_id: Option<&str>) -> String {
    if pending {
        return format!("\nPaired with {shown}. Choose its placement before sharing through a screen edge.\nStart sharing: one-mk run (if it is not already running on this computer)");
    }
    let apart = twin_id.map_or_else(String::new, |id| format!("To tell it apart:       one-mk rename {id} \"a name of your own\"\n"));
    format!(
        "\nPaired with {shown}. For now it is {}; once both computers run one-mk they agree on one arrangement \
         (see `one-mk status`).\n\
         To choose where it is:  one-mk layout {arg} left|right|top|bottom\n\
         {apart}\
         Start sharing:          one-mk run   (if it is not already running on this computer)",
        one_mk_ctl::status::on_your(side)
    )
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

/// Dials `--connect`, retrying for `step_timeout()`, then reports a clear
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
        let window = step_timeout();
        let deadline = Instant::now() + window;
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
            "could not reach {addr} within {} ({last}); check that `one-mk pair` is running there \
             and that the address and port match what it prints",
            seconds(window)
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

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::Side;

    #[test]
    fn the_times_in_the_messages_are_what_was_waited_for() {
        assert_eq!(seconds(Duration::from_secs(30)), "30 s");
        assert_eq!(seconds(Duration::from_secs(120)), "120 s");
        assert_eq!(seconds(Duration::from_millis(1500)), "1.5 s");
        assert_eq!(seconds(Duration::from_millis(2000)), "2 s");
        // The defaults, as a person is told them.
        assert_eq!(seconds(step_timeout()), "30 s");
        assert_eq!(seconds(answer_timeout()), "120 s");
    }

    /// Advice that names a computer must name it in a way that finds it:
    /// by id when its name fits another computer too, and it says how to
    /// tell the two apart.
    #[test]
    fn the_advice_after_pairing_names_the_computer_the_way_that_works() {
        let alone = paired_text("Laptop", "Laptop", Side::Left, false, None);
        assert!(alone.contains("one-mk layout Laptop left|right|top|bottom") && !alone.contains("rename"), "{alone}");
        let twin = paired_text("Laptop (ab12-cd34)", "ab12-cd34", Side::Left, false, Some("ab12-cd34"));
        assert!(twin.contains("one-mk layout ab12-cd34 left|right|top|bottom"), "{twin}");
        assert!(twin.contains("one-mk rename ab12-cd34 \"a name of your own\""), "{twin}");
        assert!(!twin.contains("layout \"Laptop\""), "{twin}");
        let pending = paired_text("Laptop (ab12-cd34)", "ab12-cd34", Side::Left, true, Some("ab12-cd34"));
        assert!(pending.contains("Choose its placement"), "{pending}");
        // "Start sharing" is not an instruction to run it where it already runs.
        assert!(!alone.contains("only on computers not already running"), "{alone}");
    }
}
