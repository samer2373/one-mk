//! EXPERIMENTAL power notifications on the capture run loop; native execution UNTESTED.

use crate::macos::{Inbox, Msg};
use objc2_core_foundation::{CFRetained, CFRunLoop, CFRunLoopSource, Type, kCFRunLoopCommonModes};
use std::ffi::c_void;
use std::sync::Arc;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IORegisterForSystemPower(context: *mut c_void, port: *mut *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, u32, u32, *mut c_void), notifier: *mut u32) -> u32;
    fn IONotificationPortGetRunLoopSource(port: *mut c_void) -> *const CFRunLoopSource;
    fn IOAllowPowerChange(root: u32, notification: isize) -> i32;
    fn IODeregisterForSystemPower(notifier: *mut u32) -> i32;
    fn IONotificationPortDestroy(port: *mut c_void);
    fn IOServiceClose(root: u32) -> i32;
}

struct Context {
    root: u32,
    inbox: Arc<Inbox>,
}

pub(crate) struct Power {
    context: Box<Context>,
    port: *mut c_void,
    notifier: u32,
    source: *const CFRunLoopSource,
    run_loop: CFRetained<CFRunLoop>,
}

impl Power {
    pub fn start(inbox: Arc<Inbox>, run_loop: &CFRunLoop) -> Option<Self> {
        let mut power = Self {
            context: Box::new(Context { root: 0, inbox }), port: std::ptr::null_mut(),
            notifier: 0, source: std::ptr::null(), run_loop: run_loop.retain(),
        };
        power.context.root = unsafe {
            IORegisterForSystemPower((&mut *power.context as *mut Context).cast(), &mut power.port, on_power, &mut power.notifier)
        };
        if power.context.root == 0 || power.port.is_null() { return None; }
        power.source = unsafe { IONotificationPortGetRunLoopSource(power.port) };
        let source = unsafe { power.source.as_ref() }?;
        run_loop.add_source(Some(source), unsafe { kCFRunLoopCommonModes });
        Some(power)
    }
}

impl Drop for Power {
    fn drop(&mut self) {
        unsafe {
            if let Some(source) = self.source.as_ref() {
                self.run_loop.remove_source(Some(source), kCFRunLoopCommonModes);
            }
            if self.notifier != 0 { IODeregisterForSystemPower(&mut self.notifier); }
            if !self.port.is_null() { IONotificationPortDestroy(self.port); }
            if self.context.root != 0 { IOServiceClose(self.context.root); }
        }
    }
}

unsafe extern "C" fn on_power(context: *mut c_void, _service: u32, message: u32, argument: *mut c_void) {
    let Some(context) = (unsafe { context.cast::<Context>().as_ref() }) else { return };
    match message {
        0xe000_0280 => context.inbox.post(Msg::Session(false)),
        0xe000_0300 => context.inbox.post(Msg::Session(true)),
        _ => {}
    }
    if matches!(message, 0xe000_0270 | 0xe000_0280) {
        unsafe { IOAllowPowerChange(context.root, argument as isize) };
    }
}
