//! System Settings, seen from outside: where its window is, which screens
//! there are, opening one of its privacy panes, and bringing our own app
//! back to the front. The permission guide (`one-mk-app`'s `guide.rs`)
//! uses these to stand its card next to the Settings window.
//!
//! Every coordinate is in points with the origin at the top-left of the
//! main display and y growing downwards: the space `CGWindowListCopyWindowInfo`
//! and `CGDisplayBounds` report in, and the one winit positions windows in.
//!
//! Only macOS has an implementation; elsewhere the functions report that
//! there is no Settings window and no screens, so callers need no `cfg`.
//! The macOS code is compile-checked for both Mac targets but has not
//! been run on a Mac (see docs/PERMISSION-GUIDE.md).

/// A rectangle in global points, top-left origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Frame {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }
}

/// The privacy panes one-mk needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    Accessibility,
    InputMonitoring,
}

impl Pane {
    /// The URL that opens the pane (macOS 11 and later, System Preferences
    /// and System Settings alike).
    pub fn url(self) -> &'static str {
        match self {
            Pane::Accessibility => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            Pane::InputMonitoring => "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
        }
    }
}

/// What can be seen of System Settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsWindow {
    /// Its main window, on screen, here, not hidden behind another app.
    Visible(Frame),
    /// On this Space, but mostly covered by another app's window (the
    /// person went to do something else): a guide pointing at it would
    /// point at nothing they can see.
    Covered(Frame),
    /// Running, but no window on the current Space: minimised, on another
    /// Space, or hidden (Cmd-H).
    Hidden,
    /// Not running (closing its last window quits it).
    NotRunning,
}

/// The bundle id of System Settings (macOS 13+) and System Preferences
/// (before): the same id, so a renamed or localised app still matches.
pub const SETTINGS_BUNDLE_ID: &str = "com.apple.systempreferences";
/// Owner names to fall back on when the process list can't be read.
const SETTINGS_NAMES: [&str; 2] = ["System Settings", "System Preferences"];
/// Smaller than this is a sheet, a tooltip or a helper, not the main window.
const MIN_SIDE: f32 = 120.0;

/// One entry of the on-screen window list, reduced to what we need.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub pid: i32,
    pub owner: String,
    pub layer: i64,
    pub alpha: f32,
    pub frame: Frame,
}

/// What is visible of Settings in a front-to-back window list: its main
/// window (see `pick_settings_window`), and whether a normal window of
/// another app (not Settings, not `own_pid`) in front of it covers more
/// than half of it.
pub fn settings_state(windows: &[WindowInfo], pids: Option<&[i32]>, own_pid: i32) -> Option<SettingsWindow> {
    let frame = pick_settings_window(windows, pids)?;
    let ours = |w: &WindowInfo| match pids {
        Some(p) => p.contains(&w.pid),
        None => SETTINGS_NAMES.contains(&w.owner.as_str()),
    };
    let index = windows.iter().position(|w| ours(w) && w.frame == frame)?;
    let covered = windows[..index]
        .iter()
        .filter(|w| w.layer == 0 && w.alpha > 0.0 && w.pid != own_pid && !ours(w))
        .map(|w| overlap(&w.frame, &frame))
        .fold(0.0f32, f32::max);
    Some(if covered > 0.5 * frame.area() { SettingsWindow::Covered(frame) } else { SettingsWindow::Visible(frame) })
}

fn overlap(a: &Frame, b: &Frame) -> f32 {
    let w = a.right().min(b.right()) - a.x.max(b.x);
    let h = a.bottom().min(b.bottom()) - a.y.max(b.y);
    w.max(0.0) * h.max(0.0)
}

/// The Settings main window among `windows`: owned by one of `pids` (or,
/// when the pids are unknown, by an app with Settings' name), at the
/// normal window layer, visible, not tiny, and the largest of those.
pub fn pick_settings_window(windows: &[WindowInfo], pids: Option<&[i32]>) -> Option<Frame> {
    windows
        .iter()
        .filter(|w| match pids {
            Some(p) => p.contains(&w.pid),
            None => SETTINGS_NAMES.contains(&w.owner.as_str()),
        })
        .filter(|w| w.layer == 0 && w.alpha > 0.0 && w.frame.w >= MIN_SIDE && w.frame.h >= MIN_SIDE)
        .max_by(|a, b| a.frame.area().total_cmp(&b.frame.area()))
        .map(|w| w.frame)
}

#[cfg(target_os = "macos")]
pub use imp::{bring_to_front, open_pane, screens, settings_window};

/// Where the Settings main window is, if it is on screen.
pub fn settings_window_frame() -> Option<Frame> {
    match settings_window() {
        SettingsWindow::Visible(f) | SettingsWindow::Covered(f) => Some(f),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn settings_window() -> SettingsWindow {
    SettingsWindow::NotRunning
}

#[cfg(not(target_os = "macos"))]
pub fn screens() -> Vec<Frame> {
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
pub fn open_pane(_pane: Pane) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "System Settings exists only on macOS"))
}

#[cfg(not(target_os = "macos"))]
pub fn bring_to_front() {}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Frame, Pane, SETTINGS_BUNDLE_ID, SettingsWindow, WindowInfo, settings_state};
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect};
    use objc2_core_graphics::{
        CGDisplayBounds, CGGetActiveDisplayList, CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo,
        CGWindowListOption, kCGNullWindowID, kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowOwnerName,
        kCGWindowOwnerPID,
    };

    // NSRunningApplication lives in AppKit.
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    /// `NSApplicationActivateAllWindows | NSApplicationActivateIgnoringOtherApps`.
    /// The second is ignored from macOS 14 on, where activation is
    /// cooperative (see `bring_to_front`).
    const ACTIVATE: usize = (1 << 0) | (1 << 1);

    fn class(name: &std::ffi::CStr) -> Option<&'static AnyClass> {
        AnyClass::get(name)
    }

    /// Process ids of the running System Settings, or None if AppKit
    /// could not be asked.
    fn settings_pids() -> Option<Vec<i32>> {
        let cls = class(c"NSRunningApplication")?;
        let id = CFString::from_str(SETTINGS_BUNDLE_ID);
        // CFString is toll-free bridged to NSString.
        let ns_id = (&*id as *const CFString).cast::<AnyObject>();
        // SAFETY: a class method taking an NSString and returning an
        // autoreleased NSArray<NSRunningApplication *> (never nil).
        let apps: Option<Retained<AnyObject>> =
            unsafe { msg_send![cls, runningApplicationsWithBundleIdentifier: ns_id] };
        let apps = apps?;
        // SAFETY: NSArray's count and objectAtIndex: within bounds; each
        // element is an NSRunningApplication answering processIdentifier.
        let n: usize = unsafe { msg_send![&*apps, count] };
        let mut pids = Vec::with_capacity(n);
        for i in 0..n {
            let app: Option<Retained<AnyObject>> = unsafe { msg_send![&*apps, objectAtIndex: i] };
            let Some(app) = app else { continue };
            let terminated: bool = unsafe { msg_send![&*app, isTerminated] };
            let pid: i32 = unsafe { msg_send![&*app, processIdentifier] };
            if !terminated && pid > 0 {
                pids.push(pid);
            }
        }
        Some(pids)
    }

    fn number(d: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<CFRetained<CFNumber>> {
        d.get(key)?.downcast::<CFNumber>().ok()
    }

    /// The on-screen windows of the current Space, front to back.
    fn on_screen_windows() -> Vec<WindowInfo> {
        let Some(list) = CGWindowListCopyWindowInfo(
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
            kCGNullWindowID,
        ) else {
            return Vec::new();
        };
        // SAFETY: CGWindowListCopyWindowInfo returns an array of
        // CFDictionary<CFString, CFType>.
        let list: &CFArray<CFDictionary<CFString, CFType>> = unsafe { list.cast_unchecked() };
        // SAFETY: the kCGWindow* keys are immutable CFStrings exported by
        // CoreGraphics.
        let (k_pid, k_owner, k_layer, k_alpha, k_bounds) =
            unsafe { (kCGWindowOwnerPID, kCGWindowOwnerName, kCGWindowLayer, kCGWindowAlpha, kCGWindowBounds) };
        let mut out = Vec::with_capacity(list.len());
        for i in 0..list.len() {
            let Some(d) = list.get(i) else { continue };
            let Some(pid) = number(&d, k_pid).and_then(|n| n.as_i32()) else { continue };
            let owner =
                d.get(k_owner).and_then(|s| s.downcast::<CFString>().ok()).map(|s| s.to_string()).unwrap_or_default();
            let layer = number(&d, k_layer).and_then(|n| n.as_i64()).unwrap_or(i64::MAX);
            let alpha = number(&d, k_alpha).and_then(|n| n.as_f64()).unwrap_or(1.0) as f32;
            let Some(bounds) = d.get(k_bounds).and_then(|b| b.downcast::<CFDictionary>().ok()) else { continue };
            let mut r = CGRect::default();
            // SAFETY: a bounds dictionary from the window list; `r` is a
            // valid CGRect to write into.
            if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(&bounds), &mut r) } {
                continue;
            }
            let frame = Frame::new(r.origin.x as f32, r.origin.y as f32, r.size.width as f32, r.size.height as f32);
            out.push(WindowInfo { pid, owner, layer, alpha, frame });
        }
        out
    }

    /// Where System Settings is. Cheap enough to call a few times a
    /// second (one window-list copy and one process lookup).
    pub fn settings_window() -> SettingsWindow {
        let pids = settings_pids();
        if pids.as_ref().is_some_and(|p| p.is_empty()) {
            return SettingsWindow::NotRunning;
        }
        let windows = on_screen_windows();
        match settings_state(&windows, pids.as_deref(), std::process::id() as i32) {
            Some(state) => state,
            // Running but nothing on this Space; without the process list
            // we can't tell hidden from quit, and "closed" asks for less.
            None if pids.is_some() => SettingsWindow::Hidden,
            None => SettingsWindow::NotRunning,
        }
    }

    /// The usable part of each screen (without the menu bar and the
    /// Dock), the main screen first. Call it on the main thread (AppKit's
    /// rule for NSScreen). Falls back to whole displays if AppKit can't
    /// be asked.
    pub fn screens() -> Vec<Frame> {
        let visible = ns_screens();
        if !visible.is_empty() {
            return visible;
        }
        let mut ids = [0u32; 16];
        let mut n = 0u32;
        // SAFETY: `ids` holds 16 entries and `n` is a valid out-pointer.
        let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut n) };
        if err.0 != 0 {
            return Vec::new();
        }
        ids[..(n as usize).min(ids.len())]
            .iter()
            .map(|&id| {
                let r = CGDisplayBounds(id);
                Frame::new(r.origin.x as f32, r.origin.y as f32, r.size.width as f32, r.size.height as f32)
            })
            .filter(|f| f.w > 0.0 && f.h > 0.0)
            .collect()
    }

    /// `NSScreen.screens[i].visibleFrame`, turned from Cocoa's
    /// bottom-left origin into the top-left one (the first screen holds
    /// the menu bar and has its origin at 0,0 in both).
    fn ns_screens() -> Vec<Frame> {
        let Some(cls) = class(c"NSScreen") else { return Vec::new() };
        // SAFETY: +[NSScreen screens] returns an NSArray<NSScreen *> (never
        // nil, possibly empty); frame and visibleFrame return NSRect, which
        // is CGRect on every 64-bit Mac.
        unsafe {
            let list: Option<Retained<AnyObject>> = msg_send![cls, screens];
            let Some(list) = list else { return Vec::new() };
            let n: usize = msg_send![&*list, count];
            let mut out = Vec::with_capacity(n);
            let mut main_h = None;
            for i in 0..n {
                let screen: Option<Retained<AnyObject>> = msg_send![&*list, objectAtIndex: i];
                let Some(screen) = screen else { continue };
                let frame: CGRect = msg_send![&*screen, frame];
                let vis: CGRect = msg_send![&*screen, visibleFrame];
                let h0 = *main_h.get_or_insert(frame.size.height);
                let top = h0 - (vis.origin.y + vis.size.height);
                let f = Frame::new(vis.origin.x as f32, top as f32, vis.size.width as f32, vis.size.height as f32);
                if f.w > 0.0 && f.h > 0.0 {
                    out.push(f);
                }
            }
            out
        }
    }

    /// Opens System Settings on `pane` (and brings it to the front).
    pub fn open_pane(pane: Pane) -> std::io::Result<()> {
        let status = std::process::Command::new("/usr/bin/open").arg(pane.url()).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("open {} failed: {status}", pane.url())))
        }
    }

    /// Asks macOS to bring this app (all its windows) to the front. From
    /// macOS 14 activation is cooperative: if the frontmost app (System
    /// Settings) did not yield, macOS may only bounce or ignore it.
    pub fn bring_to_front() {
        let Some(cls) = class(c"NSRunningApplication") else { return };
        // SAFETY: currentApplication never returns nil;
        // activateWithOptions: takes an NSUInteger and returns BOOL.
        unsafe {
            let me: Option<Retained<AnyObject>> = msg_send![cls, currentApplication];
            if let Some(me) = me {
                let _: bool = msg_send![&*me, activateWithOptions: ACTIVATE];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(pid: i32, owner: &str, layer: i64, frame: Frame) -> WindowInfo {
        WindowInfo { pid, owner: owner.into(), layer, alpha: 1.0, frame }
    }

    #[test]
    fn the_largest_normal_window_of_settings_wins() {
        let list = [
            w(10, "Finder", 0, Frame::new(0.0, 0.0, 1400.0, 900.0)),
            w(42, "System Settings", 0, Frame::new(100.0, 80.0, 715.0, 600.0)),
            w(42, "System Settings", 0, Frame::new(300.0, 200.0, 300.0, 140.0)),
            w(42, "System Settings", 3, Frame::new(0.0, 0.0, 2000.0, 2000.0)),
            w(42, "System Settings", 0, Frame::new(5.0, 5.0, 40.0, 40.0)),
        ];
        assert_eq!(pick_settings_window(&list, Some(&[42])), Some(Frame::new(100.0, 80.0, 715.0, 600.0)));
    }

    #[test]
    fn pids_beat_names_and_names_are_the_fallback() {
        let list = [
            w(7, "Systemeinstellungen", 0, Frame::new(0.0, 0.0, 700.0, 600.0)),
            w(8, "System Preferences", 0, Frame::new(0.0, 0.0, 668.0, 500.0)),
        ];
        // A localised name is still found by pid.
        assert_eq!(pick_settings_window(&list, Some(&[7])), Some(Frame::new(0.0, 0.0, 700.0, 600.0)));
        // Without pids, only the English names match.
        assert_eq!(pick_settings_window(&list, None), Some(Frame::new(0.0, 0.0, 668.0, 500.0)));
        assert_eq!(pick_settings_window(&list, Some(&[])), None);
    }

    #[test]
    fn invisible_windows_are_ignored() {
        let mut gone = w(42, "System Settings", 0, Frame::new(0.0, 0.0, 700.0, 600.0));
        gone.alpha = 0.0;
        assert_eq!(pick_settings_window(&[gone], Some(&[42])), None);
    }

    #[test]
    fn another_app_in_front_covers_it_but_ours_does_not() {
        let settings = Frame::new(100.0, 100.0, 700.0, 600.0);
        let front = |pid, owner: &str| w(pid, owner, 0, Frame::new(80.0, 80.0, 900.0, 700.0));
        let s = w(42, "System Settings", 0, settings);
        assert_eq!(settings_state(std::slice::from_ref(&s), Some(&[42]), 9), Some(SettingsWindow::Visible(settings)));
        assert_eq!(
            settings_state(&[front(5, "Safari"), s.clone()], Some(&[42]), 9),
            Some(SettingsWindow::Covered(settings))
        );
        // Our own window in front, or another app's window behind, is fine.
        assert_eq!(
            settings_state(&[front(9, "oneMK"), s.clone()], Some(&[42]), 9),
            Some(SettingsWindow::Visible(settings))
        );
        assert_eq!(
            settings_state(&[s.clone(), front(5, "Safari")], Some(&[42]), 9),
            Some(SettingsWindow::Visible(settings))
        );
        // A small window in front (a quarter of it) doesn't count; floating ones neither.
        let small = w(5, "Notes", 0, Frame::new(100.0, 100.0, 350.0, 300.0));
        let floating = w(5, "Safari", 3, Frame::new(0.0, 0.0, 2000.0, 2000.0));
        assert_eq!(settings_state(&[small, floating, s], Some(&[42]), 9), Some(SettingsWindow::Visible(settings)));
        assert_eq!(settings_state(&[], Some(&[42]), 9), None);
    }

    #[test]
    fn panes_have_their_urls() {
        assert!(Pane::Accessibility.url().ends_with("Privacy_Accessibility"));
        assert!(Pane::InputMonitoring.url().ends_with("Privacy_ListenEvent"));
    }
}
