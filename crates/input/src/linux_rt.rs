//! What the X11 and wlroots backends share: their thread, waiting on a
//! display connection's socket, and following the screen lock.

use crate::Event;
use anyhow::Result;
use std::future::Future;
use std::os::fd::{AsRawFd, RawFd};
use tokio::io::unix::AsyncFd;
use tokio::sync::{mpsc, watch};

/// Runs a backend on a thread of its own, with a single-threaded runtime:
/// input never queues behind the daemon's network work, and the display
/// connection never leaves the thread. The future is built on that thread,
/// so it need not be `Send`.
pub(crate) fn spawn_thread<F, Fut>(name: &str, make: F) -> Result<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    std::thread::Builder::new().name(name.into()).spawn(move || rt.block_on(make()))?;
    Ok(())
}

/// Passes the system watcher's events on to the daemon, keeping `locked`
/// up to date on the way.
pub(crate) async fn forward_lock(
    mut sys: mpsc::UnboundedReceiver<Event>,
    daemon: mpsc::UnboundedSender<Event>,
    locked: watch::Sender<bool>,
) {
    while let Some(ev) = sys.recv().await {
        if let Event::ScreenLocked(l) = ev {
            locked.send_replace(l);
        }
        if daemon.send(ev).is_err() {
            return;
        }
    }
}

/// Waits for the next change of `locked`; forever once its sender is gone.
pub(crate) async fn lock_changed(locked: &mut watch::Receiver<bool>) -> bool {
    if locked.changed().await.is_err() {
        std::future::pending::<()>().await;
    }
    *locked.borrow_and_update()
}

struct Fd(RawFd);

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

/// A display connection's socket, to sleep on until the server sends
/// something.
pub(crate) struct Socket(AsyncFd<Fd>);

impl Socket {
    /// # Safety
    ///
    /// `fd` must stay open (the connection that owns it alive) for as long
    /// as this exists.
    pub(crate) unsafe fn new(fd: RawFd) -> std::io::Result<Self> {
        // SAFETY: passed on to the caller.
        Ok(Self(unsafe { AsyncFd::register_with_interest(Fd(fd), tokio::io::Interest::READABLE)? }))
    }

    /// Returns once there is something to read. The caller reads until the
    /// socket would block before calling this again.
    pub(crate) async fn readable(&self) -> std::io::Result<()> {
        loop {
            let mut guard = self.0.readable().await?;
            if has_data(self.0.get_ref().0) {
                return Ok(());
            }
            // Stale readiness from data the caller already read.
            guard.clear_ready();
        }
    }
}

/// Whether `fd` has something to read (or hung up) right now.
fn has_data(fd: RawFd) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd, and a zero timeout.
    let n = unsafe { libc::poll(&mut p, 1, 0) };
    n != 0
}
