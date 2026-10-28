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
use crate::link::{self, Link, LinkEvent, Rank};
use crate::transfer::{ConnPermit, Inbox};
use anyhow::{Context, Result};
use one_mk_input::{Backend, CaptureCmd, Event, Inject};
use one_mk_net::{ControlWriter, Discovery, Fingerprint, Found, Identity, Node, Peer, TrustStore, read_stream_kind};
use one_mk_proto::{
    Control, Datagram, Desktop, HeldKeys, MotionReceiver, Pos, Received, Rect, RemoteCursor, Side, Step, StreamKind,
    sanitize_name,
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
/// If a peer doesn't answer `Enter` within this, the pointer comes home.
const ENTER_TIMEOUT: Duration = Duration::from_millis(750);
const DIAL_INTERVAL: Duration = Duration::from_secs(2);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a peer that connected to us may take to open its first stream.
const FIRST_STREAM_TIMEOUT: Duration = Duration::from_secs(10);
/// Discovery announcements from unknown devices re-read the trusted device
/// list (it may have been paired since) at most this often.
const TRUST_RELOAD_INTERVAL: Duration = Duration::from_secs(5);

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
    while let Ok((send, mut recv)) = conn.accept_bi().await {
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
    held: HeldKeys,
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
    held: HeldKeys,
    /// Last valid traffic from the owner for this visit.
    last_rx: Instant,
}

impl Owner {
    fn input(&mut self, msg: Control, inject: &mpsc::UnboundedSender<Inject>) {
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
                self.motion.sync(seq);
                inject(Inject::MoveTo(pos));
                if self.held.update(button, down) {
                    inject(Inject::Button { button, down });
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
    let services = Services {
        clip: opts.clipboard.map(|cb| ClipSync::new(cb, node.clone())),
        inbox: Inbox::new(opts.downloads.clone()),
    };
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
                Some(ev) => app.on_input(ev, &mut events),
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
/// was dialed to us may instead start with a file or clipboard stream: a
/// bulk connection, used for clipboard snapshots (so they never share
/// congestion control with input) or by `one-mk send` (which shares the
/// peer service's identity). It never replaces the link.
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
            Ok(Ok((kind @ (StreamKind::File | StreamKind::Clipboard), send, recv))) => {
                tracing::debug!(peer = %device_name, %addr, "bulk connection");
                serve_streams(peer.conn, peer.fingerprint, device_name, services, (kind, send, recv)).await;
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
    loop {
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
}

impl App {
    // ------------------------------------------------------------ local input

    fn on_input(&mut self, ev: Event, events: &mut mpsc::UnboundedReceiver<Event>) {
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
                if r.held.update(code, down) {
                    self.send_input(Control::Key { code, down });
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
        peer.ctl.send(Control::Enter { epoch: self.epoch, pos: cursor.pos() });
        tracing::debug!(peer = %peer.name, %side, "pointer moved to peer");
        let now = Instant::now();
        self.remote = Some(Remote {
            activation,
            peer: fp,
            side,
            cursor,
            held: HeldKeys::new(),
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
        let lease = self.owner.as_ref().map(|o| o.last_rx + LEASE);
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
        let mut motion = MotionReceiver::default();
        motion.enter(epoch);
        let _ = self.inject.send(Inject::MoveTo(pos));
        peer.ctl.send(Control::EnterAck { epoch });
        if let Some(clip) = &self.services.clip {
            clip.entered(fp, &peer.name);
        }
        let held = HeldKeys::new();
        self.owner = Some(Owner { peer: fp, link: id, epoch, motion, held, last_rx: Instant::now() });
    }

    /// Ends the current owner's control, releasing everything it held.
    /// With `revoke`, tells it to take its pointer back.
    fn end_owner(&mut self, revoke: bool) {
        let Some(mut o) = self.owner.take() else { return };
        o.release_all(&self.inject);
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
        if let Ok(t) = TrustStore::load(&self.dir) {
            self.trust = t;
        }
        let side = self.trust.get(&fp).map_or(Side::Right, |d| d.side());
        let addr = canonical(link.conn.remote_address());
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
        tracing::info!(peer = %link.name, %addr, %side, os = link.os, "connected");
        let desktop = (!link.screens.is_empty()).then(|| Desktop::new(link.screens.clone()));
        let (id, name, rank, sent) = (link.id, link.name.clone(), link.rank, link.sent_screens.clone());
        let (conn, ctl) = link.spawn(self.link_tx.clone());
        // Our monitors may have changed during the handshake.
        if let Some(local) = &self.local
            && local.screens != sent
        {
            ctl.send(Control::Screens(local.screens.clone()));
        }
        if let Some(old) = self.peers.insert(fp, Connected { id, name, rank, side, conn, ctl, desktop }) {
            old.conn.close(2u32.into(), b"duplicate");
        }
        self.update_sides();
    }

    fn on_link(&mut self, fp: Fingerprint, id: usize, ev: LinkEvent) {
        if !self.peers.get(&fp).is_some_and(|p| p.id == id) {
            return; // a replaced link
        }
        match ev {
            LinkEvent::Motion { epoch, seq, pos } => {
                let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id) else { return };
                match o.motion.accept(epoch, seq, pos) {
                    Received::Move(p) => {
                        o.last_rx = Instant::now();
                        let _ = self.inject.send(Inject::MoveTo(p));
                    }
                    Received::Repeat => o.last_rx = Instant::now(),
                    Received::Stale => {}
                }
            }
            LinkEvent::Control(msg) => self.on_control(fp, id, msg),
            LinkEvent::Closed => {
                let peer = self.peers.remove(&fp).expect("present");
                if self.owner.as_ref().is_some_and(|o| o.peer == fp) {
                    self.end_owner(false);
                }
                tracing::info!(peer = %peer.name, "disconnected");
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
            Control::Enter { epoch, pos } => self.on_enter(fp, id, epoch, pos),
            Control::Leave => {
                if self.is_owner(fp, id) {
                    self.end_owner(false);
                }
            }
            Control::EnterAck { epoch } => {
                let Some(r) = self.remote.as_mut().filter(|r| r.peer == fp && r.cursor.epoch() == epoch && !r.acked)
                else {
                    return;
                };
                r.acked = true;
                let peer = &self.peers[&fp];
                for m in r.queued.drain(..) {
                    peer.ctl.send(m);
                }
                r.send_motion(&self.peers, r.cursor.current());
                if let Some(clip) = &self.services.clip {
                    clip.push(fp, &peer.name, peer.conn.remote_address());
                }
            }
            Control::Busy { epoch } => self.go_home_from(fp, epoch, "it is controlled by another computer"),
            Control::Revoked { epoch } => self.go_home_from(fp, epoch, "it took back control"),
            input => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id) {
                    o.last_rx = Instant::now();
                    o.input(input, &self.inject);
                }
            }
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
                    Ok(peer) => establish(peer, &node, &dir, &screens, &tx, services).await,
                    Err(e) => tracing::trace!(%addr, "dial failed: {e:#}"),
                }
                let _ = tx.send(AppEvent::DialDone(addr));
            });
        }
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
        assert!(duo.hb.inject.try_recv().is_err());
        duo.stop().await;
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
}
