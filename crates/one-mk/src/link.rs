//! One connection to a trusted peer: the handshake, and the task that
//! applies input the peer sends us when it controls this machine.

use anyhow::{Context, Result, bail, ensure};
use one_mk_input::Inject;
use one_mk_net::{ControlWriter, Fingerprint, Identity, Peer, read_frame};
use one_mk_proto::{Control, Datagram, HeldKeys, MotionReceiver, PROTOCOL_VERSION, Rect, StreamKind};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// If the controlling peer goes silent this long while keys are held, the
/// keys are released. The sender keeps traffic flowing every 20 ms while
/// it controls us, so this only fires when something is really wrong.
const STUCK_KEY_TIMEOUT: Duration = Duration::from_millis(750);

pub struct Link {
    pub fp: Fingerprint,
    pub id: usize,
    pub name: String,
    pub initiator: bool,
    pub conn: quinn::Connection,
    pub ctl: ControlWriter,
    pub screens: Vec<Rect>,
    recv: quinn::RecvStream,
}

pub enum LinkEvent {
    Screens(Vec<Rect>),
    Closed,
}

/// Opens (or accepts) the control stream and exchanges `Hello`s.
pub async fn handshake(peer: Peer, me: &Identity, screens: Vec<Rect>) -> Result<Link> {
    let (send, mut recv) = if peer.initiator {
        peer.open_stream(StreamKind::Control).await?
    } else {
        let (kind, send, recv) = peer.accept_stream().await?;
        ensure!(kind == StreamKind::Control, "expected control stream, got {kind:?}");
        (send, recv)
    };
    let ctl = ControlWriter::spawn(send);
    ctl.send(Control::Hello { version: PROTOCOL_VERSION, name: me.name.clone(), screens });

    let mut buf = Vec::new();
    let hello = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv, &mut buf))
        .await
        .context("peer did not say hello")??;
    let Some(Control::Hello { version, name, screens }) = hello else {
        bail!("peer sent an unexpected first message");
    };
    ensure!(
        version == PROTOCOL_VERSION,
        "{name} runs an incompatible version of one-mk (protocol {version}, we speak {PROTOCOL_VERSION}); update both"
    );
    Ok(Link {
        fp: peer.fingerprint,
        id: peer.conn.stable_id(),
        name,
        initiator: peer.initiator,
        conn: peer.conn,
        ctl,
        screens,
        recv,
    })
}

impl Link {
    /// Spawns the receive side. Returns the parts the app keeps.
    pub fn spawn(
        self,
        inject: mpsc::UnboundedSender<Inject>,
        events: mpsc::UnboundedSender<(Fingerprint, usize, LinkEvent)>,
    ) -> (quinn::Connection, ControlWriter) {
        let Link { fp, id, conn, ctl, mut recv, .. } = self;
        let task_conn = conn.clone();
        tokio::spawn(async move {
            let mut target = Target::new(inject);
            let send = |e| {
                let _ = events.send((fp, id, e));
            };
            let mut buf = Vec::with_capacity(256);
            loop {
                let check_stuck = !target.held.is_empty();
                tokio::select! {
                    biased;
                    d = task_conn.read_datagram() => match d {
                        Ok(bytes) => {
                            target.last_rx = Instant::now();
                            match Datagram::decode(&bytes) {
                                Ok(Datagram::Probe { id, sent_ns }) => {
                                    let (b, n) = Datagram::ProbeEcho { id, sent_ns }.encode();
                                    let _ = task_conn.send_datagram(bytes::Bytes::copy_from_slice(&b[..n]));
                                }
                                Ok(d) => target.datagram(d),
                                Err(e) => tracing::debug!("bad datagram: {e}"),
                            }
                        }
                        Err(_) => break,
                    },
                    f = read_frame(&mut recv, &mut buf) => match f {
                        Ok(Some(msg)) => {
                            target.last_rx = Instant::now();
                            if let Some(e) = target.control(msg) {
                                send(e);
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::debug!("control stream: {e:#}");
                            break;
                        }
                    },
                    _ = tokio::time::sleep(STUCK_KEY_TIMEOUT / 3), if check_stuck => {
                        if target.last_rx.elapsed() >= STUCK_KEY_TIMEOUT {
                            tracing::warn!("controlling peer went silent; releasing held keys");
                            target.release_all();
                        }
                    }
                }
            }
            target.release_all();
            send(LinkEvent::Closed);
        });
        (conn, ctl)
    }
}

/// State for when the peer controls this machine.
struct Target {
    inject: mpsc::UnboundedSender<Inject>,
    motion: MotionReceiver,
    held: HeldKeys,
    last_rx: Instant,
}

impl Target {
    fn new(inject: mpsc::UnboundedSender<Inject>) -> Self {
        Self { inject, motion: MotionReceiver::default(), held: HeldKeys::new(), last_rx: Instant::now() }
    }

    fn datagram(&mut self, d: Datagram) {
        if let Datagram::Motion { epoch, seq, pos } = d
            && let Some(p) = self.motion.accept(epoch, seq, pos)
        {
            let _ = self.inject.send(Inject::MoveTo(p));
        }
    }

    fn control(&mut self, msg: Control) -> Option<LinkEvent> {
        let inject = |i| {
            let _ = self.inject.send(i);
        };
        match msg {
            Control::Hello { screens, .. } | Control::Screens(screens) => return Some(LinkEvent::Screens(screens)),
            Control::Enter { epoch, pos } => {
                self.motion.enter(epoch);
                inject(Inject::MoveTo(pos));
            }
            Control::Leave => {
                self.motion.leave();
                self.release_all();
            }
            Control::Key { code, down } => {
                if self.motion.is_active() && self.held.update(code, down) {
                    inject(Inject::Key { code, down });
                }
            }
            Control::Button { button, down, seq, pos } => {
                if self.motion.is_active() {
                    self.motion.sync(seq);
                    inject(Inject::MoveTo(pos));
                    if self.held.update(button, down) {
                        inject(Inject::Button { button, down });
                    }
                }
            }
            Control::Scroll { dx, dy } if self.motion.is_active() => {
                inject(Inject::Scroll { dx: f64::from(dx), dy: f64::from(dy) })
            }
            Control::ScrollDiscrete { dx, dy } if self.motion.is_active() => inject(Inject::ScrollDiscrete { dx, dy }),
            Control::ScrollStop if self.motion.is_active() => inject(Inject::ScrollStop),
            Control::Scroll { .. } | Control::ScrollDiscrete { .. } | Control::ScrollStop => {}
        }
        None
    }

    fn release_all(&mut self) {
        for code in self.held.take_all() {
            let release = if is_button(code) {
                Inject::Button { button: code, down: false }
            } else {
                Inject::Key { code, down: false }
            };
            let _ = self.inject.send(release);
        }
    }
}

/// evdev BTN_* codes for mice live in 0x110..0x120.
pub fn is_button(code: u32) -> bool {
    (0x110..0x120).contains(&code)
}
