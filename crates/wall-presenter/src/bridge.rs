use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;

#[repr(C)]
struct BusError {
    domain: u32,
    code: i32,
    message: *mut c_char,
}

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
    size: unsafe extern "C" fn(*mut c_void) -> usize,
    type_string: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    unsigned: unsafe extern "C" fn(*mut c_void) -> u32,
    string: unsafe extern "C" fn(*mut c_void, *mut usize) -> *const c_char,
}

impl Variants {
    unsafe fn load(glib: &Library) -> Result<Self, String> {
        unsafe {
            Ok(Self {
                child: *glib.get(b"g_variant_get_child_value\0").map_err(|error| error.to_string())?,
                count: *glib.get(b"g_variant_n_children\0").map_err(|error| error.to_string())?,
                integer: *glib.get(b"g_variant_get_int32\0").map_err(|error| error.to_string())?,
                boolean: *glib.get(b"g_variant_get_boolean\0").map_err(|error| error.to_string())?,
                unref: *glib.get(b"g_variant_unref\0").map_err(|error| error.to_string())?,
                size: *glib.get(b"g_variant_get_size\0").map_err(|error| error.to_string())?,
                type_string: *glib.get(b"g_variant_get_type_string\0").map_err(|error| error.to_string())?,
                unsigned: *glib.get(b"g_variant_get_uint32\0").map_err(|error| error.to_string())?,
                string: *glib.get(b"g_variant_get_string\0").map_err(|error| error.to_string())?,
            })
        }
    }

    unsafe fn valid(self, value: *mut c_void, signature: &CStr) -> bool {
        unsafe { !value.is_null() && (self.size)(value) <= 4096
            && CStr::from_ptr((self.type_string)(value)) == signature }
    }

    unsafe fn monitors(self, value: *mut c_void, index: usize) -> Option<Vec<(i32, i32, i32, i32)>> {
        unsafe {
            let array = (self.child)(value, index);
            let mut monitors = Vec::new();
            let count = (self.count)(array);
            if (1..=16).contains(&count) {
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
            crate::surface::extent(&monitors).ok().map(|_| monitors)
        }
    }
}

struct Inbox {
    variants: Variants,
    events: Vec<Event>,
    owner: CString,
}

unsafe extern "C" fn signal_received(_: *mut c_void, sender: *const c_char, _: *const c_char,
    _: *const c_char, signal: *const c_char, value: *mut c_void, data: *mut c_void) {
    unsafe {
        let inbox = &mut *data.cast::<Inbox>();
        if sender.is_null() || CStr::from_ptr(sender) != inbox.owner.as_c_str()
            || signal.is_null() || inbox.events.len() >= 32 { return; }
        match CStr::from_ptr(signal).to_bytes() {
            b"Layout" if inbox.variants.valid(value, c"(a(iiii))") => {
                if let Some(monitors) = inbox.variants.monitors(value, 0) {
                    inbox.events.push(Event::Layout(monitors));
                }
            },
            b"PauseHint" if inbox.variants.valid(value, c"(bs)") => {
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
            if self.subscription != 0 { (self.unsubscribe)(self.connection, self.subscription); }
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
            let free_error: unsafe extern "C" fn(*mut BusError) = *glib.get(b"g_error_free\0").map_err(|error| error.to_string())?;
            let variants = Variants::load(&glib)?;
            let unsubscribe = *gio.get(b"g_dbus_connection_signal_unsubscribe\0").map_err(|error| error.to_string())?;
            let object_unref = *gobject.get(b"g_object_unref\0").map_err(|error| error.to_string())?;
            let iterate = *glib.get(b"g_main_context_iteration\0").map_err(|error| error.to_string())?;
            let connection = get_bus(2, ptr::null_mut(), ptr::null_mut());
            if connection.is_null() { return Err("private session bus unavailable".into()); }
            let mut bridge = Self { _gio: gio, _glib: glib, _gobject: gobject, connection, subscription: 0, wayland_source: 0,
                unsubscribe, object_unref, iterate, inbox: Box::new(Inbox { variants, events: Vec::new(), owner: CString::default() }), monitors: Vec::new() };
            let owner_reply = call(connection, c"org.freedesktop.DBus".as_ptr(), c"/org/freedesktop/DBus".as_ptr(),
                c"org.freedesktop.DBus".as_ptr(), c"GetNameOwner".as_ptr(),
                new_variant(c"(s)".as_ptr(), c"org.onemk.WallpaperBridgeSpike1".as_ptr()),
                c"(s)".as_ptr().cast_mut().cast(), 0, 1000, ptr::null_mut(), ptr::null_mut());
            if !variants.valid(owner_reply, c"(s)") {
                if !owner_reply.is_null() { (variants.unref)(owner_reply); }
                return Err("private bridge has no typed unique owner".into());
            }
            let owner_value = (variants.child)(owner_reply, 0);
            let owner = CStr::from_ptr((variants.string)(owner_value, ptr::null_mut()));
            let owner_valid = owner.to_bytes().starts_with(b":") && owner.to_bytes().len() <= 255;
            bridge.inbox.owner = owner.to_owned();
            (variants.unref)(owner_value);
            (variants.unref)(owner_reply);
            if !owner_valid { return Err("invalid private bridge unique owner".into()); }
            let pid_reply = call(connection, c"org.freedesktop.DBus".as_ptr(), c"/org/freedesktop/DBus".as_ptr(),
                c"org.freedesktop.DBus".as_ptr(), c"GetConnectionUnixProcessID".as_ptr(),
                new_variant(c"(s)".as_ptr(), bridge.inbox.owner.as_ptr()),
                c"(u)".as_ptr().cast_mut().cast(), 0, 1000, ptr::null_mut(), ptr::null_mut());
            if !variants.valid(pid_reply, c"(u)") {
                if !pid_reply.is_null() { (variants.unref)(pid_reply); }
                return Err("private bridge owner has no typed PID".into());
            }
            let pid_value = (variants.child)(pid_reply, 0);
            let owner_pid = (variants.unsigned)(pid_value);
            (variants.unref)(pid_value);
            (variants.unref)(pid_reply);
            let descriptor = std::env::var("WAYLAND_SOCKET").map_err(|_| "missing private Wayland socket")?
                .parse().map_err(|_| "invalid private Wayland socket")?;
            authenticate_peer(descriptor, owner_pid)?;
            bridge.subscription = subscribe(connection, bridge.inbox.owner.as_ptr(),
                c"org.onemk.WallpaperBridgeSpike1".as_ptr(), ptr::null(), c"/org/onemk/WallpaperBridgeSpike1".as_ptr(),
                ptr::null(), 0, signal_received, (&mut *bridge.inbox as *mut Inbox).cast(), ptr::null_mut());
            let mut reply = ptr::null_mut();
            let mut last_error = String::from("no D-Bus error supplied");
            for _ in 0..10 {
                let mut error = ptr::null_mut();
                reply = call(connection, bridge.inbox.owner.as_ptr(), c"/org/onemk/WallpaperBridgeSpike1".as_ptr(),
                c"org.onemk.WallpaperBridgeSpike1".as_ptr(), c"Hello".as_ptr(), new_variant(c"(u)".as_ptr(), 1_u32),
                c"(ua(iiii))".as_ptr().cast_mut().cast(), 0, 100, ptr::null_mut(), &mut error);
                if !error.is_null() {
                    let error = error.cast::<BusError>();
                    last_error = format!("domain={} code={} {}", (*error).domain, (*error).code,
                        CStr::from_ptr((*error).message).to_string_lossy());
                    free_error(error);
                }
                if !reply.is_null() { break; }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if reply.is_null() { return Err(format!("private laboratory Hello failed: {last_error}")); }
            if !variants.valid(reply, c"(ua(iiii))") {
                (variants.unref)(reply);
                return Err("oversized or incorrectly typed private bridge Hello".into());
            }
            let version = (variants.child)(reply, 0);
            let supported = (variants.unsigned)(version) == 1;
            (variants.unref)(version);
            let monitors = variants.monitors(reply, 1);
            (variants.unref)(reply);
            if !supported { return Err("unsupported private bridge protocol".into()); }
            bridge.monitors = monitors.ok_or("invalid private bridge monitor geometry")?;
            Ok(bridge)
        }
    }

    pub fn poll(&mut self) -> Vec<Event> {
        unsafe { for _ in 0..32 { if (self.iterate)(ptr::null_mut(), 0) == 0 { break; } } }
        std::mem::take(&mut self.inbox.events)
    }
}

fn authenticate_peer(descriptor: i32, owner_pid: u32) -> Result<(), String> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe { libc::getsockopt(descriptor, libc::SOL_SOCKET, libc::SO_PEERCRED,
        (&mut credentials as *mut libc::ucred).cast(), &mut length) } != 0
        || credentials.pid <= 0 || owner_pid != credentials.pid as u32
        || credentials.uid != unsafe { libc::getuid() } {
        return Err("bridge unique owner is not the authenticated private Wayland peer".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn owner_must_match_wayland_peer() {
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(authenticate_peer(socket.as_raw_fd(), std::process::id()).is_ok());
        assert!(authenticate_peer(socket.as_raw_fd(), std::process::id() + 1).is_err());
        assert!(authenticate_peer(-1, std::process::id()).is_err());
    }

    #[test]
    fn variants_and_signals_are_bounded_typed_and_pinned() {
        unsafe {
            let glib = Library::new("libglib-2.0.so.0").unwrap();
            let variants = Variants::load(&glib).unwrap();
            let parse: libloading::Symbol<unsafe extern "C" fn(*const c_void, *const c_char,
                *const c_char, *mut *const c_char, *mut *mut c_void) -> *mut c_void>
                = glib.get(b"g_variant_parse\0").unwrap();
            let mut inbox = Inbox { variants, events: Vec::new(), owner: CString::new(":1.42").unwrap() };
            let mut send = |sender: &CStr, name: &CStr, text: &str| {
                let text = CString::new(text).unwrap();
                let value = parse(ptr::null(), text.as_ptr(), ptr::null(), ptr::null_mut(), ptr::null_mut());
                assert!(!value.is_null(), "invalid test variant: {text:?}");
                signal_received(ptr::null_mut(), sender.as_ptr(), ptr::null(), ptr::null(),
                    name.as_ptr(), value, (&mut inbox as *mut Inbox).cast());
                (variants.unref)(value);
            };
            send(c":1.99", c"PauseHint", "(true, 'covered')");
            send(c":1.42", c"PauseHint", "42");
            send(c":1.42", c"PauseHint", "('wrong', true)");
            send(c":1.42", c"PauseHint", &format!("(true, '{}')", "x".repeat(4096)));
            send(c":1.42", c"Layout", "([('wrong',)],)");
            send(c":1.42", c"Layout", "(@a(iiii) [],)");
            send(c":1.42", c"Layout", "([(0, 0, -1, 1080)],)");
            send(c":1.42", c"Layout", "([(0, 0, 2147483647, 1080)],)");
            send(c":1.42", c"Layout", &format!("([{}],)", vec!["(0, 0, 1, 1)"; 17].join(",")));
            assert!(inbox.events.is_empty());
            signal_received(ptr::null_mut(), c":1.42".as_ptr(), ptr::null(), ptr::null(),
                c"Layout".as_ptr(), ptr::null_mut(), (&mut inbox as *mut Inbox).cast());
            let text = c"(uint32 1, [(0, 0, 1920, 1080)])";
            let reply = parse(ptr::null(), text.as_ptr(), ptr::null(), ptr::null_mut(), ptr::null_mut());
            assert!(variants.valid(reply, c"(ua(iiii))"));
            assert!(!variants.valid(reply, c"(bs)"));
            assert_eq!(variants.monitors(reply, 1).unwrap(), [(0, 0, 1920, 1080)]);
            (variants.unref)(reply);
            for _ in 0..40 {
                let text = c"(true, 'covered')";
                let value = parse(ptr::null(), text.as_ptr(), ptr::null(), ptr::null_mut(), ptr::null_mut());
                signal_received(ptr::null_mut(), c":1.42".as_ptr(), ptr::null(), ptr::null(),
                    c"PauseHint".as_ptr(), value, (&mut inbox as *mut Inbox).cast());
                (variants.unref)(value);
            }
            assert_eq!(inbox.events.len(), 32);
            assert!(matches!(inbox.events[0], Event::Pause(true)));
        }
    }
}
