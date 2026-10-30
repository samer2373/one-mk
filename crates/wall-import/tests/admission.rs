use one_mk_wall_import::{Format, inspect};

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 256, max_shrink_iters: 256, failure_persistence: None, ..proptest::test_runner::Config::default() })]
    #[test]
    fn arbitrary_and_magic_prefixed_inputs_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..8192)) {
        let _ = inspect(&bytes);
        for magic in [b"GIF89a".as_slice(), b"\x89PNG\r\n\x1a\n", b"RIFF", b"\x1a\x45\xdf\xa3"] {
            let mut candidate = magic.to_vec();
            candidate.extend_from_slice(&bytes);
            let _ = inspect(&candidate);
        }
    }
}

#[test]
fn convert_first_for_legacy_containers_and_static_images() {
    for bytes in [b"\0\0\0\x18ftypmp42".as_slice(), b"OggS", b"\xff\xd8\xff", b"RIFF\x04\0\0\0AVI ", b"<svg/>"] {
        assert!(inspect(bytes).unwrap_err().contains("convert first"));
    }
}

#[test]
fn truncated_magic_and_huge_dimensions_are_not_admitted() {
    for bytes in [b"GIF89a".as_slice(), b"GIF89a\xff\xff\xff\xff\0\0\0;", b"\x89PNG\r\n\x1a\n", b"\x1a\x45\xdf\xa3"] {
        assert!(inspect(bytes).is_err());
    }
}

#[test]
fn generated_gif_is_recognized_but_trailing_polyglot_is_not() {
    let mut bytes = vec![];
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for _ in 0..2 {
            encoder.encode_frame(image::Frame::from_parts(image::RgbaImage::from_pixel(2, 2, image::Rgba([8, 16, 32, 255])), 0, 0, image::Delay::from_numer_denom_ms(100, 1))).unwrap();
        }
    }
    assert_eq!(inspect(&bytes).unwrap().format, Format::Gif);
    bytes.extend_from_slice(b"PK\x03\x04hostile polyglot");
    assert!(inspect(&bytes).is_err());
}

#[test]
fn seeded_parser_fuzz_is_bounded_and_never_panics() {
    let mut seed = 0x34ab_98f1_u64;
    for length in 0..2048 {
        let mut bytes = vec![0; length];
        for byte in &mut bytes {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *byte = seed as u8;
        }
        for magic in [b"GIF89a".as_slice(), b"\x89PNG\r\n\x1a\n", b"RIFF", b"\x1a\x45\xdf\xa3"] {
            if bytes.len() >= magic.len() {
                bytes[..magic.len()].copy_from_slice(magic);
            }
            let _ = inspect(&bytes);
        }
    }
}

fn ebml(identifier: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut result = identifier.to_vec();
    result.extend_from_slice(&(0x1000_0000u32 | payload.len() as u32).to_be_bytes());
    result.extend_from_slice(payload);
    result
}

pub fn generated_webm(extra: &[u8]) -> Vec<u8> {
    let mut header = ebml(&[0x1a, 0x45, 0xdf, 0xa3], &ebml(&[0x42, 0x82], b"webm"));
    let mut track = ebml(&[0x83], &[1]);
    track.extend(ebml(&[0xd7], &[1]));
    track.extend(ebml(&[0x86], b"V_VP9"));
    track.extend(ebml(&[0xe0], &[ebml(&[0xb0], &[2]), ebml(&[0xba], &[2])].concat()));
    track.extend(ebml(&[0x63, 0xa2], extra));
    let tracks = ebml(&[0x16, 0x54, 0xae, 0x6b], &ebml(&[0xae], &track));
    let cluster = ebml(&[0x1f, 0x43, 0xb6, 0x75], &[ebml(&[0xe7], &[0]), ebml(&[0xa3], &[0x81, 0, 0, 0x80, 0]), ebml(&[0xa3], &[0x81, 0, 100, 0x80, 0])].concat());
    let info = ebml(&[0x15, 0x49, 0xa9, 0x66], &ebml(&[0x44, 0x89], &200f64.to_be_bytes()));
    header.extend(ebml(&[0x18, 0x53, 0x80, 0x67], &[info, tracks, cluster].concat()));
    header
}

#[test]
fn hostile_webm_codec_metadata_is_counted_before_decoder_allocation() {
    assert!(inspect(&generated_webm(&[])).is_ok());
    assert!(inspect(&generated_webm(&vec![0x41; 256 * 1024 + 1])).is_err());
}

#[test]
fn vp8_and_audio_webm_are_convert_first() {
    let mut vp8 = generated_webm(&[]);
    let codec = vp8.windows(5).position(|value| value == b"V_VP9").unwrap();
    vp8[codec + 4] = b'8';
    assert!(inspect(&vp8).unwrap_err().contains("convert first"));
    let mut audio = generated_webm(&[]);
    let track = audio.windows(6).position(|value| value == b"\x83\x10\0\0\x01\x01").unwrap();
    audio[track + 5] = 2;
    assert!(inspect(&audio).unwrap_err().contains("convert first"));
}

#[test]
fn eight_byte_ebml_sizes_do_not_overflow() {
    let bytes = b"\x1a\x45\xdf\xa3\x01\0\0\0\0\0\0\0";
    assert!(inspect(bytes).is_err());
}
