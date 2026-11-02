use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFType};
use std::ptr::NonNull;

#[repr(C)]
struct StoreContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    description: Option<unsafe extern "C" fn(*const c_void) -> *const CFString>,
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCreate(allocator: *const c_void, name: &CFString, callback: Option<unsafe extern "C" fn(*const CFType, *const CFArray, *mut c_void)>, context: *mut StoreContext) -> *mut CFType;
    fn SCDynamicStoreSetNotificationKeys(store: &CFType, keys: &CFArray<CFString>, patterns: &CFArray<CFString>) -> u8;
    fn SCDynamicStoreCreateRunLoopSource(allocator: *const c_void, store: &CFType, order: isize) -> *mut CFRunLoopSource;
    fn SCDynamicStoreCopyValue(store: *const CFType, key: &CFString) -> *mut CFType;
}

struct NetworkChanges {
    _store: CFRetained<CFType>,
    source: CFRetained<CFRunLoopSource>,
    _control: Arc<Control>,
}

impl NetworkChanges {
    fn start(control: Arc<Control>, run_loop: &CFRunLoop) -> Option<Self> {
        let mut context = StoreContext { version: 0, info: Arc::as_ptr(&control).cast_mut().cast(), retain: None, release: None, description: None };
        unsafe {
            let store = CFRetained::from_raw(NonNull::new(SCDynamicStoreCreate(std::ptr::null(), &CFString::from_static_str("oneMK networks"), Some(network_changed), &mut context))?);
            let keys = [CFString::from_static_str("State:/Network/Global/IPv4"), CFString::from_static_str("State:/Network/Global/IPv6")];
            let patterns = [CFString::from_static_str("State:/Network/Interface/.*/IPv[46]"), CFString::from_static_str("State:/Network/Service/.*/IPv[46]")];
            if SCDynamicStoreSetNotificationKeys(&store, &CFArray::from_objects(&keys.iter().map(|key| &**key).collect::<Vec<_>>()), &CFArray::from_objects(&patterns.iter().map(|key| &**key).collect::<Vec<_>>())) == 0 { return None; }
            let source = CFRetained::from_raw(NonNull::new(SCDynamicStoreCreateRunLoopSource(std::ptr::null(), &store, 0))?);
            run_loop.add_source(Some(&source), kCFRunLoopDefaultMode);
            Some(Self { _store: store, source, _control: control })
        }
    }
}

impl Drop for NetworkChanges {
    fn drop(&mut self) { self.source.invalidate(); }
}

unsafe extern "C" fn network_changed(_store: *const CFType, _keys: *const CFArray, context: *mut c_void) {
    if let Some(control) = unsafe { context.cast::<Control>().as_ref() } { control.send(Action::Refresh); }
}

pub(super) fn gateways() -> Vec<String> {
    let mut gateways = Vec::new();
    for key in ["State:/Network/Global/IPv4", "State:/Network/Global/IPv6"] {
        let Some(value) = NonNull::new(unsafe { SCDynamicStoreCopyValue(std::ptr::null(), &CFString::from_static_str(key)) }) else { continue };
        let value = unsafe { CFRetained::from_raw(value) };
        let Some(dictionary) = value.downcast_ref::<CFDictionary>() else { continue };
        let dictionary = unsafe { dictionary.cast_unchecked::<CFString, CFType>() };
        let Some(router) = dictionary.get(&CFString::from_static_str("Router")) else { continue };
        let Some(router) = router.downcast_ref::<CFString>() else { continue };
        let router = router.to_string();
        if router.split('%').next().is_some_and(|address| address.parse::<std::net::Ipv6Addr>().is_ok()) {
            let interface = dictionary.get(&CFString::from_static_str("PrimaryInterface"))
                .and_then(|value| value.downcast_ref::<CFString>().map(|value| value.to_string()));
            let scoped = if !router.contains('%') && let Some(interface) = interface { format!("{router}%{interface}") } else { router.clone() };
            if let Ok(output) = std::process::Command::new("/usr/sbin/ndp").args(["-n", &scoped]).output()
                && output.status.success() && let Some(mac) = super::neighbor_mac(&String::from_utf8_lossy(&output.stdout)) { gateways.push(mac); }
            continue;
        }
        if router.parse::<std::net::Ipv4Addr>().is_err() { continue; }
        let read = || std::process::Command::new("/usr/sbin/arp").args(["-n", &router]).output().ok();
        let mut output = read();
        if output.as_ref().is_none_or(|output| !output.status.success() || super::neighbor_mac(&String::from_utf8_lossy(&output.stdout)).is_none()) {
            let _ = std::process::Command::new("/sbin/ping").args(["-n", "-c", "1", "-W", "200", "-t", "1", &router])
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
            output = read();
        }
        if let Some(output) = output && output.status.success()
            && let Some(mac) = super::neighbor_mac(&String::from_utf8_lossy(&output.stdout)) { gateways.push(mac); }
    }
    gateways
}
