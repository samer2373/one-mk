//! Invitation commands and the private, process-local UI bridge.
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use one_mk_ctl::client::Request;
use one_mk_ctl::status::InvitationStatus;
use one_mk_net::{Device, Discovery, Identity, Node, Peer, TrustStore};
use one_mk_net::invite::{Invitation, InvitationState, InviteSession, MAX_LINK, STEP_TIMEOUT};
use one_mk_proto::{DEFAULT_PORT, Side, sanitize_name};
use std::io::{BufRead, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

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
        if addrs.is_empty() { addrs.push(SocketAddr::from(([127, 0, 0, 1], local.port()))); }
        addrs
    };
    let invitation = Invitation::new(identity.fingerprint, addrs, args.service_port, args.expires)?;
    let state = Arc::new(InvitationState::new(&invitation));
    let status = InvitationStatus { link: invitation.link(), expires: invitation.expires };
    drop(invitation);
    // Browse-only would not advertise A's name. Use the same pairing
    // advertisement as `pair`; numeric-pairing clients fail closed.
    let _discovery = if args.no_discovery { None } else {
        match Discovery::start(identity, local.port(), true) {
            Ok((d, _)) => Some(d),
            Err(e) => { eprintln!("mDNS unavailable; use the link's direct addresses: {e}"); None }
        }
    };
    println!("Invitation from {} ({})", sanitize_name(&identity.name), identity.fingerprint);
    println!("Expires at Unix time {}. One use; closing this process cancels it.", status.expires);
    println!("Treat this link like a password: whoever uses it first can request trust.");
    println!("{}", status.link);
    print_qr(&status.link)?;
    println!("{}", if args.no_confirm { "WARNING: --no-confirm: the first valid joiner will be trusted automatically." } else { "Approval required here: you will see the joiner's name and fingerprint before trusting it." });
    std::io::stdout().flush()?;

    let pending = crate::app::Pending::default();
    let mut attempts = JoinSet::new();
    let end = tokio::time::Instant::from_std(state.deadline);
    let result = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { break Ok(()); }
            _ = tokio::time::sleep_until(end) => { break Err(anyhow::anyhow!("invitation expired; create a new one")); }
            Some((req, reply)) = control.recv() => {
                let text = match req {
                    Request::Invitation if state.active() => status.encode(),
                    Request::Status => "invitation process\n".into(),
                    _ => "none\n".into(),
                };
                let _ = reply.send(text);
            }
            Some(accepted) = node.accept(), if state.active() => {
                let peer = accepted?;
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
                        attempts.abort_all();
                        // Continue serving the UI while approval is pending, but
                        // never expose the already-burned invitation again.
                        let decision = approve_and_save(dir, &peer, session, !args.no_confirm, end, status.expires);
                        tokio::pin!(decision);
                        loop {
                            tokio::select! {
                                res = &mut decision => break res,
                                _ = tokio::signal::ctrl_c() => break Ok(()),
                                Some((_, reply)) = control.recv() => { let _ = reply.send("none\n".into()); }
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
    attempts.abort_all();
    result
}

async fn approve_and_save(dir: &Path, peer: &Peer, session: InviteSession, confirm: bool, expiry: tokio::time::Instant, wall_expiry: u64) -> Result<()> {
    let name = session.peer_name.clone();
    let port = session.service_port;
    println!("\n>>> {} joined: certificate {} <<<", name, peer.fingerprint);
    println!("Invitation consumed. This device is NOT trusted yet.");
    std::io::stdout().flush()?;
    let end = expiry.min(tokio::time::Instant::now() + Duration::from_secs(120));
    let accept = if confirm {
        println!("Trust this device with your keyboard, mouse and shared clipboard? [y/N]");
        std::io::stdout().flush()?;
        tokio::time::timeout_at(end, read_stdin(64)).await.ok().and_then(Result::ok).is_some_and(|s| s.trim().eq_ignore_ascii_case("y"))
    } else { true };
    ensure!(tokio::time::Instant::now() < end && one_mk_net::invite::unix_now() < wall_expiry, "invitation approval expired");
    let ok = tokio::time::timeout_at(end, session.confirm(accept)).await.context("invitation approval expired")??;
    if !ok { bail!("device refused; invitation consumed; nothing saved here"); }
    ensure!(tokio::time::Instant::now() < expiry && one_mk_net::invite::unix_now() < wall_expiry, "invitation approval expired");
    save(dir, peer, &name, port, Side::Left).await?;
    println!("Trusted {name} ({}).", peer.fingerprint);
    Ok(())
}

pub async fn join(dir: &Path, args: JoinArgs) -> Result<()> {
    let text = match args.link {
        Some(s) => s,
        None => { eprintln!("Paste the invitation link, then Enter:"); read_stdin(MAX_LINK + 1).await? }
    };
    let invitation = Invitation::parse(text.trim())?;
    ensure!(!invitation.is_expired(), "invitation expired");
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let node = Node::bind_any(identity.clone(), 0)?;
    let result = join_inner(dir, &node, &identity, &invitation, args.service_port).await;
    node.shutdown(Duration::from_millis(200)).await;
    result
}

async fn join_inner(dir: &Path, node: &Node, identity: &Identity, invitation: &Invitation, service_port: u16) -> Result<()> {
    let mut addrs = invitation.addrs.clone();
    // mDNS is an untrusted fallback. Only a matching TLS certificate can
    // receive our proof, regardless of which address found the server.
    if let Ok(Ok(resolved)) = tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host((invitation.mdns.as_str(), invitation.addrs[0].port()))).await {
        for addr in resolved.take(8) { if !addrs.contains(&addr) { addrs.push(addr); } }
    }
    let mut connected = None;
    for addr in addrs {
        match node.connect_timeout(addr, Duration::from_secs(3)).await {
            Ok(peer) if peer.fingerprint == invitation.fingerprint => { connected = Some(peer); break; }
            Ok(peer) => peer.conn.close(1u32.into(), b"certificate mismatch"),
            Err(_) => {}
        }
    }
    let peer = connected.context("could not reach the invitation's pinned computer (offline, expired, or certificate mismatch)")?;
    let session = tokio::time::timeout(STEP_TIMEOUT, InviteSession::join(&peer, identity.fingerprint, &identity.name, service_port, invitation)).await.context("invitation exchange timed out")??;
    let name = session.peer_name.clone();
    let port = session.service_port;
    println!("Authenticated {name} ({}). Waiting for approval there...", peer.fingerprint);
    let remaining = Duration::from_secs(invitation.expires.saturating_sub(one_mk_net::invite::unix_now())).min(Duration::from_secs(120));
    ensure!(tokio::time::timeout(remaining, session.confirm(true)).await.context("invitation approval expired")??, "invitation declined");
    ensure!(!invitation.is_expired(), "invitation approval expired");
    save(dir, &peer, &name, port, Side::Right).await?;
    println!("Trusted {name} ({}). Start `one-mk run` on both computers.", peer.fingerprint);
    Ok(())
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

async fn read_stdin(limit: usize) -> Result<String> {
    // A dedicated OS thread can be abandoned at process exit; tokio stdin
    // would keep runtime shutdown blocked when an invitation expires.
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
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
    fn bounded_expiry() {
        for (s, seconds) in [("1s", 1), ("10m", 600), ("1h", 3600), ("120", 120)] { assert_eq!(duration(s).unwrap().as_secs(), seconds); }
        for s in ["", "0", "0m", "2h", "-1", "1d", "18446744073709551615h", "é"] { assert!(duration(s).is_err()); }
    }
}
