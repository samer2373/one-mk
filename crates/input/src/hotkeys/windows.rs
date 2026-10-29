//! Windows: `RegisterHotKey` on a thread of its own, which sleeps in its
//! message loop until a shortcut is pressed or the bindings change. The
//! shortcuts are unregistered when the thread ends (the `Thread` is
//! dropped), and Windows drops them anyway when the process exits.
//!
//! While the pointer is on another computer the capture hook swallows every
//! key before Windows looks for shortcuts, so these fire only at home.

use super::{Action, Availability, Bindings, Platform};
use crate::Event;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY, WM_QUIT,
};

const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;
/// "The bindings changed."
const WM_RELOAD: u32 = WM_APP + 1;

pub(super) struct Thread {
    id: u32,
    bindings: Arc<Mutex<Bindings>>,
}

impl Platform for Thread {
    fn set(&mut self, bindings: Bindings) {
        *self.bindings.lock().unwrap_or_else(|e| e.into_inner()) = bindings;
        // SAFETY: posting to a thread id is safe; it fails if the thread is gone.
        unsafe { PostThreadMessageW(self.id, WM_RELOAD, 0, 0) };
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        // SAFETY: as above.
        unsafe { PostThreadMessageW(self.id, WM_QUIT, 0, 0) };
    }
}

pub(super) fn start(bindings: Bindings, events: mpsc::UnboundedSender<Event>) -> Option<Thread> {
    let bindings = Arc::new(Mutex::new(bindings));
    let shared = bindings.clone();
    let (id_tx, id_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("one-mk-hotkeys".into()).spawn(move || {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: plain Win32 calls with valid pointers. Peeking creates this
        // thread's message queue, so messages posted from now on arrive.
        let id = unsafe {
            PeekMessageW(&mut msg, std::ptr::null_mut(), WM_APP, WM_APP, PM_NOREMOVE);
            GetCurrentThreadId()
        };
        let _ = id_tx.send(id);
        let mut registered = register(&shared, &events);
        // SAFETY: as above; `msg` is valid for every call.
        while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
            match msg.message {
                WM_HOTKEY => {
                    let action = registered.get(msg.wParam).copied().flatten();
                    if let Some(a) = action
                        && events.send(Event::Hotkey(a)).is_err()
                    {
                        break;
                    }
                }
                WM_RELOAD => {
                    unregister(&registered);
                    registered = register(&shared, &events);
                }
                _ => {}
            }
        }
        unregister(&registered);
    });
    if let Err(e) = spawned {
        tracing::warn!("keyboard shortcuts: no thread: {e}");
        return None;
    }
    let id = id_rx.recv().ok()?;
    Some(Thread { id, bindings })
}

/// Registers every binding; the result's index is the hotkey id (`None`:
/// not registered).
fn register(bindings: &Mutex<Bindings>, events: &mpsc::UnboundedSender<Event>) -> Vec<Option<Action>> {
    let bindings = bindings.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut out = Vec::new();
    let mut taken = Vec::new();
    for (i, (action, chord)) in bindings.iter().enumerate() {
        let Some(vk) = chord.vk() else {
            out.push(None);
            continue;
        };
        // SAFETY: a thread-wide hotkey (no window); `i` is a small id.
        let ok = unsafe { RegisterHotKey(std::ptr::null_mut(), i as i32, chord.mods.windows() | MOD_NOREPEAT, u32::from(vk)) };
        if ok == 0 {
            // SAFETY: reads this thread's last error.
            let err = unsafe { GetLastError() };
            tracing::debug!(chord = %chord, err, "RegisterHotKey failed");
            if err == ERROR_HOTKEY_ALREADY_REGISTERED {
                taken.push(chord.label());
            }
            out.push(None);
        } else {
            out.push(Some(action));
        }
    }
    if !bindings.is_empty() {
        let n = out.iter().flatten().count();
        let a = match (n, taken.is_empty()) {
            (0, _) => Availability::Unavailable(format!(
                "Windows refused them{}",
                if taken.is_empty() { String::new() } else { format!(" (another program uses {})", taken.join(", ")) }
            )),
            (_, true) => Availability::Active("through Windows".into()),
            (_, false) => Availability::Active(format!("through Windows, except {}: another program uses them", taken.join(", "))),
        };
        let _ = events.send(Event::Hotkeys(a));
    }
    out
}

fn unregister(registered: &[Option<Action>]) {
    for (i, r) in registered.iter().enumerate() {
        if r.is_some() {
            // SAFETY: unregisters this thread's own hotkey.
            unsafe { UnregisterHotKey(std::ptr::null_mut(), i as i32) };
        }
    }
}
