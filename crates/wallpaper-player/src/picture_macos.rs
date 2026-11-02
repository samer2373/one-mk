use crate::Monitor;
use anyhow::{Context, Result, ensure};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSColor, NSColorSpace, NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageFillColorKey, NSWorkspaceDesktopImageScalingKey};
use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSError, NSNumber, NSRect, NSString, NSURL};
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct Previous { screen: String, url: String, scaling: Option<isize>, clipping: Option<bool>, color: Option<[f64; 4]>, #[serde(default)] applied: Option<String>, #[serde(default)] previous_applied: Option<String> }

impl Previous {
    fn owns(&self, url: &str) -> bool { self.applied.as_deref() == Some(url) || self.previous_applied.as_deref() == Some(url) }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" { fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const std::ffi::c_void; }
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFUUIDCreateString(allocator: *const std::ffi::c_void, uuid: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn CFStringGetCString(value: *const std::ffi::c_void, buffer: *mut i8, size: isize, encoding: u32) -> bool;
    fn CFRelease(value: *const std::ffi::c_void);
}

fn on_main<Value: Send>(work: impl FnOnce() -> Result<Value> + Send) -> Result<Value> {
    if MainThreadMarker::new().is_some() { return work(); }
    let mut result = None;
    dispatch2::DispatchQueue::main().exec_sync(|| result = Some(work()));
    result.context("The main queue did not complete the wallpaper operation")?
}

pub fn decode(path: &Path) -> Result<image::RgbaImage> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    unsafe {
        let data = objc2_foundation::NSData::dataWithContentsOfFile(&NSString::from_str(&path.to_string_lossy())).context("macOS could not open this picture")?;
        let picture = NSBitmapImageRep::imageRepWithData(&data).context("macOS could not read this picture")?;
        ensure!(picture.pixelsWide() > 0 && picture.pixelsHigh() > 0 && picture.pixelsWide() <= 16384 && picture.pixelsHigh() <= 16384
            && picture.pixelsWide() * picture.pixelsHigh() <= 64 * 1024 * 1024, "This picture is too large");
        let png = picture.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()).context("macOS could not read this picture")?;
        Ok(image::load_from_memory_with_format(&png.to_vec(), image::ImageFormat::Png)?.into_rgba8())
    }
}

unsafe fn screens() -> Retained<NSArray<AnyObject>> { unsafe { msg_send![class!(NSScreen), screens] } }
unsafe fn workspace() -> Retained<AnyObject> { unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] } }
unsafe fn identity(screen: &AnyObject) -> Result<String> {
    unsafe {
        let description: Retained<NSDictionary<NSString, AnyObject>> = msg_send![screen, deviceDescription];
        let number = description.objectForKey(&NSString::from_str("NSScreenNumber")).context("The screen has no display identity")?;
        let number: u32 = msg_send![&*number, unsignedIntValue];
        let uuid = CGDisplayCreateUUIDFromDisplayID(number);
        ensure!(!uuid.is_null(), "The screen has no stable display identity");
        let text = CFUUIDCreateString(std::ptr::null(), uuid);
        CFRelease(uuid);
        ensure!(!text.is_null(), "The screen identity could not be read");
        let mut buffer = [0i8; 128];
        let success = CFStringGetCString(text, buffer.as_mut_ptr(), buffer.len() as isize, 0x08000100);
        CFRelease(text);
        ensure!(success, "The screen identity could not be read");
        Ok(std::ffi::CStr::from_ptr(buffer.as_ptr()).to_str()?.to_owned())
    }
}

unsafe fn set(workspace: &AnyObject, screen: &AnyObject, url: &NSURL, previous: Option<&Previous>) -> Result<()> {
    unsafe {
        let scaling = NSNumber::new_isize(previous.and_then(|saved| saved.scaling).unwrap_or(1));
        let clipping = NSNumber::new_bool(previous.and_then(|saved| saved.clipping).unwrap_or(false));
        let color = previous.and_then(|saved| saved.color).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(color[0], color[1], color[2], color[3]);
        let options = NSDictionary::from_slices(&[NSWorkspaceDesktopImageScalingKey, NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageFillColorKey], &[&*scaling as &AnyObject, &*clipping as &AnyObject, &*color as &AnyObject]);
        let mut error: *mut NSError = std::ptr::null_mut();
        let success: bool = msg_send![workspace, setDesktopImageURL: url, forScreen: screen, options: &*options, error: &mut error];
        ensure!(success, "macOS could not change the picture wallpaper");
        Ok(())
    }
}

pub fn apply(files: &[(Monitor, PathBuf)], root: &Path) -> Result<()> {
    {
        let home = std::env::var_os("HOME").context("The Mac wallpaper settings could not be inspected safely")?;
        let store = PathBuf::from(home).join("Library/Application Support/com.apple.wallpaper/Store/Index.plist");
        ensure!(store.exists(), "macOS has not exposed wallpaper settings that oneMK can safely round-trip. Native pictures were not changed.");
        {
            let output = std::process::Command::new("/usr/bin/plutil").args(["-convert", "xml1", "-o", "-"]).arg(store).output()?;
            let text = String::from_utf8_lossy(&output.stdout);
            ensure!(output.status.success() && super::restorable_macos_store(&text),
                "Native pictures cannot safely replace this Mac's dynamic, aerial, per-Space, or opaque wallpaper settings. Native pictures were not changed.");
        }
    }
    let snapshot = root.join("previous.json");
    let mut previous: Vec<Previous> = if snapshot.exists() { serde_json::from_slice(&std::fs::read(&snapshot)?)? } else { Vec::new() };
    let assignments = on_main(|| unsafe {
        let workspace = workspace();
        let screens = screens();
        ensure!(!screens.is_empty(), "No desktop screens are available");
        {
            let mut observed = Vec::new();
            for screen in &screens {
                let identity = identity(&screen)?;
                let url: Option<Retained<NSURL>> = msg_send![&*workspace, desktopImageURLForScreen: &*screen];
                let native_url = url.context("macOS did not expose a restorable wallpaper setting")?;
                if let Some(url) = native_url.absoluteString() {
                    if let Some(saved) = previous.iter().find(|saved| saved.screen == identity) {
                        ensure!(saved.owns(&url.to_string()) || saved.url == url.to_string(), "Return to the Space where oneMK set the picture before changing it again");
                        continue;
                    }
                    ensure!(native_url.path().is_some_and(|path| path.to_string().rsplit('.').next().is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp"))), "macOS has not exposed a static wallpaper that can be restored safely");
                    let options: Option<Retained<NSDictionary<NSString, AnyObject>>> = msg_send![&*workspace, desktopImageOptionsForScreen: &*screen];
                    let scaling = options.as_ref().and_then(|options| options.objectForKey(NSWorkspaceDesktopImageScalingKey)).map(|number| msg_send![&*number, integerValue]);
                    let clipping = options.as_ref().and_then(|options| options.objectForKey(NSWorkspaceDesktopImageAllowClippingKey)).map(|number| msg_send![&*number, boolValue]);
                    let color = options.as_ref().and_then(|options| options.objectForKey(NSWorkspaceDesktopImageFillColorKey)).and_then(|color| {
                        let color: Option<Retained<NSColor>> = msg_send![&*color, colorUsingColorSpace: &*NSColorSpace::sRGBColorSpace()];
                        color.map(|color| [color.redComponent(), color.greenComponent(), color.blueComponent(), color.alphaComponent()])
                    });
                    observed.push(Previous { screen: identity, url: url.to_string(), scaling, clipping, color, applied: None, previous_applied: None });
                }
            }
            super::merge_snapshot(&mut previous, observed, |saved| &saved.screen);
        }
        let primary: NSRect = msg_send![&*screens.objectAtIndex(0), frame];
        let mut assignments = Vec::new();
        for (monitor, path) in files {
            let local = monitor.local.unwrap_or(monitor.rect);
            let screen = screens.iter().find(|screen| {
                let frame: NSRect = msg_send![&**screen, frame];
                (frame.origin.x - local.x).abs() < 2.0 && (primary.size.height - frame.origin.y - frame.size.height - local.y).abs() < 2.0
            }).context("The picture's screen is no longer connected")?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            let identity = identity(&screen)?;
            if let Some(saved) = previous.iter_mut().find(|saved| saved.screen == identity) {
                let current: Option<Retained<NSURL>> = msg_send![&*workspace, desktopImageURLForScreen: &*screen];
                let current = current.and_then(|url| url.absoluteString()).map(|url| url.to_string());
                saved.previous_applied = current.filter(|url| saved.owns(url));
                saved.applied = url.absoluteString().map(|url| url.to_string());
            }
            assignments.push((identity, path.clone()));
        }
        Ok(assignments)
    })?;
    super::atomic_write(&snapshot, &serde_json::to_vec(&previous)?)?;
    on_main(|| unsafe {
        let workspace = workspace();
        let screens = screens();
        for (display, path) in assignments {
            let screen = screens.iter().find(|screen| identity(screen).is_ok_and(|identity| identity == display)).context("The picture's screen is no longer connected")?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            set(&workspace, &screen, &url, None)?;
        }
        Ok(())
    })
}

pub fn restore(root: &Path) -> Result<bool> {
    let previous: Vec<Previous> = serde_json::from_slice(&std::fs::read(root.join("previous.json"))?)?;
    let pending = on_main(|| unsafe {
        let workspace = workspace();
        let screens = screens();
        let pending = super::unrestored(previous, |saved| {
            let Some(screen) = screens.iter().find(|screen| identity(screen).is_ok_and(|identity| identity == saved.screen)) else { return false };
            let current: Option<Retained<NSURL>> = msg_send![&*workspace, desktopImageURLForScreen: &*screen];
            let current = current.and_then(|url| url.absoluteString()).map(|url| url.to_string());
            if current.as_deref() == Some(&saved.url) { return true; }
            if !current.as_deref().is_some_and(|url| saved.owns(url)) { return false; }
            NSURL::URLWithString(&NSString::from_str(&saved.url)).is_some_and(|url| set(&workspace, &screen, &url, Some(saved)).is_ok())
        });
        Ok(pending)
    })?;
    super::atomic_write(&root.join("previous.json"), &serde_json::to_vec(&pending)?)?;
    Ok(pending.is_empty())
}
