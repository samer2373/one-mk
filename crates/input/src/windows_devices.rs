//! Which keyboard or mouse input came from, on Windows (per-device rules):
//! Raw Input, registered while capturing or while local key rules are configured
//! (`RIDEV_INPUTSINK`), names the
//! device of each event (`hDevice`); the hook's events are matched to them
//! by [`crate::devcorr::Correlator`]. The devices' identity comes from
//! their interface path (vendor, product, Bluetooth) and, where Windows
//! lets a normal program open them, their HID product and serial strings.
//!
//! COMPILE-CHECKED ONLY: never run on real Windows hardware. In
//! particular, if Windows doesn't give Raw Input the events a low-level
//! hook swallows, nothing is ever matched and every event is unknown: only
//! the rules without a device then apply (safe), and after a few misses no
//! event waits for its device any more (no latency).

use crate::common_logic::now_ms;
use crate::devcorr::{Correlated, Correlator, Token, hid_button_to_evdev, windows_path};
use crate::win_logic::hook_key_to_evdev;
use crate::{Bus, DeviceRef, Fingerprint, InputDevice, Kinds};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{HidD_GetProductString, HidD_GetSerialNumberString};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICELIST,
    RAWINPUTHEADER, RID_INPUT, RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDEV_REMOVE, RIDI_DEVICENAME, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
    RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{RI_KEY_BREAK, RI_KEY_E0, RI_MOUSE_HWHEEL, RI_MOUSE_WHEEL};

/// Generic desktop page: mouse (2) and keyboard (6).
const USAGES: [(u16, u16); 2] = [(1, 2), (1, 6)];

/// The devices Raw Input knows, and the events being matched.
#[derive(Default)]
pub(crate) struct RawDevices {
    /// Each `hDevice` (as a number) and what it is.
    table: Vec<(isize, InputDevice)>,
    next: u32,
    pub(crate) corr: Correlator,
    registered: bool,
}

impl RawDevices {
    /// Starts reading Raw Input on `hwnd` (capture begins).
    pub(crate) fn register(&mut self, hwnd: HWND) {
        let list = USAGES.map(|(page, usage)| RAWINPUTDEVICE {
            usUsagePage: page,
            usUsage: usage,
            dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
            hwndTarget: hwnd,
        });
        // SAFETY: a valid array of the size given.
        let ok = unsafe { RegisterRawInputDevices(list.as_ptr(), list.len() as u32, size_of::<RAWINPUTDEVICE>() as u32) } != 0;
        if !ok {
            tracing::debug!("Raw Input refused: input from every device counts as unknown");
        }
        self.registered = ok;
    }

    /// Stops reading Raw Input (capture ends); what waits goes to `out`.
    pub(crate) fn unregister(&mut self, out: &mut Vec<Correlated>) {
        self.corr.finish(out);
        if !std::mem::take(&mut self.registered) {
            return;
        }
        let list = USAGES.map(|(page, usage)| RAWINPUTDEVICE {
            usUsagePage: page,
            usUsage: usage,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: std::ptr::null_mut(),
        });
        // SAFETY: as above.
        unsafe { RegisterRawInputDevices(list.as_ptr(), list.len() as u32, size_of::<RAWINPUTDEVICE>() as u32) };
    }

    /// The keyboards and mice connected now; `None` if that is what was
    /// said last time. Devices gone are forgotten (their numbers are not
    /// used again); input still waiting for them goes to `out`, to be sent
    /// before the new list.
    pub(crate) fn enumerate(&mut self, out: &mut Vec<Correlated>) -> Option<Vec<InputDevice>> {
        let handles = device_list();
        let before: Vec<InputDevice> = self.table.iter().map(|(_, d)| d.clone()).collect();
        self.table.retain(|(h, _)| handles.iter().any(|(x, _)| x == h));
        for d in &before {
            if !self.table.iter().any(|(_, x)| x.id == d.id) {
                self.corr.forget(d.id, out);
            }
        }
        for (h, kind) in handles {
            if self.table.iter().any(|(x, _)| *x == h) {
                continue;
            }
            let Some(next) = self.next.checked_add(1) else { break };
            self.next = next;
            let kinds = Kinds { keyboard: kind == RIM_TYPEKEYBOARD, pointer: kind == RIM_TYPEMOUSE };
            let fingerprint = identity(h as HANDLE, kinds);
            self.table.push((h, InputDevice { id: DeviceRef(self.next), fingerprint, kinds }));
        }
        let now: Vec<InputDevice> = self.table.iter().map(|(_, d)| d.clone()).collect();
        (now != before).then_some(now)
    }

    /// `WM_INPUT`: what a device did, for the matching.
    pub(crate) fn on_input(&mut self, lparam: LPARAM, out: &mut Vec<Correlated>) {
        let mut raw = RAWINPUT::default();
        let mut size = size_of::<RAWINPUT>() as u32;
        // SAFETY: the buffer is a RAWINPUT, big enough for keyboard and
        // mouse input (the only kinds registered); the size says so.
        let n = unsafe {
            GetRawInputData(lparam as HRAWINPUT, RID_INPUT, (&raw mut raw).cast(), &mut size, size_of::<RAWINPUTHEADER>() as u32)
        };
        if n == u32::MAX || n == 0 || raw.header.hDevice.is_null() {
            return; // an error, or injected input (no device)
        }
        let h = raw.header.hDevice as isize;
        let Some(device) = self.table.iter().find(|(x, _)| *x == h).map(|(_, d)| d.id) else { return };
        let now = now_ms();
        match raw.header.dwType {
            RIM_TYPEKEYBOARD => {
                // SAFETY: the header says this is keyboard input.
                let k = unsafe { raw.data.keyboard };
                let extended = u32::from(k.Flags) & RI_KEY_E0 != 0;
                if let Some(code) = hook_key_to_evdev(u32::from(k.VKey), u32::from(k.MakeCode), extended) {
                    let down = u32::from(k.Flags) & RI_KEY_BREAK == 0;
                    self.corr.device_event(Token::Press { code, down }, device, now, out);
                }
            }
            RIM_TYPEMOUSE => {
                // SAFETY: the header says this is mouse input.
                let m = unsafe { raw.data.mouse };
                // SAFETY: both members of the union are plain integers.
                let flags = u32::from(unsafe { m.Anonymous.Anonymous.usButtonFlags });
                if m.lLastX != 0 || m.lLastY != 0 {
                    self.corr.device_motion(device, now);
                }
                // Down and up bits per button: left, right, middle, 4, 5.
                for b in 0..5u32 {
                    for (bit, down) in [(1 << (2 * b), true), (1 << (2 * b + 1), false)] {
                        if flags & bit != 0
                            && let Some(code) = hid_button_to_evdev(b + 1)
                        {
                            self.corr.device_event(Token::Press { code, down }, device, now, out);
                        }
                    }
                }
                if flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0 {
                    self.corr.device_event(Token::Wheel, device, now, out);
                }
            }
            _ => {}
        }
    }
}

/// Every keyboard and mouse Raw Input lists: handle and type.
fn device_list() -> Vec<(isize, u32)> {
    let mut n = 0u32;
    let size = size_of::<RAWINPUTDEVICELIST>() as u32;
    // SAFETY: asking for the count only.
    if unsafe { GetRawInputDeviceList(std::ptr::null_mut(), &mut n, size) } == u32::MAX {
        return Vec::new();
    }
    // A device plugged in between the two calls makes the second fail:
    // one more try with room to spare.
    for _ in 0..2 {
        let mut list = vec![RAWINPUTDEVICELIST::default(); n as usize + 4];
        let mut room = list.len() as u32;
        // SAFETY: `room` entries of the right size.
        let got = unsafe { GetRawInputDeviceList(list.as_mut_ptr(), &mut room, size) };
        if got != u32::MAX {
            list.truncate(got as usize);
            return list
                .into_iter()
                .filter(|d| d.dwType == RIM_TYPEKEYBOARD || d.dwType == RIM_TYPEMOUSE)
                .map(|d| (d.hDevice as isize, d.dwType))
                .collect();
        }
        n = room;
    }
    Vec::new()
}

/// Who a device is: its interface path, and its HID strings where the
/// device can be opened without access rights (keyboards and mice can be
/// asked for their strings).
fn identity(h: HANDLE, kinds: Kinds) -> Fingerprint {
    let path = device_path(h).unwrap_or_default();
    let (bus, vid, pid) = windows_path(&path);
    let (product, serial) = hid_strings(&path);
    let name = product.unwrap_or_else(|| {
        let what = if kinds.keyboard { "Keyboard" } else { "Mouse" };
        if bus == Bus::Unknown { what.to_owned() } else { format!("{} {what} {vid:04x}:{pid:04x}", bus.words()) }
    });
    Fingerprint::new(bus, vid, pid, serial.as_deref(), &name)
}

fn device_path(h: HANDLE) -> Option<String> {
    let mut len = 0u32;
    // SAFETY: asking for the length (in characters) only.
    unsafe { GetRawInputDeviceInfoW(h, RIDI_DEVICENAME, std::ptr::null_mut(), &mut len) };
    if len == 0 || len > 4096 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `len` characters of room.
    let got = unsafe { GetRawInputDeviceInfoW(h, RIDI_DEVICENAME, buf.as_mut_ptr().cast(), &mut len) };
    if got == u32::MAX || got as usize > buf.len() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// The HID product and serial strings, if the device opens.
fn hid_strings(path: &str) -> (Option<String>, Option<String>) {
    if path.is_empty() {
        return (None, None);
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a NUL-terminated path; no access asked (allowed for the
    // HID string queries), the handle is closed below.
    let file = unsafe {
        CreateFileW(wide.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
    };
    if file == INVALID_HANDLE_VALUE || file.is_null() {
        return (None, None);
    }
    let read = |f: unsafe extern "system" fn(HANDLE, *mut core::ffi::c_void, u32) -> bool| {
        // 126 characters is the most a USB string descriptor holds.
        let mut buf = [0u16; 128];
        // SAFETY: the buffer's size in bytes.
        let ok = unsafe { f(file, buf.as_mut_ptr().cast(), (buf.len() * 2) as u32) };
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        ok.then(|| String::from_utf16_lossy(&buf[..end])).filter(|s| !s.trim().is_empty())
    };
    let strings = (read(HidD_GetProductString), read(HidD_GetSerialNumberString));
    // SAFETY: our handle.
    unsafe { CloseHandle(file) };
    strings
}
