//! Windows: the foreground window compared with its monitor, and
//! `LockWorkStation`. Compiled and unit-tested elsewhere only; not yet run
//! on a real Windows machine.

use super::{FocusProbe, ScreenLocker};
use one_mk_proto::Focus;
use windows_sys::Win32::Foundation::{CloseHandle, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowRect, GetWindowThreadProcessId,
};

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
            // The desktop (wallpaper and icons) covers the monitor too.
            if matches!(class.as_str(), "Progman" | "WorkerW") {
                return Some(Focus::default());
            }
            let mut r: RECT = std::mem::zeroed();
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let fullscreen = GetWindowRect(hwnd, &mut r) != 0
                && GetMonitorInfoW(mon, &mut mi) != 0
                && covers(&r, &mi.rcMonitor);
            let mut apps = Vec::new();
            if let Some(exe) = process_name(hwnd) {
                apps.push(exe);
            }
            apps.push(class);
            Some(Focus { fullscreen, apps })
        }
    }
}

fn covers(w: &RECT, m: &RECT) -> bool {
    w.left <= m.left && w.top <= m.top && w.right >= m.right && w.bottom >= m.bottom
}

/// The program's file name, like `game.exe`.
fn process_name(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<String> {
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
