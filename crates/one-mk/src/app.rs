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
use crate::profiles::{NetworkId, Profiles};
use crate::manage::{LayoutStatus, LayoutVerdict, PeerStatus, Place, Status, on_your, placement, resolve_layout, takeback_hint};
use crate::settings::{Gaming, Settings};
use crate::transfer::{ConnPermit, Inbox};
use anyhow::{Context, Result};
use one_mk_ctl::client::SwitchTo;
use one_mk_input::{Armed, Backend, CaptureCmd, DisplaySleeper, Event, FocusProbe, Inhibitor, Inject, PowerEvent, ScreenLocker};
use one_mk_net::{
    ControlWriter, Discovery, Fingerprint, Found, Identity, Node, Peer, Swap, TrustStore, read_stream_kind,
};
use one_mk_proto::{
    Align, Approach, Control, Datagram, Desktop, HeldKeys, LockSync, Mods, MotionReceiver, Next, OnRemote,
    OnRemoteSleep, Pos, Received, Rect, Refusal, RelReceived, RemoteCursor, Seam, Side, SleepSync, Start, Step,
    StreamKind, FOCUS_REPEAT, FOCUS_SILENCE, caps, cycle_next, fullscreen_blocks, os, overlaps, sanitize_name,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

mod shortcuts;
mod tune;

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
/// `one-mk switch` gives up if the backend doesn't start capturing in this.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(1);
/// Paired computers that are not connected are dialed again after this,
/// doubling up to [`DIAL_MAX`] while they stay away. Only while one has an
/// address to try: with every one connected no timer runs.
const DIAL_INTERVAL: Duration = Duration::from_secs(2);
const DIAL_MAX: Duration = Duration::from_secs(30);
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

/// After input from the controller that may move the focus (a click, a
/// window-switching chord), the focus is looked at again this much later,
/// once the system has acted on it (gaming mode `auto`).
const FOCUS_SETTLE: Duration = Duration::from_millis(100);

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
    /// Start with the pointer locked to this computer (`one-mk lock-pointer`).
    pub start_locked: bool,
    /// The OS's power functions (screensaver, display sleep); `None` for
    /// those of `backend`'s kind. Tests pass fakes they can watch.
    pub power: Option<one_mk_input::Power>,
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
        (StreamKind::Clipboard, Some(clip)) => clip.receive(permit.fp(), name, send, recv).await,
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
    /// The networks this computer is on changed (only watched while some
    /// layout profile switches by itself).
    Network(NetworkId),
    /// Another window came to the front (only watched while a controller
    /// asks what has the focus, and where the OS says so: Windows).
    FocusMoved,
}

struct Connected {
    id: usize,
    name: String,
    /// The peer's OS (see [`one_mk_proto::os`]).
    os: u8,
    rank: Rank,
    side: Side,
    /// How its edge lines up with ours along `side`.
    align: Align,
    conn: quinn::Connection,
    ctl: ControlWriter,
    desktop: Option<Desktop>,
    /// What it supports besides the basics (bits of [`caps`]); 0 until it says.
    caps: u32,
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
    /// The peer says a fullscreen window has its focus (gaming mode `auto`).
    peer_fullscreen: bool,
    /// When the peer last said what has its focus.
    focus_heard: Instant,
    /// We asked the peer to report whether a fullscreen window has its
    /// focus (gaming `auto`), and which app has it (per-app input rules),
    /// with `WatchFocus`.
    watching_fullscreen: bool,
    watching_app: bool,
    /// The names of the app with the focus there, as the peer last said
    /// (empty: unknown, so no per-app rule applies), and whether it said
    /// anything about it this visit.
    peer_apps: Vec<String>,
    apps_heard: bool,
    /// We asked the peer to keep its screen awake (`Inhibit`).
    inhibit_sent: bool,
    /// Key mapper, pointer, scroll and button tuning for this peer.
    tune: tune::Tune,
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
    /// The owner sends relative motion (gaming mode), so its clicks carry
    /// no position to apply.
    relative: bool,
    /// The owner asked to hear what has the focus: whether a fullscreen
    /// window has it, which app has it (only it is ever told that), or both.
    watch_focus: bool,
    watch_fullscreen: bool,
    watch_app: bool,
    /// Whether a fullscreen window has the focus, and the app's names, as
    /// last told to the owner.
    focus_sent: Option<(bool, Vec<String>)>,
    /// When to look at the focus again (see `App::report_focus`).
    focus_check: Option<Instant>,
    /// The owner asked to keep this screen awake while it controls it.
    inhibit: bool,
    /// Modifier keys pressed during this visit; released once more after
    /// the screen unlocks (see `App::on_screen_locked`).
    mods_pressed: Mods,
}

impl Owner {
    fn new(peer: Fingerprint, link: usize, epoch: u16, pos: Pos) -> Self {
        let mut motion = MotionReceiver::default();
        motion.enter(epoch);
        let now = Instant::now();
        Self {
            peer,
            link,
            epoch,
            motion,
            pos,
            held: HeldKeys::new(),
            last_rx: now,
            last_input: now,
            relative: false,
            watch_focus: false,
            watch_fullscreen: false,
            watch_app: false,
            focus_sent: None,
            focus_check: None,
            inhibit: false,
            mods_pressed: Mods::default(),
        }
    }

    /// A relative-motion datagram from the owner.
    fn motion_by(&mut self, epoch: u16, seq: u32, sum: (i32, i32), inject: &mpsc::UnboundedSender<Inject>) {
        match self.motion.accept_relative(epoch, seq, sum) {
            RelReceived::MoveBy { dx, dy } => {
                let now = Instant::now();
                self.last_rx = now;
                self.last_input = now;
                if dx != 0 || dy != 0 {
                    let _ = inject.send(Inject::MoveBy { dx, dy });
                }
            }
            RelReceived::Repeat => self.last_rx = Instant::now(),
            RelReceived::Stale => {}
        }
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
                if down && Mods::is_modifier(code) {
                    self.mods_pressed.update(code, true);
                }
                if self.held.update(code, down) {
                    inject(Inject::Key { code, down });
                }
            }
            // Relative motion: the click happens wherever the pointer is.
            Control::Button { button, down, .. } if self.relative => {
                if self.held.update(button, down) {
                    inject(Inject::Button { button, down });
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
    backend_name: &'static str,
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
    sides: Vec<Armed>,
    /// Layout conflicts last warned about.
    conflicts: Vec<String>,
    discovered: HashMap<Fingerprint, Vec<SocketAddr>>,
    extra_addrs: Vec<SocketAddr>,
    dialing: HashSet<SocketAddr>,
    services: Services,
    /// Peers whose first time controlling this machine (this run) has been
    /// announced with how to take control back.
    announced: HashSet<Fingerprint>,
    focus: Box<dyn FocusProbe>,
    locker: Box<dyn ScreenLocker>,
    settings: Settings,
    /// `one-mk lock-pointer`: edges lead nowhere (no barriers are armed).
    locked: bool,
    /// Modifier keys held here, as far as we know (`Event::Modifiers`, and
    /// captured keys); forgotten whenever capture ends.
    mods: Mods,
    /// The pointer is held at an edge (capture `.0`), waiting to cross.
    approach: Option<(u32, Approach)>,
    /// Capture `.0` was handed straight back (a wall, a corner, the
    /// fullscreen guard): input it caught before the release took effect
    /// was meant for this computer, and is typed here.
    handed_back: Option<u32>,
    /// Keys and buttons pressed and released here since capture was
    /// handed back (`handed_back` or a release at the edge): their real
    /// releases, if that capture catches them too, add nothing.
    tapped: Vec<u32>,
    /// `one-mk switch` asked the backend to capture: the pointer goes to
    /// this peer, unless capture hasn't started by then.
    switching: Option<(Fingerprint, Instant)>,
    lock_sync: LockSync,
    /// Modifier keys to release once more when the screen unlocks.
    unlock_release: Mods,
    /// Runs `[hooks]` when the pointer arrives or leaves, peers come or go.
    hooks: crate::hooks::Engine,
    inhibitor: Box<dyn Inhibitor>,
    sleeper: Box<dyn DisplaySleeper>,
    /// Keeps this screen awake while a peer that asked controls it.
    awake: crate::power::Awake,
    sleep_sync: SleepSync,
    shortcuts: shortcuts::Shortcuts,
    /// Network changes are watched (for automatic layout profiles).
    watching_network: bool,
    /// The automatic profile the current network matched, if any.
    auto_profile: Option<String>,
    /// A network change has been seen since starting.
    network_seen: bool,
    /// What the input backend says needs the user, by part (`Event::Status`).
    input_problems: std::collections::BTreeMap<&'static str, String>,
    /// The focus probe is asked to say when the front window changes.
    focus_watched: bool,
    /// When to dial missing peers again, and the wait after that.
    dial_at: Option<Instant>,
    dial_backoff: Duration,
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

    let Backend { name, mut events, capture, inject, focus, locker } = backend;
    let services = Services {
        clip: opts.clipboard.map(|cb| ClipSync::new(cb, node.clone())),
        inbox: Inbox::new(opts.downloads.clone()),
    };
    tracing::debug!(
        clipboard = services.clip.is_some(),
        downloads = %opts.downloads.display(),
        "sharing"
    );
    if let Err(e) = crate::clipctl::apply_settings(&dir, &services) {
        tracing::warn!("settings.toml: {e:#}; using the defaults");
    }

    let mut power = opts.power.unwrap_or_else(|| one_mk_input::power(opts.backend));
    let mut power_events = std::mem::replace(&mut power.events, mpsc::unbounded_channel().1);
    let mut power_open = true;
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

    let settings = Settings::load_or_default(&dir);
    let computers = crate::hotkeys::numbered_from(&identity, &trust).len();
    let (shortcuts, mut shortcut_events) = shortcuts::Shortcuts::start(opts.backend, &settings.hotkeys, computers);
    let mut app = App {
        dir,
        backend_name: name,
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
        focus,
        locker,
        settings,
        locked: opts.start_locked,
        mods: Mods::default(),
        approach: None,
        handed_back: None,
        tapped: Vec::new(),
        switching: None,
        lock_sync: LockSync::default(),
        unlock_release: Mods::default(),
        hooks: Default::default(),
        inhibitor: power.inhibitor,
        sleeper: power.sleeper,
        awake: Default::default(),
        sleep_sync: SleepSync::default(),
        shortcuts,
        watching_network: false,
        auto_profile: None,
        network_seen: false,
        input_problems: Default::default(),
        focus_watched: false,
        dial_at: None,
        dial_backoff: DIAL_INTERVAL,
    };
    crate::hooks::warn_if_unsafe(&app.dir, &app.settings.hooks);
    if app.locked {
        println!("The pointer is locked to this computer (`one-mk lock-pointer off` frees it).");
    }
    app.dial_missing(&screens_rx);
    app.watch_network_if_wanted();

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
            ev = power_events.recv(), if power_open => match ev {
                Some(ev) => app.on_power(ev),
                None => power_open = false,
            },
            Some(ev) = rx.recv() => app.on_app(ev, &screens_rx),
            Some((req, reply)) = ipc.recv() => app.on_request(req, reply),
            Some(ev) = shortcut_events.recv() => app.on_shortcut_event(ev),
            _ = tokio::time::sleep_until(app.dial_at.unwrap_or_else(Instant::now)), if app.dial_at.is_some() => {
                app.dial_missing(&screens_rx);
            }
            _ = &mut shutdown => break,
        }
        app.sync_hooks();
        app.sync_power();
        app.shortcuts.set_controlled(app.owner.is_some());
        app.sync_focus_watch();
    }

    app.go_home();
    if let Some((id, a)) = app.approach.take() {
        app.release_capture(id, Some(a.pos()));
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
pub(crate) struct Pending(Arc<std::sync::Mutex<PendingCounts>>);

#[derive(Default)]
struct PendingCounts {
    total: usize,
    per_peer: HashMap<Fingerprint, usize>,
}

/// One connection being set up; released on drop.
pub(crate) struct PendingPermit {
    pending: Pending,
    fp: Fingerprint,
}

impl Pending {
    pub(crate) fn admit(&self, fp: Fingerprint) -> Option<PendingPermit> {
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
    /// The pointer left through the home edge, coming home here.
    leave: Option<Pos>,
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
    tune: impl Fn(f64, f64) -> (f64, f64),
) -> Coalesced {
    let mut out = Coalesced::default();
    let (dx, dy) = tune(dx, dy);
    let mut step = cursor.motion(dx, dy);
    for _ in 0..MAX_COALESCE {
        match step {
            Step::Move(d) => out.moved = Some(d),
            Step::Leave { at } => {
                out.leave = Some(at);
                return out;
            }
            Step::Still => {}
        }
        match events.try_recv() {
            Ok(Event::Motion { dx, dy }) => {
                let (dx, dy) = tune(dx, dy);
                step = cursor.motion(dx, dy);
            }
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
        Step::Leave { at } => out.leave = Some(at),
        Step::Still => {}
    }
    out
}

impl App {
    // ------------------------------------------------------------ local input

    fn on_input(&mut self, ev: Event, events: &mut mpsc::UnboundedReceiver<Event>) {
        match ev {
            Event::Key { code, down } if self.captured_shortcut(code, down) => {}
            ev @ (Event::Hotkey(_) | Event::Hotkeys(_)) => self.on_shortcut_event(ev),
            Event::Desktop(d) => {
                tracing::debug!(screens = ?d.screens, "local screens");
                for p in self.peers.values() {
                    p.ctl.send(Control::Screens(d.screens.clone()));
                }
                let _ = self.local_screens.send(d.screens.clone());
                if let Some(r) = self.remote.as_mut() {
                    r.cursor.set_local(d.clone());
                }
                self.local = Some(d);
                self.update_sides();
            }
            Event::Activated { id, side, pos } => self.on_activated(id, side, pos),
            Event::Motion { dx, dy } => {
                if let (Some((_, a)), Some(local)) = (self.approach.as_mut(), self.local.as_ref()) {
                    let next = a.motion(local, dx, dy, std::time::Instant::now());
                    self.on_approach(next, None);
                } else if let Some(next) = self.on_motion(dx, dy, events) {
                    self.on_input(next, events);
                }
            }
            Event::Button { button, down } if self.approach.is_some() => {
                self.approach_input(Inject::Button { button, down });
            }
            Event::Key { code, down } if self.approach.is_some() => {
                self.mods.update(code, down);
                let (Some((_, a)), Some(local)) = (self.approach.as_mut(), self.local.as_ref()) else { return };
                let next = a.key(local, code, down, std::time::Instant::now());
                self.on_approach(next, None); // `next` says what to type here
            }
            Event::Scroll { dx, dy } if self.approach.is_some() => self.approach_input(Inject::Scroll { dx, dy }),
            Event::ScrollDiscrete { dx, dy } if self.approach.is_some() => {
                self.approach_input(Inject::ScrollDiscrete { dx, dy });
            }
            Event::ScrollStop if self.approach.is_some() => self.approach_input(Inject::ScrollStop),
            Event::Modifiers(m) => {
                self.mods = m;
                if let (Some((_, a)), Some(local)) = (self.approach.as_mut(), self.local.as_ref()) {
                    let next = a.modifiers(local, m, std::time::Instant::now());
                    self.on_approach(next, None);
                }
            }
            Event::LocalKeyUp { code } => {
                // Stays here, but shortcuts and modifier-to-cross must no
                // longer count it.
                if self.mods.update(code, false)
                    && let (Some((_, a)), Some(local)) = (self.approach.as_mut(), self.local.as_ref())
                {
                    let next = a.modifiers(local, self.mods, std::time::Instant::now());
                    self.on_approach(next, None);
                }
            }
            Event::ScreenLocked(locked) => self.on_screen_locked(locked),
            ev @ (Event::Button { .. }
            | Event::Key { .. }
            | Event::Scroll { .. }
            | Event::ScrollDiscrete { .. }
            | Event::ScrollStop)
                if self.handed_back.is_some() && self.remote.is_none() =>
            {
                self.replay_here(ev);
            }
            Event::Button { button, down } => {
                let Some(r) = self.remote.as_mut() else { return };
                if r.held.update(button, down) {
                    self.tuned_key(button, down);
                }
            }
            Event::Key { code, down } => {
                self.mods.update(code, down);
                let Some(r) = self.remote.as_mut() else { return };
                if down && code == KEY_ESC && [CTRL, ALT, SHIFT].iter().all(|k| k.iter().any(|&c| r.held.is_held(c))) {
                    tracing::info!("emergency shortcut: bringing the pointer home");
                    self.go_home();
                    return;
                }
                // Remapped (then Ctrl/Cmd swapped) in `tune.rs`.
                if r.held.update(code, down) {
                    self.tuned_key(code, down);
                }
            }
            Event::Scroll { dx, dy } => self.tuned_scroll(tune::ScrollIn::Smooth(dx, dy)),
            Event::ScrollDiscrete { dx, dy } => self.tuned_scroll(tune::ScrollIn::Discrete(dx, dy)),
            Event::ScrollStop => self.tuned_scroll(tune::ScrollIn::Stop),
            Event::LocalInput => {
                if self.owner.is_some() {
                    tracing::info!("this computer's own mouse or keyboard was used; it takes back control");
                    self.end_owner(true);
                }
            }
            Event::Deactivated { id } => {
                if self.handed_back == Some(id) {
                    self.handed_back = None;
                    self.tapped.clear();
                }
                if self.approach.as_ref().is_some_and(|(a, _)| *a == id) {
                    self.approach = None;
                    self.mods = Mods::default();
                }
                if self.remote.as_ref().is_some_and(|r| r.activation == id) {
                    tracing::info!("capture ended by the system; pointer is home");
                    self.end_visit();
                }
            }
            Event::Error(msg) => tracing::error!("{msg}"),
            Event::Status { part, problem: Some(p) } => {
                if self.input_problems.get(part) != Some(&p) {
                    println!("{p}");
                    self.input_problems.insert(part, p);
                }
            }
            Event::Status { part, problem: None } => {
                self.input_problems.remove(part);
            }
        }
    }

    /// Moves the remote pointer, coalescing motion that queued up meanwhile.
    /// Returns the event that ended the run, to be handled next.
    fn on_motion(&mut self, dx: f64, dy: f64, events: &mut mpsc::UnboundedReceiver<Event>) -> Option<Event> {
        let r = self.remote.as_mut()?;
        let tune = &r.tune;
        let c = coalesce(&mut r.cursor, (dx, dy), events, |dx, dy| tune.motion(dx, dy));
        if let Some(d) = c.moved
            && r.acked
        {
            r.send_motion(&self.peers, d);
        }
        if let Some(at) = c.leave {
            self.go_home_at(Some(at));
        }
        c.next
    }

    /// The pointer hit the `side` edge of our desktop and is captured: go
    /// on to the peer there, hand it back, or hold it at the edge, as the
    /// edge rules say.
    fn on_activated(&mut self, id: u32, side: Side, pos: Pos) {
        self.end_visit(); // a new activation supersedes any visit in progress
        self.approach = None;
        self.handed_back = None;
        self.tapped.clear();
        if let Some((fp, _)) = self.switching.take() {
            self.enter_peer_at_center(id, fp);
            return;
        }
        let Some(local) = self.local.as_ref() else {
            self.release_capture(id, None);
            return;
        };
        if self.peer_at(side, pos).is_none() {
            // Like the wall beside a real monitor: the neighbours on this
            // side are lined up elsewhere along it.
            tracing::debug!(%side, ?pos, "not crossing: no computer faces this part of the edge");
            self.hand_back(id);
            return;
        }
        if self.locked {
            // Normally unreachable: no barriers are armed while locked.
            tracing::debug!(%side, "not crossing: {}", Refusal::Locked.as_str());
            self.hand_back(id);
            return;
        }
        let guard = self.settings.edge.guard(side);
        let edge = &self.settings.edge;
        let blocked = guard.fullscreen
            && fullscreen_blocks(true, self.focus.focus().as_ref(), &edge.fullscreen_allow, &edge.fullscreen_block);
        match Approach::start(guard, local, side, pos, self.mods, blocked, std::time::Instant::now()) {
            Start::Cross(p) => self.enter_peer(id, side, p),
            Start::Refuse(why) => {
                tracing::debug!(%side, "not crossing: {}", why.as_str());
                self.hand_back(id);
            }
            Start::Wait(a) => {
                tracing::debug!(%side, ?guard, "holding the pointer at the edge");
                self.approach = Some((id, a));
            }
        }
    }

    /// The pointer is held at an edge and something happened.
    /// `input` is what happened, if it must be replayed here when the
    /// pointer is handed back.
    fn on_approach(&mut self, next: Next, input: Option<Inject>) {
        match next {
            Next::Wait => {}
            Next::Cross(pos) => {
                let Some((id, a)) = self.approach.take() else { return };
                self.enter_peer(id, a.side(), pos);
            }
            Next::Release { pos, around, keys } => {
                let Some((id, _)) = self.approach.take() else { return };
                self.release_capture(id, Some(pos));
                self.handed_back = Some(id);
                // Captured while held, but meant for this computer. Every
                // key pressed here is released here too: the user's own
                // key-ups bypass the injector, which would hold (and on
                // Windows and macOS repeat) anything left down. A click
                // likewise (see `type_here`), so a drag cannot start at a
                // guarded edge.
                let press = around.iter().map(|&code| Inject::Key { code, down: true });
                let typed = keys.iter().map(|&(code, down)| Inject::Key { code, down });
                for i in press.chain(typed) {
                    let _ = self.inject.send(i);
                }
                if let Some(i) = input {
                    self.type_here(i);
                }
                for &code in around.iter().rev() {
                    let _ = self.inject.send(Inject::Key { code, down: false });
                }
                let pressed = keys.into_iter().filter_map(|(code, down)| down.then_some(code));
                for code in around.into_iter().chain(pressed) {
                    self.note_tapped(code);
                }
            }
        }
    }

    /// A click or scroll while the pointer is held at an edge.
    fn approach_input(&mut self, input: Inject) {
        let Some((_, a)) = self.approach.as_mut() else { return };
        let next = a.other_input();
        self.on_approach(next, Some(input));
    }

    /// Refuses capture `id` at once, leaving the pointer where it is.
    fn hand_back(&mut self, id: u32) {
        self.release_capture(id, None);
        self.handed_back = Some(id);
    }

    /// Input caught by a capture handed straight back (`handed_back`):
    /// typed here.
    fn replay_here(&mut self, ev: Event) {
        let input = match ev {
            Event::Key { code, down } => Inject::Key { code, down },
            Event::Button { button, down } => Inject::Button { button, down },
            Event::Scroll { dx, dy } => Inject::Scroll { dx, dy },
            Event::ScrollDiscrete { dx, dy } => Inject::ScrollDiscrete { dx, dy },
            Event::ScrollStop => Inject::ScrollStop,
            _ => return,
        };
        self.type_here(input);
    }

    /// Types `input`, caught by a capture handed back, on this computer. A
    /// press as a whole tap or click: its real release may be caught too,
    /// go past the injector (whose press would then stay down), or never be
    /// seen (the portal and wlroots don't report it), and no backend can be
    /// trusted to end a press held for it. A release whose press was typed
    /// so adds nothing; any other is typed (its press went to this computer
    /// before the capture began).
    fn type_here(&mut self, input: Inject) {
        let (code, down, up) = match input {
            Inject::Key { code, down } => (code, down, Inject::Key { code, down: false }),
            Inject::Button { button, down } => (button, down, Inject::Button { button, down: false }),
            other => {
                let _ = self.inject.send(other);
                return;
            }
        };
        if down {
            let _ = self.inject.send(input);
            let _ = self.inject.send(up);
            self.note_tapped(code);
        } else if let Some(i) = self.tapped.iter().position(|&c| c == code) {
            self.tapped.swap_remove(i);
        } else {
            let _ = self.inject.send(up);
        }
    }

    fn note_tapped(&mut self, code: u32) {
        if !self.tapped.contains(&code) {
            self.tapped.push(code);
        }
    }

    /// Ends capture `id`, putting the pointer at `pos` (or where it was).
    fn release_capture(&mut self, id: u32, pos: Option<Pos>) {
        let _ = self.capture.send(CaptureCmd::Release { id, pos });
        // Keys go up unseen while not captured (except where the backend
        // reports modifiers at each activation).
        self.mods = Mods::default();
    }

    /// The pointer crossed our `side` edge at `pos`: it enters the peer
    /// there at the matching point of the facing edge.
    fn enter_peer(&mut self, activation: u32, side: Side, pos: Pos) {
        self.end_visit();
        let target = self.peer_at(side, pos).and_then(|fp| Some((fp, self.seam(fp)?)));
        let Some((fp, seam)) = target else {
            // Slid along the edge (held by an edge rule) to where nothing
            // faces it: the pointer stays here.
            self.release_capture(activation, Some(pos));
            return;
        };
        let epoch = self.epoch.wrapping_add(1);
        let Some(cursor) = RemoteCursor::cross(seam, pos, epoch) else {
            self.release_capture(activation, Some(pos));
            return;
        };
        self.epoch = epoch;
        self.begin_visit(activation, fp, cursor);
    }

    /// `one-mk switch`: the pointer goes to the centre of `fp`'s main
    /// screen, without crossing an edge. Its edge facing us still leads back.
    fn enter_peer_at_center(&mut self, activation: u32, fp: Fingerprint) {
        self.end_visit();
        let Some(seam) = self.seam(fp) else {
            self.release_capture(activation, None);
            return;
        };
        self.epoch = self.epoch.wrapping_add(1);
        let center = seam.remote().primary().center();
        let cursor = RemoteCursor::enter_at(seam, center, self.epoch);
        self.begin_visit(activation, fp, cursor);
    }

    /// Starts a visit to `fp` (whose `Enter` goes out now) on capture
    /// `activation`.
    fn begin_visit(&mut self, activation: u32, fp: Fingerprint, mut cursor: RemoteCursor) {
        let peer = &self.peers[&fp];
        let setting = self.trust.get(&fp).map_or(Swap::Auto, |d| d.swap_ctrl_cmd);
        let swap = swap_applies(setting, os::local(), peer.os);
        let tune = self.tune_for(fp, swap, &[]);
        let epoch = cursor.epoch();
        peer.ctl.send(Control::Enter { epoch, pos: cursor.pos() });
        if wants_relative(self.settings.gaming, peer.caps, false) && cursor.set_relative(true) {
            peer.ctl.send(Control::PointerMode { epoch, relative: true });
        }
        tracing::debug!(peer = %peer.name, side = %peer.side, relative = cursor.is_relative(), "pointer moved to peer");
        let now = Instant::now();
        self.remote = Some(Remote {
            activation,
            peer: fp,
            side: peer.side,
            cursor,
            held: HeldKeys::new(),
            swap,
            acked: false,
            queued: Vec::new(),
            entered_at: now,
            last_send: now,
            resend_at: None,
            resends: 0,
            peer_fullscreen: false,
            focus_heard: now,
            watching_fullscreen: false,
            watching_app: false,
            peer_apps: Vec::new(),
            apps_heard: false,
            inhibit_sent: false,
            tune,
        });
        self.apply_pointer_mode(); // gaming `auto` starts watching
        self.update_sides();
    }

    /// Switches the visit in progress between absolute and relative motion
    /// when gaming mode, the peer's abilities or its fullscreen state say so.
    fn apply_pointer_mode(&mut self) {
        let Some(fp) = self.remote.as_ref().map(|r| r.peer) else { return };
        let app = self.peers.get(&fp).is_some_and(|p| p.caps & caps::FOCUS_APP != 0) && !self.app_rules_for(fp).is_empty();
        let Some(r) = self.remote.as_mut() else { return };
        let Some(p) = self.peers.get(&r.peer) else { return };
        // Watch the peer's focus only for gaming `auto` and per-app rules,
        // and stop it from looking at every click once neither wants it.
        let fullscreen = self.settings.gaming == Gaming::Auto && p.caps & caps::FOCUS != 0;
        if (fullscreen, app) != (r.watching_fullscreen, r.watching_app) {
            r.watching_fullscreen = fullscreen;
            r.watching_app = app;
            r.peer_fullscreen &= fullscreen;
            // Not fullscreen as well only if it understands that.
            let only_app = app && !fullscreen;
            p.ctl.send(Control::WatchFocus { epoch: r.cursor.epoch(), on: fullscreen || app, app, fullscreen: !only_app });
            if !app && !std::mem::take(&mut r.peer_apps).is_empty() {
                r.apps_heard = false;
                self.retune();
                return self.apply_pointer_mode();
            }
        }
        let want = wants_relative(self.settings.gaming, p.caps, r.peer_fullscreen);
        if r.cursor.set_relative(want) {
            p.ctl.send(Control::PointerMode { epoch: r.cursor.epoch(), relative: want });
            tracing::debug!(peer = %p.name, relative = want, "pointer mode changed");
            if r.acked {
                r.send_motion(&self.peers, r.cursor.current());
            }
        }
    }

    /// Ends the visit to a peer (which releases anything still held there),
    /// without touching capture.
    fn end_visit(&mut self) -> Option<Remote> {
        self.release_tuned();
        let r = self.remote.take()?;
        if let Some(p) = self.peers.get(&r.peer) {
            p.ctl.send(Control::Leave);
        }
        Some(r)
    }

    /// Returns the pointer to this machine without crossing an edge, at the
    /// point of our edge facing where it was (see `RemoteCursor::home_pos`).
    fn go_home(&mut self) {
        let Some(pos) = self.remote.as_ref().map(|r| r.cursor.home_pos()) else { return };
        self.go_home_at(Some(pos));
    }

    /// Returns the pointer to this machine at `pos`.
    fn go_home_at(&mut self, pos: Option<Pos>) {
        let Some(r) = self.end_visit() else { return };
        self.release_capture(r.activation, pos);
        self.update_sides();
        tracing::debug!("pointer is home");
    }

    /// Like `go_home`, if the visit to `fp` is `epoch`.
    fn go_home_from(&mut self, fp: Fingerprint, epoch: u16, why: &str) {
        if !self.remote.as_ref().is_some_and(|r| r.peer == fp && r.cursor.epoch() == epoch) {
            return;
        }
        let name = self.peers.get(&fp).map_or("", |p| &p.name);
        tracing::info!(peer = %name, "{why}; pointer is home");
        self.go_home();
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
        // A held approach has a deadline only while a dwell runs, and a
        // switch only until capture starts: otherwise nothing ticks.
        let waits = self.approach.as_ref().and_then(|(_, a)| a.deadline()).map(Instant::from_std);
        let waits = waits.into_iter().chain(self.switching.map(|(_, until)| until)).min();
        // A tap/hold key waiting to be decided.
        let tap = self.remote.as_ref().and_then(|r| r.tune.deadline()).map(Instant::from_std);
        let waits = waits.into_iter().chain(tap).min();
        let lease = self.owner.as_ref().map(|o| {
            let lease = (o.last_rx + LEASE).min(o.last_input + IDLE);
            o.focus_check.map_or(lease, |c| c.min(lease))
        });
        let lease = lease.into_iter().chain(waits).min();
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
        if let Some((fp, until)) = self.switching
            && now >= until
        {
            self.switching = None;
            let name = self.peers.get(&fp).map_or("", |p| &p.name);
            tracing::warn!(peer = %name, "could not switch: this computer's input capture did not start");
            self.update_sides();
        }
        if let (Some((_, a)), Some(local)) = (self.approach.as_mut(), self.local.as_ref())
            && a.deadline().is_some_and(|d| now.into_std() >= d)
        {
            let next = a.timer(local, now.into_std());
            self.on_approach(next, None);
        }
        if self.owner.as_ref().is_some_and(|o| now >= o.last_rx + LEASE) {
            tracing::warn!("the controlling computer went silent; releasing held keys");
            self.end_owner(true);
        } else if self.owner.as_ref().is_some_and(|o| now >= o.last_input + IDLE) {
            tracing::info!("no input from the controlling computer for {} s; it lets go", IDLE.as_secs());
            self.end_owner(true);
        }
        self.tune_timer(now.into_std());
        if self.owner.as_ref().and_then(|o| o.focus_check).is_some_and(|t| now >= t) {
            self.report_focus();
        }
        if let Some(r) = self.remote.as_mut()
            && r.peer_fullscreen
            && self.peers.get(&r.peer).is_some_and(|p| p.caps & caps::FOCUS_BEAT != 0)
            && now >= r.focus_heard + FOCUS_SILENCE
        {
            // It repeats "fullscreen" while that lasts: silence means it
            // can't tell any more, or is stuck. Relative motion has no
            // edge leading home, so it ends.
            tracing::warn!("the other computer stopped saying a fullscreen window has its focus; absolute motion again");
            r.peer_fullscreen = false;
            self.apply_pointer_mode();
        }
        let Some(r) = self.remote.as_mut() else { return };
        if !r.acked {
            if now >= r.entered_at + ENTER_TIMEOUT {
                let name = self.peers.get(&r.peer).map_or("", |p| &p.name);
                tracing::warn!(peer = %name, "peer did not accept the pointer; pointer is home");
                self.go_home();
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

    /// Tells the owner whether a fullscreen window has the focus here (for
    /// its gaming mode `auto`), if it asked. Looked at when it starts
    /// watching, and `FOCUS_SETTLE` after input from it that may move the
    /// focus (a click, a window-switching chord: see `may_move_focus`),
    /// once the system has acted on it; said when it changed. While
    /// fullscreen, it is looked at and said again every `FOCUS_REPEAT`:
    /// the focus can also move with no input (a game minimizing itself),
    /// and the owner leaves relative mode if it hears nothing. When this
    /// OS can't tell (a Wayland-native window), "fullscreen" is taken back
    /// if it was said, or the owner would stay in relative mode, where no
    /// edge leads home; otherwise nothing is said.
    ///
    /// The app's names go along only if the owner asked for them (its
    /// per-app input rules), only to it, and are said again whenever they
    /// change, at the same moments: no timer looks for them. When this OS
    /// can't tell (again a Wayland-native window), the app is unknown,
    /// which the owner hears if it had been told one.
    fn report_focus(&mut self) {
        let Some(o) = self.owner.as_mut() else { return };
        o.focus_check = None;
        if !o.watch_focus {
            return;
        }
        let probe = self.focus.focus();
        let fullscreen = o.watch_fullscreen && probe.as_ref().is_some_and(|f| f.fullscreen);
        let apps = match &probe {
            Some(f) if o.watch_app => one_mk_proto::appmatch::clean_names(&f.apps),
            _ => Vec::new(),
        };
        let state = (fullscreen, apps);
        // Nothing said yet and nothing known: nothing to say.
        let changed = o.focus_sent.as_ref().map_or(probe.is_some(), |sent| *sent != state);
        if changed {
            o.focus_sent = Some(state.clone());
        }
        if fullscreen {
            o.focus_check = Some(Instant::now() + FOCUS_REPEAT);
        }
        if !changed && !fullscreen {
            return;
        }
        if let Some(p) = self.peers.get(&o.peer).filter(|p| p.id == o.link) {
            let (fullscreen, apps) = state;
            if changed && o.watch_app {
                tracing::debug!(?apps, "telling the controlling computer which app has the focus");
            }
            p.ctl.send(Control::Focus { epoch: o.epoch, fullscreen, apps });
        }
    }

    /// Has the focus probe say when another window comes to the front while
    /// a controller watches the focus here, and stop when it doesn't.
    fn sync_focus_watch(&mut self) {
        let want = self.owner.as_ref().is_some_and(|o| o.watch_focus);
        if want == std::mem::replace(&mut self.focus_watched, want) {
            return;
        }
        let tx = self.tx.clone();
        let notify: Box<dyn Fn() + Send> = Box::new(move || {
            let _ = tx.send(AppEvent::FocusMoved);
        });
        self.focus.watch(want.then_some(notify));
    }

    /// Looks at the focus `FOCUS_SETTLE` from now (unless sooner anyway).
    fn check_focus_soon(&mut self) {
        if let Some(o) = self.owner.as_mut().filter(|o| o.watch_focus) {
            let at = Instant::now() + FOCUS_SETTLE;
            o.focus_check = Some(o.focus_check.map_or(at, |c| c.min(at)));
        }
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
                        self.reload_trust();
                    }
                    if !self.trust.is_trusted(&f.fingerprint) {
                        return;
                    }
                }
                tracing::debug!(name = %sanitize_name(&f.name), addrs = ?f.addrs, "discovered");
                self.discovered.insert(f.fingerprint, f.addrs);
                self.dial_backoff = DIAL_INTERVAL;
                self.dial_missing(screens);
            }
            AppEvent::DialDone(addr) => {
                self.dialing.remove(&addr);
            }
            AppEvent::Network(net) => self.on_network(&net),
            AppEvent::FocusMoved => self.check_focus_soon(),
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
                self.go_home();
            }
            if self.owner.as_ref().is_some_and(|o| o.peer == fp) {
                self.end_owner(false);
            }
        }
        self.reload_trust();
        let (side, align) = self.trust.get(&fp).map_or((Side::Right, Align::Stretch), |d| (d.side(), d.align()));
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
        ctl.send(Control::Caps(caps::LOCAL));
        // Our monitors may have changed during the handshake.
        if let Some(local) = &self.local
            && local.screens != sent
        {
            ctl.send(Control::Screens(local.screens.clone()));
        }
        if let Some(old) = self.peers.insert(fp, Connected { id, name, os, rank, side, align, conn, ctl, desktop, caps: 0 }) {
            old.conn.close(2u32.into(), b"duplicate");
        }
        self.send_layout(fp);
        self.update_sides();
        if !self.any_missing() {
            self.dial_at = None; // nothing left to dial
        }
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
            LinkEvent::MotionBy { epoch, seq, sum } => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id) {
                    o.motion_by(epoch, seq, sum, &self.inject);
                }
            }
            LinkEvent::Control(msg) => self.on_control(fp, id, msg),
            LinkEvent::Closed => {
                let peer = self.peers.remove(&fp).expect("present");
                if self.owner.as_ref().is_some_and(|o| o.peer == fp) {
                    self.end_owner(false);
                }
                println!("{} disconnected; reconnecting automatically.", peer.name);
                self.redial_soon();
                if let Some(clip) = &self.services.clip {
                    clip.forget(fp);
                }
                if let Some(r) = self.remote.take_if(|r| r.peer == fp) {
                    self.release_capture(r.activation, Some(r.cursor.home_pos()));
                }
                if self.switching.is_some_and(|(s, _)| s == fp) {
                    self.switching = None;
                }
                self.update_sides();
            }
        }
    }

    fn on_control(&mut self, fp: Fingerprint, id: usize, msg: Control) {
        match msg {
            Control::Hello { .. } => {}
            Control::Screens(screens) => self.on_peer_screens(fp, screens),
            Control::Layout { side_of_you, version, align } => self.on_peer_layout(fp, side_of_you, version, align),
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
            Control::Caps(bits) => {
                if let Some(p) = self.peers.get_mut(&fp) {
                    p.caps = bits;
                }
                self.apply_pointer_mode();
            }
            Control::PointerMode { epoch, relative } => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id && o.epoch == epoch) {
                    o.relative = relative;
                }
            }
            Control::Focus { epoch, fullscreen, apps } => {
                if let Some(r) = self.remote.as_mut().filter(|r| r.peer == fp && r.cursor.epoch() == epoch) {
                    r.peer_fullscreen = fullscreen && r.watching_fullscreen;
                    r.focus_heard = Instant::now();
                    if r.watching_app {
                        r.apps_heard = true;
                        if apps != r.peer_apps {
                            tracing::debug!(?apps, "the app with the focus on the other computer changed");
                            r.peer_apps = apps;
                            self.retune(); // per-app rules on or off
                        }
                    }
                    self.apply_pointer_mode();
                }
            }
            Control::Lock { id: lock, all } => self.on_peer_lock(fp, lock, all),
            Control::Sleep { id: sleep } => self.on_peer_sleep(fp, sleep),
            Control::Inhibit { epoch, on } => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id && o.epoch == epoch) {
                    o.inhibit = on;
                }
            }
            Control::WatchFocus { epoch, on, app, fullscreen } => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id && o.epoch == epoch) {
                    o.watch_focus = on;
                    o.watch_fullscreen = on && fullscreen;
                    o.watch_app = on && app;
                    o.focus_sent = None;
                    self.report_focus();
                }
            }
            input => {
                if let Some(o) = self.owner.as_mut().filter(|o| o.peer == fp && o.link == id) {
                    let moves = match input {
                        Control::Button { down, .. } => down,
                        Control::Key { code, down } => may_move_focus(code, down, &o.held),
                        _ => false,
                    };
                    o.input(input, &self.inject);
                    if moves {
                        self.check_focus_soon();
                    }
                }
            }
        }
    }

    fn reload_trust(&mut self) {
        if let Ok(t) = TrustStore::load(&self.dir) {
            self.trust = t;
            let computers = crate::hotkeys::numbered_from(self.node.identity(), &self.trust).len();
            self.shortcuts.set_computers(computers);
        }
    }

    /// Tells a peer where we think it is, from its point of view. It adopts
    /// that only if our version is newer than its own.
    /// Our alignment goes out mirrored: the peer's edge starting 300 px
    /// after ours is ours starting 300 px before its.
    fn send_layout(&self, fp: Fingerprint) {
        if let (Some(p), Some(d)) = (self.peers.get(&fp), self.trust.get(&fp)) {
            #[cfg(test)]
            tests::layouts_sent_by(&self.dir).fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let align = Some(d.align().mirror());
            p.ctl.send(Control::Layout { side_of_you: d.side().opposite(), version: d.layout_version, align });
        }
    }

    /// The peer says it sits on `side_of_you` of us, lined up as `align`
    /// says (`None` from a peer that predates alignment). A newer version
    /// wins; equal versions with different layouts are a tie, broken by
    /// [`resolve_layout`].
    fn on_peer_layout(&mut self, fp: Fingerprint, side_of_you: Side, version: u64, align: Option<Align>) {
        self.reload_trust();
        let our_fp = self.node.identity().fingerprint;
        let Some(dev) = self.trust.get_mut(&fp) else { return };
        // An older peer only knows sides. An alignment chosen for another
        // side means nothing on the new one, so that starts stretched; on
        // the same side ours stands.
        let align = align.unwrap_or(if dev.side() == side_of_you { dev.align() } else { Align::Stretch });
        let same = (dev.side(), dev.align()) == (side_of_you, align);
        let verdict = resolve_layout(dev.layout_version, version, same, &our_fp, &fp);
        match verdict {
            LayoutVerdict::Keep => return,
            LayoutVerdict::Adopt => {
                dev.side = side_of_you.to_string();
                dev.set_align(align);
                dev.layout_version = version;
            }
            // A newer version than the tie, which the peer (new or old)
            // takes over. It stays quiet about it, so that is the one
            // message the tie costs.
            LayoutVerdict::Win => dev.layout_version = version.saturating_add(1),
        }
        if let Err(e) = self.trust.save() {
            tracing::warn!("could not save the new layout: {e:#}");
        }
        if verdict == LayoutVerdict::Win {
            self.send_layout(fp);
        }
        self.sync_sides();
    }

    /// Applies the sides in the trust store to connected peers.
    fn sync_sides(&mut self) {
        let mut moved = Vec::new();
        for (fp, p) in &mut self.peers {
            let Some(d) = self.trust.get(fp) else { continue };
            let (side, align) = (d.side(), d.align());
            if (side, align) == (p.side, p.align) {
                continue;
            }
            (p.side, p.align) = (side, align);
            moved.push(*fp);
            println!("{} is now {}.", p.name, placement(side, align));
        }
        if self.remote.as_ref().is_some_and(|r| moved.contains(&r.peer)) {
            self.go_home();
        }
        self.update_sides();
    }

    /// Keeps the screen awake while a peer that asked controls this
    /// computer, and asks the peer we control to do so for us (`[power]`).
    fn sync_power(&mut self) {
        let block = self.settings.power.block_screensaver_on_peers;
        if let Some(r) = self.remote.as_mut() {
            let caps = self.peers.get(&r.peer).map_or(0, |p| p.caps);
            if let Some(on) = crate::power::inhibit_request(block, r.acked, caps, r.inhibit_sent) {
                r.inhibit_sent = on;
                if let Some(p) = self.peers.get(&r.peer) {
                    p.ctl.send(Control::Inhibit { epoch: r.cursor.epoch(), on });
                }
            }
        }
        let want = self.owner.as_ref().is_some_and(|o| o.inhibit);
        let inhibitor = &mut self.inhibitor;
        self.awake.set(want, || inhibitor.inhibit("A computer is controlling this one (one-mk)"));
    }

    /// The OS says this display went to sleep (or the computer is
    /// suspending), or woke.
    fn on_power(&mut self, ev: PowerEvent) {
        match ev {
            PowerEvent::DisplayAsleep(true) if self.visiting() => {
                tracing::debug!("display asleep during a visit; not asking the others to sleep (sleep sync)");
            }
            PowerEvent::DisplayAsleep(true) => {
                let fresh = link::random_u64();
                let sync = self.settings.power.sleep_sync;
                if let Some(id) = self.sleep_sync.on_local_asleep(sync, fresh, std::time::Instant::now()) {
                    tracing::debug!("display asleep; asking the connected computers to sleep too (sleep sync)");
                    self.send_sleep(id, None);
                }
            }
            PowerEvent::DisplayAsleep(false) => self.sleep_sync.on_local_awake(),
        }
    }

    /// Peer `fp` asks us to sleep (its sleep sync): followed only with sleep
    /// sync on here, and not during a visit (see `visiting`).
    fn on_peer_sleep(&mut self, fp: Fingerprint, id: u64) {
        if self.visiting() {
            tracing::debug!("a peer's display went to sleep during a visit; staying awake (sleep sync)");
            return;
        }
        match self.sleep_sync.on_remote(id, self.settings.power.sleep_sync, std::time::Instant::now()) {
            OnRemoteSleep::Ignore => {}
            OnRemoteSleep::Sleep { sleep_here } => {
                self.send_sleep(id, Some(fp));
                if sleep_here {
                    let name = self.peers.get(&fp).map_or("", |p| &p.name);
                    println!("{name}'s display went to sleep; putting this one to sleep too (sleep sync).");
                    if let Err(e) = self.sleeper.sleep_display() {
                        tracing::warn!("{e:#}");
                    }
                }
            }
        }
    }

    /// The pointer is on a peer, or a peer controls this computer: someone
    /// is using these computers, whatever their idle timers say (a
    /// controlled display blanks because injected input doesn't count, the
    /// controller's because its input goes elsewhere). Sleep sync waits.
    fn visiting(&self) -> bool {
        self.remote.is_some() || self.owner.is_some()
    }

    /// `Sleep { id }` to every peer that understands it, except `except`.
    fn send_sleep(&self, id: u64, except: Option<Fingerprint>) {
        for (fp, p) in &self.peers {
            if Some(*fp) != except && p.caps & caps::POWER != 0 {
                p.ctl.send(Control::Sleep { id });
            }
        }
    }

    /// Lets `[hooks]` see what changed: the pointer left or came back, a
    /// peer took control or let go, a peer connected or disconnected.
    fn sync_hooks(&mut self) {
        let hooks = &self.settings.hooks;
        let away = self.remote.as_ref().filter(|r| r.acked).map(|r| r.peer);
        let owner = self.owner.as_ref().map(|o| o.peer);
        let peers = self.peers.iter().map(|(fp, p)| (*fp, p.name.as_str(), p.side));
        self.hooks.sync(hooks, away, owner, peers);
    }

    /// Starts watching for network changes if a layout profile switches by
    /// itself (once; it then runs for good, idle until the network changes).
    fn watch_network_if_wanted(&mut self) {
        if self.watching_network || !Profiles::load(&self.dir).is_ok_and(|p| p.any_auto()) {
            return;
        }
        self.watching_network = true;
        let tx = self.tx.clone();
        crate::netwatch::system().start(Box::new(move |net| {
            let _ = tx.send(AppEvent::Network(net));
        }));
    }

    /// The network changed: put the first automatic profile that matches
    /// in place, unless it already was for this network, so `one-mk layout`
    /// tweaks stand until the network really changes. The first report
    /// after starting is the network we start on: if its profile is the
    /// one in use, tweaks made before the restart stand too.
    fn on_network(&mut self, net: &NetworkId) {
        let first = !std::mem::replace(&mut self.network_seen, true);
        let all = match Profiles::load(&self.dir) {
            Ok(all) => all,
            Err(e) => return tracing::warn!("{e:#}"),
        };
        let Some(p) = all.matching(net) else {
            self.auto_profile = None;
            return;
        };
        if self.auto_profile.replace(p.name.clone()).as_deref() == Some(p.name.as_str())
            || (first && all.active.as_deref() == Some(p.name.as_str()))
        {
            return;
        }
        match crate::profiles::use_profile(&self.dir, &p.name) {
            Ok((name, moved)) => {
                println!("This network matches layout profile {name:?}; using it.");
                if !moved.is_empty() {
                    self.reload_trust();
                    self.sync_sides();
                    let fps: Vec<_> = self.peers.keys().copied().collect();
                    fps.into_iter().for_each(|fp| self.send_layout(fp));
                }
            }
            Err(e) => tracing::warn!("could not switch layout profiles: {e:#}"),
        }
    }

    /// Reads `settings.toml` and applies all of it: clipboard and
    /// notifications, edge rules, gaming mode and lock sync. A file that
    /// can't be read changes nothing.
    fn reload_settings(&mut self) -> Result<()> {
        self.settings = Settings::load(&self.dir)?;
        crate::hooks::warn_if_unsafe(&self.dir, &self.settings.hooks);
        crate::clipctl::apply(&self.settings, &self.services);
        self.apply_pointer_mode();
        self.sync_sides();
        self.shortcuts.reload(&self.settings.hotkeys);
        let paired: Vec<_> = self.trust.devices().iter().map(|d| (d.name.clone(), d.fingerprint.clone(), d.os)).collect();
        for p in self.settings.input.check(&paired) {
            tracing::warn!("input tuning in settings.toml: {p}");
        }
        self.retune();
        Ok(())
    }

    fn on_request(&mut self, req: Request, reply: tokio::sync::oneshot::Sender<String>) {
        self.reload_trust();
        let text = match req {
            Request::Status => self.status().encode(),
            Request::Invitation => return crate::invite::query(&self.dir, reply),
            Request::Reload => {
                // `devices.toml` was reloaded above; layouts go out whatever
                // `settings.toml` says. A settings file that can't be read
                // keeps the settings in force (not the defaults).
                if let Err(e) = self.reload_settings() {
                    tracing::warn!("keeping the current settings: {e:#}");
                }
                let fps: Vec<_> = self.peers.keys().copied().collect();
                fps.into_iter().for_each(|fp| self.send_layout(fp));
                self.watch_network_if_wanted();
                self.redial_soon(); // a computer may have been paired
                "ok\n".to_owned()
            }
            Request::Settings => match self.reload_settings() {
                Ok(()) => "ok\n".to_owned(),
                Err(e) => format!("error {e:#}\n"),
            },
            Request::Switch(to) => self.switch(to),
            Request::LockPointer(t) => {
                let was = self.locked;
                self.locked = t.apply(was);
                if self.locked != was {
                    if self.locked {
                        println!("The pointer is locked to this computer.");
                        if let Some((id, a)) = self.approach.take() {
                            self.release_capture(id, Some(a.pos()));
                        }
                    } else {
                        println!("The pointer can cross edges again.");
                    }
                }
                self.update_sides();
                format!("ok\n{}\n", if self.locked { "on" } else { "off" })
            }
            Request::LockAll => {
                let id = self.lock_sync.lock_all(link::random_u64(), std::time::Instant::now());
                for p in self.peers.values() {
                    p.ctl.send(Control::Lock { id, all: true });
                }
                let n = self.peers.len();
                println!("Locking this computer{}.", if n > 0 { " and the connected ones" } else { "" });
                // The others are asked all the same; the reply says why
                // this one didn't lock.
                match self.lock_screen() {
                    Ok(()) => format!("ok\n{n}\n"),
                    Err(e) => format!("ok\n{n}\n{}\n", format!("{e:#}").replace('\n', " ")),
                }
            }
            req @ (Request::ClipSend(_) | Request::ClipGet(_) | Request::Transfer { .. }) => {
                let online = self.peers.iter().map(|(fp, p)| crate::clipctl::Online { fp: *fp, name: p.name.clone(), addr: canonical(p.conn.remote_address()) });
                return crate::clipctl::handle(&self.dir, &self.services, online.collect(), &self.trust, req, reply);
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
        };
        let _ = reply.send(text);
    }

    fn status(&self) -> Status {
        let id = self.node.identity();
        let profiles = Profiles::load(&self.dir).unwrap_or_default();
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
            })
        });
        let (hotkeys, hotkeys_detail) = self.shortcuts.status();
        let places = self.trust.devices().iter().filter_map(|d| {
            let live = self.peers.get(&d.fingerprint()?);
            Some(Place {
                id: d.short_id(),
                align: live.map_or_else(|| d.align(), |p| p.align),
                screens: live.and_then(|p| p.desktop.as_ref()).map_or_else(Vec::new, |d| d.screens.clone()),
            })
        });
        Status {
            name: sanitize_name(&id.name),
            id: id.fingerprint.short(),
            port: self.node.local_addr().map_or(0, |a| a.port()),
            backend: self.backend_name.into(),
            controlling: self.remote.as_ref().map(|r| name_of(&r.peer)),
            controlled_by: self.owner.as_ref().map(|o| name_of(&o.peer)),
            peers: peers.collect(),
            pointer_locked: self.locked,
            screen_locked: self.lock_sync.is_locked(),
            gaming: self.settings.gaming.to_string(),
            relative: self.remote.as_ref().is_some_and(|r| r.cursor.is_relative()),
            lock_sync: self.settings.lock_sync,
            edges: self.settings.edge.describe(),
            transfers: self.services.inbox.transfers().snapshot(),
            hotkeys,
            hotkeys_detail,
            layout: Box::new(LayoutStatus {
                screens: self.local.as_ref().map_or_else(Vec::new, |d| d.screens.clone()),
                places: places.collect(),
                profiles: profiles.names(),
                profile: profiles.active,
                warnings: self.conflicts.clone(),
            }),
            input: self.input_problems.values().cloned().collect(),
            tuning: self.app_rule_notes(),
        }
    }

    // ------------------------------------------------------------- switching

    /// `one-mk switch` (and, later, its keyboard shortcut). The reply's
    /// first line is `ok`, `already`, `here` or `error <why>`.
    fn switch(&mut self, to: SwitchTo) -> String {
        match to {
            SwitchTo::Here => self.switch_here(),
            SwitchTo::Next => {
                let mut online: Vec<_> =
                    self.peers.iter().filter(|(_, p)| p.desktop.is_some()).map(|(fp, p)| (p.name.clone(), *fp)).collect();
                online.sort();
                let order: Vec<Fingerprint> = online.into_iter().map(|(_, fp)| fp).collect();
                let current = self.switching.map(|(fp, _)| fp).or(self.remote.as_ref().map(|r| r.peer));
                match cycle_next(current, &order) {
                    Some(fp) => self.switch_to(fp),
                    None => self.switch_here(),
                }
            }
            SwitchTo::Device(name) => {
                let Some(fp) = self.trust.find(&name).and_then(|d| d.fingerprint()) else {
                    return format!("error no paired computer called {:?}; see `one-mk devices`\n", sanitize_name(&name));
                };
                match self.peers.get(&fp) {
                    Some(p) if p.desktop.is_some() => self.switch_to(fp),
                    Some(p) => format!("error {} has not said what its screens are\n", p.name),
                    None => {
                        let name = self.trust.get(&fp).map_or_else(|| sanitize_name(&name), |d| sanitize_name(&d.name));
                        format!("error {name} is not connected\n")
                    }
                }
            }
        }
    }

    /// The mouse and keyboard come back to this computer, from a peer we
    /// control, and from a peer controlling us (like `one-mk takeback`).
    fn switch_here(&mut self) -> String {
        self.switching = None;
        if let Some((id, a)) = self.approach.take() {
            self.release_capture(id, Some(a.pos()));
        }
        let mut what = Vec::new();
        if let Some(r) = self.remote.as_ref() {
            let name = self.peers.get(&r.peer).map_or_else(String::new, |p| p.name.clone());
            tracing::info!(peer = %name, "`one-mk switch here`: pointer is home");
            what.push(format!("left {name}"));
            let center = self.local.as_ref().map(|d| d.primary().center());
            self.go_home_at(center);
        }
        if let Some(fp) = self.owner.as_ref().map(|o| o.peer) {
            let name = self.peers.get(&fp).map_or_else(String::new, |p| p.name.clone());
            tracing::info!(peer = %name, "control taken back with `one-mk switch here`");
            what.push(format!("took control back from {name}"));
            self.end_owner(true);
        }
        self.update_sides();
        format!("here\n{}\n", what.join(", "))
    }

    fn switch_to(&mut self, fp: Fingerprint) -> String {
        let (name, side) = (self.peers[&fp].name.clone(), self.peers[&fp].side);
        if self.seam(fp).is_some_and(|s| !s.overlaps()) {
            // Allowed (it's the only way there), and any part of the edge
            // facing us leads back (see `Seam::exit`).
            tracing::warn!("{}", no_overlap(&name, side));
        }
        if self.remote.as_ref().is_some_and(|r| r.peer == fp) {
            return format!("already\n{name}\n");
        }
        // The person here wants this computer's mouse and keyboard: whoever
        // controls this computer lets go first.
        self.end_owner(true);
        let captured = self.remote.as_ref().map(|r| r.activation).or(self.approach.as_ref().map(|(id, _)| *id));
        self.approach = None;
        match captured {
            // Already capturing: the visit moves to the new peer.
            Some(activation) => self.enter_peer_at_center(activation, fp),
            None => {
                self.switching = Some((fp, Instant::now() + SWITCH_TIMEOUT));
                self.update_sides(); // arms `side` even while the pointer is locked
                let _ = self.capture.send(CaptureCmd::Activate { side });
            }
        }
        format!("ok\n{name}\n")
    }

    // ------------------------------------------------------------ screen lock

    /// Locks this computer's screen, letting go of everything first.
    fn lock_screen(&mut self) -> Result<()> {
        self.let_go_for_lock();
        let locked = self.locker.lock();
        if let Err(e) = &locked {
            tracing::warn!("could not lock the screen: {e:#}");
        }
        locked
    }

    /// Nothing may stay held across a lock: the lock screen would swallow
    /// the key-ups, leaving keys stuck after unlocking.
    fn let_go_for_lock(&mut self) {
        self.go_home();
        if let Some((id, a)) = self.approach.take() {
            self.release_capture(id, Some(a.pos()));
        }
        if let Some(o) = self.owner.as_ref() {
            for code in o.mods_pressed.keys() {
                self.unlock_release.update(code, true);
            }
            self.end_owner(true);
        }
    }

    /// The OS says this screen locked or unlocked.
    fn on_screen_locked(&mut self, locked: bool) {
        if !locked {
            tracing::debug!("screen unlocked");
            self.lock_sync.on_local_unlocked();
            // Releases sent while locked may have gone to the lock screen
            // instead (a Win key stuck after unlocking, typically): send
            // them again, now that they reach the desktop.
            for code in self.unlock_release.keys() {
                let _ = self.inject.send(Inject::Key { code, down: false });
            }
            self.unlock_release = Mods::default();
            return;
        }
        tracing::debug!("screen locked");
        let fresh = link::random_u64();
        if let Some(id) = self.lock_sync.on_local_locked(self.settings.lock_sync, fresh, std::time::Instant::now()) {
            if !self.peers.is_empty() {
                println!("This computer locked; locking the connected ones too (lock sync).");
            }
            for p in self.peers.values() {
                p.ctl.send(Control::Lock { id, all: false });
            }
        }
        self.let_go_for_lock();
    }

    /// Peer `fp` asks us to lock: its `lock-all` (`all`), obeyed always,
    /// or its lock sync, followed only with lock sync on here.
    fn on_peer_lock(&mut self, fp: Fingerprint, id: u64, all: bool) {
        match self.lock_sync.on_remote(id, all, self.settings.lock_sync, std::time::Instant::now()) {
            OnRemote::Ignore => {}
            OnRemote::Lock { lock_here, forward } => {
                if forward {
                    for (other, p) in &self.peers {
                        if *other != fp {
                            p.ctl.send(Control::Lock { id, all });
                        }
                    }
                }
                if lock_here {
                    let name = self.peers.get(&fp).map_or("", |p| &p.name);
                    match all {
                        true => println!("{name} asked to lock every computer (lock-all); locking this one."),
                        false => println!("{name} locked its screen; locking this one too (lock sync)."),
                    }
                    let _ = self.lock_screen(); // said in the log
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
                    self.go_home();
                }
            }
        }
        if let Some(p) = self.peers.get_mut(&fp) {
            p.desktop = desktop;
        }
        self.update_sides();
    }

    /// The edge between our desktop and peer `fp`'s, if both are known.
    fn seam(&self, fp: Fingerprint) -> Option<Seam> {
        let p = self.peers.get(&fp)?;
        Some(Seam::new(self.local.clone()?, p.desktop.clone()?, p.side, p.align))
    }

    /// The peer the pointer goes to through `side` at `pos`: one whose edge
    /// faces that point. Several can sit on one side, lined up next to each
    /// other ([`Align`]); where two overlap (a layout mistake), the one with
    /// the lowest fingerprint, so the choice doesn't depend on connection
    /// order.
    fn peer_at(&self, side: Side, pos: Pos) -> Option<Fingerprint> {
        self.peers
            .iter()
            .filter(|(_, p)| p.side == side && p.desktop.is_some())
            .map(|(fp, _)| *fp)
            .filter(|fp| self.seam(*fp).is_some_and(|s| s.enter(pos).is_some()))
            .min()
    }

    fn update_sides(&mut self) {
        let mut sides = Vec::new();
        let mut conflicts = Vec::new();
        let switching = self.switching.and_then(|(fp, _)| self.peers.get(&fp).map(|p| p.side));
        for side in Side::ALL {
            let mut here: Vec<_> =
                self.peers.iter().filter(|(_, p)| p.side == side && p.desktop.is_some()).collect();
            here.sort_by_key(|(fp, _)| **fp);
            if here.is_empty() {
                continue;
            }
            // A switch about to start needs its edge: the Linux backend
            // pushes the pointer into it to start capturing.
            let spans = if switching == Some(side) { None } else { self.armed_spans(side, here.iter().map(|(fp, _)| **fp)) };
            if spans.as_ref().is_none_or(|s| !s.is_empty()) {
                sides.push(Armed { side, spans });
            }
            // Each one's stretch of our edge; without our monitors yet, all
            // of it.
            let shared = |fp: &Fingerprint| self.seam(*fp).map(|s| s.shared());
            for (i, (fp, other)) in here.iter().enumerate() {
                let theirs = shared(fp);
                if theirs.as_ref().is_some_and(|t| t.is_empty()) {
                    conflicts.push(no_overlap(&other.name, side));
                    continue;
                }
                let Some((_, used)) = here[..i].iter().find(|(f, _)| match (&theirs, shared(f)) {
                    (Some(a), Some(b)) => overlaps(a, &b),
                    _ => true,
                }) else {
                    continue;
                };
                conflicts.push(format!(
                    "{} and {} are both {} this computer and their edges overlap, so only {} is reached there; \
                     move the other with `one-mk layout \"{}\" <left|right|top|bottom> [--offset PX]`",
                    used.name, other.name, on_your(side), used.name, other.name
                ));
            }
        }
        if conflicts != self.conflicts {
            for c in &conflicts {
                tracing::warn!("{c}");
            }
            self.conflicts = conflicts;
        }
        if self.locked {
            // Only the edge of a visit in progress (or of a switch about to
            // start) stays armed, so the pointer can come back that way.
            let keep: Vec<Side> = (self.remote.as_ref().map(|r| r.side).into_iter())
                .chain(switching)
                .collect();
            sides.retain(|a| keep.contains(&a.side));
        }
        if sides != self.sides {
            self.sides = sides.clone();
            let _ = self.capture.send(CaptureCmd::SetSides(sides));
        }
    }

    /// Paired devices that are not connected.
    fn missing(&self) -> impl Iterator<Item = (Fingerprint, &one_mk_net::Device)> {
        let me = self.node.identity().fingerprint;
        self.trust
            .devices()
            .iter()
            .filter_map(|d| Some((d.fingerprint()?, d)))
            .filter(move |(fp, _)| *fp != me && !self.peers.contains_key(fp))
    }

    fn any_missing(&self) -> bool {
        self.missing().next().is_some()
    }

    /// Dials missing peers at the next turn of the loop, the wait after
    /// that starting short again (a peer just went away, or was paired).
    fn redial_soon(&mut self) {
        self.dial_backoff = DIAL_INTERVAL;
        self.dial_at = Some(Instant::now());
    }

    /// The parts of our `side` edge that lead to one of `peers` (there):
    /// with 1:1 alignments only where their edges face ours, so the rest
    /// is a wall the pointer never gets captured at. `None` for all of it:
    /// a stretched peer, monitors not known yet, or simply the whole edge.
    fn armed_spans(&self, side: Side, peers: impl Iterator<Item = Fingerprint>) -> Option<Vec<(f64, f64)>> {
        let mut spans = Vec::new();
        for fp in peers {
            let seam = self.seam(fp).filter(|s| s.align().is_exact())?;
            spans.extend(seam.shared());
        }
        let whole = self.local.as_ref()?.edge_spans(side).into_iter().map(|(a, b)| (f64::from(a), f64::from(b)));
        let merged = merge_spans(spans);
        if merged == merge_spans(whole.collect()) { None } else { Some(merged) }
    }

    /// Dials paired devices that are not connected, at every address we
    /// know, and schedules the next try while any is left to try (with
    /// back-off); otherwise no timer runs: discovery, a disconnection or
    /// a reload start dialing again.
    fn dial_missing(&mut self, screens: &watch::Receiver<Vec<Rect>>) {
        let mut addrs: Vec<SocketAddr> = Vec::new();
        let mut any_missing = false;
        for (fp, dev) in self.missing() {
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
        self.dial_at = (!addrs.is_empty()).then(|| Instant::now() + self.dial_backoff);
        self.dial_backoff = (self.dial_backoff * 2).min(DIAL_MAX);
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

/// `[start, end)` ranges, sorted, with those that overlap or touch joined.
fn merge_spans(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// The warning for a peer on our `side` whose facing edge doesn't line up
/// with ours anywhere (an offset past the end of the edge).
fn no_overlap(name: &str, side: Side) -> String {
    format!(
        "layout of {name} does not overlap yours: no part of its edge faces this computer's, so the pointer can't \
         cross to it (`one-mk switch` still goes there, and any part of that edge comes back); fix it with \
         `one-mk layout \"{name}\" {side} --offset 0`"
    )
}

/// Whether a key from the controller may move the focus, or take a window
/// in or out of fullscreen, so gaming mode `auto` must look again: a key
/// pressed with Alt, Ctrl or the logo key held (Alt+Tab, Alt+Enter,
/// Win+Up), Alt or the logo key let go (Alt+Tab switches then; the logo
/// key opens the start menu), F11 or Esc. `held` is what the controller
/// held before this key. Not plain typing, so a game's WASD costs nothing.
fn may_move_focus(code: u32, down: bool, held: &HeldKeys) -> bool {
    const ESC: u32 = 1;
    const F11: u32 = 87;
    const ALTS: [u32; 2] = [56, 100];
    const LOGO: [u32; 2] = [125, 126];
    if !down {
        return ALTS.contains(&code) || LOGO.contains(&code);
    }
    let chord = [CTRL, ALT, [LOGO[0], LOGO[1]]].iter().flatten().any(|&m| held.is_held(m));
    (chord && !Mods::is_modifier(code)) || code == F11 || code == ESC
}

/// Whether to send relative motion (gaming mode) to a peer that supports
/// `peer_caps` and reports `peer_fullscreen`.
fn wants_relative(mode: Gaming, peer_caps: u32, peer_fullscreen: bool) -> bool {
    peer_caps & caps::RELATIVE != 0
        && match mode {
            Gaming::Off => false,
            Gaming::On => true,
            Gaming::Auto => peer_fullscreen,
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
    use one_mk_ctl::client::Toggle;
    use one_mk_input::NullHandles;
    use one_mk_net::{Device, read_frame};
    use one_mk_proto::Focus;
    use one_mk_proto::PROTOCOL_VERSION;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// A port nothing listens on. Tests run in parallel and the OS may hand
    /// out a just-released port again, so none is given out twice.
    fn free_port() -> u16 {
        static GIVEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
        let mut given = GIVEN.lock().unwrap();
        loop {
            let port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            if !given.contains(&port) {
                given.push(port);
                return port;
            }
        }
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

    #[tokio::test]
    async fn status_reports_the_open_backend_not_the_requested_kind() {
        for name in ["null", "x11", "wlroots", "portal", "native"] {
            let dir = temp_dir(&format!("backend-{name}"));
            let (mut backend, _handles) = one_mk_input::null(Desktop::new(vec![Rect::new(0, 0, 800, 600)]));
            backend.name = name;
            let (stop, mut stopped) = watch::channel(false);
            let (power, _power_handles) = one_mk_input::fake_power();
            let opts = Options {
                backend: one_mk_input::Kind::Null, port: free_port(), connect: vec![], discovery: false,
                clipboard: None, downloads: dir.join("downloads"), start_locked: false, power: Some(power),
            };
            let task = tokio::spawn(serve(dir.clone(), opts, backend, async move { let _ = stopped.wait_for(|value| *value).await; }));
            eventually_async("status socket", || async { control::request(&dir, Request::Status).await.ok().flatten().is_some() }).await;
            assert_eq!(status(&dir).await.backend.as_str(), name);
            stop.send(true).unwrap();
            task.await.unwrap().unwrap();
            std::fs::remove_dir_all(dir).unwrap();
        }
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
            start_locked: false,
            power: None,
        };
        let a = tokio::spawn(serve(da.clone(), opts(pa, vec![]), ba, stop(stop_rx.clone())));
        let b = tokio::spawn(serve(db.clone(), opts(pb, vec![SocketAddr::from(([127, 0, 0, 1], pa))]), bb, stop(stop_rx)));

        // A arms its right edge once B is connected.
        assert_eq!(next_capture(&mut ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Right)]));
        assert_eq!(next_capture(&mut hb).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Left)]));

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

    /// A paired computer that isn't there yet is dialed again (with
    /// back-off) until it is: A knows B's address, B doesn't know A's.
    #[tokio::test]
    async fn a_missing_peer_is_dialed_until_it_is_there() {
        let (da, db) = (temp_dir("redial-a"), temp_dir("redial-b"));
        let (ia, ib) = (Identity::load_or_create(&da).unwrap(), Identity::load_or_create(&db).unwrap());
        trust(&da, &ib, Side::Right);
        trust(&db, &ia, Side::Left);
        let (pa, pb) = (free_port(), free_port());
        let mut t = TrustStore::load(&da).unwrap();
        t.get_mut(&ib.fingerprint).unwrap().addrs = vec![format!("127.0.0.1:{pb}")];
        t.save().unwrap();
        let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
        let (ba, mut ha) = one_mk_input::null(desk.clone());
        let (bb, _hb) = one_mk_input::null(desk);
        let (stop, stop_rx) = watch::channel(false);
        let wait = |mut rx: watch::Receiver<bool>| async move {
            let _ = rx.wait_for(|s| *s).await;
        };
        let opts = |port, dir: &std::path::Path| Options {
            backend: one_mk_input::Kind::Null,
            port,
            connect: vec![],
            discovery: false,
            clipboard: None,
            downloads: dir.join("downloads"),
            start_locked: false,
            power: Some(one_mk_input::fake_power().0),
        };
        let a = tokio::spawn(serve(da.clone(), opts(pa, &da), ba, wait(stop_rx.clone())));
        // A's first dial finds nobody (and gives up); B starts after it.
        tokio::time::sleep(DIAL_TIMEOUT + Duration::from_millis(500)).await;
        let b = tokio::spawn(serve(db.clone(), opts(pb, &db), bb, wait(stop_rx)));
        let armed = tokio::time::timeout(Duration::from_secs(10), ha.capture.recv()).await.expect("A dialed B again");
        assert_eq!(armed, Some(CaptureCmd::SetSides(vec![Armed::whole(Side::Right)])));
        stop.send(true).unwrap();
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
        /// A's and B's fake power functions.
        powa: one_mk_input::PowerHandles,
        powb: one_mk_input::PowerHandles,
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
            let desk = Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]);
            Self::start_on(tag, clips, setup, desk.clone(), desk).await
        }

        /// Like `start_with`, with A's monitors `desk_a` and B's `desk_b`.
        async fn start_on(
            tag: &str,
            clips: Option<(one_mk_clipboard::Clipboard, one_mk_clipboard::Clipboard)>,
            setup: impl FnOnce(&mut Device, &mut Device),
            desk_a: Desktop,
            desk_b: Desktop,
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
            let (ba, ha) = one_mk_input::null(desk_a);
            let (bb, hb) = one_mk_input::null(desk_b);
            let (stop, stop_rx) = watch::channel(false);
            let wait = |mut rx: watch::Receiver<bool>| async move {
                let _ = rx.wait_for(|s| *s).await;
            };
            let (ca, cb) = clips.map_or((None, None), |(a, b)| (Some(a), Some(b)));
            let ((power_a, powa), (power_b, powb)) = (one_mk_input::fake_power(), one_mk_input::fake_power());
            let opts = |port, connect, clipboard, dir: &std::path::Path, power| Options {
                backend: one_mk_input::Kind::Null,
                port,
                connect,
                discovery: false,
                clipboard,
                downloads: dir.join("downloads"),
                start_locked: false,
                power: Some(power),
            };
            let a = tokio::spawn(serve(da.clone(), opts(pa, vec![], ca, &da, power_a), ba, wait(stop_rx.clone())));
            let to_a = vec![SocketAddr::from(([127, 0, 0, 1], pa))];
            let b = tokio::spawn(serve(db.clone(), opts(pb, to_a, cb, &db, power_b), bb, wait(stop_rx)));
            Duo { da, db, ha, hb, powa, powb, pb, fa: ia.fingerprint.to_string(), fb: ib.fingerprint.to_string(), stop, tasks: vec![a, b] }
        }

        /// Waits until each side has armed the edge facing the other.
        async fn settle(&mut self) {
            self.settle_as(Armed::whole(Side::Right), Armed::whole(Side::Left)).await;
        }

        /// Waits until A has armed `a` and B `b`.
        async fn settle_as(&mut self, a: Armed, b: Armed) {
            assert_eq!(next_capture(&mut self.ha).await, CaptureCmd::SetSides(vec![a]));
            assert_eq!(next_capture(&mut self.hb).await, CaptureCmd::SetSides(vec![b]));
        }

        /// What `daemon_dir`'s daemon says about its single paired peer.
        async fn peer_status(dir: &std::path::Path) -> PeerStatus {
            let text = control::request(dir, Request::Status).await.unwrap().expect("daemon running");
            Status::decode(&text).unwrap().peers.remove(0)
        }

        /// `one-mk layout` run on the computer with settings in `dir`.
        async fn layout(dir: &std::path::Path, peer: &str, side: Option<Side>, swap: Option<Swap>) {
            crate::manage::set_layout(dir, &peer[..8], side, None, swap).unwrap();
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

    /// Media keys cross like any key, and one still held when the pointer
    /// leaves is released on the other side: it can never stay pressed.
    #[tokio::test]
    async fn media_keys_cross_and_are_never_left_held() {
        let mut duo = Duo::start("media", None).await;
        duo.enter_b().await;
        // Play/pause, a tap: down, up. Volume up and brightness stay held.
        for (code, down) in [(164, true), (164, false), (115, true), (225, true)] {
            duo.ha.events.send(Event::Key { code, down }).unwrap();
            assert_eq!(next_inject(&mut duo.hb).await, Inject::Key { code, down });
        }
        duo.leave_b().await;
        let mut released = Vec::new();
        while released.len() < 2 {
            match next_inject(&mut duo.hb).await {
                Inject::Key { code, down: false } => released.push(code),
                Inject::MoveTo(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        released.sort();
        assert_eq!(released, vec![115, 225]);
        duo.stop().await;
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
        let seam = Seam::new(desk.clone(), desk, Side::Right, Align::Stretch);
        let mut c = RemoteCursor::cross(seam, Pos::new(1919.0, 540.0), 1).unwrap(); // at (0, 540)
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Pinned at the right edge, then back: a sum of deltas would be wrong.
        tx.send(Event::Motion { dx: 5000.0, dy: 0.0 }).unwrap();
        tx.send(Event::Motion { dx: -100.0, dy: 10.0 }).unwrap();
        tx.send(Event::Key { code: 30, down: true }).unwrap();
        tx.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
        let out = coalesce(&mut c, (10.0, 0.0), &mut rx, |dx, dy| (dx, dy));
        let pos = Pos::new(1920.0 - 1.0 / 256.0 - 100.0, 550.0);
        assert_eq!(out.moved, Some(Datagram::Motion { epoch: 1, seq: 3, pos }));
        assert_eq!(out.leave, None);
        // Stops at the key, which is handled next, in order.
        assert_eq!(out.next, Some(Event::Key { code: 30, down: true }));
        assert_eq!(rx.try_recv(), Ok(Event::Motion { dx: 1.0, dy: 0.0 }));

        // Leaving ends the run after the position reached so far.
        tx.send(Event::Motion { dx: -5000.0, dy: 0.0 }).unwrap();
        tx.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
        let out = coalesce(&mut c, (1.0, 0.0), &mut rx, |dx, dy| (dx, dy));
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
            Control::Layout { side_of_you: Side::Bottom, version: 0, align: None }.encode_frame(&mut flood);
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
        /// The `Layout` B sent when it connected.
        layout: Control,
    }

    impl RawPeer {
        async fn connect(tag: &str, to_dir: &std::path::Path, to_port: u16) -> RawPeer {
            Self::connect_as(tag, to_dir, to_port, |_| {}).await
        }

        /// Like `connect`, after `setup` has edited B's record of it (on
        /// top of B, unless `setup` says otherwise).
        async fn connect_as(tag: &str, to_dir: &std::path::Path, to_port: u16, setup: impl FnOnce(&mut Device)) -> RawPeer {
            Self::connect_in(temp_dir(tag), to_dir, to_port, setup).await
        }

        /// Like `connect_as`, from `dir`, which may hold an identity already.
        async fn connect_in(dir: PathBuf, to_dir: &std::path::Path, to_port: u16, setup: impl FnOnce(&mut Device)) -> RawPeer {
            let id = Identity::load_or_create(&dir).unwrap();
            trust(to_dir, &id, Side::Top);
            let mut t = TrustStore::load(to_dir).unwrap();
            setup(t.get_mut(&id.fingerprint).unwrap());
            t.save().unwrap();
            let node = Node::bind(Arc::new(id), "127.0.0.1:0".parse().unwrap()).unwrap();
            let peer = node.connect(SocketAddr::from(([127, 0, 0, 1], to_port))).await.unwrap();
            let (send, recv) = peer.open_stream(StreamKind::Control).await.unwrap();
            let mut raw = RawPeer { _node: node, dir, send, recv, buf: Vec::new(), layout: Control::Leave };
            // No screens, so B never sends its own pointer here.
            raw.send(Control::Hello { version: PROTOCOL_VERSION, name: "raw".into(), screens: vec![], nonce: 0, os: 0 })
                .await;
            assert!(matches!(raw.next().await, Control::Hello { .. }));
            assert_eq!(raw.next().await, Control::Caps(caps::LOCAL));
            raw.layout = raw.next().await;
            assert!(matches!(raw.layout, Control::Layout { .. }));
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
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Left)]));
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Right)]));
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
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Top)]));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Bottom)]));
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
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Left)]));
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Left)]));
        assert_eq!(next_capture(&mut duo.hb).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Right)]));
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

    async fn ask(dir: &std::path::Path, req: Request) -> String {
        control::request(dir, req).await.unwrap().expect("daemon running")
    }

    /// Changes `dir`'s settings and has its daemon apply them.
    async fn set(dir: &std::path::Path, change: impl FnOnce(&mut Settings)) {
        let mut s = Settings::load_or_default(dir);
        change(&mut s);
        s.save(dir).unwrap();
        assert_eq!(ask(dir, Request::Reload).await, "ok\n");
    }

    async fn status(dir: &std::path::Path) -> Status {
        Status::decode(&ask(dir, Request::Status).await).unwrap()
    }

    /// What was injected so far, without `Controlled`.
    fn injected(h: &mut NullHandles) -> Vec<Inject> {
        std::iter::from_fn(|| h.inject.try_recv().ok()).filter(|i| !matches!(i, Inject::Controlled(_))).collect()
    }

    fn switch_to(fp: &str) -> Request {
        Request::Switch(SwitchTo::Device(fp[..8].to_owned()))
    }

    /// `[hooks]`: the command runs on the computer the pointer arrives on
    /// (`enter`) and the one it leaves (`leave`), with the environment
    /// filled in, and not at all for what was already true when it was set.
    #[cfg(unix)]
    #[tokio::test]
    async fn hooks_run_on_enter_and_leave() {
        use crate::hooks::Command;
        let mut duo = Duo::start("hooks", None).await;
        let (la, lb) = (duo.da.join("hook.log"), duo.db.join("hook.log"));
        let hook = |log: &std::path::Path| {
            let script = format!("echo \"$ONE_MK_EVENT $ONE_MK_PEER $ONE_MK_SIDE\" >> {}", log.display());
            Command::Argv(vec!["/bin/sh".into(), "-c".into(), script])
        };
        for (dir, log) in [(&duo.da, &la), (&duo.db, &lb)] {
            set(dir, |s| {
                s.hooks.on_enter = Some(hook(log));
                s.hooks.on_leave = Some(hook(log));
            })
            .await;
        }
        let lines = |log: &std::path::Path| -> Vec<String> {
            std::fs::read_to_string(log).unwrap_or_default().lines().map(|l| l.split(' ').next().unwrap().to_owned()).collect()
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(lines(&la).is_empty() && lines(&lb).is_empty(), "setting a hook is not an event");

        // The pointer goes A -> B: it leaves A and enters B.
        duo.enter_b().await;
        eventually("the hooks of the first crossing", || lines(&la) == ["leave"] && lines(&lb) == ["enter"]).await;
        let (a, b) = (std::fs::read_to_string(&la).unwrap(), std::fs::read_to_string(&lb).unwrap());
        // B is on A's right, A on B's left; each names the other.
        assert!(a.trim_end().ends_with(" right") && b.trim_end().ends_with(" left"), "{a:?} {b:?}");
        assert!(a.contains(&format!(" {} ", Duo::peer_status(&duo.da).await.name)), "{a:?}");

        // And back: it enters A and leaves B. (After the rate limit.)
        tokio::time::sleep(crate::hooks::MIN_INTERVAL).await;
        duo.leave_b().await;
        eventually("the hooks of the way back", || lines(&la) == ["leave", "enter"] && lines(&lb) == ["enter", "leave"]).await;
        duo.stop().await;
    }

    fn held(p: &one_mk_input::PowerHandles) -> usize {
        p.held.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn sleeps(p: &one_mk_input::PowerHandles) -> usize {
        p.sleeps.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// `block_screensaver_on_peers`: B keeps its screen awake exactly while
    /// A controls it, if A's setting is on; never otherwise, and not after
    /// A's setting goes off, the pointer comes back, or A vanishes.
    #[tokio::test]
    async fn the_screen_is_kept_awake_only_while_controlled() {
        let mut duo = Duo::start("awake", None).await;
        // Off by default: a visit changes nothing.
        duo.enter_b().await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(duo.powb.inhibits.load(std::sync::atomic::Ordering::SeqCst), 0);
        duo.leave_b().await;

        set(&duo.da, |s| s.power.block_screensaver_on_peers = true).await;
        duo.enter_b().await;
        eventually("B to keep its screen awake", || held(&duo.powb) == 1).await;
        assert_eq!(held(&duo.powa), 0, "the controller's own screen is not held");
        duo.leave_b().await;
        eventually("B to let go when the pointer came back", || held(&duo.powb) == 0).await;

        // Turned off in the middle of a visit.
        duo.enter_b().await;
        eventually("B to hold again", || held(&duo.powb) == 1).await;
        set(&duo.da, |s| s.power.block_screensaver_on_peers = false).await;
        eventually("B to let go when the setting went off", || held(&duo.powb) == 0).await;
        set(&duo.da, |s| s.power.block_screensaver_on_peers = true).await;
        eventually("B to hold once more", || held(&duo.powb) == 1).await;

        // The controller vanishes: no word, no Leave. The lease ends the visit.
        let a = duo.tasks.remove(0);
        a.abort();
        let _ = a.await;
        eventually("B to let go of a controller that vanished", || held(&duo.powb) == 0).await;
        duo.stop().await;
    }

    /// `sleep_sync`: B's display sleeps when A's did, if both have it on;
    /// not with it off, and A is never asked back (nor B's own sleep
    /// reported by the OS announced again).
    #[tokio::test]
    async fn displays_sleep_together_without_echo() {
        use one_mk_input::PowerEvent;
        let duo = Duo::start("sleep", None).await;
        // Off everywhere by default.
        duo.powa.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(sleeps(&duo.powb), 0);
        duo.powa.events.send(PowerEvent::DisplayAsleep(false)).unwrap();

        set(&duo.da, |s| s.power.sleep_sync = true).await;
        set(&duo.db, |s| s.power.sleep_sync = true).await;
        duo.powa.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        eventually("B to sleep", || sleeps(&duo.powb) == 1).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!((sleeps(&duo.powa), sleeps(&duo.powb)), (0, 1), "no echo, and B slept once");

        // B off: it neither follows nor announces.
        duo.powa.events.send(PowerEvent::DisplayAsleep(false)).unwrap();
        duo.powb.events.send(PowerEvent::DisplayAsleep(false)).unwrap();
        set(&duo.db, |s| s.power.sleep_sync = false).await;
        duo.powa.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(sleeps(&duo.powb), 1);
        // And B's own display sleeping by itself is not announced to A.
        duo.powb.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(sleeps(&duo.powa), 0);
        duo.stop().await;
    }

    /// Sleep sync stays out of a visit: B, controlled from A, may blank its
    /// display (injected input doesn't count as activity), but the person
    /// is using A's keyboard on it, so A must not sleep; nor B when A's
    /// display, idle while its input goes to B, blanks.
    #[tokio::test]
    async fn sleep_sync_waits_while_a_visit_is_on() {
        use one_mk_input::PowerEvent;
        let mut duo = Duo::start("sleep-visit", None).await;
        set(&duo.da, |s| s.power.sleep_sync = true).await;
        set(&duo.db, |s| s.power.sleep_sync = true).await;
        tokio::time::sleep(Duration::from_millis(100)).await; // caps arrive
        duo.enter_b().await;
        duo.powb.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(sleeps(&duo.powa), 0, "the controller stays awake");
        duo.powb.events.send(PowerEvent::DisplayAsleep(false)).unwrap();
        duo.powa.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(sleeps(&duo.powb), 0, "the controlled one stays awake");
        duo.powa.events.send(PowerEvent::DisplayAsleep(false)).unwrap();

        duo.leave_b().await;
        duo.powa.events.send(PowerEvent::DisplayAsleep(true)).unwrap();
        eventually("B to sleep once the visit is over", || sleeps(&duo.powb) == 1).await;
        duo.stop().await;
    }

    const CENTER: Pos = Pos::new(960.0, 540.0);

    /// `one-mk switch` moves control to B's screen centre without an edge,
    /// keeps the visit lifecycle (held keys released on the way back), and
    /// `switch next` cycles through B and back here.
    #[tokio::test]
    async fn switch_moves_control_without_crossing_an_edge() {
        let mut duo = Duo::start("switch", None).await;
        let fb = duo.fb.clone();
        assert!(ask(&duo.da, Request::Switch(SwitchTo::Device("nobody".into()))).await.starts_with("error no paired computer"));
        assert_eq!(ask(&duo.da, Request::Switch(SwitchTo::Here)).await, "here\n\n", "already here");

        let reply = ask(&duo.da, switch_to(&fb)).await;
        assert!(reply.starts_with("ok\n"), "{reply}");
        // A asks its backend to capture; B gets the pointer at its centre.
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Activated { id: 9, side: Side::Right, pos: Pos::new(1919.0, 3.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));
        assert!(ask(&duo.da, switch_to(&fb)).await.starts_with("already\n"));
        assert!(status(&duo.da).await.controlling.is_some());

        duo.ha.events.send(Event::Key { code: 29, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 29, down: true });
        // Back here: A's pointer goes to its own centre, B lets go of Ctrl.
        assert!(ask(&duo.da, Request::Switch(SwitchTo::Here)).await.starts_with("here\nleft "));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 9, pos: Some(CENTER) });
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 29, down: false });
        // The Ctrl release that follows on A goes nowhere.
        duo.ha.events.send(Event::Key { code: 29, down: false }).unwrap();

        // next: here -> B -> here.
        assert!(ask(&duo.da, Request::Switch(SwitchTo::Next)).await.starts_with("ok\n"));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Activated { id: 10, side: Side::Right, pos: Pos::new(1919.0, 3.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));
        // The edge facing A still leads back.
        duo.ha.events.send(Event::Motion { dx: -2000.0, dy: 0.0 }).unwrap();
        assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 10, pos: Some(_) }));
        assert!(ask(&duo.da, Request::Switch(SwitchTo::Next)).await.starts_with("ok\n"));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Activated { id: 11, side: Side::Right, pos: Pos::new(1919.0, 3.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));
        assert!(ask(&duo.da, Request::Switch(SwitchTo::Next)).await.starts_with("here\n"));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 11, pos: Some(CENTER) });
        duo.stop().await;
    }

    /// `switch here` on the controlled computer works like `takeback`, and
    /// a switch the backend never starts times out quietly.
    #[tokio::test]
    async fn switch_here_takes_back_and_unanswered_switch_gives_up() {
        let mut duo = Duo::start("switch-back", None).await;
        duo.enter_b().await;
        duo.ha.events.send(Event::Key { code: 42, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 42, down: true });
        let reply = ask(&duo.db, Request::Switch(SwitchTo::Here)).await;
        assert!(reply.starts_with("here\ntook control back from "), "{reply}");
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 42, down: false });
        assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(_) }));

        // A switch whose capture never starts is dropped after the timeout.
        let fb = duo.fb.clone();
        assert!(ask(&duo.da, switch_to(&fb)).await.starts_with("ok\n"));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        tokio::time::sleep(SWITCH_TIMEOUT + Duration::from_millis(100)).await;
        // A later edge crossing is a normal one, not the stale switch.
        duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 100.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 100.0)));
        duo.stop().await;
    }

    /// While the pointer is locked no edge is armed (a stray activation is
    /// refused), `switch` still works and its edge leads back, and status
    /// shows the lock.
    #[tokio::test]
    async fn lock_pointer_disarms_edges() {
        let mut duo = Duo::start("lockptr", None).await;
        assert_eq!(ask(&duo.da, Request::LockPointer(Toggle::On)).await, "ok\non\n");
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![]));
        assert!(status(&duo.da).await.pointer_locked);
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 500.0) }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: None });

        let fb = duo.fb.clone();
        assert!(ask(&duo.da, switch_to(&fb)).await.starts_with("ok\n"));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Right)]));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 500.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));
        duo.ha.events.send(Event::Motion { dx: -2000.0, dy: 0.0 }).unwrap();
        assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 2, pos: Some(_) }));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![]), "locked again once home");

        assert_eq!(ask(&duo.da, Request::LockPointer(Toggle::Toggle)).await, "ok\noff\n");
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Right)]));
        assert!(!status(&duo.da).await.pointer_locked);
        duo.stop().await;
    }

    /// Keyboard shortcuts: the system's (simulated here) at home, the
    /// matcher on captured keys away. The chord's key never reaches the
    /// peer and its modifiers don't stay held there.
    #[tokio::test]
    async fn keyboard_shortcuts_switch_without_leaving_keys_held() {
        use one_mk_input::hotkeys::Action;
        const CTRL: u32 = 29;
        const ALT: u32 = 56;
        const KEY_H: u32 = 35;
        const KEY_X: u32 = 45;
        let mut duo = Duo::start("shortcuts", None).await;
        assert_eq!(status(&duo.da).await.hotkeys, "unavailable", "nothing listens with the null backend");
        let (ia, ib) = (Identity::load_or_create(&duo.da).unwrap(), Identity::load_or_create(&duo.db).unwrap());
        let order = crate::hotkeys::numbered((&ia.name, &duo.fa), [(ib.name.clone(), duo.fb.clone())]);
        let n = |fp: &str| 1 + order.iter().position(|c| c.id == fp).unwrap() as u8;

        // At home: Ctrl+Alt+<B's number> from the system moves to B.
        duo.ha.events.send(Event::Hotkey(Action::Jump(n(&duo.fb)))).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Activated { id: 5, side: Side::Right, pos: Pos::new(1919.0, 3.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));

        // On B: a key held while the chord's key goes down is typing, not a shortcut.
        for (code, down) in [(KEY_X, true), (CTRL, true), (ALT, true), (KEY_H, true), (KEY_H, false), (KEY_X, false)] {
            duo.ha.events.send(Event::Key { code, down }).unwrap();
            assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code, down });
        }
        // Ctrl+Alt+H alone (Ctrl and Alt still held) brings it home; B
        // releases Ctrl and Alt and never sees the H.
        duo.ha.events.send(Event::Key { code: KEY_H, down: true }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 5, pos: Some(CENTER) });
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: CTRL, down: false });
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: ALT, down: false });
        for code in [KEY_H, CTRL, ALT] {
            duo.ha.events.send(Event::Key { code, down: false }).unwrap();
        }

        // Back on B (the system's "next"), Ctrl+Alt+<this computer's
        // number> comes home too, Ctrl and Alt held since before the capture.
        duo.ha.events.send(Event::Hotkey(Action::Cycle)).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Activated { id: 6, side: Side::Right, pos: Pos::new(1919.0, 3.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));
        duo.ha.events.send(Event::Modifiers(Mods::from_keys([CTRL, ALT]))).unwrap();
        duo.ha.events.send(Event::Key { code: 1 + u32::from(n(&duo.fa)), down: true }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 6, pos: Some(CENTER) });
        assert!(injected(&mut duo.hb).iter().all(|i| !matches!(i, Inject::Key { .. })), "no key reached B");

        // Lock pointer, from the system, toggles like `one-mk lock-pointer toggle`.
        duo.ha.events.send(Event::Hotkey(Action::LockPointer)).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![]));
        assert!(status(&duo.da).await.pointer_locked);
        duo.ha.events.send(Event::Hotkey(Action::LockPointer)).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::SetSides(vec![Armed::whole(Side::Right)]));

        // Turned off in settings.toml: the chord is just keys again.
        set(&duo.da, |s| s.hotkeys.enabled = false).await;
        assert_eq!(status(&duo.da).await.hotkeys, "off");
        duo.enter_b().await;
        duo.ha.events.send(Event::Key { code: KEY_H, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: KEY_H, down: true });
        duo.stop().await;
    }

    /// While A controls B, B's own shortcuts don't run: its system reports
    /// the chords typed on A as if pressed on B (they come through B's
    /// injector), and they are for programs on B. Once A lets go they work.
    #[tokio::test]
    async fn a_controlled_computer_ignores_its_own_shortcuts() {
        use one_mk_input::hotkeys::Action;
        let mut duo = Duo::start("shortcuts-controlled", None).await;
        duo.enter_b().await;
        eventually_async("B to be controlled", || async { status(&duo.db).await.controlled_by.is_some() }).await;
        duo.hb.events.send(Event::Hotkey(Action::LockPointer)).unwrap();
        duo.hb.events.send(Event::Hotkey(Action::Home)).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let st = status(&duo.db).await;
        assert!(!st.pointer_locked && st.controlled_by.is_some(), "neither locked nor took control back");
        assert!(duo.ha.capture.try_recv().is_err(), "A still has the pointer on B");

        duo.leave_b().await;
        eventually_async("B to be free", || async { status(&duo.db).await.controlled_by.is_none() }).await;
        duo.hb.events.send(Event::Hotkey(Action::LockPointer)).unwrap();
        eventually_async("B's shortcut to work again", || async { status(&duo.db).await.pointer_locked }).await;
        duo.stop().await;
    }

    /// Ctrl and Alt held through the shortcut that moved the pointer away
    /// (reported at activation), then released, which Windows and macOS
    /// leave to this computer and report as `LocalKeyUp`: what is typed
    /// next is just keys, not Ctrl+Alt shortcuts.
    #[tokio::test]
    async fn modifiers_released_after_a_switch_are_forgotten() {
        use one_mk_input::hotkeys::Action;
        const CTRL: u32 = 29;
        const ALT: u32 = 56;
        const KEY_H: u32 = 35;
        let mut duo = Duo::start("stale-mods", None).await;
        duo.ha.events.send(Event::Hotkey(Action::Cycle)).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Activate { side: Side::Right });
        duo.ha.events.send(Event::Modifiers(Mods::from_keys([CTRL, ALT]))).unwrap();
        duo.ha.events.send(Event::Activated { id: 5, side: Side::Right, pos: Pos::new(1919.0, 3.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(CENTER));
        for code in [CTRL, ALT] {
            duo.ha.events.send(Event::LocalKeyUp { code }).unwrap();
        }
        duo.ha.events.send(Event::Key { code: KEY_H, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: KEY_H, down: true }, "H, not Ctrl+Alt+H");
        assert!(duo.ha.capture.try_recv().is_err(), "the pointer stayed on B");
        duo.stop().await;
    }

    /// Modifier-to-cross with a dwell: Shift was held when the pointer hit
    /// the edge, then let go (its release stays here): the dwell ending
    /// doesn't cross.
    #[tokio::test]
    async fn a_modifier_released_here_no_longer_crosses() {
        let mut duo = Duo::start("stale-mod-edge", None).await;
        set(&duo.da, |s| {
            s.edge.all.dwell_ms = Some(100);
            s.edge.all.modifier = Some("shift".into());
        })
        .await;
        duo.ha.events.send(Event::Modifiers(Mods::from_keys([42]))).unwrap();
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        duo.ha.events.send(Event::LocalKeyUp { code: 42 }).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(injected(&mut duo.hb), vec![], "did not cross");
        duo.ha.events.send(Event::Motion { dx: -30.0, dy: 0.0 }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(1889.0, 540.0)) });
        duo.stop().await;
    }

    /// A dwell holds the pointer at the edge for its time, with no timer
    /// otherwise; moving away first hands it back where it would be.
    #[tokio::test]
    async fn dwell_holds_the_pointer_at_the_edge() {
        let mut duo = Duo::start("dwell", None).await;
        set(&duo.da, |s| s.edge.all.dwell_ms = Some(200)).await;
        assert_eq!(status(&duo.da).await.edges, "dwell 200 ms");
        let started = Instant::now();
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        duo.ha.events.send(Event::Motion { dx: 40.0, dy: 10.0 }).unwrap(); // pushing, sliding down
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 550.0)));
        assert!(started.elapsed() >= Duration::from_millis(200), "{:?}", started.elapsed());
        duo.leave_b().await;

        duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        duo.ha.events.send(Event::Motion { dx: -30.0, dy: 0.0 }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 2, pos: Some(Pos::new(1889.0, 540.0)) });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(injected(&mut duo.hb), vec![], "never crossed");
        duo.stop().await;
    }

    /// Modifier-to-cross: held at activation (Windows/macOS report it) it
    /// crosses at once; pressed while held at the edge it crosses then and
    /// is not sent on; another key hands the pointer back and is typed here
    /// as a tap, so nothing stays held by the injector.
    #[tokio::test]
    async fn modifier_to_cross() {
        let mut duo = Duo::start("modifier", None).await;
        set(&duo.da, |s| s.edge.all.modifier = Some("shift".into())).await;
        duo.ha.events.send(Event::Modifiers(Mods::from_keys([42]))).unwrap();
        duo.enter_b().await;
        duo.leave_b().await;

        duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(injected(&mut duo.hb), vec![], "held at the edge: Shift was released when capture ended");
        duo.ha.events.send(Event::Key { code: 54, down: true }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 540.0)));
        duo.ha.events.send(Event::Key { code: 30, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 30, down: true }, "Shift was not sent");
        duo.ha.events.send(Event::Key { code: 30, down: false }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 30, down: false });
        duo.leave_b().await;

        duo.ha.events.send(Event::Activated { id: 3, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        duo.ha.events.send(Event::Key { code: 30, down: true }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 3, pos: Some(Pos::new(1919.0, 540.0)) });
        assert_eq!(next_inject(&mut duo.ha).await, Inject::Key { code: 30, down: true }, "typed here, not lost");
        assert_eq!(next_inject(&mut duo.ha).await, Inject::Key { code: 30, down: false }, "and released here");
        // Its real key-up comes once capture has ended; nothing more.
        duo.ha.events.send(Event::Key { code: 30, down: false }).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(injected(&mut duo.ha), vec![]);
        duo.stop().await;
    }

    /// Held at the edge by a dwell, with Shift (the modifier to cross)
    /// pressed meanwhile: a click hands the pointer back as a Shift-click
    /// here, and Shift goes up again right after it, since its real key-up
    /// will bypass the injector (the stuck key the injector used to repeat).
    #[tokio::test]
    async fn handing_back_at_the_edge_leaves_no_key_held() {
        let mut duo = Duo::start("handback", None).await;
        set(&duo.da, |s| {
            s.edge.all.dwell_ms = Some(5000);
            s.edge.all.modifier = Some("shift".into());
        })
        .await;
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        duo.ha.events.send(Event::Key { code: 42, down: true }).unwrap();
        duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(1919.0, 540.0)) });
        let mut got = Vec::new();
        for _ in 0..4 {
            got.push(next_inject(&mut duo.ha).await);
        }
        assert_eq!(
            got,
            vec![
                Inject::Key { code: 42, down: true },
                Inject::Button { button: 0x110, down: true },
                Inject::Button { button: 0x110, down: false },
                Inject::Key { code: 42, down: false },
            ]
        );
        // Their real releases, still captured: already done here.
        duo.ha.events.send(Event::Button { button: 0x110, down: false }).unwrap();
        duo.ha.events.send(Event::Key { code: 42, down: false }).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(injected(&mut duo.ha), vec![]);
        assert_eq!(injected(&mut duo.hb), vec![], "never crossed");
        duo.stop().await;
    }

    /// A click while held at the edge is replayed here as a whole click:
    /// its real button-up may have been captured too (and then adds
    /// nothing), reach the system past the injector (which on Wayland and
    /// macOS would keep a replayed press down), or never be seen at all
    /// (the portal and wlroots don't report it). Either way no button
    /// stays held. A drag cannot start at such an edge.
    #[tokio::test]
    async fn click_at_the_edge_leaves_no_button_held() {
        let mut duo = Duo::start("handback-click", None).await;
        set(&duo.da, |s| s.edge.all.dwell_ms = Some(5000)).await;
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
        duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
        // The button-up, captured before the release took effect.
        duo.ha.events.send(Event::Button { button: 0x110, down: false }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(Pos::new(1919.0, 540.0)) });
        assert_eq!(next_inject(&mut duo.ha).await, Inject::Button { button: 0x110, down: true });
        assert_eq!(next_inject(&mut duo.ha).await, Inject::Button { button: 0x110, down: false });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(injected(&mut duo.ha), vec![], "the captured button-up adds nothing");
        assert_eq!(injected(&mut duo.hb), vec![], "never crossed");
        duo.stop().await;
    }

    /// Whatever comes after a click at a guarded edge (its release
    /// reported or never seen, a new activation that holds or crosses,
    /// the capture ending, a lock, shutdown), the click is already whole
    /// and nothing more is injected here: no button was left down.
    #[tokio::test]
    async fn click_at_the_edge_is_whole_whatever_follows() {
        for after in ["nothing", "local-up", "hold-again", "cross", "incoming", "deactivated", "locked", "shutdown"] {
            let mut duo = Duo::start(&format!("click-{after}"), None).await;
            set(&duo.da, |s| s.edge.all.dwell_ms = Some(5000)).await;
            duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
            duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
            assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, .. }), "{after}");
            assert_eq!(next_inject(&mut duo.ha).await, Inject::Button { button: 0x110, down: true }, "{after}");
            // At once, not when a release that may never be seen arrives.
            let up = tokio::time::timeout(Duration::from_millis(500), next_inject(&mut duo.ha)).await;
            assert_eq!(up.ok(), Some(Inject::Button { button: 0x110, down: false }), "{after}: a whole click");
            match after {
                "nothing" => {}
                "local-up" => duo.ha.events.send(Event::LocalKeyUp { code: 0x110 }).unwrap(),
                "hold-again" => {
                    duo.ha.events.send(Event::Deactivated { id: 1 }).unwrap();
                    duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
                }
                "cross" => {
                    duo.ha.events.send(Event::Deactivated { id: 1 }).unwrap();
                    set(&duo.da, |s| s.edge.all.dwell_ms = Some(0)).await;
                    duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 540.0) }).unwrap();
                    assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 540.0)));
                }
                "incoming" => duo.hb.events.send(Event::Activated { id: 2, side: Side::Left, pos: Pos::new(0.0, 540.0) }).unwrap(),
                "deactivated" => duo.ha.events.send(Event::Deactivated { id: 1 }).unwrap(),
                "locked" => duo.ha.events.send(Event::ScreenLocked(true)).unwrap(),
                "shutdown" => duo.stop.send(true).unwrap(),
                _ => unreachable!(),
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
            let buttons: Vec<_> = injected(&mut duo.ha).into_iter().filter(|i| matches!(i, Inject::Button { .. })).collect();
            assert_eq!(buttons, vec![], "{after}: nothing left to release");
            if after == "shutdown" {
                for task in duo.tasks {
                    task.await.unwrap().unwrap();
                }
                let _ = std::fs::remove_dir_all(&duo.da);
                let _ = std::fs::remove_dir_all(&duo.db);
            } else {
                duo.stop().await;
            }
        }
    }

    /// Dead corners and the fullscreen guard refuse at once; the allow list
    /// lets a fullscreen program be left.
    #[tokio::test]
    async fn corners_and_fullscreen_refuse() {
        let mut duo = Duo::start("guards", None).await;
        set(&duo.da, |s| {
            s.edge.right = Some(crate::settings::Rules { corner: Some(20), ..Default::default() });
            s.edge.all.fullscreen_guard = Some(true);
        })
        .await;
        duo.ha.events.send(Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 5.0) }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: None });

        *duo.ha.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec!["game".into()] });
        duo.ha.events.send(Event::Activated { id: 2, side: Side::Right, pos: Pos::new(1919.0, 500.0) }).unwrap();
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 2, pos: None });
        set(&duo.da, |s| s.edge.fullscreen_allow.push("GAME".into())).await;
        duo.ha.events.send(Event::Activated { id: 3, side: Side::Right, pos: Pos::new(1919.0, 500.0) }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 500.0)));
        assert!(injected(&mut duo.hb).is_empty());
        duo.stop().await;
    }

    /// Gaming mode: relative moves that never clip at B's edge or lead
    /// home; clicks don't warp; `switch here` brings the pointer back.
    #[tokio::test]
    async fn gaming_mode_sends_relative_motion() {
        let mut duo = Duo::start("gaming", None).await;
        set(&duo.da, |s| s.gaming = Gaming::On).await;
        tokio::time::sleep(Duration::from_millis(100)).await; // B's caps arrive
        duo.enter_b().await;
        assert!(status(&duo.da).await.relative);
        duo.ha.events.send(Event::Motion { dx: 10.4, dy: 5.0 }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveBy { dx: 10, dy: 5 });
        duo.ha.events.send(Event::Motion { dx: 0.4, dy: 0.0 }).unwrap(); // the fractions add up
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveBy { dx: 1, dy: 0 });
        // Far past A's side of B: still on B, still relative.
        duo.ha.events.send(Event::Motion { dx: -5000.0, dy: 0.0 }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveBy { dx: -5000, dy: 0 });
        duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Button { button: 0x110, down: true }, "no warp first");
        assert!(duo.ha.capture.try_recv().is_err(), "the pointer did not leave B");

        assert!(ask(&duo.da, Request::Switch(SwitchTo::Here)).await.starts_with("here\n"));
        assert_eq!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(CENTER) });
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Button { button: 0x110, down: false });
        duo.stop().await;
    }

    /// Gaming mode `auto`: relative while B reports a fullscreen window,
    /// absolute again once a click finds it isn't.
    #[tokio::test]
    async fn gaming_auto_follows_the_peers_fullscreen_window() {
        let mut duo = Duo::start("gaming-auto", None).await;
        set(&duo.da, |s| s.gaming = Gaming::Auto).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        duo.enter_b().await;
        eventually_async("relative motion", || async { status(&duo.da).await.relative }).await;
        assert!(duo.hb.focus_probes.load(std::sync::atomic::Ordering::SeqCst) > 0);
        duo.ha.events.send(Event::Motion { dx: 3.0, dy: 0.0 }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveBy { dx: 3, dy: 0 });

        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: false, apps: vec![] });
        duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Button { button: 0x110, down: true });
        eventually_async("absolute motion", || async { !status(&duo.da).await.relative }).await;
        // Absolute again from the estimate (B's left edge + 3), then on.
        assert_eq!(next_non_rel_move(&mut duo.hb).await, Inject::MoveTo(Pos::new(3.0, 540.0)));
        duo.ha.events.send(Event::Motion { dx: 1.0, dy: 0.0 }).unwrap();
        assert_eq!(next_non_rel_move(&mut duo.hb).await, Inject::MoveTo(Pos::new(4.0, 540.0)));
        duo.stop().await;
    }

    /// Gaming mode `auto`, out of the game without a click: Alt+Tab on B
    /// (typed on A) makes B look at its focus again once B has switched,
    /// and A goes back to absolute motion, where B's edge leads home.
    #[tokio::test]
    async fn gaming_auto_notices_alt_tab() {
        let mut duo = Duo::start("gaming-alt-tab", None).await;
        set(&duo.da, |s| s.gaming = Gaming::Auto).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        duo.enter_b().await;
        eventually_async("relative motion", || async { status(&duo.da).await.relative }).await;
        // Plain typing (a game's W) doesn't make B look.
        let probes = duo.hb.focus_probes.load(std::sync::atomic::Ordering::SeqCst);
        for (code, down) in [(17, true), (17, false)] {
            duo.ha.events.send(Event::Key { code, down }).unwrap();
        }
        tokio::time::sleep(FOCUS_SETTLE * 3).await;
        assert_eq!(duo.hb.focus_probes.load(std::sync::atomic::Ordering::SeqCst), probes, "WASD costs nothing");
        // Alt+Tab: the game loses the focus.
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: false, apps: vec![] });
        for (code, down) in [(56, true), (15, true), (15, false), (56, false)] {
            duo.ha.events.send(Event::Key { code, down }).unwrap();
        }
        eventually_async("absolute motion", || async { !status(&duo.da).await.relative }).await;
        duo.stop().await;
    }

    /// Where the OS says when another window comes to the front (Windows),
    /// B listens only while A watches its focus, and looks again at once.
    #[tokio::test]
    async fn gaming_auto_hears_the_front_window_change() {
        let mut duo = Duo::start("gaming-front", None).await;
        let listening = |h: &NullHandles| h.focus_notify.lock().unwrap().is_some();
        set(&duo.da, |s| s.gaming = Gaming::Auto).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        assert!(!listening(&duo.hb), "nobody watches yet");
        duo.enter_b().await;
        eventually_async("relative motion", || async { status(&duo.da).await.relative }).await;
        assert!(listening(&duo.hb));
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: false, apps: vec![] });
        (duo.hb.focus_notify.lock().unwrap().as_ref().unwrap())();
        tokio::time::timeout(FOCUS_REPEAT - Duration::from_millis(500), async {
            while status(&duo.da).await.relative {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("absolute motion before B would have looked again by itself");
        duo.leave_b().await;
        eventually("B to stop listening", || !listening(&duo.hb)).await;
        duo.stop().await;
    }

    /// While a fullscreen window has B's focus, B looks again and says so
    /// every `FOCUS_REPEAT` with no input at all (the focus can move by
    /// itself), so A keeps relative motion; once it isn't, A hears it.
    #[tokio::test]
    async fn gaming_auto_rechecks_without_input() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut duo = Duo::start("gaming-beat", None).await;
        set(&duo.da, |s| s.gaming = Gaming::Auto).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        duo.enter_b().await;
        eventually_async("relative motion", || async { status(&duo.da).await.relative }).await;
        let probes = duo.hb.focus_probes.load(SeqCst);
        // Keep the visit alive (B lets go after a minute without input,
        // not before), but send nothing that moves the focus.
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: false, apps: vec![] });
        tokio::time::sleep(FOCUS_REPEAT + Duration::from_millis(500)).await;
        assert!(duo.hb.focus_probes.load(SeqCst) > probes, "B looked again by itself");
        eventually_async("absolute motion", || async { !status(&duo.da).await.relative }).await;
        duo.stop().await;
    }

    /// A peer that said "fullscreen" (and promised to repeat it) and then
    /// goes quiet: A stops relative motion after `FOCUS_SILENCE`, so the
    /// edge leads home again.
    #[tokio::test]
    async fn gaming_auto_ends_when_the_peer_goes_quiet() {
        let duo = Duo::start("gaming-quiet", None).await;
        set(&duo.db, |s| s.gaming = Gaming::Auto).await;
        // C, above B, says it supports relative motion, focus and the beat.
        let mut c = RawPeer::connect("gaming-quiet-c", &duo.db, duo.pb).await;
        c.send(Control::Screens(vec![Rect::new(0, 0, 1920, 1080)])).await;
        c.send(Control::Caps(caps::RELATIVE | caps::FOCUS | caps::FOCUS_BEAT)).await;
        let db = duo.db.clone();
        eventually_async("C's screens", || async { status(&db).await.layout.places.iter().any(|p| !p.screens.is_empty()) }).await;
        duo.hb.events.send(Event::Activated { id: 1, side: Side::Top, pos: Pos::new(960.0, 0.0) }).unwrap();
        let epoch = loop {
            if let Control::Enter { epoch, .. } = c.next().await {
                break epoch;
            }
        };
        c.send(Control::EnterAck { epoch }).await;
        loop {
            if let Control::WatchFocus { on: true, .. } = c.next().await {
                break;
            }
        }
        c.send(Control::Focus { epoch, fullscreen: true, apps: vec![] }).await;
        eventually_async("relative motion", || async { status(&db).await.relative }).await;
        let since = Instant::now();
        // Then nothing more from C.
        for _ in 0..((FOCUS_SILENCE.as_millis() + 2000) / 50) {
            if !status(&db).await.relative {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!status(&db).await.relative, "still relative after {:?}", since.elapsed());
        assert!(since.elapsed() >= FOCUS_SILENCE - Duration::from_millis(500), "not before the silence: {:?}", since.elapsed());
        assert!(status(&db).await.controlling.is_some(), "still on C, now able to come home");
        duo.stop().await;
    }

    /// Nobody looks at B's windows unless A's gaming mode is `auto` (on
    /// GNOME, just connecting to X11 can start XWayland).
    #[tokio::test]
    async fn focus_is_watched_only_for_gaming_auto() {
        let mut duo = Duo::start("focus-off", None).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        duo.enter_b().await;
        duo.ha.events.send(Event::Button { button: 0x110, down: true }).unwrap();
        assert_eq!(next_inject(&mut duo.hb).await, Inject::MoveTo(Pos::new(0.0, 540.0)));
        assert_eq!(next_inject(&mut duo.hb).await, Inject::Button { button: 0x110, down: true });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!status(&duo.da).await.relative, "gaming off: B's fullscreen window changes nothing");
        assert_eq!(duo.hb.focus_probes.load(std::sync::atomic::Ordering::SeqCst), 0, "B never looked");
        assert_eq!(duo.ha.focus_probes.load(std::sync::atomic::Ordering::SeqCst), 0, "nor A (no fullscreen guard)");
        duo.stop().await;
    }

    async fn next_non_rel_move(h: &mut NullHandles) -> Inject {
        loop {
            match next_inject(h).await {
                m @ Inject::MoveTo(_) => return m,
                Inject::MoveBy { .. } | Inject::Button { .. } => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    async fn eventually_async<F: std::future::Future<Output = bool>>(what: &str, mut ok: impl FnMut() -> F) {
        for _ in 0..100 {
            if ok().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// `lock-all` locks both screens once; with lock sync on everywhere,
    /// a user locking B locks A, and nothing bounces back to B.
    #[tokio::test]
    async fn lock_all_and_lock_sync_without_echo() {
        use std::sync::atomic::Ordering::SeqCst;
        let duo = Duo::start("lock", None).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(ask(&duo.da, Request::LockAll).await, "ok\n1\n");
        eventually("both screens to lock", || duo.ha.locks.load(SeqCst) == 1 && duo.hb.locks.load(SeqCst) == 1).await;
        assert!(status(&duo.db).await.screen_locked);
        // B's lock (reported by its OS a moment later) does not echo back.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!((duo.ha.locks.load(SeqCst), duo.hb.locks.load(SeqCst)), (1, 1));

        for h in [&duo.ha, &duo.hb] {
            h.events.send(Event::ScreenLocked(false)).unwrap();
        }
        set(&duo.da, |s| s.lock_sync = true).await;
        set(&duo.db, |s| s.lock_sync = true).await;
        assert!(status(&duo.db).await.lock_sync);
        // The user locks B by hand.
        duo.hb.events.send(Event::ScreenLocked(true)).unwrap();
        eventually("A to lock", || duo.ha.locks.load(SeqCst) == 2).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!((duo.ha.locks.load(SeqCst), duo.hb.locks.load(SeqCst)), (2, 1), "no echo");
        duo.stop().await;
    }

    /// Gaming `auto`: once B can't tell what has the focus (a Wayland-native
    /// window clicked after an XWayland game quit), the fullscreen it said
    /// is taken back, so A doesn't stay relative with no edge leading home.
    /// Switching gaming off stops B looking at the focus on every click.
    #[tokio::test]
    async fn gaming_auto_unknown_focus_and_switching_it_off() {
        use std::sync::atomic::Ordering::SeqCst;
        async fn click(duo: &mut Duo) {
            for down in [true, false] {
                duo.ha.events.send(Event::Button { button: 0x110, down }).unwrap();
                loop {
                    match next_inject(&mut duo.hb).await {
                        Inject::Button { down: d, .. } if d == down => break,
                        Inject::MoveTo(_) | Inject::MoveBy { .. } => {}
                        other => panic!("unexpected {other:?}"),
                    }
                }
            }
        }
        let mut duo = Duo::start("gaming-unknown", None).await;
        set(&duo.da, |s| s.gaming = Gaming::Auto).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        duo.enter_b().await;
        eventually_async("relative motion", || async { status(&duo.da).await.relative }).await;
        *duo.hb.focus.lock().unwrap() = None;
        click(&mut duo).await;
        eventually_async("absolute motion", || async { !status(&duo.da).await.relative }).await;

        set(&duo.da, |s| s.gaming = Gaming::Off).await;
        let probes = duo.hb.focus_probes.load(SeqCst);
        *duo.hb.focus.lock().unwrap() = Some(Focus { fullscreen: true, apps: vec![] });
        click(&mut duo).await;
        click(&mut duo).await;
        assert_eq!(duo.hb.focus_probes.load(SeqCst), probes, "B stopped looking");
        assert!(!status(&duo.da).await.relative);
        // And `auto` again starts it again, within the same visit.
        set(&duo.da, |s| s.gaming = Gaming::Auto).await;
        eventually_async("relative motion again", || async { status(&duo.da).await.relative }).await;
        duo.stop().await;
    }

    /// A computer with lock sync off doesn't follow another's lock (sync
    /// is off there for a reason), but obeys an explicit `lock-all`.
    #[tokio::test]
    async fn lock_sync_off_obeys_only_lock_all() {
        use std::sync::atomic::Ordering::SeqCst;
        let duo = Duo::start("lock-off", None).await;
        set(&duo.da, |s| s.lock_sync = true).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        duo.ha.events.send(Event::ScreenLocked(true)).unwrap(); // the user locks A
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(duo.hb.locks.load(SeqCst), 0, "B has lock sync off");
        duo.ha.events.send(Event::ScreenLocked(false)).unwrap();
        assert_eq!(ask(&duo.da, Request::LockAll).await, "ok\n1\n");
        eventually("B to lock", || duo.hb.locks.load(SeqCst) == 1).await;
        duo.stop().await;
    }

    /// Where this OS can't lock (a Mac without CGSession or the login
    /// framework), `lock-all` still asks the others and says why this one
    /// didn't lock, instead of claiming it did.
    #[tokio::test]
    async fn lock_all_says_when_this_computer_can_t_lock() {
        use std::sync::atomic::Ordering::SeqCst;
        let duo = Duo::start("lock-unsupported", None).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        *duo.ha.lock_error.lock().unwrap() = Some("locking the screen is not supported".into());
        assert_eq!(ask(&duo.da, Request::LockAll).await, "ok\n1\nlocking the screen is not supported\n");
        eventually("B to lock", || duo.hb.locks.load(SeqCst) == 1).await;
        assert_eq!(duo.ha.locks.load(SeqCst), 0);
        duo.stop().await;
    }

    /// Locking while controlled releases what the controller held, and
    /// unlocking releases its modifiers once more, in case the lock screen
    /// swallowed the first releases (the stuck Win key after unlocking).
    #[tokio::test]
    async fn lock_and_unlock_release_held_keys() {
        let mut duo = Duo::start("unlock", None).await;
        duo.enter_b().await;
        duo.ha.events.send(Event::Key { code: 125, down: true }).unwrap();
        duo.ha.events.send(Event::Key { code: 38, down: true }).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 125, down: true });
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 38, down: true });
        duo.hb.events.send(Event::ScreenLocked(true)).unwrap();
        let mut up = vec![next_non_move(&mut duo.hb).await, next_non_move(&mut duo.hb).await];
        up.sort_by_key(|i| format!("{i:?}"));
        assert_eq!(up, vec![Inject::Key { code: 125, down: false }, Inject::Key { code: 38, down: false }]);
        assert!(matches!(next_capture(&mut duo.ha).await, CaptureCmd::Release { id: 1, pos: Some(_) }));
        duo.hb.events.send(Event::ScreenLocked(false)).unwrap();
        assert_eq!(next_non_move(&mut duo.hb).await, Inject::Key { code: 125, down: false }, "the modifier again");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(injected(&mut duo.hb), vec![]);
        duo.stop().await;
    }

    /// What the input backend says needs the user (a permission dialog
    /// to approve) shows in `one-mk status` until it says all is well.
    #[tokio::test]
    async fn input_problems_show_in_status() {
        let duo = Duo::start("input-status", None).await;
        let waiting = "Waiting for your approval: allow one-mk in the desktop's dialog.";
        duo.ha.events.send(Event::Status { part: "control", problem: Some(waiting.into()) }).unwrap();
        let mut seen = Vec::new();
        for _ in 0..100 {
            seen = status(&duo.da).await.input;
            if !seen.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(seen, [waiting]);
        duo.ha.events.send(Event::Status { part: "control", problem: None }).unwrap();
        for _ in 0..100 {
            seen = status(&duo.da).await.input;
            if seen.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(seen.is_empty(), "{seen:?}");
        duo.stop().await;
    }

    #[test]
    fn keys_that_may_move_the_focus() {
        let mut held = HeldKeys::new();
        assert!(!may_move_focus(17, true, &held), "plain typing (W)");
        assert!(may_move_focus(87, true, &held), "F11");
        assert!(may_move_focus(1, true, &held), "Esc");
        held.update(56, true);
        assert!(may_move_focus(15, true, &held), "Alt+Tab");
        assert!(may_move_focus(28, true, &held), "Alt+Enter");
        assert!(!may_move_focus(42, true, &held), "Alt then Shift: not yet");
        assert!(may_move_focus(56, false, &held), "Alt let go: the switch happens");
        let mut logo = HeldKeys::new();
        logo.update(125, true);
        assert!(may_move_focus(103, true, &logo), "Win+Up");
        assert!(may_move_focus(125, false, &logo), "the start menu");
        assert!(!may_move_focus(42, false, &HeldKeys::new()), "Shift let go");
    }

    #[test]
    fn relative_only_for_peers_that_support_it() {
        assert!(wants_relative(Gaming::On, caps::RELATIVE, false));
        assert!(!wants_relative(Gaming::On, 0, true), "an older peer");
        assert!(!wants_relative(Gaming::Off, caps::LOCAL, true));
        assert!(wants_relative(Gaming::Auto, caps::LOCAL, true));
        assert!(!wants_relative(Gaming::Auto, caps::LOCAL, false));
    }

    /// Offsets, walls, multi-monitor crossings and layout sync between
    /// daemons (in `app/tests/layout.rs`).
    mod layout;

    /// How many `Layout` messages the daemon in `dir` has sent.
    pub(super) fn layouts_sent_by(dir: &std::path::Path) -> Arc<std::sync::atomic::AtomicUsize> {
        static SENT: std::sync::Mutex<Vec<(PathBuf, Arc<std::sync::atomic::AtomicUsize>)>> = std::sync::Mutex::new(Vec::new());
        let mut all = SENT.lock().unwrap();
        if let Some((_, n)) = all.iter().find(|(d, _)| d == dir) {
            return n.clone();
        }
        all.push((dir.to_owned(), Arc::default()));
        all.last().unwrap().1.clone()
    }

    /// Key mapper, button, scroll and pointer tuning between daemons (in
    /// `app/tests/tuning.rs`).
    mod tuning;

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
