use crate::{Media, Monitor, Plan};
#[cfg(target_os = "linux")]
use crate::Rect;
use anyhow::{Context, Result, ensure};
use image::{ImageDecoder, ImageReader, RgbaImage};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
#[path = "picture_macos.rs"]
mod native;

pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    return std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.to_ascii_lowercase().contains("gnome"));
    #[cfg(not(target_os = "linux"))]
    true
}
#[cfg(windows)]
#[path = "picture_windows.rs"]
mod native;

pub fn is_picture(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "heic"))
}

fn open(path: &Path) -> Result<RgbaImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    #[cfg(any(target_os = "macos", windows))]
    if reader.format().is_none() { return native::decode(path); }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().context("This picture cannot be opened. Try a PNG, JPEG or WebP picture.")?;
    let orientation = decoder.orientation()?;
    let mut pixels = image::DynamicImage::from_decoder(decoder)?;
    pixels.apply_orientation(orientation);
    Ok(pixels.into_rgba8())
}

pub fn inspect(path: &Path, poster: &Path) -> Result<Media> {
    let image = open(path)?;
    image::imageops::thumbnail(&image, 1600, 900).save_with_format(poster, image::ImageFormat::Png)?;
    Ok(Media { width: image.width(), height: image.height(), duration_ms: 1, frame_rate: 0 })
}

pub fn prepare(source: &Path, plan: &Plan, computer: &str, path: &Path) -> Result<()> {
    let image = open(source)?;
    let monitors: Vec<_> = plan.canvas.monitors.iter().filter(|monitor| monitor.computer == computer).collect();
    ensure!(!monitors.is_empty(), "No screens selected on this computer");
    let mut output = std::fs::File::create(path)?;
    output.write_all(b"oneMKpic")?;
    output.write_all(&(monitors.len() as u32).to_le_bytes())?;
    for monitor in monitors {
        let slice = plan.canvas.slice(monitor, &plan.media, plan.fit).context("Empty picture layout")?;
        let width = (monitor.rect.width * monitor.scale).round() as u32;
        let height = (monitor.rect.height * monitor.scale).round() as u32;
        ensure!(width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024, "This screen is too large for a picture wallpaper");
        let mut pixels = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        for (horizontal, vertical, pixel) in pixels.enumerate_pixels_mut() {
            let source_x = slice.x + (f64::from(horizontal) + 0.5) / f64::from(width) * slice.width;
            let source_y = slice.y + (f64::from(vertical) + 0.5) / f64::from(height) * slice.height;
            if let Some(sample) = image::imageops::sample_bilinear(&image, source_x as f32, source_y as f32) { *pixel = sample; }
        }
        let mut png = Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png)?;
        let bytes = png.into_inner();
        output.write_all(&(bytes.len() as u64).to_le_bytes())?;
        output.write_all(&bytes)?;
    }
    output.sync_all()?;
    Ok(())
}

fn slices(plan: &Plan, computer: &str, path: &Path, root: &Path) -> Result<Vec<(Monitor, PathBuf)>> {
    std::fs::create_dir_all(root)?;
    let mut input = std::fs::File::open(path)?;
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    ensure!(&magic == b"oneMKpic", "The received picture is incomplete");
    let mut count = [0; 4];
    input.read_exact(&mut count)?;
    let monitors: Vec<_> = plan.canvas.monitors.iter().filter(|monitor| monitor.computer == computer).collect();
    ensure!(u32::from_le_bytes(count) as usize == monitors.len(), "The picture does not match this computer's screens");
    let mut files = Vec::new();
    for (index, monitor) in monitors.into_iter().enumerate() {
        let mut length = [0; 8];
        input.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        ensure!(length <= 256 * 1024 * 1024, "The received picture is too large");
        let mut bytes = vec![0; length as usize];
        input.read_exact(&mut bytes)?;
        let (width, height) = ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?.into_dimensions()?;
        ensure!(width == (monitor.rect.width * monitor.scale).round() as u32 && height == (monitor.rect.height * monitor.scale).round() as u32
            && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024, "The received picture does not match this screen");
        let destination = root.join(format!("{}-{index}.png", plan.asset_hash.as_ref().unwrap_or(&plan.hash)));
        std::fs::write(&destination, bytes)?;
        files.push((monitor.clone(), destination));
    }
    ensure!(input.read(&mut [0])? == 0, "The received picture has unexpected contents");
    Ok(files)
}

pub fn apply(plan: &Plan, computer: &str, path: &Path, root: &Path) -> Result<()> {
    let files = slices(plan, computer, path, root)?;
    #[cfg(any(target_os = "macos", windows))]
    return native::apply(&files, root);
    #[cfg(target_os = "linux")]
    {
        let bounds = files.iter().map(|(monitor, _)| monitor.local.unwrap_or(monitor.rect)).reduce(Rect::union).context("No picture screens")?;
        ensure!(bounds.width * bounds.height <= 64.0 * 1024.0 * 1024.0, "This desktop is too large for a picture wallpaper");
        let mut desktop = RgbaImage::new(bounds.width as u32, bounds.height as u32);
        for (monitor, path) in files {
            let rect = monitor.local.unwrap_or(monitor.rect);
            let pixels = image::imageops::resize(&open(&path)?, rect.width as u32, rect.height as u32, image::imageops::FilterType::Triangle);
            image::imageops::overlay(&mut desktop, &pixels, (rect.x - bounds.x) as i64, (rect.y - bounds.y) as i64);
        }
        let path = root.join(format!("{}-spanned.png", plan.asset_hash.as_ref().unwrap_or(&plan.hash)));
        desktop.save(&path)?;
        let snapshot = root.join("previous.json");
        if !snapshot.exists() {
            let previous: Vec<_> = ["picture-uri", "picture-uri-dark", "picture-options"].into_iter().map(|key| settings("get", key, None).map(|value| (key.to_owned(), value))).collect::<Result<_>>()?;
            std::fs::write(&snapshot, serde_json::to_vec(&previous)?)?;
        }
        let uri = file_uri(&path)?;
        for key in ["picture-uri", "picture-uri-dark"] { settings("set", key, Some(&uri))?; }
        settings("set", "picture-options", Some("spanned"))?;
        Ok(())
    }
}

pub fn restore(root: &Path) -> Result<()> {
    if !root.join("previous.json").exists() { return Ok(()); }
    #[cfg(any(target_os = "macos", windows))]
    native::restore(root)?;
    #[cfg(target_os = "linux")]
    {
        let previous: Vec<(String, String)> = serde_json::from_slice(&std::fs::read(root.join("previous.json"))?)?;
        for (key, value) in previous { settings("set", &key, Some(value.trim()))?; }
    }
    std::fs::remove_file(root.join("previous.json"))?;
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "png") { let _ = std::fs::remove_file(path); }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn settings(action: &str, key: &str, value: Option<&str>) -> Result<String> {
    let mut command = std::process::Command::new("gsettings");
    command.args([action, "org.gnome.desktop.background", key]);
    if let Some(value) = value { command.arg(value); }
    let output = command.output().context("Picture wallpapers need a desktop with GNOME wallpaper settings")?;
    ensure!(output.status.success() && output.stderr.is_empty(), "The desktop could not change its wallpaper: {}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn file_uri(path: &Path) -> Result<String> {
    let mut uri = String::from("file://");
    for byte in path.canonicalize()?.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(byte) { uri.push(char::from(*byte)); }
        else { uri.push_str(&format!("%{byte:02X}")); }
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("oneMK-picture-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap(); Self(path)
        }
    }
    impl Drop for Directory { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    fn plan() -> Plan {
        Plan { hash: "a".repeat(64), title: "Picture".into(), media: Media { width: 8, height: 4, duration_ms: 1, frame_rate: 0 },
            canvas: crate::Canvas { monitors: [0, 4].into_iter().map(|horizontal| Monitor { computer: "host".into(), name: horizontal.to_string(),
                rect: crate::Rect { x: f64::from(horizontal), y: 0.0, width: 4.0, height: 4.0 }, local: None, scale: 1.0, online: true }).collect() },
            start_ms: 1, fit: crate::Fit::Stretch, computers: vec!["host".into()], pause_on_battery: false, paused: false, picture: true, asset_hash: None }
    }
    #[test]
    fn pictures_are_recognized_without_admitting_arbitrary_extensions() {
        assert!(is_picture(Path::new("photo.JPEG")));
        assert!(is_picture(Path::new("photo.heic")));
        assert!(!is_picture(Path::new("video.mp4")));
    }
    #[test]
    fn picture_bundles_preserve_canvas_slices_and_reject_corruption() {
        let root = Directory::new();
        let source = root.0.join("source.png");
        let image = RgbaImage::from_fn(8, 4, |horizontal, vertical| image::Rgba([(horizontal * 30) as u8, (vertical * 60) as u8, 0, 255]));
        image.save(&source).unwrap();
        let mut plan = plan();
        let bundle = root.0.join("bundle.video");
        prepare(&source, &plan, "host", &bundle).unwrap();
        let files = slices(&plan, "host", &bundle, &root.0.join("slices")).unwrap();
        for (index, (_, path)) in files.iter().enumerate() {
            let slice = open(path).unwrap();
            for (horizontal, vertical, pixel) in slice.enumerate_pixels() { assert_eq!(pixel, image.get_pixel(horizontal + index as u32 * 4, vertical)); }
        }
        plan.canvas.monitors[0].scale = 2.0;
        assert!(slices(&plan, "host", &bundle, &root.0.join("bad-size")).is_err());
        plan.canvas.monitors[0].scale = 1.0;
        std::fs::OpenOptions::new().append(true).open(&bundle).unwrap().write_all(b"extra").unwrap();
        assert!(slices(&plan, "host", &bundle, &root.0.join("bad-tail")).is_err());
    }
    #[test]
    fn fit_picture_has_black_borders_and_no_player() {
        let root = Directory::new();
        let source = root.0.join("source.png");
        RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
        let mut plan = plan();
        plan.canvas.monitors.truncate(1);
        plan.fit = crate::Fit::Fit;
        let bundle = root.0.join("bundle.video");
        prepare(&source, &plan, "host", &bundle).unwrap();
        let files = slices(&plan, "host", &bundle, &root.0.join("slices")).unwrap();
        let slice = open(&files[0].1).unwrap();
        assert_eq!(slice.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(slice.get_pixel(0, 1).0, [255, 0, 0, 255]);
        restore(&root.0).unwrap();
    }
}
