//! File transfer between paired devices.
//!
//! Files travel on `StreamKind::File` streams, either on a dedicated
//! connection (what `one-mk send` opens) or on an existing link. Each
//! stream carries a sequence of files (see [`FileMsg`]); the sender uses a
//! few streams in parallel so many small files don't wait on round trips.
//!
//! The receiver writes into a staging folder inside the download folder and
//! only moves a file into place once its BLAKE3 hash matches the sender's.
//! A partial file is kept, and if the same version of the same file from
//! the same device is offered again, only the rest is sent.
//!
//! Disk I/O and hashing run on blocking threads, connected to the network
//! side by small bounded channels of 1 MiB buffers, so the runtime thread
//! only moves memory.

use crate::notify::Notifier;
use crate::track::{FileTicket, Transfers};
use anyhow::{Context, Result, bail, ensure};
use one_mk_net::{Fingerprint, open_stream, read_file_msg, write_file_msg};
use one_mk_proto::{Batch, FileMsg, StreamKind};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read as _, Seek, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, OwnedMutexGuard, mpsc};
use unicode_normalization::UnicodeNormalization as _;

/// Size of the buffers between disk and network.
const CHUNK: usize = 1 << 20;
/// Buffers in flight per file, each way.
const DEPTH: usize = 4;
/// Folder inside the download folder for partial files.
pub const PARTIAL_DIR: &str = ".one-mk-partial";
/// A transfer that receives nothing from the sender for this long is
/// dropped (the partial file is kept).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Parallel streams when sending many files.
const STREAMS: usize = 4;

/// Transfer connections one device may have open at once.
pub const MAX_CONNS_PER_PEER: usize = 2;
/// What a receiver says when [`MAX_CONNS_PER_PEER`] is reached.
const TOO_MANY_CONNS: &str = "too many concurrent transfer connections from this device";

/// Whether `reason` (a file's failure) is a receiver at its connection limit,
/// which passes as soon as an earlier connection is seen closing.
pub fn is_busy(reason: &str) -> bool {
    reason.contains(TOO_MANY_CONNS)
}
/// File streams served at once on one connection.
pub const MAX_STREAMS_PER_CONN: usize = 4;
/// File streams served at once for all devices together.
pub const MAX_STREAMS_TOTAL: usize = 8;
/// Partial files older than this are deleted by the daily sweep.
pub const PARTIAL_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const SWEEP_EVERY: Duration = Duration::from_secs(24 * 3600);
/// How many bytes of unfinished partial files one device may leave in the
/// staging folder. A fixed number is simpler and safer than something
/// derived from the offers (a peer controls those). A single file bigger
/// than this still works: the limit is `max(this, 2 x that file)`.
pub const PEER_PARTIAL_CAP: u64 = 50 << 30;

// ---------------------------------------------------------------- receiving

/// Notifies nobody.
#[cfg(test)]
struct Quiet;

#[cfg(test)]
impl Notifier for Quiet {
    fn files_received(&self, _: crate::notify::FilesReceived) {}
}

/// Receives files from trusted peers into one folder.
pub struct Inbox {
    root: PathBuf,
    /// Partial files being written, by id.
    active: Mutex<HashMap<String, Slot>>,
    limits: Arc<Mutex<Limits>>,
    partial_cap: u64,
    /// Progress of what is arriving, and the "files received" notification.
    transfers: Arc<Transfers>,
}

#[derive(Default)]
struct Limits {
    conns: HashMap<Fingerprint, usize>,
    streams: HashMap<Fingerprint, usize>,
    total: usize,
    /// Bytes still to be written by accepted transfers, all devices.
    reserved_total: u64,
    /// The same, per device.
    reserved: HashMap<Fingerprint, u64>,
}

/// Room set aside for the bytes an accepted file still has to write, so
/// concurrent offers can't all pass the quota and free-space checks against
/// the same empty disk. Shrinks as bytes arrive; whatever is left is
/// released on drop (finished, failed, cancelled or panicked).
struct Reservation {
    limits: Arc<Mutex<Limits>>,
    fp: Fingerprint,
    left: u64,
}

impl Reservation {
    fn consume(&mut self, n: u64) {
        let n = n.min(self.left);
        self.left -= n;
        self.give_back(n);
    }

    fn give_back(&self, n: u64) {
        if n == 0 {
            return;
        }
        let mut l = self.limits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        l.reserved_total = l.reserved_total.saturating_sub(n);
        if let Some(r) = l.reserved.get_mut(&self.fp) {
            *r = r.saturating_sub(n);
            if *r == 0 {
                l.reserved.remove(&self.fp);
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.give_back(self.left);
    }
}

/// Admission of one transfer connection; see [`Inbox::admit`]. Dropping it
/// frees the slot.
pub struct ConnPermit {
    inbox: Arc<Inbox>,
    fp: Fingerprint,
    streams: Arc<AtomicUsize>,
}

impl ConnPermit {
    pub fn fp(&self) -> Fingerprint {
        self.fp
    }
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        let mut l = self.inbox.limits.lock().expect("poisoned");
        if let Some(n) = l.conns.get_mut(&self.fp) {
            *n -= 1;
            if *n == 0 {
                l.conns.remove(&self.fp);
            }
        }
    }
}

/// One file stream being served.
struct StreamGuard<'a> {
    inbox: &'a Inbox,
    fp: Fingerprint,
    conn: Option<Arc<AtomicUsize>>,
}

impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        let mut l = self.inbox.limits.lock().expect("poisoned");
        l.total -= 1;
        if let Some(n) = l.streams.get_mut(&self.fp) {
            *n -= 1;
            if *n == 0 {
                l.streams.remove(&self.fp);
            }
        }
        if let Some(c) = &self.conn {
            c.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct Slot {
    lock: Arc<tokio::sync::Mutex<()>>,
    /// Tells the current holder to give up.
    cancel: Arc<Notify>,
}

impl Inbox {
    /// Also, if called inside a tokio runtime, deletes stale partial files
    /// now and then once a day (see [`Inbox::sweep_partials`]).
    pub fn new(root: PathBuf) -> Arc<Self> {
        // Tests must not pop up notifications on the developer's desktop.
        #[cfg(not(test))]
        let notifier: Arc<dyn Notifier> = Arc::new(crate::notify::Desktop);
        #[cfg(test)]
        let notifier: Arc<dyn Notifier> = Arc::new(Quiet);
        Self::with_notifier(root, notifier)
    }

    /// An inbox that tells `notifier` about each finished transfer.
    pub fn with_notifier(root: PathBuf, notifier: Arc<dyn Notifier>) -> Arc<Self> {
        Self::build(root, PEER_PARTIAL_CAP, notifier)
    }

    /// Progress of transfers in flight, and settings for notifying about them.
    pub fn transfers(&self) -> &Arc<Transfers> {
        &self.transfers
    }

    fn build(root: PathBuf, partial_cap: u64, notifier: Arc<dyn Notifier>) -> Arc<Self> {
        let transfers = Transfers::new(root.clone(), notifier);
        let inbox = Arc::new(Self { root, active: Mutex::default(), limits: Arc::default(), partial_cap, transfers });
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let weak: Weak<Inbox> = Arc::downgrade(&inbox);
            rt.spawn(async move {
                while let Some(inbox) = weak.upgrade() {
                    let _ = tokio::task::spawn_blocking(move || {
                        let n = inbox.sweep_partials(PARTIAL_MAX_AGE);
                        if n > 0 {
                            tracing::info!(removed = n, "deleted stale partial files");
                        }
                    })
                    .await;
                    tokio::time::sleep(SWEEP_EVERY).await;
                }
            });
        }
        inbox
    }

    /// Deletes partial files not modified for `max_age`, except any that is
    /// being written right now. Returns how many were removed.
    pub fn sweep_partials(&self, max_age: Duration) -> usize {
        let Ok(dir) = fs::read_dir(long(&self.root.join(PARTIAL_DIR))) else { return 0 };
        let now = SystemTime::now();
        let mut removed = 0;
        for e in dir.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            let old = meta.is_file()
                && meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= max_age);
            if !old {
                continue;
            }
            // Holding the lock keeps `claim` from taking this file meanwhile.
            let active = self.active.lock().expect("poisoned");
            if !active.contains_key(e.file_name().to_string_lossy().as_ref()) && fs::remove_file(e.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Admits a new transfer connection from `fp`: at most
    /// [`MAX_CONNS_PER_PEER`] at a time. Hold the permit as long as the
    /// connection lives and serve its streams with [`Inbox::receive_on`],
    /// which then also enforces [`MAX_STREAMS_PER_CONN`].
    pub fn admit(self: &Arc<Self>, fp: Fingerprint) -> Result<ConnPermit, &'static str> {
        let mut l = self.limits.lock().expect("poisoned");
        let n = l.conns.entry(fp).or_default();
        if *n >= MAX_CONNS_PER_PEER {
            return Err(TOO_MANY_CONNS);
        }
        *n += 1;
        Ok(ConnPermit { inbox: self.clone(), fp, streams: Arc::default() })
    }

    fn start_stream(&self, fp: Fingerprint, conn: Option<&ConnPermit>) -> Result<StreamGuard<'_>, &'static str> {
        let mut l = self.limits.lock().expect("poisoned");
        if l.total >= MAX_STREAMS_TOTAL {
            return Err("receiver busy: too many file streams in total");
        }
        if let Some(c) = conn
            && c.streams.load(Ordering::SeqCst) >= MAX_STREAMS_PER_CONN
        {
            return Err("too many file streams on this connection");
        }
        // Without connection tracking, a device is held to what its
        // connections could use together.
        if l.streams.get(&fp).copied().unwrap_or(0) >= MAX_CONNS_PER_PEER * MAX_STREAMS_PER_CONN {
            return Err("too many file streams from this device");
        }
        l.total += 1;
        *l.streams.entry(fp).or_default() += 1;
        let conn = conn.map(|c| {
            c.streams.fetch_add(1, Ordering::SeqCst);
            c.streams.clone()
        });
        Ok(StreamGuard { inbox: self, fp, conn })
    }

    /// Serves one file stream of the connection `conn` until the sender
    /// finishes it, within the per-device, per-connection and total limits.
    pub async fn receive_on(
        &self,
        conn: &ConnPermit,
        peer: &str,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
    ) {
        self.receive_inner(Some(conn), conn.fp, peer, send, recv).await
    }

    async fn receive_inner(
        &self,
        conn: Option<&ConnPermit>,
        fp: Fingerprint,
        peer: &str,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) {
        let _guard = match self.start_stream(fp, conn) {
            Ok(g) => g,
            Err(reason) => {
                tracing::warn!(%peer, "refused a file stream: {reason}");
                let _ = write_file_msg(&mut send, &FileMsg::Failed { reason: reason.into() }).await;
                let _ = recv.stop(2u32.into());
                let _ = send.finish();
                return;
            }
        };
        let _ = send.set_priority(-1);
        loop {
            let res = match read_file_msg(&mut recv).await {
                Ok(Some(FileMsg::Offer { path, size, mtime, batch })) => {
                    self.receive_file(fp, peer, &mut send, &mut recv, &path, (size, mtime, batch)).await
                }
                Ok(None) => break,
                Ok(Some(other)) => Err(anyhow::anyhow!("expected a file offer, got {other:?}")),
                Err(e) => Err(e),
            };
            if let Err(e) = res {
                tracing::warn!(%peer, "file transfer stopped: {e:#}");
                let _ = recv.stop(1u32.into());
                return;
            }
        }
        let _ = send.finish();
    }

    #[allow(clippy::too_many_arguments)]
    async fn receive_file(
        &self,
        fp: Fingerprint,
        peer: &str,
        send: &mut quinn::SendStream,
        recv: &mut quinn::RecvStream,
        path: &str,
        (size, mtime, batch): (u64, u64, Batch),
    ) -> Result<()> {
        // Counts toward the transfer's progress and end, also when refused.
        let ticket = match self.transfers.try_begin(fp, peer, batch, size) {
            Ok(ticket) => ticket,
            Err(reason) => return write_file_msg(send, &FileMsg::Reject { reason: reason.into() }).await,
        };
        let parts = match sanitize(path) {
            Ok(p) => p,
            Err(reason) => {
                tracing::warn!(%peer, ?path, "refused a file: {reason}");
                return write_file_msg(send, &FileMsg::Reject { reason: reason.into() }).await;
            }
        };
        // One receiver per partial file. A new offer of the same file (the
        // sender restarted before we noticed the old connection was dead)
        // takes over from the old one.
        let id = part_id(fp, path, size, mtime);
        let (guard, cancel) = self.claim(&id).await;
        let res = self.receive_claimed(fp, peer, send, recv, path, parts, size, &id, &cancel, ticket).await;
        self.release(&id, guard);
        res
    }

    async fn claim(&self, id: &str) -> (OwnedMutexGuard<()>, Arc<Notify>) {
        let (lock, cancel) = {
            let mut active = self.active.lock().expect("poisoned");
            let slot = active.entry(id.to_owned()).or_default();
            (slot.lock.clone(), slot.cancel.clone())
        };
        if lock.try_lock().is_err() {
            cancel.notify_one();
        }
        let guard = lock.lock_owned().await;
        let mine = Arc::new(Notify::new());
        if let Some(slot) = self.active.lock().expect("poisoned").get_mut(id) {
            slot.cancel = mine.clone();
        }
        (guard, mine)
    }

    fn release(&self, id: &str, guard: OwnedMutexGuard<()>) {
        drop(guard);
        let mut active = self.active.lock().expect("poisoned");
        if active.get(id).is_some_and(|s| Arc::strong_count(&s.lock) == 1) {
            active.remove(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn receive_claimed(
        &self,
        fp: Fingerprint,
        peer: &str,
        send: &mut quinn::SendStream,
        recv: &mut quinn::RecvStream,
        path: &str,
        parts: Vec<String>,
        size: u64,
        id: &str,
        cancel: &Notify,
        mut ticket: FileTicket,
    ) -> Result<()> {
        let (root, id, prefix, cap) = (self.root.clone(), id.to_owned(), fp_prefix(fp), self.partial_cap);
        let limits = self.limits.clone();
        let opened = tokio::task::spawn_blocking(move || prepare(&root, &id, &prefix, size, cap, &limits, fp)).await?;
        let (file, part, offset, mut reservation) = match opened {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("cannot save {path:?}: {e:#}");
                return write_file_msg(send, &FileMsg::Reject { reason: format!("cannot save: {e:#}") }).await;
            }
        };
        write_file_msg(send, &FileMsg::Accept { offset }).await?;
        ticket.resumed(offset);
        if offset > 0 {
            tracing::info!(%peer, ?path, offset, "resuming");
        }

        // Network → bounded channel → disk writer (which also hashes). The
        // writer is always awaited, so nobody else touches the partial file
        // until it is done.
        let (tx, rx) = mpsc::channel::<Vec<u8>>(DEPTH);
        let writer = tokio::task::spawn_blocking(move || write_part(file, offset, rx));
        let received = async {
            let mut remaining = size - offset;
            while remaining > 0 {
                let mut buf = vec![0u8; (remaining as usize).min(CHUNK)];
                let mut filled = 0;
                while filled < buf.len() {
                    match guarded(cancel, path, recv.read(&mut buf[filled..])).await?? {
                        Some(n) => filled += n,
                        None => bail!("stream ended in the middle of {path:?}"),
                    }
                }
                remaining -= filled as u64;
                reservation.consume(filled as u64);
                ticket.bytes(filled as u64);
                tokio::select! {
                    sent = tx.send(buf) => if sent.is_err() {
                        return Ok(false); // the writer failed; it says why
                    },
                    _ = cancel.notified() => bail!("{path:?} is being sent again; this copy stops"),
                }
            }
            Ok(true)
        }
        .await;
        drop(tx);
        let written = writer.await?;
        received?;
        let hash = match written {
            Ok(h) => h,
            Err(e) => {
                let reason = describe_write_error(&e);
                let _ = write_file_msg(send, &FileMsg::Failed { reason: reason.clone() }).await;
                // The partial file stays, so the transfer can resume.
                bail!("writing {path:?}: {reason}");
            }
        };
        // The writer is finished, so giving up here is safe; the partial
        // file stays for a resume.
        let Some(FileMsg::End { hash: theirs }) = guarded(cancel, path, read_file_msg(recv)).await?? else {
            bail!("expected the end of {path:?}");
        };
        if hash != theirs {
            tracing::warn!(%peer, ?path, "integrity check failed; discarded");
            let _ = fs::remove_file(&part);
            return write_file_msg(send, &FileMsg::Failed { reason: "integrity check failed".into() }).await;
        }
        let root = self.root.clone();
        let placed = tokio::task::spawn_blocking(move || place(&root, &parts, &part)).await?;
        match placed {
            Ok(name) => {
                println!("Received {name} from {peer} (saved to {}).", self.root.join(&name).display());
                ticket.saved(name.clone());
                write_file_msg(send, &FileMsg::Saved { name }).await
            }
            Err(e) => {
                tracing::warn!(%peer, ?path, "cannot save: {e:#}");
                write_file_msg(send, &FileMsg::Failed { reason: format!("cannot save: {e}") }).await
            }
        }
    }
}

/// Waits for `fut` unless the transfer is replaced by a newer one or the
/// sender goes quiet for [`IDLE_TIMEOUT`].
async fn guarded<T>(cancel: &Notify, path: &str, fut: impl Future<Output = T>) -> Result<T> {
    tokio::select! {
        r = tokio::time::timeout(IDLE_TIMEOUT, fut) => r.map_err(|_| {
            anyhow::anyhow!(
                "no data from the sender for {} s while receiving {path:?}; the partial file is kept",
                IDLE_TIMEOUT.as_secs()
            )
        }),
        _ = cancel.notified() => bail!("{path:?} is being sent again; this copy stops"),
    }
}

/// Splits a peer-supplied relative path into safe components, or says why
/// it is refused. Nothing that could leave the download folder survives:
/// no absolute paths, no `..`, no separators or NULs inside names.
///
/// Names are normalized to Unicode NFC, so a Mac's NFD names don't make
/// look-alike duplicates. Names that are fine here but not on Windows
/// (reserved device names with or without an extension, trailing dots or
/// spaces) are renamed by appending `_` rather than refused, so the
/// result is the same on every platform.
pub fn sanitize(path: &str) -> Result<Vec<String>, &'static str> {
    if path.is_empty() || path.len() > FileMsg::MAX_PATH {
        return Err("invalid path length");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err("absolute paths are not allowed");
    }
    let mut parts = Vec::new();
    for p in path.split('/') {
        match p {
            "" => return Err("empty path component"),
            "." | ".." => return Err("path escapes the download folder"),
            _ => {}
        }
        if p.chars().any(|c| c == '\\' || c.is_control()) {
            return Err("invalid character in name");
        }
        let nfc: String = p.nfc().collect();
        if parts.is_empty() && nfc.trim_end_matches(['.', ' ']).to_lowercase() == PARTIAL_DIR {
            return Err("reserved name");
        }
        let fixed = windows_safe_name(&nfc);
        if fixed.len() > 255 {
            return Err("name too long");
        }
        parts.push(fixed);
    }
    Ok(parts)
}

/// Device names Windows reserves, with any extension.
fn is_reserved(stem: &str) -> bool {
    const RESERVED: [&str; 28] = [
        "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
        "COM7", "COM8", "COM9", "COM\u{b9}", "COM\u{b2}", "COM\u{b3}", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4",
        "LPT5", "LPT6", "LPT7", "LPT8",
    ];
    const MORE: [&str; 4] = ["LPT9", "LPT\u{b9}", "LPT\u{b2}", "LPT\u{b3}"];
    let up = stem.trim_end().to_uppercase();
    RESERVED.contains(&up.as_str()) || MORE.contains(&up.as_str())
}

/// Makes one NFC name valid on Windows too (see [`sanitize`]).
fn windows_safe_name(name: &str) -> String {
    let mut n = name.to_owned();
    if cfg!(windows) {
        n = n.replace(['<', '>', ':', '"', '|', '?', '*'], "_");
    }
    let stem_end = n.find('.').unwrap_or(n.len());
    if is_reserved(&n[..stem_end]) {
        n.insert(stem_end, '_');
    }
    if n.ends_with(['.', ' ']) {
        n.push('_');
    }
    n
}

/// Names the partial file for one version of one file from one device.
pub fn part_id(fp: Fingerprint, path: &str, size: u64, mtime: u64) -> String {
    let mut h = blake3::Hasher::new();
    h.update(&fp.0);
    h.update(&(path.len() as u64).to_le_bytes());
    h.update(path.as_bytes());
    h.update(&size.to_le_bytes());
    h.update(&mtime.to_le_bytes());
    let hex = h.finalize().to_hex();
    format!("{}.{}.part", fp_prefix(fp), &hex[..32])
}

/// The start of every partial file name of one device, so the staging
/// folder can be totalled per device.
fn fp_prefix(fp: Fingerprint) -> String {
    fp.0[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes of other unfinished partial files from the device `prefix`.
fn peer_partial_bytes(staging: &Path, prefix: &str, except: &str) -> u64 {
    let Ok(dir) = fs::read_dir(long(staging)) else { return 0 };
    let start = format!("{prefix}.");
    dir.flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&start) && name != except
        })
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .fold(0u64, u64::saturating_add)
}

fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let (mut v, mut u) = (n as f64, 0);
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

/// Opens the partial file after checking there is room for the rest of
/// it, and that this device has not left too much behind. Both checks count
/// what other accepted transfers still have to write, and on success the
/// rest of this file is reserved (check and reservation are one step under
/// the limits lock).
fn prepare(
    root: &Path,
    id: &str,
    prefix: &str,
    size: u64,
    cap: u64,
    limits: &Arc<Mutex<Limits>>,
    fp: Fingerprint,
) -> Result<(File, PathBuf, u64, Reservation)> {
    let (file, part, offset) = open_part(root, id, size)?;
    let need = size - offset;
    let mut l = limits.lock().expect("poisoned");
    let peer_reserved = l.reserved.get(&fp).copied().unwrap_or(0);
    if let Err(e) = check_room(root, prefix, id, size, offset, cap, l.reserved_total, peer_reserved) {
        drop(l);
        drop(file);
        if offset == 0 {
            let _ = fs::remove_file(&part);
        }
        return Err(e);
    }
    l.reserved_total = l.reserved_total.saturating_add(need);
    let r = l.reserved.entry(fp).or_default();
    *r = r.saturating_add(need);
    drop(l);
    Ok((file, part, offset, Reservation { limits: limits.clone(), fp, left: need }))
}

#[allow(clippy::too_many_arguments)]
fn check_room(
    root: &Path,
    prefix: &str,
    id: &str,
    size: u64,
    offset: u64,
    cap: u64,
    reserved_total: u64,
    peer_reserved: u64,
) -> Result<()> {
    let need = size - offset;
    match fs4::available_space(long(root)) {
        Ok(free) if free < need.saturating_add(reserved_total) => {
            bail!(
                "not enough free disk space: {} needed, {} free ({} already promised to other transfers)",
                human(need),
                human(free),
                human(reserved_total)
            )
        }
        Ok(_) => {}
        Err(e) => tracing::debug!("cannot read free disk space: {e}"),
    }
    let limit = cap.max(size.saturating_mul(2));
    let others = peer_partial_bytes(&root.join(PARTIAL_DIR), prefix, id);
    if others.saturating_add(peer_reserved).saturating_add(size) > limit {
        bail!(
            "too many unfinished transfers from this device ({} kept in {PARTIAL_DIR}, limit {})",
            human(others),
            human(limit)
        );
    }
    Ok(())
}

/// A clear failure message for the sender; the partial file is kept.
fn describe_write_error(e: &anyhow::Error) -> String {
    let full = e.chain().filter_map(|c| c.downcast_ref::<std::io::Error>()).any(|io| {
        io.kind() == ErrorKind::StorageFull || io.kind() == ErrorKind::QuotaExceeded || io.raw_os_error() == Some(28)
    });
    if full {
        "disk full on the receiving device; the partial file is kept, send again to resume".into()
    } else {
        format!("write error on the receiving device ({e:#}); the partial file is kept, send again to resume")
    }
}

/// Windows refuses paths over 260 characters unless they carry the `\\?\`
/// prefix (which also turns off `/` and `..` handling, so the path is
/// cleaned first). Elsewhere paths are used as they are.
#[cfg(windows)]
fn long(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    match abs.to_str() {
        Some(s) => PathBuf::from(long_path_string(s)),
        None => abs,
    }
}

#[cfg(not(windows))]
fn long(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// The text transformation behind [`long`], testable everywhere.
#[allow(dead_code)]
fn long_path_string(s: &str) -> String {
    const MIN: usize = 200;
    if s.starts_with(r"\\?\") || s.len() < MIN {
        return s.to_owned();
    }
    let s = s.replace('/', "\\");
    if let Some(unc) = s.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else if s.as_bytes().get(1) == Some(&b':') {
        format!(r"\\?\{s}")
    } else {
        s
    }
}

/// Creates `dir` unless it exists; refuses symlinks and non-folders.
fn real_dir(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(long(dir)) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => bail!("{} exists and is not a folder", dir.display()),
        Err(e) if e.kind() == ErrorKind::NotFound => match fs::create_dir(long(dir)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => real_dir(dir),
            Err(e) => Err(e).with_context(|| format!("creating {}", dir.display())),
        },
        Err(e) => Err(e.into()),
    }
}

/// Opens (or creates) the partial file. Returns how much of it is usable.
fn open_part(root: &Path, id: &str, size: u64) -> Result<(File, PathBuf, u64)> {
    fs::create_dir_all(long(root)).with_context(|| format!("creating {}", root.display()))?;
    let staging = root.join(PARTIAL_DIR);
    real_dir(&staging)?;
    let part = long(&staging.join(id));
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let file = opts.open(&part).with_context(|| format!("opening {}", part.display()))?;
    ensure!(file.metadata()?.is_file(), "{} is not a file", part.display());
    let mut len = file.metadata()?.len();
    if len > size {
        file.set_len(0)?;
        len = 0;
    }
    Ok((file, part, len))
}

/// Hashes the existing prefix, then appends and hashes what arrives.
fn write_part(mut file: File, offset: u64, mut rx: mpsc::Receiver<Vec<u8>>) -> Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    if offset > 0 {
        file.seek(SeekFrom::Start(0))?;
        let mut prefix = (&mut file).take(offset);
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = prefix.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    file.seek(SeekFrom::Start(offset))?;
    file.set_len(offset)?;
    while let Some(buf) = rx.blocking_recv() {
        hasher.update(&buf);
        file.write_all(&buf)?;
    }
    Ok(hasher.finalize().into())
}

/// Moves a verified file to its place, creating folders and choosing a free
/// name ("name (1).ext") if one is taken. Never overwrites, and never
/// follows symlinks: hard-linking fails if the target name exists.
fn place(root: &Path, parts: &[String], part: &Path) -> Result<String> {
    let (name, dirs) = parts.split_last().context("empty path")?;
    let mut dir = root.to_path_buf();
    for d in dirs {
        dir.push(d);
        real_dir(&dir)?;
    }
    for n in 0..10_000 {
        let candidate = if n == 0 { name.clone() } else { numbered(name, n) };
        let dest = long(&dir.join(&candidate));
        let done = match fs::hard_link(long(part), &dest) {
            Ok(()) => {
                fs::remove_file(long(part))?;
                true
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => false,
            // Filesystems without hard links (FAT, some network shares).
            Err(_) if fs::symlink_metadata(&dest).is_err() => {
                fs::rename(long(part), &dest)?;
                true
            }
            Err(_) => false,
        };
        if done {
            let mut rel: Vec<&str> = dirs.iter().map(String::as_str).collect();
            rel.push(&candidate);
            return Ok(rel.join("/"));
        }
    }
    bail!("no free name for {name}")
}

/// "photo.jpg" → "photo (n).jpg"; ".bashrc" → ".bashrc (n)".
fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

// ------------------------------------------------------------------ sending

/// A file to send.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub path: PathBuf,
    /// Name on the receiver, relative and `/`-separated.
    pub rel: String,
    pub size: u64,
    pub mtime: u64,
}

/// Lists the files to send: plain files as themselves, folders
/// recursively under their own name. Symlinks inside folders are skipped.
pub fn collect(paths: &[PathBuf]) -> Result<Vec<Outgoing>> {
    let mut out = Vec::new();
    for p in paths {
        let meta = fs::metadata(p).with_context(|| format!("{}", p.display()))?;
        let name = match p.file_name() {
            Some(n) => n.to_owned(),
            None => fs::canonicalize(p)?.file_name().context("cannot send the root folder")?.to_owned(),
        };
        let name = name.to_string_lossy().into_owned();
        if meta.is_dir() {
            walk(p, &name, &mut out)?;
        } else if meta.is_file() {
            out.push(outgoing(p.clone(), name, &meta));
        } else {
            bail!("{} is not a file or folder", p.display());
        }
    }
    Ok(out)
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<Outgoing>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let ft = e.file_type()?;
        let rel = format!("{rel}/{}", e.file_name().to_string_lossy());
        if ft.is_dir() {
            walk(&e.path(), &rel, out)?;
        } else if ft.is_file() {
            out.push(outgoing(e.path(), rel, &e.metadata()?));
        } else {
            tracing::warn!("skipping {} (not a regular file)", e.path().display());
        }
    }
    Ok(())
}

fn outgoing(path: PathBuf, rel: String, meta: &fs::Metadata) -> Outgoing {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    Outgoing { path, rel, size: meta.len(), mtime }
}

/// Live counters for a progress display.
#[derive(Default)]
pub struct Progress {
    pub total: AtomicU64,
    /// Bytes done, including ones the receiver already had.
    pub done: AtomicU64,
    /// Bytes skipped because the receiver already had them.
    pub skipped: AtomicU64,
}

fn add_progress(counter: &AtomicU64, amount: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while let Err(actual) = counter.compare_exchange_weak(current, current.saturating_add(amount), Ordering::Relaxed, Ordering::Relaxed) {
        current = actual;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved { name: String, resumed_from: u64 },
    Rejected(String),
    Failed(String),
}

/// Sends `files` over `conn`, using up to [`STREAMS`] streams at once.
/// Returns the outcome for each file, in the order given.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn send_files(conn: &quinn::Connection, files: Vec<Outgoing>, progress: Arc<Progress>) -> Vec<(Outgoing, Outcome)> {
    send_files_as(conn, files, progress, new_batch_id()).await
}

/// A random id for one send; never 0, which means "unknown".
pub fn new_batch_id() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    // std seeds RandomState from the OS and changes its keys on every call.
    RandomState::new().hash_one(0u8).max(1)
}

/// [`send_files`] with the batch id chosen by the caller, who may want to
/// show it (as the id of the transfer) too.
pub async fn send_files_as(
    conn: &quinn::Connection,
    files: Vec<Outgoing>,
    progress: Arc<Progress>,
    id: u64,
) -> Vec<(Outgoing, Outcome)> {
    let bytes = files.iter().map(|file| file.size).fold(0u64, u64::saturating_add);
    if files.len() > 10_000 || bytes > 1 << 40 {
        return files.into_iter().map(|file| (file, Outcome::Rejected("file batch exceeds the limit of 10000 files or 1 TiB".into()))).collect();
    }
    progress.total.store(bytes, Ordering::Relaxed);
    let batch = Batch { id, files: u32::try_from(files.len()).unwrap_or(u32::MAX), bytes };
    let n = files.len();
    let queue = Arc::new(Mutex::new(files.into_iter().enumerate().collect::<VecDeque<_>>()));
    let results = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..STREAMS.min(n) {
        let (conn, queue, results, progress) = (conn.clone(), queue.clone(), results.clone(), progress.clone());
        workers.spawn(async move {
            let (mut send, mut recv) = match open_stream(&conn, StreamKind::File).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("opening a file stream: {e:#}");
                    return;
                }
            };
            let _ = send.set_priority(-1);
            loop {
                let Some((i, file)) = queue.lock().expect("poisoned").pop_front() else { break };
                let res = send_one(&mut send, &mut recv, &file, batch, &progress).await;
                let broken = res.is_err();
                let outcome = res.unwrap_or_else(|e| Outcome::Failed(format!("{e:#}")));
                results.lock().expect("poisoned").push((i, file, outcome));
                if broken {
                    return;
                }
            }
            let _ = send.finish();
            // Wait until the receiver has seen the end of the stream.
            let _ = read_file_msg(&mut recv).await;
        });
    }
    while workers.join_next().await.is_some() {}
    let mut results = std::mem::take(&mut *results.lock().expect("poisoned"));
    for (i, file) in queue.lock().expect("poisoned").drain(..) {
        results.push((i, file, Outcome::Failed("connection lost".into())));
    }
    results.sort_by_key(|(i, ..)| *i);
    results.into_iter().map(|(_, f, o)| (f, o)).collect()
}

async fn send_one(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    file: &Outgoing,
    batch: Batch,
    progress: &Progress,
) -> Result<Outcome> {
    write_file_msg(send, &FileMsg::Offer { path: file.rel.clone(), size: file.size, mtime: file.mtime, batch }).await?;
    let offset = match read_file_msg(recv).await? {
        Some(FileMsg::Accept { offset }) => offset,
        Some(FileMsg::Reject { reason }) => {
            add_progress(&progress.done, file.size);
            add_progress(&progress.skipped, file.size);
            return Ok(Outcome::Rejected(reason));
        }
        Some(FileMsg::Failed { reason }) => bail!("receiver: {reason}"),
        other => bail!("unexpected reply {other:?}"),
    };
    ensure!(offset <= file.size, "receiver asked for an offset past the end");
    add_progress(&progress.done, offset);
    add_progress(&progress.skipped, offset);

    // Disk reader (which also hashes) → bounded channel → network.
    let (tx, mut rx) = mpsc::channel::<bytes::Bytes>(DEPTH);
    let (path, size) = (file.path.clone(), file.size);
    let reader = tokio::task::spawn_blocking(move || read_file(&path, size, offset, tx));
    while let Some(chunk) = rx.recv().await {
        let n = chunk.len() as u64;
        if let Err(e) = send.write_chunk(chunk).await {
            drop(rx);
            let _ = reader.await;
            // The receiver may have said why it stopped (disk full, ...).
            let said = tokio::time::timeout(Duration::from_secs(2), read_file_msg(recv)).await;
            if let Ok(Ok(Some(FileMsg::Failed { reason }))) = said {
                bail!("receiver: {reason}");
            }
            return Err(e.into());
        }
        add_progress(&progress.done, n);
    }
    let hash = reader.await??;
    write_file_msg(send, &FileMsg::End { hash }).await?;
    match read_file_msg(recv).await? {
        Some(FileMsg::Saved { name }) => Ok(Outcome::Saved { name, resumed_from: offset }),
        Some(FileMsg::Failed { reason }) => Ok(Outcome::Failed(reason)),
        other => bail!("unexpected reply {other:?}"),
    }
}

/// Hashes the whole file and sends the part from `offset` on.
fn read_file(path: &Path, size: u64, offset: u64, tx: mpsc::Sender<bytes::Bytes>) -> Result<[u8; 32]> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut pos = 0u64;
    while pos < size {
        let mut buf = vec![0u8; ((size - pos) as usize).min(CHUNK)];
        file.read_exact(&mut buf).with_context(|| format!("{} changed while sending", path.display()))?;
        hasher.update(&buf);
        let start = offset.saturating_sub(pos).min(buf.len() as u64) as usize;
        pos += buf.len() as u64;
        if start < buf.len() {
            let mut chunk = bytes::Bytes::from(buf);
            if start > 0 {
                chunk = chunk.slice(start..);
            }
            if tx.blocking_send(chunk).is_err() {
                bail!("transfer stopped");
            }
        }
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_escapes() {
        assert_eq!(sanitize("a/b.txt").unwrap(), vec!["a", "b.txt"]);
        assert_eq!(sanitize("..data/x..y").unwrap(), vec!["..data", "x..y"]);
        for bad in [
            "", "/etc/passwd", "\\\\server\\x", "../evil", "a/../../evil", "a/./b", "a//b", "a/", ".", "..",
            "a\\..\\b", "nul\0byte", "new\nline", ".one-mk-partial/x.part",
        ] {
            assert!(sanitize(bad).is_err(), "{bad:?} accepted");
        }
        assert!(sanitize(&"x".repeat(256)).is_err());
    }

    #[test]
    fn sanitize_windows_and_unicode() {
        let one = |p: &str| sanitize(p).unwrap().join("/");
        // Reserved names, any case, with or without extension.
        assert_eq!(one("aux.txt"), "aux_.txt");
        assert_eq!(one("Nul"), "Nul_");
        assert_eq!(one("dir/CONIN$"), "dir/CONIN$_");
        assert_eq!(one("conout$.log"), "conout$_.log");
        assert_eq!(one("com0"), "com0_");
        assert_eq!(one("LPT0.x"), "LPT0_.x");
        for n in ["COM\u{b9}", "com\u{b2}.txt", "COM\u{b3}", "LPT\u{b9}", "lpt\u{b2}", "LPT\u{b3}.a", "Com1 .txt"] {
            assert!(one(n).contains('_'), "{n} not renamed");
        }
        assert_eq!(one("auxiliary.txt"), "auxiliary.txt");
        assert_eq!(one("COM10"), "COM10");
        // Trailing dots and spaces.
        assert_eq!(one("report."), "report._");
        assert_eq!(one("a/b "), "a/b _");
        assert_eq!(one("x..y"), "x..y");
        // The staging folder, whatever the case or decoration.
        for bad in [".ONE-MK-PARTIAL/x", ".One-Mk-Partial", ".one-mk-partial./x", ".one-mk-partial /x"] {
            assert!(sanitize(bad).is_err(), "{bad:?} accepted");
        }
        assert!(sanitize("a/.ONE-MK-PARTIAL").is_ok());
        // NFD (Mac) and NFC spellings give one name.
        let nfd = "Cafe\u{301}/u\u{308}.txt";
        assert_eq!(sanitize(nfd).unwrap(), sanitize("Caf\u{e9}/\u{fc}.txt").unwrap());
        assert_eq!(one(nfd), "Caf\u{e9}/\u{fc}.txt");
    }

    #[test]
    fn long_paths_get_the_extended_prefix() {
        let deep = format!(r"C:\dl\{}", "x".repeat(250));
        assert_eq!(long_path_string(&deep), format!(r"\\?\{deep}"));
        let unc = format!(r"\\srv\share\{}", "x".repeat(250));
        assert_eq!(long_path_string(&unc), format!(r"\\?\UNC\srv\share\{}", "x".repeat(250)));
        let mixed = format!("C:\\dl/{}", "y".repeat(250));
        assert!(long_path_string(&mixed).starts_with(r"\\?\C:\dl\y"));
        assert!(!long_path_string(&mixed).contains('/'));
        assert_eq!(long_path_string(r"C:\short"), r"C:\short");
        let done = format!(r"\\?\{deep}");
        assert_eq!(long_path_string(&done), done);
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn refuses_when_disk_is_too_small() {
        let root = temp("space");
        let fp = Fingerprint([7; 32]);
        let id = part_id(fp, "big", u64::MAX / 2, 1);
        let lim = Arc::<Mutex<Limits>>::default();
        let err = prepare(&root, &id, &fp_prefix(fp), u64::MAX / 2, PEER_PARTIAL_CAP, &lim, fp).err().unwrap();
        assert!(format!("{err:#}").contains("not enough free disk space"), "{err:#}");
        // The empty partial file was not left behind.
        assert!(!root.join(PARTIAL_DIR).join(&id).exists());
        // Something small fits.
        let id = part_id(fp, "small", 10, 1);
        assert!(prepare(&root, &id, &fp_prefix(fp), 10, PEER_PARTIAL_CAP, &lim, fp).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn full_disk_error_is_explained() {
        let e = anyhow::Error::new(std::io::Error::from(ErrorKind::StorageFull)).context("writing");
        let msg = describe_write_error(&e);
        assert!(msg.contains("disk full") && msg.contains("resume"), "{msg}");
        let e = anyhow::Error::new(std::io::Error::from_raw_os_error(28));
        assert!(describe_write_error(&e).contains("disk full"));
        let e = anyhow::Error::new(std::io::Error::from(ErrorKind::PermissionDenied));
        assert!(describe_write_error(&e).contains("write error"));
        // A write to /dev/full fails with ENOSPC and is reported as such.
        #[cfg(target_os = "linux")]
        {
            let f = OpenOptions::new().write(true).open("/dev/full").unwrap();
            let mut f = f;
            let r: Result<()> = f.write_all(&[1, 2, 3]).map_err(Into::into);
            assert!(describe_write_error(&r.unwrap_err()).contains("disk full"));
        }
    }

    #[test]
    fn staging_is_capped_per_peer() {
        let root = temp("cap");
        let (a, b) = (Fingerprint([1; 32]), Fingerprint([2; 32]));
        let staging = root.join(PARTIAL_DIR);
        fs::create_dir_all(&staging).unwrap();
        // Device A already keeps 600 bytes; B's files don't count against A.
        fs::write(staging.join(part_id(a, "old1", 400, 1)), vec![0; 400]).unwrap();
        fs::write(staging.join(part_id(a, "old2", 200, 1)), vec![0; 200]).unwrap();
        fs::write(staging.join(part_id(b, "other", 900, 1)), vec![0; 900]).unwrap();
        let pa = fp_prefix(a);
        let id = part_id(a, "new", 300, 1);
        // 600 + 300 <= 1000
        let lim = Arc::<Mutex<Limits>>::default();
        assert!(prepare(&root, &id, &pa, 300, 1000, &lim, a).is_ok());
        let id2 = part_id(a, "new2", 500, 1);
        // 600 + 500 > 1000, and 2 x 500 does not save it either.
        let err = prepare(&root, &id2, &pa, 500, 1000, &lim, a).err().unwrap();
        assert!(format!("{err:#}").contains("too many unfinished"), "{err:#}");
        // A file bigger than the cap is still allowed on its own: 2 x size.
        let big = part_id(b, "big", 5000, 1);
        assert!(prepare(&root, &big, &fp_prefix(b), 5000, 1000, &lim, b).is_ok()); // 900 + 5000 <= 2 x 5000
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_offers_cannot_exceed_the_quota() {
        let root = temp("reserve");
        let fp = Fingerprint([9; 32]);
        let lim = Arc::<Mutex<Limits>>::default();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (root, lim, barrier) = (root.clone(), lim.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let id = part_id(fp, &format!("f{i}"), 400, 1);
                    barrier.wait();
                    prepare(&root, &id, &fp_prefix(fp), 400, 1000, &lim, fp)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        // 1000 holds two 400-byte files; the other six are refused.
        let ok: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
        assert_eq!(ok.len(), 2);
        assert_eq!(lim.lock().unwrap().reserved_total, 800);
        assert_eq!(lim.lock().unwrap().reserved.get(&fp), Some(&800));
        // Reservations shrink as bytes arrive and vanish when dropped.
        let mut it = ok.into_iter();
        let (.., mut first) = it.next().unwrap();
        first.consume(300);
        assert_eq!(lim.lock().unwrap().reserved_total, 500);
        drop(first);
        assert_eq!(lim.lock().unwrap().reserved_total, 400);
        drop(it);
        let l = lim.lock().unwrap();
        assert_eq!(l.reserved_total, 0);
        assert!(l.reserved.is_empty());
        drop(l);
        // Refused offers left no empty partial files; room is free again.
        let id = part_id(fp, "again", 400, 1);
        assert!(prepare(&root, &id, &fp_prefix(fp), 400, 1000, &lim, fp).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn replacement_takes_over_a_stalled_transfer() {
        use one_mk_net::{Identity, Node};
        let node = || Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (a, b) = (node(), node());
        let addr = b.local_addr().unwrap();
        let (pa, pb) = tokio::join!(a.connect(addr), async { b.accept().await.unwrap() });
        let (pa, pb) = (pa.unwrap(), pb.unwrap());

        let root = temp("stall");
        let inbox = Inbox::new(root.clone());
        let fp = pb.fingerprint;
        let offer = FileMsg::Offer { path: "f.bin".into(), size: 10, mtime: 1, batch: Batch::default() };
        let serve = |inbox: Arc<Inbox>, send, recv| tokio::spawn(async move { inbox.receive_inner(None, fp, "t", send, recv).await });
        let secs = |f| tokio::time::timeout(Duration::from_secs(5), f);

        // First stream: all bytes, never End.
        let (mut s1, mut r1) = open_stream(&pa.conn, StreamKind::File).await.unwrap();
        let (_, ps1, pr1) = pb.accept_stream().await.unwrap();
        let t1 = serve(inbox.clone(), ps1, pr1);
        write_file_msg(&mut s1, &offer).await.unwrap();
        assert!(matches!(secs(read_file_msg(&mut r1)).await.unwrap().unwrap(), Some(FileMsg::Accept { offset: 0 })));
        s1.write_all(&[7; 10]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!t1.is_finished());

        // Second stream offers the same file: it is accepted promptly and
        // the stalled stream ends.
        let (mut s2, mut r2) = open_stream(&pa.conn, StreamKind::File).await.unwrap();
        let (_, ps2, pr2) = pb.accept_stream().await.unwrap();
        let t2 = serve(inbox.clone(), ps2, pr2);
        write_file_msg(&mut s2, &offer).await.unwrap();
        let got = secs(read_file_msg(&mut r2)).await.expect("replacement hung").unwrap();
        assert!(matches!(got, Some(FileMsg::Accept { offset: 10 })), "{got:?}");
        tokio::time::timeout(Duration::from_secs(5), t1).await.expect("stalled stream did not end").unwrap();
        assert!(root.join(PARTIAL_DIR).join(part_id(fp, "f.bin", 10, 1)).exists(), "partial kept");
        // Its reservation is gone; the replacement holds none (nothing left).
        assert_eq!(inbox.limits.lock().unwrap().reserved_total, 0);
        t2.abort();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_keeps_fresh_and_active_partials() {
        let root = temp("sweep");
        let staging = root.join(PARTIAL_DIR);
        fs::create_dir_all(&staging).unwrap();
        let inbox = Inbox::new(root.clone());
        for n in ["stale", "active", "fresh"] {
            fs::write(staging.join(n), "x").unwrap();
        }
        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        for n in ["stale", "active"] {
            File::options().write(true).open(staging.join(n)).unwrap().set_modified(old).unwrap();
        }
        inbox.active.lock().unwrap().insert("active".into(), Slot::default());
        assert_eq!(inbox.sweep_partials(PARTIAL_MAX_AGE), 1);
        assert!(!staging.join("stale").exists());
        assert!(staging.join("active").exists() && staging.join("fresh").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stream_and_connection_limits() {
        let inbox = Inbox::new(temp("limits"));
        let fp = Fingerprint([3; 32]);
        let c1 = inbox.admit(fp).unwrap();
        let c2 = inbox.admit(fp).unwrap();
        assert!(inbox.admit(fp).is_err());
        // Another device has its own connections.
        let other = inbox.admit(Fingerprint([4; 32])).unwrap();
        drop(c2);
        let c2 = inbox.admit(fp).unwrap();

        // 4 streams per connection.
        let mut g: Vec<_> = (0..MAX_STREAMS_PER_CONN).map(|_| inbox.start_stream(fp, Some(&c1)).unwrap()).collect();
        assert!(inbox.start_stream(fp, Some(&c1)).is_err());
        g.pop();
        g.push(inbox.start_stream(fp, Some(&c1)).unwrap());
        // c2 can fill up too (8 for this device), then the device is at its limit.
        g.extend((0..MAX_STREAMS_PER_CONN).map(|_| inbox.start_stream(fp, Some(&c2)).unwrap()));
        assert!(inbox.start_stream(fp, None).is_err());
        // 8 in total across devices: the device above holds all 8.
        assert!(inbox.start_stream(Fingerprint([4; 32]), Some(&other)).is_err());
        g.clear();
        assert!(inbox.start_stream(Fingerprint([4; 32]), Some(&other)).is_ok());
        assert_eq!(inbox.limits.lock().unwrap().total, 0);
    }

    /// Two connected nodes: the sender's connection and the receiving side's.
    async fn linked() -> (one_mk_net::Peer, one_mk_net::Peer, one_mk_net::Node, one_mk_net::Node) {
        use one_mk_net::{Identity, Node};
        let node = || Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (a, b) = (node(), node());
        let addr = b.local_addr().unwrap();
        let (pa, pb) = tokio::join!(a.connect(addr), async { b.accept().await.unwrap() });
        (pa.unwrap(), pb.unwrap(), a, b)
    }

    async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// A folder sent with `send_files` is one transfer on the receiving
    /// side: one notification, naming what arrived, once the last file is in.
    #[tokio::test]
    async fn a_received_folder_raises_one_notification() {
        use crate::track::tests::Recorder;
        let (pa, pb, _a, _b) = linked().await;
        let src = temp("notify-src");
        for (rel, len) in [("photos/a.jpg", 1000usize), ("photos/sub/b.jpg", 2000), ("photos/c.jpg", 3000), ("notes.txt", 10)] {
            let path = src.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![7u8; len]).unwrap();
        }
        let files = collect(&[src.join("photos"), src.join("notes.txt")]).unwrap();
        assert_eq!(files.len(), 4);

        let root = temp("notify-dst");
        let rec = Arc::new(Recorder::default());
        let inbox = Inbox::with_notifier(root.clone(), rec.clone());
        let fp = pb.fingerprint;
        let conn = pb.conn.clone();
        let server = {
            let inbox = inbox.clone();
            tokio::spawn(async move {
                let permit = Arc::new(inbox.admit(fp).unwrap());
                while let Ok((send, mut recv)) = conn.accept_bi().await {
                    let (inbox, permit) = (inbox.clone(), permit.clone());
                    tokio::spawn(async move {
                        let _ = one_mk_net::read_stream_kind(&mut recv).await;
                        inbox.receive_on(&permit, "laptop", send, recv).await;
                    });
                }
            })
        };

        let results = send_files(&pa.conn, files, Arc::new(Progress::default())).await;
        assert!(results.iter().all(|(_, o)| matches!(o, Outcome::Saved { .. })), "{results:?}");

        let got = rec.all();
        assert_eq!(got.len(), 1, "one transfer, one notification: {got:?}");
        let n = &got[0];
        assert_eq!(n.peer, "laptop");
        assert_eq!(n.bytes, 6010);
        assert_eq!(n.folder, root);
        let mut names = n.names.clone();
        names.sort();
        assert_eq!(names, ["notes.txt", "photos/a.jpg", "photos/c.jpg", "photos/sub/b.jpg"]);
        assert_eq!(n.title(), "4 files received from laptop");
        assert_eq!(n.reveal(), root, "a folder and a file: the download folder");
        assert!(inbox.transfers().snapshot().is_empty(), "nothing is in flight any more");

        // Sending again is a new transfer: a new notification (the files get new names).
        let files = collect(&[src.join("notes.txt")]).unwrap();
        send_files(&pa.conn, files, Arc::new(Progress::default())).await;
        let got = rec.all();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].names, ["notes (1).txt"]);
        assert_eq!(got[1].title(), "notes (1).txt received from laptop");

        // The setting turns it off at once.
        inbox.transfers().set_notify(false);
        let files = collect(&[src.join("notes.txt")]).unwrap();
        send_files(&pa.conn, files, Arc::new(Progress::default())).await;
        assert_eq!(rec.all().len(), 2);
        server.abort();
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&root);
    }

    /// The receiving side reports bytes done and the whole send's total
    /// while a transfer is in flight, across files and streams.
    #[tokio::test]
    async fn progress_of_a_transfer_in_flight_is_visible() {
        use crate::track::tests::Recorder;
        let (pa, pb, _a, _b) = linked().await;
        let root = temp("progress");
        let rec = Arc::new(Recorder::default());
        let inbox = Inbox::with_notifier(root.clone(), rec.clone());
        let fp = pb.fingerprint;
        // Bytes are counted a buffer (CHUNK) at a time.
        let size = 2 * CHUNK as u64;
        let batch = Batch { id: 42, files: 2, bytes: 2 * size };

        let (mut s, mut r) = open_stream(&pa.conn, StreamKind::File).await.unwrap();
        let (_, ps, pr) = pb.accept_stream().await.unwrap();
        let inbox2 = inbox.clone();
        let task = tokio::spawn(async move { inbox2.receive_inner(None, fp, "desk", ps, pr).await });

        let data: Vec<u8> = (0..4 * CHUNK).map(|i| (i % 251) as u8).collect();
        for (i, name) in ["one.bin", "two.bin"].into_iter().enumerate() {
            let body = &data[i * 2 * CHUNK..(i + 1) * 2 * CHUNK];
            let offer = FileMsg::Offer { path: name.into(), size, mtime: 1, batch };
            write_file_msg(&mut s, &offer).await.unwrap();
            assert!(matches!(tokio::time::timeout(Duration::from_secs(5), read_file_msg(&mut r)).await.unwrap().unwrap(), Some(FileMsg::Accept { offset: 0 })));
            s.write_all(&body[..CHUNK]).await.unwrap();
            // Half-way into the file, so half or three quarters of the whole send.
            let want = (i as u64) * size + CHUNK as u64;
            eventually("the bytes to show", || inbox.transfers().snapshot().first().is_some_and(|t| t.done == want)).await;
            let t = inbox.transfers().snapshot().remove(0);
            assert_eq!((t.id, t.sending, t.peer.as_str(), t.files, t.total), (42, false, "desk", 2, 2 * size), "{t:?}");
            assert_eq!(inbox.transfers().snapshot().len(), 1, "one line for the whole send");

            s.write_all(&body[CHUNK..]).await.unwrap();
            write_file_msg(&mut s, &FileMsg::End { hash: blake3::hash(body).into() }).await.unwrap();
            assert!(matches!(tokio::time::timeout(Duration::from_secs(5), read_file_msg(&mut r)).await.unwrap().unwrap(), Some(FileMsg::Saved { .. })));
            if i == 0 {
                assert!(rec.all().is_empty(), "the second file is still to come");
            }
        }
        // The last file of the batch ends it.
        eventually("the notification", || !rec.all().is_empty()).await;
        assert_eq!(rec.all().len(), 1);
        assert_eq!(rec.all()[0].names, ["one.bin", "two.bin"]);
        assert!(inbox.transfers().snapshot().is_empty());
        s.finish().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered("photo.jpg", 1), "photo (1).jpg");
        assert_eq!(numbered("README", 2), "README (2)");
        assert_eq!(numbered(".bashrc", 1), ".bashrc (1)");
    }

    #[cfg(unix)]
    #[test]
    fn place_never_follows_symlinks() {
        let root = std::env::temp_dir().join(format!("one-mk-place-{}", std::process::id()));
        let outside = root.with_extension("outside");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("target"), root.join("dangling")).unwrap();

        let part = root.join("p.part");
        fs::write(&part, "x").unwrap();
        assert!(place(&root, &["link".into(), "f".into()], &part).is_err());
        assert_eq!(place(&root, &["dangling".into()], &part).unwrap(), "dangling (1)");
        assert!(!outside.join("target").exists());
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }
}
