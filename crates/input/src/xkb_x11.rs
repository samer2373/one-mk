use super::{LayoutResolver, LayoutState, State, lib};
use std::ffi::{CString, c_char, c_int, c_void};
use std::ptr::NonNull;
use std::sync::OnceLock;

#[repr(C)]
struct Cookie {
    sequence: u32,
}

macro_rules! functions {
    ($name:ident, $library:literal, {$($field:ident = $symbol:literal: fn($($arg:ty),*) -> $result:ty;)*}) => {
        struct $name {
            $($field: unsafe extern "C" fn($($arg),*) -> $result,)*
        }

        impl $name {
            fn load() -> Option<Self> {
                unsafe {
                    let handle = libc::dlopen($library.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                    if handle.is_null() { return None; }
                    Some(Self {
                        $($field: {
                            let address = libc::dlsym(handle, $symbol.as_ptr());
                            if address.is_null() {
                                libc::dlclose(handle);
                                return None;
                            }
                            std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) -> $result>(address)
                        },)*
                    })
                }
            }
        }
    };
}

functions! { Xcb, c"libxcb.so.1", {
    connect = c"xcb_connect": fn(*const c_char, *mut c_int) -> *mut c_void;
    disconnect = c"xcb_disconnect": fn(*mut c_void) -> ();
    has_error = c"xcb_connection_has_error": fn(*mut c_void) -> c_int;
    grab_server = c"xcb_grab_server_checked": fn(*mut c_void) -> Cookie;
    ungrab_server = c"xcb_ungrab_server": fn(*mut c_void) -> Cookie;
    request_check = c"xcb_request_check": fn(*mut c_void, Cookie) -> *mut c_void;
    flush = c"xcb_flush": fn(*mut c_void) -> c_int;
}}

functions! { XkbX11, c"libxkbcommon-x11.so.0", {
    setup = c"xkb_x11_setup_xkb_extension": fn(*mut c_void, u16, u16, c_int, *mut u16, *mut u16, *mut u8, *mut u8) -> c_int;
    core_device = c"xkb_x11_get_core_keyboard_device_id": fn(*mut c_void) -> c_int;
    keymap = c"xkb_x11_keymap_new_from_device": fn(*mut c_void, *mut c_void, c_int, c_int) -> *mut c_void;
    state = c"xkb_x11_state_new_from_device": fn(*mut c_void, *mut c_void, c_int) -> *mut c_void;
}}

fn libraries() -> Option<&'static (Xcb, XkbX11)> {
    static LIBRARIES: OnceLock<Option<(Xcb, XkbX11)>> = OnceLock::new();
    LIBRARIES.get_or_init(|| Some((Xcb::load()?, XkbX11::load()?))).as_ref()
}

/// An owned, read-only native X11 keyboard connection. No polling thread or timer.
/// The display is explicit; an empty name never falls back to the user's desktop.
/// Missing runtime libraries, XKB or a usable connection yield physical fallback.
pub struct X11Keyboard {
    connection: NonNull<c_void>,
    libraries: &'static (Xcb, XkbX11),
}

/// A coherent keymap and modifier snapshot, independent of later server changes.
pub struct X11LayoutSnapshot {
    pub resolver: LayoutResolver,
    pub masks: LayoutState,
}

impl X11Keyboard {
    pub fn connect(display: &str) -> Option<Self> {
        if display.is_empty() { return None; }
        let display = CString::new(display).ok()?;
        let libraries = libraries()?;
        lib()?;
        let connection = NonNull::new(unsafe { (libraries.0.connect)(display.as_ptr(), std::ptr::null_mut()) })?;
        let keyboard = Self { connection, libraries };
        let native = keyboard.connection.as_ptr();
        if unsafe { (libraries.0.has_error)(native) } != 0 {
            return None;
        }
        let available = unsafe {
            (libraries.1.setup)(native, 1, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
        };
        (available != 0).then_some(keyboard)
    }

    /// Queries only when called, under a short server grab so a concurrent map
    /// change cannot pair one map with another map's modifier indices.
    pub fn snapshot(&mut self) -> Option<X11LayoutSnapshot> {
        let native = self.connection.as_ptr();
        let xcb = &self.libraries.0;
        let x11 = &self.libraries.1;
        let library = lib()?;
        let error = unsafe { (xcb.request_check)(native, (xcb.grab_server)(native)) };
        if !error.is_null() {
            unsafe { libc::free(error) };
            return None;
        }
        let _grab = ServerGrab(self);
        unsafe {
            let device = (x11.core_device)(native);
            if device < 0 { return None; }
            let context = (library.context_new)(0);
            if context.is_null() { return None; }
            let keymap = (x11.keymap)(context, native, device, 0);
            (library.context_unref)(context);
            if keymap.is_null() { return None; }
            let state = (x11.state)(keymap, native, device);
            (library.keymap_unref)(keymap);
            if state.is_null() { return None; }
            let state = State { lib: library, state };
            if (xcb.has_error)(native) != 0 { return None; }
            let modifiers = state.modifiers();
            Some(X11LayoutSnapshot {
                resolver: LayoutResolver { state },
                masks: LayoutState {
                    depressed: modifiers.depressed,
                    latched: modifiers.latched,
                    locked: modifiers.locked,
                    group: modifiers.group,
                },
            })
        }
    }
}

struct ServerGrab<'keyboard>(&'keyboard X11Keyboard);

impl Drop for ServerGrab<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.0.libraries.0.ungrab_server)(self.0.connection.as_ptr());
            (self.0.libraries.0.flush)(self.0.connection.as_ptr());
        }
    }
}

impl Drop for X11Keyboard {
    fn drop(&mut self) {
        unsafe { (self.libraries.0.disconnect)(self.connection.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_implicitly_connect_to_the_live_desktop() {
        assert!(X11Keyboard::connect("").is_none());
        assert!(X11Keyboard::connect(":0\0").is_none());
    }
}
