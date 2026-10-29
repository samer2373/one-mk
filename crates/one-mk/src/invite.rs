//! Invitation commands and the private, process-local UI bridge.
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use one_mk_ctl::client::Request;
use one_mk_net::{Device, Discovery, Identity, Node, Peer, TrustStore};
use one_mk_net::invite::{Invitation, InvitationState, InviteSession, MAX_LINK, STEP_TIMEOUT};
use one_mk_proto::{DEFAULT_PORT, Side, sanitize_name};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use zeroize::{Zeroize, Zeroizing};

#[derive(Args)]
pub struct InviteArgs {
    /// Invitation lifetime: e.g. 30s, 10m, 1h (maximum 1 hour).
    #[arg(long, default_value = "10m", value_parser = duration)]
    expires: Duration,
    /// Require y on this computer before trusting the joiner (the default).
    #[arg(long, conflicts_with = "no_confirm")]
    confirm: bool,
    /// Trust the first valid joiner without a local prompt. Protect the link!
    #[arg(long)]
    no_confirm: bool,
    /// Pairing listener address; default is all interfaces, a free port.
    #[arg(long)]
    listen: Option<SocketAddr>,
    /// Skip mDNS (the link still contains direct addresses).
    #[arg(long)]
    no_discovery: bool,
    /// Port used later by `run` on this computer.
    #[arg(long, default_value_t = DEFAULT_PORT, value_parser = clap::value_parser!(u16).range(1..))]
    service_port: u16,
}

#[derive(Args)]
pub struct JoinArgs {
    /// Invitation link. Omit to read one line from stdin (avoids shell history).
    link: Option<String>,
    /// Trust the pinned host without prompting (required for non-terminal stdin).
    #[arg(long)]
    yes: bool,
    /// Port used later by `run` on this computer.
    #[arg(long, default_value_t = DEFAULT_PORT, value_parser = clap::value_parser!(u16).range(1..))]
    service_port: u16,
}

fn duration(text: &str) -> Result<Duration, String> {
    let (n, scale) = match text.as_bytes().last() {
        Some(b's') => (&text[..text.len() - 1], 1),
        Some(b'm') => (&text[..text.len() - 1], 60),
        Some(b'h') => (&text[..text.len() - 1], 3600),
        _ => (text, 1),
    };
    let seconds = n.parse::<u64>().ok().and_then(|n| n.checked_mul(scale)).filter(|n| (1..=3600).contains(n));
    seconds.map(Duration::from_secs).ok_or_else(|| "use 1s to 1h (default 10m)".to_owned())
}

fn bridge_dir(dir: &Path) -> PathBuf { dir.join("invitation") }

/// A separate control endpoint keeps the link in the invite process's
/// memory. The daemon forwards this one request; no secret file is needed.
pub fn query(dir: &Path, reply: oneshot::Sender<String>) {
    let dir = bridge_dir(dir);
    tokio::spawn(async move {
        let text = crate::control::request(&dir, Request::Invitation).await.ok().flatten().unwrap_or_else(|| "none\n".into());
        let _ = reply.send(text);
    });
}

pub async fn run(dir: &Path, args: InviteArgs) -> Result<()> {
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let bridge = bridge_dir(dir);
    std::fs::create_dir_all(&bridge)?;
    #[cfg(unix)]
    std::fs::set_permissions(&bridge, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(bridge.join("lock"))?;
    lock.try_lock().context("an invitation is already open for this computer")?;
    let mut control = crate::control::Server::start(&bridge).await?;
    let node = match args.listen {
        Some(addr) => Node::bind(identity.clone(), addr)?,
        None => Node::bind_any(identity.clone(), 0)?,
    };
    let result = serve(dir, &args, &identity, &node, &mut control).await;
    node.shutdown(Duration::from_millis(200)).await;
    result
}

async fn serve(dir: &Path, args: &InviteArgs, identity: &Arc<Identity>, node: &Node, control: &mut crate::control::Server) -> Result<()> {
    let local = node.local_addr()?;
    let addrs = if !local.ip().is_unspecified() { vec![local] } else {
        let mut addrs: Vec<_> = if_addrs::get_if_addrs()?.into_iter()
            .filter(|i| !i.is_loopback() && !one_mk_net::is_virtual_interface(&i.name))
            .map(|i| SocketAddr::new(i.ip(), local.port()))
            .filter(|a| !a.ip().is_unspecified() && !a.ip().is_multicast() && !matches!(a.ip(), std::net::IpAddr::V6(ip) if ip.is_unicast_link_local()))
            .collect();
        addrs.sort();
        addrs.dedup();
        addrs.truncate(8);
        addrs
    };
    let invitation = Invitation::new(identity.fingerprint, addrs, args.service_port, args.expires)?;
    let state = Arc::new(InvitationState::new(&invitation));
    let mut link = invitation.link();
    let wall_expiry = invitation.expires;
    drop(invitation);
    // Browse-only would not advertise A's name. Use the same pairing
    // advertisement as `pair`; numeric-pairing clients fail closed.
    let mut discovery = if args.no_discovery { None } else {
        match Discovery::start(identity, local.port(), true) {
            Ok((d, _)) => Some(d),
            Err(e) => { eprintln!("mDNS unavailable; use the link's direct addresses: {e}"); None }
        }
    };
    println!("Invitation from {} ({})", sanitize_name(&identity.name), identity.fingerprint);
    println!("Expires at Unix time {wall_expiry}. One use; closing this process cancels it.");
    println!("Treat this link like a password: whoever uses it first can request trust.");
    println!("{}", link.as_str());
    print_qr(&link)?;
    println!("{}", if args.no_confirm { "WARNING: --no-confirm: the first valid joiner will be trusted automatically." } else { "Approval required here: you will see the joiner's name and fingerprint before trusting it." });
    std::io::stdout().flush()?;

    let pending = crate::app::Pending::default();
    let mut attempts = JoinSet::new();
    let mut sources = SourceLimit::default();
    let mut clock = tokio::time::interval(Duration::from_millis(250));
    let end = tokio::time::Instant::from_std(state.deadline);
    let result = loop {
        if state.expired() {
            break Err(anyhow::anyhow!("invitation expired; create a new one"));
        }
        if !state.active() {
            link.zeroize();
            withdraw(&mut discovery).await;
            if attempts.is_empty() { break Err(anyhow::anyhow!("invitation burned or expired; create a new one")); }
        }
        tokio::select! {
            _ = clock.tick() => {}
            _ = tokio::signal::ctrl_c() => { break Ok(()); }
            _ = tokio::time::sleep_until(end) => { break Err(anyhow::anyhow!("invitation expired; create a new one")); }
            Some((req, reply)) = control.recv() => {
                let text = match req {
                    Request::Invitation if state.active() => format!("invitation\t{wall_expiry}\t{}\n", link.as_str()),
                    Request::Status => "invitation process\n".into(),
                    _ => "none\n".into(),
                };
                let _ = reply.send(text);
            }
            Some(accepted) = node.accept() => {
                let peer = accepted?;
                if !state.active() {
                    peer.conn.close(3u32.into(), b"invitation already used or expired");
                    continue;
                }
                if !sources.admit(peer.conn.remote_address().ip()) {
                    peer.conn.close(4u32.into(), b"invitation busy");
                    continue;
                }
                let Some(permit) = pending.admit(peer.fingerprint) else {
                    peer.conn.close(4u32.into(), b"busy");
                    continue;
                };
                let (identity, state) = (identity.clone(), state.clone());
                let service_port = args.service_port;
                attempts.spawn(async move {
                    let exchange = tokio::time::timeout(STEP_TIMEOUT, InviteSession::accept(&peer, identity.fingerprint, &identity.name, service_port, &state)).await;
                    (peer, exchange, permit)
                });
            }
            Some(done) = attempts.join_next(), if !attempts.is_empty() => {
                let (peer, exchange, _permit) = done?;
                match exchange {
                    Ok(Ok(session)) => {
                        link.zeroize();
                        withdraw(&mut discovery).await;
                        attempts.abort_all();
                        // Continue serving the UI while approval is pending, but
                        // never expose the already-burned invitation again.
                        let decision = approve_and_save(dir, &peer, session, !args.no_confirm, end, wall_expiry);
                        tokio::pin!(decision);
                        loop {
                            tokio::select! {
                                res = &mut decision => break res,
                                _ = tokio::signal::ctrl_c() => break Ok(()),
                                Some((_, reply)) = control.recv() => { let _ = reply.send("none\n".into()); }
                                Some(accepted) = node.accept() => { accepted?.conn.close(3u32.into(), b"invitation already used or expired"); }
                            }
                        }?;
                        break Ok(());
                    }
                    _ => {
                        peer.conn.close(1u32.into(), b"invitation failed");
                        eprintln!("Invitation attempt rejected.");
                        if !state.active() && attempts.is_empty() { break Err(anyhow::anyhow!("invitation burned or expired; create a new one")); }
                    }
                }
            }
        }
    };
    state.burn();
    link.zeroize();
    withdraw(&mut discovery).await;
    attempts.abort_all();
    result
}

async fn approve_and_save(dir: &Path, peer: &Peer, mut session: InviteSession, confirm: bool, expiry: tokio::time::Instant, wall_expiry: u64) -> Result<()> {
    let name = session.peer_name.clone();
    let port = session.service_port;
    println!("\n>>> {} joined: certificate {} <<<", name, grouped(peer.fingerprint));
    println!("Invitation consumed. This device is NOT trusted yet.");
    std::io::stdout().flush()?;
    let end = expiry.min(tokio::time::Instant::now() + Duration::from_secs(120));
    let accept = if confirm {
        println!("Does the other computer show {}? Trust it with your keyboard, mouse and shared clipboard? [y/N]", session.code);
        std::io::stdout().flush()?;
        read_approval(end, Some(wall_expiry)).await
    } else { println!("WARNING: --no-confirm skips comparing code {} and approves automatically.", session.code); true };
    let accept = accept && tokio::time::Instant::now() < end && one_mk_net::invite::unix_now() < wall_expiry;
    let ok = tokio::time::timeout_at(end, session.confirm(accept)).await.context("invitation approval expired")??;
    if !ok { bail!("device refused; invitation consumed; nothing saved here"); }
    save(dir, peer, &name, port, Side::Left).await?;
    acknowledge(session).await?;
    println!("Trusted {name} ({}).", peer.fingerprint);
    Ok(())
}

pub async fn join(dir: &Path, args: JoinArgs) -> Result<()> {
    ensure!(args.yes || std::io::stdin().is_terminal(), "non-interactive stdin requires --yes; this skips host trust approval");
    let text = match args.link {
        Some(s) => { eprintln!("WARNING: positional invitation links are visible in process arguments and may remain in shell history; prefer `one-mk join`."); Zeroizing::new(s) }
        None => { eprintln!("Paste the invitation link, then Enter:"); read_stdin(MAX_LINK + 1).await? }
    };
    let invitation = Invitation::parse(text.trim())?;
    drop(text);
    ensure!(!invitation.is_expired(), "invitation expired");
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let node = Node::bind_any(identity.clone(), 0)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut result = join_inner(dir, &node, &identity, &invitation, args.service_port, args.yes, deadline).await;
    if result.as_ref().is_err_and(|error| format!("{error:#}").contains("busy")) {
        tokio::time::sleep(Duration::from_millis(150 + u64::from(rand::random::<u8>()))).await;
        result = join_inner(dir, &node, &identity, &invitation, args.service_port, args.yes, deadline).await;
    }
    node.shutdown(Duration::from_millis(200)).await;
    result
}

async fn join_inner(dir: &Path, node: &Node, identity: &Identity, invitation: &Invitation, service_port: u16, yes: bool, deadline: tokio::time::Instant) -> Result<()> {
    let mut connected = None;
    let mut tried = HashSet::new();
    for addr in &invitation.addrs {
        if tokio::time::Instant::now() >= deadline { break; }
        if !tried.insert(*addr) { continue; }
        let end = deadline.min(tokio::time::Instant::now() + Duration::from_secs(2));
        match tokio::time::timeout_at(end, node.connect_pinned(*addr, invitation.fingerprint)).await {
            Ok(Ok(peer)) => { connected = Some(peer); break; }
            Ok(Err(error)) if format!("{error:#}").contains("busy") => return Err(error),
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline { break; }
    }
    if connected.is_none() && tried.len() < 8 && tokio::time::Instant::now() < deadline
        && let Ok((discovery, mut found)) = Discovery::browse(identity) {
        let end = deadline.min(tokio::time::Instant::now() + Duration::from_secs(2));
        'browse: while let Ok(Some(found)) = tokio::time::timeout_at(end, found.recv()).await {
            if found.fingerprint != invitation.fingerprint || !found.pairing { continue; }
            for addr in found.addrs {
                if !one_mk_net::invite::usable_address(addr.ip()) || tried.contains(&addr) { continue; }
                if tried.len() >= 8 || tokio::time::Instant::now() >= end { break 'browse; }
                tried.insert(addr);
                let attempt_end = end.min(tokio::time::Instant::now() + Duration::from_secs(2));
                match tokio::time::timeout_at(attempt_end, node.connect_pinned(addr, invitation.fingerprint)).await {
                    Ok(Ok(peer)) => { connected = Some(peer); break 'browse; }
                    Ok(Err(error)) if format!("{error:#}").contains("busy") => { discovery.stop().await; return Err(error); }
                    _ => {}
                }
            }
        }
        discovery.stop().await;
    }
    let peer = connected.context("could not reach the invitation's pinned computer (offline, expired, or certificate mismatch)")?;
    let mut session = tokio::time::timeout(STEP_TIMEOUT, InviteSession::join(&peer, identity.fingerprint, &identity.name, service_port, invitation)).await.context("invitation exchange timed out")??;
    let name = session.peer_name.clone();
    let port = session.service_port;
    println!("Authenticated {name} ({}).", grouped(peer.fingerprint));
    println!("Your code: {} (the other computer will ask you to compare it)", session.code);
    let end = tokio::time::Instant::now() + Duration::from_secs(120);
    let accept = if yes {
        println!("WARNING: --yes skips the host trust prompt; verify the code on the host unless it uses --no-confirm.");
        true
    } else {
        println!("Trust {name} ({})? [y/N]", grouped(peer.fingerprint));
        std::io::stdout().flush()?;
        read_approval(end, None).await
    };
    std::io::stdout().flush()?;
    ensure!(tokio::time::timeout_at(end, session.confirm(accept)).await.context("invitation approval expired")??, "invitation declined; nothing saved here");
    save(dir, &peer, &name, port, Side::Right).await?;
    acknowledge(session).await?;
    println!("Trusted {name} ({}). Start `one-mk run` on both computers.", peer.fingerprint);
    Ok(())
}

async fn withdraw(discovery: &mut Option<Discovery>) {
    if let Some(discovery) = discovery.take() { discovery.stop().await; }
}

async fn read_approval(end: tokio::time::Instant, wall_expiry: Option<u64>) -> bool {
    let input = read_stdin(64);
    tokio::pin!(input);
    let mut clock = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            result = &mut input => return result.is_ok_and(|text| text.trim().eq_ignore_ascii_case("y")),
            _ = tokio::time::sleep_until(end) => return false,
            _ = clock.tick() => {
                if wall_expiry.is_some_and(|expiry| one_mk_net::invite::unix_now() >= expiry) { return false; }
            }
        }
    }
}

async fn acknowledge(session: InviteSession) -> Result<()> {
    tokio::time::timeout(STEP_TIMEOUT, session.committed()).await.context("Trust saved locally, but peer commit is uncertain. Run `one-mk forget <peer>` on both computers and create a new invitation.")?
        .context("Trust saved locally, but peer commit is uncertain. Run `one-mk forget <peer>` on both computers and create a new invitation.")
}

fn grouped(fingerprint: one_mk_net::Fingerprint) -> String {
    fingerprint.to_string().as_bytes().chunks(4).map(|part| std::str::from_utf8(part).expect("hex")).collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
struct SourceLimit(HashMap<std::net::IpAddr, (std::time::Instant, u16)>);

impl SourceLimit {
    fn admit(&mut self, source: std::net::IpAddr) -> bool {
        let now = std::time::Instant::now();
        self.0.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(1));
        if self.0.len() >= 256 { return false; }
        let (_, count) = self.0.entry(source.to_canonical()).or_insert((now, 0));
        *count = count.saturating_add(1);
        *count <= 32
    }
}

async fn save(dir: &Path, peer: &Peer, name: &str, port: u16, side: Side) -> Result<()> {
    // Reload immediately before writing, as ordinary pairing does not own
    // the daemon's in-memory trust store.
    let mut trust = TrustStore::load(dir)?;
    let addr = SocketAddr::new(peer.conn.remote_address().ip().to_canonical(), port);
    trust.upsert(Device { name: name.to_owned(), fingerprint: peer.fingerprint.to_string(), side: side.to_string(), addrs: vec![addr.to_string()], ..Device::default() });
    trust.save()?;
    let _ = crate::control::request(dir, Request::Reload).await;
    Ok(())
}

async fn read_stdin(limit: usize) -> Result<Zeroizing<String>> {
    // A dedicated OS thread can be abandoned at process exit; tokio stdin
    // would keep runtime shutdown blocked when an invitation expires.
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let mut text = Zeroizing::new(String::new());
        let result = std::io::stdin().lock().take(limit as u64).read_line(&mut text).map(|_| text);
        let _ = tx.send(result);
    });
    Ok(rx.await??)
}

fn print_qr(link: &str) -> Result<()> {
    let qr = qrcode::QrCode::new(link.as_bytes()).context("encoding invitation QR")?;
    let n = qr.width() as isize;
    // Four-module quiet zone, two modules per terminal row. Explicit
    // black/white ANSI colours work on both light and dark terminals.
    for y in (-4..n + 4).step_by(2) {
        for x in -4..n + 4 {
            let dark = |y| x >= 0 && y >= 0 && x < n && y < n && qr[(x as usize, y as usize)] == qrcode::Color::Dark;
            let top = if dark(y) { 30 } else { 97 };
            let bottom = if dark(y + 1) { 40 } else { 107 };
            print!("\x1b[{top};{bottom}m▀");
        }
        println!("\x1b[0m");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_limits_are_bounded_and_do_not_change_invitations() {
        let mut limit = SourceLimit::default();
        let source = "192.0.2.1".parse().unwrap();
        for _ in 0..32 { assert!(limit.admit(source)); }
        for _ in 0..2000 { assert!(!limit.admit(source)); }
        assert!(limit.admit("192.0.2.2".parse().unwrap()));
        limit.0.get_mut(&source).unwrap().0 -= Duration::from_secs(1);
        assert!(limit.admit(source));
        for address in 0..300 { limit.admit(std::net::Ipv4Addr::from(0xc6336400 + address).into()); }
        assert!(limit.0.len() <= 256);
    }

    #[test]
    fn fingerprint_prompt_contains_all_grouped_hex_digits() {
        let fingerprint = one_mk_net::Fingerprint([0xab; 32]);
        let text = grouped(fingerprint);
        assert_eq!(text.replace(' ', ""), fingerprint.to_string());
        assert_eq!(text.split(' ').count(), 16);
    }
    #[test]
    fn bounded_expiry() {
        for (s, seconds) in [("1s", 1), ("10m", 600), ("1h", 3600), ("120", 120)] { assert_eq!(duration(s).unwrap().as_secs(), seconds); }
        for s in ["", "0", "0m", "2h", "-1", "1d", "18446744073709551615h", "é"] { assert!(duration(s).is_err()); }
    }
}
