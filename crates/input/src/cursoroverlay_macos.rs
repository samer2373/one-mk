//! The drawn pointer on macOS (see `cursoroverlay.rs`). Compiled here, not
//! run: there is no Mac to test on.
//!
//! The probe asks IOKit whether a pointing device (HID generic-desktop
//! mouse or pointer, or a digitizer touch pad) is registered. Whether the
//! pointer shows can't be asked (`CGCursorIsVisible` is deprecated and
//! "no longer supported"), so `auto` draws when no pointing device is
//! registered. When control starts that way, `CGDisplayShowCursor` and
//! `CGAssociateMouseAndMouseCursorPosition(true)` are called first, which
//! is all macOS offers to show the pointer; as there is no way to see
//! whether that worked, ours is drawn anyway. It is the system arrow at
//! the same hot spot, so if both show they look like one.
//!
//! AppKit windows can only be made on a process's main thread, and the
//! daemon's main thread runs tokio, not an AppKit event loop. So the
//! window lives in a small helper: this same program started with
//! [`HELPER_ARG`](crate::cursoroverlay::HELPER_ARG), which runs
//! `NSApplication` (as an accessory: no Dock icon, no menu bar) with one
//! borderless window at the screen-saver level that ignores the mouse,
//! joins every Space and shows the system arrow (`NSCursor.arrowCursor`).
//! It reads `x y` lines (global display points, as the daemon uses) from
//! its stdin and exits when stdin closes, so it never outlives the daemon.
//! It runs only while the pointer is drawn.

use crate::cursoroverlay::{HELPER_ARG, Overlay, Platform, Probe};
use anyhow::{Context, Result};
use one_mk_proto::Pos;
use one_mk_proto::overlay::OsCursor;
use objc2_core_foundation::{CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{CGAssociateMouseAndMouseCursorPosition, CGDisplayShowCursor, CGMainDisplayID};
use std::ffi::{c_char, c_void};
use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::ptr::NonNull;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingServices(main_port: u32, matching: *const c_void, existing: *mut u32) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IORegistryEntryCreateCFProperty(entry: u32, key: &CFString, allocator: *const c_void, options: u32) -> *mut CFType;
    fn IOObjectRelease(object: u32) -> i32;
}

/// Whether IOKit lists a pointing device; `None` if it can't be asked.
fn pointing_device() -> Option<bool> {
    // SAFETY: IOKit registry calls; the matching dictionary is consumed by
    // `IOServiceGetMatchingServices`, every object we get is released, and
    // the property is a +1 reference handed to `CFRetained`.
    unsafe {
        let matching = IOServiceMatching(c"IOHIDDevice".as_ptr());
        if matching.is_null() {
            return None;
        }
        let mut iter = 0u32;
        if IOServiceGetMatchingServices(0, matching, &mut iter) != 0 {
            return None;
        }
        let (page_key, usage_key) = (CFString::from_static_str("PrimaryUsagePage"), CFString::from_static_str("PrimaryUsage"));
        let number = |entry: u32, key: &CFString| -> Option<i64> {
            let raw = NonNull::new(IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0))?;
            let value: CFRetained<CFType> = CFRetained::from_raw(raw);
            value.downcast_ref::<CFNumber>()?.as_i64()
        };
        let mut found = false;
        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            // Generic desktop pointer (1) or mouse (2); digitizer touch pad.
            let kind = (number(entry, &page_key), number(entry, &usage_key));
            found |= matches!(kind, (Some(1), Some(1 | 2)) | (Some(0x0D), Some(5)));
            IOObjectRelease(entry);
        }
        IOObjectRelease(iter);
        Some(found)
    }
}

pub(crate) struct Mac;

impl Platform for Mac {
    fn probe(&mut self) -> Probe {
        Probe { pointing_device: pointing_device(), os_cursor: OsCursor::Unknown }
    }

    fn reveal(&mut self) -> bool {
        let _ = CGDisplayShowCursor(CGMainDisplayID());
        let _ = CGAssociateMouseAndMouseCursorPosition(true);
        false
    }

    fn create(&mut self, at: Pos) -> Result<Box<dyn Overlay>> {
        let exe = std::env::current_exe().context("can't find this program to start the pointer helper")?;
        let mut child = Command::new(exe)
            .arg(HELPER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("can't start the pointer helper")?;
        let stdin = child.stdin.take().context("no pipe to the pointer helper")?;
        // Never wait on the helper: a move it can't take yet is dropped
        // (the next one replaces it anyway).
        {
            use std::os::fd::AsRawFd;
            // SAFETY: plain fcntl on a pipe we own.
            unsafe {
                let fd = stdin.as_raw_fd();
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        let mut helper = Helper { child, stdin: Some(stdin) };
        helper.move_to(at);
        Ok(Box::new(helper))
    }
}

struct Helper {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Overlay for Helper {
    fn move_to(&mut self, pos: Pos) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(format!("{} {}\n", pos.x, pos.y).as_bytes());
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        drop(self.stdin.take()); // the helper exits on end of input
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ------------------------------------------------------------ the helper

mod helper {
    use dispatch2::DispatchQueue;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSCursor, NSImageView,
        NSScreenSaverWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_core_foundation::{CGPoint, CGRect};
    use objc2_core_graphics::{CGDisplayBounds, CGMainDisplayID};
    use std::cell::RefCell;
    use std::io::BufRead;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Shown {
        window: Retained<NSWindow>,
        hot: CGPoint,
        /// The main display's height: AppKit's y grows upwards from its bottom.
        main_h: f64,
        ordered_front: bool,
    }

    thread_local! {
        static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
    }
    /// The latest position, and whether a move to it is queued on the main
    /// thread (moves coalesce: the main queue never piles up).
    static LATEST: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
    static QUEUED: AtomicBool = AtomicBool::new(false);

    fn place() {
        QUEUED.store(false, Ordering::SeqCst);
        let (x, y) = *LATEST.lock().unwrap_or_else(|e| e.into_inner());
        SHOWN.with(|s| {
            if let Some(s) = s.borrow_mut().as_mut() {
                s.window.setFrameTopLeftPoint(CGPoint::new(x - s.hot.x, s.main_h - (y - s.hot.y)));
                if !s.ordered_front {
                    s.window.orderFrontRegardless();
                    s.ordered_front = true;
                }
            }
        });
    }

    pub(super) fn main() -> ! {
        let Some(mtm) = MainThreadMarker::new() else { std::process::exit(2) };
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let cursor = NSCursor::arrowCursor();
        let image = cursor.image();
        let rect = CGRect::new(CGPoint::new(0.0, 0.0), image.size());
        // SAFETY: a plain borderless window, kept alive in `SHOWN`
        // (not released when closed).
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: we keep our own reference.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setLevel(NSScreenSaverWindowLevel);
        window.setIgnoresMouseEvents(true);
        window.setOpaque(false);
        window.setHasShadow(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        let view = NSImageView::imageViewWithImage(&image, mtm);
        window.setContentView(Some(&view));
        let main_h = CGDisplayBounds(CGMainDisplayID()).size.height;
        let hot = cursor.hotSpot();
        SHOWN.with(|s| *s.borrow_mut() = Some(Shown { window, hot, main_h, ordered_front: false }));

        std::thread::spawn(|| {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let mut it = line.split_whitespace().map(str::parse::<f64>);
                let (Some(Ok(x)), Some(Ok(y))) = (it.next(), it.next()) else { continue };
                if !(x.is_finite() && y.is_finite()) {
                    continue;
                }
                *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = (x, y);
                if !QUEUED.swap(true, Ordering::SeqCst) {
                    DispatchQueue::main().exec_async(place);
                }
            }
            std::process::exit(0);
        });
        app.run();
        std::process::exit(0)
    }
}

/// Runs the helper (see the module docs); never returns.
pub(crate) fn helper_main() -> ! {
    helper::main()
}
