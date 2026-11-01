use crate::Input;
use anyhow::{Context, Result, ensure};
use one_mk_wallpaper::{Correction, Hold, Holds, Media, PlayerConfig, correction, now_ms, position};
use std::{path::Path, sync::{Arc, atomic::{AtomicU32, Ordering}}, sync::mpsc::Receiver, time::{Duration, Instant}};
use windows::{core::{BOOL, BSTR, Interface, implement, w}, Win32::{Foundation::*, Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::*, Dxgi::{Common::*, *}}, Media::MediaFoundation::*, System::{Com::*, Power::*}, UI::WindowsAndMessaging::*}};
use windows::Win32::Graphics::Gdi::ScreenToClient;

#[implement(IMFMediaEngineNotify)]
struct Notify(Arc<AtomicU32>);
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _: usize, _: u32) -> windows::core::Result<()> { self.0.store(event, Ordering::Relaxed); Ok(()) }
}

struct Engine { media: IMFMediaEngine, device: ID3D11Device, _manager: IMFDXGIDeviceManager, events: Arc<AtomicU32> }
impl Engine {
    unsafe fn new(path: &Path) -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
            let mut device = None;
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
            let device = device.context("Windows could not create the video device")?;
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.context("Windows could not create the video decoder")?;
            manager.ResetDevice(&device, token)?;
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 4)?;
            let attributes = attributes.context("Windows could not prepare video playback")?;
            let events = Arc::new(AtomicU32::new(0));
            let notify: IMFMediaEngineNotify = Notify(events.clone()).into();
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
            let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let media = factory.CreateInstance(0, &attributes)?;
            media.SetMuted(true)?;
            media.SetLoop(true)?;
            media.SetSource(&BSTR::from(path.to_string_lossy().as_ref()))?;
            media.Load()?;
            Ok(Self { media, device, _manager: manager, events })
        }
    }
    fn metadata(&self) -> Result<Media> {
        let started = Instant::now();
        loop {
            let mut width = 0;
            let mut height = 0;
            unsafe { self.media.GetNativeVideoSize(Some(&mut width), Some(&mut height))?; }
            let seconds = unsafe { self.media.GetDuration() };
            if width > 0 && height > 0 && seconds.is_finite() && seconds > 0.0 { return Ok(Media { width, height, duration_ms: (seconds * 1000.0) as u64 }); }
            ensure!(self.events.load(Ordering::Relaxed) != MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32, "Windows cannot decode this video. Try an H.264 MP4 video.");
            ensure!(started.elapsed() < Duration::from_secs(20), "Windows took too long to open this video");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) { unsafe { let _ = self.media.Shutdown(); let _ = MFShutdown(); CoUninitialize(); } }
}

pub fn probe(path: &Path, poster: Option<&Path>) -> Result<Media> {
    let engine = unsafe { Engine::new(path)? };
    let media = engine.metadata()?;
    if let Some(path) = poster { unsafe { save_poster(&engine, &media, path)?; } }
    Ok(media)
}

unsafe fn save_poster(engine: &Engine, media: &Media, path: &Path) -> Result<()> {
    unsafe {
        let scale = (1600.0 / f64::from(media.width)).min(900.0 / f64::from(media.height)).min(1.0);
        let width = (f64::from(media.width) * scale).round().max(1.0) as u32;
        let height = (f64::from(media.height) * scale).round().max(1.0) as u32;
        let mut description = D3D11_TEXTURE2D_DESC { Width: width, Height: height, MipLevels: 1, ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32, ..Default::default() };
        let mut texture = None;
        engine.device.CreateTexture2D(&description, None, Some(&mut texture))?;
        let texture = texture.context("Windows could not prepare the video preview")?;
        engine.media.Play()?;
        let started = Instant::now();
        while engine.media.OnVideoStreamTick().is_err() {
            ensure!(started.elapsed() < Duration::from_secs(10), "Windows could not read the video preview");
            std::thread::sleep(Duration::from_millis(10));
        }
        engine.media.TransferVideoFrame(&texture, None, &RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 }, None)?;
        engine.media.Pause()?;
        description.Usage = D3D11_USAGE_STAGING;
        description.BindFlags = 0;
        description.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        engine.device.CreateTexture2D(&description, None, Some(&mut staging))?;
        let staging = staging.context("Windows could not read the video preview")?;
        let context = engine.device.GetImmediateContext()?;
        context.CopyResource(&staging, &texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let bytes = std::slice::from_raw_parts(mapped.pData.cast::<u8>().add(row as usize * mapped.RowPitch as usize), width as usize * 4);
            for pixel in bytes.chunks_exact(4) { rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]); }
        }
        context.Unmap(&staging, 0);
        image::save_buffer_with_format(path, &rgba, width, height, image::ColorType::Rgba8, image::ImageFormat::Png)?;
        Ok(())
    }
}

unsafe extern "system" fn find_worker(window: HWND, parameter: LPARAM) -> BOOL {
    unsafe {
        if FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), None).is_ok()
            && let Ok(worker) = FindWindowExW(None, Some(window), w!("WorkerW"), None)
        { *(parameter.0 as *mut HWND) = worker; }
    }
    true.into()
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, first: WPARAM, second: LPARAM) -> LRESULT {
    unsafe { if message == WM_NCHITTEST { LRESULT(HTTRANSPARENT as isize) } else { DefWindowProcW(window, message, first, second) } }
}

struct Desktop { windows: Vec<HWND>, chains: Vec<IDXGISwapChain>, destinations: Vec<RECT>, sources: Vec<MFVideoNormalizedRect> }
impl Desktop {
    unsafe fn new(engine: &Engine, config: &PlayerConfig) -> Result<Self> {
        unsafe {
            let progman = FindWindowW(w!("Progman"), None).context("The Windows desktop is not available")?;
            let _ = SendMessageTimeoutW(progman, 0x052c, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, None);
            let mut worker = HWND::default();
            EnumWindows(Some(find_worker), LPARAM(&mut worker as *mut HWND as isize))?;
            ensure!(!worker.is_invalid(), "Windows did not provide a wallpaper surface");
            let class = WNDCLASSW { lpfnWndProc: Some(window_proc), lpszClassName: w!("oneMKWallpaper"), ..Default::default() };
            RegisterClassW(&class);
            let dxgi: IDXGIDevice = engine.device.cast()?;
            let factory: IDXGIFactory = dxgi.GetAdapter()?.GetParent()?;
            let local: Vec<_> = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).collect();
            ensure!(!local.is_empty(), "No monitors selected on this computer");
            let mut desktop = Self { windows: Vec::new(), chains: Vec::new(), destinations: Vec::new(), sources: Vec::new() };
            for monitor in local {
                let width = (monitor.rect.width * monitor.scale) as u32;
                let height = (monitor.rect.height * monitor.scale) as u32;
                let local_rect = monitor.local.unwrap_or(monitor.rect);
                let mut origin = POINT { x: local_rect.x as i32, y: local_rect.y as i32 };
                let _ = ScreenToClient(worker, &mut origin);
                let window = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW, w!("oneMKWallpaper"), w!("oneMK Wallpaper"), WS_CHILD | WS_VISIBLE, origin.x, origin.y, width as i32, height as i32, Some(worker), None, None, None)?;
                desktop.windows.push(window);
                let descriptor = DXGI_SWAP_CHAIN_DESC { BufferDesc: DXGI_MODE_DESC { Width: width, Height: height, Format: DXGI_FORMAT_B8G8R8A8_UNORM, ..Default::default() }, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 2, OutputWindow: window, Windowed: true.into(), SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, ..Default::default() };
                let mut chain = None;
                factory.CreateSwapChain(&engine.device, &descriptor, &mut chain).ok()?;
                desktop.chains.push(chain.context("Windows could not create a wallpaper surface")?);
                desktop.destinations.push(RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 });
                let slice = config.plan.canvas.slice(monitor, &config.plan.media, config.plan.fit).context("Empty canvas")?;
                let left = slice.x.max(0.0);
                let top = slice.y.max(0.0);
                let right = (slice.x + slice.width).min(1.0);
                let bottom = (slice.y + slice.height).min(1.0);
                desktop.sources.push(MFVideoNormalizedRect { left: left as f32, top: top as f32, right: right as f32, bottom: bottom as f32 });
                if left != slice.x || top != slice.y || right != slice.x + slice.width || bottom != slice.y + slice.height {
                    let destination = desktop.destinations.last_mut().unwrap();
                    destination.left = ((left - slice.x) / slice.width * f64::from(width)).round() as i32;
                    destination.top = ((top - slice.y) / slice.height * f64::from(height)).round() as i32;
                    destination.right = ((right - slice.x) / slice.width * f64::from(width)).round() as i32;
                    destination.bottom = ((bottom - slice.y) / slice.height * f64::from(height)).round() as i32;
                }
            }
            Ok(desktop)
        }
    }
}
impl Drop for Desktop { fn drop(&mut self) { for window in &self.windows { unsafe { let _ = DestroyWindow(*window); } } } }

pub fn run(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    let engine = unsafe { Engine::new(&config.path)? };
    engine.metadata()?;
    let desktop = unsafe { Desktop::new(&engine, &config)? };
    let mut holds = Holds::default();
    let mut playing = false;
    let mut last_sync = Instant::now();
    let mut last_message = "";
    loop {
        match receiver.recv_timeout(Duration::from_millis(if playing { 8 } else { 250 })) {
            Ok(Input::End) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Ok(Input::Offset(offset)) => config.clock_offset_ms = offset,
            _ => {}
        }
        unsafe {
            let mut power = SYSTEM_POWER_STATUS::default();
            let _ = GetSystemPowerStatus(&mut power);
            holds.set(Hold::Battery, config.plan.pause_on_battery && power.ACLineStatus == 0);
            holds.set(Hold::User, config.plan.paused);
            if holds.message() != last_message { one_mk_wallpaper::report(&config, &holds); last_message = holds.message(); }
            if playing != holds.playing() {
                playing = holds.playing();
                if playing { engine.media.SetCurrentTime(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms) as f64 / 1000.0)?; engine.media.Play()?; }
                else { engine.media.Pause()?; }
            }
            if playing && last_sync.elapsed() >= Duration::from_secs(2) {
                let expected = position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms);
                match correction((engine.media.GetCurrentTime() * 1000.0) as u64, expected, config.plan.media.duration_ms) {
                    Correction::None => engine.media.SetPlaybackRate(1.0)?,
                    Correction::Rate(rate) => engine.media.SetPlaybackRate(rate)?,
                    Correction::Seek(target) => engine.media.SetCurrentTime(target as f64 / 1000.0)?,
                }
                last_sync = Instant::now();
            }
            if playing && engine.media.OnVideoStreamTick().is_ok() {
                for (index, chain) in desktop.chains.iter().enumerate() {
                    let source = &desktop.sources[index];
                    if source.left >= source.right || source.top >= source.bottom { continue; }
                    let surface: ID3D11Texture2D = chain.GetBuffer(0)?;
                    engine.media.TransferVideoFrame(&surface, Some(&desktop.sources[index]), &desktop.destinations[index], Some(&MFARGB { rgbAlpha: 255, ..Default::default() }))?;
                    chain.Present(0, DXGI_PRESENT(0)).ok()?;
                }
            }
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() { DispatchMessageW(&message); }
        }
    }
}
