//! The drawn pointer on Windows (see `cursoroverlay.rs`).
//!
//! Windows starts the cursor's display count at -1 when no mouse is
//! installed, so the cursor stays hidden even while `SendInput` moves it
//! (and Windows 8+ suppresses it after touch or pen input). The probe
//! reads `GetSystemMetrics(SM_MOUSEPRESENT)` and `GetCursorInfo`; both are
//! single cheap calls.
//!
//! The overlay is a layered window (`WS_EX_LAYERED | WS_EX_TRANSPARENT |
//! WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`: per-pixel alpha,
//! clicks go through, never focused, not in the taskbar or Alt+Tab) on a
//! thread of its own that sleeps in `GetMessageW`. The daemon moves it with
//! `SetWindowPos(SWP_ASYNCWINDOWPOS)`, which only posts to that thread, on
//! each injected move: no timer. The window registers for mouse device
//! arrivals and reports them to the controller, which decides: in `auto`
//! it has told the window to hide at once (so no stale arrow lingers
//! until the next injected event, when the controller removes it), in
//! `always` the arrow stays. Showing and hiding otherwise come from the
//! controller (`ShowWindowAsync`). Dropping it closes the window and joins
//! the thread.

use crate::cursoroverlay::{Overlay, Platform, Probe, Px, arrow, arrow_scale};
use one_mk_proto::Pos;
use one_mk_proto::overlay::OsCursor;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint,
    ReleaseDC, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSOR_SUPPRESSED, CURSORINFO, CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVTYP_DEVICEINTERFACE,
    DEV_BROADCAST_DEVICEINTERFACE_W, DEV_BROADCAST_HDR, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetCursorInfo, GetMessageW, GetSystemMetrics, HTTRANSPARENT, HWND_TOPMOST, IsWindowVisible, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterDeviceNotificationW, SM_MOUSEPRESENT, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE, SetWindowPos, ShowWindow, ShowWindowAsync, TranslateMessage, ULW_ALPHA, UnregisterDeviceNotification,
    UpdateLayeredWindow, WM_CLOSE, WM_DESTROY, WM_DEVICECHANGE, WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// `GUID_DEVINTERFACE_MOUSE` {378DE44C-56EF-11D1-BC8C-00A0C91405DD}.
const MOUSE_INTERFACE: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0x378D_E44C,
    data2: 0x56EF,
    data3: 0x11D1,
    data4: [0xBC, 0x8C, 0x00, 0xA0, 0xC9, 0x14, 0x05, 0xDD],
};

pub(crate) struct Windows;

impl Platform for Windows {
    fn probe(&mut self) -> Probe {
        // SAFETY: plain query.
        let mouse = unsafe { GetSystemMetrics(SM_MOUSEPRESENT) } != 0;
        let mut ci = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..unsafe { std::mem::zeroed() } };
        // SAFETY: `ci` is sized and outlives the call.
        let os_cursor = if unsafe { GetCursorInfo(&mut ci) } == 0 {
            OsCursor::Unknown
        } else if ci.flags & CURSOR_SUPPRESSED != 0 {
            OsCursor::Suppressed
        } else if ci.flags & CURSOR_SHOWING != 0 {
            OsCursor::Shown
        } else {
            OsCursor::Hidden
        };
        Probe { pointing_device: Some(mouse), os_cursor }
    }

    fn create(&mut self, at: Pos) -> anyhow::Result<Box<dyn Overlay>> {
        let arrival = Arc::new(Arrival::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = arrival.clone();
        let thread = std::thread::Builder::new()
            .name("one-mk-pointer".into())
            .spawn(move || window_thread(at, shared, tx))?;
        match rx.recv() {
            Ok(Ok((hwnd, scale, hot))) => Ok(Box::new(Window { hwnd, scale, hot, arrival, thread: Some(thread) })),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                anyhow::bail!("the pointer window's thread ended")
            }
        }
    }
}

struct Window {
    /// The `HWND`, as an integer so the overlay can move between threads.
    hwnd: usize,
    /// Logical (the daemon's) to physical pixels: the primary monitor's
    /// DPI / 96, the same rule as the injection's `Space`.
    scale: f64,
    /// Nothing: the tip is the window's top-left pixel. Kept for clarity
    /// should the shape ever change.
    hot: (i32, i32),
    arrival: Arc<Arrival>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A mouse arriving, between the window's thread and the controller.
#[derive(Default)]
struct Arrival {
    /// One arrived since the controller last asked.
    seen: AtomicBool,
    /// The controller's decision for that moment: hide at once (`auto`).
    hide: AtomicBool,
}

impl Overlay for Window {
    fn move_to(&mut self, pos: Pos) {
        let x = (pos.x * self.scale).round() as i32 - self.hot.0;
        let y = (pos.y * self.scale).round() as i32 - self.hot.1;
        // SAFETY: the window lives until `drop`; ASYNC only posts the move
        // to its thread, so the daemon never waits on it.
        unsafe {
            SetWindowPos(
                self.hwnd as HWND,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            )
        };
    }

    fn set_visible(&mut self, on: bool) {
        // SAFETY: the window lives until `drop`; IsWindowVisible only reads
        // its style, and ShowWindowAsync posts to its thread.
        unsafe {
            if (IsWindowVisible(self.hwnd as HWND) != 0) != on {
                ShowWindowAsync(self.hwnd as HWND, if on { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
    }

    fn take_arrival(&mut self) -> bool {
        self.arrival.seen.swap(false, Ordering::AcqRel)
    }

    fn hide_on_arrival(&mut self, on: bool) {
        self.arrival.hide.store(on, Ordering::Release);
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: posting to a window of ours; its thread destroys it.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

thread_local! {
    /// The window thread's side of [`Arrival`], for `proc`.
    static ARRIVAL: RefCell<Option<Arc<Arrival>>> = const { RefCell::new(None) };
}

type Created = anyhow::Result<(usize, f64, (i32, i32))>;

fn window_thread(at: Pos, arrival: Arc<Arrival>, tx: std::sync::mpsc::Sender<Created>) {
    ARRIVAL.with(|a| *a.borrow_mut() = Some(arrival));
    let made = match unsafe { make_window(at) } {
        Ok(made) => made,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };
    let (hwnd, _, _) = made;
    let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
        dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
        dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
        dbcc_reserved: 0,
        dbcc_classguid: MOUSE_INTERFACE,
        dbcc_name: [0],
    };
    // SAFETY: valid window and filter; unregistered below.
    let notify = unsafe {
        RegisterDeviceNotificationW(hwnd as HWND, (&filter as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(), DEVICE_NOTIFY_WINDOW_HANDLE)
    };
    let _ = tx.send(Ok(made));
    // SAFETY: standard message loop on the window's own thread.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !notify.is_null() {
            UnregisterDeviceNotification(notify);
        }
    }
}

/// Creates and shows the window with the arrow; `(hwnd, scale, hot spot)`.
///
/// # Safety
/// Call on the thread that will run the window's message loop.
unsafe fn make_window(at: Pos) -> Created {
    // SAFETY (whole function): standard Win32 window and GDI calls; every
    // pointer passed outlives its call, and every GDI object made is freed.
    unsafe {
        if SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_null() {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
        }
        let primary = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(primary, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) < 0 || dx == 0 {
            dx = 96;
        }
        let scale = f64::from(dx) / 96.0;
        let rows = arrow(arrow_scale(scale));
        let (w, h) = (rows[0].len() as i32, rows.len() as i32);

        let class: Vec<u16> = "one-mk-pointer\0".encode_utf16().collect();
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hinst, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            anyhow::bail!("RegisterClassW failed ({})", GetLastError());
        }
        let (x, y) = ((at.x * scale).round() as i32, (at.y * scale).round() as i32);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            anyhow::bail!("CreateWindowExW failed ({})", GetLastError());
        }

        // The arrow as premultiplied BGRA, top-down.
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let screen = GetDC(std::ptr::null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let mut ok = false;
        if !bitmap.is_null() && !bits.is_null() {
            let px = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (w * h) as usize);
            for (i, p) in rows.iter().flatten().enumerate() {
                px[i] = match p {
                    Px::Clear => 0,
                    Px::Outline => 0xFF00_0000,
                    Px::Fill => 0xFFFF_FFFF,
                };
            }
            let old = SelectObject(mem, bitmap);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let (dst, size, src) = (POINT { x, y }, SIZE { cx: w, cy: h }, POINT { x: 0, y: 0 });
            ok = UpdateLayeredWindow(hwnd, screen, &dst, &size, mem, &src, 0, &blend, ULW_ALPHA) != 0;
            SelectObject(mem, old);
            DeleteObject(bitmap);
        }
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
        if !ok {
            DestroyWindow(hwnd);
            anyhow::bail!("drawing the pointer window failed ({})", GetLastError());
        }
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        Ok((hwnd as usize, scale, (0, 0)))
    }
}

/// Whether a `DBT_DEVICEARRIVAL`'s `lparam` announces a mouse interface.
fn is_mouse_interface(lparam: isize) -> bool {
    if lparam == 0 {
        return false;
    }
    // SAFETY: for DBT_DEVICEARRIVAL, lparam points to a DEV_BROADCAST_HDR;
    // the interface fields are read only once its type says it has them.
    unsafe {
        let hdr = &*(lparam as *const DEV_BROADCAST_HDR);
        if hdr.dbch_devicetype != DBT_DEVTYP_DEVICEINTERFACE {
            return false;
        }
        let guid = (*(lparam as *const DEV_BROADCAST_DEVICEINTERFACE_W)).dbcc_classguid;
        (guid.data1, guid.data2, guid.data3, guid.data4)
            == (MOUSE_INTERFACE.data1, MOUSE_INTERFACE.data2, MOUSE_INTERFACE.data3, MOUSE_INTERFACE.data4)
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        // Clicks go to whatever is under it (WS_EX_TRANSPARENT does this
        // for layered windows already; this covers hit tests).
        WM_NCHITTEST => return HTTRANSPARENT as isize,
        // Volumes and ports arrive here too (broadcast to every top-level
        // window): only a mouse interface counts.
        WM_DEVICECHANGE if wparam as u32 == DBT_DEVICEARRIVAL && is_mouse_interface(lparam) => {
            // A mouse was plugged in: tell the controller, which decides
            // (and said beforehand whether to hide at once).
            ARRIVAL.with(|a| {
                if let Some(a) = a.borrow().as_ref() {
                    a.seen.store(true, Ordering::Release);
                    if a.hide.load(Ordering::Acquire) {
                        // SAFETY: our window, on its own thread.
                        unsafe { ShowWindow(hwnd, SW_HIDE) };
                    }
                }
            });
            return 1;
        }
        WM_CLOSE => {
            // SAFETY: our window, on its own thread.
            unsafe { DestroyWindow(hwnd) };
            return 0;
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        _ => {}
    }
    // SAFETY: default handling.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
