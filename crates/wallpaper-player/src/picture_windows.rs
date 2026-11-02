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

pub fn apply(files: &[(Monitor, PathBuf)], root: &Path) -> Result<()> {
    let desktop = Desktop::new()?;
    unsafe {
        let mut monitors = Vec::new();
        for index in 0..desktop.0.GetMonitorDevicePathCount()? { monitors.push(string(desktop.0.GetMonitorDevicePathAt(index)?)?); }
        let snapshot = root.join("previous.json");
        if !snapshot.exists() {
            let mut previous = Vec::new();
            for monitor in &monitors { previous.push((monitor.clone(), string(desktop.0.GetWallpaper(PCWSTR(wide(monitor).as_ptr()))?)?)); }
            std::fs::write(&snapshot, serde_json::to_vec(&(previous, desktop.0.GetPosition()?.0))?)?;
        }
        desktop.0.SetPosition(DWPOS_STRETCH)?;
        for (monitor, path) in files {
            let local = monitor.local.unwrap_or(monitor.rect);
            let id = monitors.iter().find(|id| {
                desktop.0.GetMonitorRECT(PCWSTR(wide(id).as_ptr())).is_ok_and(|rect| (f64::from(rect.left) - local.x).abs() < 2.0 && (f64::from(rect.top) - local.y).abs() < 2.0)
            }).context("The picture's screen is no longer connected")?;
            desktop.0.SetWallpaper(PCWSTR(wide(id).as_ptr()), PCWSTR(wide(&path.to_string_lossy()).as_ptr()))?;
        }
    }
    Ok(())
}

pub fn restore(root: &Path) -> Result<()> {
    let (previous, position): (Vec<(String, String)>, i32) = serde_json::from_slice(&std::fs::read(root.join("previous.json"))?)?;
    let desktop = Desktop::new()?;
    unsafe {
        for (monitor, path) in previous { desktop.0.SetWallpaper(PCWSTR(wide(&monitor).as_ptr()), PCWSTR(wide(&path).as_ptr()))?; }
        desktop.0.SetPosition(DESKTOP_WALLPAPER_POSITION(position))?;
    }
    Ok(())
}
