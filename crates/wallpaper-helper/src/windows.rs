use crate::Input;
use anyhow::{Context, Result, ensure};
use one_mk_wallpaper::{Correction, Hold, Holds, Media, PlayerConfig, correction, now_ms, position};
use std::{path::Path, sync::{Arc, OnceLock, atomic::{AtomicBool, AtomicU32, Ordering}}, sync::mpsc::Receiver, time::{Duration, Instant}};
use windows::{core::{BOOL, BSTR, Interface, implement, w}, Win32::{Foundation::*, Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::*, Dxgi::{Common::*, *}}, Media::MediaFoundation::*, System::{Com::*, Power::*}, UI::WindowsAndMessaging::*}};
use windows::Win32::{Graphics::{Gdi::*, Dwm::DwmFlush}, System::{Threading::*, SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_POWER_SAVING_STATUS as GUID_ENERGY_SAVER_STATUS}}, UI::{Shell::*, Accessibility::*, HiDpi::*}};

static INPUT_EVENT: OnceLock<usize> = OnceLock::new();
static DISPLAYS_CHANGED: AtomicBool = AtomicBool::new(false);
static HOLDS_CHANGED: AtomicBool = AtomicBool::new(true);
static POWER_CHANGED: AtomicBool = AtomicBool::new(true);
static ON_BATTERY: AtomicBool = AtomicBool::new(false);
static ENERGY_SAVER: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
static ASLEEP: AtomicBool = AtomicBool::new(false);
fn input_event() -> HANDLE { HANDLE(*INPUT_EVENT.get_or_init(|| unsafe { CreateEventW(None, false, false, None).expect("wallpaper command event").0 as usize }) as *mut _) }
pub fn wake() { unsafe { let _ = SetEvent(input_event()); } }
unsafe extern "system" fn foreground_changed(_: HWINEVENTHOOK, event: u32, window: HWND, object: i32, _: i32, _: u32, _: u32) {
    if window.is_invalid() { return; }
    if event >= EVENT_OBJECT_CREATE && (object != OBJID_WINDOW.0 || unsafe { GetAncestor(window, GA_ROOT) } != window) { return; }
    HOLDS_CHANGED.store(true, Ordering::Relaxed);
    wake();
}

struct Notifications { window: HWND, hooks: Vec<HWINEVENTHOOK>, power: Vec<HPOWERNOTIFY> }
impl Notifications {
    unsafe fn new() -> Result<Self> {
        unsafe {
            let window = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, w!("oneMKWallpaper"), w!("oneMK Wallpaper events"), WS_POPUP, 0, 0, 0, 0, None, None, None, None)?;
            let hooks = [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND, EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, EVENT_OBJECT_DESTROY].into_iter().map(|event|
                SetWinEventHook(event, event, None, Some(foreground_changed), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)).collect();
            let mut notifications = Self { window, hooks, power: Vec::new() };
            windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(window, 0)?;
            let mut power = SYSTEM_POWER_STATUS::default();
            if GetSystemPowerStatus(&mut power).is_ok() { ON_BATTERY.store(power.ACLineStatus == 0, Ordering::Relaxed); ENERGY_SAVER.store(power.SystemStatusFlag == 1, Ordering::Relaxed); }
            for guid in [&GUID_ACDC_POWER_SOURCE, &GUID_ENERGY_SAVER_STATUS] { notifications.power.push(RegisterPowerSettingNotification(HANDLE(window.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE)?); }
            Ok(notifications)
        }
    }
}
impl Drop for Notifications { fn drop(&mut self) { unsafe { for hook in &self.hooks { let _ = UnhookWinEvent(*hook); } for notification in &self.power { let _ = UnregisterPowerSettingNotification(*notification); } let _ = windows::Win32::System::RemoteDesktop::WTSUnRegisterSessionNotification(self.window); let _ = DestroyWindow(self.window); } } }

#[implement(IMFMediaEngineNotify)]
struct Notify(Arc<AtomicU32>);
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _: usize, _: u32) -> windows::core::Result<()> {
        if self.0.load(Ordering::Relaxed) != MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 { self.0.store(event, Ordering::Relaxed); }
        wake();
        Ok(())
    }
}

struct Engine { media: IMFMediaEngine, device: ID3D11Device, _manager: IMFDXGIDeviceManager, events: Arc<AtomicU32> }
unsafe fn frame_ready(media: &IMFMediaEngine) -> windows::core::Result<bool> {
    let mut timestamp = 0;
    let result = unsafe { (Interface::vtable(media).OnVideoStreamTick)(Interface::as_raw(media), &mut timestamp) };
    result.ok()?;
    Ok(one_mk_wallpaper::has_video_frame(result.0))
}
impl Engine {
    unsafe fn new(path: &Path) -> Result<Self> {
        let path = &crate::playable::path(path)?;
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
            if width > 0 && height > 0 && seconds.is_finite() && seconds > 0.0 { return Ok(Media { width, height, duration_ms: (seconds * 1000.0) as u64, frame_rate: 60 }); }
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
    let mut media = engine.metadata()?;
    media.frame_rate = frame_rate(path).unwrap_or(60.0).round() as u32;
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
        while !frame_ready(&engine.media)? {
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
            for pixel in bytes.as_chunks::<4>().0 { rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]); }
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
    if message == WM_DISPLAYCHANGE || message == WM_DPICHANGED { DISPLAYS_CHANGED.store(true, Ordering::Relaxed); }
    if message == WM_WTSSESSION_CHANGE {
        if first.0 == WTS_SESSION_LOCK as usize { LOCKED.store(true, Ordering::Relaxed); }
        if first.0 == WTS_SESSION_UNLOCK as usize { LOCKED.store(false, Ordering::Relaxed); }
    }
    if message == WM_POWERBROADCAST {
        if first.0 == PBT_APMSUSPEND as usize { ASLEEP.store(true, Ordering::Relaxed); }
        if first.0 == PBT_APMRESUMEAUTOMATIC as usize || first.0 == PBT_APMRESUMESUSPEND as usize { ASLEEP.store(false, Ordering::Relaxed); }
    }
    if message == WM_POWERBROADCAST && first.0 == PBT_POWERSETTINGCHANGE as usize && second.0 != 0 {
        let setting = unsafe { &*(second.0 as *const POWERBROADCAST_SETTING) };
        if setting.DataLength == 4 {
            let value = unsafe { setting.Data.as_ptr().cast::<u32>().read_unaligned() };
            if setting.PowerSetting == GUID_ACDC_POWER_SOURCE { ON_BATTERY.store(value != 0, Ordering::Relaxed); }
            if setting.PowerSetting == GUID_ENERGY_SAVER_STATUS { ENERGY_SAVER.store(value != 0, Ordering::Relaxed); }
            POWER_CHANGED.store(true, Ordering::Relaxed);
        }
    }
    unsafe {
        let pixels = GetWindowLongPtrW(window, GWLP_USERDATA) as *const image::RgbaImage;
        if message == WM_PAINT && !pixels.is_null() {
            let pixels = &*pixels;
            let mut paint = PAINTSTRUCT::default();
            let context = BeginPaint(window, &mut paint);
            let bitmap = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: pixels.width() as i32, biHeight: -(pixels.height() as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
            SetDIBitsToDevice(context, 0, 0, pixels.width(), pixels.height(), 0, 0, 0, pixels.height(), pixels.as_ptr().cast(), &bitmap, DIB_RGB_COLORS);
            let _ = EndPaint(window, &paint);
            return LRESULT(0);
        }
        if message == WM_ERASEBKGND && !pixels.is_null() { return LRESULT(1); }
        if message == WM_NCHITTEST { LRESULT(HTTRANSPARENT as isize) } else { DefWindowProcW(window, message, first, second) }
    }
}

struct Chain { swap: IDXGISwapChain2, latency: HANDLE }
impl Drop for Chain { fn drop(&mut self) { unsafe { let _ = CloseHandle(self.latency); } } }
struct Desktop { windows: Vec<HWND>, chains: Vec<Chain>, destinations: Vec<RECT>, sources: Vec<MFVideoNormalizedRect> }
impl Desktop {
    unsafe fn new(device: Option<&ID3D11Device>, config: &PlayerConfig) -> Result<Self> {
        unsafe {
            let progman = FindWindowW(w!("Progman"), None).context("The Windows desktop is not available")?;
            let _ = SendMessageTimeoutW(progman, 0x052c, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, None);
            let mut worker = FindWindowExW(Some(progman), None, w!("WorkerW"), None).unwrap_or_default();
            if worker.is_invalid() { EnumWindows(Some(find_worker), LPARAM(&mut worker as *mut HWND as isize))?; }
            if worker.is_invalid() && FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None).is_ok() { worker = progman; }
            ensure!(!worker.is_invalid(), "Windows did not provide a wallpaper surface");
            let class = WNDCLASSW { lpfnWndProc: Some(window_proc), lpszClassName: w!("oneMKWallpaper"), ..Default::default() };
            RegisterClassW(&class);
            let factory: Option<IDXGIFactory2> = device.map(|device| -> windows::core::Result<_> {
                let dxgi: IDXGIDevice = device.cast()?; dxgi.GetAdapter()?.GetParent()
            }).transpose()?;
            let local: Vec<_> = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).collect();
            ensure!(!local.is_empty(), "No monitors selected on this computer");
            let mut desktop = Self { windows: Vec::new(), chains: Vec::new(), destinations: Vec::new(), sources: Vec::new() };
            for monitor in local {
                let width = (monitor.rect.width * monitor.scale).round() as u32;
                let height = (monitor.rect.height * monitor.scale).round() as u32;
                let local_rect = monitor.local.unwrap_or(monitor.rect);
                let mut origin = POINT { x: local_rect.x as i32, y: local_rect.y as i32 };
                let _ = ScreenToClient(worker, &mut origin);
                let window = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW, w!("oneMKWallpaper"), w!("oneMK Wallpaper"), WS_CHILD, origin.x, origin.y, width as i32, height as i32, Some(worker), None, None, None)?;
                desktop.windows.push(window);
                let descriptor = DXGI_SWAP_CHAIN_DESC1 { Width: width, Height: height, Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 2, SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32, ..Default::default() };
                if let (Some(factory), Some(device)) = (&factory, device) {
                    let chain: IDXGISwapChain2 = factory.CreateSwapChainForHwnd(device, window, &descriptor, None, None)?.cast()?;
                    chain.SetMaximumFrameLatency(1)?;
                    let latency = chain.GetFrameLatencyWaitableObject();
                    ensure!(!latency.is_invalid(), "Windows did not provide a frame pacing handle");
                    desktop.chains.push(Chain { swap: chain, latency });
                }
                desktop.destinations.push(RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 });
                let slice = config.plan.slice(monitor).context("Empty canvas")?;
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

unsafe fn covered(desktop: &Desktop) -> bool {
    unsafe {
        if SHQueryUserNotificationState().is_ok_and(|state| state == QUNS_PRESENTATION_MODE) { return true; }
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() || desktop.windows.contains(&foreground) { return false; }
        let mut foreground_rect = RECT::default();
        if GetWindowRect(foreground, &mut foreground_rect).is_err() || IsIconic(foreground).as_bool() { return false; }
        desktop.windows.iter().all(|window| {
            let mut rect = RECT::default();
            GetWindowRect(*window, &mut rect).is_ok() && foreground_rect.left <= rect.left && foreground_rect.top <= rect.top && foreground_rect.right >= rect.right && foreground_rect.bottom >= rect.bottom
        })
    }
}
impl Drop for Desktop { fn drop(&mut self) { for window in &self.windows { unsafe { let _ = DestroyWindow(*window); } } } }

unsafe extern "system" fn collect_monitor(_: HMONITOR, _: HDC, rect: *mut RECT, data: LPARAM) -> BOOL {
    unsafe { (*(data.0 as *mut Vec<RECT>)).push(*rect); }
    true.into()
}

fn refresh_displays(config: &mut PlayerConfig) -> Result<()> {
    let mut detected = Vec::<RECT>::new();
    unsafe { ensure!(EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(&mut detected as *mut _ as isize)).as_bool(), "Windows could not read its screens"); }
    let previous: Vec<_> = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).cloned().collect();
    let Some(first) = previous.first() else { return Ok(()) };
    let local = first.local.unwrap_or(first.rect);
    let offset = (first.rect.x - local.x, first.rect.y - local.y);
    config.plan.canvas.monitors.retain(|monitor| monitor.computer != config.computer);
    for (index, rect) in detected.into_iter().enumerate() {
        let local = one_mk_wallpaper::Rect { x: f64::from(rect.left), y: f64::from(rect.top), width: f64::from(rect.right - rect.left), height: f64::from(rect.bottom - rect.top) };
        let mut monitor = previous.get(index).unwrap_or(first).clone();
        monitor.name = format!("Windows · {}", index + 1);
        monitor.local = Some(local);
        monitor.rect = one_mk_wallpaper::Rect { x: local.x + offset.0, y: local.y + offset.1, ..local };
        monitor.scale = 1.0;
        config.plan.canvas.monitors.push(monitor);
    }
    Ok(())
}

pub fn run(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    unsafe { let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
    if config.plan.picture { return run_picture(config, receiver); }
    let engine = unsafe { Engine::new(&config.path)? };
    engine.metadata()?;
    let mut desktop = unsafe { Desktop::new(Some(&engine.device), &config)? };
    let mut preparing_since = Instant::now();
    let _notifications = unsafe { Notifications::new()? };
    let mut holds = Holds::default();
    let mut playing = false;
    let mut last_sync = Instant::now();
    let mut last_message = "";
    let mut ready = false;
    let mut retiring = None;
    let frame_interval = Duration::from_secs_f64(1.0 / frame_rate(&config.path).unwrap_or(60.0).clamp(1.0, 60.0));
    let mut next_frame = Instant::now();
    let mut latency_ready = vec![false; desktop.chains.len()];
    loop {
        while let Ok(input) = receiver.try_recv() {
            match input { Input::End => return Ok(()), Input::Offset(offset) => config.clock_offset_ms = offset, Input::Pause(paused) => config.plan.paused = paused }
        }
        unsafe {
            ensure!(engine.events.load(Ordering::Relaxed) != MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32, "Windows could not keep playing this video. Try an H.264 MP4 video.");
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() { DispatchMessageW(&message); }
            if DISPLAYS_CHANGED.swap(false, Ordering::Relaxed) {
                refresh_displays(&mut config)?;
                retiring = Some(std::mem::replace(&mut desktop, Desktop::new(Some(&engine.device), &config)?));
                latency_ready = vec![false; desktop.chains.len()];
                ready = false;
                preparing_since = Instant::now();
                HOLDS_CHANGED.store(true, Ordering::Relaxed);
            }
            if POWER_CHANGED.swap(false, Ordering::Relaxed) {
                holds.set(Hold::Battery, config.plan.pause_on_battery && ON_BATTERY.load(Ordering::Relaxed));
                holds.set(Hold::LowPower, ENERGY_SAVER.load(Ordering::Relaxed));
            }
            if HOLDS_CHANGED.swap(false, Ordering::Relaxed) { holds.set(Hold::Occluded, covered(&desktop)); }
            holds.set(Hold::User, config.plan.paused);
            if holds.message() != last_message { one_mk_wallpaper::report(&config, &holds); last_message = holds.message(); }
            if playing != holds.playing() {
                playing = holds.playing();
                if playing { preparing_since = Instant::now(); engine.media.SetCurrentTime(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms) as f64 / 1000.0)?; engine.media.Play()?; }
                else { engine.media.Pause()?; }
                if playing && ready { one_mk_wallpaper::event(one_mk_wallpaper::Playback::Ready); }
            }
            ensure!(ready || !playing || preparing_since.elapsed() < Duration::from_secs(40), "Windows could not show the first video frame. Try another video.");
            if playing && last_sync.elapsed() >= Duration::from_secs(2) {
                let expected = position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms);
                match correction((engine.media.GetCurrentTime() * 1000.0) as u64, expected, config.plan.media.duration_ms) {
                    Correction::None => engine.media.SetPlaybackRate(1.0)?,
                    Correction::Rate(rate) => engine.media.SetPlaybackRate(rate)?,
                    Correction::Seek(target) => engine.media.SetCurrentTime(target as f64 / 1000.0)?,
                }
                last_sync = Instant::now();
            }
            if playing && Instant::now() >= next_frame && latency_ready.iter().all(|ready| *ready) {
                next_frame = Instant::now() + frame_interval;
                if frame_ready(&engine.media)? {
                for (index, chain) in desktop.chains.iter().enumerate() {
                    let source = &desktop.sources[index];
                    if source.left >= source.right || source.top >= source.bottom { continue; }
                    let surface: ID3D11Texture2D = chain.swap.GetBuffer(0)?;
                    engine.media.TransferVideoFrame(&surface, Some(&desktop.sources[index]), &desktop.destinations[index], Some(&MFARGB { rgbAlpha: 255, ..Default::default() }))?;
                    chain.swap.Present(0, DXGI_PRESENT(0)).ok()?;
                }
                latency_ready.fill(false);
                if !ready {
                    for window in &desktop.windows { let _ = ShowWindow(*window, SW_SHOWNOACTIVATE); }
                    let _ = DwmFlush();
                    drop(retiring.take());
                    one_mk_wallpaper::event(one_mk_wallpaper::Playback::Ready); ready = true;
                }
                }
            }
            let pending: Vec<_> = desktop.chains.iter().enumerate().filter(|(index, _)| !latency_ready[*index]).collect();
            let mut handles = vec![input_event()];
            if playing { handles.extend(pending.iter().map(|(_, chain)| chain.latency)); }
            let timeout = if playing && latency_ready.iter().all(|ready| *ready) { next_frame.saturating_duration_since(Instant::now()).as_millis().max(1) as u32 } else { INFINITE };
            let result = MsgWaitForMultipleObjectsEx(Some(&handles), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE).0;
            if result > WAIT_OBJECT_0.0 && (result as usize) < handles.len() { latency_ready[pending[result as usize - 1].0] = true; }
        }
    }
}

// The windows drop before their backing pixels; WM_PAINT's userdata remains
// valid even when DestroyWindow dispatches a final message.
struct PictureDesktop { desktop: Desktop, _pixels: Vec<image::RgbaImage> }
fn picture_desktop(config: &PlayerConfig, image: &image::RgbaImage) -> Result<PictureDesktop> {
    let desktop = unsafe { Desktop::new(None, config)? };
    let mut pixels = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer)
        .map(|monitor| crate::picture::render(image, &config.plan, monitor)).collect::<Result<Vec<_>>>()?;
    for (window, pixels) in desktop.windows.iter().zip(&mut pixels) {
        for pixel in pixels.pixels_mut() { pixel.0.swap(0, 2); }
        unsafe { SetWindowLongPtrW(*window, GWLP_USERDATA, pixels as *const image::RgbaImage as isize); }
    }
    Ok(PictureDesktop { desktop, _pixels: pixels })
}

fn run_picture(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    let image = crate::picture::player_source(&config)?;
    let mut picture = picture_desktop(&config, &image)?;
    let _notifications = unsafe { Notifications::new()? };
    let mut visible = false;
    let mut last = None;
    loop {
        while let Ok(input) = receiver.try_recv() { if matches!(input, Input::End) { return Ok(()); } }
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() { DispatchMessageW(&message); }
            if DISPLAYS_CHANGED.swap(false, Ordering::Relaxed) {
                refresh_displays(&mut config)?;
                let replacement = picture_desktop(&config, &image)?;
                picture = replacement;
                visible = false;
            }
            let show = !LOCKED.load(Ordering::Relaxed) && !ASLEEP.load(Ordering::Relaxed);
            if show != visible {
                for window in &picture.desktop.windows {
                    let _ = ShowWindow(*window, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
                    if show { let _ = InvalidateRect(Some(*window), None, false); let _ = UpdateWindow(*window); }
                }
                visible = show;
            }
            let playback = if show { one_mk_wallpaper::Playback::Picture }
                else { one_mk_wallpaper::Playback::Paused("Paused while this computer is locked or asleep".into()) };
            if last.as_ref() != Some(&playback) { one_mk_wallpaper::event(playback.clone()); last = Some(playback); }
            // GDI repaints only damaged windows. There is no swapchain,
            // media engine, frame pacing or idle polling for a still image.
            MsgWaitForMultipleObjectsEx(Some(&[input_event()]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

fn frame_rate(path: &Path) -> Result<f64> {
    let path = &crate::playable::path(path)?;
    unsafe {
        let path: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let reader = MFCreateSourceReaderFromURL(windows::core::PCWSTR(path.as_ptr()), None)?;
        let media = reader.GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, 0)?;
        let ratio = media.GetUINT64(&MF_MT_FRAME_RATE)?;
        ensure!(ratio as u32 != 0, "No frame rate found");
        Ok((ratio >> 32) as f64 / (ratio as u32) as f64)
    }
}

pub fn prepare(path: &Path, output: &Path, target: one_mk_wallpaper::ImportTarget) -> Result<()> {
    let engine = unsafe { Engine::new(path)? };
    let media = engine.metadata()?;
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 4)?;
        let attributes = attributes.context("Windows could not prepare video encoding")?;
        attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        attributes.SetUINT32(&MF_MPEG4SINK_MOOV_BEFORE_MDAT, 1)?;
        let wide = |path: &Path| path.to_string_lossy().encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let source = MFCreateSourceReaderFromURL(windows::core::PCWSTR(wide(path).as_ptr()), &attributes)?;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        source.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        source.SetStreamSelection(video, true)?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(target.width) << 32) | u64::from(target.height))?;
        source.SetCurrentMediaType(video, None, &input)?;
        let input = source.GetCurrentMediaType(video)?;
        let mut rate = input.GetUINT64(&MF_MT_FRAME_RATE)?;
        let source_rate = (rate >> 32) as f64 / (rate as u32).max(1) as f64;
        if source_rate > 60.0 { rate = (60u64 << 32) | 1; }
        let encoded = MFCreateMediaType()?;
        encoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        encoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        encoded.SetUINT32(&MF_MT_AVG_BITRATE, target.bitrate)?;
        encoded.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        encoded.SetUINT32(&MF_MT_MPEG2_PROFILE, 100)?;
        encoded.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(target.width) << 32) | u64::from(target.height))?;
        encoded.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
        encoded.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        let writer = MFCreateSinkWriterFromURL(windows::core::PCWSTR(wide(output).as_ptr()), None, &attributes)?;
        let stream = writer.AddStream(&encoded)?;
        writer.SetInputMediaType(stream, &input, None)?;
        writer.BeginWriting()?;
        let extended: IMFSinkWriterEx = writer.cast()?;
        let mut hardware = false;
        for index in 0..16 {
            let mut category = windows::core::GUID::zeroed();
            let mut transform = None;
            if extended.GetTransformForStream(stream, index, Some(&mut category), &mut transform).is_err() { break; }
            if category == MFT_CATEGORY_VIDEO_ENCODER && let Some(transform) = transform {
                hardware = transform.GetAttributes().and_then(|attributes| attributes.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute)).is_ok_and(|length| length > 0);
            }
        }
        ensure!(hardware, "Windows did not provide a hardware video encoder");
        let mut last = None;
        let mut progress = 0;
        loop {
            let mut flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
            source.ReadSample(video, 0, None, Some(&mut flags), Some(&mut timestamp), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 { break; }
            ensure!(flags & MF_SOURCE_READERF_ERROR.0 as u32 == 0, "Windows could not finish reading this video");
            if let Some(sample) = sample && last.is_none_or(|last| timestamp - last >= 10_000_000 / 60) {
                if source_rate > 60.0 { sample.SetSampleDuration(10_000_000 / 60)?; }
                writer.WriteSample(stream, &sample)?;
                last = Some(timestamp);
            }
            let percent = (timestamp.max(0) as u64 / 10_000 * 100 / media.duration_ms.max(1)).min(99);
            if percent != progress { println!("progress {percent}"); progress = percent; }
        }
        writer.Finalize()?;
        println!("progress 100");
    }
    Ok(())
}
