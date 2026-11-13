use crate::{Cmd, history::History};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, mpsc};
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xfixes::{ConnectionExt as _, SelectionEventMask}, xproto::{ConnectionExt as _, AtomEnum, CreateWindowAux, WindowClass}};

pub(super) fn start(tx: mpsc::Sender<Cmd>, history: Arc<Mutex<History>>, apps: Arc<Mutex<Vec<String>>>) -> Option<UnixStream> {
    let (stop, wake) = UnixStream::pair().ok()?;
    let report = history.clone();
    let spawned = std::thread::Builder::new().name("one-mk-history-events".into()).spawn(move || {
        // XWayland can restart under us (see `refresh_xauthority`): connect
        // again, with a growing wait, until asked to stop.
        let mut wait = std::time::Duration::from_secs(1);
        loop {
            crate::refresh_xauthority();
            let started = std::time::Instant::now();
            match watch(&wake, &tx, &history, &apps) {
                Ok(()) => return,
                Err(error) => {
                    report.lock().unwrap_or_else(|error| error.into_inner()).status().monitoring_unavailable.store(true, std::sync::atomic::Ordering::Relaxed);
                    tracing::debug!("Clipboard history change notifications unavailable: {error}");
                }
            }
            if started.elapsed() > std::time::Duration::from_secs(10) {
                wait = std::time::Duration::from_secs(1);
            }
            if stopped(&wake, wait) {
                return;
            }
            wait = (wait * 2).min(std::time::Duration::from_secs(30));
        }
    });
    spawned.ok().map(|_| stop)
}

/// Whether the daemon asked to stop (its end of `wake` was written or closed) within `wait`.
fn stopped(wake: &UnixStream, wait: std::time::Duration) -> bool {
    let mut descriptor = libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    let ms = wait.as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: one valid pollfd.
    let ready = unsafe { libc::poll(&mut descriptor, 1, ms) };
    ready > 0
}

fn watch(wake: &UnixStream, tx: &mpsc::Sender<Cmd>, history: &Arc<Mutex<History>>, apps: &Arc<Mutex<Vec<String>>>) -> anyhow::Result<()> {
    let (connection, screen) = x11rb::connect(None)?;
    connection.xfixes_query_version(5, 0)?.reply()?;
    let root = connection.setup().roots.get(screen).ok_or_else(|| anyhow::anyhow!("no X11 screen"))?.root;
    let window = connection.generate_id()?;
    connection.create_window(0, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?.check()?;
    let clipboard = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    let active = connection.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
    connection.xfixes_select_selection_input(window, clipboard,
        SelectionEventMask::SET_SELECTION_OWNER | SelectionEventMask::SELECTION_WINDOW_DESTROY | SelectionEventMask::SELECTION_CLIENT_CLOSE)?.check()?;
    connection.flush()?;
    history.lock().unwrap_or_else(|error| error.into_inner()).status().monitoring_unavailable.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut descriptors = [
        libc::pollfd { fd: connection.stream().as_raw_fd(), events: libc::POLLIN, revents: 0 },
        libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
    ];
    loop {
        let mut changed = None;
        while let Some(event) = connection.poll_for_event()? {
            if let Event::XfixesSelectionNotify(event) = event { changed = Some(event); }
        }
        if let Some(event) = changed {
            let skip_apps = apps.lock().unwrap_or_else(|error| error.into_inner()).clone();
            // Nothing to record or skip (settings changed meanwhile): no round trips.
            if skip_apps.is_empty() && history.lock().unwrap_or_else(|error| error.into_inner()).disabled() { continue; }
            if connection.get_selection_owner(clipboard)?.reply()?.owner != event.owner { continue; }
            let skip = focused_apps(&connection, root, active).iter().any(|name| skip_apps.iter().any(|app| name.eq_ignore_ascii_case(app)));
            if history.lock().unwrap_or_else(|error| error.into_inner()).disabled() && !skip { continue; }
            if tx.send(Cmd::Capture { skip_history: skip, owner: event.owner, timestamp: event.selection_timestamp }).is_err() { return Ok(()); }
        }
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as libc::nfds_t, -1) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted { continue; }
            return Err(error.into());
        }
        if descriptors.get(1).is_some_and(|descriptor| descriptor.revents != 0) { return Ok(()); }
        if descriptors.first().is_some_and(|descriptor| descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0) { anyhow::bail!("X11 disconnected"); }
    }
}

pub(super) fn focused_apps(connection: &x11rb::rust_connection::RustConnection, root: u32, active: u32) -> Vec<String> {
    let query = || -> anyhow::Result<Vec<String>> {
        let reply = connection.get_property(false, root, active, AtomEnum::WINDOW, 0, 1)?.reply()?;
        let Some(window) = reply.value32().and_then(|mut values| values.next()) else { return Ok(Vec::new()); };
        let reply = connection.get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)?.reply()?;
        Ok(reply.value.split(|byte| *byte == 0).filter_map(|bytes| std::str::from_utf8(bytes).ok().map(str::to_owned)).collect())
    };
    query().unwrap_or_default()
}
