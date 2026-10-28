//! The daemon: owns the local input backend and all peer links, and decides
//! which machine the pointer is on.

use crate::clip::ClipSync;
use crate::link::{self, Link, LinkEvent};
use crate::transfer::Inbox;
use anyhow::{Context, Result};
use one_mk_input::{Backend, CaptureCmd, Event, Inject};
use one_mk_net::{ControlWriter, Discovery, Fingerprint, Found, Identity, Node, Peer, TrustStore, read_stream_kind};
use one_mk_proto::{Control, Datagram, Desktop, HeldKeys, Pos, Rect, RemoteCursor, Side, Step, StreamKind};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

/// While the pointer is on a peer, send something at least this often so
/// Wi-Fi radios never doze off between mouse movements.
const KEEPALIVE: Duration = Duration::from_millis(20);
/// After the mouse stops, resend the final position twice, in case the
/// last motion datagram was lost.
const RESEND_AFTER: [Duration; 2] = [Duration::from_millis(4), Duration::from_millis(16)];
const DIAL_INTERVAL: Duration = Duration::from_secs(2);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a peer that connected to us may take to open its first stream.
const FIRST_STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// evdev codes for the emergency "bring my pointer home" shortcut:
/// Ctrl+Alt+Shift+Esc.
const KEY_ESC: u32 = 1;
const CTRL: [u32; 2] = [29, 97];
const ALT: [u32; 2] = [56, 100];
const SHIFT: [u32; 2] = [42, 54];

pub struct Options {
    pub backend: one_mk_input::Kind,
    pub port: u16,
    pub connect: Vec<SocketAddr>,
    pub discovery: bool,
    /// Clipboard to share; `None` turns clipboard sharing off.
    pub clipboard: Option<one_mk_clipboard::Clipboard>,
    /// Where received files go.
    pub downloads: PathBuf,
}

/// What peers can use besides input: clipboard and file streams.
#[derive(Clone)]
pub struct Services {
    pub clip: Option<Arc<ClipSync>>,
    pub inbox: Arc<Inbox>,
}

/// Serves the bulk streams a peer opens on `conn` until it closes, each on
/// its own task. On a transfer-only connection (`link == false`) only
/// file streams are accepted.
pub async fn serve_streams(conn: quinn::Connection, fp: Fingerprint, name: String, services: Services, link: bool) {
    while let Ok((send, mut recv)) = conn.accept_bi().await {
        let (services, name) = (services.clone(), name.clone());
        tokio::spawn(async move {
            let kind = match tokio::time::timeout(FIRST_STREAM_TIMEOUT, read_stream_kind(&mut recv)).await {
                Ok(Ok(k)) => k,
                Ok(Err(e)) => return tracing::debug!(peer = %name, "bad stream: {e:#}"),
                Err(_) => return,
            };
            serve_stream(kind, fp, &name, &services, link, send, recv).await;
        });
    }
}

async fn serve_stream(
    kind: StreamKind,
    fp: Fingerprint,
    name: &str,
    services: &Services,
    link: bool,
    send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) {
    match (kind, &services.clip) {
        (StreamKind::File, _) => services.inbox.receive(fp, name, send, recv).await,
        (StreamKind::Clipboard, Some(clip)) if link => clip.receive(fp, name, recv).await,
        (StreamKind::Clipboard, None) if link => {
            let _ = recv.stop(0u32.into()); // clipboard sharing is off here
        }
        _ => {
            tracing::debug!(peer = %name, "unexpected {kind:?} stream");
            let _ = recv.stop(0u32.into());
        }
    }
}

enum AppEvent {
    Connected(Link),
    Found(Found),
    DialDone(SocketAddr),
}

struct Connected {
    id: usize,
    name: String,
    initiator: bool,
    side: Side,
    conn: quinn::Connection,
    ctl: ControlWriter,
    desktop: Option<Desktop>,
}

/// We control a peer.
struct Remote {
    /// The capture activation this visit belongs to.
    activation: u32,
    peer: Fingerprint,
    /// Our edge the pointer left through.
    side: Side,
    cursor: RemoteCursor,
    held: HeldKeys,
    last_send: Instant,
    resend_at: Option<Instant>,
    resends: usize,
}

struct App {
    dir: PathBuf,
    node: Node,
    trust: TrustStore,
    peers: HashMap<Fingerprint, Connected>,
    remote: Option<Remote>,
    local: Option<Desktop>,
    local_screens: watch::Sender<Vec<Rect>>,
    epoch: u16,
    capture: mpsc::UnboundedSender<CaptureCmd>,
    inject: mpsc::UnboundedSender<Inject>,
    tx: mpsc::UnboundedSender<AppEvent>,
    link_tx: mpsc::UnboundedSender<(Fingerprint, usize, LinkEvent)>,
    sides: Vec<Side>,
    discovered: HashMap<Fingerprint, Vec<SocketAddr>>,
    extra_addrs: Vec<SocketAddr>,
    dialing: HashSet<SocketAddr>,
    services: Services,
}

pub async fn run(dir: PathBuf, opts: Options) -> Result<()> {
    let backend = one_mk_input::open(opts.backend, dir.clone()).await?;
    serve(dir, opts, backend, shutdown_signal()).await
}

/// Runs the daemon with a given input backend until `shutdown` completes.
pub async fn serve(
    dir: PathBuf,
    opts: Options,
    backend: Backend,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let identity = Arc::new(Identity::load_or_create(&dir)?);
    let trust = TrustStore::load(&dir)?;
    if trust.devices().is_empty() {
        tracing::warn!("no paired computers yet; run `one-mk pair` on this and the other computer");
    }
    let node = bind(identity.clone(), opts.port)?;
    let port = node.local_addr()?.port();
    tracing::info!(name = %identity.name, id = %identity.fingerprint.short(), port, "one-mk running");

    let Backend { mut events, capture, inject } = backend;
    let services = Services { clip: opts.clipboard.map(ClipSync::new), inbox: Inbox::new(opts.downloads.clone()) };
    tracing::info!(
        clipboard = services.clip.is_some(),
        downloads = %opts.downloads.display(),
        "sharing"
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (link_tx, mut link_rx) = mpsc::unbounded_channel();
    let (local_screens, screens_rx) = watch::channel(Vec::new());

    spawn_accept(node.clone(), dir.clone(), screens_rx.clone(), tx.clone(), services.clone());
    let _discovery = if opts.discovery {
        let (d, mut found) = Discovery::start(&identity, port, false)?;
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(f) = found.recv().await {
                if tx.send(AppEvent::Found(f)).is_err() {
                    break;
                }
            }
        });
        Some(d)
    } else {
        None
    };

    let mut app = App {
        dir,
        node: node.clone(),
        trust,
        peers: HashMap::new(),
        remote: None,
        local: None,
        local_screens,
        epoch: 0,
        capture,
        inject,
        tx,
        link_tx,
        sides: Vec::new(),
        discovered: HashMap::new(),
        extra_addrs: opts.connect,
        dialing: HashSet::new(),
        services,
    };
    app.dial_missing(&screens_rx);

    let mut dial_tick = tokio::time::interval(DIAL_INTERVAL);
    dial_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut input_open = true;
    tokio::pin!(shutdown);

    loop {
        let deadline = app.next_deadline();
        tokio::select! {
            biased;
            ev = events.recv(), if input_open => match ev {
                Some(ev) => app.on_input(ev),
                None => {
                    tracing::error!("input backend stopped");
                    input_open = false;
                }
            },
            Some((fp, id, ev)) = link_rx.recv() => app.on_link(fp, id, ev),
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => app.on_timer(),
            Some(ev) = rx.recv() => app.on_app(ev, &screens_rx),
            _ = dial_tick.tick() => app.dial_missing(&screens_rx),
            _ = &mut shutdown => break,
        }
    }

    if let Some(r) = app.remote.as_ref() {
        let t = r.cursor.home_fraction();
        app.go_home(t);
    }
    node.close();
    tokio::time::sleep(Duration::from_millis(50)).await; // let the Leave and close go out
    Ok(())
}

fn bind(identity: Arc<Identity>, port: u16) -> Result<Node> {
    // Dual-stack first; fall back to IPv4 where IPv6 is disabled.
    Node::bind(identity.clone(), SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)))
        .or_else(|_| Node::bind(identity, SocketAddr::from(([0, 0, 0, 0], port))))
        .with_context(|| format!("UDP port {port} is busy; is one-mk already running? (use --port)"))
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("signal handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
}

fn spawn_accept(
    node: Node,
    dir: PathBuf,
    screens: watch::Receiver<Vec<Rect>>,
    tx: mpsc::UnboundedSender<AppEvent>,
    services: Services,
) {
    tokio::spawn(async move {
        while let Some(res) = node.accept().await {
            let peer = match res {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!("incoming connection failed: {e:#}");
                    continue;
                }
            };
            let (node, dir, screens, tx, services) =
                (node.clone(), dir.clone(), screens.clone(), tx.clone(), services.clone());
            tokio::spawn(async move { establish(peer, &node, &dir, &screens, &tx, services).await });
        }
    });
}

/// Checks the peer is paired, then handshakes and hands the link to the app.
///
/// A connection we dialed always starts with a control stream. One that
/// was dialed to us may instead start with a file stream: that is `one-mk
/// send` on the peer, which shares the peer service's identity. It gets a
/// transfer-only connection, without a link, so it never replaces the
/// service's link.
async fn establish(
    peer: Peer,
    node: &Node,
    dir: &std::path::Path,
    screens: &watch::Receiver<Vec<Rect>>,
    tx: &mpsc::UnboundedSender<AppEvent>,
    services: Services,
) {
    // Re-read the store so devices paired while we run are accepted.
    let device = TrustStore::load(dir).ok().and_then(|t| t.get(&peer.fingerprint).map(|d| d.name.clone()));
    let Some(device_name) = device else {
        tracing::info!(peer = %peer.fingerprint.short(), "rejecting unpaired device");
        peer.conn.close(1u32.into(), b"not paired");
        return;
    };
    let addr = peer.conn.remote_address();
    let first = if peer.initiator {
        peer.open_stream(StreamKind::Control).await
    } else {
        match tokio::time::timeout(FIRST_STREAM_TIMEOUT, peer.accept_stream()).await {
            Ok(Ok((StreamKind::Control, send, recv))) => Ok((send, recv)),
            Ok(Ok((StreamKind::File, send, recv))) => {
                tracing::debug!(peer = %device_name, %addr, "transfer connection");
                let fp = peer.fingerprint;
                let conn = peer.conn;
                tokio::spawn(serve_streams(conn.clone(), fp, device_name.clone(), services.clone(), false));
                serve_stream(StreamKind::File, fp, &device_name, &services, false, send, recv).await;
                return;
            }
            Ok(Ok((kind, ..))) => Err(anyhow::anyhow!("unexpected first stream {kind:?}")),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow::anyhow!("peer opened no stream")),
        }
    };
    let res = match first {
        Ok(streams) => {
            let local = screens.borrow().clone();
            link::handshake(peer, streams, node.identity(), local).await
        }
        Err(e) => Err(e),
    };
    match res {
        Ok(link) => {
            let _ = tx.send(AppEvent::Connected(link));
        }
        Err(e) => tracing::warn!(%addr, "handshake failed: {e:#}"),
    }
}

impl App {
    // ------------------------------------------------------------ local input

    fn on_input(&mut self, ev: Event) {
        match ev {
            Event::Desktop(d) => {
                tracing::info!(screens = ?d.screens, "local screens");
                for p in self.peers.values() {
                    p.ctl.send(Control::Screens(d.screens.clone()));
                }
                let _ = self.local_screens.send(d.screens.clone());
                self.local = Some(d);
                self.update_sides();
            }
            Event::Activated { id, side, pos } => self.enter_peer(id, side, pos),
            Event::Motion { dx, dy } => {
                let Some(r) = self.remote.as_mut() else { return };
                match r.cursor.motion(dx, dy) {
                    Step::Move(d) => {
                        let now = Instant::now();
                        send_datagram(&self.peers, r.peer, d);
                        r.last_send = now;
                        r.resends = 0;
                        r.resend_at = Some(now + RESEND_AFTER[0]);
                    }
                    Step::Leave { t } => self.go_home(t),
                    Step::Still => {}
                }
            }
            Event::Button { button, down } => {
                let Some(r) = self.remote.as_mut() else { return };
                if r.held.update(button, down) {
                    let msg = Control::Button { button, down, seq: r.cursor.seq(), pos: r.cursor.pos() };
                    self.send_ctl(msg);
                }
            }
            Event::Key { code, down } => {
                let Some(r) = self.remote.as_mut() else { return };
                if down && code == KEY_ESC && [CTRL, ALT, SHIFT].iter().all(|k| k.iter().any(|&c| r.held.is_held(c))) {
                    tracing::info!("emergency shortcut: bringing the pointer home");
                    let t = r.cursor.home_fraction();
                    self.go_home(t);
                    return;
                }
                if r.held.update(code, down) {
                    self.send_ctl(Control::Key { code, down });
                }
            }
            Event::Scroll { dx, dy } => self.send_ctl(Control::Scroll { dx: dx as f32, dy: dy as f32 }),
            Event::ScrollDiscrete { dx, dy } => self.send_ctl(Control::ScrollDiscrete { dx, dy }),
            Event::ScrollStop => self.send_ctl(Control::ScrollStop),
            Event::LocalInput => {}
            Event::Deactivated { id } => {
                if let Some(r) = self.remote.take_if(|r| r.activation == id) {
                    tracing::info!("capture ended by the system; pointer is home");
                    if let Some(p) = self.peers.get(&r.peer) {
                        p.ctl.send(Control::Leave);
                    }
                }
            }
            Event::Error(msg) => tracing::error!("{msg}"),
        }
    }

    /// The pointer hit the `side` edge of our desktop.
    fn enter_peer(&mut self, activation: u32, side: Side, pos: Pos) {
        let target = self
            .peers
            .iter()
            .find(|(_, p)| p.side == side && p.desktop.is_some())
            .map(|(fp, p)| (*fp, p));
        let (Some(local), Some((fp, peer))) = (self.local.as_ref(), target) else {
            let _ = self.capture.send(CaptureCmd::Release { id: activation, pos: None });
            return;
        };
        self.epoch = self.epoch.wrapping_add(1);
        let t = local.edge_fraction(side, pos);
        let desktop = peer.desktop.clone().expect("checked above");
        let cursor = RemoteCursor::enter(desktop, side.opposite(), t, self.epoch);
        peer.ctl.send(Control::Enter { epoch: self.epoch, pos: cursor.pos() });
        tracing::debug!(peer = %peer.name, %side, "pointer moved to peer");
        if let Some(clip) = &self.services.clip {
            clip.push(fp, &peer.name, peer.conn.clone());
        }
        self.remote = Some(Remote {
            activation,
            peer: fp,
            side,
            cursor,
            held: HeldKeys::new(),
            last_send: Instant::now(),
            resend_at: None,
            resends: 0,
        });
    }

    /// Returns the pointer to this machine at fraction `t` along the edge.
    fn go_home(&mut self, t: f64) {
        let Some(r) = self.remote.take() else { return };
        if let Some(p) = self.peers.get(&r.peer) {
            // Leave makes the peer release anything still held.
            p.ctl.send(Control::Leave);
        }
        let pos = self.local.as_ref().map(|d| d.point_on_edge(r.side, t, 2.0));
        let _ = self.capture.send(CaptureCmd::Release { id: r.activation, pos });
        tracing::debug!("pointer is home");
    }

    fn send_ctl(&self, msg: Control) {
        if let Some(p) = self.remote.as_ref().and_then(|r| self.peers.get(&r.peer)) {
            p.ctl.send(msg);
        }
    }

    // ----------------------------------------------------------------- timers

    fn next_deadline(&self) -> Option<Instant> {
        let r = self.remote.as_ref()?;
        let keepalive = r.last_send + KEEPALIVE;
        Some(r.resend_at.map_or(keepalive, |t| t.min(keepalive)))
    }

    fn on_timer(&mut self) {
        let Some(r) = self.remote.as_mut() else { return };
        let now = Instant::now();
        if let Some(at) = r.resend_at
            && now >= at
        {
            send_datagram(&self.peers, r.peer, r.cursor.current());
            r.last_send = now;
            r.resends += 1;
            r.resend_at = RESEND_AFTER.get(r.resends).map(|d| now + (*d - RESEND_AFTER[r.resends - 1]));
        }
        if now >= r.last_send + KEEPALIVE {
            send_datagram(&self.peers, r.peer, Datagram::Keepalive);
            r.last_send = now;
        }
    }

    // ------------------------------------------------------------------ links

    fn on_app(&mut self, ev: AppEvent, screens: &watch::Receiver<Vec<Rect>>) {
        match ev {
            AppEvent::Connected(link) => self.on_connected(link),
            AppEvent::Found(f) => {
                if f.pairing {
                    return;
                }
                if !self.trust.is_trusted(&f.fingerprint) {
                    // It may have been paired since we started.
                    if let Ok(t) = TrustStore::load(&self.dir) {
                        self.trust = t;
                    }
                    if !self.trust.is_trusted(&f.fingerprint) {
                        return;
                    }
                }
                tracing::debug!(name = %f.name, addrs = ?f.addrs, "discovered");
                self.discovered.insert(f.fingerprint, f.addrs);
                self.dial_missing(screens);
            }
            AppEvent::DialDone(addr) => {
                self.dialing.remove(&addr);
            }
        }
    }

    fn on_connected(&mut self, link: Link) {
        let fp = link.fp;
        let me = self.node.identity().fingerprint;
        // Both sides may dial at once. Keep the connection dialed by the
        // lower fingerprint; both sides reach the same decision.
        let dialer = if link.initiator { me } else { fp };
        if let Some(existing) = self.peers.get(&fp) {
            let existing_dialer = if existing.initiator { me } else { fp };
            if existing_dialer == me.min(fp) || dialer != me.min(fp) {
                link.conn.close(2u32.into(), b"duplicate");
                return;
            }
        }
        if let Ok(t) = TrustStore::load(&self.dir) {
            self.trust = t;
        }
        let side = self.trust.get(&fp).map_or(Side::Right, |d| d.side());
        let addr = link.conn.remote_address();
        let addr = SocketAddr::new(addr.ip().to_canonical(), addr.port());
        if let Some(dev) = self.trust.get_mut(&fp) {
            let a = addr.to_string();
            if dev.addrs.first() != Some(&a) {
                dev.addrs.retain(|x| *x != a);
                dev.addrs.insert(0, a);
                dev.addrs.truncate(4);
                if let Err(e) = self.trust.save() {
                    tracing::debug!("saving device address: {e:#}");
                }
            }
        }
        tracing::info!(peer = %link.name, %addr, %side, "connected");
        let desktop = (!link.screens.is_empty()).then(|| Desktop::new(link.screens.clone()));
        let (id, name, initiator) = (link.id, link.name.clone(), link.initiator);
        let (conn, ctl) = link.spawn(self.inject.clone(), self.link_tx.clone(), self.services.clone());
        if let Some(old) = self.peers.insert(fp, Connected { id, name, initiator, side, conn, ctl, desktop }) {
            old.conn.close(2u32.into(), b"duplicate");
        }
        self.update_sides();
    }

    fn on_link(&mut self, fp: Fingerprint, id: usize, ev: LinkEvent) {
        let Some(peer) = self.peers.get_mut(&fp).filter(|p| p.id == id) else { return };
        match ev {
            LinkEvent::Screens(screens) => {
                let desktop = (!screens.is_empty()).then(|| Desktop::new(screens));
                if let (Some(r), Some(d)) = (self.remote.as_mut(), desktop.as_ref())
                    && r.peer == fp
                {
                    r.cursor.set_desktop(d.clone());
                }
                peer.desktop = desktop;
                self.update_sides();
            }
            LinkEvent::Entered => {
                if let Some(clip) = &self.services.clip {
                    clip.entered(fp, &peer.name);
                }
            }
            LinkEvent::Left => {
                if let Some(clip) = &self.services.clip {
                    clip.left(fp, peer.conn.clone());
                }
            }
            LinkEvent::Closed => {
                let peer = self.peers.remove(&fp).expect("present");
                tracing::info!(peer = %peer.name, "disconnected");
                if let Some(clip) = &self.services.clip {
                    clip.forget(fp);
                }
                if self.remote.as_ref().is_some_and(|r| r.peer == fp) {
                    self.go_home(0.5);
                }
                self.update_sides();
            }
        }
    }

    fn update_sides(&mut self) {
        let mut sides: Vec<Side> =
            self.peers.values().filter(|p| p.desktop.is_some()).map(|p| p.side).collect();
        sides.sort_by_key(|s| *s as u8);
        sides.dedup();
        if sides != self.sides {
            self.sides = sides.clone();
            let _ = self.capture.send(CaptureCmd::SetSides(sides));
        }
    }

    /// Dials paired devices that are not connected, at every address we know.
    fn dial_missing(&mut self, screens: &watch::Receiver<Vec<Rect>>) {
        let me = self.node.identity().fingerprint;
        let mut addrs: Vec<SocketAddr> = Vec::new();
        let mut any_missing = false;
        for dev in self.trust.devices() {
            let Some(fp) = dev.fingerprint() else { continue };
            if fp == me || self.peers.contains_key(&fp) {
                continue;
            }
            any_missing = true;
            addrs.extend(self.discovered.get(&fp).into_iter().flatten());
            addrs.extend(dev.addrs.iter().filter_map(|a| a.parse::<SocketAddr>().ok()));
        }
        if any_missing {
            addrs.extend(&self.extra_addrs);
        }
        for addr in addrs {
            if !self.dialing.insert(addr) {
                continue;
            }
            let (node, dir, screens, tx, services) =
                (self.node.clone(), self.dir.clone(), screens.clone(), self.tx.clone(), self.services.clone());
            tokio::spawn(async move {
                match tokio::time::timeout(DIAL_TIMEOUT, node.connect(addr)).await {
                    Ok(Ok(peer)) => establish(peer, &node, &dir, &screens, &tx, services).await,
                    Ok(Err(e)) => tracing::trace!(%addr, "dial failed: {e:#}"),
                    Err(_) => tracing::trace!(%addr, "dial timed out"),
                }
                let _ = tx.send(AppEvent::DialDone(addr));
            });
        }
    }
}

fn send_datagram(peers: &HashMap<Fingerprint, Connected>, fp: Fingerprint, d: Datagram) {
    if let Some(p) = peers.get(&fp) {
        let (b, n) = d.encode();
        if let Err(e) = p.conn.send_datagram(bytes::Bytes::copy_from_slice(&b[..n])) {
            tracing::trace!("datagram: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_input::NullHandles;
    use one_mk_net::Device;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn free_port() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn trust(dir: &std::path::Path, other: &Identity, side: Side) {
        let mut t = TrustStore::load(dir).unwrap();
        t.upsert(Device {
            name: other.name.clone(),
            fingerprint: other.fingerprint.to_string(),
            side: side.to_string(),
            addrs: vec![],
        });
        t.save().unwrap();
    }

    async fn next_inject(h: &mut NullHandles) -> Inject {
        tokio::time::timeout(Duration::from_secs(5), h.inject.recv()).await.expect("timed out").unwrap()
    }

    async fn next_capture(h: &mut NullHandles) -> CaptureCmd {
        tokio::time::timeout(Duration::from_secs(5), h.capture.recv()).await.expect("timed out").unwrap()
    }

    /// Two daemons on loopback with fake input: cross the edge, move, type,
    /// come back, and check that held keys are released on the way out.
    #[tokio::test]
    async fn pointer_crosses_and_returns() {
        let (da, db) = (temp_dir("a"), temp_dir("b"));
        let (ia, ib) = (Identity::load_or_create(&da).unwrap(), Identity::load_or_create(&db).unwrap());
        trust(&da, &ib, Side::Right); // B is right of A
        trust(&db, &ia, Side::Left);
        let (pa, pb) = (free_port(), free_port());

        let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
        let (ba, mut ha) = one_mk_input::null(desk.clone());
        let (bb, mut hb) = one_mk_input::null(desk);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let stop = |mut rx: watch::Receiver<bool>| async move {
            let _ = rx.wait_for(|s| *s).await;
        };
        let opts = |port, connect: Vec<SocketAddr>| Options {
            backend: one_mk_input::Kind::Null,
            port,
            connect,
            discovery: false,
            clipboard: None,
            downloads: std::env::temp_dir().join("one-mk-test-unused-downloads"),
        };
        let a = tokio::spawn(serve(da.clone(), opts(pa, vec![]), ba, stop(stop_rx.clone())));
        let b = tokio::spawn(serve(db.clone(), opts(pb, vec![SocketAddr::from(([127, 0, 0, 1], pa))]), bb, stop(stop_rx)));

        // A arms its right edge once B is connected.
        assert_eq!(next_capture(&mut ha).await, CaptureCmd::SetSides(vec![Side::Right]));
        assert_eq!(next_capture(&mut hb).await, CaptureCmd::SetSides(vec![Side::Left]));

        // Pointer hits A's right edge halfway down: it appears on B's left edge.
        ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        assert_eq!(next_inject(&mut hb).await, Inject::MoveTo(Pos::new(0.0, 540.0)));

        ha.events.send(Event::Motion { dx: 100.0, dy: -40.0 }).unwrap();
        assert_eq!(next_inject(&mut hb).await, Inject::MoveTo(Pos::new(100.0, 500.0)));

        // Hold Shift, click.
        ha.events.send(Event::Key { code: 42, down: true }).unwrap();
        assert_eq!(next_inject(&mut hb).await, Inject::Key { code: 42, down: true });
        ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
        assert_eq!(next_inject(&mut hb).await, Inject::MoveTo(Pos::new(100.0, 500.0)));
        assert_eq!(next_inject(&mut hb).await, Inject::Button { button: 0x110, down: true });

        // Move back across the edge while Shift and the button are held.
        ha.events.send(Event::Motion { dx: -300.0, dy: 0.0 }).unwrap();
        match next_capture(&mut ha).await {
            CaptureCmd::Release { pos: Some(p), .. } => {
                assert_eq!(p.x, 1917.0);
                assert!((p.y - 500.0).abs() < 1.0, "{p:?}");
            }
            other => panic!("expected release, got {other:?}"),
        }
        // B releases everything that was held (resent motion may come first).
        let mut released = Vec::new();
        while released.len() < 2 {
            match next_inject(&mut hb).await {
                Inject::Key { code, down: false } => released.push(code),
                Inject::Button { button, down: false } => released.push(button),
                Inject::MoveTo(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        released.sort();
        assert_eq!(released, vec![42, 0x110]);

        stop_tx.send(true).unwrap();
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(da);
        let _ = std::fs::remove_dir_all(db);
    }

    /// Two linked daemons on loopback (B to the right of A).
    struct Duo {
        da: PathBuf,
        db: PathBuf,
        ha: NullHandles,
        hb: NullHandles,
        pb: u16,
        stop: watch::Sender<bool>,
        tasks: Vec<tokio::task::JoinHandle<Result<()>>>,
    }

    impl Duo {
        async fn start(tag: &str, clips: Option<(one_mk_clipboard::Clipboard, one_mk_clipboard::Clipboard)>) -> Duo {
            let (da, db) = (temp_dir(&format!("{tag}-a")), temp_dir(&format!("{tag}-b")));
            let (ia, ib) = (Identity::load_or_create(&da).unwrap(), Identity::load_or_create(&db).unwrap());
            trust(&da, &ib, Side::Right);
            trust(&db, &ia, Side::Left);
            let (pa, pb) = (free_port(), free_port());
            let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
            let (ba, mut ha) = one_mk_input::null(desk.clone());
            let (bb, mut hb) = one_mk_input::null(desk);
            let (stop, stop_rx) = watch::channel(false);
            let wait = |mut rx: watch::Receiver<bool>| async move {
                let _ = rx.wait_for(|s| *s).await;
            };
            let (ca, cb) = clips.map_or((None, None), |(a, b)| (Some(a), Some(b)));
            let opts = |port, connect, clipboard, dir: &std::path::Path| Options {
                backend: one_mk_input::Kind::Null,
                port,
                connect,
                discovery: false,
                clipboard,
                downloads: dir.join("downloads"),
            };
            let a = tokio::spawn(serve(da.clone(), opts(pa, vec![], ca, &da), ba, wait(stop_rx.clone())));
            let to_a = vec![SocketAddr::from(([127, 0, 0, 1], pa))];
            let b = tokio::spawn(serve(db.clone(), opts(pb, to_a, cb, &db), bb, wait(stop_rx)));
            assert_eq!(next_capture(&mut ha).await, CaptureCmd::SetSides(vec![Side::Right]));
            assert_eq!(next_capture(&mut hb).await, CaptureCmd::SetSides(vec![Side::Left]));
            Duo { da, db, ha, hb, pb, stop, tasks: vec![a, b] }
        }

        /// Moves the pointer from A onto B, and waits until B has it.
        async fn enter_b(&mut self) {
            self.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
            assert_eq!(next_inject(&mut self.hb).await, Inject::MoveTo(Pos::new(0.0, 540.0)));
        }

        /// Moves the pointer back from B to A.
        async fn leave_b(&mut self) {
            self.ha.events.send(Event::Motion { dx: -100.0, dy: 0.0 }).unwrap();
            assert!(matches!(next_capture(&mut self.ha).await, CaptureCmd::Release { pos: Some(_), .. }));
        }

        async fn stop(self) {
            self.stop.send(true).unwrap();
            for t in self.tasks {
                t.await.unwrap().unwrap();
            }
            let _ = std::fs::remove_dir_all(self.da);
            let _ = std::fs::remove_dir_all(self.db);
        }
    }

    async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..250 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// The clipboard follows the pointer to B, is not echoed back, and comes
    /// back to A only when the user copied something on B.
    #[tokio::test]
    async fn clipboard_follows_the_pointer() {
        let (ca, ma) = one_mk_clipboard::Clipboard::memory();
        let (cb, mb) = one_mk_clipboard::Clipboard::memory();
        let mut duo = Duo::start("clip", Some((ca, cb))).await;

        ma.set_text("copied on A");
        duo.enter_b().await;
        eventually("B to get A's clipboard", || mb.text().as_deref() == Some("copied on A")).await;
        duo.leave_b().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(ma.writes(), 0, "B echoed the clipboard back to A");

        // Unchanged on A: entering B again sends nothing.
        duo.enter_b().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(mb.writes(), 1);

        // The user copies on B while controlling it, then comes back.
        mb.set_text("copied on B");
        duo.leave_b().await;
        eventually("A to get B's clipboard", || ma.text().as_deref() == Some("copied on B")).await;
        assert_eq!(ma.writes(), 1);

        // And it is not sent straight back to B.
        duo.enter_b().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(mb.writes(), 1);
        duo.stop().await;
    }

    /// `one-mk send` from A's identity on its own connection: a folder, a
    /// resumed file, a file whose partial copy is corrupt, and a path that
    /// tries to escape. The service link between A and B must survive.
    #[tokio::test]
    async fn send_folder_to_running_peer() {
        use crate::transfer::{self, Outcome, Outgoing, PARTIAL_DIR, Progress};
        let mut duo = Duo::start("send", None).await;

        let src = temp_dir("send-src");
        let folder = src.join("album");
        std::fs::create_dir_all(folder.join("sub/deeper")).unwrap();
        std::fs::write(folder.join("a.txt"), "hello").unwrap();
        std::fs::write(folder.join("empty"), "").unwrap();
        std::fs::write(folder.join("sub/b.bin"), (0..100_000u32).map(|i| i as u8).collect::<Vec<_>>()).unwrap();
        std::fs::write(folder.join("sub/deeper/c.txt"), "deep").unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(src.join("big.bin"), &big).unwrap();
        std::fs::write(src.join("other.bin"), &big[..1_500_000]).unwrap();
        std::fs::write(src.join("evil"), "pwned").unwrap();

        let mut files = transfer::collect(&[folder.clone(), src.join("big.bin"), src.join("other.bin")]).unwrap();
        let mut rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        rels.sort();
        assert_eq!(rels, ["album/a.txt", "album/empty", "album/sub/b.bin", "album/sub/deeper/c.txt", "big.bin", "other.bin"]);
        let evil = Outgoing { rel: "../evil".into(), ..transfer::collect(&[src.join("evil")]).unwrap().remove(0) };
        files.push(evil);

        // B already has the first third of big.bin from an interrupted
        // transfer, and a corrupt start of other.bin.
        let me = Identity::load_or_create(&duo.da).unwrap();
        let downloads = duo.db.join("downloads");
        let staging = downloads.join(PARTIAL_DIR);
        std::fs::create_dir_all(&staging).unwrap();
        let part = |f: &Outgoing| staging.join(transfer::part_id(me.fingerprint, &f.rel, f.size, f.mtime));
        let by_rel = |r: &str| files.iter().find(|f| f.rel == r).unwrap().clone();
        std::fs::write(part(&by_rel("big.bin")), &big[..1_000_000]).unwrap();
        std::fs::write(part(&by_rel("other.bin")), vec![0xAA; 500_000]).unwrap();

        // Dial B with A's identity, like `one-mk send` does.
        let node = Node::bind(Arc::new(me), "127.0.0.1:0".parse().unwrap()).unwrap();
        let peer = node.connect(SocketAddr::from(([127, 0, 0, 1], duo.pb))).await.unwrap();
        let progress = Arc::new(Progress::default());
        let results = transfer::send_files(&peer.conn, files.clone(), progress.clone()).await;
        let outcome = |r: &str| &results.iter().find(|(f, _)| f.rel == r).unwrap().1;
        assert_eq!(outcome("album/a.txt"), &Outcome::Saved { name: "album/a.txt".into(), resumed_from: 0 });
        assert_eq!(outcome("big.bin"), &Outcome::Saved { name: "big.bin".into(), resumed_from: 1_000_000 });
        assert_eq!(outcome("other.bin"), &Outcome::Failed("integrity check failed".into()));
        assert!(matches!(outcome("../evil"), Outcome::Rejected(r) if r.contains("escapes")));
        let total: u64 = files.iter().map(|f| f.size).sum();
        assert_eq!(progress.done.load(std::sync::atomic::Ordering::Relaxed), total);
        // Resumed bytes of big.bin and of the corrupt other.bin, and the refused file.
        assert_eq!(progress.skipped.load(std::sync::atomic::Ordering::Relaxed), 1_000_000 + 500_000 + 5);

        for (rel, src_path) in [
            ("album/a.txt", folder.join("a.txt")),
            ("album/empty", folder.join("empty")),
            ("album/sub/b.bin", folder.join("sub/b.bin")),
            ("album/sub/deeper/c.txt", folder.join("sub/deeper/c.txt")),
            ("big.bin", src.join("big.bin")),
        ] {
            assert_eq!(std::fs::read(downloads.join(rel)).unwrap(), std::fs::read(src_path).unwrap(), "{rel}");
        }
        assert!(!downloads.join("other.bin").exists());
        assert!(!duo.db.join("evil").exists() && !downloads.join("evil").exists());

        // The corrupt partial file was dropped, so sending again works. A
        // second copy of a.txt gets a new name instead of overwriting.
        let again = vec![by_rel("other.bin"), by_rel("album/a.txt")];
        let results = transfer::send_files(&peer.conn, again, Arc::new(Progress::default())).await;
        assert_eq!(results[0].1, Outcome::Saved { name: "other.bin".into(), resumed_from: 0 });
        assert_eq!(results[1].1, Outcome::Saved { name: "album/a (1).txt".into(), resumed_from: 0 });
        assert_eq!(std::fs::read(downloads.join("other.bin")).unwrap(), &big[..1_500_000]);
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
        peer.conn.close(0u32.into(), b"done");

        // The service link is still there.
        duo.enter_b().await;
        duo.stop().await;
        let _ = std::fs::remove_dir_all(src);
    }
}
