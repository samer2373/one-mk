use crate::{MAX_DURATION_MS, MAX_FRAMES, MAX_METADATA, MAX_SOURCE, dimensions};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format { Gif, Apng, Webp, Vp9Webm, Av1Webm }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub duration_ms: u64,
}

pub fn inspect(bytes: &[u8]) -> Result<Media, String> {
    if bytes.len() as u64 > MAX_SOURCE { return Err("source exceeds 512 MiB".into()); }
    let media = if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        gif(bytes)?
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)?
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        webp(bytes)?
    } else if bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        webm(bytes)?
    } else {
        return Err("unsupported media: convert first to VP9/AV1 WebM, GIF, APNG or animated WebP".into());
    };
    validate_media(&media)?;
    Ok(media)
}

pub(crate) fn validate_media(media: &Media) -> Result<(), String> {
    dimensions(media.width, media.height)?;
    if media.frames < 2 || media.frames > MAX_FRAMES || media.duration_ms > MAX_DURATION_MS {
        return Err("animation requires 2..36000 frames and duration <=600 seconds; convert first".into());
    }
    if media.duration_ms == 0 || u64::from(media.frames) * 1000 > media.duration_ms * 60 {
        return Err("source exceeds 60 fps; convert first".into());
    }
    if u64::from(media.width) * u64::from(media.height) * u64::from(media.frames) > crate::MAX_DECODED_PIXELS {
        return Err("decoded pixel budget exceeds 1,000,000,000; convert first to fewer frames, shorter duration or lower resolution".into());
    }
    Ok(())
}

struct Cursor<'a> { bytes: &'a [u8], position: usize }
impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self.position.checked_add(length).ok_or("length overflow")?;
        let value = self.bytes.get(self.position..end).ok_or("truncated media")?;
        self.position = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    fn blocks(&mut self) -> Result<usize, String> {
        let mut total = 0usize;
        loop {
            let length = usize::from(self.byte()?);
            if length == 0 { return Ok(total); }
            self.take(length)?;
            total = total.checked_add(length).ok_or("block overflow")?;
        }
    }
}
fn le16(bytes: &[u8]) -> u32 { u32::from(u16::from_le_bytes([bytes[0], bytes[1]])) }
fn le24(bytes: &[u8]) -> u32 { u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]) }
fn le32(bytes: &[u8]) -> u32 { u32::from_le_bytes(bytes[..4].try_into().expect("four-byte field")) }
fn be32(bytes: &[u8]) -> u32 { u32::from_be_bytes(bytes[..4].try_into().expect("four-byte field")) }

fn gif(bytes: &[u8]) -> Result<Media, String> {
    let mut cursor = Cursor { bytes, position: 6 };
    let header = cursor.take(7)?;
    let (width, height) = (le16(header), le16(&header[2..]));
    dimensions(width, height)?;
    if header[4] & 0x80 != 0 { cursor.take(3usize << ((header[4] & 7) + 1))?; }
    let mut media = Media { format: Format::Gif, width, height, frames: 0, duration_ms: 0 };
    let mut delay = 100;
    let mut metadata = 0;
    loop {
        match cursor.byte()? {
            0x21 => {
                let label = cursor.byte()?;
                if label == 0xf9 {
                    if cursor.byte()? != 4 { return Err("invalid GIF control".into()); }
                    let control = cursor.take(4)?;
                    delay = u64::from(le16(&control[1..])) * 10;
                    if cursor.byte()? != 0 { return Err("invalid GIF terminator".into()); }
                    metadata += 8;
                } else { metadata += cursor.blocks()?; }
                if metadata > MAX_METADATA { return Err("metadata exceeds 256 KiB".into()); }
            }
            0x2c => {
                let frame = cursor.take(9)?;
                let (left, top, frame_width, frame_height) = (le16(frame), le16(&frame[2..]), le16(&frame[4..]), le16(&frame[6..]));
                dimensions(frame_width, frame_height)?;
                if left + frame_width > width || top + frame_height > height { return Err("GIF frame outside canvas".into()); }
                if frame[8] & 0x80 != 0 { cursor.take(3usize << ((frame[8] & 7) + 1))?; }
                if !(2..=8).contains(&cursor.byte()?) { return Err("invalid GIF LZW width".into()); }
                cursor.blocks()?;
                add_frame(&mut media, delay.max(10))?;
                delay = 100;
            }
            0x3b if cursor.position == bytes.len() => return Ok(media),
            _ => return Err("invalid GIF or trailing polyglot data".into()),
        }
    }
}

fn add_frame(media: &mut Media, delay: u64) -> Result<(), String> {
    if delay < 17 { return Err("source exceeds 60 fps; convert first".into()); }
    media.frames += 1;
    media.duration_ms = media.duration_ms.checked_add(delay).ok_or("duration overflow")?;
    if media.frames > MAX_FRAMES || media.duration_ms > MAX_DURATION_MS { return Err("frame/duration limit exceeded; convert first".into()); }
    if u64::from(media.width) * u64::from(media.height) * u64::from(media.frames) > crate::MAX_DECODED_PIXELS { return Err("decoded pixel budget exceeded; convert first to fewer frames or lower resolution".into()); }
    Ok(())
}

fn png(bytes: &[u8]) -> Result<Media, String> {
    let mut cursor = Cursor { bytes, position: 8 };
    let mut media = Media { format: Format::Apng, width: 0, height: 0, frames: 0, duration_ms: 0 };
    let mut declared = None;
    let mut metadata = 0usize;
    loop {
        let length = be32(cursor.take(4)?) as usize;
        let kind = cursor.take(4)?;
        let chunk = cursor.take(length)?;
        cursor.take(4)?;
        if kind != b"IDAT" && kind != b"fdAT" {
            metadata = metadata.checked_add(length + 12).ok_or("metadata overflow")?;
            if metadata > MAX_METADATA { return Err("metadata exceeds 256 KiB".into()); }
        }
        match kind {
            b"IHDR" if length == 13 && cursor.position == 33 => {
                media.width = be32(chunk); media.height = be32(&chunk[4..]);
                dimensions(media.width, media.height)?;
            }
            b"acTL" if length == 8 && declared.is_none() => { declared = Some(be32(chunk)); }
            b"fcTL" if length == 26 => {
                let (width, height, left, top) = (be32(&chunk[4..]), be32(&chunk[8..]), be32(&chunk[12..]), be32(&chunk[16..]));
                dimensions(width, height)?;
                if u64::from(left) + u64::from(width) > u64::from(media.width) || u64::from(top) + u64::from(height) > u64::from(media.height) { return Err("APNG frame outside canvas".into()); }
                let numerator = u16::from_be_bytes([chunk[20], chunk[21]]);
                let denominator = u16::from_be_bytes([chunk[22], chunk[23]]);
                let delay = (u64::from(numerator) * 1000 / u64::from(if denominator == 0 { 100 } else { denominator })).max(10);
                add_frame(&mut media, delay)?;
            }
            b"IEND" if length == 0 && cursor.position == bytes.len() && declared == Some(media.frames) => return Ok(media),
            b"IEND" => return Err("static PNG, invalid animation or trailing polyglot; convert first".into()),
            b"IHDR" | b"acTL" | b"fcTL" => return Err("invalid APNG structure".into()),
            b"iCCP" | b"zTXt" | b"iTXt" | b"eXIf" => return Err("compressed/extended image metadata is not accepted; convert first".into()),
            _ => {}
        }
    }
}

fn webp(bytes: &[u8]) -> Result<Media, String> {
    let mut cursor = Cursor { bytes, position: 4 };
    if u64::from(le32(cursor.take(4)?)) + 8 != bytes.len() as u64 { return Err("WebP length mismatch/polyglot".into()); }
    cursor.take(4)?;
    let mut media = Media { format: Format::Webp, width: 0, height: 0, frames: 0, duration_ms: 0 };
    let mut animation = false;
    let mut metadata = 0usize;
    while cursor.position < bytes.len() {
        let kind = cursor.take(4)?;
        let length = le32(cursor.take(4)?) as usize;
        let chunk = cursor.take(length)?;
        if length & 1 != 0 && cursor.byte()? != 0 { return Err("invalid WebP padding".into()); }
        match kind {
            b"VP8X" if length == 10 && cursor.position == 30 && chunk[0] & 2 != 0 => {
                media.width = le24(&chunk[4..]) + 1; media.height = le24(&chunk[7..]) + 1;
                dimensions(media.width, media.height)?;
            }
            b"ANIM" if length == 6 && !animation => animation = true,
            b"ANMF" if length >= 16 && animation => {
                let (left, top, width, height) = (le24(chunk) * 2, le24(&chunk[3..]) * 2, le24(&chunk[6..]) + 1, le24(&chunk[9..]) + 1);
                dimensions(width, height)?;
                if left + width > media.width || top + height > media.height { return Err("WebP frame outside canvas".into()); }
                add_frame(&mut media, u64::from(le24(&chunk[12..])).max(10))?;
            }
            b"VP8X" | b"ANIM" | b"ANMF" => return Err("invalid/static WebP; convert first".into()),
            _ => {
                metadata = metadata.checked_add(length).ok_or("metadata overflow")?;
                if metadata > MAX_METADATA { return Err("metadata exceeds 256 KiB".into()); }
            }
        }
    }
    Ok(media)
}

fn vint(cursor: &mut Cursor<'_>, identifier: bool) -> Result<u64, String> {
    let first = cursor.byte()?;
    let length = first.leading_zeros() as usize + 1;
    if length > if identifier { 4 } else { 8 } { return Err("invalid EBML integer".into()); }
    let mut value = u64::from(if identifier { first } else { first & (0xffu16 >> length) as u8 });
    for byte in cursor.take(length - 1)? { value = (value << 8) | u64::from(*byte); }
    if !identifier && value == (1u64 << (length * 7)) - 1 { return Ok(u64::MAX); }
    Ok(value)
}

fn elements<'a>(bytes: &'a [u8], mut visit: impl FnMut(u64, &'a [u8]) -> Result<(), String>) -> Result<(), String> {
    let mut cursor = Cursor { bytes, position: 0 };
    let mut count = 0;
    while cursor.position < bytes.len() {
        count += 1;
        if count > 100_000 { return Err("EBML element limit".into()); }
        let identifier = vint(&mut cursor, true)?;
        let size = vint(&mut cursor, false)?;
        let size = if size == u64::MAX && identifier == 0x18538067 { bytes.len() - cursor.position }
            else { usize::try_from(size).map_err(|_| "unknown/oversize EBML element")? };
        visit(identifier, cursor.take(size)?)?;
    }
    Ok(())
}
fn integer(bytes: &[u8]) -> Result<u64, String> {
    if bytes.is_empty() || bytes.len() > 8 { return Err("invalid EBML field".into()); }
    Ok(bytes.iter().fold(0, |value, byte| (value << 8) | u64::from(*byte)))
}

fn webm(bytes: &[u8]) -> Result<Media, String> {
    let mut media = Media { format: Format::Vp9Webm, width: 0, height: 0, frames: 0, duration_ms: 0 };
    let mut header = false;
    let mut segment = false;
    let mut tracks = false;
    let mut scale = 1_000_000u64;
    let mut last_time = None;
    let mut metadata = 0usize;
    let mut declared_duration = None;
    elements(bytes, |identifier, payload| {
        match identifier {
            0x1a45dfa3 if !header && !segment => {
                metadata = payload.len();
                if metadata > MAX_METADATA { return Err("metadata exceeds 256 KiB".into()); }
                elements(payload, |identifier, value| {
                    if identifier == 0x4282 { header = value == b"webm" || value == b"webm\0"; }
                    Ok(())
                })?;
                if !header { return Err("only WebM accepted, not Matroska; convert first".into()); }
            }
            0x18538067 if header && !segment => {
                segment = true;
                elements(payload, |identifier, value| {
                    match identifier {
                        0x1549a966 if !tracks => {
                            metadata = metadata.checked_add(value.len()).ok_or("metadata overflow")?;
                            elements(value, |identifier, field| {
                            if identifier == 0x2ad7b1 { scale = integer(field)?; }
                            if identifier == 0x4489 {
                                let value = match field.len() {
                                    4 => f64::from(f32::from_be_bytes(field.try_into().map_err(|_| "invalid duration")?)),
                                    8 => f64::from_be_bytes(field.try_into().map_err(|_| "invalid duration")?),
                                    _ => return Err("invalid WebM duration".into()),
                                };
                                if !value.is_finite() || value <= 0.0 { return Err("invalid WebM duration".into()); }
                                declared_duration = Some(value);
                            }
                            if scale == 0 || scale > 1_000_000_000 { return Err("invalid WebM timebase".into()); }
                            Ok(())
                            })?;
                        }
                        0x1654ae6b if !tracks => {
                            metadata = metadata.checked_add(value.len()).ok_or("metadata overflow")?;
                            if metadata > MAX_METADATA { return Err("metadata exceeds 256 KiB".into()); }
                            let mut track_count = 0;
                            elements(value, |identifier, track| {
                                if identifier != 0xae { return Err("unknown track field; convert first".into()); }
                                track_count += 1;
                                if track_count != 1 { return Err("extra video/audio tracks forbidden; convert first".into()); }
                                let mut video = false;
                                let mut codec = false;
                                elements(track, |identifier, field| {
                                    match identifier {
                                        0x83 => video = integer(field)? == 1,
                                        0xd7 => { if integer(field)? != 1 { return Err("only track 1 accepted; convert first".into()); } }
                                        0x86 => { media.format = match field { b"V_VP9" | b"V_VP9\0" => Format::Vp9Webm, b"V_AV1" | b"V_AV1\0" => Format::Av1Webm, _ => return Err("only VP9/AV1 accepted; convert first".into()) }; codec = true; }
                                        0xe0 => elements(field, |identifier, dimension| {
                                            match identifier { 0xb0 => media.width = u32::try_from(integer(dimension)?).map_err(|_| "width overflow")?, 0xba => media.height = u32::try_from(integer(dimension)?).map_err(|_| "height overflow")?, _ => {} }
                                            Ok(())
                                        })?,
                                        0x6d80 => return Err("encoded/encrypted track forbidden; convert first".into()),
                                        0x73c5 | 0x9c | 0x23e383 | 0x55ee | 0x22b59c | 0x536e | 0xb9 | 0x88 | 0x55aa | 0x63a2 | 0x258688 => {}
                                        _ => return Err("unknown/external-reference track field; convert first".into()),
                                    }
                                    Ok(())
                                })?;
                                if !video || !codec { return Err("missing VP9/AV1 video track; convert first".into()); }
                                dimensions(media.width, media.height)
                            })?;
                            tracks = track_count == 1;
                        }
                        0x1f43b675 if tracks => {
                            let mut cluster_time = 0;
                            elements(value, |identifier, block| {
                                match identifier {
                                    0xe7 => cluster_time = integer(block)?,
                                    0xa3 => {
                                        if block.len() < 5 || block[0] != 0x81 || block[3] & 0x06 != 0 { return Err("lacing/extra tracks unsupported; convert first".into()); }
                                        let relative = i64::from(i16::from_be_bytes([block[1], block[2]]));
                                        let ticks = i64::try_from(cluster_time).map_err(|_| "timestamp overflow")?.checked_add(relative).ok_or("timestamp overflow")?;
                                        let time = u64::try_from(ticks).map_err(|_| "negative timestamp")?.checked_mul(scale).ok_or("timestamp overflow")?;
                                        if time > MAX_DURATION_MS * 1_000_000 || last_time.is_some_and(|previous| time <= previous || time - previous < 16_000_000) { return Err("nonmonotonic/excessive WebM timeline".into()); }
                                        let tail = last_time.map_or(17_000_000, |previous| (time - previous).max(17_000_000));
                                        last_time = Some(time);
                                        media.frames += 1;
                                        if media.frames > MAX_FRAMES { return Err("too many WebM frames".into()); }
                                        media.duration_ms = time.checked_add(tail).ok_or("duration overflow")?.div_ceil(1_000_000);
                                    }
                                    0xa7 | 0xab | 0xbf | 0xec => {}
                                    _ => return Err("unsupported WebM block structure; convert first".into()),
                                }
                                Ok(())
                            })?;
                        }
                        0x114d9b74 | 0x1c53bb6b | 0xec | 0x1254c367 => {
                            metadata = metadata.checked_add(value.len()).ok_or("metadata overflow")?;
                        }
                        _ => return Err("attachments/audio/subtitles/unsupported WebM structure; convert first".into()),
                    }
                    if metadata > MAX_METADATA { return Err("metadata exceeds 256 KiB".into()); }
                    Ok(())
                })?;
            }
            _ => return Err("invalid WebM/trailing polyglot; convert first".into()),
        }
        Ok(())
    })?;
    if !tracks || !segment { return Err("incomplete WebM".into()); }
    let duration = declared_duration.ok_or("finite WebM duration required; convert first")? * scale as f64 / 1_000_000.0;
    if !duration.is_finite() || duration > MAX_DURATION_MS as f64 || last_time.is_none_or(|time| duration <= time as f64 / 1_000_000.0) || (duration - media.duration_ms as f64).abs() > 2.0 {
        return Err("declared WebM duration contradicts bounded decoded timeline; convert first".into());
    }
    media.duration_ms = duration.round() as u64;
    Ok(media)
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn independent_admission_limits_reach_their_boundaries() {
        let mut media = Media { format: Format::Gif, width: 1, height: 1, frames: MAX_FRAMES, duration_ms: MAX_DURATION_MS };
        assert!(validate_media(&media).is_ok());
        media.frames += 1;
        assert!(validate_media(&media).unwrap_err().contains("36000"));
        media.frames = MAX_FRAMES;
        media.duration_ms += 1;
        assert!(validate_media(&media).unwrap_err().contains("600 seconds"));
        media.duration_ms = MAX_DURATION_MS - 1;
        assert!(validate_media(&media).unwrap_err().contains("60 fps"));
        media = Media { format: Format::Gif, width: 1000, height: 1000, frames: 1000, duration_ms: 100_000 };
        assert!(validate_media(&media).is_ok());
        media.frames += 1;
        assert!(validate_media(&media).unwrap_err().contains("pixel budget"));
    }
}
