//! Hides this computer's pointer while the desktop has captured it for
//! another computer.
//!
//! GNOME keeps drawing the pointer where it left while the mouse is on
//! another computer, and the InputCapture portal has no way to hide it. An
//! optional GNOME Shell extension (`extensions/gnome/one-mk-pointer@...`,
//! installed with `one-mk pointer-extension install`) does, when asked over
//! D-Bus: `Hide` while captured, asked again every [`RENEW`] (it shows the
//! pointer by itself after 15 seconds without), `Show` when it is back. After
//! another computer has moved this one's pointer and let go, `HideUntilMoved`
//! hides it where it stands until this computer's own mouse is used. Without
//! the extension the calls fail and nothing else happens.
use super::*;

const NAME: &str = "org.onemk.Pointer1";
const PATH: &str = "/org/onemk/Pointer1";
/// How often a standing `Hide` is renewed (the extension lets go after 15 s).
const RENEW: Duration = Duration::from_secs(5);
const CALL: Duration = Duration::from_secs(1);

/// What the pointer should be doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Want {
    /// Drawn as usual.
    Shown,
    /// Hidden while the desktop holds it captured for another computer.
    Hidden,
    /// Hidden where it stands until this computer's own mouse is used.
    UntilMoved,
}

/// Follows what the pointer should be doing, off the capture loop's back.
pub(super) struct PointerHider {
    want: watch::Sender<Want>,
}

impl PointerHider {
    pub(super) fn start(conn: zbus::Connection) -> Self {
        let (want, rx) = watch::channel(Want::Shown);
        tokio::spawn(follow(conn, rx));
        Self { want }
    }

    pub(super) fn set(&self, want: Want) {
        self.want.send_if_modified(|now| std::mem::replace(now, want) != want);
    }
}

async fn call(conn: &zbus::Connection, method: &str) -> bool {
    let call = conn.call_method(Some(NAME), PATH, Some(NAME), method, &());
    match tokio::time::timeout(CALL, call).await {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            tracing::debug!("pointer extension: {method}: {e}");
            false
        }
        Err(_) => {
            tracing::debug!("pointer extension: {method} did not answer");
            false
        }
    }
}

async fn follow(conn: zbus::Connection, mut want: watch::Receiver<Want>) {
    // Whether a hide stands, so a `Show` is only sent after one.
    let mut asked = false;
    loop {
        let now = *want.borrow_and_update();
        match now {
            Want::Hidden => {
                asked = call(&conn, "Hide").await;
                // Without the extension, don't ask again until the next capture.
                let renew = if asked { RENEW } else { Duration::MAX };
                tokio::select! {
                    changed = want.changed() => if changed.is_err() { break },
                    _ = tokio::time::sleep(renew) => {}
                }
            }
            Want::UntilMoved => {
                asked = call(&conn, "HideUntilMoved").await;
                if want.changed().await.is_err() {
                    break;
                }
            }
            Want::Shown => {
                if std::mem::take(&mut asked) {
                    call(&conn, "Show").await;
                }
                if want.changed().await.is_err() {
                    break;
                }
            }
        }
    }
    if asked {
        call(&conn, "Show").await;
    }
}
