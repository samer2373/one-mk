use crate::{Format, MAX_DECODED_PIXELS, MAX_FRAMES, Media};
use image::{AnimationDecoder, ImageDecoder};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Decoded {
    pub media: Media,
    pub poster: Vec<u8>,
}

pub(crate) fn decode(bytes: &[u8], mut media: Media) -> Result<Decoded, String> {
    if !matches!(media.format, Format::Vp9Webm | Format::Av1Webm) { crate::sandbox::deny_subprocesses()?; }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(2160);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let poster = match media.format {
        Format::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
            decoder.set_limits(limits).map_err(|error| error.to_string())?;
            animation(decoder.into_frames(), &mut media)?
        }
        Format::Apng => {
            let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
            decoder.set_limits(limits).map_err(|error| error.to_string())?;
            animation(decoder.apng().map_err(|error| error.to_string())?.into_frames(), &mut media)?
        }
        Format::Webp => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
            decoder.set_limits(limits).map_err(|error| error.to_string())?;
            animation(decoder.into_frames(), &mut media)?
        }
        Format::Vp9Webm | Format::Av1Webm => webm(&media)?,
    };
    Ok(Decoded { media, poster })
}

fn animation(frames: image::Frames<'_>, media: &mut Media) -> Result<Vec<u8>, String> {
    let mut count = 0u32;
    let mut pixels = 0u64;
    let mut duration = 0u64;
    let mut poster = None;
    for frame in frames {
        let frame = frame.map_err(|error| error.to_string())?;
        let image = frame.buffer();
        if image.dimensions() != (media.width, media.height) { return Err("decoded dimensions changed".into()); }
        count += 1;
        pixels += u64::from(image.width()) * u64::from(image.height());
        if count > MAX_FRAMES || pixels > MAX_DECODED_PIXELS { return Err("decoded frame/pixel budget exceeded".into()); }
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = (u64::from(numerator) / u64::from(denominator)).max(10);
        if delay < 17 { return Err("decoded source exceeds 60 fps; convert first".into()); }
        duration = duration.checked_add(delay).ok_or("duration overflow")?;
        if duration > crate::MAX_DURATION_MS { return Err("decoded duration limit exceeded".into()); }
        if poster.is_none() {
            let rgb = image::RgbImage::from_fn(image.width(), image.height(), |left, top| {
                let pixel = image.get_pixel(left, top).0;
                let alpha = u16::from(pixel[3]);
                image::Rgb([((u16::from(pixel[0]) * alpha + 127) / 255) as u8, ((u16::from(pixel[1]) * alpha + 127) / 255) as u8, ((u16::from(pixel[2]) * alpha + 127) / 255) as u8])
            });
            poster = Some(poster_bytes(&rgb));
        }
    }
    if count != media.frames { return Err("decoded frame count differs from container".into()); }
    media.duration_ms = duration;
    poster.ok_or_else(|| "no decoded poster".into())
}

fn poster_bytes(image: &image::RgbImage) -> Vec<u8> {
    let thumbnail = image::DynamicImage::ImageRgb8(image.clone()).thumbnail(320, 180).to_rgb8();
    let mut bytes = format!("P6\n{} {}\n255\n", thumbnail.width(), thumbnail.height()).into_bytes();
    bytes.extend_from_slice(thumbnail.as_raw());
    bytes
}

fn webm(media: &Media) -> Result<Vec<u8>, String> {
    let decoder = match media.format { Format::Vp9Webm => "vp9", Format::Av1Webm => "av1", _ => unreachable!() };
    let mut child = std::process::Command::new("/worker")
        .args(["--decode-webm", decoder, &media.width.to_string(), &media.height.to_string()])
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
        .spawn().map_err(|error| format!("pinned GStreamer unavailable: {error}"))?;
    crate::checkpoint("decoder");
    if let Err(error) = crate::sandbox::deny_subprocesses() { let _ = child.kill(); let _ = child.wait(); return Err(error); }
    let result = (|| {
        let mut output = child.stdout.take().ok_or("missing decoder stream")?;
        let stride = (media.width as usize * 3).div_ceil(4) * 4;
        let mut frame = vec![0; stride * media.height as usize];
        let mut poster = None;
        let mut frames = 0u32;
        loop {
            let first = output.read(&mut frame[..1]).map_err(|error| error.to_string())?;
            if first == 0 { break; }
            output.read_exact(&mut frame[1..]).map_err(|_| "truncated decoded frame")?;
            frames += 1;
            if frames > media.frames || u64::from(frames) * u64::from(media.width) * u64::from(media.height) > MAX_DECODED_PIXELS { return Err("decoded pixel/frame budget exceeded".into()); }
            if poster.is_none() {
                let mut image = image::RgbImage::new(media.width, media.height);
                for (row, pixels) in image.as_mut().chunks_mut(media.width as usize * 3).enumerate() {
                    pixels.copy_from_slice(&frame[row * stride..row * stride + pixels.len()]);
                }
                poster = Some(poster_bytes(&image));
            }
        }
        if frames != media.frames { return Err("decoded WebM frame count differs from container".into()); }
        poster.ok_or_else(|| "no decoded WebM frame".into())
    })();
    if result.is_err() { let _ = child.kill(); }
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() { return Err("pinned WebM decoder rejected media".into()); }
    result
}

#[cfg(target_os = "linux")]
pub(crate) fn exec_webm(codec: &str, width: u32, height: u32) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    crate::sandbox::check_worker()?;
    crate::sandbox::deny_subprocesses()?;
    crate::dimensions(width, height)?;
    let decoder = match codec { "vp9" => "vp9dec", "av1" => "av1dec", _ => return Err("unsupported codec; convert first".into()) };
    let caps = format!("video/x-raw,format=RGB,width={width},height={height}");
    let error = std::process::Command::new("/bin/gst-launch-1.0")
        .args(["--quiet", "filesrc", "location=/input", "!", "matroskademux", "!", decoder, "!", "videoconvert", "!", &caps, "!", "fdsink", "fd=1", "sync=false"])
        .exec();
    Err(format!("pinned decoder exec failed: {error}"))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn exec_webm(_codec: &str, _width: u32, _height: u32) -> Result<(), String> { crate::sandbox::worker_restrictions() }
