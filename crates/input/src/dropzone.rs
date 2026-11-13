//! OS-owned edge drop targets. No clipboard polling and no input injection.

use anyhow::{Result, bail};
use one_mk_proto::Rect;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

#[cfg(target_os = "linux")]
#[path = "dropzone_x11.rs"]
mod x11;
#[cfg(windows)]
#[path = "dropzone_windows.rs"]
mod win;
#[cfg(target_os = "macos")]
#[path = "dropzone_macos.rs"]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::with_main_loop;

pub const MAX_URI_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Zone { pub rect: Rect, pub peer: String }

#[derive(Debug)]
pub struct Drop { pub peer: String, pub paths: Vec<PathBuf> }

pub trait DropTarget {
    fn set_zones(&self, zones: Vec<Zone>);
}

pub struct Handle { zones: watch::Sender<Vec<Zone>> }

impl DropTarget for Handle {
    fn set_zones(&self, zones: Vec<Zone>) {
        self.zones.send_if_modified(|current| {
            if *current == zones { false } else { *current = zones; true }
        });
    }
}

/// Whether this desktop can offer an edge drop target at all: Windows,
/// macOS, or a Linux session on X11. Wayland offers no way to place one.
/// This is not a default: file edge drops are always off until switched on.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "macos")) || (cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some())
}

/// Words for people about where file edge drops work, and how well that is known.
pub fn availability() -> &'static str {
    if cfg!(windows) { "Experimental on Windows: built, but not yet tried on a real Windows desktop (interactive desktop only)." }
    else if cfg!(target_os = "macos") { "Experimental on macOS: built, but not yet tried on a real Mac (needs Accessibility and Input Monitoring permission)." }
    else if supported() { X11_NOTE }
    else { UNAVAILABLE }
}

pub const X11_NOTE: &str = "Experimental on X11: tried with scripted drags, not yet with a file manager on a real desktop.";

/// Why a Wayland (or other) desktop cannot have file edge drops.
pub const UNAVAILABLE: &str = "File edge drops are not available on this desktop (they need X11, Windows or macOS; Wayland has no screen-edge drop target); use `one-mk send` or the file manager's Send to entry.";

/// The edge drop target for `backend`. Nothing of the desktop's is touched
/// here: the target (on Windows a second pair of low-level hooks, on macOS
/// event monitors, on X11 a thread and a connection) starts the first time
/// there are zones, which needs file edge drops switched on, and stops once
/// there have been none for `IDLE_STOP`. With drops off (the default) it
/// never runs. Must be called inside a Tokio runtime.
pub fn start(backend: &str) -> Result<(Handle, mpsc::Receiver<Drop>)> {
    let (zones, changed) = watch::channel(Vec::new());
    let (dropped, events) = mpsc::channel(4);
    if let Some(platform) = platform(backend)? {
        tokio::spawn(supervise(changed, dropped, platform, IDLE_STOP));
    }
    Ok((Handle { zones }, events))
}

/// How long the target stays up with no zones. Zones also go away while
/// this computer controls a peer or is controlled, and coming back from a
/// short visit should not restart it.
const IDLE_STOP: Duration = Duration::from_secs(30);

/// Starts a platform target fed by a zones channel. It stops itself, and
/// lets go of everything it installed, when that channel's sender is dropped.
type Platform = fn(watch::Receiver<Vec<Zone>>, mpsc::Sender<Drop>) -> Result<()>;

fn platform(backend: &str) -> Result<Option<Platform>> {
    #[cfg(target_os = "macos")]
    if backend == "native" {
        mac::check()?;
        return Ok(Some(mac::start));
    }
    #[cfg(windows)]
    if backend == "native" {
        return Ok(Some(win::start));
    }
    #[cfg(target_os = "linux")]
    if backend == "x11" && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Ok(Some(x11::start));
    }
    let _ = backend;
    Ok(None)
}

/// Runs the platform target only while it has zones (see `start`). A target
/// that failed to start, or stopped on its own, is tried again the next time
/// zones appear after there were none.
async fn supervise(mut wanted: watch::Receiver<Vec<Zone>>, dropped: mpsc::Sender<Drop>, start: Platform, idle: Duration) {
    let mut running: Option<watch::Sender<Vec<Zone>>> = None;
    let mut failed = false;
    loop {
        if running.as_ref().is_some_and(watch::Sender::is_closed) {
            tracing::warn!("file drop target stopped");
            running = None;
            failed = true;
        }
        let zones = wanted.borrow_and_update().clone();
        if zones.is_empty() {
            failed = false;
            if let Some(target) = &running {
                target.send_replace(zones);
                tokio::select! {
                    changed = wanted.changed() => if changed.is_err() { break },
                    () = tokio::time::sleep(idle) => running = None,
                }
                continue;
            }
        } else if let Some(target) = &running {
            target.send_replace(zones);
        } else if !failed {
            let (target, receiver) = watch::channel(zones);
            let sender = dropped.clone();
            match tokio::task::spawn_blocking(move || start(receiver, sender)).await {
                Ok(Ok(())) => running = Some(target),
                Ok(Err(error)) => {
                    tracing::warn!("file edge drops unavailable: {error:#}");
                    failed = true;
                }
                Err(error) => {
                    tracing::warn!("file edge drops unavailable: {error}");
                    failed = true;
                }
            }
        }
        if wanted.changed().await.is_err() { break; }
    }
}

pub fn uri_list(bytes: &[u8]) -> Result<Vec<PathBuf>> {
    if bytes.len() > MAX_URI_BYTES { bail!("drop URI list exceeds 1 MiB"); }
    let text = std::str::from_utf8(bytes)?;
    let mut paths = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty() && !line.starts_with('#')) {
        if paths.len() == 10_000 { bail!("too many dropped paths"); }
        let uri = line.strip_prefix("file://").ok_or_else(|| anyhow::anyhow!("only local file URLs are accepted"))?;
        let path = if uri.starts_with('/') { uri } else {
            uri.strip_prefix("localhost/").map(|rest| &uri[uri.len() - rest.len() - 1..])
                .ok_or_else(|| anyhow::anyhow!("remote file URL host is not accepted"))?
        };
        if path.contains(['?', '#', '\0', '\\']) { bail!("invalid local file URL"); }
        let mut decoded = Vec::with_capacity(path.len());
        let mut input = path.as_bytes().iter().copied();
        while let Some(byte) = input.next() {
            let byte = if byte == b'%' {
                let high = input.next().and_then(|byte| (byte as char).to_digit(16));
                let low = input.next().and_then(|byte| (byte as char).to_digit(16));
                match (high, low) { (Some(high), Some(low)) => (high * 16 + low) as u8, _ => bail!("invalid percent escape") }
            } else { byte };
            if byte == 0 { bail!("NUL in file URL"); }
            decoded.push(byte);
        }
        #[cfg(unix)]
        let path = { use std::os::unix::ffi::OsStringExt; PathBuf::from(std::ffi::OsString::from_vec(decoded)) };
        #[cfg(not(unix))]
        let path = PathBuf::from(String::from_utf8(decoded)?.trim_start_matches('/'));
        paths.push(path);
    }
    if paths.is_empty() { bail!("empty file drop"); }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_intake_is_local_bounded_and_strict() {
        assert!(uri_list(b"# a comment\r\nfile:///tmp/hello%20world\r\n").is_ok());
        assert!(uri_list(b"file://localhost/tmp/a").is_ok());
        for bad in ["file://server/a", "https://example.com/a", "file:///a%", "file:///a%XZ", "file:///a%00", "file:///a?query", "file:///a#part", ""] {
            assert!(uri_list(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(uri_list(&vec![b'a'; MAX_URI_BYTES + 1]).is_err());
    }

    type Targets = std::sync::Mutex<Vec<watch::Receiver<Vec<Zone>>>>;
    static STARTED: Targets = std::sync::Mutex::new(Vec::new());
    static REFUSED: Targets = std::sync::Mutex::new(Vec::new());

    fn fake(zones: watch::Receiver<Vec<Zone>>, _: mpsc::Sender<Drop>) -> Result<()> {
        STARTED.lock().unwrap().push(zones);
        Ok(())
    }

    fn refusing(zones: watch::Receiver<Vec<Zone>>, _: mpsc::Sender<Drop>) -> Result<()> {
        REFUSED.lock().unwrap().push(zones);
        bail!("no desktop")
    }

    fn zones(peer: &str) -> Vec<Zone> { vec![Zone { rect: Rect::new(0, 0, 8, 100), peer: peer.into() }] }

    /// Lets the supervisor task run what was just sent.
    async fn settle() { tokio::time::sleep(Duration::from_millis(30)).await; }

    fn started(targets: &Targets) -> usize { targets.lock().unwrap().len() }

    /// The zones the last target started was given, or `None` once it was stopped.
    fn last(targets: &Targets) -> Option<Vec<Zone>> {
        let targets = targets.lock().unwrap();
        let target = targets.last()?;
        target.has_changed().ok().map(|_| target.borrow().clone())
    }

    #[tokio::test]
    async fn target_runs_only_while_there_are_zones() {
        const IDLE: Duration = Duration::from_millis(300);
        let (zones_tx, wanted) = watch::channel(Vec::new());
        let handle = Handle { zones: zones_tx };
        let (dropped, _events) = mpsc::channel(4);
        let supervisor = tokio::spawn(supervise(wanted, dropped, fake, IDLE));
        // Drops off (the default): zones stay empty and nothing is started, ever.
        for _ in 0..3 { handle.set_zones(Vec::new()); }
        tokio::time::sleep(IDLE * 2).await;
        assert_eq!(started(&STARTED), 0);
        // Switched on: started once, and fed every change.
        handle.set_zones(zones("a"));
        settle().await;
        assert_eq!((started(&STARTED), last(&STARTED)), (1, Some(zones("a"))));
        handle.set_zones(zones("b"));
        settle().await;
        assert_eq!((started(&STARTED), last(&STARTED)), (1, Some(zones("b"))));
        // A short time without zones (a visit to a peer) keeps it running.
        handle.set_zones(Vec::new());
        settle().await;
        assert_eq!(last(&STARTED), Some(Vec::new()), "the target hears its zones went away");
        handle.set_zones(zones("a"));
        settle().await;
        assert_eq!((started(&STARTED), last(&STARTED)), (1, Some(zones("a"))));
        // Switched off: stopped once the zones have stayed away for IDLE...
        handle.set_zones(Vec::new());
        tokio::time::sleep(IDLE / 2).await;
        assert_eq!(last(&STARTED), Some(Vec::new()));
        tokio::time::sleep(IDLE).await;
        assert_eq!(last(&STARTED), None, "stopped: everything it installed is let go");
        // ...and switched on again: a new one.
        handle.set_zones(zones("c"));
        settle().await;
        assert_eq!((started(&STARTED), last(&STARTED)), (2, Some(zones("c"))));
        // A target that stopped on its own is not restarted until the zones
        // go away and come back.
        STARTED.lock().unwrap().pop();
        handle.set_zones(zones("d"));
        settle().await;
        assert_eq!(started(&STARTED), 1);
        handle.set_zones(Vec::new());
        settle().await;
        handle.set_zones(zones("e"));
        settle().await;
        assert_eq!((started(&STARTED), last(&STARTED)), (2, Some(zones("e"))));
        // The daemon going away stops it.
        drop(handle);
        supervisor.await.unwrap();
        assert_eq!(last(&STARTED), None);
    }

    #[tokio::test]
    async fn a_target_that_cannot_start_is_retried_only_when_switched_on_again() {
        let (zones_tx, wanted) = watch::channel(Vec::new());
        let handle = Handle { zones: zones_tx };
        let (dropped, _events) = mpsc::channel(4);
        let supervisor = tokio::spawn(supervise(wanted, dropped, refusing, Duration::from_millis(50)));
        handle.set_zones(zones("a"));
        settle().await;
        handle.set_zones(zones("b"));
        settle().await;
        assert_eq!(started(&REFUSED), 1, "not tried again on every edge change");
        handle.set_zones(Vec::new());
        settle().await;
        handle.set_zones(zones("a"));
        settle().await;
        assert_eq!(started(&REFUSED), 2);
        drop(handle);
        supervisor.await.unwrap();
    }

    #[test]
    fn a_desktop_without_drop_targets_starts_nothing() {
        #[cfg(target_os = "linux")]
        assert!(platform("portal").unwrap().is_none() && platform("wlroots").unwrap().is_none() && platform("null").unwrap().is_none());
    }
}
