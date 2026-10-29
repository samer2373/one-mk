//! The daemon: owns the local input backend and all peer links, and decides
//! which machine the pointer is on.
//!
//! Two roles, both handled here on one task:
//! - controller: the pointer left our screen and we send input to a peer
//!   ([`Remote`]).
//! - target: a peer controls this machine ([`Owner`]). Only one peer at a
//!   time may; the first one wins until it leaves, goes silent, or someone
//!   uses this machine's own mouse or keyboard.

use crate::clip::ClipSync;
use crate::control::{self, Request};
use crate::link::{self, Link, LinkEvent, Rank};
use crate::manage::{PeerStatus, Status, on_your, takeback_hint};
use crate::transfer::{ConnPermit, Inbox};
use anyhow::{Context, Result};
use one_mk_input::{Backend, CaptureCmd, Event, Inject};
use one_mk_net::{
    ControlWriter, Discovery, Fingerprint, Found, Identity, Node, Peer, Swap, TrustStore, read_stream_kind,
};
use one_mk_proto::{
    Control, Datagram, Desktop, HeldKeys, MotionReceiver, Pos, Received, Rect, RemoteCursor, Side, Step, StreamKind,
    os, sanitize_name, swap_ctrl_meta,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

/// While the pointer is on a peer, resend its position at least this often
/// so Wi-Fi radios never doze off between mouse movements, and the peer
/// knows we are still there.
const KEEPALIVE: Duration = Duration::from_millis(20);
/// After the mouse stops, resend the final position twice, in case the
/// last motion datagram was lost.
const RESEND_AFTER: [Duration; 2] = [Duration::from_millis(4), Duration::from_millis(16)];
/// A controller that sends nothing valid for this long loses control of
/// this machine, and anything it held is released. Its keepalives arrive
/// every 20 ms, so this only fires when something is really wrong.
const LEASE: Duration = Duration::from_millis(750);
/// A controller whose only traffic for this long is keepalives (no key,
/// button, scroll or pointer movement) is let go: a forgotten session must
/// not hold this machine forever, least of all on Linux, where using this
/// machine's own mouse and keyboard can't be detected. The controller's
/// pointer goes home, and its next movement enters again.
const IDLE: Duration = Duration::from_secs(60);
/// If a peer doesn't answer `Enter` within this, the pointer comes home.
const ENTER_TIMEOUT: Duration = Duration::from_millis(750);
const DIAL_INTERVAL: Duration = Duration::from_secs(2);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a peer that connected to us may take to open its first stream.
const FIRST_STREAM_TIMEOUT: Duration = Duration::from_secs(10);
/// Discovery announcements from unknown devices re-read the trusted device
/// list (it may have been paired since) at most this often.
const TRUST_RELOAD_INTERVAL: Duration = Duration::from_secs(5);
/// Incoming connections still being set up (trust check, first stream,
/// `Hello`), in all and from one device. More are closed at once.
const MAX_PENDING: usize = 16;
const MAX_PENDING_PER_PEER: usize = 4;
/// At most this many queued motion events are merged into one send, so a
/// burst can't hold off the safety deadlines for long.
const MAX_COALESCE: usize = 256;

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
    /// A computer that was forgotten: its bulk connections end.
    pub revoked: tokio::sync::broadcast::Sender<Fingerprint>,
}

/// Serves the streams a peer opens on a bulk connection (clipboard and
/// files; never the input link) until it closes, each on its own task.
/// `first` is the stream that identified the connection, already read.
pub async fn serve_streams(
    conn: quinn::Connection,
    fp: Fingerprint,
    name: String,
    services: Services,
    first: (StreamKind, quinn::SendStream, quinn::RecvStream),
) {
    // Limits how many bulk connections one device may hold open at once.
    let permit = match services.inbox.admit(fp) {
        Ok(p) => Arc::new(p),
        Err(reason) => {
            tracing::warn!(peer = %name, "{reason}");
            conn.close(3u32.into(), reason.as_bytes());
            return;
        }
    };
    {
        let (services, name, permit) = (services.clone(), name.clone(), permit.clone());
        let (kind, send, recv) = first;
        tokio::spawn(async move { serve_stream(kind, &permit, &name, &services, send, recv).await });
    }
    let mut revoked = services.revoked.subscribe();
    loop {
        let (send, mut recv) = tokio::select! {
            r = conn.accept_bi() => match r {
                Ok(s) => s,
                Err(_) => break,
            },
            Ok(gone) = revoked.recv() => {
                if gone == fp {
                    conn.close(3u32.into(), b"forgotten");
                    break;
                }
                continue;
            }
        };
        let (services, name, permit) = (services.clone(), name.clone(), permit.clone());
        tokio::spawn(async move {
            let kind = match tokio::time::timeout(FIRST_STREAM_TIMEOUT, read_stream_kind(&mut recv)).await {
                Ok(Ok(k)) => k,
                Ok(Err(e)) => return tracing::debug!(peer = %name, "bad stream: {e:#}"),
                Err(_) => return,
            };
            serve_stream(kind, &permit, &name, &services, send, recv).await;
        });
    }
}

async fn serve_stream(
    kind: StreamKind,
    permit: &ConnPermit,
    name: &str,
    services: &Services,
    send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) {
    match (kind, &services.clip) {
        (StreamKind::File, _) => services.inbox.receive_on(permit, name, send, recv).await,
        (StreamKind::Clipboard, Some(clip)) => clip.receive(permit.fp(), name, recv).await,
        _ => {
            // Includes clipboard streams when clipboard sharing is off here.
            tracing::debug!(peer = %name, "refused {kind:?} stream");
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
    /// The peer's OS (see [`one_mk_proto::os`]).
    os: u8,
    rank: Rank,
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
    /// What we hold, by the codes we read locally.
    held: HeldKeys,
    /// Swap Ctrl and Cmd in keys sent. Fixed for the whole visit, so a key
    /// pressed as one code is always released as the same code.
    swap: bool,
    /// The peer accepted the visit. Until then motion only updates
    /// `cursor` (the latest position is sent with the ack), and other input
    /// waits in `queued`.
    acked: bool,
    queued: Vec<Control>,
    entered_at: Instant,
    last_send: Instant,
    resend_at: Option<Instant>,
    resends: usize,
}

impl Remote {
    /// Sends a new position and schedules its idle resends.
    fn send_motion(&mut self, peers: &HashMap<Fingerprint, Connected>, d: Datagram) {
        let now = Instant::now();
        send_datagram(peers, self.peer, d);
        self.last_send = now;
        self.resends = 0;
        self.resend_at = Some(now + RESEND_AFTER[0]);
    }
}

/// A peer controls this machine.
struct Owner {
    peer: Fingerprint,
    link: usize,
    epoch: u16,
    motion: MotionReceiver,
    /// The newest position applied (by motion or a click), where a late
    /// click must leave the pointer.
    pos: Pos,
    held: HeldKeys,
    /// Last valid traffic from the owner for this visit.
    last_rx: Instant,
    /// Last real input: a key, button, scroll or a new position (not a
    /// keepalive repeating the same one).
    last_input: Instant,
}

impl Owner {
    fn new(peer: Fingerprint, link: usize, epoch: u16, pos: Pos) -> Self {
        let mut motion = MotionReceiver::default();
        motion.enter(epoch);
        let now = Instant::now();
        Self { peer, link, epoch, motion, pos, held: HeldKeys::new(), last_rx: now, last_input: now }
    }

    /// A motion datagram from the owner.
    fn motion(&mut self, epoch: u16, seq: u32, pos: Pos, inject: &mpsc::UnboundedSender<Inject>) {
        match self.motion.accept(epoch, seq, pos) {
            Received::Move(p) => {
                let now = Instant::now();
                self.last_rx = now;
                self.last_input = now;
                self.pos = p;
                let _ = inject.send(Inject::MoveTo(p));
            }
            Received::Repeat => self.last_rx = Instant::now(),
            Received::Stale => {}
        }
    }

    fn input(&mut self, msg: Control, inject: &mpsc::UnboundedSender<Inject>) {
        let now = Instant::now();
        self.last_rx = now;
        self.last_input = now;
        let inject = |i| {
            let _ = inject.send(i);
        };
        match msg {
            Control::Key { code, down } => {
                if self.held.update(code, down) {
                    inject(Inject::Key { code, down });
                }
            }
            Control::Button { button, down, seq, pos } => {
                if self.motion.sync(seq) {
                    self.pos = pos;
                    inject(Inject::MoveTo(pos));
                    if self.held.update(button, down) {
                        inject(Inject::Button { button, down });
                    }
                } else if self.held.update(button, down) {
                    // A newer position overtook this click (datagrams
                    // and the control stream race). Click where the user
                    // clicked, then put the pointer back: the newer
                    // position's resends count as repeats and would never
                    // move it there again.
                    inject(Inject::MoveTo(pos));
                    inject(Inject::Button { button, down });
                    inject(Inject::MoveTo(self.pos));
                }
            }
            Control::Scroll { dx, dy } => inject(Inject::Scroll { dx: f64::from(dx), dy: f64::from(dy) }),
            Control::ScrollDiscrete { dx, dy } => inject(Inject::ScrollDiscrete { dx, dy }),
            Control::ScrollStop => inject(Inject::ScrollStop),
            _ => {}
        }
    }

    fn release_all(&mut self, inject: &mpsc::UnboundedSender<Inject>) {
        for code in self.held.take_all() {
            let release = if link::is_button(code) {
                Inject::Button { button: code, down: false }
            } else {
                Inject::Key { code, down: false }
            };
            let _ = inject.send(release);
        }
    }
}

struct App {
    dir: PathBuf,
    node: Node,
    trust: TrustStore,
    trust_reloaded: Option<Instant>,
    peers: HashMap<Fingerprint, Connected>,
    remote: Option<Remote>,
    owner: Option<Owner>,
    local: Option<Desktop>,
    local_screens: watch::Sender<Vec<Rect>>,
    epoch: u16,
    capture: mpsc::UnboundedSender<CaptureCmd>,
    inject: mpsc::UnboundedSender<Inject>,
    tx: mpsc::UnboundedSender<AppEvent>,
    link_tx: link::LinkTx,
    sides: Vec<Side>,
    /// Layout conflicts last warned about.
    conflicts: Vec<String>,
    discovered: HashMap<Fingerprint, Vec<SocketAddr>>,
    extra_addrs: Vec<SocketAddr>,
    dialing: HashSet<SocketAddr>,
    services: Services,
    /// Peers whose first time controlling this machine (this run) has been
    /// announced with how to take control back.
    announced: HashSet<Fingerprint>,
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
    let node = bind(identity.clone(), opts.port)?;
    let port = node.local_addr()?.port();
    let mut ipc = control::Server::start(&dir).await?;
    println!("one-mk is running on {}.", sanitize_name(&identity.name));
    println!("Tip: Ctrl+Alt+Shift+Esc always brings the pointer back.");
    if trust.devices().is_empty() {
        println!("No computers are paired yet. Run `one-mk pair` on this computer and on the other one.");
    }
    tracing::debug!(id = %identity.fingerprint.short(), port, "listening");

    let Backend { mut events, capture, inject } = backend;
    let services = Services {
        clip: opts.clipboard.map(|cb| ClipSync::new(cb, node.clone())),
        inbox: Inbox::new(opts.downloads.clone()),
        revoked: tokio::sync::broadcast::channel(8).0,
    };
    tracing::debug!(
        clipboard = services.clip.is_some(),
        downloads = %opts.downloads.display(),
        "sharing"
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (link_tx, mut link_rx) = mpsc::unbounded_channel();
    let (local_screens, screens_rx) = watch::channel(Vec::new());

    spawn_accept(node.clone(), dir.clone(), screens_rx.clone(), tx.clone(), services.clone(), Pending::default());
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
        trust_reloaded: None,
        peers: HashMap::new(),
        remote: None,
        owner: None,
        local: None,
        local_screens,
        epoch: 0,
        capture,
        inject,
        tx,
        link_tx,
        sides: Vec::new(),
        conflicts: Vec::new(),
        discovered: HashMap::new(),
        extra_addrs: opts.connect,
        dialing: HashSet::new(),
        services,
        announced: HashSet::new(),
    };
    app.dial_missing(&screens_rx);

    let mut dial_tick = tokio::time::interval(DIAL_INTERVAL);
    dial_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut input_open = true;
    tokio::pin!(shutdown);

    loop {
        // Safety deadlines come first. The select below prefers events, so
        // a steady stream of them (a peer flooding control messages, or
        // just busy traffic) must not hold off a lease expiry, the release
        // of held keys or an `Enter` timeout. Each pass handles one event
        // (a run of motion at most `MAX_COALESCE`), then checks again.
        if app.next_deadline().is_some_and(|d| d <= Instant::now()) {
            app.on_timer();
        }
        let deadline = app.next_deadline();
        tokio::select! {
            biased;
            ev = events.recv(), if input_open => match ev {
                Some(ev) => app.on_input(ev, &mut events),
                None => {
                    tracing::error!("input backend stopped");
                    input_open = false;
                }
            },
            Some((fp, id, ev)) = link_rx.recv() => app.on_link(fp, id, ev),
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => app.on_timer(),
            Some(ev) = rx.recv() => app.on_app(ev, &screens_rx),
            Some((req, reply)) = ipc.recv() => if app.on_request(req, reply) { break },
            _ = dial_tick.tick() => app.dial_missing(&screens_rx),
            _ = &mut shutdown => break,
        }
    }

    if let Some(r) = app.remote.as_ref() {
        let t = r.cursor.home_fraction();
        app.go_home(t);
    }
    app.end_owner(false);
    node.close();
    tokio::time::sleep(Duration::from_millis(50)).await; // let the Leave and close go out
    Ok(())
}

fn bind(identity: Arc<Identity>, port: u16) -> Result<Node> {
    Node::bind_any(identity, port)
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

/// Counts incoming connections that are still being set up, so a peer
/// that finishes TLS over and over but never opens its first stream can't
/// pile up establishment tasks. Established connections are limited
/// elsewhere: one input link per device (`App::peers`) and
/// [`crate::transfer::MAX_CONNS_PER_PEER`] bulk connections.
#[derive(Clone, Default)]
struct Pending(Arc<std::sync::Mutex<PendingCounts>>);

#[derive(Default)]
struct PendingCounts {
    total: usize,
    per_peer: HashMap<Fingerprint, usize>,
}

/// One connection being set up; released on drop.
struct PendingPermit {
    pending: Pending,
    fp: Fingerprint,
}

impl Pending {
    fn admit(&self, fp: Fingerprint) -> Option<PendingPermit> {
        let mut c = self.0.lock().expect("not poisoned");
        let mine = c.per_peer.get(&fp).copied().unwrap_or(0);
        if c.total >= MAX_PENDING || mine >= MAX_PENDING_PER_PEER {
            return None;
        }
        c.total += 1;
        c.per_peer.insert(fp, mine + 1);
        Some(PendingPermit { pending: self.clone(), fp })
    }
}

impl Drop for PendingPermit {
    fn drop(&mut self) {
        let mut c = self.pending.0.lock().expect("not poisoned");
        c.total -= 1;
        if let Some(n) = c.per_peer.get_mut(&self.fp) {
            *n -= 1;
            if *n == 0 {
                c.per_peer.remove(&self.fp);
            }
        }
    }
}

fn spawn_accept(
    node: Node,
    dir: PathBuf,
    screens: watch::Receiver<Vec<Rect>>,
    tx: mpsc::UnboundedSender<AppEvent>,
    services: Services,
    pending: Pending,
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
            let Some(permit) = pending.admit(peer.fingerprint) else {
                tracing::debug!(peer = %peer.fingerprint.short(), "too many connections being set up; refusing");
                peer.conn.close(4u32.into(), b"busy");
                continue;
            };
            let (node, dir, screens, tx, services) =
                (node.clone(), dir.clone(), screens.clone(), tx.clone(), services.clone());
            tokio::spawn(async move { establish(peer, &node, &dir, &screens, &tx, services, Some(permit)).await });
        }
    });
}

/// Checks the peer is paired, then handshakes and hands the link to the app.
///
/// A connection we dialed always starts with a control stream. One that
/// was dialed to us may instead start with a file or clipboard stream: a
/// bulk connection, used for clipboard snapshots (so they never share
/// congestion control with input) or by `one-mk send` (which shares the
/// peer service's identity). It never replaces the link.
///
/// `permit` (for incoming connections) is held until the connection is
/// identified: handed to the app as a link, or to the bulk stream server.
async fn establish(
    peer: Peer,
    node: &Node,
    dir: &std::path::Path,
    screens: &watch::Receiver<Vec<Rect>>,
    tx: &mpsc::UnboundedSender<AppEvent>,
    services: Services,
    permit: Option<PendingPermit>,
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
            Ok(Ok((kind @ (StreamKind::File | StreamKind::Clipboard), send, recv))) => {
                tracing::debug!(peer = %device_name, %addr, "bulk connection");
                drop(permit); // the transfer limits take over
                serve_streams(peer.conn, peer.fingerprint, device_name, services, (kind, send, recv)).await;
                return;
            }
            Ok(Ok((StreamKind::Probe, _send, _recv))) => {
                // `one-mk bench --peer`: echo its probes on this connection
                // only; the link (and its rank) is not involved.
                tracing::debug!(peer = %device_name, %addr, "probe connection");
                drop(permit);
                while let Ok(b) = peer.conn.read_datagram().await {
                    if let Ok(Datagram::Probe { id, sent_ns }) = Datagram::decode(&b) {
                        let (e, n) = Datagram::ProbeEcho { id, sent_ns }.encode();
                        let _ = peer.conn.send_datagram(bytes::Bytes::copy_from_slice(&e[..n]));
                    }
                }
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
            link::handshake(peer, streams, node.identity(), local, &device_name).await
        }
        Err(e) => Err(e),
    };
    drop(permit);
    match res {
        Ok(link) => {
            let _ = tx.send(AppEvent::Connected(link));
        }
        Err(e) => tracing::warn!(%addr, "handshake failed: {e:#}"),
    }
}

/// The outcome of applying a run of queued motion events.
#[derive(Debug, Default, PartialEq)]
struct Coalesced {
    /// The last position reached; the only one worth sending.
    moved: Option<Datagram>,
    /// The pointer left through the home edge at this fraction.
    leave: Option<f64>,
    /// The first non-motion event, to handle next.
    next: Option<Event>,
}

/// Applies a motion and every motion already queued behind it, in order
/// (the cursor clamps at edges, so deltas can't simply be summed). Stops at
/// the first other event or when the pointer leaves.
fn coalesce(
    cursor: &mut RemoteCursor,
    (dx, dy): (f64, f64),
    events: &mut mpsc::UnboundedReceiver<Event>,
) -> Coalesced {
    let mut out = Coalesced::default();
    let mut step = cursor.motion(dx, dy);
    for _ in 0..MAX_COALESCE {
        match step {
            Step::Move(d) => out.moved = Some(d),
            Step::Leave { t } => {
                out.leave = Some(t);
                return out;
            }
            Step::Still => {}
        }
        match events.try_recv() {
            Ok(Event::Motion { dx, dy }) => step = cursor.motion(dx, dy),
            Ok(ev) => {
                out.next = Some(ev);
                return out;
            }
            Err(_) => return out,
        }
    }
    // The rest of the run is handled next time round.
    match step {
        Step::Move(d) => out.moved = Some(d),
        Step::Leave { t } => out.leave = Some(t),
        Step::Still => {}
    }
    out
}

impl App {
    // ------------------------------------------------------------ local input

    fn on_input(&mut self, ev: Event, events: &mut mpsc::UnboundedReceiver<Event>) {
        match ev {
            Event::Desktop(d) => {
                tracing::debug!(screens = ?d.screens, "local screens");
                for p in self.peers.values() {
                    p.ctl.send(Control::Screens(d.screens.clone()));
                }
                let _ = self.local_screens.send(d.screens.clone());
                self.local = Some(d);
                self.update_sides();
            }
            Event::Activated { id, side, pos } => self.enter_peer(id, side, pos),
            Event::Motion { dx, dy } => {
                if let Some(next) = self.on_motion(dx, dy, events) {
                    self.on_input(next, events);
                }
            }
            Event::Button { button, down } => {
                let Some(r) = self.remote.as_mut() else { return };
                if r.held.update(button, down) {
                    let msg = Control::Button { button, down, seq: r.cursor.seq(), pos: r.cursor.pos() };
                    self.send_input(msg);
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
                let wire = if r.swap { swap_ctrl_meta(code) } else { code };
                if r.held.update(code, down) {
                    self.send_input(Control::Key { code: wire, down });
                }
            }
            Event::Scroll { dx, dy } => self.send_input(Control::Scroll { dx: dx as f32, dy: dy as f32 }),
            Event::ScrollDiscrete { dx, dy } => self.send_input(Control::ScrollDiscrete { dx, dy }),
            Event::ScrollStop => self.send_input(Control::ScrollStop),
            Event::LocalInput => {
                if self.owner.is_some() {
                    tracing::info!("this computer's own mouse or keyboard was used; it takes back control");
                    self.end_owner(true);
                }
            }
            Event::Deactivated { id } => {
                if self.remote.as_ref().is_some_and(|r| r.activation == id) {
                    tracing::info!("capture ended by the system; pointer is home");
                    self.end_visit();
                }
            }
            Event::Error(msg) => tracing::error!("{msg}"),
        }
    }

    /// Moves the remote pointer, coalescing motion that queued up meanwhile.
    /// Returns the event that ended the run, to be handled next.
    fn on_motion(&mut self, dx: f64, dy: f64, events: &mut mpsc::UnboundedReceiver<Event>) -> Option<Event> {
        let r = self.remote.as_mut()?;
        let c = coalesce(&mut r.cursor, (dx, dy), events);
        if let Some(d) = c.moved
            && r.acked
        {
            r.send_motion(&self.peers, d);
        }
        if let Some(t) = c.leave {
            self.go_home(t);
        }
        c.next
    }

    /// The pointer hit the `side` edge of our desktop.
    fn enter_peer(&mut self, activation: u32, side: Side, pos: Pos) {
        self.end_visit(); // a new activation supersedes any visit in progress
        let target = self.peer_at(side).and_then(|fp| Some((fp, self.peers.get(&fp)?.desktop.clone()?)));
        let (Some(local), Some((fp, desktop))) = (self.local.as_ref(), target) else {
            let _ = self.capture.send(CaptureCmd::Release { id: activation, pos: None });
            return;
        };
        self.epoch = self.epoch.wrapping_add(1);
        let t = local.edge_fraction(side, pos);
        let cursor = RemoteCursor::enter(desktop, side.opposite(), t, self.epoch);
        let peer = &self.peers[&fp];
        let setting = self.trust.get(&fp).map_or(Swap::Auto, |d| d.swap_ctrl_cmd);
        let swap = swap_applies(setting, os::local(), peer.os);
        peer.ctl.send(Control::Enter { epoch: self.epoch, pos: cursor.pos() });
        tracing::debug!(peer = %peer.name, %side, "pointer moved to peer");
        let now = Instant::now();
        self.remote = Some(Remote {
            activation,
            peer: fp,
            side,
            cursor,
            held: HeldKeys::new(),
            swap,
            acked: false,
            queued: Vec::new(),
            entered_at: now,
            last_send: now,
            resend_at: None,
            resends: 0,
        });
    }

    /// Ends the visit to a peer (which releases anything still held there),
    /// without touching capture.
    fn end_visit(&mut self) -> Option<Remote> {
        let r = self.remote.take()?;
        if let Some(p) = self.peers.get(&r.peer) {
            p.ctl.send(Control::Leave);
        }
        Some(r)
    }

    /// Returns the pointer to this machine at fraction `t` along the edge.
    fn go_home(&mut self, t: f64) {
        let Some(r) = self.end_visit() else { return };
        let pos = self.local.as_ref().map(|d| d.point_on_edge(r.side, t, 2.0));
        let _ = self.capture.send(CaptureCmd::Release { id: r.activation, pos });
        tracing::debug!("pointer is home");
    }

    /// Like `go_home`, if the visit to `fp` is `epoch`.
    fn go_home_from(&mut self, fp: Fingerprint, epoch: u16, why: &str) {
        let Some(r) = self.remote.as_ref().filter(|r| r.peer == fp && r.cursor.epoch() == epoch) else { return };
        let name = self.peers.get(&fp).map_or("", |p| &p.name);
        tracing::info!(peer = %name, "{why}; pointer is home");
        let t = r.cursor.home_fraction();
        self.go_home(t);
    }

    /// Forwards input to the peer we control, or holds it until the peer
    /// accepts the visit.
    fn send_input(&mut self, msg: Control) {
        let Some(r) = self.remote.as_mut() else { return };
        if !r.acked {
            r.queued.push(msg);
        } else if let Some(p) = self.peers.get(&r.peer) {
            p.ctl.send(msg);
        }
    }

    // ----------------------------------------------------------------- timers

    fn next_deadline(&self) -> Option<Instant> {
        let lease = self.owner.as_ref().map(|o| (o.last_rx + LEASE).min(o.last_input + IDLE));
        let visit = self.remote.as_ref().map(|r| {
            if r.acked {
                let keepalive = r.last_send + KEEPALIVE;
                r.resend_at.map_or(keepalive, |t| t.min(keepalive))
            } else {
                r.entered_at + ENTER_TIMEOUT
            }
        });
        lease.into_iter().chain(visit).min()
    }

    fn on_timer(&mut self) {
        let now = Instant::now();
        if self.owner.as_ref().is_some_and(|o| now >= o.last_rx + LEASE) {
            tracing::warn!("the controlling computer went silent; releasing held keys");
            self.end_owner(true);
        } else if self.owner.as_ref().is_some_and(|o| now >= o.last_input + IDLE) {
            tracing::info!("no input from the controlling computer for {} s; it lets go", IDLE.as_secs());
            self.end_owner(true);
        }
        let Some(r) = self.remote.as_mut() else { return };
        if !r.acked {
            if now >= r.entered_at + ENTER_TIMEOUT {
                let name = self.peers.get(&r.peer).map_or("", |p| &p.name);
                tracing::warn!(peer = %name, "peer did not accept the pointer; pointer is home");
                let t = r.cursor.home_fraction();
                self.go_home(t);
            }
            return;
        }
        let resend = r.resend_at.is_some_and(|at| now >= at);
        if resend {
            r.resends += 1;
            r.resend_at = RESEND_AFTER.get(r.resends).map(|d| now + (*d - RESEND_AFTER[r.resends - 1]));
        }
        // An idle resend is also the keepalive: the same position and seq.
        if resend || now >= r.last_send + KEEPALIVE {
            send_datagram(&self.peers, r.peer, r.cursor.current());
            r.last_send = now;
        }
    }

    // ------------------------------------------------- being controlled

    fn is_owner(&self, fp: Fingerprint, id: usize) -> bool {
        self.owner.as_ref().is_some_and(|o| o.peer == fp && o.link == id)
    }

    /// Peer `fp` (link `id`) moves its pointer onto this machine.
    fn on_enter(&mut self, fp: Fingerprint, id: usize, epoch: u16, pos: Pos) {
        let peer = &self.peers[&fp];
        if self.owner.is_some() && !self.is_owner(fp, id) {
            tracing::info!(peer = %peer.name, "refused control: another computer controls this one");
            peer.ctl.send(Control::Busy { epoch });
            return;
        }
        if let Some(mut old) = self.owner.take() {
            old.release_all(&self.inject); // same controller, new visit
        }
        let _ = self.inject.send(Inject::Controlled(true));
        let _ = self.inject.send(Inject::MoveTo(pos));
        peer.ctl.send(Control::EnterAck { epoch });
        if let Some(clip) = &self.services.clip {
            clip.entered(fp, &peer.name);
        }
        if self.announced.insert(fp) {
            println!("{} is controlling this computer. {}", peer.name, takeback_hint());
        }
        self.owner = Some(Owner::new(fp, id, epoch, pos));
    }

    /// Ends the current owner's control, releasing everything it held.
    /// With `revoke`, tells it to take its pointer back.
    fn end_owner(&mut self, revoke: bool) {
        let Some(mut o) = self.owner.take() else { return };
        o.release_all(&self.inject);
        let _ = self.inject.send(Inject::Controlled(false));
        if let Some(p) = self.peers.get(&o.peer).filter(|p| p.id == o.link) {
            if revoke {
                p.ctl.send(Control::Revoked { epoch: o.epoch });
            }
            if let Some(clip) = &self.services.clip {
                clip.left(o.peer, p.conn.remote_address());
            }
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
                    if self.trust_reloaded.is_none_or(|t| t.elapsed() >= TRUST_RELOAD_INTERVAL) {
                        self.trust_reloaded = Some(Instant::now());
                        if let Ok(t) = TrustStore::load(&self.dir) {
                            self.trust = t;
                        }
                    }
                    if !self.trust.is_trusted(&f.fingerprint) {
                        return;
                    }
                }
                tracing::debug!(name = %sanitize_name(&f.name), addrs = ?f.addrs, "discovered");
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
        // Both sides may dial, at several addresses. Both keep the same
        // connection: the one with the lowest rank.
        if let Some(existing) = self.peers.get(&fp) {
            if existing.rank <= link.rank {
                link.conn.close(2u32.into(), b"duplicate");
                return;
            }
            // The link we are replacing ends any visit over it, both ways.
            if self.remote.as_ref().is_some_and(|r| r.peer == fp) {
                let t = self.remote.as_ref().map_or(0.5, |r| r.cursor.home_fraction());
                self.go_home(t);
            }
            if self.owner.as_ref().is_some_and(|o| o.peer == fp) {
                self.end_owner(false);
            }
        }
        self.reload_trust();
        let side = self.trust.get(&fp).map_or(Side::Right, |d| d.side());
        let addr = canonical(link.conn.remote_address());
        if let Some(dev) = self.trust.get_mut(&fp) {
            let a = addr.to_string();
            let mut changed = std::mem::replace(&mut dev.os, link.os) != link.os;
            if dev.addrs.first() != Some(&a) {
                dev.addrs.retain(|x| *x != a);
                dev.addrs.insert(0, a);
                dev.addrs.truncate(4);
                changed = true;
            }
            if changed && let Err(e) = self.trust.save() {
                tracing::debug!("saving device details: {e:#}");
            }
        }
        // A better duplicate connection replaces the first one quietly.
        if !self.peers.contains_key(&fp) {
            println!("Connected to {} ({}).", link.name, on_your(side));
        }
        tracing::debug!(peer = %link.name, %addr, os = link.os, "connected");
        let desktop = (!link.screens.is_empty()).then(|| Desktop::new(link.screens.clone()));
        let (id, name, os, rank, sent) = (link.id, link.name.clone(), link.os, link.rank, link.sent_screens.clone());
        let (conn, ctl) = link.spawn(self.link_tx.clone());
        // Our monitors may have changed during the handshake.
        if let Some(local) = &self.local
            && local.screens != sent
        {
            ctl.send(Control::Screens(local.screens.clone()));
        }
        if let Some(old) = self.peers.insert(fp, Connected { id, name, os, rank, side, conn, ctl, desktop }) {
            old.conn.close(2u32.into(), b"duplicate");
        }
        self.send_layout(fp);
        self.update_sides();
    }

    fn on_link(&mut self, fp: Fingerprint, id: usize, ev: LinkEvent) {
        if !self.peers.get(&fp).is_some_and(|p| p.id == id) {
            return; // a replaced link
        }
        match ev {
            LinkEvent::Motion { epoch, seq, pos } => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id) {
                    o.motion(epoch, seq, pos, &self.inject);
                }
            }
            LinkEvent::Control(msg) => self.on_control(fp, id, msg),
            LinkEvent::Closed => {
                let peer = self.peers.remove(&fp).expect("present");
                if self.owner.as_ref().is_some_and(|o| o.peer == fp) {
                    self.end_owner(false);
                }
                println!("{} disconnected; reconnecting automatically.", peer.name);
                if let Some(clip) = &self.services.clip {
                    clip.forget(fp);
                }
                if let Some(r) = self.remote.take_if(|r| r.peer == fp) {
                    let pos = self.local.as_ref().map(|d| d.point_on_edge(r.side, r.cursor.home_fraction(), 2.0));
                    let _ = self.capture.send(CaptureCmd::Release { id: r.activation, pos });
                }
                self.update_sides();
            }
        }
    }

    fn on_control(&mut self, fp: Fingerprint, id: usize, msg: Control) {
        match msg {
            Control::Hello { .. } => {}
            Control::Screens(screens) => self.on_peer_screens(fp, screens),
            Control::Layout { side_of_you, version } => self.on_peer_layout(fp, side_of_you, version),
            Control::Enter { epoch, pos } => self.on_enter(fp, id, epoch, pos),
            Control::Leave => {
                if self.is_owner(fp, id) {
                    self.end_owner(false);
                }
            }
            Control::EnterAck { epoch } => {
                let now = Instant::now();
                // A late ack is ignored: the timeout has sent (or is about
                // to send) the pointer home.
                let Some(r) = self.remote.as_mut().filter(|r| {
                    r.peer == fp && r.cursor.epoch() == epoch && !r.acked && now < r.entered_at + ENTER_TIMEOUT
                }) else {
                    return;
                };
                r.acked = true;
                let peer = &self.peers[&fp];
                if r.queued.is_empty() {
                    r.send_motion(&self.peers, r.cursor.current());
                } else {
                    // Input queued while waiting goes first, in order. It
                    // travels on the control stream, which another task
                    // writes, so the newest position follows a moment
                    // later (the first resend) instead of overtaking a
                    // queued click on the faster datagram path. (The
                    // receiver copes with a late click anyway.)
                    for m in r.queued.drain(..) {
                        peer.ctl.send(m);
                    }
                    r.last_send = now;
                    r.resends = 0;
                    r.resend_at = Some(now + RESEND_AFTER[0]);
                }
                if let Some(clip) = &self.services.clip {
                    clip.push(fp, &peer.name, peer.conn.remote_address());
                }
            }
            Control::Busy { epoch } => self.go_home_from(fp, epoch, "it is controlled by another computer"),
            Control::Revoked { epoch } => self.go_home_from(fp, epoch, "it took back control"),
            input => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id) {
                    o.input(input, &self.inject);
                }
            }
        }
    }

    /// Re-reads the paired computers. One that was forgotten is let go at
    /// once: its input link, and any clipboard or file connection it holds.
    fn reload_trust(&mut self) {
        let Ok(new) = TrustStore::load(&self.dir) else { return };
        let gone: Vec<_> = self.trust.devices().iter().filter_map(|d| d.fingerprint()).filter(|fp| !new.is_trusted(fp)).collect();
        self.trust = new;
        for fp in gone {
            if let Some(p) = self.peers.get(&fp) {
                p.conn.close(3u32.into(), b"forgotten");
            }
            let _ = self.services.revoked.send(fp);
        }
    }

    /// Tells a peer where we think it is, from its point of view. It adopts
    /// that only if our version is newer than its own.
    fn send_layout(&self, fp: Fingerprint) {
        if let (Some(p), Some(d)) = (self.peers.get(&fp), self.trust.get(&fp)) {
            p.ctl.send(Control::Layout { side_of_you: d.side().opposite(), version: d.layout_version });
        }
    }

    /// The peer says it sits on `side_of_you` of us.
    fn on_peer_layout(&mut self, fp: Fingerprint, side_of_you: Side, version: u64) {
        self.reload_trust();
        let Some(dev) = self.trust.get_mut(&fp).filter(|d| version > d.layout_version) else { return };
        dev.side = side_of_you.to_string();
        dev.layout_version = version;
        if let Err(e) = self.trust.save() {
            tracing::warn!("could not save the new layout: {e:#}");
        }
        self.sync_sides();
    }

    /// Applies the sides in the trust store to connected peers.
    fn sync_sides(&mut self) {
        let mut moved = Vec::new();
        for (fp, p) in &mut self.peers {
            let Some(side) = self.trust.get(fp).map(|d| d.side()).filter(|s| *s != p.side) else { continue };
            p.side = side;
            moved.push(*fp);
            println!("{} is now {}.", p.name, on_your(side));
        }
        if let Some(r) = self.remote.as_ref().filter(|r| moved.contains(&r.peer)) {
            let t = r.cursor.home_fraction();
            self.go_home(t);
        }
        self.update_sides();
    }

    /// Answers a control request; true when it asked the daemon to stop.
    fn on_request(&mut self, req: Request, reply: tokio::sync::oneshot::Sender<String>) -> bool {
        self.reload_trust();
        let text = match req {
            Request::Status => self.status().encode(),
            Request::Reload => {
                self.sync_sides();
                let fps: Vec<_> = self.peers.keys().copied().collect();
                fps.into_iter().for_each(|fp| self.send_layout(fp));
                "ok\n".to_owned()
            }
            Request::TakeBack => match self.owner.as_ref().map(|o| o.peer) {
                Some(fp) => {
                    let name = self.peers.get(&fp).map_or_else(String::new, |p| p.name.clone());
                    tracing::info!(peer = %name, "control taken back with `one-mk takeback`");
                    self.end_owner(true);
                    format!("ok\n{name}\n")
                }
                None => "none\n".to_owned(),
            },
            Request::Stop => "ok\n".to_owned(),
        };
        let _ = reply.send(text);
        req == Request::Stop
    }

    fn status(&self) -> Status {
        let id = self.node.identity();
        let name_of = |fp: &Fingerprint| self.peers.get(fp).map_or_else(String::new, |p| p.name.clone());
        let peers = self.trust.devices().iter().filter_map(|d| {
            let live = self.peers.get(&d.fingerprint()?);
            Some(PeerStatus {
                id: d.short_id(),
                name: sanitize_name(&d.name),
                side: live.map_or_else(|| d.side(), |p| p.side),
                os: live.map_or(d.os, |p| p.os),
                online: live.is_some(),
                swap: d.swap_ctrl_cmd.to_string(),
                screens: live.and_then(|p| p.desktop.as_ref()).map_or_else(Vec::new, |d| d.screens.clone()),
            })
        });
        Status {
            name: sanitize_name(&id.name),
            id: id.fingerprint.short(),
            port: self.node.local_addr().map_or(0, |a| a.port()),
            controlling: self.remote.as_ref().map(|r| name_of(&r.peer)),
            controlled_by: self.owner.as_ref().map(|o| name_of(&o.peer)),
            peers: peers.collect(),
            screens: self.local.as_ref().map_or_else(Vec::new, |d| d.screens.clone()),
        }
    }

    fn on_peer_screens(&mut self, fp: Fingerprint, screens: Vec<Rect>) {
        let desktop = (!screens.is_empty()).then(|| Desktop::new(screens));
        if let Some(r) = self.remote.as_mut().filter(|r| r.peer == fp) {
            match &desktop {
                Some(d) => {
                    let moved = r.cursor.set_desktop(d.clone());
                    if r.acked {
                        r.send_motion(&self.peers, moved);
                    }
                }
                None => {
                    let t = r.cursor.home_fraction();
                    self.go_home(t);
                }
            }
        }
        if let Some(p) = self.peers.get_mut(&fp) {
            p.desktop = desktop;
        }
        self.update_sides();
    }

    /// The peer the pointer goes to through `side`. If several sit there (a
    /// layout mistake), the one with the lowest fingerprint, so the choice
    /// doesn't depend on connection order.
    fn peer_at(&self, side: Side) -> Option<Fingerprint> {
        self.peers.iter().filter(|(_, p)| p.side == side && p.desktop.is_some()).map(|(fp, _)| *fp).min()
    }

    fn update_sides(&mut self) {
        let mut sides = Vec::new();
        let mut conflicts = Vec::new();
        for side in Side::ALL {
            let mut here: Vec<_> =
                self.peers.iter().filter(|(_, p)| p.side == side && p.desktop.is_some()).collect();
            here.sort_by_key(|(fp, _)| **fp);
            let Some((_, used)) = here.first() else { continue };
            sides.push(side);
            for (_, other) in &here[1..] {
                conflicts.push(format!(
                    "{} and {} are both set to the {side} of this computer, so only {} is used; \
                     move the other with `one-mk layout \"{}\" <left|right|top|bottom>`",
                    used.name, other.name, used.name, other.name
                ));
            }
        }
        if conflicts != self.conflicts {
            for c in &conflicts {
                tracing::warn!("{c}");
            }
            self.conflicts = conflicts;
        }
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
            // Not those of connected peers: a redundant connection could
            // outrank and replace a working link.
            let connected: HashSet<SocketAddr> = self.peers.values().map(|p| canonical(p.conn.remote_address())).collect();
            addrs.extend(self.extra_addrs.iter().filter(|a| !connected.contains(&canonical(**a))));
        }
        for addr in addrs {
            if !self.dialing.insert(addr) {
                continue;
            }
            let (node, dir, screens, tx, services) =
                (self.node.clone(), self.dir.clone(), screens.clone(), self.tx.clone(), self.services.clone());
            tokio::spawn(async move {
                match node.connect_timeout(addr, DIAL_TIMEOUT).await {
                    Ok(peer) => establish(peer, &node, &dir, &screens, &tx, services, None).await,
                    Err(e) => tracing::trace!(%addr, "dial failed: {e:#}"),
                }
                let _ = tx.send(AppEvent::DialDone(addr));
            });
        }
    }
}

/// Whether keys sent to a computer running `peer_os` trade Ctrl and Cmd.
/// Automatic means exactly one of the two computers is a Mac.
fn swap_applies(setting: Swap, local_os: u8, peer_os: u8) -> bool {
    match setting {
        Swap::On => true,
        Swap::Off => false,
        Swap::Auto => local_os != peer_os && [local_os, peer_os].contains(&os::MACOS) && !(local_os == os::UNKNOWN || peer_os == os::UNKNOWN),
    }
}

fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
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
    use one_mk_net::{Device, read_frame};
    use one_mk_proto::PROTOCOL_VERSION;

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
            ..Device::default()
        });
        t.save().unwrap();
    }

    /// The next injected event, skipping `Controlled` (see `next_raw_inject`).
    async fn next_inject(h: &mut NullHandles) -> Inject {
        loop {
            match next_raw_inject(h).await {
                Inject::Controlled(_) => {}
                other => return other,
            }
        }
    }

    async fn next_raw_inject(h: &mut NullHandles) -> Inject {
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
        /// Fingerprints of A and B.
        fa: String,
        fb: String,
        stop: watch::Sender<bool>,
        tasks: Vec<tokio::task::JoinHandle<Result<()>>>,
    }

    impl Duo {
        async fn start(tag: &str, clips: Option<(one_mk_clipboard::Clipboard, one_mk_clipboard::Clipboard)>) -> Duo {
            let mut duo = Self::start_with(tag, clips, |_, _| {}).await;
            duo.settle().await;
            duo
        }

        /// Starts both daemons after `setup` has edited A's record of B and
        /// B's record of A.
        async fn start_with(
            tag: &str,
            clips: Option<(one_mk_clipboard::Clipboard, one_mk_clipboard::Clipboard)>,
            setup: impl FnOnce(&mut Device, &mut Device),
        ) -> Duo {
            let (da, db) = (temp_dir(&format!("{tag}-a")), temp_dir(&format!("{tag}-b")));
            let (ia, ib) = (Identity::load_or_create(&da).unwrap(), Identity::load_or_create(&db).unwrap());
            trust(&da, &ib, Side::Right);
            trust(&db, &ia, Side::Left);
            let (mut ta, mut tb) = (TrustStore::load(&da).unwrap(), TrustStore::load(&db).unwrap());
            setup(ta.get_mut(&ib.fingerprint).unwrap(), tb.get_mut(&ia.fingerprint).unwrap());
            ta.save().unwrap();
            tb.save().unwrap();
            let (pa, pb) = (free_port(), free_port());
            let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
            let (ba, ha) = one_mk_input::null(desk.clone());
            let (bb, hb) = one_mk_input::null(desk);
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
            Duo { da, db, ha, hb, pb, fa: ia.fingerprint.to_string(), fb: ib.fingerprint.to_string(), stop, tasks: vec![a, b] }
        }

        /// Waits until each side has armed the edge facing the other.
        async fn settle(&mut self) {
            assert_eq!(next_capture(&mut self.ha).await, CaptureCmd::SetSides(vec![Side::Right]));
            assert_eq!(next_capture(&mut self.hb).await, CaptureCmd::SetSides(vec![Side::Left]));
        }

        /// What `daemon_dir`'s daemon says about its single paired peer.
        async fn peer_status(dir: &std::path::Path) -> PeerStatus {
            let text = control::request(dir, Request::Status).await.unwrap().expect("daemon running");
            Status::decode(&text).unwrap().peers.remove(0)
        }

        /// `one-mk layout` run on the computer with settings in `dir`.
        async fn layout(dir: &std::path::Path, peer: &str, side: Option<Side>, swap: Option<Swap>) {
            crate::manage::set_layout(dir, &peer[..8], side, swap).unwrap();
            assert!(control::request(dir, Request::Reload).await.unwrap().is_some());
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

    /// Queued motion is applied in order through the cursor, which clamps
    /// at edges, and only the final position is sent.
    #[test]
    fn coalesces_queued_motion() {
        let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
        let mut c = RemoteCursor::enter(desk, Side::Left, 0.5, 1); // at (0, 540)
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Pinned at the right edge, then back: a sum of deltas would be wrong.
        tx.send(Event::Motion { dx: 5000.0, dy: 0.0 }).unwrap();
        tx.send(Event::Motion { dx: -100.0, dy: 10.0 }).unwrap();
        tx.send(Event::Key { code: 30, down: true }).unwrap();
        tx.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
        let out = coalesce(&mut c, (10.0, 0.0), &mut rx);
        let pos = Pos::new(1920.0 - 1.0 / 256.0 - 100.0, 550.0);
        assert_eq!(out.moved, Some(Datagram::Motion { epoch: 1, seq: 3, pos }));
        assert_eq!(out.leave, None);
        // Stops at the key, which is handled next, in order.
        assert_eq!(out.next, Some(Event::Key { code: 30, down: true }));
        assert_eq!(rx.try_recv(), Ok(Event::Motion { dx: 1.0, dy: 0.0 }));

        // Leaving ends the run after the position reached so far.
        tx.send(Event::Motion { dx: -5000.0, dy: 0.0 }).unwrap();
        tx.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
        let out = coalesce(&mut c, (1.0, 0.0), &mut rx);
        assert!(matches!(out.moved, Some(Datagram::Motion { seq: 4, .. })));
        assert!(out.leave.is_some());
        assert_eq!(out.next, None);
        assert_eq!(rx.try_recv(), Ok(Event::Motion { dx: 1.0, dy: 0.0 }));
        assert!(rx.try_recv().is_err());
    }

    /// Waits for the next injected event that is not a pointer move.
    async fn next_non_move(h: &mut NullHandles) -> Inject {
        loop {
            match next_inject(h).await {
                Inject::MoveTo(_) => {}
                other => return other,
            }
        }
    }

    /// Control messages interleaved with a stream of motion datagrams all
    /// arrive, in order. (Reading the control stream used to be cancelled
    /// whenever a datagram won the race, which could split a frame.)
    #[tokio::test]
    async fn control_stream_survives_motion_flood() {
        let mut duo = Duo::start("flood", None).await;
        duo.enter_b().await;
        const N: usize = 400;
        for i in 0..N {
            let dy = if i % 2 == 0 { 1.0 } else { -1.0 };
            duo.ha.events.send(Event::Motion { dx: 0.0, dy }).unwrap();
            duo.ha.events.send(Event::Key { code: 30, down: i % 2 == 0 }).unwrap();
            if i % 50 == 0 {
                tokio::task::yield_now().await;
            }
        }
        for i in 0..N {
            assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 30, down: i % 2 == 0 }, "key {i}");
        }
        duo.stop().await;
    }

    /// When someone uses B's own mouse or keyboard, B releases what A held
    /// and A takes its pointer back.
    #[tokio::test]
    async fn local_input_takes_control_back() {
        let mut duo = Duo::start("local", None).await;
        duo.enter_b().await;
        duo.ha.events.send(Event::Key { code: 42, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 42, down: true });
        duo.hb.events.send(Event::LocalInput).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 42, down: false });
        assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(_) }));
        // A's input no longer reaches B.
        duo.ha.events.send(Event::Key { code: 30, down: true }).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let after: Vec<_> = std::iter::from_fn(|| duo.hb.inject.try_recv().ok()).collect();
        assert_eq!(after, vec![Inject::Controlled(false)], "only the end of control");
        duo.stop().await;
    }

    /// `one-mk takeback` on B (for Linux, where B's own mouse can't be
    /// noticed) works like using B's mouse: B releases what A held, and A
    /// takes its pointer back. The backend is told when control starts and
    /// ends, after the releases.
    #[tokio::test]
    async fn takeback_command_takes_control_back() {
        let mut duo = Duo::start("takeback", None).await;
        assert_eq!(control::request(&duo.db, Request::TakeBack).await.unwrap().as_deref(), Some("none\n"));
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        assert_eq!(next_raw_inject(&mut duo.hb).await, Inject::Controlled(true));
        assert_eq!(next_raw_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 540.0)));
        duo.ha.events.send(Event::Key { code: 42, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 42, down: true });

        let reply = control::request(&duo.db, Request::TakeBack).await.unwrap().unwrap();
        assert!(reply.starts_with("ok\n"), "{reply}");
        let mut rest = Vec::new();
        loop {
            match next_raw_inject(&mut duo.hb).await {
                Inject::MoveTo(_) => {}
                Inject::Controlled(false) => break,
                other => rest.push(other),
            }
        }
        assert_eq!(rest, vec![Inject::Key { code: 42, down: false }]);
        assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(_) }));
        let status = Status::decode(&control::request(&duo.db, Request::Status).await.unwrap().unwrap()).unwrap();
        assert_eq!(status.controlled_by, None);
        duo.stop().await;
    }

    /// Forgetting a computer ends its connection when the daemon reloads.
    #[tokio::test]
    async fn forgetting_a_computer_disconnects_it() {
        let duo = Duo::start("forget", None).await;
        assert!(Duo::peer_status(&duo.da).await.online);
        let mut trust = TrustStore::load(&duo.db).unwrap();
        trust.remove(&duo.fa[..8]).unwrap();
        trust.save().unwrap();
        assert!(control::request(&duo.db, Request::Reload).await.unwrap().is_some());
        for _ in 0..250 {
            if !Duo::peer_status(&duo.da).await.online {
                duo.stop().await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("A still sees B online after B forgot A");
    }

    fn owner_for_test() -> (Owner, mpsc::UnboundedReceiver<Inject>, mpsc::UnboundedSender<Inject>) {
        let fp = Identity::ephemeral().unwrap().fingerprint;
        let (tx, rx) = mpsc::unbounded_channel();
        (Owner::new(fp, 1, 7, Pos::new(0.0, 540.0)), rx, tx)
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<Inject>) -> Vec<Inject> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// A click that arrives after a newer position (it was queued until
    /// the ack, while the newest position went out at once on the faster
    /// path) happens where the user clicked, and the pointer then goes
    /// back to the newer position. Before, it stayed at the click: the
    /// newer position's resends were taken for repeats.
    #[test]
    fn late_click_does_not_rewind_the_pointer() {
        let (mut o, mut rx, tx) = owner_for_test();
        let (x, y) = (Pos::new(100.0, 500.0), Pos::new(900.0, 300.0));
        o.motion(7, 5, y, &tx); // Y (seq 5) overtakes...
        let click = |down| Control::Button { button: 0x110, down, seq: 2, pos: x };
        o.input(click(true), &tx); // ...a click at X (seq 2)
        o.input(click(false), &tx);
        o.motion(7, 5, y, &tx); // Y's resends
        let down = Inject::Button { button: 0x110, down: true };
        let up = Inject::Button { button: 0x110, down: false };
        let (mx, my) = (Inject::MoveTo(x), Inject::MoveTo(y));
        assert_eq!(drain(&mut rx), vec![my, mx, down, my, mx, up, my]);

        // An up-to-date click moves the pointer there and stays.
        let z = Pos::new(10.0, 10.0);
        o.input(Control::Button { button: 0x110, down: true, seq: 6, pos: z }, &tx);
        o.motion(7, 5, y, &tx); // an older datagram still in flight
        assert_eq!(drain(&mut rx), vec![Inject::MoveTo(z), down]);
        assert_eq!(o.pos, z);
    }

    /// Only real input keeps a controller from being let go after `IDLE`;
    /// keepalives (the same position again) only keep the lease.
    #[test]
    fn only_real_input_counts_against_idle() {
        let (mut o, _rx, tx) = owner_for_test();
        let start = o.last_input;
        std::thread::sleep(Duration::from_millis(5));
        o.motion(7, 0, Pos::new(0.0, 540.0), &tx); // keepalive of the Enter position
        assert_eq!(o.last_input, start);
        assert!(o.last_rx > start);
        o.motion(7, 1, Pos::new(1.0, 540.0), &tx);
        assert!(o.last_input > start, "a new position is input");
        let moved = o.last_input;
        std::thread::sleep(Duration::from_millis(5));
        o.input(Control::Key { code: 30, down: true }, &tx);
        assert!(o.last_input > moved, "a key is input");
    }

    /// A non-owner flooding B with control messages must not starve B's
    /// timers: the silent owner still loses its lease on time, and the key
    /// it held is released.
    #[tokio::test]
    async fn flood_does_not_delay_the_lease() {
        let mut duo = Duo::start("starve", None).await;
        let mut owner = RawPeer::connect("starve-c", &duo.db, duo.pb).await;
        let mut flooder = RawPeer::connect("starve-d", &duo.db, duo.pb).await;
        owner.send(Control::Enter { epoch: 1, pos: Pos::new(5.0, 5.0) }).await;
        assert_eq!(owner.next().await, Control::EnterAck { epoch: 1 });
        owner.send(Control::Key { code: 30, down: true }).await;
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(5.0, 5.0)));
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: true });
        let held_at = Instant::now();

        // Each of these makes B re-read its device list, so B handles them
        // far slower than they arrive: its event queue never runs dry.
        let mut flood = Vec::new();
        for _ in 0..30_000 {
            Control::Layout { side_of_you: Side::Bottom, version: 0 }.encode_frame(&mut flood);
        }
        let (tx_done, mut rx_done) = mpsc::unbounded_channel();
        let send = async {
            flooder.send.write_all(&flood).await.unwrap();
            let _ = tx_done.send(());
        };
        let released = async {
            assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: false });
            held_at.elapsed()
        };
        let ((), took) = tokio::join!(send, released);
        assert!(rx_done.try_recv().is_ok());
        assert!(took < LEASE + Duration::from_millis(600), "the lease ran {took:?}");
        assert_eq!(owner.next().await, Control::Revoked { epoch: 1 });
        drop((owner, flooder));
        duo.stop().await;
    }

    /// A paired device can't pile up connections that never say what they
    /// are for: past the limit, new ones are closed at once.
    #[tokio::test]
    async fn connections_being_set_up_are_limited() {
        let duo = Duo::start("pending", None).await;
        let dir = temp_dir("pending-c");
        let id = Identity::load_or_create(&dir).unwrap();
        trust(&duo.db, &id, Side::Top);
        let node = Node::bind(Arc::new(id), "127.0.0.1:0".parse().unwrap()).unwrap();
        let to = SocketAddr::from(([127, 0, 0, 1], duo.pb));
        let mut held = Vec::new();
        for _ in 0..MAX_PENDING_PER_PEER {
            held.push(node.connect(to).await.unwrap());
        }
        let extra = node.connect(to).await.unwrap();
        let closed = tokio::time::timeout(Duration::from_secs(3), extra.conn.closed()).await;
        assert!(matches!(closed, Ok(quinn::ConnectionError::ApplicationClosed(_))), "{closed:?}");
        for p in &held {
            assert!(p.conn.close_reason().is_none(), "within the limit, still open");
        }
        // Once those are gone, the device may connect again.
        for p in held.drain(..) {
            p.conn.close(0u32.into(), b"");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let again = node.connect(to).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(again.conn.close_reason().is_none());
        drop((again, extra, node));
        duo.stop().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pending_limits_and_release() {
        let p = Pending::default();
        let a = Identity::ephemeral().unwrap().fingerprint;
        let mine: Vec<_> = (0..MAX_PENDING_PER_PEER).map(|_| p.admit(a).unwrap()).collect();
        assert!(p.admit(a).is_none(), "per device");
        let others: Vec<_> = (MAX_PENDING_PER_PEER..MAX_PENDING)
            .map(|_| p.admit(Identity::ephemeral().unwrap().fingerprint).unwrap())
            .collect();
        assert!(p.admit(Identity::ephemeral().unwrap().fingerprint).is_none(), "in all");
        drop(mine);
        assert!(p.admit(a).is_some());
        drop(others);
        assert_eq!(p.0.lock().unwrap().total, 0);
        assert!(p.0.lock().unwrap().per_peer.is_empty());
    }

    /// A hand-driven peer, trusted by B, speaking the control protocol.
    struct RawPeer {
        _node: Node,
        dir: PathBuf,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        buf: Vec<u8>,
    }

    impl RawPeer {
        async fn connect(tag: &str, to_dir: &std::path::Path, to_port: u16) -> RawPeer {
            let dir = temp_dir(tag);
            let id = Identity::load_or_create(&dir).unwrap();
            trust(to_dir, &id, Side::Top);
            let node = Node::bind(Arc::new(id), "127.0.0.1:0".parse().unwrap()).unwrap();
            let peer = node.connect(SocketAddr::from(([127, 0, 0, 1], to_port))).await.unwrap();
            let (send, recv) = peer.open_stream(StreamKind::Control).await.unwrap();
            let mut raw = RawPeer { _node: node, dir, send, recv, buf: Vec::new() };
            // No screens, so B never sends its own pointer here.
            raw.send(Control::Hello { version: PROTOCOL_VERSION, name: "raw".into(), screens: vec![], nonce: 0, os: 0 })
                .await;
            assert!(matches!(raw.next().await, Control::Hello { .. }));
            assert!(matches!(raw.next().await, Control::Layout { .. }));
            raw
        }

        async fn send(&mut self, msg: Control) {
            let mut b = Vec::new();
            msg.encode_frame(&mut b);
            self.send.write_all(&b).await.unwrap();
        }

        async fn send_body(&mut self, body: &[u8]) {
            let mut b = (body.len() as u16).to_le_bytes().to_vec();
            b.extend_from_slice(body);
            self.send.write_all(&b).await.unwrap();
        }

        async fn next(&mut self) -> Control {
            let f = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.recv, &mut self.buf));
            f.await.expect("timed out").unwrap().expect("stream ended")
        }
    }

    impl Drop for RawPeer {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Only one machine controls B at a time; malformed messages don't
    /// break the link; a silent controller loses its lease.
    #[tokio::test]
    async fn one_controller_at_a_time() {
        let mut duo = Duo::start("owner", None).await;
        duo.enter_b().await; // A controls B
        let mut c = RawPeer::connect("owner-c", &duo.db, duo.pb).await;

        // Garbage that must be skipped, not tear the link down: a screen of
        // zero width, a message from a newer version, and a NaN scroll.
        let mut bad_screens = vec![2, 1];
        for v in [0i32, 0, 0, 1080] {
            bad_screens.extend_from_slice(&v.to_le_bytes());
        }
        c.send_body(&bad_screens).await;
        c.send_body(&[200, 1, 2, 3]).await;
        let mut nan = vec![7];
        nan.extend_from_slice(&f32::NAN.to_bits().to_le_bytes());
        nan.extend_from_slice(&0f32.to_bits().to_le_bytes());
        c.send_body(&nan).await;

        // A got there first: C is told B is busy, and its input is ignored.
        c.send(Control::Enter { epoch: 5, pos: Pos::new(5.0, 5.0) }).await;
        assert_eq!(c.next().await, Control::Busy { epoch: 5 });
        c.send(Control::Key { code: 31, down: true }).await;
        duo.ha.events.send(Event::Motion { dx: 10.0, dy: 0.0 }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(10.0, 540.0)));

        // Once A leaves, C may enter.
        duo.leave_b().await;
        c.send(Control::Enter { epoch: 6, pos: Pos::new(5.0, 5.0) }).await;
        assert_eq!(c.next().await, Control::EnterAck { epoch: 6 });
        while next_inject(&mut duo.hb).await != Inject::MoveTo(Pos::new(5.0, 5.0)) {}

        // C holds a key and goes silent: B releases it when the lease runs out.
        c.send(Control::Key { code: 30, down: true }).await;
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: true });
        let held_at = Instant::now();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code: 30, down: false });
        assert!(held_at.elapsed() >= LEASE - Duration::from_millis(50), "{:?}", held_at.elapsed());
        assert_eq!(c.next().await, Control::Revoked { epoch: 6 });

        // C's input is refused until it enters again.
        c.send(Control::Key { code: 31, down: true }).await;
        c.send(Control::Enter { epoch: 7, pos: Pos::new(6.0, 6.0) }).await;
        assert_eq!(c.next().await, Control::EnterAck { epoch: 7 });
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(6.0, 6.0)));
        drop(c);
        duo.stop().await;
    }

    /// Changing the layout on one computer applies at once, with no
    /// restart, and the other computer records the mirror image.
    #[tokio::test]
    async fn layout_change_converges() {
        let mut duo = Duo::start("layout", None).await;
        let (fa, fb) = (duo.fa.clone(), duo.fb.clone());

        // On A: B is now on the left.
        Duo::layout(&duo.da, &fb, Some(Side::Left), None).await;
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Side::Left]));
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Side::Right]));
        let (sa, sb) = (Duo::peer_status(&duo.da).await, Duo::peer_status(&duo.db).await);
        assert_eq!((sa.side, sa.online), (Side::Left, true));
        assert_eq!((sb.side, sb.online), (Side::Right, true));
        let version = |dir: &std::path::Path, fp: &str| {
            let t = TrustStore::load(dir).unwrap();
            let d = t.get(&fp.parse().unwrap()).unwrap().clone();
            (d.side(), d.layout_version)
        };
        let (side_a, v) = version(&duo.da, &fb);
        assert_eq!(side_a, Side::Left);
        assert!(v > 0);
        assert_eq!(version(&duo.db, &fa), (Side::Right, v), "B stored the same version");

        // Then on B: A is above. The newer change wins over the old one.
        Duo::layout(&duo.db, &fa, Some(Side::Top), None).await;
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Side::Top]));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Side::Bottom]));
        assert_eq!(Duo::peer_status(&duo.da).await.side, Side::Bottom);
        assert!(version(&duo.da, &fb).1 > v);
        duo.stop().await;
    }

    /// A layout changed while the daemons were not talking is settled when
    /// they connect: the newer version wins on both.
    #[tokio::test]
    async fn newer_layout_wins_on_connect() {
        let mut duo = Duo::start_with("version", None, |a_sees_b, _| {
            // A was told "B is on the left" while offline; B still thinks
            // A is on its left too (an old, version 0 layout).
            a_sees_b.side = Side::Left.to_string();
            a_sees_b.layout_version = 5;
        })
        .await;
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Side::Left]));
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Side::Left]));
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Side::Right]));
        let t = TrustStore::load(&duo.db).unwrap();
        let d = t.get(&duo.fa.parse().unwrap()).unwrap();
        assert_eq!((d.side(), d.layout_version), (Side::Right, 5));
        duo.stop().await;
    }

    /// With swapping on, Ctrl and Cmd trade places in what A sends. The
    /// choice is fixed per visit, so a key released after the setting
    /// changed still releases the code that was pressed.
    #[tokio::test]
    async fn ctrl_and_cmd_swap_consistently() {
        let mut duo = Duo::start_with("swap", None, |a_sees_b, _| a_sees_b.swap_ctrl_cmd = Swap::On).await;
        duo.settle().await;
        duo.enter_b().await;
        for (code, sent) in [(29, 125), (30, 30)] {
            duo.ha.events.send(Event::Key { code, down: true }).unwrap();
            assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: sent, down: true });
        }
        // The setting changes mid-visit.
        let fb = duo.fb.clone();
        Duo::layout(&duo.da, &fb, None, Some(Swap::Off)).await;
        for (code, sent) in [(29, 125), (30, 30)] {
            duo.ha.events.send(Event::Key { code, down: false }).unwrap();
            assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: sent, down: false });
        }
        // Still swapped for the rest of this visit; leaving releases the swapped code.
        duo.ha.events.send(Event::Key { code: 97, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 126, down: true });
        duo.leave_b().await;
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 126, down: false });

        // The next visit follows the new setting.
        duo.enter_b().await;
        duo.ha.events.send(Event::Key { code: 29, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 29, down: true });
        duo.stop().await;
    }

    #[test]
    fn swaps_only_between_a_mac_and_something_else() {
        use one_mk_proto::os::{LINUX, MACOS, UNKNOWN, WINDOWS};
        for (local, peer, expected) in [
            (LINUX, MACOS, true),
            (MACOS, WINDOWS, true),
            (MACOS, LINUX, true),
            (MACOS, MACOS, false),
            (LINUX, WINDOWS, false),
            (LINUX, LINUX, false),
            (MACOS, UNKNOWN, false),
        ] {
            assert_eq!(swap_applies(Swap::Auto, local, peer), expected, "{local} -> {peer}");
            assert!(swap_applies(Swap::On, local, peer));
            assert!(!swap_applies(Swap::Off, local, peer));
        }
    }
}
