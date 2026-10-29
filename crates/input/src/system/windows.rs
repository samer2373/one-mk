//! Windows: the foreground window compared with its monitor (see
//! [`is_fullscreen`]), `LockWorkStation`, and display power (keeping the
//! screen awake, turning it off, noticing it went off). Compiled and
//! unit-tested elsewhere only; not yet run on a real Windows machine.

use super::{DisplaySleeper, FocusProbe, Inhibitor, PowerEvent, ScreenLocker};
use tokio::sync::mpsc;
use crate::win_logic::{Ltrb, is_fullscreen};
use one_mk_proto::Focus;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
    SetThreadExecutionState,
};
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW, GWL_STYLE, GetClassNameW,
    GetDesktopWindow, GetForegroundWindow, GetMessageW, GetShellWindow, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, HWND_BROADCAST, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    PBT_POWERSETTINGCHANGE, PostMessageW, RegisterClassW, SC_MONITORPOWER, TranslateMessage, WM_POWERBROADCAST,
    WM_SYSCOMMAND, WNDCLASSW,
};
use std::sync::OnceLock;

pub(super) struct Foreground;

impl FocusProbe for Foreground {
    fn focus(&mut self) -> Option<Focus> {
        // SAFETY: plain Win32 calls; every out-pointer is valid and sized.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() || hwnd == GetShellWindow() || hwnd == GetDesktopWindow() {
                return Some(Focus::default());
            }
            let mut class = [0u16; 256];
            let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
            let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let fullscreen = match (window_bounds(hwnd), GetMonitorInfoW(mon, &mut mi) != 0) {
                (Some(r), true) => is_fullscreen(&class, style, r, ltrb(&mi.rcMonitor)),
                _ => false,
            };
            let mut apps = Vec::new();
            if let Some(exe) = process_name(hwnd) {
                apps.push(exe);
            }
            apps.push(class);
            Some(Focus { fullscreen, apps })
        }
    }
}

fn ltrb(r: &RECT) -> Ltrb {
    [r.left, r.top, r.right, r.bottom]
}

/// The window as drawn: DWM's extended frame bounds, without the invisible
/// resize borders `GetWindowRect` includes (about 8 px on each side), or
/// `GetWindowRect` if DWM can't tell.
fn window_bounds(hwnd: HWND) -> Option<Ltrb> {
    // SAFETY: both calls write one RECT to a valid, correctly sized pointer.
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        let size = size_of::<RECT>() as u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS as u32, (&mut r as *mut RECT).cast(), size) >= 0 {
            return Some(ltrb(&r));
        }
        (GetWindowRect(hwnd, &mut r) != 0).then(|| ltrb(&r))
    }
}

/// The program's file name, like `game.exe`.
fn process_name(hwnd: HWND) -> Option<String> {
    // SAFETY: plain Win32 calls; the handle is closed before returning.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        ok.then(|| path.rsplit('\\').next().unwrap_or(&path).to_owned())
    }
}

pub(super) struct Workstation;

impl ScreenLocker for Workstation {
    fn lock(&mut self) -> anyhow::Result<()> {
        // SAFETY: plain call; it only starts the lock.
        if unsafe { LockWorkStation() } == 0 {
            anyhow::bail!("LockWorkStation failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Keeps the display and the system awake: `SetThreadExecutionState` is
/// per thread, so a thread of its own sets it and waits (blocked, not
/// polling) until the guard is dropped, then clears it and ends.
pub(super) struct KeepAwake;

impl Inhibitor for KeepAwake {
    fn inhibit(&mut self, _reason: &str) -> anyhow::Result<Box<dyn Send>> {
        let (release, released) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new().name("one-mk-awake".into()).spawn(move || {
            // SAFETY: plain calls that only change this thread's state.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED) };
            let _ = released.recv(); // fails when the sender is dropped
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        })?;
        Ok(Box::new(release))
    }
}

/// `SC_MONITORPOWER` 2 (off), posted to every top-level window (posted, so
/// one that doesn't answer can't hold us up). Any input turns it back on.
pub(super) struct MonitorPower;

impl DisplaySleeper for MonitorPower {
    fn sleep_display(&mut self) -> anyhow::Result<()> {
        // SAFETY: plain call; the message carries no pointers.
        if unsafe { PostMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER as usize, 2) } == 0 {
            anyhow::bail!("could not turn the display off: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}

fn is_display_state(g: &windows_sys::core::GUID) -> bool {
    let want = &GUID_CONSOLE_DISPLAY_STATE;
    (g.data1, g.data2, g.data3, g.data4) == (want.data1, want.data2, want.data3, want.data4)
}

static POWER_EVENTS: OnceLock<mpsc::UnboundedSender<PowerEvent>> = OnceLock::new();

/// Reports the display turning off or on (`GUID_CONSOLE_DISPLAY_STATE`) and
/// the computer suspending or resuming, from a hidden window on a thread
/// of its own that sleeps in `GetMessageW` until Windows says something.
pub(super) fn watch_display(events: mpsc::UnboundedSender<PowerEvent>) {
    if POWER_EVENTS.set(events).is_err() {
        return; // already watching
    }
    let _ = std::thread::Builder::new().name("one-mk-power".into()).spawn(|| {
        let class: Vec<u16> = "one-mk-power\0".encode_utf16().collect();
        // SAFETY: standard window creation; all pointers outlive the calls.
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW { lpfnWndProc: Some(power_proc), hInstance: hinst, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            if RegisterClassW(&wc) == 0 {
                return;
            }
            // Never shown, but top level: suspend and resume are broadcast
            // to those only.
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return;
            }
            RegisterPowerSettingNotification(hwnd, &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

unsafe extern "system" fn power_proc(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if msg == WM_POWERBROADCAST {
        let asleep = match wparam as u32 {
            PBT_APMSUSPEND => Some(true),
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => Some(false),
            PBT_POWERSETTINGCHANGE => {
                // SAFETY: for this notification lparam points to a POWERBROADCAST_SETTING.
                let setting = unsafe { &*(lparam as *const POWERBROADCAST_SETTING) };
                // 0 off, 1 on, 2 dimmed.
                (is_display_state(&setting.PowerSetting) && setting.DataLength >= 4)
                    .then(|| unsafe { std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>()) } == 0)
            }
            _ => None,
        };
        if let (Some(asleep), Some(tx)) = (asleep, POWER_EVENTS.get()) {
            let _ = tx.send(PowerEvent::DisplayAsleep(asleep));
        }
        return 1; // TRUE: handled
    }
    // SAFETY: default handling.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
