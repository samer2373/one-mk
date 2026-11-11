use one_mk_wallpaper::{Media, Monitor, Plan};
#[cfg(target_os = "linux")]
use one_mk_wallpaper::Rect;
use anyhow::{Context, Result, ensure};
use image::{ImageDecoder, ImageReader, RgbaImage};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

pub fn command(arguments: &[std::ffi::OsString]) -> Result<()> {
    let action = arguments.first().context("Missing picture operation")?.to_str().context("Invalid picture operation")?;
    if action == "--picture-restore" {
        ensure!(arguments.len() == 2, "Invalid picture restore arguments");
        return restore(Path::new(&arguments[1]));
    }
    if action == "--picture-inspect" {
        ensure!(arguments.len() == 3, "Invalid picture inspect arguments");
        let media = inspect(Path::new(&arguments[1]), Path::new(&arguments[2]))?;
        println!("{}", serde_json::to_string(&media)?);
        return Ok(());
    }
    ensure!(arguments.len() == 5, "Invalid picture arguments");
    let json = arguments[3].to_str().context("Invalid picture layout")?;
    ensure!(json.len() <= 12000, "The picture layout is too large");
    let plan: Plan = serde_json::from_str(json)?;
    ensure!(plan.valid() && plan.picture, "Invalid picture wallpaper plan");
    let computer = arguments[4].to_str().context("Invalid picture computer")?;
    ensure!(!computer.is_empty() && computer.len() <= 128, "Invalid picture computer");
    match action {
        "--picture-prepare" => prepare(Path::new(&arguments[1]), &plan, computer, Path::new(&arguments[2])),
        "--picture-apply" => { println!("{}", serde_json::to_string(&apply(&plan, computer, Path::new(&arguments[1]), Path::new(&arguments[2]))?)?); Ok(()) },
        _ => anyhow::bail!("Unknown picture operation"),
    }
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_file(path, |file| { file.write_all(bytes)?; Ok(()) })
}

fn atomic_file(path: &Path, write: impl FnOnce(&mut std::fs::File) -> Result<()>) -> Result<()> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = path.with_extension(format!("{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        write(&mut file)?; file.sync_all()?; drop(file);
        std::fs::rename(&temporary, path)?;
        // The rename itself survives a crash or power cut only once the
        // folder is written too (the saved original wallpaper depends on it).
        #[cfg(unix)]
        if let Some(parent) = path.parent() { std::fs::File::open(parent)?.sync_all()?; }
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(temporary); }
    result
}

fn referenced(value: &serde_json::Value, path: &Path) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains(path.file_name().unwrap().to_string_lossy().as_ref()),
        serde_json::Value::Array(values) => values.iter().any(|value| referenced(value, path)),
        serde_json::Value::Object(values) => values.values().any(|value| referenced(value, path)),
        _ => false,
    }
}

fn prune_pictures(root: &Path, keep: &[PathBuf]) -> Result<()> {
    let snapshot = root.join("previous.json");
    let saved = if snapshot.exists() { serde_json::from_slice(&std::fs::read(snapshot)?)? } else { serde_json::Value::Null };
    let current = root.join("current.json");
    let current = if current.exists() { serde_json::from_slice(&std::fs::read(current)?)? } else { serde_json::Value::Null };
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "png") && !keep.contains(&path) && !referenced(&saved, &path) && !referenced(&current, &path) { std::fs::remove_file(path)?; }
    }
    Ok(())
}

fn write_picture(root: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    fn occupied(root: &Path) -> Result<u64> {
        let mut bytes = 0u64;
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            bytes = bytes.saturating_add(if kind.is_file() { entry.metadata()?.len() } else if kind.is_dir() { occupied(&entry.path())? } else { 0 });
        }
        Ok(bytes)
    }
    ensure!(occupied(root.parent().context("The picture cache has no parent")?)?.saturating_add(bytes.len() as u64) <= 8 << 30, "The wallpaper cache is full. Turn off wallpaper before choosing another picture.");
    atomic_write(path, bytes)
}

#[cfg(any(target_os = "macos", windows, test))]
pub(super) fn merge_snapshot<Entry>(saved: &mut Vec<Entry>, observed: impl IntoIterator<Item = Entry>, key: impl Fn(&Entry) -> &str) {
    for entry in observed { if !saved.iter().any(|previous| key(previous) == key(&entry)) { saved.push(entry); } }
}

#[cfg(any(target_os = "macos", windows, test))]
pub(super) fn unrestored<Entry>(saved: Vec<Entry>, mut restore: impl FnMut(&Entry) -> bool) -> Vec<Entry> {
    saved.into_iter().filter(|entry| !restore(entry)).collect()
}

#[cfg(any(target_os = "macos", windows, test))]
pub(super) fn native_or_overlay<Prepared>(prepared: Result<Prepared>, apply: impl FnOnce(Prepared) -> Result<()>) -> Result<one_mk_wallpaper::picture::Mode> {
    use one_mk_wallpaper::picture::Mode;
    if Mode::for_native_safety(prepared.is_ok()) == Mode::Overlay { return Ok(Mode::Overlay); }
    // A failed OS write is not a safety refusal: it may already have changed
    // a screen. Keep its snapshot and surface the error instead of hiding it.
    apply(prepared?)?;
    Ok(Mode::Native)
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn restorable_macos_store(text: &str) -> bool {
    let providers: Vec<_> = text.split("<string>com.apple.wallpaper.choice.").skip(1).filter_map(|part| part.split_once("</string>").map(|(provider, _)| provider)).collect();
    let spaces = text.split("<key>Spaces</key>").skip(1).map(str::trim_start);
    !text.contains("<data>") && !providers.is_empty() && providers.iter().all(|provider| *provider == "image")
        && spaces.into_iter().all(|spaces| spaces.starts_with("<dict/>") || spaces.starts_with("<dict></dict>"))
}

#[cfg(target_os = "macos")]
#[path = "picture_macos.rs"]
mod native;

#[cfg(windows)]
#[path = "picture_windows.rs"]
mod native;

/// A picture's pixels. A decoder's own complaint ("Not enough bytes…") is
/// said as what it means to a person; a size refusal is already worded.
fn open(path: &Path) -> Result<RgbaImage> {
    decode(path).map_err(|error| if error.to_string().contains("too large") { error }
        else { error.context("This picture cannot be opened: it may be damaged or in a format oneMK can't read. Try a PNG, JPEG or WebP picture.") })
}

fn decode(path: &Path) -> Result<RgbaImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    match reader.format() {
        Some(image::ImageFormat::Jpeg) => return jpeg(path),
        Some(image::ImageFormat::WebP) => return webp(path),
        _ => {}
    }
    #[cfg(any(target_os = "macos", windows))]
    if reader.format().is_none() { return native::decode(path); }
    reader.limits(limits());
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure!(u64::from(width) * u64::from(height) <= 64 * 1024 * 1024, "This picture is too large");
    let orientation = decoder.orientation()?;
    let mut pixels = image::DynamicImage::from_decoder(decoder)?;
    pixels.apply_orientation(orientation);
    ensure!(u64::from(pixels.width()) * u64::from(pixels.height()) <= 64 * 1024 * 1024, "This picture is too large");
    Ok(pixels.into_rgba8())
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits
}

fn pixel_limits(width: u32, height: u32) -> Result<()> {
    let mut limits = limits();
    limits.check_dimensions(width, height)?;
    ensure!(width > 0 && height > 0, "This picture has no pixels");
    limits.reserve(u64::from(width) * u64::from(height) * 4)?;
    Ok(())
}

fn jpeg(path: &Path) -> Result<RgbaImage> {
    let mut input = std::fs::File::open(path)?;
    ensure!(input.metadata()?.len() <= 256 * 1024 * 1024, "This compressed picture is too large");
    let mut bytes = Vec::new();
    (&mut input).take(256 * 1024 * 1024).read_to_end(&mut bytes)?;
    ensure!(input.read(&mut [0])? == 0, "This compressed picture is too large");
    let options = zune_core::options::DecoderOptions::default().set_strict_mode(false)
        .set_max_width(16384).set_max_height(16384).jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(zune_core::bytestream::ZCursor::new(bytes.as_slice()), options);
    decoder.decode_headers().map_err(|error| anyhow::anyhow!("{error}"))?;
    let (width, height) = decoder.dimensions().context("This picture has no dimensions")?;
    let (width, height) = (u32::try_from(width)?, u32::try_from(height)?);
    pixel_limits(width, height)?;
    let orientation = decoder.exif().and_then(|exif| image::metadata::Orientation::from_exif_chunk(exif));
    let bytes = decoder.decode().map_err(|error| anyhow::anyhow!("{error}"))?;
    let pixels = RgbaImage::from_raw(width, height, bytes).context("This picture has invalid pixels")?;
    let mut pixels = image::DynamicImage::ImageRgba8(pixels);
    if let Some(orientation) = orientation { pixels.apply_orientation(orientation); }
    Ok(pixels.into_rgba8())
}

fn webp(path: &Path) -> Result<RgbaImage> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::BufReader::new(std::fs::File::open(path)?))?;
    decoder.set_memory_limit(256 * 1024 * 1024);
    let (width, height) = decoder.dimensions();
    pixel_limits(width, height)?;
    let orientation = decoder.exif_metadata()?.and_then(|exif| image::metadata::Orientation::from_exif_chunk(&exif));
    let mut bytes = vec![0; decoder.output_buffer_size().context("This picture is too large")?];
    decoder.read_image(&mut bytes)?;
    let mut pixels = if decoder.has_alpha() {
        image::DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, bytes).context("This picture has invalid pixels")?)
    } else {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(width, height, bytes).context("This picture has invalid pixels")?)
    };
    if let Some(orientation) = orientation { pixels.apply_orientation(orientation); }
    Ok(pixels.into_rgba8())
}

pub fn inspect(path: &Path, poster: &Path) -> Result<Media> {
    let image = open(path)?;
    let mut png = Cursor::new(Vec::new());
    image::imageops::thumbnail(&image, 1600, 900).write_to(&mut png, image::ImageFormat::Png)?;
    atomic_write(poster, &png.into_inner())?;
    Ok(Media { width: image.width(), height: image.height(), duration_ms: 1, frame_rate: 0 })
}

pub fn prepare(source: &Path, plan: &Plan, computer: &str, path: &Path) -> Result<()> {
    let image = open(source)?;
    ensure!(plan.canvas.monitors.iter().any(|monitor| monitor.computer == computer), "No screens selected on this computer");
    ensure!(image.dimensions() == (plan.media.width, plan.media.height), "The picture dimensions changed");
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    let bytes = png.into_inner();
    ensure!(bytes.len() <= 256 * 1024 * 1024, "This compressed picture is too large");
    // Keep the original pixels, not just today's slices: a peer can then
    // reslice after a screen is connected without asking the owner for a file.
    atomic_file(path, |output| {
        output.write_all(b"oneMKpi2")?;
        output.write_all(&(bytes.len() as u64).to_le_bytes())?;
        output.write_all(&bytes)?;
        Ok(())
    })
}

fn source(input: &mut impl Read, plan: &Plan) -> Result<RgbaImage> {
    let mut length = [0; 8]; input.read_exact(&mut length)?;
    let length = u64::from_le_bytes(length);
    ensure!(length <= 256 * 1024 * 1024, "The received picture is too large");
    let mut bytes = vec![0; length as usize]; input.read_exact(&mut bytes)?;
    ensure!(input.read(&mut [0])? == 0, "The received picture has unexpected contents");
    ensure!(image::guess_format(&bytes)? == image::ImageFormat::Png, "The received picture must be PNG");
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    reader.limits(limits());
    let decoder = reader.into_decoder()?;
    ensure!(decoder.dimensions() == (plan.media.width, plan.media.height), "The received picture dimensions do not match the plan");
    pixel_limits(plan.media.width, plan.media.height)?;
    Ok(image::DynamicImage::from_decoder(decoder)?.into_rgba8())
}

pub fn player_source(config: &one_mk_wallpaper::PlayerConfig) -> Result<RgbaImage> {
    let mut input = std::fs::File::open(&config.path)?;
    let mut magic = [0; 8]; input.read_exact(&mut magic)?;
    ensure!(&magic == b"oneMKpi2", "Choose the picture again to show it without changing your own wallpaper");
    source(&mut input, &config.plan)
}

pub fn render(image: &RgbaImage, plan: &Plan, monitor: &Monitor) -> Result<RgbaImage> {
    let slice = plan.slice(monitor).context("Empty picture layout")?;
    let width = (monitor.rect.width * monitor.scale).round() as u32;
    let height = (monitor.rect.height * monitor.scale).round() as u32;
    pixel_limits(width, height).context("This screen is too large for a picture wallpaper")?;
    let mut pixels = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for (horizontal, vertical, pixel) in pixels.enumerate_pixels_mut() {
        let source_x = slice.x + (f64::from(horizontal) + 0.5) / f64::from(width) * slice.width;
        let source_y = slice.y + (f64::from(vertical) + 0.5) / f64::from(height) * slice.height;
        if let Some(sample) = image::imageops::sample_bilinear(image, source_x as f32, source_y as f32) {
            for channel in 0..3 { pixel.0[channel] = ((u16::from(sample.0[channel]) * u16::from(sample.0[3]) + 127) / 255) as u8; }
        }
    }
    Ok(pixels)
}

fn slices(plan: &Plan, computer: &str, path: &Path, root: &Path) -> Result<Vec<(Monitor, PathBuf)>> {
    std::fs::create_dir_all(root)?;
    let mut input = std::fs::File::open(path)?;
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    if &magic == b"oneMKpi2" {
        let image = source(&mut input, plan)?;
        let mut files = Vec::new();
        for (index, monitor) in plan.canvas.monitors.iter().filter(|monitor| monitor.computer == computer).enumerate() {
            let pixels = render(&image, plan, monitor)?;
            let mut png = Cursor::new(Vec::new()); pixels.write_to(&mut png, image::ImageFormat::Png)?;
            let destination = root.join(format!("{}-{index}-{}.png", plan.asset_hash.as_ref().unwrap_or(&plan.hash), std::process::id()));
            write_picture(root, &destination, &png.into_inner())?;
            files.push((monitor.clone(), destination));
        }
        ensure!(!files.is_empty(), "No screens selected on this computer");
        return Ok(files);
    }
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
        ensure!(image::guess_format(&bytes)? == image::ImageFormat::Png, "The received picture slice must be PNG");
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png);
        reader.limits(limits());
        let decoder = reader.into_decoder()?;
        let (width, height) = decoder.dimensions();
        ensure!(width == (monitor.rect.width * monitor.scale).round() as u32 && height == (monitor.rect.height * monitor.scale).round() as u32
            && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024, "The received picture does not match this screen");
        let pixels = image::DynamicImage::from_decoder(decoder)?;
        let mut png = Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png)?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let destination = root.join(format!("{}-{index}-{}-{}.png", plan.asset_hash.as_ref().unwrap_or(&plan.hash), std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        write_picture(root, &destination, &png.into_inner())?;
        files.push((monitor.clone(), destination));
    }
    ensure!(input.read(&mut [0])? == 0, "The received picture has unexpected contents");
    Ok(files)
}

pub fn apply(plan: &Plan, computer: &str, path: &Path, root: &Path) -> Result<one_mk_wallpaper::picture::Mode> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && one_mk_wallpaper::wayland::available().is_ok() {
        return Ok(one_mk_wallpaper::picture::Mode::Overlay);
    }
    std::fs::create_dir_all(root)?;
    prune_pictures(root, &[])?;
    let files = slices(plan, computer, path, root)?;
    #[cfg(any(target_os = "macos", windows))]
    {
        let result = native::apply(&files, root);
        let paths: Vec<_> = files.iter().map(|(_, path)| path.clone()).collect();
        if result.is_ok() { atomic_write(&root.join("current.json"), &serde_json::to_vec(&paths)?)?; }
        prune_pictures(root, &paths)?;
        result
    }
    #[cfg(target_os = "linux")]
    {
        // Other Linux desktops (KDE, Xfce, Cinnamon…) keep their wallpaper
        // elsewhere: writing GNOME's keys there would report a change nobody
        // sees. Checked after the picture itself, so a bad picture still says so.
        if !one_mk_wallpaper::picture::gnome_desktop() {
            prune_pictures(root, &[])?;
            anyhow::bail!("{}", one_mk_wallpaper::picture::UNSUPPORTED_DESKTOP);
        }
        let bounds = files.iter().map(|(monitor, _)| monitor.local.unwrap_or(monitor.rect)).reduce(Rect::union).context("No picture screens")?;
        ensure!(bounds.width * bounds.height <= 64.0 * 1024.0 * 1024.0, "This desktop is too large for a picture wallpaper");
        let mut desktop = RgbaImage::new(bounds.width as u32, bounds.height as u32);
        for (monitor, path) in files {
            let rect = monitor.local.unwrap_or(monitor.rect);
            let pixels = image::imageops::resize(&open(&path)?, rect.width as u32, rect.height as u32, image::imageops::FilterType::Triangle);
            image::imageops::overlay(&mut desktop, &pixels, (rect.x - bounds.x) as i64, (rect.y - bounds.y) as i64);
        }
        let path = root.join(format!("{}-spanned.png", plan.asset_hash.as_ref().unwrap_or(&plan.hash)));
        let mut png = Cursor::new(Vec::new());
        desktop.write_to(&mut png, image::ImageFormat::Png)?;
        write_picture(root, &path, &png.into_inner())?;
        let snapshot = root.join("previous.json");
        if !snapshot.exists() { atomic_write(&snapshot, &serde_json::to_vec(&gnome_snapshot(root)?)?)?; }
        let uri = file_uri(&path)?;
        for key in ["picture-uri", "picture-uri-dark"] { settings("set", key, Some(&uri))?; }
        settings("set", "picture-options", Some("spanned"))?;
        atomic_write(&root.join("current.json"), &serde_json::to_vec(&[&path])?)?;
        prune_pictures(root, &[path])?;
        Ok(one_mk_wallpaper::picture::Mode::Native)
    }
}

pub fn restore(root: &Path) -> Result<()> {
    if !root.join("previous.json").exists() { return Ok(()); }
    #[cfg(any(target_os = "macos", windows))]
    ensure!(native::restore(root)?, "Some original wallpapers could not be restored yet. Reconnect those screens and return to the original desktop, then turn wallpaper off again. The snapshot and pictures were retained.");
    #[cfg(target_os = "linux")]
    {
        // Older snapshots hold a value for every key; newer ones hold none
        // for a key that was at the desktop's default, which is reset.
        let previous: Vec<(String, Option<String>)> = serde_json::from_slice(&std::fs::read(root.join("previous.json"))?)?;
        ensure!(previous.iter().all(|(key, _)| GNOME_KEYS.contains(&key.as_str())), "The saved wallpaper settings are not recognised");
        for (key, value) in previous {
            match value { Some(value) => settings("set", &key, Some(value.trim()))?, None => settings("reset", &key, None)? };
        }
    }
    std::fs::remove_file(root.join("previous.json"))?;
    let _ = std::fs::remove_file(root.join("current.json"));
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "png") { let _ = std::fs::remove_file(path); }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
const GNOME_KEYS: [&str; 3] = ["picture-uri", "picture-uri-dark", "picture-options"];

/// The person's own wallpaper, read once before oneMK first changes it. A
/// key still at the desktop's default is saved as `None` and restored with
/// a reset, so a later desktop or theme default keeps applying. A key that
/// already points at oneMK's own pictures (an earlier snapshot was lost) is
/// never saved as the original: it is reset too.
#[cfg(target_os = "linux")]
fn gnome_snapshot(root: &Path) -> Result<Vec<(String, Option<String>)>> {
    let ours = file_uri(root)?;
    GNOME_KEYS.into_iter().map(|key| {
        let value = settings("get", key, None)?.trim().to_owned();
        let own = value.trim_matches('\'').starts_with(&ours);
        let default = default_setting(key);
        Ok((key.to_owned(), (!own && default.as_deref() != Some(value.as_str())).then_some(value)))
    }).collect()
}

/// The desktop's default for `key`: GSettings' memory backend reads only the
/// schema and its vendor overrides, never the person's settings. `None` if
/// it can't be read (the value is then always saved as it is).
#[cfg(target_os = "linux")]
fn default_setting(key: &str) -> Option<String> {
    let output = std::process::Command::new("gsettings").env("GSETTINGS_BACKEND", "memory")
        .args(["get", "org.gnome.desktop.background", key]).output().ok()?;
    (output.status.success() && output.stderr.is_empty()).then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
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

    #[test]
    fn only_a_read_only_safety_refusal_selects_the_picture_player() {
        use one_mk_wallpaper::picture::Mode;
        let writes = std::cell::Cell::new(0);
        let refused: Result<()> = Err(anyhow::anyhow!("opaque wallpaper"));
        assert_eq!(native_or_overlay(refused, |_| { writes.set(1); Ok(()) }).unwrap(), Mode::Overlay);
        assert_eq!(writes.get(), 0);
        assert_eq!(native_or_overlay(Ok(()), |_| { writes.set(1); Ok(()) }).unwrap(), Mode::Native);
        assert_eq!(writes.get(), 1);
        assert!(native_or_overlay(Ok(()), |_| anyhow::bail!("OS write failed")).unwrap_err().to_string().contains("OS write failed"));
    }
    #[test]
    fn macos_guard_refuses_opaque_dynamic_and_per_space_settings() {
        let image = "<string>com.apple.wallpaper.choice.image</string><key>Spaces</key><dict/>";
        assert!(restorable_macos_store(image));
        assert!(!restorable_macos_store(&image.replace("choice.image", "choice.aerial")));
        assert!(!restorable_macos_store(&image.replace("<dict/>", "<dict><key>space-id</key><dict/></dict>")));
        assert!(!restorable_macos_store(&format!("{image}<data>opaque settings</data>")));
        assert!(!restorable_macos_store(&format!("{image}<key>Spaces</key><dict><key>another-space</key><dict/></dict>")));
        assert!(!restorable_macos_store("<plist/>"));
    }
    #[test]
    fn picture_pruning_keeps_current_and_unrestored_screen_references() {
        let root = Directory::new();
        let pending = root.0.join("pending-screen.png");
        let current = root.0.join("current.png");
        let previous_attempt = root.0.join("failed-swap.png");
        let stale = root.0.join("stale.png");
        for path in [&pending, &current, &previous_attempt, &stale] { std::fs::write(path, b"picture").unwrap(); }
        atomic_write(&root.0.join("previous.json"), &serde_json::to_vec(&serde_json::json!({ "applied": { "unplugged": pending }, "previous_applied": { "failed-swap": previous_attempt } })).unwrap()).unwrap();
        atomic_write(&root.0.join("current.json"), &serde_json::to_vec(&[&current]).unwrap()).unwrap();
        prune_pictures(&root.0, &[]).unwrap();
        assert!(pending.exists() && current.exists() && previous_attempt.exists()); assert!(!stale.exists());
    }
    #[test]
    fn snapshots_merge_without_overwriting_and_retain_unrestored_screens() {
        let mut saved = vec![("stable-a".to_owned(), "original-a".to_owned())];
        merge_snapshot(&mut saved, [("stable-a".into(), "oneMK".into()), ("stable-b".into(), "original-b".into())], |entry| &entry.0);
        assert_eq!(saved[0].1, "original-a");
        let pending = unrestored(saved, |entry| entry.0 == "stable-b");
        assert_eq!(pending, [("stable-a".into(), "original-a".into())]);
        assert!(unrestored(pending, |_| true).is_empty());
    }
    #[test]
    fn atomic_picture_replacement_keeps_existing_readers_intact() {
        let root = Directory::new();
        let path = root.0.join("slice.png");
        atomic_write(&path, b"before").unwrap();
        let mut reader = std::fs::File::open(&path).unwrap();
        atomic_write(&path, b"after").unwrap();
        let mut original = String::new(); reader.read_to_string(&mut original).unwrap();
        assert_eq!(original, "before");
        assert_eq!(std::fs::read(path).unwrap(), b"after");
    }
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
            canvas: one_mk_wallpaper::Canvas { monitors: [0, 4].into_iter().map(|horizontal| Monitor { computer: "host".into(), name: horizontal.to_string(),
                rect: one_mk_wallpaper::Rect { x: f64::from(horizontal), y: 0.0, width: 4.0, height: 4.0 }, local: None, scale: 1.0, online: true }).collect() },
            start_ms: 1, fit: one_mk_wallpaper::Fit::Stretch, computers: vec!["host".into()], pause_on_battery: false, paused: false, picture: true, asset_hash: None, spread: Default::default() }
    }
    #[test]
    fn pictures_are_recognized_without_admitting_arbitrary_extensions() {
        use one_mk_wallpaper::picture::is_picture;
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
        let config = one_mk_wallpaper::PlayerConfig { plan: plan.clone(), computer: "host".into(), path: bundle.clone(), clock_offset_ms: 0 };
        assert_eq!(player_source(&config).unwrap(), image);
        let files = slices(&plan, "host", &bundle, &root.0.join("slices")).unwrap();
        for (index, (_, path)) in files.iter().enumerate() {
            let slice = open(path).unwrap();
            for (horizontal, vertical, pixel) in slice.enumerate_pixels() { assert_eq!(pixel, image.get_pixel(horizontal + index as u32 * 4, vertical)); }
        }
        plan.canvas.monitors[0].scale = 2.0;
        let resized = slices(&plan, "host", &bundle, &root.0.join("new-size")).unwrap();
        assert_eq!(open(&resized[0].1).unwrap().dimensions(), (8, 8));
        plan.canvas.monitors[0].scale = 1.0;
        std::fs::OpenOptions::new().append(true).open(&bundle).unwrap().write_all(b"extra").unwrap();
        assert!(slices(&plan, "host", &bundle, &root.0.join("bad-tail")).is_err());
    }
    #[test]
    fn picture_player_slicing_uses_the_video_canvas_for_every_fit() {
        use one_mk_wallpaper::Fit;
        let mut plan = plan();
        plan.media.height = 8;
        for monitor in &mut plan.canvas.monitors { monitor.rect.x -= 20.0; monitor.rect.y = 11.0; }
        let pixels = RgbaImage::from_fn(8, 8, |horizontal, vertical| image::Rgba([(horizontal * 30) as u8, (vertical * 30) as u8, 0, 255]));
        for (fit, left, right) in [(Fit::Fill, [0, 60, 0, 255], [120, 60, 0, 255]),
            (Fit::Fit, [0, 0, 0, 255], [135, 15, 0, 255]), (Fit::Stretch, [0, 15, 0, 255], [120, 15, 0, 255])] {
            plan.fit = fit;
            assert_eq!(render(&pixels, &plan, &plan.canvas.monitors[0]).unwrap().get_pixel(0, 0).0, left);
            assert_eq!(render(&pixels, &plan, &plan.canvas.monitors[1]).unwrap().get_pixel(0, 0).0, right);
        }
        plan.fit = Fit::Stretch;
        plan.canvas.monitors[1].computer = "peer".into();
        plan.canvas.monitors[0].scale = 2.0;
        assert_eq!(render(&pixels, &plan, &plan.canvas.monitors[0]).unwrap().dimensions(), (8, 8));
        let before = render(&pixels, &plan, &plan.canvas.monitors[0]).unwrap();
        plan.canvas.monitors[0].rect.width = 8.0;
        let after = render(&pixels, &plan, &plan.canvas.monitors[0]).unwrap();
        assert_eq!(after.dimensions(), (16, 8));
        assert_ne!(before.get_pixel(7, 3), after.get_pixel(15, 3));
    }

    #[test]
    fn each_screen_shows_the_whole_picture_by_itself() {
        use one_mk_wallpaper::{Fit, Spread};
        let mut plan = plan();
        plan.fit = Fit::Stretch;
        plan.spread = Spread::Each;
        // A picture whose left edge is red and right edge is blue.
        let pixels = RgbaImage::from_fn(8, 8, |horizontal, _| if horizontal < 4 { image::Rgba([250, 0, 0, 255]) } else { image::Rgba([0, 0, 250, 255]) });
        for monitor in &plan.canvas.monitors {
            let shown = render(&pixels, &plan, monitor).unwrap();
            let (width, _) = shown.dimensions();
            assert_eq!(shown.get_pixel(0, 0).0, [250, 0, 0, 255], "every screen starts on the picture's left edge");
            assert_eq!(shown.get_pixel(width - 1, 0).0, [0, 0, 250, 255], "and ends on its right edge");
        }
        plan.spread = Spread::Span;
        let first = render(&pixels, &plan, &plan.canvas.monitors[0]).unwrap();
        let last = &plan.canvas.monitors[plan.canvas.monitors.len() - 1];
        let second = render(&pixels, &plan, last).unwrap();
        assert_eq!(first.get_pixel(first.width() - 1, 0).0, [250, 0, 0, 255], "spanning cuts the picture between the screens");
        assert_eq!(second.get_pixel(second.width() - 1, 0).0, [0, 0, 250, 255]);
    }

    #[test]
    fn transparent_picture_pixels_are_composited_over_black() {
        let plan = plan();
        let pixels = RgbaImage::from_pixel(8, 4, image::Rgba([200, 100, 50, 128]));
        assert_eq!(render(&pixels, &plan, &plan.canvas.monitors[0]).unwrap().get_pixel(1, 1).0, [100, 50, 25, 255]);
    }

    #[test]
    fn picture_player_rejects_wrong_dimensions_and_trailing_data() {
        let root = Directory::new();
        let source = root.0.join("source.png");
        RgbaImage::new(8, 4).save(&source).unwrap();
        let bundle = root.0.join("bundle.video");
        let mut config = one_mk_wallpaper::PlayerConfig { plan: plan(), computer: "host".into(), path: bundle.clone(), clock_offset_ms: 0 };
        prepare(&source, &config.plan, "host", &bundle).unwrap();
        config.plan.media.width = 9;
        assert!(player_source(&config).is_err());
        config.plan.media.width = 8;
        std::fs::OpenOptions::new().append(true).open(&bundle).unwrap().write_all(b"extra").unwrap();
        assert!(player_source(&config).is_err());
    }
    #[test]
    fn fit_picture_has_black_borders_and_no_player() {
        let root = Directory::new();
        let source = root.0.join("source.png");
        RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
        let mut plan = plan();
        plan.canvas.monitors.truncate(1);
        plan.fit = one_mk_wallpaper::Fit::Fit;
        let bundle = root.0.join("bundle.video");
        prepare(&source, &plan, "host", &bundle).unwrap();
        let files = slices(&plan, "host", &bundle, &root.0.join("slices")).unwrap();
        let slice = open(&files[0].1).unwrap();
        assert_eq!(slice.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(slice.get_pixel(0, 1).0, [255, 0, 0, 255]);
        restore(&root.0).unwrap();
    }
}
