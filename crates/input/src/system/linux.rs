//! Linux: logind for the screen lock, X11 for the focused window.
//!
//! GNOME and KDE on Wayland have no API for "which window has the focus"
//! (on purpose), so only X11 windows can be seen: on an X11 session all of
//! them; on Wayland those that run through XWayland, which includes most
//! games (Wine and Proton). When a Wayland-native window has the focus,
//! XWayland reports an internal window instead, and the probe says
//! "unknown", so the fullscreen guard lets the pointer cross.

use super::{FocusProbe, ScreenLocker};
use crate::Event;
use ashpd::zbus;
use futures::StreamExt;
use one_mk_proto::Focus;
use tokio::sync::mpsc;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as mpsc_std;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::properties::WmClass;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, GetPropertyReply, Window};
use x11rb::rust_connection::RustConnection;

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";

/// Our graphical session's object path. `auto` is the caller's session or,
/// for a service outside any session (as `one-mk install` runs it), the
/// user's display session.
async fn session_path(conn: &zbus::Connection) -> zbus::Result<zbus::zvariant::OwnedObjectPath> {
    let manager = zbus::Proxy::new(conn, LOGIN1, "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await?;
    manager.call("GetSession", &("auto",)).await
}

/// Reports `LockedHint` changes of our session as `Event::ScreenLocked`.
/// Event-driven: it sleeps until logind says something. Desktops set the
/// hint when their lock screen comes up (GNOME, KDE, and most others).
pub(super) fn watch_lock(events: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
        if let Err(e) = watch(&events).await {
            tracing::debug!("not watching the screen lock: {e}");
        }
    });
}

async fn watch(events: &mpsc::UnboundedSender<Event>) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let path = session_path(&conn).await?;
    let session = zbus::Proxy::new(&conn, LOGIN1, path, SESSION).await?;
    let mut changes = session.receive_property_changed::<bool>("LockedHint").await;
    let mut last = None;
    while let Some(change) = changes.next().await {
        let Ok(locked) = change.get().await else { continue };
        // The first value is the current state, not a change.
        if last.replace(locked).is_some_and(|was| was != locked) && events.send(Event::ScreenLocked(locked)).is_err() {
            break;
        }
    }
    Ok(())
}

/// Locks through logind, which tells the desktop's lock screen.
pub(super) struct Logind;

impl ScreenLocker for Logind {
    fn lock(&mut self) -> anyhow::Result<()> {
        let rt = tokio::runtime::Handle::try_current()?;
        rt.spawn(async {
            let result = async {
                let conn = zbus::Connection::system().await?;
                let path = session_path(&conn).await?;
                let session = zbus::Proxy::new(&conn, LOGIN1, path, SESSION).await?;
                session.call::<_, _, ()>("Lock", &()).await
            };
            if let Err(e) = result.await {
                tracing::warn!("could not lock the screen: {e}");
            }
        });
        Ok(())
    }
}

/// How long the daemon waits for the focused window: a probe is two round
/// trips to a local X server, well under a millisecond. Past this the
/// answer is "unknown" (and the late one is dropped).
const ANSWER_WITHIN: Duration = Duration::from_millis(5);
/// The same when the probe must connect first (after being idle).
const CONNECT_WITHIN: Duration = Duration::from_millis(15);
/// The X connection, and the thread holding it, close after this long
/// without a probe (probes come only from the fullscreen guard at an
/// edge, and from gaming mode `auto` on clicks).
const IDLE: Duration = Duration::from_secs(60);

/// The focused X11 window. X11 calls block, so they run on a thread of
/// their own (started on demand, ended when idle) and the daemon waits
/// for the answer only briefly; a slow or hung X server makes the answer
/// "unknown" and never stalls the daemon.
///
/// On a Wayland session it connects only to an XWayland that is already
/// running: GNOME starts XWayland when a client connects, and keeps it
/// running while one is connected. So it also lets go as soon as no X11
/// window has the focus there.
#[derive(Default)]
pub(super) struct X11Focus {
    worker: Option<Worker>,
}

type Reply = mpsc_std::SyncSender<Option<Focus>>;

struct Worker {
    requests: mpsc_std::Sender<Reply>,
    /// A probe is running: don't queue behind it.
    busy: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
}

impl Worker {
    fn spawn() -> Option<Worker> {
        let (requests, rx) = mpsc_std::channel();
        let busy = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let (b, c) = (busy.clone(), connected.clone());
        let spawned = std::thread::Builder::new().name("one-mk-x11-focus".into()).spawn(move || serve(&rx, &b, &c));
        if let Err(e) = spawned {
            tracing::debug!("no focus probe thread: {e}");
            return None;
        }
        Some(Worker { requests, busy, connected })
    }
}

impl FocusProbe for X11Focus {
    fn focus(&mut self) -> Option<Focus> {
        std::env::var_os("DISPLAY")?;
        if self.worker.as_ref().is_some_and(|w| w.busy.load(Ordering::Acquire)) {
            return None;
        }
        let (tx, rx) = mpsc_std::sync_channel(1);
        let mut request = Some(tx);
        if let Some(w) = &self.worker {
            // Fails if the thread ended, idle.
            request = w.requests.send(request.take()?).err().map(|e| e.0);
        }
        if let Some(tx) = request {
            let w = Worker::spawn()?;
            w.requests.send(tx).ok()?;
            self.worker = Some(w);
        }
        let w = self.worker.as_ref()?;
        let within = if w.connected.load(Ordering::Acquire) { ANSWER_WITHIN } else { CONNECT_WITHIN };
        rx.recv_timeout(within).ok().flatten()
    }
}

/// The probe thread: answers requests until idle for [`IDLE`], then ends,
/// closing the X connection.
fn serve(requests: &mpsc_std::Receiver<Reply>, busy: &AtomicBool, connected: &AtomicBool) {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let mut conn: Option<X11> = None;
    while let Ok(reply) = requests.recv_timeout(IDLE) {
        busy.store(true, Ordering::Release);
        // SAFETY: geteuid has no preconditions.
        let uid = || unsafe { libc::geteuid() };
        if conn.is_none() && may_connect(wayland, || xwayland_running(Path::new("/proc"), uid())) {
            conn = X11::connect().map_err(|e| tracing::debug!("focus probe: no X11 connection: {e}")).ok();
        }
        let focus = match conn.as_ref().map(X11::probe) {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                tracing::debug!("focus probe: {e}; reconnecting next time");
                conn = None;
                None
            }
            None => None,
        };
        // On Wayland, with no X11 window focused, let go of XWayland so that
        // if GNOME started it on demand (for a game now gone), it can exit.
        if focus.is_none() && wayland {
            conn = None;
        }
        connected.store(conn.is_some(), Ordering::Release);
        busy.store(false, Ordering::Release);
        let _ = reply.send(focus);
    }
}

/// Whether to connect to the X server: always on an X11 session; on
/// Wayland only if XWayland already runs (connecting would start it).
fn may_connect(wayland: bool, xwayland_running: impl FnOnce() -> bool) -> bool {
    !wayland || xwayland_running()
}

/// Whether a process named `Xwayland` runs as `uid`, looking in `proc`.
fn xwayland_running(proc: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir(proc) else { return false };
    dir.flatten().any(|e| {
        let name = e.file_name();
        name.to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && e.metadata().is_ok_and(|m| m.uid() == uid)
            && std::fs::read(e.path().join("comm")).is_ok_and(|c| c.trim_ascii_end() == b"Xwayland")
    })
}

/// A connection with the atoms the probe needs, interned once.
struct X11 {
    conn: RustConnection,
    root: Window,
    active: Atom,
    clients: Atom,
    state: Atom,
    fullscreen: Atom,
}

type XResult<T> = Result<T, Box<dyn std::error::Error>>;

impl X11 {
    fn connect() -> XResult<X11> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots.get(screen).ok_or("no such screen")?.root;
        // All four requests go out before waiting for the first answer.
        let names: [&[u8]; 4] = [b"_NET_ACTIVE_WINDOW", b"_NET_CLIENT_LIST", b"_NET_WM_STATE", b"_NET_WM_STATE_FULLSCREEN"];
        let cookies = names.map(|n| conn.intern_atom(false, n));
        let mut atoms = [0; 4];
        for (a, c) in atoms.iter_mut().zip(cookies) {
            *a = c?.reply()?.atom;
        }
        let [active, clients, state, fullscreen] = atoms;
        Ok(X11 { conn, root, active, clients, state, fullscreen })
    }

    /// The focused window: two round trips.
    fn probe(&self) -> XResult<Option<Focus>> {
        let c = &self.conn;
        let windows = |r: GetPropertyReply| -> Vec<u32> { r.value32().map(Iterator::collect).unwrap_or_default() };
        let active = windows(c.get_property(false, self.root, self.active, AtomEnum::ANY, 0, 1)?.reply()?);
        let active = active.first().copied().unwrap_or(0);
        if active == 0 {
            return Ok(None);
        }
        // The rest in one round trip.
        let clients = c.get_property(false, self.root, self.clients, AtomEnum::ANY, 0, 4096)?;
        let state = c.get_property(false, active, self.state, AtomEnum::ANY, 0, 64)?;
        let class = WmClass::get(c, active)?;
        // Not one of the window manager's client windows: a Wayland window
        // (or nothing) has the focus, which X11 can't see.
        if !windows(clients.reply()?).contains(&active) {
            return Ok(None);
        }
        let fullscreen = windows(state.reply()?).contains(&self.fullscreen);
        let mut apps = Vec::new();
        if let Some(class) = class.reply()? {
            for name in [class.instance(), class.class()] {
                let name = String::from_utf8_lossy(name).into_owned();
                if !name.is_empty() && !apps.contains(&name) {
                    apps.push(name);
                }
            }
        }
        Ok(Some(Focus { fullscreen, apps }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against whatever X server `DISPLAY` names, if any: it must not
    /// fail, and with a Wayland window focused under XWayland it says
    /// "unknown" rather than "not fullscreen".
    #[test]
    fn probe_degrades_gracefully() {
        let mut p = X11Focus::default();
        let started = std::time::Instant::now();
        let first = p.focus();
        eprintln!("focused window: {first:?}");
        assert!(started.elapsed() < CONNECT_WITHIN + Duration::from_millis(50), "{:?}", started.elapsed());
        assert_eq!(p.focus().is_some(), first.is_some(), "stable between calls");
    }

    #[test]
    fn connects_on_wayland_only_to_a_running_xwayland() {
        assert!(may_connect(false, || panic!("X11 session: no need to look")));
        assert!(may_connect(true, || true));
        assert!(!may_connect(true, || false));

        let proc = std::env::temp_dir().join(format!("one-mk-test-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&proc);
        let me = unsafe { libc::geteuid() };
        for (pid, comm) in [("1", "systemd\n"), ("200", "gnome-shell\n"), ("self", "Xwayland\n")] {
            std::fs::create_dir_all(proc.join(pid)).unwrap();
            std::fs::write(proc.join(pid).join("comm"), comm).unwrap();
        }
        assert!(!xwayland_running(&proc, me), "only numbered entries are processes");
        std::fs::create_dir_all(proc.join("4321")).unwrap();
        std::fs::write(proc.join("4321").join("comm"), "Xwayland\n").unwrap();
        assert!(xwayland_running(&proc, me));
        assert!(!xwayland_running(&proc, me.wrapping_add(1)), "another user's");
        assert!(!xwayland_running(&proc.join("missing"), me));
        let _ = std::fs::remove_dir_all(&proc);
    }

    /// A probe thread that ended (idle) is replaced by the next probe.
    #[test]
    fn idle_worker_is_replaced() {
        let mut p = X11Focus::default();
        let _ = p.focus();
        // Pretend the thread ended while idle: its channel is closed.
        if let Some(w) = &mut p.worker {
            w.requests = mpsc_std::channel().0;
        }
        let _ = p.focus();
        assert!(std::env::var_os("DISPLAY").is_none() || p.worker.is_some());
    }
}
