//! Linux: the GlobalShortcuts desktop portal (GNOME 48+, KDE Plasma 6).
//!
//! One session at a time on a D-Bus connection of its own (so nothing here
//! can disturb the input portals' connection). The chords are only
//! suggestions: the desktop asks the user to confirm them (GNOME in a
//! dialog) and lets them change the keys in its settings. The task waits on
//! the portal's signals and on new bindings, nothing else.

use super::{Action, Availability, Bindings, Platform};
use crate::Event;
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use ashpd::desktop::{ResponseError, Session};
use futures::StreamExt;
use tokio::sync::{mpsc, watch};

const NO_PORTAL: &str = "this desktop has no GlobalShortcuts portal";

pub(super) struct Portal(watch::Sender<Bindings>);

impl Platform for Portal {
    fn set(&mut self, bindings: Bindings) {
        self.0.send_if_modified(|b| std::mem::replace(b, bindings) != *b);
    }
}

pub(super) fn start(bindings: Bindings, events: mpsc::UnboundedSender<Event>) -> Portal {
    let (tx, rx) = watch::channel(bindings);
    tokio::spawn(run(rx, events));
    Portal(tx)
}

/// Whether the portal is there, for `one-mk doctor`: its version.
pub async fn probe() -> Result<u32, String> {
    let portal = GlobalShortcuts::new().await.map_err(|e| one_line(&e))?;
    Ok(portal.version())
}

fn one_line(e: &ashpd::Error) -> String {
    match e {
        ashpd::Error::PortalNotFound(_) => NO_PORTAL.to_owned(),
        e => e.to_string(),
    }
}

/// How a session ended.
enum End {
    /// New bindings: start over with them.
    Changed,
    /// The daemon is gone.
    Quit,
}

async fn run(mut rx: watch::Receiver<Bindings>, events: mpsc::UnboundedSender<Event>) {
    let status = |a: Availability| {
        let _ = events.send(Event::Hotkeys(a));
    };
    let portal = async {
        let conn = ashpd::zbus::Connection::session().await?;
        GlobalShortcuts::with_connection(conn).await
    };
    let portal = match portal.await {
        Ok(p) => p,
        Err(e) => return status(Availability::Unavailable(one_line(&e))),
    };
    loop {
        let bindings = rx.borrow_and_update().clone();
        let end = if bindings.is_empty() {
            match rx.changed().await {
                Ok(()) => End::Changed,
                Err(_) => End::Quit,
            }
        } else {
            match session(&portal, &bindings, &mut rx, &events).await {
                Ok(end) => end,
                Err(why) => {
                    tracing::debug!("global shortcuts: {why}");
                    status(Availability::Unavailable(why));
                    match rx.changed().await {
                        Ok(()) => End::Changed,
                        Err(_) => End::Quit,
                    }
                }
            }
        };
        if matches!(end, End::Quit) {
            return;
        }
    }
}

/// Binds `bindings` and reports their presses until they change.
async fn session(
    portal: &GlobalShortcuts,
    bindings: &Bindings,
    rx: &mut watch::Receiver<Bindings>,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<End, String> {
    let mut activated = portal.receive_activated().await.map_err(|e| one_line(&e))?;
    let session = portal.create_session(Default::default()).await.map_err(|e| one_line(&e))?;
    let _ = events.send(Event::Hotkeys(Availability::Starting(
        "asking the desktop; confirm the shortcuts if it shows a dialog".into(),
    )));
    let shortcuts: Vec<NewShortcut> = bindings
        .iter()
        .map(|(a, c)| NewShortcut::new(a.id(), a.describe()).preferred_trigger(c.xdg_trigger().as_str()))
        .collect();
    // The desktop may wait for the user here.
    let bound = tokio::select! {
        r = portal.bind_shortcuts(&session, &shortcuts, None, Default::default()) => r.and_then(|r| r.response()),
        changed = rx.changed() => {
            close(&session).await;
            return Ok(if changed.is_ok() { End::Changed } else { End::Quit });
        }
    };
    match bound {
        Ok(b) => {
            let keys: Vec<String> = b
                .shortcuts()
                .iter()
                .filter(|s| !s.trigger_description().is_empty())
                .map(|s| s.trigger_description().to_owned())
                .collect();
            tracing::debug!(?keys, "global shortcuts bound");
            let how = if keys.is_empty() {
                "through the desktop, which has no keys set for them yet: set them in its keyboard settings".to_owned()
            } else {
                "through the desktop (change the keys in its keyboard settings)".to_owned()
            };
            let _ = events.send(Event::Hotkeys(Availability::Active(how)));
        }
        Err(e) => {
            close(&session).await;
            return Err(match e {
                ashpd::Error::Response(ResponseError::Cancelled) => {
                    "they were turned down in the desktop's dialog; restart one-mk to be asked again".into()
                }
                e => one_line(&e),
            });
        }
    }
    let end = loop {
        tokio::select! {
            a = activated.next() => match a {
                Some(a) => match Action::from_id(a.shortcut_id()) {
                    Some(action) => {
                        if events.send(Event::Hotkey(action)).is_err() {
                            break End::Quit;
                        }
                    }
                    None => tracing::debug!(id = a.shortcut_id(), "unknown shortcut"),
                },
                None => {
                    close(&session).await;
                    return Err("the desktop's shortcut service went away".into());
                }
            },
            changed = rx.changed() => break if changed.is_ok() { End::Changed } else { End::Quit },
        }
    };
    close(&session).await;
    Ok(end)
}

async fn close(session: &Session<GlobalShortcuts>) {
    if let Err(e) = session.close().await {
        tracing::debug!("closing the shortcut session: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::Chord;
    use std::time::Duration;

    /// Binds one throwaway shortcut through the real portal and reports what
    /// happens. Not run by default: the desktop may show a dialog. Run with
    /// `cargo test -p one-mk-input real_portal -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn real_portal() {
        println!("probe: {:?}", probe().await);
        let chord: Chord = "ctrl+alt+shift+f12".parse().unwrap();
        let bindings = Bindings::new(vec![(Action::Jump(9), chord)]).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let portal = start(bindings, tx);
        let deadline = tokio::time::sleep(Duration::from_secs(20));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                ev = rx.recv() => {
                    println!("event: {ev:?}");
                    if matches!(ev, None | Some(Event::Hotkeys(Availability::Active(_) | Availability::Unavailable(_)))) {
                        break;
                    }
                }
                _ = &mut deadline => {
                    println!("no answer in 20 s (a dialog waiting for the user?)");
                    break;
                }
            }
        }
        drop(portal); // closes the session, and with it any dialog
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
