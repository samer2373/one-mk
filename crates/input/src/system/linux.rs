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
use x11rb::connection::Connection;
use x11rb::properties::WmClass;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};
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

/// The focused X11 window, over a connection kept open between probes
/// (they happen only when the pointer reaches an armed edge).
#[derive(Default)]
pub(super) struct X11Focus {
    conn: Option<(RustConnection, Window)>,
}

impl FocusProbe for X11Focus {
    fn focus(&mut self) -> Option<Focus> {
        std::env::var_os("DISPLAY")?;
        if self.conn.is_none() {
            let (conn, screen) = x11rb::connect(None).ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            self.conn = Some((conn, root));
        }
        let (conn, root) = self.conn.as_ref()?;
        let focus = probe(conn, *root);
        if focus.is_err() {
            self.conn = None; // reconnect next time
        }
        focus.ok().flatten()
    }
}

fn probe(conn: &RustConnection, root: Window) -> Result<Option<Focus>, Box<dyn std::error::Error>> {
    let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };
    let windows = |w: Window, prop: u32| -> Result<Vec<u32>, Box<dyn std::error::Error>> {
        let r = conn.get_property(false, w, prop, AtomEnum::ANY, 0, 4096)?.reply()?;
        Ok(r.value32().map(Iterator::collect).unwrap_or_default())
    };
    let active = windows(root, atom(b"_NET_ACTIVE_WINDOW")?)?.first().copied().unwrap_or(0);
    // Not one of the window manager's client windows: a Wayland window
    // (or nothing) has the focus, which X11 can't see.
    if active == 0 || !windows(root, atom(b"_NET_CLIENT_LIST")?)?.contains(&active) {
        return Ok(None);
    }
    let state = windows(active, atom(b"_NET_WM_STATE")?)?;
    let fullscreen = state.contains(&atom(b"_NET_WM_STATE_FULLSCREEN")?);
    let mut apps = Vec::new();
    if let Some(class) = WmClass::get(conn, active)?.reply()? {
        for name in [class.instance(), class.class()] {
            let name = String::from_utf8_lossy(name).into_owned();
            if !name.is_empty() && !apps.contains(&name) {
                apps.push(name);
            }
        }
    }
    Ok(Some(Focus { fullscreen, apps }))
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
        let first = p.focus();
        eprintln!("focused window: {first:?}");
        assert_eq!(p.focus().is_some(), first.is_some(), "stable between calls");
    }
}
