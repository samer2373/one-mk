use crate::Input;
use anyhow::{Context, Result, ensure};
use objc2::{DefinedClass, MainThreadOnly, AnyThread, define_class, msg_send, sel};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOcclusionState, NSWindowStyleMask, NSWorkspace};
use objc2_av_foundation::{AVAssetImageGenerator, AVLayerVideoGravityResize, AVMediaTypeVideo, AVPlayerItem, AVPlayerLayer, AVPlayerLooper, AVQueuePlayer, AVURLAsset};
use objc2_core_media::CMTime;
use objc2_foundation::{MainThreadMarker, NSArray, NSDate, NSDefaultRunLoopMode, NSDictionary, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect, NSRunLoop, NSSize, NSString, NSURL};
use one_mk_wallpaper::{Correction, Hold, Holds, Media, PlayerConfig, correction, now_ms, position};
use std::{cell::Cell, ffi::c_void, path::Path, sync::{Mutex, mpsc::Receiver}, time::{Duration, Instant}};

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
    fn CFRunLoopSourceCreate(allocator: *const c_void, order: isize, context: *const WakeContext) -> *mut c_void;
    fn CFRunLoopSourceSignal(source: *const c_void);
    fn CFRunLoopWakeUp(run_loop: *const c_void);
    static kCFRunLoopDefaultMode: *const c_void;
}

#[repr(C)]
struct WakeContext { version: isize, info: *mut c_void, callbacks: [usize; 7], perform: unsafe extern "C" fn(*mut c_void) }
static INPUT_SOURCE: Mutex<usize> = Mutex::new(0);
unsafe extern "C" fn input_ready(_: *mut c_void) {}
pub fn wake() {
    let source = INPUT_SOURCE.lock().unwrap();
    if *source != 0 { unsafe { CFRunLoopSourceSignal(*source as *const c_void); CFRunLoopWakeUp(CFRunLoopGetMain()); } }
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
    fn drop(&mut self) {
        let mut input = INPUT_SOURCE.lock().unwrap();
        if *input == self.0 as usize { *input = 0; }
        if !self.0.is_null() { unsafe { CFRunLoopRemoveSource(CFRunLoopGetMain(), self.0, kCFRunLoopDefaultMode); CFRelease(self.0); } }
    }
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
        Ok(Media { width, height, duration_ms, frame_rate: track.nominalFrameRate().ceil() as u32 })
    }
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" { static kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: *const NSString; }
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" { static kCVPixelBufferPixelFormatTypeKey: *const NSString; }
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" { fn CMSampleBufferGetPresentationTimeStamp(sample: *const c_void) -> CMTime; }

#[allow(deprecated)]
pub fn prepare(path: &Path, output: &Path, target: one_mk_wallpaper::ImportTarget) -> Result<()> {
    use objc2::{class, runtime::AnyObject};
    use objc2_foundation::{NSError, NSNumber};
    use objc2_av_foundation::*;
    unsafe {
        let asset = asset(path);
        let track = asset.tracksWithMediaType(AVMediaTypeVideo.context("Video decoding is unavailable")?).objectAtIndex(0);
        let mut error: *mut NSError = std::ptr::null_mut();
        let allocated: objc2::rc::Allocated<AnyObject> = msg_send![class!(AVAssetReader), alloc];
        let reader: Option<Retained<AnyObject>> = msg_send![allocated, initWithAsset: &*asset, error: &mut error];
        let reader = reader.context("macOS could not open this video for preparation")?;
        let pixel = NSNumber::new_u32(0x34323076);
        let pixels = NSDictionary::from_slices(&[&*kCVPixelBufferPixelFormatTypeKey], &[&*pixel as &AnyObject]);
        let allocated: objc2::rc::Allocated<AnyObject> = msg_send![class!(AVAssetReaderTrackOutput), alloc];
        let reader_output: Retained<AnyObject> = msg_send![allocated, initWithTrack: &*track, outputSettings: &*pixels];
        let _: () = msg_send![&*reader_output, setAlwaysCopiesSampleData: false];
        let _: () = msg_send![&*reader, addOutput: &*reader_output];
        let url = NSURL::fileURLWithPath(&NSString::from_str(&output.to_string_lossy()));
        let allocated: objc2::rc::Allocated<AnyObject> = msg_send![class!(AVAssetWriter), alloc];
        let writer: Option<Retained<AnyObject>> = msg_send![allocated, initWithURL: &*url, fileType: AVFileTypeMPEG4.context("MP4 is unavailable")?, error: &mut error];
        let writer = writer.context("macOS could not create the prepared video")?;
        let hardware = NSNumber::new_bool(true);
        let specification = NSDictionary::from_slices(&[&*kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder], &[&*hardware as &AnyObject]);
        let bitrate = NSNumber::new_u32(target.bitrate);
        let compression = NSDictionary::from_slices(&[AVVideoAverageBitRateKey.context("Video encoding is unavailable")?, AVVideoProfileLevelKey.context("Video encoding is unavailable")?], &[&*bitrate as &AnyObject, AVVideoProfileLevelH264HighAutoLevel.context("H.264 encoding is unavailable")? as &AnyObject]);
        let mut transform = track.preferredTransform();
        let rotated = transform.a.abs() < 0.5;
        let width = NSNumber::new_u32(if rotated { target.height } else { target.width });
        let height = NSNumber::new_u32(if rotated { target.width } else { target.height });
        let scale = if rotated { f64::from(target.height) } else { f64::from(target.width) } / track.naturalSize().width;
        transform.tx *= scale;
        transform.ty *= scale;
        let settings = NSDictionary::from_slices(&[AVVideoCodecKey.context("Video encoding is unavailable")?, AVVideoWidthKey.unwrap(), AVVideoHeightKey.unwrap(), AVVideoCompressionPropertiesKey.unwrap(), AVVideoEncoderSpecificationKey.unwrap()], &[AVVideoCodecTypeH264.context("H.264 encoding is unavailable")? as &AnyObject, &*width as &AnyObject, &*height as &AnyObject, &*compression as &AnyObject, &*specification as &AnyObject]);
        let writer_input: Retained<AnyObject> = msg_send![class!(AVAssetWriterInput), assetWriterInputWithMediaType: AVMediaTypeVideo.unwrap(), outputSettings: &*settings];
        let _: () = msg_send![&*writer_input, setTransform: transform];
        let _: () = msg_send![&*writer, addInput: &*writer_input];
        let _: () = msg_send![&*writer, setShouldOptimizeForNetworkUse: true];
        let started: bool = msg_send![&*writer, startWriting];
        ensure!(started, "The hardware encoder could not start");
        let _: () = msg_send![&*writer, startSessionAtSourceTime: time(0)];
        let started: bool = msg_send![&*reader, startReading];
        ensure!(started, "macOS could not read this video");
        let duration = millis(asset.duration()).max(1);
        let mut last = None;
        let mut progress = 0;
        loop {
            let sample: *const c_void = msg_send![&*reader_output, copyNextSampleBuffer];
            if sample.is_null() { break; }
            let timestamp = CMSampleBufferGetPresentationTimeStamp(sample);
            let seconds = timestamp.value as f64 / f64::from(timestamp.timescale.max(1));
            if last.is_none_or(|last| seconds - last >= 1.0 / 60.0 - 0.000_001) {
                loop {
                    let ready: bool = msg_send![&*writer_input, isReadyForMoreMediaData];
                    if ready { break; }
                    let status: isize = msg_send![&*writer, status];
                    if status != 1 { CFRelease(sample); anyhow::bail!("The hardware encoder could not prepare this video"); }
                    std::thread::sleep(Duration::from_millis(5));
                }
                let appended: bool = msg_send![&*writer_input, appendSampleBuffer: sample];
                if !appended { CFRelease(sample); anyhow::bail!("The hardware encoder could not prepare this video"); }
                last = Some(seconds);
            }
            CFRelease(sample);
            let percent = (millis(timestamp).saturating_mul(100) / duration).min(99);
            if percent != progress { println!("progress {percent}"); progress = percent; }
        }
        let status: isize = msg_send![&*reader, status];
        ensure!(status == 2, "macOS could not finish reading this video");
        let _: () = msg_send![&*writer_input, markAsFinished];
        let (sender, receiver) = std::sync::mpsc::channel();
        let completed = block2::RcBlock::new(move || { let _ = sender.send(()); });
        let _: () = msg_send![&*writer, finishWritingWithCompletionHandler: &*completed];
        receiver.recv_timeout(Duration::from_secs(120)).context("macOS took too long to finish preparing this video")?;
        let status: isize = msg_send![&*writer, status];
        ensure!(status == 2, "The hardware encoder could not finish this video");
        println!("progress 100");
        Ok(())
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

struct Output { window: Retained<NSWindow>, layer: Retained<AVPlayerLayer> }
impl Drop for Output { fn drop(&mut self) { self.window.orderOut(None); unsafe { self.layer.setPlayer(None); } } }

fn refresh_displays(config: &mut PlayerConfig, mtm: MainThreadMarker) {
    let previous: Vec<_> = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).cloned().collect();
    let Some(first) = previous.first() else { return };
    let local = first.local.unwrap_or(first.rect);
    let offset = (first.rect.x - local.x, first.rect.y - local.y);
    let screens = NSScreen::screens(mtm);
    if screens.is_empty() { return; }
    let primary_top = screens.objectAtIndex(0).frame().size.height;
    config.plan.canvas.monitors.retain(|monitor| monitor.computer != config.computer);
    for (index, screen) in screens.iter().enumerate() {
        let frame = screen.frame();
        let local = one_mk_wallpaper::Rect { x: frame.origin.x, y: primary_top - frame.origin.y - frame.size.height, width: frame.size.width, height: frame.size.height };
        let mut monitor = previous.get(index).unwrap_or(first).clone();
        monitor.name = format!("Mac · {}", index + 1);
        monitor.local = Some(local);
        monitor.rect = one_mk_wallpaper::Rect { x: local.x + offset.0, y: local.y + offset.1, ..local };
        monitor.scale = screen.backingScaleFactor();
        config.plan.canvas.monitors.push(monitor);
    }
}

fn outputs(player: &AVQueuePlayer, config: &PlayerConfig, mtm: MainThreadMarker) -> Result<Vec<Output>> {
    let screens = NSScreen::screens(mtm);
    ensure!(!screens.is_empty(), "No desktop screens are available");
    let primary_top = screens.objectAtIndex(0).frame().size.height;
    let image = config.plan.canvas.image_rect(&config.plan.media, config.plan.fit).context("Empty canvas")?;
    let mut outputs = Vec::new();
    for screen in screens {
        let frame = screen.frame();
        let Some(monitor) = config.plan.canvas.monitors.iter().filter(|monitor| monitor.computer == config.computer).find(|monitor| {
            let local = monitor.local.unwrap_or(monitor.rect);
            (local.x - frame.origin.x).abs() < 2.0 && (local.y - (primary_top - frame.origin.y - frame.size.height)).abs() < 2.0
        }) else { continue };
        let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel, NSBackingStoreType::Buffered, false) };
        unsafe { window.setReleasedWhenClosed(false); }
        window.setIgnoresMouseEvents(true);
        window.setLevel(unsafe { CGWindowLevelForKey(18) - 1 } as isize);
        window.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle);
        let view = NSView::new(mtm);
        view.setWantsLayer(true);
        let root = view.layer().context("The desktop video layer could not be created")?;
        root.setMasksToBounds(true);
        let layer = unsafe { AVPlayerLayer::playerLayerWithPlayer(Some(player)) };
        unsafe { layer.setVideoGravity(AVLayerVideoGravityResize.context("Video layers are unavailable")?); }
        layer.setContentsScale(screen.backingScaleFactor());
        let scale_x = frame.size.width / monitor.rect.width;
        let scale_y = frame.size.height / monitor.rect.height;
        layer.setFrame(NSRect::new(NSPoint::new((image.x - monitor.rect.x) * scale_x, frame.size.height - (image.y - monitor.rect.y + image.height) * scale_y), NSSize::new(image.width * scale_x, image.height * scale_y)));
        root.addSublayer(&layer);
        window.setContentView(Some(&view));
        window.orderBack(None);
        outputs.push(Output { window, layer });
    }
    Ok(outputs)
}

pub fn run(mut config: PlayerConfig, receiver: Receiver<Input>) -> Result<()> {
    let mtm = MainThreadMarker::new().context("The wallpaper must start on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let context = WakeContext { version: 0, info: std::ptr::null_mut(), callbacks: [0; 7], perform: input_ready };
    let input_source = PowerSource(unsafe { CFRunLoopSourceCreate(std::ptr::null(), 0, &context) });
    ensure!(!input_source.0.is_null(), "The wallpaper could not listen for changes");
    unsafe { CFRunLoopAddSource(CFRunLoopGetMain(), input_source.0, kCFRunLoopDefaultMode); }
    *INPUT_SOURCE.lock().unwrap() = input_source.0 as usize;
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
    let mut outputs = outputs(&player, &config, mtm)?;
    let mut retiring = Vec::new();
    app.finishLaunching();
    let mut holds = Holds::default();
    let mut playing = false;
    let mut last_sync = Instant::now() - Duration::from_secs(5);
    let mut last_message = "";
    let mut ready = false;
    let mut preparing_since = Instant::now();
    loop {
        while let Ok(input) = receiver.try_recv() {
            match input { Input::End => {
                unsafe { player.pause(); looper.disableLooping(); center.removeObserver(&observer); workspace.removeObserver(&observer); distributed.removeObserver(&observer); }
                return Ok(());
            }, Input::Offset(offset) => config.clock_offset_ms = offset }
        }
        let flags = observer.ivars().get();
        if flags & 4 != 0 {
            refresh_displays(&mut config, mtm);
            retiring.extend(std::mem::replace(&mut outputs, self::outputs(&player, &config, mtm)?));
            ready = false;
            preparing_since = Instant::now();
        }
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
        holds.set(Hold::Occluded, outputs.is_empty() || ready && outputs.iter().all(|output| !output.window.occlusionState().contains(NSWindowOcclusionState::Visible)));
        if holds.message() != last_message { one_mk_wallpaper::report(&config, &holds); last_message = holds.message(); }
        if playing != holds.playing() {
            playing = holds.playing();
            unsafe {
                if playing { preparing_since = Instant::now(); player.seekToTime(time(position(now_ms(), config.clock_offset_ms, config.plan.start_ms, config.plan.media.duration_ms))); player.setRate(1.0); if ready { one_mk_wallpaper::event(one_mk_wallpaper::Playback::Ready); } }
                else { player.pause(); }
            }
        }
        ensure!(unsafe { player.error().is_none() && item.error().is_none() }, "macOS could not keep playing this video. Try an H.264 MP4 video.");
        ensure!(ready || !playing || preparing_since.elapsed() < Duration::from_secs(40), "macOS could not show the first video frame. Try another video.");
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
        if !ready && !outputs.is_empty() && outputs.iter().all(|output| unsafe { output.layer.isReadyForDisplay() }) {
            for output in &outputs { output.window.orderFrontRegardless(); }
            retiring.clear();
            if playing { one_mk_wallpaper::event(one_mk_wallpaper::Playback::Ready); }
            ready = true;
        }
        NSRunLoop::mainRunLoop().runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &NSDate::dateWithTimeIntervalSinceNow(if playing { if ready { 2.0 } else { 0.02 } } else { 1.0e10 }));
    }
}
