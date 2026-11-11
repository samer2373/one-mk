//! Clipboard behaviour through the public API with the in-memory clipboard:
//! image decoding (every colour type, and hostile files), size limits at their
//! exact boundaries, change detection (the loop guard), and sensitive content.

use one_mk_clipboard::history::{self, History};
use one_mk_clipboard::{
    CEILING_BYTES, ClipData, Clipboard, DEFAULT_MAX_BYTES, Filter, Image, Raw, Read, decode_png, encode_png,
};
use std::sync::Arc;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn rgba(width: usize, height: usize, seed: u64) -> Image {
    let mut rng = Rng(seed | 1);
    Image { width, height, rgba: (0..width * height * 4).map(|_| rng.next() as u8).collect() }
}

fn encode(
    color: png::ColorType,
    depth: png::BitDepth,
    width: u32,
    height: u32,
    data: &[u8],
    tweak: impl FnOnce(&mut png::Encoder<'_, &mut Vec<u8>>),
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(color);
    encoder.set_depth(depth);
    tweak(&mut encoder);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
    out
}

/// A PNG whose header claims `width` x `height` pixels and whose pixel data is a tiny
/// valid-looking stream: what a decompression bomb looks like on arrival.
fn claimed_size(width: u32, height: u32, color: u8, depth: u8) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[depth, color, 0, 0, 0]);
    png.extend(chunk(b"IHDR", &header));
    png.extend(chunk(b"IDAT", &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]));
    png.extend(chunk(b"IEND", &[]));
    png
}

// ---------------------------------------------------------------------- PNG

#[test]
fn images_survive_encode_and_decode_at_awkward_sizes() {
    for (w, h) in [(1, 1), (1, 4096), (4096, 1), (3, 5), (257, 129), (64, 64)] {
        let image = rgba(w, h, (w * 31 + h) as u64);
        let png = encode_png(&image).unwrap();
        assert_eq!(decode_png(&png).unwrap(), image, "{w}x{h}");
    }
    // Transparent and opaque pixels keep their alpha exactly (no premultiplying on the way).
    let image = Image { width: 4, height: 1, rgba: vec![255, 0, 0, 0, 0, 255, 0, 1, 0, 0, 255, 128, 9, 9, 9, 255] };
    assert_eq!(decode_png(&encode_png(&image).unwrap()).unwrap(), image);
    // Pictures that cannot be encoded are errors, never panics.
    assert!(encode_png(&Image { width: 0, height: 5, rgba: vec![] }).is_err());
    assert!(encode_png(&Image { width: 5, height: 0, rgba: vec![] }).is_err());
    assert!(encode_png(&Image { width: 2, height: 2, rgba: vec![0; 15] }).is_err(), "too few bytes");
    assert!(encode_png(&Image { width: 2, height: 2, rgba: vec![0; 17] }).is_err(), "too many bytes");
}

#[test]
fn every_png_colour_type_and_depth_becomes_rgba() {
    use png::{BitDepth::*, ColorType::*};
    let case = |color, depth, w: u32, h: u32, data: &[u8], tweak: fn(&mut png::Encoder<'_, &mut Vec<u8>>)| {
        decode_png(&encode(color, depth, w, h, data, tweak)).unwrap()
    };
    // Rgb: opaque.
    let image = case(Rgb, Eight, 2, 1, &[1, 2, 3, 4, 5, 6], |_| {});
    assert_eq!((image.width, image.height, image.rgba), (2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]));
    // Grayscale and gray+alpha.
    assert_eq!(case(Grayscale, Eight, 2, 1, &[10, 200], |_| {}).rgba, [10, 10, 10, 255, 200, 200, 200, 255]);
    assert_eq!(case(GrayscaleAlpha, Eight, 1, 2, &[7, 0, 9, 128], |_| {}).rgba, [7, 7, 7, 0, 9, 9, 9, 128]);
    // One-bit gray is scaled to the full range, not left at 0/1.
    assert_eq!(case(Grayscale, One, 8, 1, &[0b1010_0000], |_| {}).rgba[..8], [255, 255, 255, 255, 0, 0, 0, 255]);
    // 16-bit samples are reduced to 8 (high byte).
    let sixteen = case(Rgba, Sixteen, 1, 1, &[0xFF, 0xFF, 0x80, 0x00, 0x00, 0x00, 0x40, 0xFF], |_| {});
    assert_eq!(sixteen.rgba, [255, 128, 0, 64]);
    // A palette picture, with transparency for one entry.
    let palette = case(Indexed, Eight, 3, 1, &[0, 1, 2], |e| {
        e.set_palette(vec![255, 0, 0, 0, 255, 0, 0, 0, 255]);
        e.set_trns(vec![0, 128]);
    });
    assert_eq!(palette.rgba, [255, 0, 0, 0, 0, 255, 0, 128, 0, 0, 255, 255]);
    // Rgb with a colour-key transparency (tRNS): the keyed colour is clear.
    let keyed = case(Rgb, Eight, 2, 1, &[1, 2, 3, 4, 5, 6], |e| e.set_trns(vec![0, 1, 0, 2, 0, 3]));
    assert_eq!(keyed.rgba, [1, 2, 3, 0, 4, 5, 6, 255]);
}

#[test]
fn damaged_and_hostile_pictures_are_errors_within_bounds() {
    let good = encode_png(&rgba(17, 9, 5)).unwrap();
    // Every cut that loses picture data is an error (the picture is whole or it is nothing), no
    // panic; a cut inside the closing IEND chunk (12 bytes) loses none, and gives the same picture.
    let original = decode_png(&good).unwrap();
    for cut in 0..good.len() {
        match decode_png(&good[..cut]) {
            Err(_) => {}
            Ok(image) => assert!(cut >= good.len() - 12 && image == original, "cut at {cut} of {}", good.len()),
        }
    }
    // Bytes flipped anywhere never panic; if it still decodes it is a sane picture.
    let mut rng = Rng(0xFACE);
    for _ in 0..3000 {
        let mut bytes = good.clone();
        for _ in 0..1 + rng.next() % 3 {
            let at = (rng.next() % bytes.len() as u64) as usize;
            bytes[at] ^= 1 << (rng.next() % 8);
        }
        if let Ok(image) = decode_png(&bytes) {
            assert_eq!(image.rgba.len(), image.width * image.height * 4);
            assert!(image.width > 0 && image.height > 0);
        }
    }
    // Not images at all.
    for junk in [&b""[..], b"\x89PNG\r\n\x1a\n", b"GIF89a", &[0u8; 100], &[0xFF; 100]] {
        assert!(decode_png(junk).is_err());
    }
    // A header that claims a picture beyond the limit is refused up front: nothing that large
    // is allocated for a few dozen bytes of data.
    for (w, h, color, depth) in [
        (20_000, 20_000, 6, 8),
        (65_535, 65_535, 6, 16),
        (1 << 20, 1 << 20, 0, 1),
        (16_385, 4_096, 6, 8),
        (u32::MAX >> 1, 2, 6, 8),
    ] {
        let started = std::time::Instant::now();
        let bomb = claimed_size(w, h, color, depth);
        assert!(bomb.len() < 200);
        let err = decode_png(&bomb).err().unwrap_or_else(|| panic!("{w}x{h} accepted"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{w}x{h} took long: {err:#}");
    }
    // A zero-sized header is refused too.
    assert!(decode_png(&claimed_size(0, 10, 6, 8)).is_err());
    assert!(decode_png(&claimed_size(10, 0, 6, 8)).is_err());
}

// ------------------------------------------------------------ size and filter

#[tokio::test]
async fn the_size_limit_is_exact_for_text_html_and_both_together() {
    let (clipboard, memory) = Clipboard::memory();
    clipboard.set_filter(Filter { max_bytes: 100, ..Filter::default() });
    let read = |memory: &one_mk_clipboard::MemoryClipboard, text: usize, html: usize| {
        memory.set(Raw {
            text: (text > 0).then(|| "t".repeat(text)),
            html: (html > 0).then(|| "h".repeat(html)),
            ..Raw::default()
        });
    };
    for (text, html, fits) in [
        (100, 0, true),
        (101, 0, false),
        (0, 100, true),
        (0, 101, false),
        (60, 40, true),
        (60, 41, false),
        (1, 99, true),
        (50, 51, false),
    ] {
        read(&memory, text, html);
        match clipboard.read(vec![]).await.unwrap() {
            Read::Changed { data, .. } => assert!(fits && data.len() == text + html, "{text}+{html}"),
            Read::TooLarge { size, .. } => assert!(!fits && size == text + html, "{text}+{html} reported as {size}"),
            other => panic!("{text}+{html}: {other:?}"),
        }
    }
    // A limit of zero refuses everything that has content, and still sees an empty clipboard.
    clipboard.set_filter(Filter { max_bytes: 0, ..Filter::default() });
    memory.set_text("x");
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::TooLarge { size: 1, .. }));
    memory.set(Raw::default());
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Empty));
    // The bound the settings cannot exceed.
    assert_eq!(Filter { max_bytes: usize::MAX, ..Filter::default() }.max_bytes(), CEILING_BYTES);
    assert_eq!(Filter::default().max_bytes(), DEFAULT_MAX_BYTES);
    const { assert!(DEFAULT_MAX_BYTES < CEILING_BYTES) };
}

#[tokio::test]
async fn too_large_stays_too_large_until_the_content_or_the_limit_changes() {
    let (clipboard, memory) = Clipboard::memory();
    clipboard.set_filter(Filter { max_bytes: 10, ..Filter::default() });
    memory.set_text("0123456789ABC");
    let Read::TooLarge { digest, size } = clipboard.read(vec![]).await.unwrap() else { panic!() };
    assert_eq!(size, 13);
    // A caller that remembers the digest is told it did not change, so nothing is re-sent.
    assert!(matches!(clipboard.read(vec![digest]).await.unwrap(), Read::Unchanged(d) if d == digest));
    clipboard.set_filter(Filter { max_bytes: 13, ..Filter::default() });
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Changed { .. }));
    memory.set_text("short");
    assert!(matches!(clipboard.read(vec![digest]).await.unwrap(), Read::Changed { .. }));
}

#[tokio::test]
async fn a_huge_image_alone_is_too_large_and_a_picture_over_the_limit_after_encoding_too() {
    let (clipboard, memory) = Clipboard::memory();
    // Over the decoded-size bound: refused before it is even encoded.
    memory.set(Raw {
        image: Some(Image { width: 16_384, height: 4_097, rgba: vec![0; 16_384 * 4_097 * 4] }),
        ..Raw::default()
    });
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::TooLarge { size, .. } if size > 256 << 20));
    // A noisy picture does not compress: its PNG is what the limit counts.
    let image = rgba(200, 200, 3);
    memory.set(Raw { image: Some(image.clone()), ..Raw::default() });
    let png_len = encode_png(&image).unwrap().len();
    assert!(png_len > 100_000);
    clipboard.set_filter(Filter { max_bytes: png_len - 1, ..Filter::default() });
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::TooLarge { size, .. } if size == png_len));
    clipboard.set_filter(Filter { max_bytes: png_len, ..Filter::default() });
    let Read::Changed { data, .. } = clipboard.read(vec![]).await.unwrap() else { panic!() };
    assert_eq!(decode_png(data.png.as_ref().unwrap()).unwrap(), image);
}

// ------------------------------------------------------- change detection

#[tokio::test]
async fn digests_tell_formats_apart_and_are_stable() {
    let (clipboard, memory) = Clipboard::memory();
    let digest_of = |raw: Raw| {
        memory.set(raw);
        let clipboard = clipboard.clone();
        async move { clipboard.digest().await.unwrap().unwrap() }
    };
    let a = digest_of(Raw { text: Some("a".into()), ..Raw::default() }).await;
    assert_eq!(a, digest_of(Raw { text: Some("a".into()), ..Raw::default() }).await, "same content, same digest");
    let b = digest_of(Raw { html: Some("a".into()), ..Raw::default() }).await;
    let c = digest_of(Raw { text: Some("a".into()), html: Some("a".into()), ..Raw::default() }).await;
    let d = digest_of(Raw { text: Some("aa".into()), ..Raw::default() }).await;
    let e = digest_of(Raw { text: Some("".into()), html: Some("aa".into()), ..Raw::default() }).await;
    let f = digest_of(Raw { text: Some("a".into()), html: Some("a".into()), concealed: true, ..Raw::default() }).await;
    // The boundary between formats is part of the digest: "a"+"a" is not "aa".
    for (x, y) in [(a, b), (a, c), (b, c), (c, d), (d, e), (a, d)] {
        assert_ne!(x, y);
    }
    assert_eq!(c, f, "being marked sensitive does not change what the content is");
    // Two images of different shape but the same bytes differ.
    let wide = Image { width: 4, height: 1, rgba: vec![7; 16] };
    let tall = Image { width: 1, height: 4, rgba: vec![7; 16] };
    assert_ne!(
        digest_of(Raw { image: Some(wide), ..Raw::default() }).await,
        digest_of(Raw { image: Some(tall), ..Raw::default() }).await
    );
    memory.set(Raw::default());
    assert_eq!(clipboard.digest().await.unwrap(), None, "an empty clipboard has no digest");
}

/// The loop guard: what this computer writes for a peer is not read back as a new local copy.
#[tokio::test]
async fn a_write_is_recognised_when_read_back_for_every_format_mix() {
    let (clipboard, memory) = Clipboard::memory();
    let picture = encode_png(&rgba(5, 4, 9)).unwrap();
    let cases = [
        ClipData { text: Some("hello".into()), ..ClipData::default() },
        ClipData { html: Some("<b>x</b>".into()), ..ClipData::default() },
        ClipData { text: Some("hello".into()), html: Some("<b>hello</b>".into()), png: None },
        ClipData { png: Some(picture.clone()), ..ClipData::default() },
        // Text and a picture together (a spreadsheet range): text wins on both ends.
        ClipData { text: Some("cells".into()), png: Some(picture.clone()), ..ClipData::default() },
        ClipData { text: Some("é界😀 \n\t \u{0}".into()), ..ClipData::default() },
    ];
    for data in cases {
        let written = clipboard.write(data.clone()).await.unwrap();
        assert!(
            matches!(clipboard.read(vec![written]).await.unwrap(), Read::Unchanged(d) if d == written),
            "{data:?} came back as a change"
        );
        assert_eq!(clipboard.digest().await.unwrap(), Some(written));
        // What a person copies next is a change.
        memory.set_text("something else");
        assert!(matches!(clipboard.read(vec![written]).await.unwrap(), Read::Changed { .. }));
    }
}

#[tokio::test]
async fn an_invalid_picture_from_a_peer_is_refused_and_leaves_the_clipboard_alone() {
    let (clipboard, memory) = Clipboard::memory();
    memory.set_text("mine");
    let writes = memory.writes();
    for bad in [
        vec![],
        b"not a png".to_vec(),
        claimed_size(30_000, 30_000, 6, 8),
        encode_png(&rgba(4, 4, 1)).unwrap()[..40].to_vec(),
    ] {
        assert!(clipboard.write(ClipData { png: Some(bad), ..ClipData::default() }).await.is_err());
    }
    assert_eq!(memory.text().as_deref(), Some("mine"));
    assert_eq!(memory.writes(), writes);
    // And the clipboard still works afterwards.
    clipboard.write(ClipData { text: Some("fine".into()), ..ClipData::default() }).await.unwrap();
    assert_eq!(memory.text().as_deref(), Some("fine"));
}

#[tokio::test]
async fn sensitive_content_follows_the_rules_in_every_combination() {
    let (clipboard, memory) = Clipboard::memory();
    memory.set(Raw { text: Some("secret".into()), concealed: true, ..Raw::default() });
    // Automatic reads never return it; the digest still lets the caller tell it from the next copy.
    let Read::Concealed(digest) = clipboard.read(vec![]).await.unwrap() else { panic!() };
    // A caller that already knows the digest hears "unchanged", the cheapest answer.
    assert!(matches!(clipboard.read(vec![digest]).await.unwrap(), Read::Unchanged(_)));
    // An explicit send gets it, marked as sensitive, with the size limit still applying.
    assert!(matches!(clipboard.read_explicit(vec![]).await.unwrap(), Read::Sensitive { .. }));
    clipboard.set_filter(Filter { max_bytes: 3, ..Filter::default() });
    assert!(matches!(clipboard.read_explicit(vec![]).await.unwrap(), Read::TooLarge { size: 6, .. }));
    clipboard.set_filter(Filter { text: false, ..Filter::default() });
    assert!(matches!(clipboard.read_explicit(vec![]).await.unwrap(), Read::Empty), "a format that is off stays off");
    // Nothing sensitive is ever recorded, in any path.
    assert!(clipboard.history().lock().unwrap().is_empty());
    // With the skip switched off the local user's own sensitive copy goes out as an ordinary one...
    clipboard.set_filter(Filter { skip_concealed: false, ..Filter::default() });
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Changed { .. }));
    assert!(clipboard.history().lock().unwrap().is_empty(), "...but is still kept out of the history");
    // A marked arrival is not read back as the local user's copy, nor hidden by a later write.
    clipboard.write_concealed(ClipData { text: Some("from a peer".into()), ..ClipData::default() }).await.unwrap();
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Concealed(_)));
    clipboard.write(ClipData { text: Some("ordinary".into()), ..ClipData::default() }).await.unwrap();
    assert!(matches!(clipboard.read(vec![]).await.unwrap(), Read::Changed { .. }));
}

#[tokio::test]
async fn an_empty_piece_of_text_is_content_to_the_clipboard_but_not_to_the_sender() {
    let (clipboard, memory) = Clipboard::memory();
    memory.set(Raw { text: Some(String::new()), ..Raw::default() });
    let Read::Changed { data, .. } = clipboard.read(vec![]).await.unwrap() else {
        panic!("an empty string is a format that is present")
    };
    assert!(!data.is_empty() && data.has_no_content(), "callers must check has_no_content before sending");
    assert!(
        ClipData::default().has_no_content() && ClipData { png: Some(vec![]), ..ClipData::default() }.has_no_content()
    );
    assert!(!ClipData { html: Some("x".into()), ..ClipData::default() }.has_no_content());
    assert_eq!(ClipData { text: Some("ab".into()), html: Some("c".into()), png: Some(vec![0; 4]) }.len(), 7);
    // And empty content is not what the history keeps.
    assert!(
        clipboard.history().lock().unwrap().is_empty(),
        "a blank entry is not worth keeping (this used to be recorded)"
    );
}

// ------------------------------------------------------------------ history

#[test]
fn history_previews_and_kinds_describe_what_is_held_without_trusting_it() {
    let mut history = History::default();
    let picture = encode_png(&rgba(30, 20, 2)).unwrap();
    history.record(Arc::new(ClipData { png: Some(picture), ..ClipData::default() }), false, "desk");
    history.record(Arc::new(ClipData { png: Some(vec![1, 2, 3]), ..ClipData::default() }), false, "desk");
    history.record(
        Arc::new(ClipData { html: Some("<p>h\u{202e}tml</p>".into()), text: None, png: None }),
        false,
        "bad\u{1b}[2Kname",
    );
    let list = history.list();
    assert_eq!((list[0].kind(), list[0].preview().as_str()), ("html", "<p>html</p>"));
    assert_eq!(list[0].origin, "bad[2Kname", "an origin from the network cannot carry escapes");
    assert_eq!(
        (list[1].kind(), list[1].preview().as_str()),
        ("image", "image"),
        "an undecodable picture is just an image"
    );
    assert_eq!((list[2].kind(), list[2].preview().as_str()), ("image", "image 30x20"));
    // With both formats, the plain text is what is shown.
    history.record(
        Arc::new(ClipData { html: Some("<b>x</b>".into()), text: Some("plain".into()), png: None }),
        false,
        "x",
    );
    assert_eq!((history.list()[0].kind(), history.list()[0].preview().as_str()), ("html", "plain"));
    assert_eq!(history::digest(&list[0].data), list[0].digest);
}

#[test]
fn history_bounds_hold_for_hostile_settings() {
    let mut history = History::default();
    // Settings far beyond the hard limits are cut to them, not trusted.
    history.configure(usize::MAX, usize::MAX, usize::MAX, None);
    for i in 0..1200 {
        history.record(Arc::new(ClipData { text: Some(format!("entry {i}")), ..ClipData::default() }), false, "x");
    }
    assert_eq!(history.len(), 1000, "at most a thousand items");
    assert_eq!(history.list()[0].preview(), "entry 1199");
    // Shrinking the settings trims what is held at once, oldest first.
    history.configure(3, usize::MAX, usize::MAX, None);
    assert_eq!(
        history.list().iter().map(|e| e.preview()).collect::<Vec<_>>(),
        ["entry 1199", "entry 1198", "entry 1197"]
    );
    // A byte budget smaller than one item keeps nothing and refuses to keep the next.
    history.configure(10, 4, 100, None);
    assert!(history.is_empty());
    history.record(Arc::new(ClipData { text: Some("12345".into()), ..ClipData::default() }), false, "x");
    assert!(history.is_empty());
    history.record(Arc::new(ClipData { text: Some("1234".into()), ..ClipData::default() }), false, "x");
    assert_eq!(history.len(), 1);
    // Ids never repeat, even after the item is deleted.
    let id = history.list()[0].id;
    assert!(history.delete(id));
    assert!(!history.delete(id));
    history.record(Arc::new(ClipData { text: Some("abcd".into()), ..ClipData::default() }), false, "x");
    assert!(history.list()[0].id > id);
    // Concealed and empty content is never recorded.
    history.clear();
    history.record(Arc::new(ClipData { text: Some("secret".into()), ..ClipData::default() }), true, "x");
    history.record(Arc::new(ClipData::default()), false, "x");
    assert!(history.is_empty());
}
