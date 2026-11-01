use crate::Input;
use anyhow::{Context, Result, ensure};
use objc2::{DefinedClass, MainThreadOnly, AnyThread, define_class, msg_send, sel};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOcclusionState, NSWindowStyleMask, NSWorkspace};
use objc2_av_foundation::{AVAssetImageGenerator, AVLayerVideoGravityResize, AVMediaTypeVideo, AVPlayerItem, AVPlayerLayer, AVPlayerLooper, AVQueuePlayer, AVURLAsset};
use objc2_core_media::CMTime;
use objc2_foundation::{MainThreadMarker, NSArray, NSDate, NSDefaultRunLoopMode, NSDictionary, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect, NSRunLoop, NSSize, NSString, NSURL};
use one_mk_wallpaper::{Correction, Hold, Holds, Media, PlayerConfig, correction, now_ms, position};
use std::{cell::Cell, ffi::c_void, path::Path, sync::mpsc::Receiver, time::{Duration, Instant}};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" { fn CGWindowLevelForKey(key: i32) -> i32; }
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> *const c_void;
    fn IOPSGetProvidingPowerSourceType(info: *const c_void) -> *const c_void;
    fn IOPSNotificationCreateRunLoopSource(callback: unsafe extern "C" fn(*mut c_void), context: *mut c_void) -> *const c_void;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFEqual(first: *const c_void, second: *const c_void) -> bool;
    fn CFRunLoopGetMain() -> *const c_void;
    fn CFRunLoopAddSource(run_loop: *const c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *const c_void, source: *const c_void, mode: *const c_void);
    static kCFRunLoopDefaultMode: *const c_void;
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<u32>]
    struct Observer;
    unsafe impl NSObjectProtocol for Observer {}
    impl Observer {
        #[unsafe(method(changed:))]
        fn changed(&self, note: &NSNotification) {
            let name = note.name().to_string();
            let mut flags = self.ivars().get();
            match name.as_str() {
                "com.apple.screenIsLocked" => flags |= 1,
                "com.apple.screenIsUnlocked" => flags &= !1,
                "NSWorkspaceScreensDidSleepNotification" | "NSWorkspaceWillSleepNotification" => flags |= 2,
                "NSWorkspaceScreensDidWakeNotification" | "NSWorkspaceDidWakeNotification" => flags &= !2,
                "NSApplicationDidChangeScreenParametersNotification" => flags |= 4,
                _ => flags |= 8,
            }
            self.ivars().set(flags);
        }
    }
);

unsafe extern "C" fn power_changed(context: *mut c_void) {
    let observer = unsafe { &*context.cast::<Observer>() };
    observer.ivars().set(observer.ivars().get() | 8);
}

struct PowerSource(*const c_void);
impl Drop for PowerSource {
    fn drop(&mut self) { if !self.0.is_null() { unsafe { CFRunLoopRemoveSource(CFRunLoopGetMain(), self.0, kCFRunLoopDefaultMode); CFRelease(self.0); } } }
}

fn time(milliseconds: u64) -> CMTime { CMTime { value: milliseconds.min(i64::MAX as u64) as i64, timescale: 1000, flags: objc2_core_media::CMTimeFlags(1), epoch: 0 } }
fn millis(value: CMTime) -> u64 { if value.timescale <= 0 { 0 } else { (value.value.max(0) as u64).saturating_mul(1000) / value.timescale as u64 } }
fn asset(path: &Path) -> Retained<AVURLAsset> {
    unsafe { AVURLAsset::URLAssetWithURL_options(&NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())), None) }
}

#[allow(deprecated)]
pub fn probe(path: &Path, poster: Option<&Path>) -> Result<Media> {
    let asset = asset(path);
    unsafe {
        let duration_ms = millis(asset.duration());
        let tracks = asset.tracksWithMediaType(AVMediaTypeVideo.context("Video decoding is unavailable")?);
        ensure!(!tracks.is_empty() && duration_ms > 0, "This file has no playable video track");
        let track = tracks.objectAtIndex(0);
        let size = track.naturalSize();
        let transform = track.preferredTransform();
        let width = (size.width * transform.a + size.height * transform.c).abs().round() as u32;
        let height = (size.width * transform.b + size.height * transform.d).abs().round() as u32;
        if let Some(poster) = poster {
            let generator = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset);
            generator.setAppliesPreferredTrackTransform(true);
            generator.setMaximumSize(NSSize::new(1600.0, 900.0));
            let image = generator.copyCGImageAtTime_actualTime_error(time(0), std::ptr::null_mut()).map_err(|error| anyhow::anyhow!("{error}"))?;
            let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
            let data = bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()).context("Could not make a video preview")?;
            std::fs::write(poster, data.to_vec())?;
        }
        Ok(Media { width, height, duration_ms })
    }
}

fn battery() -> bool {
    unsafe {
        let info = IOPSCopyPowerSourcesInfo();
        if info.is_null() { return false; }
        let source = IOPSGetProvidingPowerSourceType(info);
        let ac_power = NSString::from_str("AC Power");
        let on_battery = !source.is_null() && !CFEqual(source, (&*ac_power as *const NSString).cast());
        CFRelease(info);
        on_battery
    }
}

fn geometry(window: &NSWindow, layer: &AVPlayerLayer, config: &PlayerConfig, mtm: MainThreadMarker) -> Result<()> {
    let screens = NSScreen::screens(mtm);
    ensure!(!screens.is_empty(), "No desktop screens are available");
    let mut frame = screens.objectAtIndex(0).frame();
    for screen in screens.iter().skip(1) {
        let next = screen.frame();
        let right = (frame.origin.x + frame.size.width).max(next.origin.x + next.size.width);
        let top = (frame.origin.y + frame.size.height).max(next.origin.y + next.size.height);
        frame.origin.x = frame.origin.x.min(next.origin.x);
        frame.origin.y = frame.origin.y.min(next.origin.y);
        frame.size = NSSize::new(right - frame.origin.x, top - frame.origin.y);
    }
    let local = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).map(|monitor| monitor.rect).reduce(one_mk_wallpaper::Rect::union).context("No monitors selected on this computer")?;
    let image = config.plan.canvas.image_rect(&config.plan.media, config.plan.fit).context("Empty canvas")?;
    window.setFrame_display(frame, true);
    layer.setFrame(NSRect::new(NSPoint::new(image.x - local.x, frame.size.height - (image.y - local.y) - image.height), NSSize::new(image.width, image.height)));
    Ok(())
}

pub fn run(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    let mtm = MainThreadMarker::new().context("The wallpaper must start on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let observer: Retained<Observer> = unsafe { msg_send![super(Observer::alloc(mtm).set_ivars(Cell::new(0))), init] };
    observer.ivars().set(8);
    let _power_source = PowerSource(unsafe { IOPSNotificationCreateRunLoopSource(power_changed, (&*observer as *const Observer).cast_mut().cast()) });
    if !_power_source.0.is_null() { unsafe { CFRunLoopAddSource(CFRunLoopGetMain(), _power_source.0, kCFRunLoopDefaultMode); } }
    let center = NSNotificationCenter::defaultCenter();
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let distributed: Retained<NSNotificationCenter> = unsafe { msg_send![objc2::class!(NSDistributedNotificationCenter), defaultCenter] };
    for (center, names) in [(&*center, &["NSWindowDidChangeOcclusionStateNotification", "NSApplicationDidChangeScreenParametersNotification", "NSProcessInfoPowerStateDidChangeNotification"][..]), (&*workspace, &["NSWorkspaceScreensDidSleepNotification", "NSWorkspaceScreensDidWakeNotification", "NSWorkspaceWillSleepNotification", "NSWorkspaceDidWakeNotification"][..]), (&*distributed, &["com.apple.screenIsLocked", "com.apple.screenIsUnlocked"][..])] {
        for name in names { unsafe { center.addObserver_selector_name_object(&observer, sel!(changed:), Some(&NSString::from_str(name)), None); } }
    }
    let asset = asset(&config.path);
    let item = unsafe { AVPlayerItem::playerItemWithAsset(&asset, mtm) };
    unsafe { item.setPreferredForwardBufferDuration(2.0); }
    let player = unsafe { AVQueuePlayer::queuePlayerWithItems(&NSArray::new(), mtm) };
    let looper = unsafe { AVPlayerLooper::playerLooperWithPlayer_templateItem(&player, &item) };
    unsafe {
        player.setMuted(true);
        player.setPreventsDisplaySleepDuringVideoPlayback(false);
        player.setAutomaticallyWaitsToMinimizeStalling(true);
    }
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)), NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false); }
    window.setIgnoresMouseEvents(true);
    window.setLevel(unsafe { CGWindowLevelForKey(18) - 1 } as isize);
    window.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle);
    let view = NSView::new(mtm);
    view.setWantsLayer(true);
    let root = view.layer().context("The desktop video layer could not be created")?;
    let layer = unsafe { AVPlayerLayer::playerLayerWithPlayer(Some(&player)) };
    unsafe { layer.setVideoGravity(AVLayerVideoGravityResize.context("Video layers are unavailable")?); }
    root.setMasksToBounds(true);
    root.addSublayer(&layer);
    window.setContentView(Some(&view));
    geometry(&window, &layer, &config, mtm)?;
    window.orderBack(None);
    app.finishLaunching();
    let mut holds = Holds::default();
    let mut playing = false;
    let mut last_sync = Instant::now() - Duration::from_secs(5);
    let mut last_message = "";
    loop {
        while let Ok(input) = receiver.try_recv() {
            match input { Input::End => {
                unsafe { player.pause(); looper.disableLooping(); center.removeObserver(&observer); workspace.removeObserver(&observer); distributed.removeObserver(&observer); }
                window.orderOut(None);
                return Ok(());
            }, Input::Offset(offset) => config.clock_offset_ms = offset }
        }
        let flags = observer.ivars().get();
        if flags & 4 != 0 { geometry(&window, &layer, &config, mtm)?; }
        if flags & 8 != 0 {
            holds.set(Hold::Battery, config.plan.pause_on_battery && battery());
            let process = NSProcessInfo::processInfo();
            let low_power: bool = unsafe { process.respondsToSelector(sel!(isLowPowerModeEnabled)) && msg_send![&*process, isLowPowerModeEnabled] };
            holds.set(Hold::LowPower, low_power);
        }
        observer.ivars().set(flags & !12);
        holds.set(Hold::Locked, flags & 1 != 0);
        holds.set(Hold::Asleep, flags & 2 != 0);
        holds.set(Hold::User, config.plan.paused);
        holds.set(Hold::Occluded, !window.occlusionState().contains(NSWindowOcclusionState::Visible));
        if holds.message() != last_message { one_mk_wallpaper::report(&config, &holds); last_message = holds.message(); }
        if playing != holds.playing() {
            playing = holds.playing();
            unsafe {
                if playing { player.seekToTime(time(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms))); player.setRate(1.0); }
                else { player.pause(); }
            }
        }
        if playing && last_sync.elapsed() >= Duration::from_secs(2) {
            let expected = position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms);
            unsafe {
                match correction(millis(player.currentTime()), expected, config.plan.media.duration_ms) {
                    Correction::None => player.setRate(1.0),
                    Correction::Rate(rate) => player.setRate(rate as f32),
                    Correction::Seek(target) => player.seekToTime(time(target)),
                }
            }
            last_sync = Instant::now();
        }
        NSRunLoop::mainRunLoop().runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &NSDate::dateWithTimeIntervalSinceNow(0.25));
    }
}
