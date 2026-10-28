//! Windows backend: low-level hooks for capture, `SendInput` for injection.
//!
//! - Capture runs on its own thread with a Win32 message loop. `WH_MOUSE_LL`
//!   and `WH_KEYBOARD_LL` callbacks run on that thread while it waits for
//!   messages. When the pointer touches an armed outer edge of the desktop,
//!   the hooks start swallowing local input: the cursor is parked in the
//!   middle of a screen under a tiny invisible window that hides it, and
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
//! (once per activation) warp the cursor. Everything else is posted to the
//! window and done by the message loop.
//!
//! TODO: run as a SYSTEM service (with a helper in the user's session) so
//! that UAC prompts, the secure desktop (Ctrl+Alt+Del) and the login screen
//! can be controlled. A normal process can't see or inject input there, and
//! can't inject into windows of elevated programs (UIPI).

use crate::win_logic::{
    BTN_EXTRA, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_SIDE, SmoothScroll, Space, WinKey, evdev_to_win,
    hook_key_to_evdev, wheel_to_wire, wire_to_wheel,
};
use crate::{Backend, CaptureCmd, Event, Inject};
use anyhow::{Result, bail};
use one_mk_proto::{Rect, Side};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::RemoteDesktop::{NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    HC_ACTION, HHOOK, HWND_TOPMOST, KBDLLHOOKSTRUCT, KillTimer, LLKHF_EXTENDED, LLKHF_UP, LLMHF_INJECTED,
    LWA_ALPHA, MONITORINFOF_PRIMARY, MSG, MSLLHOOKSTRUCT, PostMessageW, PostQuitMessage, RegisterClassW,
    SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetCursor, SetCursorPos, SetLayeredWindowAttributes, SetTimer,
    SetWindowPos, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP,
    WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT,
    WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
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

    let (tx, sh) = (ev_tx.clone(), shared.clone());
    std::thread::Builder::new().name("one-mk-capture".into()).spawn(move || {
        if let Err(e) = capture_thread(tx.clone(), cap_rx, sh) {
            tracing::warn!("input capture unavailable: {e:#}");
            let _ = tx.send(Event::Error(format!("Can't share this computer's mouse and keyboard with others: {e:#}")));
        }
    })?;
    std::thread::Builder::new().name("one-mk-inject".into()).spawn(move || inject_thread(ev_tx, inj_rx, shared))?;
    Ok(Backend { events: ev_rx, capture: cap_tx, inject: inj_tx })
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
    sides: Vec<Side>,
    cmds: std_mpsc::Receiver<CaptureCmd>,
    captured: Option<Captured>,
    /// evdev keys and buttons that went down locally (not forwarded); their
    /// release must reach this machine even while captured, or they would
    /// stay stuck down here.
    local: Held,
}

#[derive(Clone, Copy)]
struct Captured {
    /// Where the cursor is kept while captured.
    park: (i32, i32),
    /// Where it hit the edge, for `Release { pos: None }`.
    entry: (i32, i32),
}

/// A set of evdev codes below 0x200 (all keys and mouse buttons we map).
#[derive(Default)]
struct Held([u64; 8]);

impl Held {
    fn set(&mut self, code: u32, down: bool) {
        if let Some(word) = self.0.get_mut(code as usize / 64) {
            let bit = 1u64 << (code % 64);
            if down { *word |= bit } else { *word &= !bit }
        }
    }
    fn has(&self, code: u32) -> bool {
        self.0.get(code as usize / 64).is_some_and(|w| w & (1u64 << (code % 64)) != 0)
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
        // Lets us give the pointer back when the session is locked.
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        hwnd
    };

    let (cmd_tx, cmd_rx) = std_mpsc::channel();
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            tx,
            hwnd,
            shared,
            space,
            sides: Vec::new(),
            cmds: cmd_rx,
            captured: None,
            local: Held::default(),
        })
    });

    // SAFETY: the hook procedures are valid for the life of the thread; a
    // null module handle is correct for low-level hooks in our own exe.
    let (mouse, keyboard) = unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hinst, 0);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinst, 0);
        (mouse, keyboard)
    };
    let unhook = |h: HHOOK| {
        if !h.is_null() {
            // SAFETY: a hook handle we installed.
            unsafe { UnhookWindowsHookEx(h) };
        }
    };
    if mouse.is_null() || keyboard.is_null() {
        // SAFETY: plain call.
        let err = unsafe { GetLastError() };
        unhook(mouse);
        unhook(keyboard);
        // SAFETY: our window.
        unsafe { DestroyWindow(hwnd) };
        bail!("Windows refused to let one-mk watch the mouse and keyboard (error {err}); security software may be blocking it");
    }
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

    unhook(mouse);
    unhook(keyboard);
    // If we exit while captured, give the user their cursor back.
    let restore = STATE.with(|s| s.borrow_mut().take().and_then(|st| st.captured.map(|c| c.entry)));
    if let Some((x, y)) = restore {
        // SAFETY: plain call.
        unsafe { SetCursorPos(x, y) };
    }
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
                SetCursorPos(x, y);
                PostMessageW(hwnd, WM_APP_OVERLAY, 1, 0);
            }
            Effect::Unpark((x, y)) => {
                PostMessageW(hwnd, WM_APP_OVERLAY, 0, 0);
                SetCursorPos(x, y);
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

impl State {
    /// Sends to the daemon; if it is gone while captured, gives the pointer back.
    fn send(&mut self, ev: Event) -> Effect {
        if self.tx.send(ev).is_err() {
            return self.release(None);
        }
        Effect::None
    }

    fn release(&mut self, to: Option<(i32, i32)>) -> Effect {
        match self.captured.take() {
            Some(c) => Effect::Unpark(to.unwrap_or(c.entry)),
            None => Effect::None,
        }
    }

    /// Returns whether to swallow the event, and what to do afterwards.
    fn on_mouse(&mut self, msg: u32, info: &MSLLHOOKSTRUCT) -> (bool, Effect) {
        if info.dwExtraInfo == OUR_INPUT {
            return (false, Effect::None);
        }
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
            if msg == WM_MOUSEMOVE
                && let Some(side) = self.space.edge_hit(&self.sides, x, y)
            {
                let pos = self.space.to_logical(x, y);
                if self.tx.send(Event::Activated { side, pos }).is_ok() {
                    let park = self.space.park_point(x, y);
                    let entry = self.space.to_phys(pos);
                    self.captured = Some(Captured { park, entry });
                    return (true, Effect::Park(park));
                }
            }
            return (false, Effect::None);
        };

        let effect = match msg {
            WM_MOUSEMOVE => {
                // Absolute moves injected by other software (tablets, remote
                // tools) are not deltas; drop them.
                let (dx, dy) = (x - cap.park.0, y - cap.park.1);
                if info.flags & LLMHF_INJECTED != 0 || (dx == 0 && dy == 0) {
                    Effect::None
                } else {
                    let (dx, dy) = self.space.delta_to_logical(dx, dy);
                    self.send(Event::Motion { dx, dy })
                }
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let (dx, dy) = wheel_to_wire(hi as i16, msg == WM_MOUSEHWHEEL);
                self.send(Event::ScrollDiscrete { dx, dy })
            }
            _ => match button {
                // A button held before the pointer left: release it here.
                Some(b) if !down && self.local.has(b) => {
                    self.local.set(b, false);
                    return (false, Effect::None);
                }
                Some(b) => self.send(Event::Button { button: b, down }),
                None => Effect::None,
            },
        };
        (true, effect)
    }

    fn on_key(&mut self, info: &KBDLLHOOKSTRUCT) -> (bool, Effect) {
        if info.dwExtraInfo == OUR_INPUT {
            return (false, Effect::None);
        }
        let down = info.flags & LLKHF_UP == 0;
        let code = hook_key_to_evdev(info.vkCode, info.scanCode, info.flags & LLKHF_EXTENDED != 0);
        if self.captured.is_none() {
            if let Some(c) = code {
                self.local.set(c, down);
            }
            return (false, Effect::None);
        }
        let Some(code) = code else {
            // Unmapped, or AltGr's fake Ctrl: keep it from both machines.
            return (true, Effect::None);
        };
        if self.local.has(code) {
            // Held since before the pointer left: auto-repeat stays
            // swallowed, the release goes to this machine.
            if down {
                return (true, Effect::None);
            }
            self.local.set(code, false);
            return (false, Effect::None);
        }
        (true, self.send(Event::Key { code, down }))
    }

    fn command(&mut self, cmd: CaptureCmd) -> Effect {
        match cmd {
            CaptureCmd::SetSides(sides) => {
                self.sides = sides;
                Effect::None
            }
            CaptureCmd::Release { pos } => {
                let to = pos.map(|p| self.space.to_phys(p));
                self.release(to)
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
        let mut effect = self.send(Event::Desktop(self.space.logical().clone()));
        // The park point may now be off-screen; start over cleanly.
        if self.captured.is_some() {
            effect = self.release(None);
            let _ = self.tx.send(Event::Deactivated);
        }
        effect
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
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
            // Something else (an app, a remote tool) moved the cursor: put
            // it back, or the next delta would be measured from the wrong
            // place and the overlay would no longer hide it.
            if let Some(Some((x, y))) = with_state(|s| s.captured.map(|c| c.park)) {
                let mut p = POINT { x: 0, y: 0 };
                // SAFETY: valid out-pointer.
                if unsafe { GetCursorPos(&mut p) } != 0 && (p.x, p.y) != (x, y) {
                    // SAFETY: plain call.
                    unsafe { SetCursorPos(x, y) };
                }
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
            if matches!(wparam as u32, WTS_SESSION_LOCK | WTS_SESSION_LOGOFF | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_CONNECT) {
                // Hooks see nothing on the lock screen; hand the pointer
                // back so it is not stuck hidden when the user returns.
                let effect = with_state(|s| {
                    let e = s.release(None);
                    if matches!(e, Effect::Unpark(_)) {
                        let _ = s.tx.send(Event::Deactivated);
                    }
                    e
                });
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

fn inject_thread(tx: mpsc::UnboundedSender<Event>, mut rx: mpsc::UnboundedReceiver<Inject>, shared: Arc<Mutex<Space>>) {
    set_thread_dpi_awareness();
    tracing::info!("input injection ready");
    let mut scroll = SmoothScroll::default();
    let mut inputs: Vec<INPUT> = Vec::new();
    let mut failing = false;
    while let Some(first) = rx.blocking_recv() {
        // Apply everything already queued with one SendInput call.
        {
            let Ok(space) = shared.lock() else { return };
            let mut next = Some(first);
            while let Some(m) = next {
                build(&mut inputs, m, &space, &mut scroll);
                next = rx.try_recv().ok();
            }
        }
        if inputs.is_empty() {
            continue;
        }
        // SAFETY: `inputs` is a valid slice of initialized INPUTs.
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
        if (sent as usize) < inputs.len() {
            if !failing {
                // SAFETY: plain call.
                let err = unsafe { GetLastError() };
                tracing::warn!(sent, total = inputs.len(), err, "SendInput failed");
                let _ = tx.send(Event::Error(
                    "Windows blocked input from another computer. This happens on the lock screen, \
                     on administrator (UAC) prompts, and in programs running as administrator."
                        .into(),
                ));
                failing = true;
            }
        } else {
            failing = false;
        }
        inputs.clear();
    }
}

fn mouse(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: OUR_INPUT },
        },
    }
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: OUR_INPUT } },
    }
}

fn build(out: &mut Vec<INPUT>, m: Inject, space: &Space, scroll: &mut SmoothScroll) {
    const ABS_MOVE: u32 = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    match m {
        Inject::MoveTo(p) => {
            let (x, y) = space.to_phys(p);
            let (nx, ny) = space.normalize(x, y);
            let mv = mouse(nx, ny, 0, ABS_MOVE);
            // Consecutive moves in one batch: only the last one matters.
            // SAFETY: reading the `mi` arm of an INPUT_MOUSE input.
            match out.last_mut() {
                Some(last) if last.r#type == INPUT_MOUSE && unsafe { last.Anonymous.mi.dwFlags } == ABS_MOVE => {
                    *last = mv
                }
                _ => out.push(mv),
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
        Inject::Key { code, down } => {
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
