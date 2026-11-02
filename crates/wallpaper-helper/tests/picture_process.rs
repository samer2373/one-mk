use one_mk_wallpaper::{Canvas, Fit, Media, Monitor, Plan, Rect, picture};
use std::path::Path;

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08,
    0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0a, 0x0c, 0x14, 0x0d, 0x0c, 0x0b, 0x0b, 0x0c, 0x19, 0x12,
    0x13, 0x0f, 0x14, 0x1d, 0x1a, 0x1f, 0x1e, 0x1d, 0x1a, 0x1c, 0x1c, 0x20, 0x24, 0x2e, 0x27, 0x20,
    0x22, 0x2c, 0x23, 0x1c, 0x1c, 0x28, 0x37, 0x29, 0x2c, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1f, 0x27,
    0x39, 0x3d, 0x38, 0x32, 0x3c, 0x2e, 0x33, 0x34, 0x32, 0xff, 0xdb, 0x00, 0x43, 0x01, 0x09, 0x09,
    0x09, 0x0c, 0x0b, 0x0c, 0x18, 0x0d, 0x0d, 0x18, 0x32, 0x21, 0x1c, 0x21, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32,
    0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0x32, 0xff, 0xc0,
    0x00, 0x11, 0x08, 0x00, 0x04, 0x00, 0x08, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11,
    0x01, 0xff, 0xc4, 0x00, 0x15, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0xff, 0xc4, 0x00, 0x14, 0x10, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xc4,
    0x00, 0x14, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xff, 0xc4, 0x00, 0x14, 0x11, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xda, 0x00, 0x0c, 0x03, 0x01,
    0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0x9f, 0xc0, 0x07, 0xff, 0xd9,
];

fn helper() -> &'static Path { Path::new(env!("CARGO_BIN_EXE_one-mk-wallpaper")) }

fn plan() -> Plan {
    Plan { hash: "a".repeat(64), title: "Picture".into(), media: Media { width: 8, height: 4, duration_ms: 1, frame_rate: 0 },
        canvas: Canvas { monitors: [0, 4].into_iter().map(|horizontal| Monitor { computer: "host".into(), name: horizontal.to_string(),
            rect: Rect { x: f64::from(horizontal), y: 0.0, width: 4.0, height: 4.0 }, local: None, scale: 1.0, online: true }).collect() },
        start_ms: 1, fit: Fit::Stretch, computers: vec!["host".into()], pause_on_battery: false, paused: false, picture: true, asset_hash: None }
}

fn bundle(bytes: &[u8]) -> Vec<u8> {
    let mut bundle = b"oneMKpic".to_vec();
    bundle.extend_from_slice(&2u32.to_le_bytes());
    for _ in 0..2 { bundle.extend_from_slice(&(bytes.len() as u64).to_le_bytes()); bundle.extend_from_slice(bytes); }
    bundle
}

#[test]
fn helper_inspects_all_supported_formats_and_prepares_png_only_slices() {
    use std::io::Read;
    let root = tempfile::tempdir().unwrap();
    for extension in ["png", "jpg", "webp"] {
        let source = root.path().join(format!("source.{extension}"));
        let original = image::RgbImage::from_fn(8, 4, |horizontal, vertical| image::Rgb([(horizontal * 30) as u8, (vertical * 60) as u8, 0]));
        match extension {
            "jpg" => std::fs::write(&source, JPEG).unwrap(),
            "webp" => image_webp::WebPEncoder::new(std::fs::File::create(&source).unwrap())
                .encode(original.as_raw(), 8, 4, image_webp::ColorType::Rgb8).unwrap(),
            _ => original.save(&source).unwrap(),
        }
        let poster = root.path().join("poster.png");
        let media = picture::inspect(helper(), &source, &poster).unwrap();
        assert_eq!(media, plan().media);
        let thumbnail = image::open(&poster).unwrap();
        assert!(thumbnail.width() <= 1600 && thumbnail.height() <= 900);
        let output = root.path().join("bundle.video");
        picture::prepare(helper(), &source, &plan(), "host", &output).unwrap();
        let mut input = std::io::Cursor::new(std::fs::read(output).unwrap());
        let mut header = [0; 8]; input.read_exact(&mut header).unwrap();
        assert_eq!(&header, b"oneMKpi2");
        {
            let mut length = [0; 8]; input.read_exact(&mut length).unwrap();
            let mut bytes = vec![0; u64::from_le_bytes(length) as usize]; input.read_exact(&mut bytes).unwrap();
            assert_eq!(image::guess_format(&bytes).unwrap(), image::ImageFormat::Png);
            let original_pixels = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap().into_rgb8();
            assert_eq!(original_pixels.dimensions(), (8, 4));
            if extension != "jpg" {
                assert_eq!(original_pixels, original);
            }
        }
        assert_eq!(input.position(), input.get_ref().len() as u64);
    }
}

#[test]
fn helper_refuses_oversized_and_crafted_images_without_replacing_outputs() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("oversized.png");
    image::RgbaImage::new(16385, 1).save(&source).unwrap();
    let poster = root.path().join("poster.png");
    let prepared = root.path().join("bundle.video");
    std::fs::write(&poster, b"previous poster").unwrap();
    std::fs::write(&prepared, b"previous bundle").unwrap();
    assert!(picture::inspect(helper(), &source, &poster).unwrap_err().to_string().contains("picture helper failed"));
    assert!(picture::prepare(helper(), &source, &plan(), "host", &prepared).is_err());
    let oversized_jpeg = root.path().join("oversized.jpg");
    let mut bytes = JPEG.to_vec();
    let frame = bytes.windows(2).position(|marker| marker == [0xff, 0xc0]).unwrap();
    bytes[frame + 7..frame + 9].copy_from_slice(&16385u16.to_be_bytes());
    std::fs::write(&oversized_jpeg, &bytes).unwrap();
    assert!(picture::inspect(helper(), &oversized_jpeg, &poster).is_err());
    std::fs::write(&oversized_jpeg, JPEG).unwrap();
    std::fs::OpenOptions::new().write(true).open(&oversized_jpeg).unwrap().set_len(256 * 1024 * 1024 + 1).unwrap();
    assert!(picture::inspect(helper(), &oversized_jpeg, &poster).unwrap_err().to_string().contains("compressed picture is too large"));
    let mut bytes = Vec::new();
    image_webp::WebPEncoder::new(&mut bytes).encode(&[0, 0, 0], 1, 1, image_webp::ColorType::Rgb8).unwrap();
    let frame = bytes.windows(4).position(|marker| marker == b"VP8L").unwrap();
    bytes[frame + 9..frame + 13].copy_from_slice(&0x0fff_ffffu32.to_le_bytes());
    let oversized_webp = root.path().join("oversized.webp");
    std::fs::write(&oversized_webp, bytes).unwrap();
    assert!(picture::inspect(helper(), &oversized_webp, &poster).is_err());
    for bytes in [b"\xff\xd8\xff\xe0crafted JPEG".as_slice(), b"RIFF\x10\x00\x00\x00WEBPVP8 crafted", b"\x89PNG\r\n\x1a\ncrafted PNG"] {
        std::fs::write(&source, bytes).unwrap();
        assert!(picture::inspect(helper(), &source, &poster).is_err());
    }
    assert_eq!(std::fs::read(&poster).unwrap(), b"previous poster");
    assert_eq!(std::fs::read(&prepared).unwrap(), b"previous bundle");
    image::RgbaImage::new(8, 4).save(&source).unwrap();
    assert_eq!(picture::inspect(helper(), &source, &poster).unwrap(), plan().media);
    assert!(!root.path().join("previous.json").exists());
}

#[test]
fn receiver_helper_rejects_non_png_and_corrupt_png_before_native_application() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("slice.jpg");
    std::fs::write(&source, JPEG).unwrap();
    let output = root.path().join("bundle.video");
    let pictures = root.path().join("pictures");
    std::fs::write(&output, bundle(&std::fs::read(&source).unwrap())).unwrap();
    assert!(picture::apply(helper(), &plan(), "host", &output, &pictures).unwrap_err().to_string().contains("must be PNG"));
    let png = root.path().join("slice.png");
    image::RgbaImage::new(4, 4).save(&png).unwrap();
    let mut bytes = std::fs::read(&png).unwrap();
    bytes.truncate(33);
    std::fs::write(&output, bundle(&bytes)).unwrap();
    assert!(picture::apply(helper(), &plan(), "host", &output, &pictures).is_err());
    assert!(!pictures.join("previous.json").exists());
    assert!(!pictures.join("current.json").exists());
    assert!(std::fs::read_dir(pictures).unwrap().next().is_none());
}
