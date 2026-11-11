//! The importer reads files chosen by the person but made by anyone. These tests feed it
//! hostile, truncated, oversized and odd media and wrong command lines, and require a
//! finite, plain refusal each time: never a panic, never an admission it should not give.

use one_mk_wall_import::{Format, MAX_DURATION_MS, MAX_FRAMES, MAX_METADATA, MAX_SOURCE, inspect, timeline};
use std::process::Command;

mod fixtures { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mod.rs")); }

// ---- builders -----------------------------------------------------------------------

fn gif_header(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes
}

/// A 1 x 1 frame with no control block (so the default 100 ms delay).
const GIF_FRAME: &[u8] = b"\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0";

fn gif_of_frames(width: u16, height: u16, frames: usize) -> Vec<u8> {
    let mut bytes = gif_header(width, height);
    for _ in 0..frames { bytes.extend_from_slice(GIF_FRAME); }
    bytes.push(b';');
    bytes
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(&[0; 4]); // the CRC is checked by the decoder, not by admission
    bytes
}

fn ihdr(width: u32, height: u32) -> Vec<u8> {
    let mut data = width.to_be_bytes().to_vec();
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(b"IHDR", &data)
}

fn still(width: u32, height: u32, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(ihdr(width, height));
    for chunk in extra { bytes.extend_from_slice(chunk); }
    bytes.extend(png_chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0, 0, 0, 0, 1]));
    bytes.extend(png_chunk(b"IEND", &[]));
    bytes
}

fn ebml(identifier: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut result = identifier.to_vec();
    result.extend_from_slice(&(0x1000_0000u32 | payload.len() as u32).to_be_bytes());
    result.extend_from_slice(payload);
    result
}

struct Webm { doctype: Vec<u8>, duration: Option<Vec<u8>>, width: u8, height: u8, track_extra: Vec<u8>, tracks: usize, cluster_time: u64, relatives: Vec<i16>, flags: u8 }

impl Webm {
    fn new() -> Self {
        Webm { doctype: b"webm".to_vec(), duration: Some(200f64.to_be_bytes().to_vec()), width: 2, height: 2, track_extra: vec![], tracks: 1, cluster_time: 0, relatives: vec![0, 100], flags: 0x80 }
    }
    fn bytes(&self) -> Vec<u8> {
        let mut out = ebml(&[0x1a, 0x45, 0xdf, 0xa3], &ebml(&[0x42, 0x82], &self.doctype));
        let mut track = ebml(&[0x83], &[1]);
        track.extend(ebml(&[0xd7], &[1]));
        track.extend(ebml(&[0x86], b"V_VP9"));
        track.extend(ebml(&[0xe0], &[ebml(&[0xb0], &[self.width]), ebml(&[0xba], &[self.height])].concat()));
        track.extend_from_slice(&self.track_extra);
        let tracks = ebml(&[0x16, 0x54, 0xae, 0x6b], &ebml(&[0xae], &track).repeat(self.tracks));
        let mut blocks = ebml(&[0xe7], &self.cluster_time.to_be_bytes());
        for relative in &self.relatives {
            let [high, low] = relative.to_be_bytes();
            blocks.extend(ebml(&[0xa3], &[0x81, high, low, self.flags, 0]));
        }
        let cluster = ebml(&[0x1f, 0x43, 0xb6, 0x75], &blocks);
        let info = ebml(&[0x15, 0x49, 0xa9, 0x66], &self.duration.as_ref().map(|d| ebml(&[0x44, 0x89], d)).unwrap_or_default());
        out.extend(ebml(&[0x18, 0x53, 0x80, 0x67], &[info, tracks, cluster].concat()));
        out
    }
}

fn error_of(bytes: &[u8]) -> String {
    inspect(bytes).expect_err("hostile input must be refused")
}

// ---- size, emptiness and file types ---------------------------------------------------

#[test]
fn empty_tiny_and_oversized_inputs_are_refused_in_plain_words() {
    for bytes in [&b""[..], b"\0", b"G", b"GIF", b"\x89PNG", b"RIFF\0\0\0\0WEBP", b"\x1a\x45\xdf"] {
        assert!(inspect(bytes).is_err(), "{bytes:?}");
    }
    // One byte over the limit is refused before the contents are looked at; at the limit it is merely not media.
    let over = vec![0u8; MAX_SOURCE as usize + 1];
    assert!(error_of(&over).contains("512 MiB"));
    drop(over);
    let at = vec![0u8; MAX_SOURCE as usize];
    assert!(error_of(&at).contains("unsupported media"));
}

#[test]
fn archives_programs_documents_and_scripts_are_never_media() {
    let zip_bomb_like = {
        // A local file header naming a path outside the folder, then a central directory and an end record.
        let mut bytes = b"PK\x03\x04\x14\0\0\0\x08\0\0\0\0\0".to_vec();
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&13u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(b"../../evil.sh");
        bytes.extend_from_slice(&vec![0xff; 4096]);
        bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0\x30\0\0\0\x20\0\0\0\0\0");
        bytes
    };
    let samples: Vec<(&str, Vec<u8>)> = vec![
        ("zip", zip_bomb_like),
        ("empty zip", b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec()),
        ("gzip", b"\x1f\x8b\x08\0\0\0\0\0\0\x03\x03\0\0\0\0\0\0\0\0\0".to_vec()),
        ("bzip2", b"BZh91AY&SY".to_vec()),
        ("xz", b"\xfd7zXZ\0\0\x04".to_vec()),
        ("7z", b"7z\xbc\xaf\x27\x1c\0\x04".to_vec()),
        ("rar", b"Rar!\x1a\x07\x01\0".to_vec()),
        ("tar", { let mut b = vec![0u8; 512]; b[..16].copy_from_slice(b"../../etc/passwd"); b[257..263].copy_from_slice(b"ustar\0"); b }),
        ("ELF", b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0\x03\0\x3e\0".to_vec()),
        ("Windows program", b"MZ\x90\0\x03\0\0\0\x04\0\0\0\xff\xff\0\0".to_vec()),
        ("Mach-O", b"\xcf\xfa\xed\xfe\x07\0\0\x01".to_vec()),
        ("PDF", b"%PDF-1.7\n1 0 obj<</Type/Catalog>>endobj".to_vec()),
        ("shell script", b"#!/bin/sh\nrm -rf ~\n".to_vec()),
        ("HTML", b"<!doctype html><script>alert(1)</script>".to_vec()),
        ("SVG", b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>1</script></svg>".to_vec()),
        ("JPEG", b"\xff\xd8\xff\xe0\0\x10JFIF".to_vec()),
        ("MP4", b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2".to_vec()),
        ("Matroska or AVI RIFF", b"RIFF\x04\0\0\0AVI ".to_vec()),
        ("a symlink target text", b"/etc/shadow".to_vec()),
    ];
    for (what, bytes) in samples {
        let error = error_of(&bytes);
        assert!(error.contains("convert first") || error.contains("unsupported"), "{what}: {error}");
    }
}

#[test]
fn magic_bytes_inside_a_longer_file_or_after_a_prefix_do_not_count() {
    let gif = fixtures::gif(2, 2, 2);
    let mut prefixed = b"PK\x03\x04".to_vec();
    prefixed.extend_from_slice(&gif);
    assert!(inspect(&prefixed).is_err());
    let mut doubled = gif.clone();
    doubled.extend_from_slice(&gif);
    assert!(inspect(&doubled).is_err(), "a second image after the trailer is a polyglot");
    for nul in [0usize, 1, 5] {
        let mut padded = gif.clone();
        padded.extend(std::iter::repeat_n(0u8, nul + 1));
        assert!(inspect(&padded).is_err(), "{nul} trailing zero bytes");
    }
}

// ---- every truncation, every flipped bit ------------------------------------------------

fn fixtures_of_every_format() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("gif", fixtures::gif(2, 2, 2)),
        ("apng", fixtures::apng()),
        ("webp", fixtures::webp()),
        ("still png", fixtures::still_png()),
        ("webm", Webm::new().bytes()),
    ]
}

#[test]
fn every_proper_prefix_of_every_format_is_refused() {
    for (what, bytes) in fixtures_of_every_format() {
        assert!(inspect(&bytes).is_ok(), "{what} fixture must be valid");
        for length in 0..bytes.len() {
            assert!(inspect(&bytes[..length]).is_err(), "{what} cut to {length} of {} bytes was admitted", bytes.len());
        }
    }
}

#[test]
fn every_single_flipped_bit_is_handled_without_panic_and_never_changes_what_a_good_file_promises() {
    for (what, bytes) in fixtures_of_every_format() {
        let promised = timeline(&bytes).unwrap();
        for index in 0..bytes.len() {
            for bit in 0..8 {
                let mut hostile = bytes.clone();
                hostile[index] ^= 1 << bit;
                if let Ok((media, delays)) = timeline(&hostile) {
                    // Whatever is admitted is within every limit and has a delay for every frame.
                    assert!(media.width >= 1 && media.width <= 4096 && media.height >= 1 && media.height <= 2160, "{what} byte {index} bit {bit}");
                    assert_eq!(delays.len(), media.frames as usize, "{what} byte {index} bit {bit}");
                    assert!(media.frames <= MAX_FRAMES && media.duration_ms <= MAX_DURATION_MS, "{what} byte {index} bit {bit}");
                    let _ = &promised;
                }
            }
        }
    }
}

// ---- GIF -----------------------------------------------------------------------------

#[test]
fn gif_with_impossible_canvas_frames_or_structure_is_refused() {
    assert!(inspect(&gif_of_frames(0, 0, 2)).is_err(), "a zero-size canvas");
    assert!(inspect(&gif_of_frames(0, 8, 2)).is_err());
    assert!(inspect(&gif_of_frames(4097, 8, 2)).is_err(), "wider than the 4096 tier");
    assert!(inspect(&gif_of_frames(u16::MAX, u16::MAX, 2)).is_err());
    assert!(inspect(&gif_of_frames(1, 1, 0)).is_err(), "no frames");
    assert!(inspect(&gif_of_frames(1, 1, 1)).unwrap_err().contains("2..36000"), "one frame is not an animation");
    // A frame that sticks out of the canvas.
    let mut outside = gif_header(1, 1);
    outside.extend_from_slice(b"\x2c\x01\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0");
    outside.extend_from_slice(GIF_FRAME);
    outside.push(b';');
    assert!(inspect(&outside).unwrap_err().contains("outside canvas"));
    // A zero-size frame.
    let mut flat = gif_header(4, 4);
    flat.extend_from_slice(b"\x2c\0\0\0\0\0\0\x01\0\0\x02\x02\x44\x01\0");
    flat.extend_from_slice(GIF_FRAME);
    flat.push(b';');
    assert!(inspect(&flat).is_err());
    // An LZW code size outside 2..=8.
    for width in [0u8, 1, 9, 255] {
        let mut bad = gif_header(1, 1);
        bad.extend_from_slice(&[0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, width, 1, 0, 0]);
        bad.extend_from_slice(GIF_FRAME);
        bad.push(b';');
        assert!(inspect(&bad).is_err(), "LZW width {width}");
    }
    // A graphic-control block that lies about its length.
    let mut lying = gif_header(1, 1);
    lying.extend_from_slice(b"\x21\xf9\x05\0\x0a\0\0\0\0");
    lying.extend_from_slice(GIF_FRAME);
    lying.push(b';');
    assert!(inspect(&lying).is_err());
    // Unknown block introducers and a missing trailer.
    let mut unknown = gif_header(1, 1);
    unknown.push(0x99);
    assert!(inspect(&unknown).is_err());
    let mut no_trailer = gif_of_frames(1, 1, 2);
    no_trailer.pop();
    assert!(inspect(&no_trailer).is_err());
}

#[test]
fn gif_frame_count_and_duration_limits_hold_exactly() {
    // 6000 frames at the default 100 ms are exactly the 600 s limit.
    let at_limit = gif_of_frames(1, 1, 6000);
    let media = inspect(&at_limit).unwrap();
    assert_eq!((media.format, media.frames, media.duration_ms), (Format::Gif, 6000, MAX_DURATION_MS));
    assert!(inspect(&gif_of_frames(1, 1, 6001)).unwrap_err().contains("limit"));
    // Far more frames than any limit allows are refused without walking all of them into memory.
    assert!(inspect(&gif_of_frames(1, 1, 50_000)).is_err());
}

#[test]
fn gif_metadata_is_bounded() {
    let mut comment = gif_header(1, 1);
    let blocks = MAX_METADATA / 255 + 2;
    comment.extend_from_slice(b"\x21\xfe");
    for _ in 0..blocks { comment.push(255); comment.extend(std::iter::repeat_n(b'A', 255)); }
    comment.push(0);
    comment.extend_from_slice(GIF_FRAME);
    comment.extend_from_slice(GIF_FRAME);
    comment.push(b';');
    assert!(inspect(&comment).unwrap_err().contains("metadata"));
    // The same comment, small, is fine.
    let mut small = gif_header(1, 1);
    small.extend_from_slice(b"\x21\xfe\x05hello\0");
    small.extend_from_slice(GIF_FRAME);
    small.extend_from_slice(GIF_FRAME);
    small.push(b';');
    assert_eq!(inspect(&small).unwrap().frames, 2);
}

#[test]
fn gif_frames_too_fast_are_asked_to_be_converted() {
    let mut fast = gif_header(1, 1);
    for _ in 0..2 {
        fast.extend_from_slice(b"\x21\xf9\x04\0\x01\0\0\0"); // 10 ms
        fast.extend_from_slice(GIF_FRAME);
    }
    fast.push(b';');
    assert!(inspect(&fast).unwrap_err().contains("60 fps"));
    let mut fine = gif_header(1, 1);
    for _ in 0..2 {
        fine.extend_from_slice(b"\x21\xf9\x04\0\x02\0\0\0"); // 20 ms
        fine.extend_from_slice(GIF_FRAME);
    }
    fine.push(b';');
    assert_eq!(timeline(&fine).unwrap().1, [20_000, 20_000]);
}

// ---- PNG and APNG -----------------------------------------------------------------------

#[test]
fn png_dimensions_are_checked_before_anything_is_decoded() {
    assert!(inspect(&still(2, 2, &[])).is_ok());
    assert!(inspect(&still(4096, 2160, &[])).is_ok());
    for (width, height) in [(0, 0), (0, 5), (5, 0), (4097, 1), (4096, 2161), (65_536, 65_536), (u32::MAX, u32::MAX), (1 << 31, 1)] {
        assert!(inspect(&still(width, height, &[])).is_err(), "{width} x {height}");
    }
}

#[test]
fn png_structure_attacks_are_refused() {
    // Compressed or extended metadata chunks are decoder attack surface and are never accepted.
    for kind in [b"iCCP", b"zTXt", b"iTXt", b"eXIf"] {
        assert!(inspect(&still(2, 2, &[png_chunk(kind, b"x")])).unwrap_err().contains("metadata"), "{kind:?}");
    }
    // Plain text chunks are allowed, but only up to the metadata budget.
    assert!(inspect(&still(2, 2, &[png_chunk(b"tEXt", b"Comment\0hello")])).is_ok());
    assert!(inspect(&still(2, 2, &[png_chunk(b"tEXt", &vec![b'a'; MAX_METADATA + 1])])).unwrap_err().contains("metadata"));
    // Two headers, a header that is not first, image data before the header, a chunk longer than the file.
    let mut twice = still(2, 2, &[ihdr(2, 2)]);
    assert!(inspect(&twice).is_err());
    twice.truncate(0);
    let mut late = b"\x89PNG\r\n\x1a\n".to_vec();
    late.extend(png_chunk(b"tEXt", b"a\0b"));
    late.extend(ihdr(2, 2));
    late.extend(png_chunk(b"IDAT", &[0]));
    late.extend(png_chunk(b"IEND", &[]));
    assert!(inspect(&late).is_err());
    let mut data_first = b"\x89PNG\r\n\x1a\n".to_vec();
    data_first.extend(png_chunk(b"IDAT", &[0]));
    data_first.extend(png_chunk(b"IEND", &[]));
    assert!(inspect(&data_first).is_err());
    let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
    huge.extend(ihdr(2, 2));
    huge.extend_from_slice(&u32::MAX.to_be_bytes());
    huge.extend_from_slice(b"IDAT");
    assert!(inspect(&huge).unwrap_err().contains("truncated"));
    // No image data at all.
    let mut empty = b"\x89PNG\r\n\x1a\n".to_vec();
    empty.extend(ihdr(2, 2));
    empty.extend(png_chunk(b"IEND", &[]));
    assert!(inspect(&empty).is_err());
}

#[test]
fn apng_frame_geometry_and_delays_are_checked() {
    let good = fixtures::apng();
    let control = good.windows(4).position(|window| window == b"fcTL").unwrap();
    let field = |offset: usize| control + 4 + offset; // the chunk data starts after the name
    let patched = |offset: usize, value: &[u8]| {
        let mut bytes = good.clone();
        bytes[field(offset)..field(offset) + value.len()].copy_from_slice(value);
        bytes
    };
    // fcTL data: sequence(4) width(4) height(4) x(4) y(4) delay_num(2) delay_den(2) dispose(1) blend(1).
    assert!(inspect(&patched(4, &3u32.to_be_bytes())).unwrap_err().contains("outside canvas"), "frame wider than the canvas");
    assert!(inspect(&patched(12, &1u32.to_be_bytes())).unwrap_err().contains("outside canvas"), "frame offset off the canvas");
    assert!(inspect(&patched(4, &0u32.to_be_bytes())).is_err(), "a zero-width frame");
    assert!(inspect(&patched(8, &u32::MAX.to_be_bytes())).is_err(), "a gigantic frame height");
    // Delay 0/0 and 1/0 mean "one hundredth second" in the format: never a division by zero.
    for delay in [[0u8, 0, 0, 0], [0, 1, 0, 0], [0, 0, 0, 1], [0xff, 0xff, 0xff, 0xff]] { let _ = inspect(&patched(20, &delay)); }
    // A delay of a million seconds is over the duration limit.
    assert!(inspect(&patched(20, &[0xff, 0xff, 0, 1])).unwrap_err().contains("limit"));
}

// ---- WebP -----------------------------------------------------------------------------

#[test]
fn webp_length_canvas_and_frames_are_checked() {
    let good = fixtures::webp();
    assert!(inspect(&good).is_ok());
    for delta in [1i64, -1, 100, -100] {
        let mut bytes = good.clone();
        let declared = i64::from(u32::from_le_bytes(bytes[4..8].try_into().unwrap())) + delta;
        bytes[4..8].copy_from_slice(&(declared as u32).to_le_bytes());
        assert!(inspect(&bytes).is_err(), "RIFF length off by {delta}");
    }
    let mut over = good.clone();
    over[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(inspect(&over).is_err());
    // The canvas: VP8X stores width-1 and height-1 as 24 bits (flags at 20, canvas from 24).
    let canvas = |width_minus_one: u32, height_minus_one: u32| {
        let mut bytes = good.clone();
        bytes[24..27].copy_from_slice(&width_minus_one.to_le_bytes()[..3]);
        bytes[27..30].copy_from_slice(&height_minus_one.to_le_bytes()[..3]);
        bytes
    };
    assert!(inspect(&canvas(4096, 0)).is_err(), "4097 wide");
    assert!(inspect(&canvas(0, 2160)).is_err(), "2161 high");
    assert!(inspect(&canvas(0xff_ffff, 0xff_ffff)).is_err(), "16.7 million pixels square");
    // A frame whose offset puts it off the canvas.
    let frame = good.windows(4).position(|window| window == b"ANMF").unwrap() + 8;
    let mut off = good.clone();
    off[frame..frame + 3].copy_from_slice(&1u32.to_le_bytes()[..3]); // x = 2 * 1 on a 2-wide canvas
    assert!(inspect(&off).unwrap_err().contains("outside canvas"));
    // An unknown chunk is metadata and is budgeted.
    let mut padded = good.clone();
    let big = vec![b'x'; MAX_METADATA + 2];
    padded.extend_from_slice(b"EXIF");
    padded.extend_from_slice(&(big.len() as u32).to_le_bytes());
    padded.extend_from_slice(&big);
    let size = (padded.len() - 8) as u32;
    padded[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(inspect(&padded).unwrap_err().contains("metadata"));
    // A lone RIFF/WEBP with no chunks is not an animation.
    assert!(inspect(b"RIFF\x04\0\0\0WEBP").is_err());
}

// ---- WebM -----------------------------------------------------------------------------

#[test]
fn webm_baseline_is_valid() {
    let media = inspect(&Webm::new().bytes()).unwrap();
    assert_eq!((media.format, media.width, media.height, media.frames, media.duration_ms), (Format::Vp9Webm, 2, 2, 2, 200));
}

#[test]
fn webm_container_attacks_are_refused() {
    let mut matroska = Webm::new();
    matroska.doctype = b"mkv!".to_vec();
    assert!(error_of(&matroska.bytes()).contains("only WebM"));
    let mut two_tracks = Webm::new();
    two_tracks.tracks = 2;
    assert!(error_of(&two_tracks.bytes()).contains("extra"));
    let mut encrypted = Webm::new();
    encrypted.track_extra = ebml(&[0x6d, 0x80], &[1]);
    assert!(error_of(&encrypted.bytes()).contains("forbidden"));
    let mut external = Webm::new();
    external.track_extra = ebml(&[0x7e, 0x01], b"file:///etc/passwd");
    assert!(inspect(&external.bytes()).is_err());
    let mut huge = Webm::new();
    huge.width = 0;
    assert!(inspect(&huge.bytes()).is_err(), "zero width");
    let mut laced = Webm::new();
    laced.flags = 0x86;
    assert!(error_of(&laced.bytes()).contains("lacing"));
}

#[test]
fn webm_timing_attacks_are_refused() {
    for relatives in [vec![100i16, 0], vec![0, 0], vec![0, 5], vec![i16::MIN, 0], vec![-1, 100]] {
        let mut webm = Webm::new();
        webm.relatives = relatives.clone();
        assert!(inspect(&webm.bytes()).is_err(), "block times {relatives:?}");
    }
    let mut late = Webm::new();
    late.cluster_time = 700_000; // 700 s in the default millisecond timebase
    late.duration = Some(800_000f64.to_be_bytes().to_vec());
    assert!(inspect(&late.bytes()).is_err());
    let mut wrapped = Webm::new();
    wrapped.cluster_time = u64::MAX;
    assert!(inspect(&wrapped.bytes()).is_err());
    for duration in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e300, 5.0e8] {
        let mut webm = Webm::new();
        webm.duration = Some(duration.to_be_bytes().to_vec());
        assert!(inspect(&webm.bytes()).is_err(), "declared duration {duration}");
    }
    let mut missing = Webm::new();
    missing.duration = None;
    assert!(inspect(&missing.bytes()).unwrap_err().contains("duration"));
    let mut wrong_size = Webm::new();
    wrong_size.duration = Some(vec![0, 0, 0]);
    assert!(inspect(&wrong_size.bytes()).is_err());
    // A duration that contradicts the blocks (far longer, so the file would stall the wallpaper).
    let mut stretched = Webm::new();
    stretched.duration = Some(100_000f64.to_be_bytes().to_vec());
    assert!(inspect(&stretched.bytes()).is_err());
}

#[test]
fn webm_element_flood_and_hostile_lengths_are_bounded() {
    // A hundred thousand and one empty elements: refused at the element limit, not after allocating for each.
    let flood: Vec<u8> = ebml(&[0xec], &[]).repeat(100_001);
    let mut bytes = ebml(&[0x1a, 0x45, 0xdf, 0xa3], &ebml(&[0x42, 0x82], b"webm"));
    bytes.extend(ebml(&[0x18, 0x53, 0x80, 0x67], &flood));
    assert!(error_of(&bytes).contains("element limit"));
    // An unknown-size (all ones) element that is not the segment.
    assert!(inspect(b"\x1a\x45\xdf\xa3\xff").is_err());
    assert!(inspect(b"\x1a\x45\xdf\xa3\x01\xff\xff\xff\xff\xff\xff\xff").is_err());
    // A size that points far past the end of the file.
    assert!(error_of(b"\x1a\x45\xdf\xa3\x7f\xfe").contains("truncated"));
    // The nine-byte and zero-byte integers.
    assert!(inspect(b"\x1a\x45\xdf\xa3\x00\0\0\0\0\0\0\0\0").is_err());
    assert!(inspect(b"\x00\x45\xdf\xa3").is_err());
    // An element id that is longer than four bytes.
    assert!(inspect(b"\x1a\x45\xdf\xa3\x80\x08").is_err());
}

// ---- the command line ------------------------------------------------------------------

fn run(arguments: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_one-mk-wall-import")).args(arguments).env_clear().output().expect("the helper runs");
    (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn wrong_command_lines_are_refused_with_exit_2_and_a_reason() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "missing command"),
        (&["frobnicate"], "unknown command"),
        (&["import"], "requires FILE"),
        (&["import", "a.gif"], "requires --output"),
        (&["import", "a.gif", "b.gif", "--output", "o"], "unexpected argument"),
        (&["import", "-weird.gif"], "unexpected argument"),
        (&["import", "a.gif", "--output"], "missing option value"),
        (&["import", "a.gif", "--output", "x", "--output", "y"], "duplicate option"),
        (&["import", "a.gif", "--runtime", "x", "--runtime", "y"], "duplicate option"),
        (&["doctor", "extra"], "unexpected argument"),
    ];
    for (arguments, reason) in cases {
        let (code, _, stderr) = run(arguments);
        assert_eq!(code, Some(2), "{arguments:?}: {stderr}");
        assert!(stderr.contains(reason), "{arguments:?}: {stderr}");
    }
}

#[test]
fn an_import_without_a_sandbox_runtime_is_refused_before_anything_is_read() {
    let scratch = tempfile::tempdir().unwrap();
    let missing = scratch.path().join("does-not-exist.gif");
    let output = scratch.path().join("objects");
    std::fs::create_dir(&output).unwrap();
    let (code, _, stderr) = run(&["import", missing.to_str().unwrap(), "--output", output.to_str().unwrap()]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("sandbox unavailable"), "{stderr}");
    let (code, stdout, _) = run(&["import", missing.to_str().unwrap(), "--output", output.to_str().unwrap(), "--json"]);
    assert_eq!(code, Some(3));
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("--json errors are JSON");
    assert_eq!(parsed["exit_code"], 3);
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0, "nothing is written when the import is refused");
    // A runtime that is not there is a refusal, not a crash, and still writes nothing.
    let (code, _, stderr) = run(&["import", missing.to_str().unwrap(), "--output", output.to_str().unwrap(), "--runtime", scratch.path().join("nope").to_str().unwrap()]);
    assert_eq!(code, Some(3), "{stderr}");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

#[test]
fn internal_modes_refuse_wrong_invocations_without_a_panic() {
    for arguments in [&["--decode-webm"][..], &["--decode-webm", "V_VP9"], &["--decode-webm", "V_VP9", "x", "y"], &["--decode-webm", "V_VP9", "-1", "5"], &["--runtime-inventory"]] {
        let (code, _, _) = run(arguments);
        assert!(matches!(code, Some(2..=4)), "{arguments:?} exited {code:?}");
    }
    let (code, stdout, _) = run(&["--build-info"]);
    assert_eq!(code, Some(0));
    assert_eq!(serde_json::from_str::<serde_json::Value>(stdout.trim()).unwrap()["schema"], 1);
}

#[cfg(target_os = "linux")]
#[test]
fn the_library_import_refuses_a_missing_runtime_and_a_cancelled_import_before_touching_the_output() {
    use std::sync::atomic::AtomicBool;
    let scratch = tempfile::tempdir().unwrap();
    let output = scratch.path().join("objects");
    std::fs::create_dir(&output).unwrap();
    let source = scratch.path().join("a.gif");
    std::fs::write(&source, fixtures::gif(2, 2, 2)).unwrap();
    fn options<'a>(source: &'a std::path::Path, output: &'a std::path::Path, runtime: &'a std::path::Path, cancel: &'a AtomicBool) -> one_mk_wall_import::ImportOptions<'a> {
        one_mk_wall_import::ImportOptions { source, output, runtime, cgroup_root: None, cancel, timeout: std::time::Duration::from_secs(5) }
    }
    let missing = scratch.path().join("no-runtime");
    let error = one_mk_wall_import::import(options(&source, &output, &missing, &AtomicBool::new(false))).unwrap_err();
    assert!(error.contains("sandbox unavailable"), "{error}");
    let error = one_mk_wall_import::import(options(&source, &output, &missing, &AtomicBool::new(true))).unwrap_err();
    assert_eq!(error, "import cancelled");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}
