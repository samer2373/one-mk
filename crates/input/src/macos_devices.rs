//! Which keyboard or mouse input came from, on macOS (per-device rules):
//! IOHIDManager, on the capture run loop, lists the keyboards and mice
//! (vendor, product, name, transport, serial) and says when they come and
//! go; while capturing, their input values are matched to the event tap's
//! events by [`crate::devcorr::Correlator`]. It needs Input Monitoring,
//! which one-mk asks for anyway.
//!
//! COMPILE-CHECKED ONLY: never run on a real Mac. Unmatched events are
//! unknown, so only the rules without a device apply to them (safe).

use crate::common_logic::now_ms;
use crate::devcorr::{Correlator, Token, hid_button_to_evdev, hid_key_to_evdev};
use crate::{Bus, DeviceRef, Event, Fingerprint, InputDevice, Kinds};
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFBoolean, CFDictionary, CFNumber, CFRetained, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext,
    CFString, CFType, kCFRunLoopCommonModes,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use tokio::sync::mpsc;

type Ref = *mut c_void;
type DeviceCallback = unsafe extern "C-unwind" fn(context: *mut c_void, result: i32, sender: *mut c_void, device: Ref);
type ValueCallback = unsafe extern "C-unwind" fn(context: *mut c_void, result: i32, sender: *mut c_void, value: Ref);

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C-unwind" {
    fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> Ref;
    fn IOHIDManagerSetDeviceMatching(manager: Ref, matching: *const c_void);
    fn IOHIDManagerRegisterDeviceMatchingCallback(manager: Ref, callback: Option<DeviceCallback>, context: *mut c_void);
    fn IOHIDManagerRegisterDeviceRemovalCallback(manager: Ref, callback: Option<DeviceCallback>, context: *mut c_void);
    fn IOHIDManagerScheduleWithRunLoop(manager: Ref, run_loop: *const c_void, mode: *const c_void);
    fn IOHIDManagerOpen(manager: Ref, options: u32) -> i32;
    fn IOHIDDeviceRegisterInputValueCallback(device: Ref, callback: Option<ValueCallback>, context: *mut c_void);
    fn IOHIDDeviceGetProperty(device: Ref, key: *const c_void) -> *const c_void;
    fn IOHIDDeviceConformsTo(device: Ref, usage_page: u32, usage: u32) -> u8;
    fn IOHIDValueGetElement(value: Ref) -> Ref;
    fn IOHIDValueGetIntegerValue(value: Ref) -> isize;
    fn IOHIDElementGetUsagePage(element: Ref) -> u32;
    fn IOHIDElementGetUsage(element: Ref) -> u32;
}

/// The devices, the matching, and where events go.
pub(crate) struct Hid {
    tx: mpsc::UnboundedSender<Event>,
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Each device (by its `IOHIDDeviceRef`) and what it is.
    table: Vec<(usize, InputDevice)>,
    next: u32,
    corr: Correlator,
    capturing: bool,
    timer: Option<CFRetained<CFRunLoopTimer>>,
}

/// "Never", for a timer: a century away.
const NEVER: f64 = 3.0e9;

impl Hid {
    /// Starts watching the keyboards and mice on `run_loop` (the capture
    /// thread's). Lives for the rest of the process, like the capture state.
    pub(crate) fn start(tx: mpsc::UnboundedSender<Event>, run_loop: &CFRetained<CFRunLoop>) -> &'static Hid {
        let hid: &'static Hid = Box::leak(Box::new(Hid { tx, inner: RefCell::new(Inner::default()) }));
        let info = hid as *const Hid as *mut c_void;
        // SAFETY: an immutable CF constant.
        let modes = unsafe { kCFRunLoopCommonModes };
        let mut timer_ctx = CFRunLoopTimerContext { version: 0, info, retain: None, release: None, copyDescription: None };
        // SAFETY: the context is copied; `info` is 'static.
        let timer = unsafe { CFRunLoopTimer::new(None, CFAbsoluteTimeGetCurrent() + NEVER, NEVER, 0, 0, Some(on_timer), &mut timer_ctx) };
        if let Some(t) = &timer {
            run_loop.add_timer(Some(t), modes);
        }
        hid.inner.borrow_mut().timer = timer;
        // Generic desktop devices only (keyboards, mice, trackpads).
        let page = CFString::from_static_str("DeviceUsagePage");
        let one = CFNumber::new_i32(1);
        let matching = CFDictionary::<CFString, CFNumber>::from_slices(&[&page], &[&one]);
        let mode = modes.map_or(std::ptr::null(), |m| m as *const _ as *const c_void);
        // SAFETY: plain IOKit calls; the callbacks match their types and
        // `info` lives forever; the run loop and mode are valid.
        unsafe {
            let manager = IOHIDManagerCreate(std::ptr::null(), 0);
            if manager.is_null() {
                tracing::debug!("IOHIDManager not created: input from every device counts as unknown");
                return hid;
            }
            IOHIDManagerSetDeviceMatching(manager, CFRetained::as_ptr(&matching).as_ptr() as *const c_void);
            IOHIDManagerRegisterDeviceMatchingCallback(manager, Some(on_added), info);
            IOHIDManagerRegisterDeviceRemovalCallback(manager, Some(on_removed), info);
            IOHIDManagerScheduleWithRunLoop(manager, CFRetained::as_ptr(run_loop).as_ptr() as *const c_void, mode);
            let r = IOHIDManagerOpen(manager, 0);
            if r != 0 {
                tracing::debug!(r, "IOHIDManagerOpen failed: input from every device counts as unknown");
            }
        }
        hid
    }

    /// Capture starts (true) or ends: device input is read only meanwhile.
    pub(crate) fn capture(&self, on: bool) {
        let Ok(mut inner) = self.inner.try_borrow_mut() else { return };
        if inner.capturing == on {
            return;
        }
        inner.capturing = on;
        let info = self as *const Hid as *mut c_void;
        for (dev, _) in &inner.table {
            // SAFETY: a device the manager handed us and hasn't removed.
            unsafe { IOHIDDeviceRegisterInputValueCallback(*dev as Ref, if on { Some(on_value) } else { None }, info) };
        }
        if !on {
            let mut out = Vec::new();
            inner.corr.finish(&mut out);
            for ev in out {
                let _ = self.tx.send(ev);
            }
            inner.arm();
        }
    }

    /// An event of the tap: while capturing, it goes once its device is
    /// known (or not), in order.
    pub(crate) fn send(&self, ev: Event) {
        match self.inner.try_borrow_mut() {
            Ok(mut inner) if inner.capturing => {
                let mut out = Vec::new();
                inner.corr.hook(ev, now_ms(), &mut out);
                inner.arm();
                for ev in out {
                    let _ = self.tx.send(ev);
                }
            }
            _ => {
                let _ = self.tx.send(ev);
            }
        }
    }

    fn devices(&self, inner: &Inner) {
        let _ = self.tx.send(Event::Devices(inner.table.iter().map(|(_, d)| d.clone()).collect()));
    }
}

impl Inner {
    /// The timer runs only while an event waits for its device.
    fn arm(&self) {
        let Some(t) = &self.timer else { return };
        let now = CFAbsoluteTimeGetCurrent();
        t.set_next_fire_date(match self.corr.deadline() {
            Some(_) => now + crate::devcorr::WAIT_MS as f64 / 1000.0,
            None => now + NEVER,
        });
    }
}

/// A device's property, if it has it.
fn property<'a>(device: Ref, key: &'static str) -> Option<&'a CFType> {
    let key = CFString::from_static_str(key);
    // SAFETY: a valid device and key; the result is borrowed from the
    // device, which outlives this use (the table holds it until removal).
    let p = unsafe { IOHIDDeviceGetProperty(device, CFRetained::as_ptr(&key).as_ptr() as *const c_void) };
    // SAFETY: a CFTypeRef, or null.
    (!p.is_null()).then(|| unsafe { &*(p as *const CFType) })
}

fn number(device: Ref, key: &'static str) -> Option<i64> {
    property(device, key)?.downcast_ref::<CFNumber>()?.as_i64()
}

fn string(device: Ref, key: &'static str) -> Option<String> {
    Some(property(device, key)?.downcast_ref::<CFString>()?.to_string())
}

/// Who a device is: vendor, product, name, how it is connected, serial.
fn identity(device: Ref, kinds: Kinds) -> Fingerprint {
    let vid = number(device, "VendorID").and_then(|v| u16::try_from(v).ok()).unwrap_or(0);
    let pid = number(device, "ProductID").and_then(|v| u16::try_from(v).ok()).unwrap_or(0);
    let built_in = property(device, "Built-In").and_then(|p| p.downcast_ref::<CFBoolean>()).is_some_and(|b| b.as_bool());
    let transport = string(device, "Transport").unwrap_or_default().to_ascii_lowercase();
    let bus = if built_in || transport == "spi" || transport == "fifo" {
        Bus::Internal
    } else if transport.starts_with("bluetooth") {
        Bus::Bluetooth
    } else if transport == "usb" {
        Bus::Usb
    } else {
        Bus::Unknown
    };
    let name = string(device, "Product").unwrap_or_else(|| (if kinds.keyboard { "Keyboard" } else { "Mouse" }).to_owned());
    let serial = string(device, "SerialNumber");
    Fingerprint::new(bus, vid, pid, serial.as_deref(), &name)
}

unsafe extern "C-unwind" fn on_added(context: *mut c_void, _result: i32, _sender: *mut c_void, device: Ref) {
    // SAFETY: `context` is the leaked `Hid`.
    let hid = unsafe { &*(context as *const Hid) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: a valid device, asked about its usages.
        let (keyboard, pointer) = unsafe { (IOHIDDeviceConformsTo(device, 1, 6) != 0, IOHIDDeviceConformsTo(device, 1, 2) != 0) };
        if !keyboard && !pointer {
            return;
        }
        let Ok(mut inner) = hid.inner.try_borrow_mut() else { return };
        if inner.table.iter().any(|(d, _)| *d == device as usize) {
            return;
        }
        inner.next = inner.next.wrapping_add(1);
        let kinds = Kinds { keyboard, pointer };
        let id = DeviceRef(inner.next);
        inner.table.push((device as usize, InputDevice { id, fingerprint: identity(device, kinds), kinds }));
        if inner.capturing {
            // SAFETY: as in `capture`.
            unsafe { IOHIDDeviceRegisterInputValueCallback(device, Some(on_value), context) };
        }
        hid.devices(&inner);
    }));
}

unsafe extern "C-unwind" fn on_removed(context: *mut c_void, _result: i32, _sender: *mut c_void, device: Ref) {
    // SAFETY: `context` is the leaked `Hid`.
    let hid = unsafe { &*(context as *const Hid) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut inner) = hid.inner.try_borrow_mut() else { return };
        let Some(at) = inner.table.iter().position(|(d, _)| *d == device as usize) else { return };
        let (_, gone) = inner.table.remove(at);
        inner.corr.forget(gone.id);
        hid.devices(&inner);
    }));
}

unsafe extern "C-unwind" fn on_value(context: *mut c_void, _result: i32, sender: *mut c_void, value: Ref) {
    // SAFETY: `context` is the leaked `Hid`.
    let hid = unsafe { &*(context as *const Hid) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: a valid value, with its element.
        let (page, usage, v) = unsafe {
            let e = IOHIDValueGetElement(value);
            if e.is_null() {
                return;
            }
            (IOHIDElementGetUsagePage(e), IOHIDElementGetUsage(e), IOHIDValueGetIntegerValue(value))
        };
        let Ok(mut inner) = hid.inner.try_borrow_mut() else { return };
        // The sender of a device's value callback is the device.
        let Some(device) = inner.table.iter().find(|(d, _)| *d == sender as usize).map(|(_, d)| d.id) else { return };
        let now = now_ms();
        let mut out = Vec::new();
        match (page, usage) {
            (7, u) => {
                if let Some(code) = hid_key_to_evdev(u) {
                    inner.corr.device_event(Token::Press { code, down: v != 0 }, device, now, &mut out);
                }
            }
            (9, n) => {
                if let Some(code) = hid_button_to_evdev(n) {
                    inner.corr.device_event(Token::Press { code, down: v != 0 }, device, now, &mut out);
                }
            }
            (1, 0x30 | 0x31) if v != 0 => inner.corr.device_motion(device, now),
            (1, 0x38) | (0x0c, 0x238) if v != 0 => inner.corr.device_event(Token::Wheel, device, now, &mut out),
            _ => {}
        }
        inner.arm();
        for ev in out {
            let _ = hid.tx.send(ev);
        }
    }));
}

unsafe extern "C-unwind" fn on_timer(_timer: *mut CFRunLoopTimer, info: *mut c_void) {
    // SAFETY: `info` is the leaked `Hid`.
    let hid = unsafe { &*(info as *const Hid) };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut inner) = hid.inner.try_borrow_mut() else { return };
        let mut out = Vec::new();
        inner.corr.tick(now_ms(), &mut out);
        inner.arm();
        for ev in out {
            let _ = hid.tx.send(ev);
        }
    }));
}
