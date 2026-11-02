use crate::Monitor;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use windows::{core::{PCWSTR, PWSTR}, Win32::{System::Com::*, UI::Shell::*}};

pub fn decode(path: &Path) -> Result<image::RgbaImage> {
    use windows::Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*};
    let _desktop = Desktop::new()?;
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).context("Windows could not open this picture. It may need a picture decoder from the Microsoft Store.")?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        anyhow::ensure!(width > 0 && height > 0 && width <= 16384 && height <= 16384 && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024, "This picture is too large");
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut pixels = image::RgbaImage::new(width, height);
        converter.CopyPixels(std::ptr::null(), width * 4, pixels.as_mut())?;
        Ok(pixels)
    }
}

struct Desktop(IDesktopWallpaper);
impl Desktop { fn new() -> Result<Self> { unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()?; Ok(Self(CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)?)) } } }
impl Drop for Desktop { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
unsafe fn string(pointer: PWSTR) -> Result<String> {
    let result = unsafe { pointer.to_string() };
    unsafe { CoTaskMemFree(Some(pointer.0.cast())); }
    Ok(result?)
}
fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot { screens: Vec<(String, String)>, position: i32, #[serde(default)] color: Option<u32>, #[serde(default)] applied: std::collections::BTreeMap<String, String>, #[serde(default)] previous_applied: std::collections::BTreeMap<String, String> }
fn snapshot(root: &Path) -> Result<Option<Snapshot>> {
    let path = root.join("previous.json");
    if !path.exists() { return Ok(None); }
    let bytes = std::fs::read(path)?;
    if let Ok(saved) = serde_json::from_slice(&bytes) { return Ok(Some(saved)); }
    let (screens, position) = serde_json::from_slice(&bytes)?;
    Ok(Some(Snapshot { screens, position, color: None, applied: Default::default(), previous_applied: Default::default() }))
}

pub fn apply(files: &[(Monitor, PathBuf)], root: &Path) -> Result<()> {
    let desktop = Desktop::new()?;
    unsafe {
        let slideshow_items = match desktop.0.GetSlideshow() { Ok(items) => items.GetCount()?, Err(_) => 0 };
        anyhow::ensure!(!desktop.0.GetStatus()?.contains(DSS_SLIDESHOW) && slideshow_items == 0,
            "Native pictures cannot safely replace a Windows slideshow. Choose a static picture in Windows first.");
        let mut monitors = Vec::new();
        for index in 0..desktop.0.GetMonitorDevicePathCount()? { monitors.push(string(desktop.0.GetMonitorDevicePathAt(index)?)?); }
        let snapshot = root.join("previous.json");
        let mut previous = self::snapshot(root)?.unwrap_or(Snapshot { screens: Vec::new(), position: desktop.0.GetPosition()?.0,
            color: Some(desktop.0.GetBackgroundColor()?.0), applied: Default::default(), previous_applied: Default::default() });
        let observed = monitors.iter().map(|monitor| Ok((monitor.clone(), string(desktop.0.GetWallpaper(PCWSTR(wide(monitor).as_ptr()))?)?))).collect::<Result<Vec<_>>>()?;
        super::merge_snapshot(&mut previous.screens, observed, |entry| &entry.0);
        super::atomic_write(&snapshot, &serde_json::to_vec(&previous)?)?;
        desktop.0.SetPosition(DWPOS_STRETCH)?;
        for (monitor, path) in files {
            let local = monitor.local.unwrap_or(monitor.rect);
            let id = monitors.iter().find(|id| {
                desktop.0.GetMonitorRECT(PCWSTR(wide(id).as_ptr())).is_ok_and(|rect| (f64::from(rect.left) - local.x).abs() < 2.0 && (f64::from(rect.top) - local.y).abs() < 2.0)
            }).context("The picture's screen is no longer connected")?;
            let current = string(desktop.0.GetWallpaper(PCWSTR(wide(id).as_ptr()))?)?;
            if previous.applied.get(id) == Some(&current) || previous.previous_applied.get(id) == Some(&current) { previous.previous_applied.insert(id.clone(), current); }
            else { previous.previous_applied.remove(id); }
            previous.applied.insert(id.clone(), path.to_string_lossy().into_owned());
            super::atomic_write(&snapshot, &serde_json::to_vec(&previous)?)?;
            desktop.0.SetWallpaper(PCWSTR(wide(id).as_ptr()), PCWSTR(wide(&path.to_string_lossy()).as_ptr()))?;
        }
    }
    Ok(())
}

pub fn restore(root: &Path) -> Result<bool> {
    let Some(mut previous) = snapshot(root)? else { return Ok(true) };
    let desktop = Desktop::new()?;
    unsafe {
        let monitors = (0..desktop.0.GetMonitorDevicePathCount()?).map(|index| string(desktop.0.GetMonitorDevicePathAt(index)?)).collect::<Result<Vec<_>>>()?;
        previous.screens = super::unrestored(previous.screens, |(monitor, path)| monitors.contains(monitor)
            && desktop.0.SetWallpaper(PCWSTR(wide(monitor).as_ptr()), PCWSTR(wide(path).as_ptr())).is_ok());
        previous.applied.retain(|monitor, _| previous.screens.iter().any(|(pending, _)| pending == monitor));
        previous.previous_applied.retain(|monitor, _| previous.screens.iter().any(|(pending, _)| pending == monitor));
        let position_restored = desktop.0.SetPosition(DESKTOP_WALLPAPER_POSITION(previous.position)).is_ok();
        let color_restored = previous.color.is_none_or(|color| desktop.0.SetBackgroundColor(windows::Win32::Foundation::COLORREF(color)).is_ok());
        super::atomic_write(&root.join("previous.json"), &serde_json::to_vec(&previous)?)?;
        Ok(previous.screens.is_empty() && position_restored && color_restored)
    }
}
