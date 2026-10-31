//! Playback decoding for an already imported item, and the bounded frame
//! stream between the sandboxed decode worker and the renderer.
//!
//! The worker (`one-mk-wall-import --play`, launched only through
//! [`crate::sandbox::spawn_playback`]) re-admits the source inside the import
//! sandbox, then writes one [`StreamHeader`] followed by opaque frames to
//! standard output: BGRx for images, the decoder's own 8-bit I420 for video
//! (no colour conversion in the worker: the renderer converts only the pixels
//! it shows, with the matrix and range the header names). Animated
//! media repeat forever (hard-cut loop, design section 5); a still image sends
//! its one frame and exits. The renderer never parses media: it validates the
//! header against the import record and treats frame bytes as pixels only.

use crate::{Format, MAX_DURATION_MS, MAX_FRAMES};
use std::io::Read;

pub const STREAM_MAGIC: &[u8; 4] = b"WSTR";
pub const STREAM_VERSION: u8 = 1;
/// Fixed part of the header; `frames` little-endian u32 durations follow.
pub const HEADER_BYTES: usize = 20;
/// Shortest animated frame the stream admits (admission is stricter: 17 ms
/// for images, 16 ms between WebM blocks).
pub const MIN_FRAME_MICROS: u32 = 10_000;

/// How one frame's bytes are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pixels {
    /// `width * height` little-endian XRGB8888 words (images).
    Bgrx,
    /// GStreamer's default 8-bit I420 layout (planes Y, U, V; strides rounded
    /// up to four bytes), with the YCbCr matrix and range of the stream.
    I420 { bt709: bool, full_range: bool },
}

impl Pixels {
    fn code(self) -> u8 {
        match self {
            Self::Bgrx => 0,
            Self::I420 { bt709, full_range } => 1 + u8::from(bt709) + 2 * u8::from(full_range),
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Bgrx),
            1..=4 => Some(Self::I420 { bt709: (code - 1) & 1 != 0, full_range: (code - 1) & 2 != 0 }),
            _ => None,
        }
    }
}

/// Plane geometry of an I420 frame: (Y stride, chroma stride, chroma rows,
/// U offset, V offset, total bytes).
pub fn i420_planes(width: u32, height: u32) -> (usize, usize, usize, usize, usize, usize) {
    let round = |value: usize| value.div_ceil(4) * 4;
    let (width, height) = (width as usize, height as usize);
    let luma = round(width);
    let chroma = round(width.div_ceil(2));
    let rows = height.div_ceil(2);
    let u = luma * height.next_multiple_of(2);
    let v = u + chroma * rows;
    (luma, chroma, rows, u, v, v + chroma * rows)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamHeader {
    pub format: Format,
    pub pixels: Pixels,
    pub width: u32,
    pub height: u32,
    /// Display duration of each frame in microseconds; `[0]` for a still.
    pub durations: Vec<u32>,
}

fn format_code(format: Format) -> u8 {
    match format {
        Format::Gif => 1, Format::Apng => 2, Format::Webp => 3,
        Format::Vp9Webm => 4, Format::Av1Webm => 5, Format::StillPng => 6,
    }
}

fn format_from(code: u8) -> Option<Format> {
    Some(match code {
        1 => Format::Gif, 2 => Format::Apng, 3 => Format::Webp,
        4 => Format::Vp9Webm, 5 => Format::Av1Webm, 6 => Format::StillPng,
        _ => return None,
    })
}

impl StreamHeader {
    pub fn still(&self) -> bool { self.format == Format::StillPng }

    /// Bytes of one frame.
    pub fn frame_bytes(&self) -> usize {
        match self.pixels {
            Pixels::Bgrx => self.width as usize * self.height as usize * 4,
            Pixels::I420 { .. } => i420_planes(self.width, self.height).5,
        }
    }

    /// Total loop length in microseconds (0 for a still).
    pub fn loop_micros(&self) -> u64 { self.durations.iter().map(|&value| u64::from(value)).sum() }

    pub fn validate(&self) -> Result<(), String> {
        crate::dimensions(self.width, self.height)?;
        let video = matches!(self.format, Format::Vp9Webm | Format::Av1Webm);
        if video != matches!(self.pixels, Pixels::I420 { .. }) { return Err("pixel layout does not match the format".into()); }
        if self.still() {
            if self.durations != [0] { return Err("a still stream has exactly one frame".into()); }
            return Ok(());
        }
        if self.durations.len() < 2 || self.durations.len() > MAX_FRAMES as usize {
            return Err("animated stream needs 2..36000 frames".into());
        }
        if self.durations.iter().any(|&value| value < MIN_FRAME_MICROS || u64::from(value) > MAX_DURATION_MS * 1000) {
            return Err("stream frame duration out of bounds".into());
        }
        if self.loop_micros() > MAX_DURATION_MS * 1000 { return Err("stream loop exceeds 600 seconds".into()); }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(HEADER_BYTES + self.durations.len() * 4);
        bytes.extend_from_slice(STREAM_MAGIC);
        bytes.extend_from_slice(&[STREAM_VERSION, format_code(self.format), self.pixels.code(), 0]);
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&(self.durations.len() as u32).to_le_bytes());
        for duration in &self.durations { bytes.extend_from_slice(&duration.to_le_bytes()); }
        Ok(bytes)
    }

    /// Reads and validates a header from an untrusted worker stream.
    pub fn read(reader: &mut impl Read) -> Result<Self, String> {
        let mut fixed = [0_u8; HEADER_BYTES];
        reader.read_exact(&mut fixed).map_err(|error| format!("frame stream header: {error}"))?;
        if &fixed[..4] != STREAM_MAGIC || fixed[4] != STREAM_VERSION || fixed[7] != 0 {
            return Err("frame stream magic/version mismatch".into());
        }
        let format = format_from(fixed[5]).ok_or("unknown frame stream format")?;
        let pixels = Pixels::from_code(fixed[6]).ok_or("unknown frame stream pixel layout")?;
        let word = |offset: usize| u32::from_le_bytes(fixed[offset..offset + 4].try_into().expect("four bytes"));
        let (width, height, frames) = (word(8), word(12), word(16));
        if frames == 0 || frames > MAX_FRAMES { return Err("frame stream frame count out of bounds".into()); }
        let mut table = vec![0_u8; frames as usize * 4];
        reader.read_exact(&mut table).map_err(|error| format!("frame stream timeline: {error}"))?;
        let durations = table.as_chunks::<4>().0.iter().map(|chunk| u32::from_le_bytes(*chunk)).collect();
        let header = Self { format, pixels, width, height, durations };
        header.validate()?;
        Ok(header)
    }
}

/// Opaque black flattening into BGRx, the layout of `wl_shm` XRGB8888.
pub fn rgba_to_bgrx(rgba: &[u8], bgrx: &mut [u8]) {
    for (source, target) in rgba.as_chunks::<4>().0.iter().zip(bgrx.as_chunks_mut::<4>().0.iter_mut()) {
        let alpha = u16::from(source[3]);
        let channel = |value: u8| ((u16::from(value) * alpha + 127) / 255) as u8;
        target[0] = channel(source[2]);
        target[1] = channel(source[1]);
        target[2] = channel(source[0]);
        target[3] = 0xff;
    }
}

/// Worker entry point (`--play`): runs inside the playback sandbox only.
#[cfg(target_os = "linux")]
pub fn worker() -> Result<(), String> {
    use std::io::Write;
    crate::sandbox::check_playback_worker()?;
    let mut bytes = Vec::new();
    std::fs::File::open("/input").map_err(|error| error.to_string())?
        .take(crate::MAX_SOURCE + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let (media, durations) = crate::admission::timeline(&bytes).map_err(|error| format!("validation: {error}"))?;
    // Decoding never needs a new process; threads stay allowed.
    crate::sandbox::deny_subprocesses()?;
    // SAFETY: descriptor 1 is the frame pipe handed over by bubblewrap; it is
    // borrowed for the lifetime of the worker and never closed here.
    let mut output = std::mem::ManuallyDrop::new(unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(1) });
    let mut header = StreamHeader { format: media.format, pixels: Pixels::Bgrx, width: media.width, height: media.height, durations };
    match media.format {
        Format::Vp9Webm | Format::Av1Webm => gstreamer::play(&media, |pixels| {
            header.pixels = pixels;
            output.write_all(&header.encode()?).map_err(|error| format!("frame pipe: {error}"))
        }),
        _ => {
            output.write_all(&header.encode()?).map_err(|error| format!("frame pipe: {error}"))?;
            images(&bytes, &media, &mut *output)
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn worker() -> Result<(), String> { crate::sandbox::worker_restrictions() }

#[cfg(target_os = "linux")]
fn images(bytes: &[u8], media: &crate::Media, output: &mut impl std::io::Write) -> Result<(), String> {
    use image::{AnimationDecoder, ImageDecoder};
    use std::io::Cursor;
    let limits = || {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(2160);
        limits.max_alloc = Some(256 * 1024 * 1024);
        limits
    };
    let mut frame = vec![0_u8; media.width as usize * media.height as usize * 4];
    let mut emit = |image: &image::RgbaImage, frame: &mut Vec<u8>| -> Result<(), String> {
        if image.dimensions() != (media.width, media.height) { return Err("decoded dimensions changed".into()); }
        rgba_to_bgrx(image.as_raw(), frame);
        output.write_all(frame).map_err(|error| format!("frame pipe: {error}"))
    };
    if media.format == Format::StillPng {
        let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
        decoder.set_limits(limits()).map_err(|error| error.to_string())?;
        let image = image::DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?.to_rgba8();
        return emit(&image, &mut frame);
    }
    loop {
        let frames = match media.format {
            Format::Gif => {
                let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
                decoder.set_limits(limits()).map_err(|error| error.to_string())?;
                decoder.into_frames()
            }
            Format::Apng => {
                let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
                decoder.set_limits(limits()).map_err(|error| error.to_string())?;
                decoder.apng().map_err(|error| error.to_string())?.into_frames()
            }
            Format::Webp => {
                let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
                decoder.set_limits(limits()).map_err(|error| error.to_string())?;
                decoder.into_frames()
            }
            _ => return Err("not an image format".into()),
        };
        let mut count = 0_u32;
        for decoded in frames {
            let decoded = decoded.map_err(|error| error.to_string())?;
            count += 1;
            if count > media.frames { return Err("decoded frame count differs from container".into()); }
            emit(decoded.buffer(), &mut frame)?;
        }
        if count != media.frames { return Err("decoded frame count differs from container".into()); }
    }
}

/// In-process GStreamer from the pinned runtime (`/lib`, `/plugins`), loaded
/// only inside the sandbox. The fixed pipeline has no URI loader, decodebin or
/// encoder; a flushing seek to zero after end-of-stream makes the loop.
#[cfg(target_os = "linux")]
mod gstreamer {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::ptr;

    #[repr(C)]
    struct GError { domain: u32, code: c_int, message: *mut c_char }

    /// The public, ABI-stable head of `GstMessage` (GStreamer 1.x).
    #[repr(C)]
    struct MiniObject {
        type_: usize, refcount: c_int, lockstate: c_int, flags: u32,
        copy: *mut c_void, dispose: *mut c_void, free: *mut c_void,
        priv_uint: u32, priv_pointer: *mut c_void,
    }
    #[repr(C)]
    struct Message { mini_object: MiniObject, kind: u32 }

    const MESSAGE_EOS: u32 = 1;
    const MESSAGE_ERROR: u32 = 2;
    const MESSAGE_ASYNC_DONE: u32 = 1 << 21;
    const STATE_NULL: c_int = 1;
    const STATE_PAUSED: c_int = 3;
    const STATE_PLAYING: c_int = 4;
    const FORMAT_TIME: c_int = 3;
    const SEEK_FLUSH: c_int = 1;

    struct Library(*mut c_void);
    impl Library {
        fn open(name: &CStr) -> Result<Self, String> {
            // SAFETY: dlopen with a constant NUL-terminated name.
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if handle.is_null() { return Err(format!("pinned runtime library {} unavailable", name.to_string_lossy())); }
            Ok(Self(handle))
        }
        /// # Safety
        /// `Function` must be the exact C signature of `name`.
        unsafe fn get<Function: Copy>(&self, name: &CStr) -> Result<Function, String> {
            // SAFETY: the caller states the symbol's signature.
            let symbol = unsafe { libc::dlsym(self.0, name.as_ptr()) };
            if symbol.is_null() { return Err(format!("missing runtime symbol {}", name.to_string_lossy())); }
            // SAFETY: function pointers and data pointers share a size on Linux.
            Ok(unsafe { std::mem::transmute_copy::<*mut c_void, Function>(&symbol) })
        }
    }

    fn take_error(error: *mut GError, free: unsafe extern "C" fn(*mut GError)) -> String {
        if error.is_null() { return "GStreamer failure without details".into(); }
        // SAFETY: a non-null GError from GStreamer has a valid message.
        let text = unsafe { CStr::from_ptr((*error).message) }.to_string_lossy().chars().take(256).collect();
        // SAFETY: the error is owned by this caller.
        unsafe { free(error) };
        text
    }

    /// The stream's YCbCr matrix and range from negotiated caps text such as
    /// `colorimetry=(string)bt709` or `colorimetry=(string)2:4:0:0`. Unknown
    /// values fall back to GStreamer's own default (BT.709 from 720 rows).
    pub(super) fn pixels(caps: &str, height: u32) -> super::Pixels {
        let value = caps.split("colorimetry=(string)").nth(1)
            .map(|rest| rest.split([',', ';', ' ']).next().unwrap_or_default().trim_matches('"')).unwrap_or_default();
        let fallback = height >= 720;
        let (bt709, full_range) = match value {
            "bt709" => (true, false),
            "bt601" | "smpte170m" => (false, false),
            "sRGB" => (true, true),
            numeric => {
                let fields: Vec<&str> = numeric.split(':').collect();
                let range = fields.first().copied();
                let matrix = fields.get(1).copied();
                (match matrix { Some("3") => true, Some("4") => false, _ => fallback }, range == Some("1"))
            }
        };
        super::Pixels::I420 { bt709, full_range }
    }

    pub(super) fn play(media: &crate::Media, mut ready: impl FnMut(super::Pixels) -> Result<(), String>) -> Result<(), String> {
        let decoder = match media.format { crate::Format::Vp9Webm => "vp9dec threads=2", _ => "av1dec" };
        // videoconvert passes the decoder's 8-bit I420 through untouched; it
        // only works for other decoder layouts.
        let description = CString::new(format!(
            "filesrc location=/input ! matroskademux ! {decoder} ! videoconvert ! \
             video/x-raw,format=I420,width={},height={} ! fdsink name=frames fd=1 sync=false",
            media.width, media.height)).map_err(|error| error.to_string())?;
        let gstreamer = Library::open(c"libgstreamer-1.0.so.0")?;
        let glib = Library::open(c"libglib-2.0.so.0")?;
        // SAFETY: each signature matches the GStreamer 1.x / GLib C API.
        unsafe {
            let init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char, *mut *mut GError) -> c_int = gstreamer.get(c"gst_init_check")?;
            let parse: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_void = gstreamer.get(c"gst_parse_launch")?;
            let set_state: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = gstreamer.get(c"gst_element_set_state")?;
            let get_bus: unsafe extern "C" fn(*mut c_void) -> *mut c_void = gstreamer.get(c"gst_element_get_bus")?;
            let pop: unsafe extern "C" fn(*mut c_void, u64, u32) -> *mut Message = gstreamer.get(c"gst_bus_timed_pop_filtered")?;
            let parse_error: unsafe extern "C" fn(*mut Message, *mut *mut GError, *mut *mut c_char) = gstreamer.get(c"gst_message_parse_error")?;
            let seek: unsafe extern "C" fn(*mut c_void, c_int, c_int, i64) -> c_int = gstreamer.get(c"gst_element_seek_simple")?;
            let unref_message: unsafe extern "C" fn(*mut Message) = gstreamer.get(c"gst_mini_object_unref")?;
            let unref: unsafe extern "C" fn(*mut c_void) = gstreamer.get(c"gst_object_unref")?;
            let free_error: unsafe extern "C" fn(*mut GError) = glib.get(c"g_error_free")?;
            let free: unsafe extern "C" fn(*mut c_void) = glib.get(c"g_free")?;
            let by_name: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = gstreamer.get(c"gst_bin_get_by_name")?;
            let static_pad: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = gstreamer.get(c"gst_element_get_static_pad")?;
            let current_caps: unsafe extern "C" fn(*mut c_void) -> *mut c_void = gstreamer.get(c"gst_pad_get_current_caps")?;
            let caps_text: unsafe extern "C" fn(*mut c_void) -> *mut c_char = gstreamer.get(c"gst_caps_to_string")?;
            let unref_caps: unsafe extern "C" fn(*mut c_void) = gstreamer.get(c"gst_mini_object_unref")?;
            let mut error = ptr::null_mut();
            if init(ptr::null_mut(), ptr::null_mut(), &mut error) == 0 { return Err(take_error(error, free_error)); }
            let pipeline = parse(description.as_ptr(), &mut error);
            if pipeline.is_null() || !error.is_null() {
                if !pipeline.is_null() { unref(pipeline); }
                return Err(take_error(error, free_error));
            }
            let bus = get_bus(pipeline);
            let result = (|| {
                if bus.is_null() { return Err("pipeline has no bus".to_string()); }
                // Preroll first: the negotiated caps name the colour matrix
                // before any frame is written.
                if set_state(pipeline, STATE_PAUSED) == 0 { return Err("GStreamer refused PAUSED".into()); }
                let message = pop(bus, 30_000_000_000, MESSAGE_ASYNC_DONE | MESSAGE_ERROR);
                if message.is_null() { return Err("decoder did not preroll within 30 s".into()); }
                let kind = (*message).kind;
                if kind == MESSAGE_ERROR {
                    let mut detail = ptr::null_mut();
                    let mut text = ptr::null_mut();
                    parse_error(message, &mut detail, &mut text);
                    if !text.is_null() { free(text.cast()); }
                    unref_message(message);
                    return Err(format!("decoder: {}", take_error(detail, free_error)));
                }
                unref_message(message);
                let sink = by_name(pipeline, c"frames".as_ptr());
                if sink.is_null() { return Err("frame sink missing".into()); }
                let pad = static_pad(sink, c"sink".as_ptr());
                unref(sink);
                if pad.is_null() { return Err("frame sink pad missing".into()); }
                let caps = current_caps(pad);
                unref(pad);
                if caps.is_null() { return Err("decoder negotiated no caps".into()); }
                let text = caps_text(caps);
                unref_caps(caps);
                if text.is_null() { return Err("caps text unavailable".into()); }
                let negotiated = CStr::from_ptr(text).to_string_lossy().into_owned();
                free(text.cast());
                ready(pixels(&negotiated, media.height))?;
                if set_state(pipeline, STATE_PLAYING) == 0 { return Err("GStreamer refused PLAYING".into()); }
                loop {
                    // Block until end-of-stream or an error; nothing else is polled.
                    let message = pop(bus, u64::MAX, MESSAGE_EOS | MESSAGE_ERROR);
                    if message.is_null() { continue; }
                    let kind = (*message).kind;
                    if kind == MESSAGE_ERROR {
                        let mut detail = ptr::null_mut();
                        let mut text = ptr::null_mut();
                        parse_error(message, &mut detail, &mut text);
                        if !text.is_null() { free(text.cast()); }
                        unref_message(message);
                        return Err(format!("decoder: {}", take_error(detail, free_error)));
                    }
                    unref_message(message);
                    if seek(pipeline, FORMAT_TIME, SEEK_FLUSH, 0) == 0 { return Err("loop seek refused".into()); }
                }
            })();
            set_state(pipeline, STATE_NULL);
            if !bus.is_null() { unref(bus); }
            unref(pipeline);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> StreamHeader {
        StreamHeader { format: Format::Gif, pixels: Pixels::Bgrx, width: 4, height: 2, durations: vec![100_000, 50_000, 17_000] }
    }

    #[test]
    fn header_round_trips_and_every_truncation_is_refused() {
        let bytes = header().encode().unwrap();
        assert_eq!(bytes.len(), HEADER_BYTES + 12);
        assert_eq!(StreamHeader::read(&mut bytes.as_slice()).unwrap(), header());
        for length in 0..bytes.len() {
            assert!(StreamHeader::read(&mut &bytes[..length]).is_err(), "{length}");
        }
        assert_eq!(header().frame_bytes(), 32);
        assert_eq!(header().loop_micros(), 167_000);
    }

    #[test]
    fn hostile_headers_are_refused() {
        let good = header().encode().unwrap();
        let mutate = |offset: usize, value: u8| { let mut bytes = good.clone(); bytes[offset] = value; bytes };
        for bytes in [mutate(0, b'X'), mutate(4, 2), mutate(5, 0), mutate(5, 7), mutate(6, 1), mutate(7, 1),
            mutate(8, 0), mutate(13, 0x40), mutate(16, 0), mutate(29, 0), mutate(19, 0x80)] {
            assert!(StreamHeader::read(&mut bytes.as_slice()).is_err(), "{bytes:?}");
        }
        for bad in [mutate(6, 1), mutate(6, 9)] { assert!(StreamHeader::read(&mut bad.as_slice()).is_err()); }
        let still = StreamHeader { format: Format::StillPng, pixels: Pixels::Bgrx, width: 2, height: 2, durations: vec![0] };
        assert!(StreamHeader::read(&mut still.encode().unwrap().as_slice()).unwrap().still());
        for durations in [vec![], vec![0, 0], vec![1]] {
            assert!(StreamHeader { durations, ..still.clone() }.encode().is_err());
        }
        for durations in [vec![100_000], vec![9_999, 100_000], vec![300_000_000, 300_000_001]] {
            assert!(StreamHeader { durations, ..header() }.encode().is_err());
        }
        let oversized = StreamHeader { width: 4097, ..header() };
        assert!(oversized.encode().is_err());
    }

    #[test]
    fn video_streams_carry_i420_with_their_colorimetry() {
        let video = StreamHeader { format: Format::Vp9Webm, pixels: Pixels::I420 { bt709: true, full_range: false },
            width: 5, height: 3, durations: vec![33_000, 33_000] };
        let bytes = video.encode().unwrap();
        assert_eq!(StreamHeader::read(&mut bytes.as_slice()).unwrap(), video);
        assert_eq!(i420_planes(5, 3), (8, 4, 2, 32, 40, 48));
        assert_eq!(video.frame_bytes(), 48);
        assert!(StreamHeader { pixels: Pixels::Bgrx, ..video.clone() }.encode().is_err());
        for code in 0..=4 { assert_eq!(Pixels::from_code(code).unwrap().code(), code); }
        #[cfg(target_os = "linux")]
        {
            use gstreamer::pixels;
            assert_eq!(pixels("video/x-raw, colorimetry=(string)bt709, framerate", 1080), Pixels::I420 { bt709: true, full_range: false });
            assert_eq!(pixels("video/x-raw, colorimetry=(string)2:4:0:0, framerate", 180), Pixels::I420 { bt709: false, full_range: false });
            assert_eq!(pixels("colorimetry=(string)1:3:5:1", 180), Pixels::I420 { bt709: true, full_range: true });
            assert_eq!(pixels("no colorimetry", 1080), Pixels::I420 { bt709: true, full_range: false });
            assert_eq!(pixels("no colorimetry", 480), Pixels::I420 { bt709: false, full_range: false });
        }
    }

    #[test]
    fn flattening_is_opaque_black_bgrx() {
        let mut out = [0_u8; 8];
        rgba_to_bgrx(&[10, 20, 30, 255, 200, 100, 50, 0], &mut out);
        assert_eq!(out, [30, 20, 10, 255, 0, 0, 0, 255]);
    }
}
