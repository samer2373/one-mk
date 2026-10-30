//! System clipboard access for one-mk, kept off the async runtime.
//!
//! All platform calls happen on one dedicated thread ([`Clipboard::system`]);
//! the async API only sends it requests. That thread also owns the
//! clipboard contents we set, which on X11 must be served by a live process.
//!
//! Platforms go through [`arboard`]: Win32 on Windows, NSPasteboard on
//! macOS, and on Linux the Wayland data-control protocol where the
//! compositor offers it (KDE, wlroots), otherwise X11.
//!
//! **GNOME on Wayland** (tested on GNOME Shell 50.1, Ubuntu 26.04): Mutter
//! offers neither `zwlr_data_control_manager_v1` nor
//! `ext_data_control_manager_v1`, so a background process cannot use the
//! Wayland clipboard directly (arboard's Wayland backend fails to
//! initialise). It falls back to X11 through XWayland, and that works:
//! Mutter bridges the X11 CLIPBOARD selection and the Wayland clipboard
//! both ways, without the X11 client needing focus. Checked with separate
//! processes: text, HTML and PNG copied with `wl-copy` are read back, and
//! text we set is what `wl-paste` (a native Wayland client) sees, also
//! after our process exits (Mutter keeps a copy). This needs XWayland,
//! which GNOME starts on demand when `DISPLAY` is set.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::oneshot;

pub mod history;
mod history_save;
#[cfg(target_os = "linux")]
mod history_watch;
mod sensitivity;

/// Default for the largest clipboard content we send or accept, in bytes
/// (all formats together).
pub const DEFAULT_MAX_BYTES: usize = 25 << 20;
/// The most [`Filter::max_bytes`] can be set to: received data is held in
/// memory, so the limit is bounded whatever the settings say.
pub const CEILING_BYTES: usize = 256 << 20;
/// Largest decoded image we handle (RGBA bytes), about a 16K x 4K screen.
const MAX_RGBA: usize = 256 << 20;

/// BLAKE3 of clipboard content as this machine reads it.
pub type Digest = [u8; 32];

/// Clipboard content in the formats we carry over the network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipData {
    pub text: Option<String>,
    pub html: Option<String>,
    /// An image, PNG encoded.
    pub png: Option<Vec<u8>>,
}

impl ClipData {
    pub fn len(&self) -> usize {
        self.text.as_ref().map_or(0, String::len)
            + self.html.as_ref().map_or(0, String::len)
            + self.png.as_ref().map_or(0, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none() && self.png.is_none()
    }
}

/// What the user lets us share, applied when reading the clipboard (and
/// by the caller to what arrives). It can be changed while running with
/// [`Clipboard::set_filter`]; a change makes the clipboard look "changed",
/// since the digests are over what passes the filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub text: bool,
    pub html: bool,
    pub image: bool,
    /// Largest content to carry, all formats together (as sent: images
    /// count as PNG). At most [`CEILING_BYTES`].
    pub max_bytes: usize,
    /// Don't share what the source marked as sensitive (a password
    /// manager's copy). See [`Raw::concealed`] for what is detected.
    pub skip_concealed: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self { text: true, html: true, image: true, max_bytes: DEFAULT_MAX_BYTES, skip_concealed: true }
    }
}

impl Filter {
    pub fn max_bytes(&self) -> usize {
        self.max_bytes.min(CEILING_BYTES)
    }

    /// Drops the formats that are switched off.
    pub fn apply(&self, mut data: ClipData) -> ClipData {
        if !self.text {
            data.text = None;
        }
        if !self.html {
            data.html = None;
        }
        if !self.image {
            data.png = None;
        }
        data
    }
}

/// Result of [`Clipboard::read`].
#[derive(Debug)]
pub enum Read {
    /// Content matches one of the digests the caller already knows.
    Unchanged(Digest),
    /// Nothing we may carry is on the clipboard (empty, or only formats
    /// that are switched off).
    Empty,
    /// Content is larger than [`Filter::max_bytes`].
    TooLarge { digest: Digest, size: usize },
    /// The source marked the content as sensitive and the filter skips it.
    Concealed(Digest),
    Changed { digest: Digest, data: Arc<ClipData> },
}

/// Handle to the clipboard thread. Cheap to clone.
#[derive(Clone)]
pub struct Clipboard {
    tx: mpsc::Sender<Cmd>,
    filter: Arc<Mutex<Filter>>,
    history: Arc<Mutex<history::History>>,
    history_status: Arc<history::Status>,
    history_apps: Arc<Mutex<Vec<String>>>,
    #[cfg(target_os = "linux")]
    _watch: Option<Arc<std::os::unix::net::UnixStream>>,
}

enum Cmd {
    Read { skip: Vec<Digest>, explicit: bool, reply: oneshot::Sender<Result<Read>> },
    Digest { reply: oneshot::Sender<Result<Option<Digest>>> },
    #[cfg(target_os = "linux")]
    Capture { skip_history: bool, owner: u32, timestamp: u32 },
    Write { data: ClipData, origin: String, record: bool, reply: oneshot::Sender<Result<Digest>> },
}

impl Clipboard {
    /// The system clipboard. The platform clipboard is opened on the worker
    /// thread (and reopened after errors), so this never fails.
    pub fn system() -> Self {
        #[allow(unused_mut)]
        let mut clipboard = Self::spawn(Backend::System(None));
        clipboard.history.lock().unwrap_or_else(|error| error.into_inner()).configure(0, history::DEFAULT_BYTES, DEFAULT_MAX_BYTES, None);
        #[cfg(target_os = "linux")]
        { clipboard._watch = history_watch::start(clipboard.tx.clone(), clipboard.history.clone(), clipboard.history_apps.clone()); }
        clipboard
    }

    /// An in-process clipboard, for tests. The returned [`MemoryClipboard`]
    /// plays the role of "the user" copying and pasting.
    pub fn memory() -> (Self, MemoryClipboard) {
        let mem = MemoryClipboard::default();
        (Self::spawn(Backend::Memory(mem.clone())), mem)
    }

    fn spawn(backend: Backend) -> Self {
        let (tx, rx) = mpsc::channel();
        let filter = Arc::new(Mutex::new(Filter::default()));
        let shared = filter.clone();
        let history = Arc::new(Mutex::new(history::History::default()));
        let history_status = history.lock().unwrap().status();
        let worker_history = history.clone();
        let history_apps = Arc::new(Mutex::new(Vec::new()));
        let worker_apps = history_apps.clone();
        std::thread::Builder::new()
            .name("one-mk-clipboard".into())
            .spawn(move || Worker { backend, cache: None, filter: shared, history: worker_history, excluded: VecDeque::new(), restored: None,
                #[cfg(target_os = "linux")]
                restored_owner: None,
                history_apps: worker_apps }.run(rx))
            .expect("spawn clipboard thread");
        Self { tx, filter, history, history_status, history_apps,
            #[cfg(target_os = "linux")]
            _watch: None,
        }
    }

    /// Changes what is shared, from the next read on.
    pub fn set_filter(&self, filter: Filter) {
        *self.filter.lock().expect("poisoned") = filter;
    }

    pub fn filter(&self) -> Filter {
        *self.filter.lock().expect("poisoned")
    }

    /// Reads the clipboard. Content whose digest is in `skip` is not
    /// encoded or copied, so asking "has it changed?" is cheap.
    pub async fn read(&self, skip: Vec<Digest>) -> Result<Read> {
        self.call(|reply| Cmd::Read { skip, explicit: false, reply }).await
    }

    /// Like [`read`](Self::read), for when the user asked for this copy to
    /// be sent: sensitive content is carried too. The formats and size
    /// limit still apply.
    pub async fn read_explicit(&self, skip: Vec<Digest>) -> Result<Read> {
        self.call(|reply| Cmd::Read { skip, explicit: true, reply }).await
    }

    /// The digest of the current content, or `None` if empty.
    pub async fn digest(&self) -> Result<Option<Digest>> {
        self.call(|reply| Cmd::Digest { reply }).await
    }

    /// Replaces the clipboard content. Returns the digest of the content as
    /// read back afterwards, which is what later [`read`](Self::read)s see.
    pub async fn write(&self, data: ClipData) -> Result<Digest> {
        self.write_from(data, "this computer".into(), true).await
    }

    pub fn history(&self) -> &Arc<Mutex<history::History>> { &self.history }
    pub fn history_status(&self) -> &history::Status { &self.history_status }

    pub async fn flush_history(&self) -> Result<()> {
        let saver = self.history.lock().unwrap_or_else(|error| error.into_inner()).saver();
        if let Some(saver) = saver { tokio::task::spawn_blocking(move || saver.flush()).await??; }
        Ok(())
    }

    pub fn history_skip_apps(&self, apps: Vec<String>) {
        *self.history_apps.lock().unwrap_or_else(|error| error.into_inner()) = apps;
    }

    pub async fn write_from(&self, data: ClipData, origin: String, record: bool) -> Result<Digest> {
        self.call(|reply| Cmd::Write { data, origin, record, reply }).await
    }

    async fn call<T>(&self, cmd: impl FnOnce(oneshot::Sender<Result<T>>) -> Cmd) -> Result<T> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(cmd(reply)).map_err(|_| anyhow!("clipboard thread stopped"))?;
        rx.await.map_err(|_| anyhow!("clipboard thread stopped"))?
    }
}

/// Decoded clipboard content, as the platform hands it to us.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    pub text: Option<String>,
    pub html: Option<String>,
    pub image: Option<Image>,
    /// The source marked this as sensitive (what password managers do:
    /// `x-kde-passwordManagerHint` on Linux, `org.nspasteboard.ConcealedType`
    /// on macOS, `ExcludeClipboardContentFromMonitorProcessing` on Windows).
    ///
    /// Native markers are queried separately from arboard. Unmarked secrets
    /// cannot be identified reliably; app exclusion is X11-only.
    pub concealed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// 8-bit RGBA, row-major.
    pub rgba: Vec<u8>,
}

impl Raw {
    fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none() && self.image.is_none()
    }

    /// Drops the formats that are switched off.
    fn filtered(mut self, f: &Filter) -> Self {
        if !f.text {
            self.text = None;
        }
        if !f.html {
            self.html = None;
        }
        if !f.image {
            self.image = None;
        }
        self
    }

    /// When text or HTML is present, drops the image: apps like Excel put a
    /// rendered bitmap next to the text, which is huge and would be encoded
    /// (or make the snapshot too large) for nothing. Writing prefers text
    /// over an image anyway.
    fn prefer_text(mut self) -> Self {
        if self.text.is_some() || self.html.is_some() {
            self.image = None;
        }
        self
    }

    fn digest(&self) -> Digest {
        let mut h = blake3::Hasher::new();
        let mut field = |tag: u8, bytes: Option<&[u8]>| {
            if let Some(b) = bytes {
                h.update(&[tag]);
                h.update(&(b.len() as u64).to_le_bytes());
                h.update(b);
            }
        };
        field(1, self.text.as_deref().map(str::as_bytes));
        field(2, self.html.as_deref().map(str::as_bytes));
        if let Some(i) = &self.image {
            h.update(&[3]);
            h.update(&(i.width as u64).to_le_bytes());
            h.update(&(i.height as u64).to_le_bytes());
            h.update(&i.rgba);
        }
        h.finalize().into()
    }
}

/// A fake clipboard shared between the test and the worker thread.
#[derive(Clone, Default)]
pub struct MemoryClipboard {
    raw: Arc<Mutex<Raw>>,
    writes: Arc<AtomicUsize>,
}

impl MemoryClipboard {
    /// What the user copied.
    pub fn set(&self, raw: Raw) {
        *self.raw.lock().expect("poisoned") = raw;
    }
    pub fn set_text(&self, text: &str) {
        self.set(Raw { text: Some(text.to_owned()), ..Raw::default() });
    }
    /// Marks what is on the clipboard as sensitive, as a password manager would.
    pub fn set_concealed(&self, concealed: bool) {
        self.raw.lock().expect("poisoned").concealed = concealed;
    }
    pub fn get(&self) -> Raw {
        self.raw.lock().expect("poisoned").clone()
    }
    pub fn text(&self) -> Option<String> {
        self.get().text
    }
    /// How many times [`Clipboard::write`] changed this clipboard.
    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

enum Backend {
    System(Option<arboard::Clipboard>),
    Memory(MemoryClipboard),
}

impl Backend {
    /// Reads what `filter` lets through; a format that is switched off is
    /// not even fetched.
    fn read(&mut self, filter: &Filter) -> Result<Raw> {
        match self {
            Backend::Memory(m) => Ok(m.get().filtered(filter)),
            Backend::System(cb) => {
                let cb = open(cb)?;
                let text = if filter.text { cb.get_text().ok() } else { None };
                let html = if filter.html { cb.get().html().ok() } else { None };
                // Text wins over an image (see `Raw::prefer_text`), so don't
                // even fetch a possibly huge bitmap (an Excel range).
                let image = if !filter.image || text.is_some() || html.is_some() {
                    None
                } else {
                    cb.get_image().ok().and_then(|i| {
                        let rgba = i.bytes.into_owned();
                        (i.width > 0 && i.height > 0 && rgba.len() == i.width * i.height * 4)
                            .then_some(Image { width: i.width, height: i.height, rgba })
                    })
                };
                Ok(Raw { text, html, image, concealed: false })
            }
        }
    }

    /// Platforms hold one representation at a time (plus alt text for
    /// HTML), so we choose: an image when there is no text, else HTML
    /// with its text, else text.
    fn write(&mut self, raw: Raw) -> Result<()> {
        match self {
            Backend::Memory(m) => {
                let raw = match raw {
                    Raw { text: None, html: None, image: Some(i), .. } => Raw { image: Some(i), ..Raw::default() },
                    Raw { text, html, .. } => Raw { text, html, ..Raw::default() },
                };
                m.set(raw);
                m.writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            Backend::System(cb) => {
                let cb = open(cb)?;
                let res = match raw {
                    Raw { text: None, html: None, image: Some(i), .. } => cb.set_image(arboard::ImageData {
                        width: i.width,
                        height: i.height,
                        bytes: i.rgba.into(),
                    }),
                    Raw { text, html: Some(html), .. } => cb.set_html(html, text),
                    Raw { text: Some(text), .. } => cb.set_text(text),
                    Raw { .. } => cb.clear(),
                };
                res.context("setting the clipboard")
            }
        }
    }
}

fn open(cb: &mut Option<arboard::Clipboard>) -> Result<&mut arboard::Clipboard> {
    if cb.is_none() {
        *cb = Some(arboard::Clipboard::new().context("opening the clipboard")?);
    }
    Ok(cb.as_mut().expect("just set"))
}

struct Worker {
    backend: Backend,
    /// The last content we encoded, so repeated sends of one big image
    /// encode it only once.
    cache: Option<(Digest, Arc<ClipData>)>,
    filter: Arc<Mutex<Filter>>,
    history: Arc<Mutex<history::History>>,
    excluded: VecDeque<Digest>,
    restored: Option<Digest>,
    #[cfg(target_os = "linux")]
    restored_owner: Option<u32>,
    history_apps: Arc<Mutex<Vec<String>>>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Cmd>) {
        let mut queue = VecDeque::new();
        while let Ok(cmd) = rx.recv() {
            queue.push_back(cmd);
            queue.extend(rx.try_iter());
            while let Some(cmd) = queue.pop_front() {
                match cmd {
                    #[cfg(target_os = "linux")]
                    Cmd::Capture { skip_history, owner, timestamp } => {
                        if matches!(self.backend, Backend::System(_)) && (timestamp == 0 || sensitivity::owner().ok() != Some(owner)) { continue; }
                        if let Ok(raw) = self.backend_read() {
                            if matches!(self.backend, Backend::System(_)) && sensitivity::owner().ok() != Some(owner) { continue; }
                            let digest = raw.digest();
                            if self.restored != Some(digest) && skip_history { self.exclude(digest); }
                            let _ = self.read_raw(raw, &[], false);
                        }
                    }
                    Cmd::Read { skip, explicit, reply } => {
                        let r = self.read(&skip, explicit);
                        let _ = reply.send(r);
                    }
                    Cmd::Digest { reply } => {
                        let r = self.backend_read().map(|raw| (!raw.is_empty()).then(|| raw.digest()));
                        let _ = reply.send(r);
                    }
                    Cmd::Write { data, origin, record, reply } => {
                        let saved = record.then(|| Arc::new(data.clone()));
                        let r = self.write(data);
                        if r.is_ok() && let Some(saved) = saved {
                            let digest = history::digest(&saved);
                            self.history.lock().unwrap_or_else(|error| error.into_inner()).record_hashed(saved, digest, false, &origin);
                        }
                        let _ = reply.send(r);
                    }
                }
            }
        }
    }

    fn filter(&self) -> Filter {
        *self.filter.lock().expect("poisoned")
    }

    fn backend_read(&mut self) -> Result<Raw> {
        let filter = self.filter();
        #[cfg(target_os = "linux")]
        let owner = if matches!(self.backend, Backend::System(_)) && std::env::var_os("WAYLAND_DISPLAY").is_none() { Some(sensitivity::owner()?) } else { None };
        #[cfg(any(windows, target_os = "macos"))]
        let generation = sensitivity::generation();
        let concealed = if matches!(self.backend, Backend::System(_)) {
            let apps = self.history_apps.lock().unwrap_or_else(|error| error.into_inner());
            #[cfg(target_os = "linux")]
            let skip_apps = if owner.is_some() && owner == self.restored_owner { &[][..] } else { apps.as_slice() };
            #[cfg(not(target_os = "linux"))]
            let skip_apps = apps.as_slice();
            sensitivity::concealed(skip_apps)?
        } else { false };
        let r = self.backend.read(&filter).map(|raw| {
            let mut raw = raw.prefer_text();
            raw.concealed |= concealed;
            raw
        });
        #[cfg(target_os = "linux")]
        if let Some(owner) = owner && sensitivity::owner()? != owner { bail!("clipboard owner changed during read; retry"); }
        #[cfg(any(windows, target_os = "macos"))]
        if matches!(self.backend, Backend::System(_)) && sensitivity::generation() != generation { bail!("clipboard changed during sensitivity check; retry"); }
        if r.is_err()
            && let Backend::System(cb) = &mut self.backend
        {
            *cb = None; // reconnect next time (e.g. XWayland restarted)
        }
        r
    }

    fn read(&mut self, skip: &[Digest], explicit: bool) -> Result<Read> {
        let raw = self.backend_read()?;
        self.read_raw(raw, skip, explicit)
    }

    fn exclude(&mut self, digest: Digest) {
        if !self.excluded.contains(&digest) {
            if self.excluded.len() == 32 { self.excluded.pop_front(); }
            self.excluded.push_back(digest);
        }
        let cached = self.cache.as_ref().filter(|(cached, _)| *cached == digest).map(|(_, data)| history::digest(data));
        let mut history = self.history.lock().unwrap_or_else(|error| error.into_inner());
        for entry in history.list() {
            if cached == Some(entry.digest) { history.delete(entry.id); }
        }
    }

    fn read_raw(&mut self, raw: Raw, skip: &[Digest], explicit: bool) -> Result<Read> {
        let max = self.filter().max_bytes();
        let skip_concealed = self.filter().skip_concealed;
        if raw.is_empty() {
            return Ok(Read::Empty);
        }
        let digest = raw.digest();
        if raw.concealed { self.exclude(digest); }
        let concealed = raw.concealed || self.excluded.contains(&digest);
        if skip.contains(&digest) {
            return Ok(Read::Unchanged(digest));
        }
        if concealed && skip_concealed && !explicit {
            return Ok(Read::Concealed(digest));
        }
        if let Some((d, data)) = &self.cache
            && *d == digest
        {
            return Ok(sized(digest, data.clone(), max));
        }
        let text_len = raw.text.as_ref().map_or(0, String::len) + raw.html.as_ref().map_or(0, String::len);
        if text_len > max || raw.image.as_ref().is_some_and(|i| i.rgba.len() > MAX_RGBA) {
            let size = text_len + raw.image.as_ref().map_or(0, |i| i.rgba.len());
            return Ok(Read::TooLarge { digest, size });
        }
        let png = raw.image.as_ref().map(encode_png).transpose()?;
        let data = Arc::new(ClipData { text: raw.text, html: raw.html, png });
        if data.len() <= max {
            let history_digest = history::digest(&data);
            self.history.lock().unwrap_or_else(|error| error.into_inner()).record_hashed(data.clone(), history_digest, concealed, "this computer");
        }
        self.cache = Some((digest, data.clone()));
        Ok(sized(digest, data, max))
    }

    fn write(&mut self, data: ClipData) -> Result<Digest> {
        let saved = Arc::new(data.clone());
        let image = data.png.as_deref().map(decode_png).transpose()?;
        let raw = Raw { text: data.text, html: data.html, image, concealed: false };
        if let Err(e) = self.backend.write(raw) {
            if let Backend::System(cb) = &mut self.backend {
                *cb = None;
            }
            return Err(e);
        }
        let back = self.backend_read()?;
        let digest = back.digest();
        self.cache = Some((digest, saved));
        self.restored = Some(digest);
        #[cfg(target_os = "linux")]
        if matches!(self.backend, Backend::System(_)) { self.restored_owner = sensitivity::owner().ok(); }
        Ok(digest)
    }
}

fn sized(digest: Digest, data: Arc<ClipData>, max: usize) -> Read {
    let size = data.len();
    if size > max { Read::TooLarge { digest, size } } else { Read::Changed { digest, data } }
}

pub fn encode_png(i: &Image) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, i.width as u32, i.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut w = enc.write_header()?;
    w.write_image_data(&i.rgba)?;
    w.finish()?;
    Ok(out)
}

pub fn decode_png(bytes: &[u8]) -> Result<Image> {
    let mut dec = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: MAX_RGBA });
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info()?;
    let size = reader.output_buffer_size().context("image too large")?;
    if size > MAX_RGBA {
        bail!("image too large");
    }
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("unexpected indexed image after expansion"),
    };
    if rgba.len() != w * h * 4 {
        bail!("bad image size");
    }
    Ok(Image { width: w, height: h, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn excluded_copy_is_concealed_but_not_sticky_for_peer_writes() {
        let (clipboard, memory) = Clipboard::memory();
        memory.set_text("excluded secret");
        clipboard.tx.send(Cmd::Capture { skip_history: true, owner: 0, timestamp: 0 }).unwrap();
        assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Concealed(_)));
        assert!(clipboard.history().lock().unwrap().is_empty());
        clipboard.write_from(ClipData { text: Some("peer copy".into()), ..Default::default() }, "peer".into(), true).await.unwrap();
        assert_eq!(clipboard.history().lock().unwrap().len(), 1);
        memory.set_text("unrelated");
        assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Changed { .. }));
        assert_eq!(clipboard.history().lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn restored_item_survives_a_later_read_without_duplicate_or_new_origin() {
        let (clipboard, _) = Clipboard::memory();
        let data = ClipData { text: Some("original".into()), ..Default::default() };
        clipboard.write_from(data.clone(), "peer".into(), true).await.unwrap();
        clipboard.write(ClipData { text: Some("other".into()), ..Default::default() }).await.unwrap();
        clipboard.write_from(data, "peer".into(), false).await.unwrap();
        clipboard.read(vec![]).await.unwrap();
        let history = clipboard.history().lock().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history.get(1).unwrap().origin, "peer");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires an explicitly private X11 display"]
    async fn real_x11_restore_does_not_duplicate_after_selection_notifications() {
        assert_eq!(std::env::var("ONE_MK_PRIVATE_X11").as_deref(), Ok("1"));
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
        let clipboard = Clipboard::system();
        clipboard.history().lock().unwrap().configure(20, history::DEFAULT_BYTES, CEILING_BYTES, None);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let first = ClipData { text: Some("real X11 original".into()), ..Default::default() };
        clipboard.write_from(first.clone(), "peer".into(), true).await.unwrap();
        clipboard.write(ClipData { text: Some("real X11 second".into()), ..Default::default() }).await.unwrap();
        clipboard.write_from(first, "peer".into(), false).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        clipboard.read(vec![]).await.unwrap();
        let history = clipboard.history().lock().unwrap();
        assert_eq!(history.len(), 2, "own XFixes selection events must not re-record restores");
        assert_eq!(history.get(1).unwrap().origin, "peer");
    }

    #[tokio::test]
    async fn history_never_records_concealed_even_for_explicit_send_or_skip_off() {
        let (clipboard, memory) = Clipboard::memory();
        memory.set(Raw { text: Some("password".into()), concealed: true, ..Raw::default() });
        clipboard.read(vec![]).await.unwrap();
        assert!(clipboard.history().lock().unwrap().is_empty());
        clipboard.read_explicit(vec![]).await.unwrap();
        assert!(clipboard.history().lock().unwrap().is_empty());
        clipboard.set_filter(Filter { skip_concealed: false, ..Filter::default() });
        clipboard.read(vec![]).await.unwrap();
        assert!(clipboard.history().lock().unwrap().is_empty());
        memory.set_text("ordinary");
        clipboard.read(vec![]).await.unwrap();
        assert_eq!(clipboard.history().lock().unwrap().len(), 1);
        clipboard.write_from(ClipData { text: Some("peer text".into()), ..ClipData::default() }, "desk".into(), true).await.unwrap();
        let history = clipboard.history().lock().unwrap().list();
        assert_eq!(history[0].origin, "desk");
        assert_eq!(history[0].preview(), "peer text");
    }

    fn image() -> Image {
        let rgba = (0..4 * 3 * 4).map(|i| (i * 7) as u8).collect();
        Image { width: 4, height: 3, rgba }
    }

    #[test]
    fn png_round_trip() {
        let i = image();
        assert_eq!(decode_png(&encode_png(&i).unwrap()).unwrap(), i);
        assert!(decode_png(b"not a png").is_err());
    }

    #[tokio::test]
    async fn memory_read_write() {
        let (cb, mem) = Clipboard::memory();
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Empty));

        mem.set_text("hello");
        let Read::Changed { digest, data } = cb.read(vec![]).await.unwrap() else { panic!() };
        assert_eq!(data.text.as_deref(), Some("hello"));
        assert!(matches!(cb.read(vec![digest]).await.unwrap(), Read::Unchanged(d) if d == digest));
        assert_eq!(cb.digest().await.unwrap(), Some(digest));

        // Writing returns the digest later reads will see.
        let png = encode_png(&image()).unwrap();
        let d = cb.write(ClipData { png: Some(png.clone()), ..Default::default() }).await.unwrap();
        assert_eq!(mem.get().image, Some(image()));
        let Read::Changed { digest, data } = cb.read(vec![]).await.unwrap() else { panic!() };
        assert_eq!(digest, d);
        assert_eq!(decode_png(data.png.as_ref().unwrap()).unwrap(), image());

        let d = cb
            .write(ClipData { text: Some("t".into()), html: Some("<b>t</b>".into()), png: None })
            .await
            .unwrap();
        assert!(matches!(cb.read(vec![d]).await.unwrap(), Read::Unchanged(_)));
    }

    #[tokio::test]
    async fn too_large_is_reported() {
        let (cb, mem) = Clipboard::memory();
        mem.set_text(&"x".repeat(DEFAULT_MAX_BYTES + 1));
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::TooLarge { size, .. } if size == DEFAULT_MAX_BYTES + 1));
    }

    #[tokio::test]
    async fn size_cap_is_configurable_and_live() {
        let (cb, mem) = Clipboard::memory();
        mem.set_text(&"x".repeat(3 << 20));
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Changed { .. }), "under the default cap");

        cb.set_filter(Filter { max_bytes: 2 << 20, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::TooLarge { size, .. } if size == 3 << 20));

        cb.set_filter(Filter { max_bytes: 4 << 20, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Changed { .. }));

        // The cap on an image counts its PNG, not the RGBA behind it.
        let i = image();
        mem.set(Raw { image: Some(i.clone()), ..Raw::default() });
        let png_len = encode_png(&i).unwrap().len();
        cb.set_filter(Filter { max_bytes: png_len, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Changed { .. }));
        cb.set_filter(Filter { max_bytes: png_len - 1, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::TooLarge { .. }));

        // Whatever the setting, it is bounded.
        let huge = Filter { max_bytes: usize::MAX, ..Filter::default() };
        assert_eq!(huge.max_bytes(), CEILING_BYTES);
    }

    #[tokio::test]
    async fn formats_can_be_switched_off() {
        let (cb, mem) = Clipboard::memory();
        mem.set(Raw { text: Some("t".into()), html: Some("<b>t</b>".into()), ..Raw::default() });

        cb.set_filter(Filter { html: false, ..Filter::default() });
        let Read::Changed { digest: no_html, data } = cb.read(vec![]).await.unwrap() else { panic!() };
        assert_eq!((data.text.as_deref(), data.html.as_deref()), (Some("t"), None));

        cb.set_filter(Filter { text: false, html: false, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Empty), "nothing allowed is left");
        assert_eq!(cb.digest().await.unwrap(), None);

        // The digest is over what passes the filter, so a baseline taken
        // with the same filter matches and a changed filter looks like a change.
        cb.set_filter(Filter { html: false, ..Filter::default() });
        assert_eq!(cb.digest().await.unwrap(), Some(no_html));
        assert!(matches!(cb.read(vec![no_html]).await.unwrap(), Read::Unchanged(_)));
        cb.set_filter(Filter::default());
        assert!(matches!(cb.read(vec![no_html]).await.unwrap(), Read::Changed { .. }));

        // Images off: an image-only clipboard is empty; with text off and
        // image on, the image of a text+bitmap copy (Excel) is what is shared.
        mem.set(Raw { image: Some(image()), ..Raw::default() });
        cb.set_filter(Filter { image: false, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Empty));
        mem.set(Raw { text: Some("cells".into()), image: Some(image()), ..Raw::default() });
        cb.set_filter(Filter { text: false, ..Filter::default() });
        let Read::Changed { data, .. } = cb.read(vec![]).await.unwrap() else { panic!() };
        assert!(data.png.is_some() && data.text.is_none());
    }

    #[test]
    fn filter_applies_to_received_data() {
        let all = ClipData { text: Some("t".into()), html: Some("h".into()), png: Some(vec![1]) };
        let f = Filter { html: false, image: false, ..Filter::default() };
        assert_eq!(f.apply(all.clone()), ClipData { text: Some("t".into()), ..Default::default() });
        assert_eq!(Filter::default().apply(all.clone()), all);
    }

    #[tokio::test]
    async fn concealed_content_is_skipped_unless_asked_for() {
        let (cb, mem) = Clipboard::memory();
        mem.set_text("hunter2");
        mem.set_concealed(true);
        let Read::Concealed(d) = cb.read(vec![]).await.unwrap() else { panic!("a password manager's copy was read") };
        // The digest is still given so callers can tell it from the next copy.
        assert_eq!(cb.digest().await.unwrap(), Some(d));

        // The user asking to send it explicitly overrides.
        let Read::Changed { data, .. } = cb.read_explicit(vec![]).await.unwrap() else { panic!() };
        assert_eq!(data.text.as_deref(), Some("hunter2"));

        // And the skip can be turned off.
        cb.set_filter(Filter { skip_concealed: false, ..Filter::default() });
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Changed { .. }));
        cb.set_filter(Filter::default());
        mem.set_concealed(false);
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::Concealed(_)), "a known excluded digest stays private even when the source drops its hint");
    }

    #[tokio::test]
    async fn text_wins_over_huge_image() {
        let (cb, mem) = Clipboard::memory();
        // Zeroed, so the allocation is lazy; it is over MAX_RGBA.
        let big = Image { width: 16384, height: 4097, rgba: vec![0; 16384 * 4097 * 4] };
        assert!(big.rgba.len() > MAX_RGBA);
        mem.set(Raw { text: Some("cells".into()), html: Some("<td>cells</td>".into()), image: Some(big.clone()), concealed: false });
        let Read::Changed { digest, data } = cb.read(vec![]).await.unwrap() else { panic!("expected content, not TooLarge") };
        assert_eq!(data.text.as_deref(), Some("cells"));
        assert!(data.html.is_some());
        assert!(data.png.is_none());
        assert_eq!(cb.digest().await.unwrap(), Some(digest));

        // An image alone is still carried.
        mem.set(Raw { image: Some(image()), ..Raw::default() });
        let Read::Changed { data, .. } = cb.read(vec![]).await.unwrap() else { panic!() };
        assert!(data.png.is_some() && data.text.is_none());
    }

    /// Touches the real clipboard, so only on request:
    /// `cargo test -p one-mk-clipboard -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn system_round_trip() {
        let cb = Clipboard::system();
        let before = cb.read(vec![]).await.unwrap();
        let d = cb.write(ClipData { text: Some("one-mk test".into()), ..Default::default() }).await.unwrap();
        let Read::Unchanged(got) = cb.read(vec![d]).await.unwrap() else { panic!("not what we wrote") };
        assert_eq!(got, d);
        if let Read::Changed { data, .. } = before {
            cb.write((*data).clone()).await.unwrap();
        }
    }
}
