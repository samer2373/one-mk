use crate::Monitor;
use anyhow::{Context, Result, ensure};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSColor, NSColorSpace, NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageFillColorKey, NSWorkspaceDesktopImageScalingKey};
use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSError, NSNumber, NSRect, NSString, NSURL};
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct Previous { screen: String, url: String, scaling: Option<isize>, clipping: Option<bool>, color: Option<[f64; 4]> }

fn on_main(work: impl FnOnce() -> Result<()> + Send) -> Result<()> {
    if MainThreadMarker::new().is_some() { return work(); }
    let mut result = Ok(());
    dispatch2::DispatchQueue::main().exec_sync(|| result = work());
    result
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
unsafe fn identity(screen: &AnyObject) -> String {
    unsafe {
        let description: Retained<NSDictionary<NSString, AnyObject>> = msg_send![screen, deviceDescription];
        description.objectForKey(&NSString::from_str("NSScreenNumber")).map_or_else(String::new, |number| {
            let description: Retained<NSString> = msg_send![&*number, description];
            description.to_string()
        })
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
    on_main(|| unsafe {
        let workspace = workspace();
        let screens = screens();
        ensure!(!screens.is_empty(), "No desktop screens are available");
        let snapshot = root.join("previous.json");
        if !snapshot.exists() {
            let mut previous = Vec::new();
            for screen in &screens {
                let url: Option<Retained<NSURL>> = msg_send![&*workspace, desktopImageURLForScreen: &*screen];
                if let Some(url) = url.and_then(|url| url.absoluteString()) {
                    let options: Option<Retained<NSDictionary<NSString, AnyObject>>> = msg_send![&*workspace, desktopImageOptionsForScreen: &*screen];
                    let scaling = options.as_ref().and_then(|options| options.objectForKey(NSWorkspaceDesktopImageScalingKey)).map(|number| msg_send![&*number, integerValue]);
                    let clipping = options.as_ref().and_then(|options| options.objectForKey(NSWorkspaceDesktopImageAllowClippingKey)).map(|number| msg_send![&*number, boolValue]);
                    let color = options.as_ref().and_then(|options| options.objectForKey(NSWorkspaceDesktopImageFillColorKey)).and_then(|color| {
                        let color: Option<Retained<NSColor>> = msg_send![&*color, colorUsingColorSpace: &*NSColorSpace::sRGBColorSpace()];
                        color.map(|color| [color.redComponent(), color.greenComponent(), color.blueComponent(), color.alphaComponent()])
                    });
                    previous.push(Previous { screen: identity(&screen), url: url.to_string(), scaling, clipping, color });
                }
            }
            std::fs::write(&snapshot, serde_json::to_vec(&previous)?)?;
        }
        let primary: NSRect = msg_send![&*screens.objectAtIndex(0), frame];
        for (monitor, path) in files {
            let local = monitor.local.unwrap_or(monitor.rect);
            let screen = screens.iter().find(|screen| {
                let frame: NSRect = msg_send![&**screen, frame];
                (frame.origin.x - local.x).abs() < 2.0 && (primary.size.height - frame.origin.y - frame.size.height - local.y).abs() < 2.0
            }).context("The picture's screen is no longer connected")?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            set(&workspace, &screen, &url, None)?;
        }
        Ok(())
    })
}

pub fn restore(root: &Path) -> Result<()> {
    let previous: Vec<Previous> = serde_json::from_slice(&std::fs::read(root.join("previous.json"))?)?;
    on_main(|| unsafe {
        let workspace = workspace();
        for screen in screens() {
            if let Some(saved) = previous.iter().find(|saved| saved.screen == identity(&screen)) {
                let url = NSURL::URLWithString(&NSString::from_str(&saved.url)).context("The previous wallpaper could not be found")?;
                set(&workspace, &screen, &url, Some(saved))?;
            }
        }
        Ok(())
    })
}
