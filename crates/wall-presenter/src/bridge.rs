use libloading::Library;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;

pub enum Event {
    Layout(Vec<(i32, i32, i32, i32)>),
    Pause(bool),
}

#[derive(Clone, Copy)]
struct Variants {
    child: unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void,
    count: unsafe extern "C" fn(*mut c_void) -> usize,
    integer: unsafe extern "C" fn(*mut c_void) -> i32,
    boolean: unsafe extern "C" fn(*mut c_void) -> i32,
    unref: unsafe extern "C" fn(*mut c_void),
}

impl Variants {
    unsafe fn monitors(self, value: *mut c_void, index: usize) -> Vec<(i32, i32, i32, i32)> {
        unsafe {
            let array = (self.child)(value, index);
            let mut monitors = Vec::new();
            let count = (self.count)(array);
            if count <= 16 {
                for index in 0..count {
                    let monitor = (self.child)(array, index);
                    let mut fields = [0; 4];
                    for (index, field) in fields.iter_mut().enumerate() {
                        let child = (self.child)(monitor, index);
                        *field = (self.integer)(child);
                        (self.unref)(child);
                    }
                    (self.unref)(monitor);
                    monitors.push((fields[0], fields[1], fields[2], fields[3]));
                }
            }
            (self.unref)(array);
            monitors
        }
    }
}

struct Inbox {
    variants: Variants,
    events: Vec<Event>,
}

unsafe extern "C" fn signal_received(_: *mut c_void, _: *const c_char, _: *const c_char,
    _: *const c_char, signal: *const c_char, value: *mut c_void, data: *mut c_void) {
    unsafe {
        let inbox = &mut *data.cast::<Inbox>();
        if inbox.events.len() >= 32 { return; }
        match CStr::from_ptr(signal).to_bytes() {
            b"Layout" => inbox.events.push(Event::Layout(inbox.variants.monitors(value, 0))),
            b"PauseHint" => {
                let child = (inbox.variants.child)(value, 0);
                inbox.events.push(Event::Pause((inbox.variants.boolean)(child) != 0));
                (inbox.variants.unref)(child);
            }
            _ => {},
        }
    }
}

pub struct Bridge {
    _gio: Library,
    _glib: Library,
    _gobject: Library,
    connection: *mut c_void,
    subscription: u32,
    wayland_source: u32,
    unsubscribe: unsafe extern "C" fn(*mut c_void, u32),
    object_unref: unsafe extern "C" fn(*mut c_void),
    iterate: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    inbox: Box<Inbox>,
    pub monitors: Vec<(i32, i32, i32, i32)>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        unsafe {
            if self.wayland_source != 0 {
                let remove: libloading::Symbol<unsafe extern "C" fn(u32) -> i32>
                    = self._glib.get(b"g_source_remove\0").expect("loaded GLib source API");
                remove(self.wayland_source);
            }
            (self.unsubscribe)(self.connection, self.subscription);
            (self.object_unref)(self.connection);
        }
    }
}

impl Bridge {
    pub fn wait(&mut self, wayland_fd: std::os::fd::RawFd) -> Result<(), String> {
        unsafe extern "C" fn ready(_: i32, _: u32, _: *mut c_void) -> i32 { 1 }
        unsafe {
            let add: unsafe extern "C" fn(i32, i32, u32,
                unsafe extern "C" fn(i32, u32, *mut c_void) -> i32, *mut c_void, *mut c_void) -> u32
                = *self._glib.get(b"g_unix_fd_add_full\0").map_err(|error| error.to_string())?;
            if self.wayland_source == 0 {
                self.wayland_source = add(0, wayland_fd, 1 | 8 | 16, ready, ptr::null_mut(), ptr::null_mut());
                if self.wayland_source == 0 { return Err("cannot watch private Wayland connection".into()); }
            }
            (self.iterate)(ptr::null_mut(), 1);
        }
        Ok(())
    }

    pub unsafe fn new() -> Result<Self, String> {
        unsafe {
            let gio = Library::new("libgio-2.0.so.0").map_err(|error| error.to_string())?;
            let glib = Library::new("libglib-2.0.so.0").map_err(|error| error.to_string())?;
            let gobject = Library::new("libgobject-2.0.so.0").map_err(|error| error.to_string())?;
            let get_bus: unsafe extern "C" fn(i32, *mut c_void, *mut *mut c_void) -> *mut c_void = *gio.get(b"g_bus_get_sync\0").map_err(|error| error.to_string())?;
            let subscribe: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char,
                *const c_char, *const c_char, u32, unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char,
                    *const c_char, *const c_char, *mut c_void, *mut c_void), *mut c_void, *mut c_void) -> u32
                = *gio.get(b"g_dbus_connection_signal_subscribe\0").map_err(|error| error.to_string())?;
            let call: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char, *const c_char,
                *mut c_void, *mut c_void, u32, i32, *mut c_void, *mut *mut c_void) -> *mut c_void
                = *gio.get(b"g_dbus_connection_call_sync\0").map_err(|error| error.to_string())?;
            let new_variant: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void = *glib.get(b"g_variant_new\0").map_err(|error| error.to_string())?;
            let variants = Variants {
                child: *glib.get(b"g_variant_get_child_value\0").map_err(|error| error.to_string())?,
                count: *glib.get(b"g_variant_n_children\0").map_err(|error| error.to_string())?,
                integer: *glib.get(b"g_variant_get_int32\0").map_err(|error| error.to_string())?,
                boolean: *glib.get(b"g_variant_get_boolean\0").map_err(|error| error.to_string())?,
                unref: *glib.get(b"g_variant_unref\0").map_err(|error| error.to_string())?,
            };
            let unsubscribe = *gio.get(b"g_dbus_connection_signal_unsubscribe\0").map_err(|error| error.to_string())?;
            let object_unref = *gobject.get(b"g_object_unref\0").map_err(|error| error.to_string())?;
            let iterate = *glib.get(b"g_main_context_iteration\0").map_err(|error| error.to_string())?;
            let connection = get_bus(2, ptr::null_mut(), ptr::null_mut());
            if connection.is_null() { return Err("private session bus unavailable".into()); }
            let mut bridge = Self { _gio: gio, _glib: glib, _gobject: gobject, connection, subscription: 0, wayland_source: 0,
                unsubscribe, object_unref, iterate, inbox: Box::new(Inbox { variants, events: Vec::new() }), monitors: Vec::new() };
            bridge.subscription = subscribe(connection, c"org.onemk.WallpaperBridgeSpike1".as_ptr(),
                c"org.onemk.WallpaperBridgeSpike1".as_ptr(), ptr::null(), c"/org/onemk/WallpaperBridgeSpike1".as_ptr(),
                ptr::null(), 0, signal_received, (&mut *bridge.inbox as *mut Inbox).cast(), ptr::null_mut());
            let mut reply = ptr::null_mut();
            for _ in 0..10 {
                reply = call(connection, c"org.onemk.WallpaperBridgeSpike1".as_ptr(), c"/org/onemk/WallpaperBridgeSpike1".as_ptr(),
                c"org.onemk.WallpaperBridgeSpike1".as_ptr(), c"Hello".as_ptr(), new_variant(c"(u)".as_ptr(), 1_u32),
                ptr::null_mut(), 0, 100, ptr::null_mut(), ptr::null_mut());
                if !reply.is_null() { break; }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if reply.is_null() { return Err("private laboratory Hello failed".into()); }
            bridge.monitors = variants.monitors(reply, 1);
            (variants.unref)(reply);
            Ok(bridge)
        }
    }

    pub fn poll(&mut self) -> Vec<Event> {
        unsafe { for _ in 0..32 { if (self.iterate)(ptr::null_mut(), 0) == 0 { break; } } }
        std::mem::take(&mut self.inbox.events)
    }
}
