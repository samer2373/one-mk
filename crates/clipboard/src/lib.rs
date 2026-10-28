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

/// Largest clipboard content we send or accept, in bytes (all formats).
pub const MAX_BYTES: usize = 32 << 20;
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

/// Result of [`Clipboard::read`].
#[derive(Debug)]
pub enum Read {
    /// Content matches one of the digests the caller already knows.
    Unchanged(Digest),
    /// Nothing we can carry is on the clipboard.
    Empty,
    /// Content is larger than [`MAX_BYTES`].
    TooLarge { digest: Digest, size: usize },
    Changed { digest: Digest, data: Arc<ClipData> },
}

/// Handle to the clipboard thread. Cheap to clone.
#[derive(Clone)]
pub struct Clipboard {
    tx: mpsc::Sender<Cmd>,
}

enum Cmd {
    Read { skip: Vec<Digest>, reply: oneshot::Sender<Result<Read>> },
    Digest { reply: oneshot::Sender<Result<Option<Digest>>> },
    Write { data: ClipData, reply: oneshot::Sender<Result<Digest>> },
}

impl Clipboard {
    /// The system clipboard. The platform clipboard is opened on the worker
    /// thread (and reopened after errors), so this never fails.
    pub fn system() -> Self {
        Self::spawn(Backend::System(None))
    }

    /// An in-process clipboard, for tests. The returned [`MemoryClipboard`]
    /// plays the role of "the user" copying and pasting.
    pub fn memory() -> (Self, MemoryClipboard) {
        let mem = MemoryClipboard::default();
        (Self::spawn(Backend::Memory(mem.clone())), mem)
    }

    fn spawn(backend: Backend) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("one-mk-clipboard".into())
            .spawn(move || Worker { backend, cache: None }.run(rx))
            .expect("spawn clipboard thread");
        Self { tx }
    }

    /// Reads the clipboard. Content whose digest is in `skip` is not
    /// encoded or copied, so asking "has it changed?" is cheap.
    pub async fn read(&self, skip: Vec<Digest>) -> Result<Read> {
        self.call(|reply| Cmd::Read { skip, reply }).await
    }

    /// The digest of the current content, or `None` if empty.
    pub async fn digest(&self) -> Result<Option<Digest>> {
        self.call(|reply| Cmd::Digest { reply }).await
    }

    /// Replaces the clipboard content. Returns the digest of the content as
    /// read back afterwards, which is what later [`read`](Self::read)s see.
    pub async fn write(&self, data: ClipData) -> Result<Digest> {
        self.call(|reply| Cmd::Write { data, reply }).await
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
    fn read(&mut self) -> Result<Raw> {
        match self {
            Backend::Memory(m) => Ok(m.get()),
            Backend::System(cb) => {
                let cb = open(cb)?;
                let text = cb.get_text().ok();
                let html = cb.get().html().ok();
                let image = cb.get_image().ok().and_then(|i| {
                    let rgba = i.bytes.into_owned();
                    (i.width > 0 && i.height > 0 && rgba.len() == i.width * i.height * 4)
                        .then_some(Image { width: i.width, height: i.height, rgba })
                });
                Ok(Raw { text, html, image })
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
                    Raw { text: None, html: None, image: Some(i) } => Raw { image: Some(i), ..Raw::default() },
                    Raw { text, html, .. } => Raw { text, html, image: None },
                };
                m.set(raw);
                m.writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            Backend::System(cb) => {
                let cb = open(cb)?;
                let res = match raw {
                    Raw { text: None, html: None, image: Some(i) } => cb.set_image(arboard::ImageData {
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
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Cmd>) {
        let mut queue = VecDeque::new();
        while let Ok(cmd) = rx.recv() {
            queue.push_back(cmd);
            queue.extend(rx.try_iter());
            while let Some(cmd) = queue.pop_front() {
                match cmd {
                    Cmd::Read { skip, reply } => {
                        let r = self.read(&skip);
                        let _ = reply.send(r);
                    }
                    Cmd::Digest { reply } => {
                        let r = self.backend_read().map(|raw| (!raw.is_empty()).then(|| raw.digest()));
                        let _ = reply.send(r);
                    }
                    Cmd::Write { data, reply } => {
                        let r = self.write(data);
                        let _ = reply.send(r);
                    }
                }
            }
        }
    }

    fn backend_read(&mut self) -> Result<Raw> {
        let r = self.backend.read();
        if r.is_err()
            && let Backend::System(cb) = &mut self.backend
        {
            *cb = None; // reconnect next time (e.g. XWayland restarted)
        }
        r
    }

    fn read(&mut self, skip: &[Digest]) -> Result<Read> {
        let raw = self.backend_read()?;
        if raw.is_empty() {
            return Ok(Read::Empty);
        }
        let digest = raw.digest();
        if skip.contains(&digest) {
            return Ok(Read::Unchanged(digest));
        }
        if let Some((d, data)) = &self.cache
            && *d == digest
        {
            return Ok(sized(digest, data.clone()));
        }
        let text_len = raw.text.as_ref().map_or(0, String::len) + raw.html.as_ref().map_or(0, String::len);
        if text_len > MAX_BYTES || raw.image.as_ref().is_some_and(|i| i.rgba.len() > MAX_RGBA) {
            let size = text_len + raw.image.as_ref().map_or(0, |i| i.rgba.len());
            return Ok(Read::TooLarge { digest, size });
        }
        let png = raw.image.as_ref().map(encode_png).transpose()?;
        let data = Arc::new(ClipData { text: raw.text, html: raw.html, png });
        self.cache = Some((digest, data.clone()));
        Ok(sized(digest, data))
    }

    fn write(&mut self, data: ClipData) -> Result<Digest> {
        let image = data.png.as_deref().map(decode_png).transpose()?;
        let raw = Raw { text: data.text, html: data.html, image };
        if let Err(e) = self.backend.write(raw) {
            if let Backend::System(cb) = &mut self.backend {
                *cb = None;
            }
            return Err(e);
        }
        let back = self.backend_read()?;
        Ok(back.digest())
    }
}

fn sized(digest: Digest, data: Arc<ClipData>) -> Read {
    let size = data.len();
    if size > MAX_BYTES { Read::TooLarge { digest, size } } else { Read::Changed { digest, data } }
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
        mem.set_text(&"x".repeat(MAX_BYTES + 1));
        assert!(matches!(cb.read(vec![]).await.unwrap(), Read::TooLarge { size, .. } if size == MAX_BYTES + 1));
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
