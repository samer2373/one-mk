//! Windows backend: low-level hooks for capture, `SendInput` for injection.
//!
//! - Capture runs on its own thread with a Win32 message loop. `WH_MOUSE_LL`
//!   and `WH_KEYBOARD_LL` callbacks run on that thread while it waits for
//!   messages. When the pointer touches an armed edge segment (outer or
//!   an inner exposed edge, see `CaptureCmd::SetEdges`), the hooks start
//!   swallowing local input: the cursor is parked in the middle of a
//!   screen under a tiny invisible window that hides it, and
//!   each mouse event's distance from the park point is the motion delta.
//!   Because the swallowed event never moves the cursor, that distance is
//!   exactly the post-acceleration movement Windows would have applied.
//! - Injection runs on a second thread and turns [`Inject`] messages into
//!   `SendInput` calls, batching everything already queued into one call.
//! - Coordinates: see [`crate::win_logic`] for the space we report and why.
//!
//! Windows silently removes low-level hooks that take longer than
//! `LowLevelHooksTimeout` (about 300 ms to 1 s) to return, so the hook
//! callbacks only update in-thread state, push to an unbounded channel and
//! (once per activation) warp the cursor and read the key state. Everything
//! else is posted to the window and done by the message loop. A guard timer
//! notices if the hooks were removed anyway (the parked cursor keeps
//! moving), reinstalls them, and gives the pointer back if that doesn't help.
//!
//! Which keyboard or mouse each event came from (per-device rules) is read
//! with Raw Input while capturing, and matched to the hooks' events (see
//! `windows_devices`; compile-checked only).
//!
//! Pen, touch and injected mouse input never starts a crossing and is never
//! forwarded; injected keys are left alone. Real input while a peer is in
//! control here is reported as `Event::LocalInput`.
//!
//! TODO: run as a SYSTEM service (with a helper in the user's session) so
//! that UAC prompts, the secure desktop (Ctrl+Alt+Del) and the login screen
//! can be controlled. A normal process can't see or inject input there, and
//! can't inject into windows of elevated programs (UIPI).

use crate::common_logic::{Controlled, KeyRepeat, LocalInputGate, drain_coalesced, is_repeatable, now_ms};
use crate::win_logic::{
    BTN_EXTRA, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_SIDE, EdgeGate, Heading, Held, HookWatchdog, LLMHF_INJECTED,
    LLMHF_LOWER_IL_INJECTED, SenderKeys, SmoothScroll, Space, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1,
    VK_XBUTTON2, Watch, WinKey, evdev_to_win, hook_key_to_evdev, is_synthetic_mouse, repeat_timing, vk_to_evdev,
    wheel_to_wire, wire_to_wheel,
};
use crate::{Backend, CaptureCmd, Event, Inject};
use crate::localmap_handoff::Destination;
use crate::windows_localmap::Local;
use anyhow::{Result, bail};
use one_mk_proto::{Edge, Mods, Rect, Side};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::RemoteDesktop::{NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetSystemMetrics, GetWindowThreadProcessId, HC_ACTION, HHOOK, HWND_TOPMOST,
    KBDLLHOOKSTRUCT, KillTimer, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, LWA_ALPHA, MONITORINFOF_PRIMARY, MSG,
    MSLLHOOKSTRUCT, PostMessageW, PostQuitMessage, RegisterClassW, SM_SWAPBUTTON, SPI_GETKEYBOARDDELAY,
    SPI_GETKEYBOARDSPEED, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetCursor, SetCursorPos,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, SetWindowsHookExW, ShowWindow, SystemParametersInfoW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP,
    WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_INPUT, WM_INPUT_DEVICE_CHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT,
    WTS_SESSION_LOCK, WTS_SESSION_LOGOFF, WTS_SESSION_UNLOCK,
};

/// Tag in `dwExtraInfo` of everything we inject, so our own hooks let it
/// through and never mistake it for local input. ("1mk!")
const OUR_INPUT: usize = 0x316d_6b21;

/// Posted by the command forwarder: commands are waiting in the queue.
const WM_APP_CMD: u32 = WM_APP + 1;
/// Posted by the hook: show (wparam 1) or hide (0) the cursor-hiding window.
const WM_APP_OVERLAY: u32 = WM_APP + 2;

/// While captured, keeps the cursor parked even if something else moves it.
const TIMER_GUARD: usize = 1;
/// Monitor changes arrive in bursts; re-read the layout once they settle.
const TIMER_DISPLAY: usize = 2;
/// An event waits for Raw Input to say its device; runs only while one does.
const TIMER_DEVICES: usize = 3;
/// Posted when capture starts: read Raw Input from now on (not in the hook,
/// which must return quickly).
const WM_APP_DEVICES: u32 = WM_APP + 3;
pub(crate) const WM_APP_LOCAL: u32 = WM_APP + 4;
const TIMER_LOCAL: usize = 4;
pub(crate) const WM_APP_LOCAL_DESKTOP: u32 = WM_APP + 5;

/// Side of the cursor-hiding window, in physical pixels.
const OVERLAY: i32 = 64;

const XBUTTON1: u16 = 1;
const XBUTTON2: u16 = 2;

pub(crate) async fn open(_state_dir: PathBuf) -> Result<Backend> {
    // SAFETY: plain Win32 call with a predefined context. It fails if the
    // awareness was already set (e.g. by a manifest); every thread of ours
    // also sets it for itself below, so that is fine.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    // Enumerate on a thread whose DPI awareness we control, not on a shared
    // runtime worker.
    let space = std::thread::spawn(|| {
        set_thread_dpi_awareness();
        read_space()
    })
    .join()
    .map_err(|_| anyhow::anyhow!("reading the monitor layout crashed"))??;

    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cap_tx, cap_rx) = mpsc::unbounded_channel();
    let (inj_tx, inj_rx) = mpsc::unbounded_channel();
    let _ = ev_tx.send(Event::Desktop(space.logical().clone()));
    tracing::info!(screens = ?space.phys().screens, scale = space.scale(), "monitors (physical pixels)");
    let shared = Arc::new(Mutex::new(space));
    let controlled = Arc::new(Controlled::default());

    let (focus, locker) = crate::system::native(&ev_tx);
    let (tx, sh, ctl) = (ev_tx.clone(), shared.clone(), controlled.clone());
    std::thread::Builder::new().name("one-mk-capture".into()).spawn(move || {
        if let Err(e) = capture_thread(tx.clone(), cap_rx, sh, ctl) {
            tracing::warn!("input capture unavailable: {e:#}");
            let _ = tx.send(Event::Error(format!("Can't share this computer's mouse and keyboard with others: {e:#}")));
        }
    })?;
    std::thread::Builder::new()
        .name("one-mk-inject".into())
        .spawn(move || inject_thread(ev_tx, inj_rx, shared, controlled))?;
    Ok(Backend { keyboard: None, name: "native", events: ev_rx, capture: cap_tx, inject: inj_tx, focus, locker })
}

fn set_thread_dpi_awareness() {
    // SAFETY: plain Win32 calls with predefined contexts.
    unsafe {
        if SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_null() {
            // Windows 10 before 1703 has no v2; v1 gives the same coordinates.
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
        }
    }
}

fn desktop_active() -> bool {
    use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, DESKTOP_READOBJECTS, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME};
    // SAFETY: query-only input-desktop access; no desktop switching or input installation.
    unsafe {
        let desktop = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
        if desktop.is_null() { return false; }
        let mut name = [0_u16; 64];
        let mut needed = 0;
        let read = GetUserObjectInformationW(desktop, UOI_NAME, name.as_mut_ptr().cast(), size_of_val(&name) as u32, &mut needed);
        CloseDesktop(desktop);
        read != 0 && name[..8] == [68, 101, 102, 97, 117, 108, 116, 0]
    }
}

/// The monitors in physical pixels and the primary monitor's DPI.
fn read_space() -> Result<Space> {
    struct Found {
        screens: Vec<Rect>,
        primary_dpi: u32,
    }
    unsafe extern "system" fn each(mon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        // SAFETY: `data` is the `&mut Found` passed below, alive for the call.
        let found = unsafe { &mut *(data as *mut Found) };
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..unsafe { std::mem::zeroed() } };
        // SAFETY: valid monitor handle from the enumeration; `mi` is sized.
        if unsafe { GetMonitorInfoW(mon, &mut mi) } != 0 {
            let r = mi.rcMonitor;
            found.screens.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
            if mi.dwFlags & MONITORINFOF_PRIMARY != 0 {
                let (mut x, mut y) = (0u32, 0u32);
                // SAFETY: valid monitor handle and out-pointers.
                if unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y) } >= 0 {
                    found.primary_dpi = x;
                }
            }
        }
        1
    }
    let mut found = Found { screens: Vec::new(), primary_dpi: 96 };
    // SAFETY: the callback only touches `found`, which outlives the call.
    let ok = unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), &mut found as *mut Found as LPARAM)
    };
    found.screens.retain(|r| r.w > 0 && r.h > 0);
    if ok == 0 || found.screens.is_empty() {
        bail!("Windows reported no monitors");
    }
    Ok(Space::new(found.screens, found.primary_dpi))
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- capture

/// State shared by the hooks and the window procedure. Both run on the
/// capture thread, so a thread-local `RefCell` is all the locking needed.
/// Win32 calls are made only after the borrow ends, in case one of them
/// lets a hook run re-entrantly.
struct State {
    tx: mpsc::UnboundedSender<Event>,
    hwnd: HWND,
    shared: Arc<Mutex<Space>>,
    space: Space,
    edges: Vec<Edge>,
    cmds: std_mpsc::Receiver<CaptureCmd>,
    captured: Option<Captured>,
    /// The last activation id handed out.
    next_id: u32,
    /// evdev keys and buttons that went down locally (not forwarded); their
    /// release must reach this machine even while captured, or they would
    /// stay stuck down here. Rebuilt from the real key state when capture
    /// starts, because Win+L, Ctrl+Alt+Del and UAC prompts hide key-ups.
    local: Held,
    /// Keys forwarded while captured, to drop auto-repeat.
    fwd: SenderKeys,
    gate: EdgeGate,
    /// The local mouse's direction of movement, for corners.
    heading: Heading,
    controlled: Arc<Controlled>,
    local_gate: LocalInputGate,
    hooks: Hooks,
    watchdog: HookWatchdog,
    /// We run elevated ourselves, so elevated windows are no problem.
    self_elevated: bool,
    /// Foreground window and whether it is elevated, to avoid asking twice.
    elevated_cache: Option<(usize, bool)>,
    /// Raw Input: the devices, and the matching of events to them.
    raw: crate::windows_devices::RawDevices,
    local_rules: Option<Local>,
    local_recovery: Option<crate::windows_localmap::Recovery>,
    session_active: bool,
    awake: bool,
    session_notifications: bool,
}

#[derive(Clone, Copy)]
struct Captured {
    id: u32,
    /// Where the cursor is kept while captured.
    park: (i32, i32),
    /// Where it hit the edge, for `Release { pos: None }`.
    entry: (i32, i32),
}

#[derive(Clone, Copy)]
struct Hooks {
    mouse: HHOOK,
    keyboard: HHOOK,
}

impl Hooks {
    /// Installs both low-level hooks, or neither (returning the error code).
    fn install() -> std::result::Result<Self, u32> {
        // SAFETY: the hook procedures are valid for the life of the thread;
        // the module handle is our own exe.
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let hooks = Hooks {
                mouse: SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hinst, 0),
                keyboard: SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinst, 0),
            };
            if hooks.mouse.is_null() || hooks.keyboard.is_null() {
                let err = GetLastError();
                hooks.remove();
                return Err(err);
            }
            Ok(hooks)
        }
    }

    fn remove(self) {
        for h in [self.mouse, self.keyboard] {
            if !h.is_null() {
                // SAFETY: a hook handle we installed. Fails harmlessly if
                // Windows already removed it.
                unsafe { UnhookWindowsHookEx(h) };
            }
        }
    }
}

/// Whether the real (async) state says `vk` is down.
fn async_down(vk: u16) -> bool {
    // SAFETY: plain call.
    unsafe { GetAsyncKeyState(i32::from(vk)) as u16 & 0x8000 != 0 }
}

/// Keys and buttons physically held right now, as evdev codes.
fn async_held() -> Held {
    // SAFETY: plain call.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let mut held = Held::default();
    for vk in 1u16..=0xFE {
        if async_down(vk) {
            // SAFETY: plain call.
            let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC_EX) };
            if let Some(code) = vk_to_evdev(vk, scan, swapped) {
                held.set(code, true);
            }
        }
    }
    held
}

/// Whether any mouse button is physically held right now.
fn async_any_button() -> bool {
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2].into_iter().any(async_down)
}

/// Whether process `pid` runs elevated. `None` if that can't be told; a
/// process we may not even query counts as elevated.
fn process_elevated(pid: u32) -> Option<bool> {
    // SAFETY: handles are checked and closed; the out-pointers are valid.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return (GetLastError() == ERROR_ACCESS_DENIED).then_some(true);
        }
        let mut token: HANDLE = std::ptr::null_mut();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token) != 0;
        let err = GetLastError();
        CloseHandle(process);
        if !opened {
            return (err == ERROR_ACCESS_DENIED).then_some(true);
        }
        let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut info as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        ) != 0;
        CloseHandle(token);
        ok.then_some(info.TokenIsElevated != 0)
    }
}

thread_local! {
    /// `timeBeginPeriod(1)` is in effect (only while captured).
    static HIRES: Cell<bool> = const { Cell::new(false) };
}

fn set_hires(on: bool) {
    if HIRES.with(|h| h.replace(on)) != on {
        // SAFETY: plain calls; begin and end are kept balanced by HIRES.
        unsafe {
            if on {
                timeBeginPeriod(1);
            } else {
                timeEndPeriod(1);
            }
        }
    }
}

/// Win32 work a hook or command asks for, done after the state borrow ends.
enum Effect {
    None,
    /// Captured: park the cursor and hide it.
    Park((i32, i32)),
    /// Released: put the cursor back and show it.
    Unpark((i32, i32)),
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn capture_thread(
    tx: mpsc::UnboundedSender<Event>,
    mut cmds: mpsc::UnboundedReceiver<CaptureCmd>,
    shared: Arc<Mutex<Space>>,
    controlled: Arc<Controlled>,
) -> Result<()> {
    set_thread_dpi_awareness();
    let space = shared.lock().map_err(|_| anyhow::anyhow!("state poisoned"))?.clone();
    let class = utf16("one-mk-capture");
    // SAFETY: standard window creation; all pointers outlive the calls.
    let hwnd = unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            // No class cursor: WM_SETCURSOR sets none, which hides it.
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            bail!("couldn't register a window class (error {})", GetLastError());
        }
        // A tiny, nearly transparent, topmost tool window that never takes
        // focus. It is shown under the parked cursor to hide it, and as a
        // hidden top-level window it also receives WM_DISPLAYCHANGE.
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            OVERLAY,
            OVERLAY,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            bail!("couldn't create a window (error {})", GetLastError());
        }
        // Alpha 1, not 0: fully transparent layered windows are skipped by
        // hit testing, and then the cursor shape would come from whatever
        // is underneath.
        SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
        hwnd
    };
    // SAFETY: register once on our own capture window and retain failure state.
    let session_notifications = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;

    let hooks = match Hooks::install() {
        Ok(h) => h,
        Err(err) => {
            // SAFETY: our window.
            unsafe { DestroyWindow(hwnd) };
            bail!(
                "Windows refused to let one-mk watch the mouse and keyboard (error {err}); security software may be blocking it"
            );
        }
    };
    // SAFETY: plain call.
    let self_elevated = process_elevated(unsafe { GetCurrentProcessId() }).unwrap_or(false);

    let (cmd_tx, cmd_rx) = std_mpsc::channel();
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            tx,
            hwnd,
            shared,
            space,
            edges: Vec::new(),
            cmds: cmd_rx,
            captured: None,
            next_id: 0,
            local: Held::default(),
            fwd: SenderKeys::default(),
            gate: EdgeGate::default(),
            heading: Heading::default(),
            controlled,
            local_gate: LocalInputGate::default(),
            hooks,
            watchdog: HookWatchdog::default(),
            self_elevated,
            elevated_cache: None,
            raw: Default::default(),
            local_rules: None,
            local_recovery: None,
            session_active: true,
            awake: true,
            session_notifications,
        })
    });
    with_state(State::devices_changed);
    tracing::info!("input capture ready");

    // The daemon's command channel is async; forward it into the message
    // loop by queueing the command and waking the window.
    let hwnd_addr = hwnd as usize;
    std::thread::Builder::new().name("one-mk-capture-cmd".into()).spawn(move || {
        let hwnd = hwnd_addr as HWND;
        while let Some(cmd) = cmds.blocking_recv() {
            if cmd_tx.send(cmd).is_err() {
                return;
            }
            // SAFETY: posting to a window handle is safe even if it is gone.
            unsafe { PostMessageW(hwnd, WM_APP_CMD, 0, 0) };
        }
        // The daemon is gone: shut the capture thread down.
        // SAFETY: as above.
        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    })?;

    // SAFETY: standard message loop on the thread that owns the window and
    // the hooks; hook callbacks are delivered from inside GetMessageW.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
    }

    // If we exit while captured, give the user their cursor back.
    let state = STATE.with(|s| s.borrow_mut().take());
    if let Some(st) = state {
        st.hooks.remove();
        if let Some(c) = st.captured {
            // SAFETY: plain call.
            unsafe { SetCursorPos(c.entry.0, c.entry.1) };
        }
    }
    set_hires(false);
    Ok(())
}

/// Runs `f` on the state, unless it is missing or already borrowed (which
/// would mean a re-entrant call; then the event is passed through).
fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

fn apply(effect: Effect, hwnd: HWND) {
    // SAFETY: plain Win32 calls on our own window.
    unsafe {
        match effect {
            Effect::None => {}
            Effect::Park((x, y)) => {
                set_hires(true);
                SetCursorPos(x, y);
                PostMessageW(hwnd, WM_APP_OVERLAY, 1, 0);
            }
            Effect::Unpark((x, y)) => {
                PostMessageW(hwnd, WM_APP_OVERLAY, 0, 0);
                SetCursorPos(x, y);
                set_hires(false);
            }
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points to an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if let Some((swallow, effect, hwnd)) = with_state(|s| {
            let (swallow, effect) = s.on_mouse(wparam as u32, info);
            (swallow, effect, s.hwnd)
        }) {
            apply(effect, hwnd);
            if swallow {
                return 1;
            }
        }
    }
    // SAFETY: forwarding the unmodified arguments.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let local_rules = with_state(|state| state.local_rules.is_some()).unwrap_or(false);
        if info.dwExtraInfo == OUR_INPUT || local_rules && (info.flags & LLKHF_INJECTED != 0 || info.vkCode == 0xE5) {
            // SAFETY: unmodified synthetic input and VK_PROCESSKEY bypass all routing.
            return unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) };
        }
        if let Some((swallow, effect, hwnd)) = with_state(|s| {
            let (swallow, effect) = s.on_key(info);
            (swallow, effect, s.hwnd)
        }) {
            apply(effect, hwnd);
            if swallow {
                return 1;
            }
        }
    }
    // SAFETY: forwarding the unmodified arguments.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// What the guard timer found.
enum Guard {
    Rewarp((i32, i32)),
    /// Reinstall these hooks, then re-park at the point.
    Reinstall(Hooks, (i32, i32)),
    /// Capture was ended; apply the effect, then reinstall the hooks so the
    /// next crossing works.
    GaveUp(Hooks, Effect),
    Ended(Effect),
}

impl State {
    fn local_status(&self, status: &str) {
        let _ = self.tx.send(Event::Status {
            part: "local-key-rules",
            problem: Some(format!("Local key rules {status}; EXPERIMENTAL, native Windows UNTESTED (compile-checked only)")),
        });
    }

    fn local_drain(&mut self) {
        let mut out = Vec::new();
        if let Some(local) = self.local_rules.as_mut() {
            while let Some(key) = local.admission.pop() {
                local.waiting.push_back(key);
                let event = Event::Key { code: key.code, down: key.down, device: None };
                if local.snapshots.load().maps.has_device_rules() {
                    self.raw.corr.hook(event, now_ms(), &mut out);
                } else { local.forward(&event); }
            }
        }
        self.send_all(out);
    }

    fn local_fence(&mut self, destination: Destination) {
        let Some(local) = self.local_rules.as_ref() else { return };
        local.gate.close();
        if destination == Destination::Remote { local.admission.fail(); }
        self.local_drain();
        let mut out = Vec::new();
        self.raw.corr.finish(&mut out);
        self.send_all(out);
        if let Some(local) = self.local_rules.as_mut() { local.fence(destination); }
        // SAFETY: our window; this watchdog exists only while work is outstanding.
        unsafe { SetTimer(self.hwnd, TIMER_LOCAL, 10, None) };
    }

    fn local_pause(&mut self) {
        if let Some(local) = self.local_rules.as_mut() {
            local.gate.close();
            local.admission.fail();
            local.paused = true;
        }
        self.local_drain();
        let mut out = Vec::new();
        self.raw.corr.finish(&mut out);
        self.send_all(out);
        if let Some(local) = self.local_rules.as_mut() { local.fail(); }
        if self.local_rules.is_some() { self.local_status("inactive: session/desktop/device change; reload local rules to resume"); }
    }

    fn local_pump(&mut self) -> Effect {
        if self.local_recovery.as_ref().is_some_and(|recovery| recovery.finished()) { self.local_recovery = None; }
        self.local_drain();
        let mut action = None;
        let mut done = None;
        let mut failed = false;
        let mut input = false;
        if let Some(local) = self.local_rules.as_mut() {
            input = std::mem::take(&mut local.input);
            while let Some(next) = local.action() { action = Some(next); }
            if self.tx.is_closed() || local.pending.is_some_and(|(_, started)| started.elapsed() >= crate::localmap_runtime::LEASE)
                || !local.admission.healthy(Instant::now())
                    && (local.pending.is_none() || local.admission.feedback().token() != local.recovery)
            {
                local.fail();
                failed = true;
            } else {
                done = local.completion();
            }
            // SAFETY: our own event-driven watchdog, never armed at idle.
            unsafe {
                if local.pending.is_some() || local.admission.pending() {
                    SetTimer(self.hwnd, TIMER_LOCAL, 10, None);
                } else {
                    KillTimer(self.hwnd, TIMER_LOCAL);
                }
            }
        }
        if input { self.local_input(); }
        if failed { self.local_status("inactive: worker failed, timed out, or emergency shortcut; reload to recover"); }
        if let Some(action) = action { let _ = self.tx.send(Event::Hotkey(action)); }
        if let Some(done) = done {
            if !done.success {
                self.local_status("inactive: injection/fence failed; reload to retry releases");
            } else if let Some(local) = self.local_rules.as_mut() {
                let activation = local.activation.take();
                let count = local.snapshots.load().count;
                let enabled = local.enabled;
                if done.destination == Destination::Remote {
                    self.local_status("inactive: forwarding to another computer");
                    if let Some((side, x, y)) = activation { return self.activate_now(side, x, y).1; }
                } else if enabled {
                    self.local_status(&format!("active ({count})"));
                } else {
                    let reason = local.inactive.clone();
                    self.local_status(&format!("inactive: {reason}"));
                }
            }
        }
        Effect::None
    }

    /// Sends to the daemon; if it is gone while captured, gives the pointer back.
    fn send(&mut self, ev: Event) -> Effect {
        if self.captured.is_some() {
            // In order, once Raw Input has said (or not) which device it was.
            let mut out = Vec::new();
            self.raw.corr.hook(ev, now_ms(), &mut out);
            return self.send_all(out);
        }
        if self.tx.send(ev).is_err() {
            return self.release(None);
        }
        Effect::None
    }

    /// Sends what the device matching let through; its timer runs only
    /// while an event waits.
    fn send_all(&mut self, out: Vec<Event>) -> Effect {
        for ev in out {
            if self.local_rules.as_mut().is_some_and(|local| local.forward(&ev)) { continue; }
            if self.tx.send(ev).is_err() {
                return self.release(None);
            }
        }
        let wait = u32::try_from(crate::devcorr::WAIT_MS).unwrap_or(8);
        // SAFETY: our own window and timer.
        unsafe {
            if self.raw.corr.deadline().is_some() {
                SetTimer(self.hwnd, TIMER_DEVICES, wait, None);
            } else {
                KillTimer(self.hwnd, TIMER_DEVICES);
            }
        }
        Effect::None
    }

    /// Says which keyboards and mice are connected, if that changed.
    fn devices_changed(&mut self) {
        let mut out = Vec::new();
        let list = self.raw.enumerate(&mut out);
        // What waited for a device that went away comes before the news.
        for ev in out {
            if !self.local_rules.as_mut().is_some_and(|local| local.forward(&ev)) {
                let _ = self.tx.send(ev);
            }
        }
        if let Some(list) = list {
            if self.local_rules.is_some() { self.local_pause(); }
            let _ = self.tx.send(Event::Devices(list));
        }
    }

    /// Ends capture. Without a position the pointer goes back to where it
    /// left, on the edge, so the edge waits for it to move off first.
    fn release(&mut self, to: Option<(i32, i32)>) -> Effect {
        match self.captured.take() {
            Some(c) => {
                // Events still waiting for their device go now, unknown.
                let mut out = Vec::new();
                if self.local_rules.as_ref().is_some_and(|local| local.enabled) {
                    self.raw.corr.finish(&mut out);
                } else {
                    self.raw.unregister(&mut out);
                }
                for ev in out {
                    let _ = self.tx.send(ev);
                }
                // SAFETY: our own window and timer.
                unsafe { KillTimer(self.hwnd, TIMER_DEVICES) };
                self.fwd.clear();
                if let Some(local) = self.local_rules.as_mut() { local.gate.close(); }
                if self.local_rules.as_ref().is_some_and(|local| local.enabled && !local.paused) {
                    self.local_fence(Destination::Local);
                }
                self.heading.reset();
                if to.is_none() {
                    self.gate.disarm();
                }
                Effect::Unpark(to.unwrap_or(c.entry))
            }
            None => Effect::None,
        }
    }

    /// Ends capture on our own initiative, telling the daemon.
    fn end(&mut self) -> Effect {
        let Some(id) = self.captured.map(|c| c.id) else { return Effect::None };
        let effect = self.release(None);
        let _ = self.tx.send(Event::Deactivated { id });
        effect
    }

    /// Real input happened here; tell the daemon if a peer is in control.
    fn local_input(&mut self) {
        if self.local_gate.check(now_ms(), self.controlled.get()) {
            let _ = self.tx.send(Event::LocalInput);
        }
    }

    fn activate(&mut self, side: Side, x: i32, y: i32) -> (bool, Effect) {
        self.local_fence(Destination::Remote);
        self.activate_now(side, x, y)
    }

    fn activate_now(&mut self, side: Side, x: i32, y: i32) -> (bool, Effect) {
        let pos = self.space.to_logical(x, y);
        let id = self.next_id.wrapping_add(1);
        // Key-ups hidden by the lock screen, Ctrl+Alt+Del or a UAC prompt
        // would leave stale entries; start from what is really held. The
        // modifiers among them go first, for the daemon's modifier-to-cross.
        let held = async_held();
        let _ = self.tx.send(Event::Modifiers(Mods::from_keys(Mods::KEYS.into_iter().filter(|&k| held.has(k)))));
        if self.tx.send(Event::Activated { id, side, pos }).is_err() {
            return (false, Effect::None);
        }
        self.next_id = id;
        self.local = held;
        self.fwd.clear();
        self.watchdog.reset();
        self.heading.reset();
        let park = self.space.park_point(x, y);
        let entry = self.space.to_phys(pos);
        self.captured = Some(Captured { id, park, entry });
        // SAFETY: posting to our own window.
        unsafe { PostMessageW(self.hwnd, WM_APP_DEVICES, 0, 0) };
        (true, Effect::Park(park))
    }

    /// Returns whether to swallow the event, and what to do afterwards.
    fn on_mouse(&mut self, msg: u32, info: &MSLLHOOKSTRUCT) -> (bool, Effect) {
        if info.dwExtraInfo == OUR_INPUT {
            return (false, Effect::None);
        }
        let synthetic = is_synthetic_mouse(info.flags, info.dwExtraInfo);
        let injected = info.flags & (LLMHF_INJECTED | LLMHF_LOWER_IL_INJECTED) != 0;
        let hi = (info.mouseData >> 16) as u16;
        let button = match msg {
            WM_LBUTTONDOWN | WM_LBUTTONUP => Some(BTN_LEFT),
            WM_RBUTTONDOWN | WM_RBUTTONUP => Some(BTN_RIGHT),
            WM_MBUTTONDOWN | WM_MBUTTONUP => Some(BTN_MIDDLE),
            WM_XBUTTONDOWN | WM_XBUTTONUP => match hi {
                XBUTTON1 => Some(BTN_SIDE),
                XBUTTON2 => Some(BTN_EXTRA),
                _ => None,
            },
            _ => None,
        };
        let down = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN);
        let (x, y) = (info.pt.x, info.pt.y);

        let Some(cap) = self.captured else {
            if let Some(b) = button {
                self.local.set(b, down);
            }
            if !injected {
                self.local_input();
            }
            if msg != WM_MOUSEMOVE {
                return (false, Effect::None);
            }
            if synthetic {
                // Pen, touch or software: never a crossing.
                self.heading.reset();
                return (false, Effect::None);
            }
            let (dx, dy) = self.heading.update(x, y);
            // Only the armed segments: the parts of edges that face a peer.
            let hit = self.space.edge_hit(&self.edges, x, y, dx, dy);
            // Only asked at an edge: a drag must not cross machines.
            let dragging = hit.is_some() && async_any_button();
            return match self.gate.check(hit, dragging) {
                Some(side) => self.activate(side, x, y),
                None => (false, Effect::None),
            };
        };

        if synthetic {
            // Pen, touch or another program: not ours to forward, and it
            // must not move the parked cursor either.
            return (true, Effect::None);
        }
        let effect = match msg {
            WM_MOUSEMOVE => {
                let (dx, dy) = (x - cap.park.0, y - cap.park.1);
                if dx == 0 && dy == 0 {
                    Effect::None
                } else {
                    let (dx, dy) = self.space.delta_to_logical(dx, dy);
                    self.send(Event::Motion { dx, dy, device: None })
                }
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let (dx, dy) = wheel_to_wire(hi as i16, msg == WM_MOUSEHWHEEL);
                self.send(Event::ScrollDiscrete { dx, dy, device: None })
            }
            _ => match button {
                // A button held before the pointer left: release it here.
                Some(b) if !down && self.local.has(b) => {
                    self.local.set(b, false);
                    return (false, Effect::None);
                }
                Some(b) => self.send(Event::Button { button: b, down, device: None }),
                None => Effect::None,
            },
        };
        (true, effect)
    }

    fn on_key(&mut self, info: &KBDLLHOOKSTRUCT) -> (bool, Effect) {
        if info.dwExtraInfo == OUR_INPUT {
            return (false, Effect::None);
        }
        let injected = info.flags & LLKHF_INJECTED != 0;
        let down = info.flags & LLKHF_UP == 0;
        let code = hook_key_to_evdev(info.vkCode, info.scanCode, info.flags & LLKHF_EXTENDED != 0);
        if self.captured.is_none() {
            if let Some(code) = code
                && let Some(local) = self.local_rules.as_mut()
            {
                if self.tx.is_closed() { local.admission.fail(); }
                let swallow = local.hook(code, down, Instant::now());
                // SAFETY: deferred wake only; no mapper, allocation or injection in this callback.
                unsafe { PostMessageW(self.hwnd, WM_APP_LOCAL, 0, 0) };
                if !swallow { self.local.set(code, down); }
                return (swallow, Effect::None);
            }
            if let Some(c) = code {
                self.local.set(c, down);
            }
            if !injected {
                self.local_input();
            }
            return (false, Effect::None);
        }
        if injected {
            // Another program's keystrokes: leave them to this machine.
            return (false, Effect::None);
        }
        let Some(code) = code else {
            // Unmapped, or AltGr's fake Ctrl: keep it from both machines.
            return (true, Effect::None);
        };
        if let Some(swallow) = self.local_rules.as_mut().and_then(|local| local.shortcut(code, down)) {
            // SAFETY: shortcut cleanup and daemon notification run outside the hook.
            unsafe { PostMessageW(self.hwnd, WM_APP_LOCAL, 0, 0) };
            return (swallow, Effect::None);
        }
        if self.local_rules.as_mut().is_some_and(|local| local.suppress_owned(code, down)) {
            return (true, Effect::None);
        }
        if self.local.has(code) || self.local_rules.as_ref().is_some_and(|local| local.os_owned(code)) {
            // Held since before the pointer left: auto-repeat stays
            // swallowed, the release goes to this machine. The daemon
            // counted a modifier as held (`Event::Modifiers` at activation),
            // so it hears that it no longer is.
            if down {
                return (true, Effect::None);
            }
            self.local.set(code, false);
            if let Some(local) = self.local_rules.as_mut() { local.os_release(code); }
            if Mods::is_modifier(code) {
                let _ = self.tx.send(Event::LocalKeyUp { code });
            }
            return (false, Effect::None);
        }
        if self.local_rules.as_mut().is_some_and(|local| local.remote_key(code, down)) {
            return (true, Effect::None);
        }
        // Auto-repeat (Caps Lock's too) is dropped; the peer repeats.
        if !self.fwd.forward(code, down) {
            return (true, Effect::None);
        }
        (true, self.send(Event::Key { code, down, device: None }))
    }

    fn command(&mut self, cmd: CaptureCmd) -> Effect {
        match cmd {
            CaptureCmd::LocalRules { rules, inactive } => {
                if rules.as_ref().is_none_or(|rules| rules.count == 0 || !Local::supported(rules)) {
                    self.local_pause();
                    if let Some(local) = self.local_rules.take() { self.local_recovery = Some(local.retire()); }
                    unsafe { KillTimer(self.hwnd, TIMER_LOCAL) };
                    if self.captured.is_none() {
                        let mut out = Vec::new();
                        self.raw.unregister(&mut out);
                        self.send_all(out);
                    }
                    let problem = rules.as_ref().filter(|rules| rules.count > 0).map(|_| {
                        "Local key rules inactive: unsupported native output; EXPERIMENTAL, Windows UNTESTED".into()
                    });
                    let _ = self.tx.send(Event::Status { part: "local-key-rules", problem });
                    return Effect::None;
                }
                if self.local_recovery.as_ref().is_some_and(|recovery| !recovery.finished()) {
                    if let Some(recovery) = &self.local_recovery { recovery.retry(); }
                    self.local_status("inactive: pending releases from previous local rules; retry after returning to the normal desktop");
                    return Effect::None;
                }
                self.local_recovery = None;
                if rules.is_some() && !self.session_notifications {
                    // SAFETY: retry only after a failed registration on our own window.
                    self.session_notifications = unsafe { WTSRegisterSessionNotification(self.hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;
                }
                if rules.is_some() && !self.session_notifications {
                    self.local_pause();
                    self.local_status("inactive: Windows session notifications unavailable");
                    return Effect::None;
                }
                if self.local_rules.is_none() && rules.is_some() {
                    match Local::start(self.hwnd) {
                        Ok(local) => self.local_rules = Some(local),
                        Err(error) => {
                            self.local_status(&format!("inactive: worker could not start: {error}"));
                            return Effect::None;
                        }
                    }
                }
                let rules = rules.unwrap_or_default();
                let supported = Local::supported(&rules);
                let active = self.session_active && self.awake && desktop_active();
                if let Some(local) = self.local_rules.as_mut() {
                    let held = async_held();
                    local.baseline((0..one_mk_proto::keynames::CODE_LIMIT).filter(|&code| held.has(code)));
                    local.gate.close();
                    local.enabled = supported && rules.count > 0;
                    local.paused = !active;
                    local.inactive = if !supported { "only physical scancode outputs and layers are supported".into() }
                        else { inactive.unwrap_or_else(|| "preference off or no local rules".into()) };
                    local.snapshots.replace(if supported { rules } else { Arc::default() });
                    self.raw.register(self.hwnd);
                    if active {
                        self.local_fence(if self.captured.is_some() { Destination::Remote } else { Destination::Local });
                    } else {
                        self.local_pause();
                    }
                    if !supported { self.local_status("inactive: only physical scancode outputs and layers are supported"); }
                } else {
                    self.local_status(&format!("inactive: {}", inactive.unwrap_or_else(|| "preference off".into())));
                }
                Effect::None
            }
            CaptureCmd::SetEdges(edges) => {
                self.edges = edges;
                Effect::None
            }
            CaptureCmd::Release { id, pos } => {
                if !self.captured.is_some_and(|c| c.id == id) {
                    return Effect::None; // stale
                }
                let to = pos.map(|p| self.space.to_phys(p));
                self.release(to)
            }
            CaptureCmd::Activate { side } => {
                if self.captured.is_some() {
                    return Effect::None;
                }
                let mut p = POINT { x: 0, y: 0 };
                // SAFETY: valid out-pointer.
                if unsafe { GetCursorPos(&mut p) } == 0 {
                    return Effect::None;
                }
                self.activate(side, p.x, p.y).1
            }
        }
    }

    /// Whether the foreground window belongs to an elevated program, whose
    /// input our low-level hooks can't see.
    fn foreground_elevated(&mut self) -> bool {
        if self.self_elevated {
            return false;
        }
        // SAFETY: plain call.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return false;
        }
        if let Some((h, elevated)) = self.elevated_cache
            && h == hwnd as usize
        {
            return elevated;
        }
        let mut pid = 0u32;
        // SAFETY: valid window handle (or a stale one, which just fails).
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let elevated = pid != 0 && process_elevated(pid).unwrap_or(false);
        self.elevated_cache = Some((hwnd as usize, elevated));
        elevated
    }

    /// Runs every 200 ms while captured.
    fn guard_tick(&mut self, cursor: Option<(i32, i32)>) -> Option<Guard> {
        let cap = self.captured?;
        if self.foreground_elevated() {
            tracing::warn!("an elevated window is in front; giving the pointer back");
            let effect = self.end();
            let _ = self.tx.send(Event::Error(
                "A program running as administrator is in front on this PC, and Windows hides its \
                 keyboard and mouse from one-mk, so the pointer was brought back here. Switch to \
                 another window (or run one-mk as administrator) to control your other computers."
                    .into(),
            ));
            return Some(Guard::Ended(effect));
        }
        // Something else (an app, a remote tool) may have moved the cursor;
        // if it keeps happening our hooks are probably gone.
        let moved = cursor.is_some_and(|c| c != cap.park);
        match self.watchdog.tick(moved) {
            Watch::Fine => None,
            Watch::Rewarp => Some(Guard::Rewarp(cap.park)),
            Watch::Reinstall => {
                tracing::warn!("the parked cursor keeps moving; reinstalling the input hooks");
                Some(Guard::Reinstall(self.hooks, cap.park))
            }
            Watch::GiveUp => {
                tracing::warn!("lost the input hooks; giving the pointer back");
                let effect = self.end();
                let _ = self.tx.send(Event::Error(
                    "Windows stopped passing this PC's mouse and keyboard to one-mk (this can happen \
                     when the computer is very busy), so the pointer was brought back here."
                        .into(),
                ));
                Some(Guard::GaveUp(self.hooks, effect))
            }
        }
    }

    /// Re-reads the monitors and tells the daemon if they changed.
    fn refresh_space(&mut self) -> Effect {
        let space = match read_space() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("re-reading monitors: {e:#}");
                return Effect::None;
            }
        };
        if space == self.space {
            return Effect::None;
        }
        tracing::info!(screens = ?space.phys().screens, scale = space.scale(), "monitors changed");
        if let Ok(mut shared) = self.shared.lock() {
            *shared = space.clone();
        }
        self.space = space;
        let effect = self.send(Event::Desktop(self.space.logical().clone()));
        // The park point may now be off-screen; start over cleanly.
        if self.captured.is_some() { self.end() } else { effect }
    }
}

/// Replaces the hooks (they may have been removed by Windows already).
fn reinstall_hooks(old: Hooks) -> bool {
    old.remove();
    match Hooks::install() {
        Ok(h) => {
            with_state(|s| s.hooks = h);
            true
        }
        Err(err) => {
            tracing::warn!(err, "reinstalling input hooks failed");
            with_state(|s| s.hooks = Hooks { mouse: std::ptr::null_mut(), keyboard: std::ptr::null_mut() });
            false
        }
    }
}

pub(crate) unsafe extern "system" fn desktop_changed(
    _hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK, event: u32, _hwnd: HWND,
    _object: i32, _child: i32, _thread: u32, _time: u32,
) {
    if !matches!(event, 3 | 0x20) { return; }
    with_state(|state| {
        if let Some(recovery) = &state.local_recovery { recovery.retry(); }
        if state.local_rules.is_none() { return; }
        if event == 3 && !state.foreground_elevated() {
            let held = async_held();
            if let Some(local) = state.local_rules.as_mut() && local.paused {
                local.retry_releases();
                local.baseline((0..one_mk_proto::keynames::CODE_LIMIT).filter(|&code| held.has(code)));
            }
            return;
        }
        if let Some(local) = state.local_rules.as_mut() { local.retry_releases(); local.paused = true; }
        // SAFETY: our window; release and suspension run outside the notification callback.
        unsafe { PostMessageW(state.hwnd, WM_APP_LOCAL_DESKTOP, 0, 0) };
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_LOCAL | WM_TIMER if msg == WM_APP_LOCAL || wparam == TIMER_LOCAL => {
            if let Some(effect) = with_state(State::local_pump) { apply(effect, hwnd); }
            0
        }
        WM_APP_LOCAL_DESKTOP | 0x218 if msg == WM_APP_LOCAL_DESKTOP || matches!(wparam, 4 | 7 | 18) => {
            if let Some(effect) = with_state(|state| {
                if msg == 0x218 && wparam == 4 { state.awake = false; }
                if msg == 0x218 && matches!(wparam, 7 | 18) { state.awake = true; }
                if let Some(recovery) = &state.local_recovery { recovery.retry(); }
                if state.local_rules.is_none() { return Effect::None; }
                state.local_pause();
                state.end()
            }) { apply(effect, hwnd); }
            1
        }
        WM_APP_CMD => {
            let effects = with_state(|s| {
                let mut out = Vec::new();
                while let Ok(cmd) = s.cmds.try_recv() {
                    out.push(s.command(cmd));
                }
                out
            });
            for e in effects.into_iter().flatten() {
                apply(e, hwnd);
            }
            0
        }
        WM_APP_OVERLAY => {
            let park = with_state(|s| s.captured.map(|c| c.park)).flatten();
            // SAFETY: plain Win32 calls on our own window.
            unsafe {
                match park {
                    Some((x, y)) if wparam == 1 => {
                        let h = OVERLAY / 2;
                        SetWindowPos(hwnd, HWND_TOPMOST, x - h, y - h, OVERLAY, OVERLAY, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                        // Re-warp so Windows re-evaluates the cursor shape
                        // over the window that is now under it.
                        SetCursorPos(x, y);
                        SetTimer(hwnd, TIMER_GUARD, 200, None);
                    }
                    Some(_) => {}
                    None => {
                        KillTimer(hwnd, TIMER_GUARD);
                        ShowWindow(hwnd, SW_HIDE);
                    }
                }
            }
            0
        }
        WM_TIMER if wparam == TIMER_GUARD => {
            // A moved cursor is put back, or the next delta would be measured
            // from the wrong place and the overlay would no longer hide it.
            let mut p = POINT { x: 0, y: 0 };
            // SAFETY: valid out-pointer.
            let cursor = (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y));
            let rewarp = |(x, y): (i32, i32)| {
                // SAFETY: plain call.
                unsafe { SetCursorPos(x, y) };
            };
            match with_state(|s| s.guard_tick(cursor)).flatten() {
                None => {}
                Some(Guard::Rewarp(park)) => rewarp(park),
                Some(Guard::Reinstall(hooks, park)) => {
                    if reinstall_hooks(hooks) {
                        rewarp(park);
                    } else if let Some(e) = with_state(|s| {
                        let _ = s.tx.send(Event::Error(
                            "Windows no longer lets one-mk watch this PC's mouse and keyboard, so it \
                             can't control other computers until it is restarted."
                                .into(),
                        ));
                        s.end()
                    }) {
                        apply(e, hwnd);
                    }
                }
                Some(Guard::GaveUp(hooks, effect)) => {
                    apply(effect, hwnd);
                    reinstall_hooks(hooks);
                }
                Some(Guard::Ended(effect)) => apply(effect, hwnd),
            }
            0
        }
        WM_APP_DEVICES => {
            with_state(|s| {
                if s.captured.is_some() {
                    s.raw.register(s.hwnd);
                    s.devices_changed();
                }
            });
            0
        }
        WM_INPUT => {
            let effect = with_state(|s| {
                let mut out = Vec::new();
                s.raw.on_input(lparam, &mut out);
                s.send_all(out)
            });
            if let Some(e) = effect {
                apply(e, hwnd);
            }
            // SAFETY: default handling (it frees the input's data).
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_INPUT_DEVICE_CHANGE => {
            with_state(State::devices_changed);
            0
        }
        WM_TIMER if wparam == TIMER_DEVICES => {
            let effect = with_state(|s| {
                let mut out = Vec::new();
                s.raw.corr.tick(now_ms(), &mut out);
                s.send_all(out)
            });
            if let Some(e) = effect {
                apply(e, hwnd);
            }
            0
        }
        WM_TIMER if wparam == TIMER_DISPLAY => {
            // SAFETY: our own timer.
            unsafe { KillTimer(hwnd, TIMER_DISPLAY) };
            if let Some(e) = with_state(State::refresh_space) {
                apply(e, hwnd);
            }
            0
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
            // SAFETY: our own window; restarting the timer debounces.
            unsafe { SetTimer(hwnd, TIMER_DISPLAY, 250, None) };
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_WTSSESSION_CHANGE => {
            with_state(|state| {
                if wparam as u32 == WTS_SESSION_UNLOCK {
                    state.session_active = true;
                    if let Some(recovery) = &state.local_recovery { recovery.retry(); }
                    let held = async_held();
                    if let Some(local) = state.local_rules.as_mut() {
                        local.retry_releases();
                        local.baseline((0..one_mk_proto::keynames::CODE_LIMIT).filter(|&code| held.has(code)));
                    }
                }
                if matches!(wparam as u32, WTS_SESSION_LOCK | WTS_SESSION_LOGOFF | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_CONNECT) {
                    state.session_active = false;
                }
            });
            match wparam as u32 {
                WTS_SESSION_LOCK => with_state(|s| s.tx.send(Event::ScreenLocked(true))),
                WTS_SESSION_UNLOCK => with_state(|s| s.tx.send(Event::ScreenLocked(false))),
                _ => None,
            };
            if matches!(wparam as u32, WTS_SESSION_LOCK | WTS_SESSION_LOGOFF | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_CONNECT) {
                // Hooks see nothing on the lock screen; hand the pointer
                // back so it is not stuck hidden when the user returns.
                let effect = with_state(|state| { state.local_pause(); state.end() });
                if let Some(e) = effect {
                    apply(e, hwnd);
                }
            }
            0
        }
        WM_SETCURSOR => {
            // SAFETY: hiding the cursor while it is over our window.
            unsafe { SetCursor(std::ptr::null_mut()) };
            1
        }
        WM_DESTROY => {
            // SAFETY: plain call.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: default handling for everything else (WM_CLOSE destroys).
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// --------------------------------------------------------------- injection

fn inject_thread(
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Inject>,
    shared: Arc<Mutex<Space>>,
    controlled: Arc<Controlled>,
) {
    set_thread_dpi_awareness();
    // A small runtime of its own, for the key repeat timer.
    let rt = match tokio::runtime::Builder::new_current_thread().enable_time().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = tx.send(Event::Error(format!("Other computers can't control this one: {e}")));
            return;
        }
    };
    tracing::info!("input injection ready");
    rt.block_on(inject_loop(tx, rx, shared, controlled));
}

/// The user's key repeat delay and rate (Control Panel > Keyboard).
fn repeat_setting() -> Option<(Duration, Duration)> {
    let (mut delay, mut speed) = (1u32, 31u32);
    // SAFETY: both actions write one u32 to the valid out-pointer.
    unsafe {
        SystemParametersInfoW(SPI_GETKEYBOARDDELAY, 0, (&mut delay as *mut u32).cast(), 0);
        SystemParametersInfoW(SPI_GETKEYBOARDSPEED, 0, (&mut speed as *mut u32).cast(), 0);
    }
    Some(repeat_timing(delay, speed))
}

async fn inject_loop(
    tx: mpsc::UnboundedSender<Event>,
    mut rx: mpsc::UnboundedReceiver<Inject>,
    shared: Arc<Mutex<Space>>,
    controlled: Arc<Controlled>,
) {
    let mut scroll = SmoothScroll::default();
    let mut rel = (0.0, 0.0);
    let mut inputs: Vec<INPUT> = Vec::new();
    let mut batch: Vec<Inject> = Vec::new();
    // SendInput doesn't auto-repeat, and senders drop their repeats.
    let mut repeat = KeyRepeat::new(repeat_setting());
    let mut failing = false;
    loop {
        let deadline = repeat.deadline();
        let wake = tokio::time::Instant::from_std(deadline.unwrap_or_else(|| std::time::Instant::now() + Duration::from_secs(3600)));
        tokio::select! {
            msg = rx.recv() => {
                let Some(first) = msg else { return };
                // Everything already queued goes out in one SendInput call,
                // without the moves that later ones make stale.
                drain_coalesced(first, &mut rx, &mut batch);
                if controlled.apply(&mut batch) {
                    repeat.clear();
                }
                let now = std::time::Instant::now();
                let Ok(space) = shared.lock() else { return };
                for m in batch.drain(..) {
                    if let Inject::Key { code, down } | Inject::CharacterKey { code, down, .. } = m {
                        if down && is_repeatable(code) {
                            repeat.set_timing(repeat_setting());
                        }
                        if !repeat.key(code, down, now) {
                            continue;
                        }
                    }
                    build(&mut inputs, m, &space, &mut scroll, &mut rel);
                }
            }
            _ = tokio::time::sleep_until(wake), if deadline.is_some() => {
                let Some(code) = repeat.poll(std::time::Instant::now()) else { continue };
                let Ok(space) = shared.lock() else { return };
                build(&mut inputs, Inject::Key { code, down: true }, &space, &mut scroll, &mut rel);
            }
        }
        send_inputs(&mut inputs, &tx, &mut failing);
    }
}

fn send_inputs(inputs: &mut Vec<INPUT>, tx: &mpsc::UnboundedSender<Event>, failing: &mut bool) {
    if inputs.is_empty() {
        return;
    }
    // SAFETY: `inputs` is a valid slice of initialized INPUTs.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if (sent as usize) < inputs.len() {
        if !*failing {
            // SAFETY: plain call.
            let err = unsafe { GetLastError() };
            tracing::warn!(sent, total = inputs.len(), err, "SendInput failed");
            let _ = tx.send(Event::Error(
                "Windows blocked input from another computer. This happens on the lock screen, \
                 on administrator (UAC) prompts, and in programs running as administrator."
                    .into(),
            ));
            *failing = true;
        }
    } else {
        *failing = false;
    }
    inputs.clear();
}

fn mouse(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: OUR_INPUT },
        },
    }
}

pub(crate) fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: OUR_INPUT } },
    }
}

/// `rel` keeps the fraction of a physical pixel that relative moves (in
/// logical pixels) did not use yet, so scaling never drifts.
fn build(out: &mut Vec<INPUT>, m: Inject, space: &Space, scroll: &mut SmoothScroll, rel: &mut (f64, f64)) {
    const ABS_MOVE: u32 = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    match m {
        Inject::MoveTo(p) => {
            let (x, y) = space.to_phys(p);
            let (nx, ny) = space.normalize(x, y);
            out.push(mouse(nx, ny, 0, ABS_MOVE));
        }
        // A plain relative move: what games reading raw input see. (Windows
        // applies the pointer speed setting to it for the desktop cursor.)
        Inject::MoveBy { dx, dy } => {
            let fx = rel.0 + f64::from(dx) * space.scale();
            let fy = rel.1 + f64::from(dy) * space.scale();
            let (px, py) = (fx.round(), fy.round());
            *rel = (fx - px, fy - py);
            if px != 0.0 || py != 0.0 {
                out.push(mouse(px as i32, py as i32, 0, MOUSEEVENTF_MOVE));
            }
        }
        Inject::Button { button, down } => {
            let (flags, data) = match (button, down) {
                (BTN_LEFT, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (BTN_LEFT, false) => (MOUSEEVENTF_LEFTUP, 0),
                (BTN_RIGHT, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (BTN_RIGHT, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (BTN_MIDDLE, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (BTN_MIDDLE, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (BTN_SIDE, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1)),
                (BTN_SIDE, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON1)),
                (BTN_EXTRA, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON2)),
                (BTN_EXTRA, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON2)),
                _ => {
                    tracing::debug!(button, "no Windows equivalent for mouse button");
                    return;
                }
            };
            out.push(mouse(0, 0, data, flags));
        }
        Inject::ScrollDiscrete { dx, dy } => {
            let (h, v) = wire_to_wheel(dx, dy);
            push_wheel(out, h, v);
        }
        Inject::Scroll { dx, dy } => {
            let (h, v) = scroll.push(dx, dy);
            push_wheel(out, h, v);
        }
        Inject::ScrollStop => scroll.stop(),
        // Handled by the injection loop.
        Inject::Controlled(_) => {}
        Inject::Text { .. } => tracing::warn!("text unsupported; sender must use physical fallback"),
        Inject::Key { code, down } | Inject::CharacterKey { code, down, .. } => {
            let up = if down { 0 } else { KEYEVENTF_KEYUP };
            match evdev_to_win(code) {
                Some(WinKey::Scan { scan, extended }) => {
                    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                    out.push(key(0, scan, KEYEVENTF_SCANCODE | ext | up));
                }
                Some(WinKey::Vk { vk, scan, extended }) => {
                    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                    out.push(key(vk, scan, ext | up));
                }
                None => tracing::debug!(code, "no Windows equivalent for key"),
            }
        }
    }
}

/// Wheel amounts are signed but travel in the unsigned `mouseData`.
fn push_wheel(out: &mut Vec<INPUT>, h: i32, v: i32) {
    if v != 0 {
        out.push(mouse(0, 0, v as u32, MOUSEEVENTF_WHEEL));
    }
    if h != 0 {
        out.push(mouse(0, 0, h as u32, MOUSEEVENTF_HWHEEL));
    }
}
