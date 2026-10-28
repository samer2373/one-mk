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

use anyhow::{Context, Result, bail, ensure};
use one_mk_net::{Fingerprint, open_stream, read_file_msg, write_file_msg};
use one_mk_proto::{FileMsg, StreamKind};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read as _, Seek, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;
use tokio::sync::{Notify, OwnedMutexGuard, mpsc};

/// Size of the buffers between disk and network.
const CHUNK: usize = 1 << 20;
/// Buffers in flight per file, each way.
const DEPTH: usize = 4;
/// Folder inside the download folder for partial files.
pub const PARTIAL_DIR: &str = ".one-mk-partial";
/// Parallel streams when sending many files.
const STREAMS: usize = 4;

// ---------------------------------------------------------------- receiving

/// Receives files from trusted peers into one folder.
pub struct Inbox {
    root: PathBuf,
    /// Partial files being written, by id.
    active: Mutex<HashMap<String, Slot>>,
}

#[derive(Default)]
struct Slot {
    lock: Arc<tokio::sync::Mutex<()>>,
    /// Tells the current holder to give up.
    cancel: Arc<Notify>,
}

impl Inbox {
    pub fn new(root: PathBuf) -> Arc<Self> {
        Arc::new(Self { root, active: Mutex::default() })
    }

    /// Serves one file stream until the sender finishes it.
    pub async fn receive(&self, fp: Fingerprint, peer: &str, mut send: quinn::SendStream, mut recv: quinn::RecvStream) {
        let _ = send.set_priority(-1);
        loop {
            let res = match read_file_msg(&mut recv).await {
                Ok(Some(FileMsg::Offer { path, size, mtime })) => {
                    self.receive_file(fp, peer, &mut send, &mut recv, &path, size, mtime).await
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
        size: u64,
        mtime: u64,
    ) -> Result<()> {
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
        let res = self.receive_claimed(peer, send, recv, path, parts, size, &id, &cancel).await;
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
        peer: &str,
        send: &mut quinn::SendStream,
        recv: &mut quinn::RecvStream,
        path: &str,
        parts: Vec<String>,
        size: u64,
        id: &str,
        cancel: &Notify,
    ) -> Result<()> {
        let (root, id) = (self.root.clone(), id.to_owned());
        let opened = tokio::task::spawn_blocking(move || open_part(&root, &id, size)).await?;
        let (file, part, offset) = match opened {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("cannot save {path:?}: {e:#}");
                return write_file_msg(send, &FileMsg::Reject { reason: format!("cannot save: {e}") }).await;
            }
        };
        write_file_msg(send, &FileMsg::Accept { offset }).await?;
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
                    tokio::select! {
                        n = recv.read(&mut buf[filled..]) => match n? {
                            Some(n) => filled += n,
                            None => bail!("stream ended in the middle of {path:?}"),
                        },
                        _ = cancel.notified() => bail!("{path:?} is being sent again; this copy stops"),
                    }
                }
                remaining -= filled as u64;
                if tx.send(buf).await.is_err() {
                    return Ok(false); // the writer failed; it says why
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
                let _ = write_file_msg(send, &FileMsg::Failed { reason: format!("{e:#}") }).await;
                bail!("writing {path:?}: {e:#}");
            }
        };
        let Some(FileMsg::End { hash: theirs }) = read_file_msg(recv).await? else {
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
                tracing::info!(%peer, file = %name, bytes = size, "received file");
                write_file_msg(send, &FileMsg::Saved { name }).await
            }
            Err(e) => {
                tracing::warn!(%peer, ?path, "cannot save: {e:#}");
                write_file_msg(send, &FileMsg::Failed { reason: format!("cannot save: {e}") }).await
            }
        }
    }
}

/// Splits a peer-supplied relative path into safe components, or says why
/// it is refused. Nothing that could leave the download folder survives:
/// no absolute paths, no `..`, no separators or NULs inside names.
pub fn sanitize(path: &str) -> Result<Vec<String>, &'static str> {
    if path.is_empty() || path.len() > FileMsg::MAX_PATH {
        return Err("invalid path length");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err("absolute paths are not allowed");
    }
    let parts: Vec<String> = path.split('/').map(str::to_owned).collect();
    for p in &parts {
        match p.as_str() {
            "" => return Err("empty path component"),
            "." | ".." => return Err("path escapes the download folder"),
            _ => {}
        }
        if p.len() > 255 {
            return Err("name too long");
        }
        if p.chars().any(|c| c == '\\' || c.is_control()) {
            return Err("invalid character in name");
        }
        if cfg!(windows) && !windows_safe(p) {
            return Err("name not allowed on Windows");
        }
    }
    if parts[0] == PARTIAL_DIR {
        return Err("reserved name");
    }
    Ok(parts)
}

fn windows_safe(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or("").trim_end();
    !name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        && !name.ends_with(['.', ' '])
        && !RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem))
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
    format!("{}.part", &hex[..32])
}

/// Creates `dir` unless it exists; refuses symlinks and non-folders.
fn real_dir(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => bail!("{} exists and is not a folder", dir.display()),
        Err(e) if e.kind() == ErrorKind::NotFound => match fs::create_dir(dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => real_dir(dir),
            Err(e) => Err(e).with_context(|| format!("creating {}", dir.display())),
        },
        Err(e) => Err(e.into()),
    }
}

/// Opens (or creates) the partial file. Returns how much of it is usable.
fn open_part(root: &Path, id: &str, size: u64) -> Result<(File, PathBuf, u64)> {
    fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    let staging = root.join(PARTIAL_DIR);
    real_dir(&staging)?;
    let part = staging.join(id);
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
        let dest = dir.join(&candidate);
        let done = match fs::hard_link(part, &dest) {
            Ok(()) => {
                fs::remove_file(part)?;
                true
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => false,
            // Filesystems without hard links (FAT, some network shares).
            Err(_) if fs::symlink_metadata(&dest).is_err() => {
                fs::rename(part, &dest)?;
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

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved { name: String, resumed_from: u64 },
    Rejected(String),
    Failed(String),
}

/// Sends `files` over `conn`, using up to [`STREAMS`] streams at once.
/// Returns the outcome for each file, in the order given.
pub async fn send_files(conn: &quinn::Connection, files: Vec<Outgoing>, progress: Arc<Progress>) -> Vec<(Outgoing, Outcome)> {
    progress.total.store(files.iter().map(|f| f.size).sum(), Ordering::Relaxed);
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
                let res = send_one(&mut send, &mut recv, &file, &progress).await;
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
    progress: &Progress,
) -> Result<Outcome> {
    write_file_msg(send, &FileMsg::Offer { path: file.rel.clone(), size: file.size, mtime: file.mtime }).await?;
    let offset = match read_file_msg(recv).await? {
        Some(FileMsg::Accept { offset }) => offset,
        Some(FileMsg::Reject { reason }) => {
            progress.done.fetch_add(file.size, Ordering::Relaxed);
            progress.skipped.fetch_add(file.size, Ordering::Relaxed);
            return Ok(Outcome::Rejected(reason));
        }
        other => bail!("unexpected reply {other:?}"),
    };
    ensure!(offset <= file.size, "receiver asked for an offset past the end");
    progress.done.fetch_add(offset, Ordering::Relaxed);
    progress.skipped.fetch_add(offset, Ordering::Relaxed);

    // Disk reader (which also hashes) → bounded channel → network.
    let (tx, mut rx) = mpsc::channel::<bytes::Bytes>(DEPTH);
    let (path, size) = (file.path.clone(), file.size);
    let reader = tokio::task::spawn_blocking(move || read_file(&path, size, offset, tx));
    while let Some(chunk) = rx.recv().await {
        let n = chunk.len() as u64;
        send.write_chunk(chunk).await?;
        progress.done.fetch_add(n, Ordering::Relaxed);
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
