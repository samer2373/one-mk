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
    poll_for_event = c"xcb_poll_for_event": fn(*mut c_void) -> *mut c_void;
}}

functions! { XcbXkb, c"libxcb-xkb.so.1", {
    select_events = c"xcb_xkb_select_events_checked": fn(*mut c_void, u16, u16, u16, u16, u16, u16, *const c_void) -> Cookie;
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

/// `xcb_xkb_select_events`: the core keyboard, whichever device it is now.
const XKB_USE_CORE_KBD: u16 = 0x100;
/// XKB event types (`xkbType`, the second byte of an XKB event) that mean the keymap changed:
/// `NewKeyboardNotify` and `MapNotify`. (`StateNotify`, the modifiers, is read when asked for.)
const XKB_NEW_KEYBOARD_NOTIFY: u8 = 0;
const XKB_MAP_NOTIFY: u8 = 1;
const XKB_KEYMAP_EVENTS: u16 = 1 << XKB_NEW_KEYBOARD_NOTIFY | 1 << XKB_MAP_NOTIFY;
/// All map parts, for `MapNotify`.
const XKB_ALL_MAP_PARTS: u16 = 0xFF;

/// An owned, read-only native X11 keyboard connection. No polling thread or timer.
/// The display is explicit; an empty name never falls back to the user's desktop.
/// Missing runtime libraries, XKB or a usable connection yield physical fallback.
pub struct X11Keyboard {
    connection: NonNull<c_void>,
    libraries: &'static (Xcb, XkbX11),
    /// The event code of XKB events on this server; `None` if they could not be selected (every
    /// snapshot then loads the keymap again).
    events: Option<u8>,
    /// The keymap last loaded and the keyboard it came from, kept until the server says it changed.
    keymap: Option<Loaded>,
    /// How many times the keymap was loaded (it needs a server grab and many round trips).
    loads: u32,
}

/// A keymap downloaded from the server (our reference to it).
struct Loaded {
    keymap: *mut c_void,
    device: c_int,
}

unsafe impl Send for X11Keyboard {}

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
        let mut keyboard = Self { connection, libraries, events: None, keymap: None, loads: 0 };
        let native = keyboard.connection.as_ptr();
        if unsafe { (libraries.0.has_error)(native) } != 0 {
            return None;
        }
        let mut base_event = 0u8;
        let available = unsafe {
            (libraries.1.setup)(native, 1, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), &mut base_event, std::ptr::null_mut())
        };
        if available == 0 {
            return None;
        }
        keyboard.events = keyboard.select_keymap_events(base_event);
        Some(keyboard)
    }

    /// Asks the server to say when the keymap changes, so it need not be loaded again for every
    /// key. `None` (and loading it every time) if that is not possible.
    fn select_keymap_events(&self, base_event: u8) -> Option<u8> {
        static XKB: OnceLock<Option<XcbXkb>> = OnceLock::new();
        let xkb = XKB.get_or_init(XcbXkb::load).as_ref()?;
        if base_event == 0 { return None; }
        let native = self.connection.as_ptr();
        let xcb = &self.libraries.0;
        // SAFETY: every detail is selected for these events (`selectAll`), so none follows.
        let error = unsafe {
            let cookie = (xkb.select_events)(
                native, XKB_USE_CORE_KBD, XKB_KEYMAP_EVENTS, 0, XKB_KEYMAP_EVENTS, XKB_ALL_MAP_PARTS, XKB_ALL_MAP_PARTS, std::ptr::null(),
            );
            (xcb.request_check)(native, cookie)
        };
        if !error.is_null() {
            unsafe { libc::free(error) };
            return None;
        }
        Some(base_event)
    }

    /// How many times the keymap was loaded from the server.
    pub fn loads(&self) -> u32 { self.loads }

    /// Whether the server said the keymap changed since it was last looked at. Reads what has
    /// arrived, without waiting.
    fn keymap_changed(&mut self) -> bool {
        let Some(base_event) = self.events else { return true };
        let native = self.connection.as_ptr();
        let mut changed = false;
        loop {
            // SAFETY: a connection of ours; the event is ours to free.
            let event = unsafe { (self.libraries.0.poll_for_event)(native) };
            if event.is_null() { break; }
            // SAFETY: every event is at least 32 bytes: the response type, then for XKB the type.
            let (kind, xkb_type) = unsafe { (*event.cast::<u8>() & 0x7f, *event.cast::<u8>().add(1)) };
            if kind == base_event && matches!(xkb_type, XKB_NEW_KEYBOARD_NOTIFY | XKB_MAP_NOTIFY) { changed = true; }
            unsafe { libc::free(event) };
        }
        changed || unsafe { (self.libraries.0.has_error)(native) } != 0
    }

    /// The keymap and the modifiers, as one coherent pair.
    ///
    /// The keymap is downloaded under a short server grab, so a concurrent map change cannot pair
    /// one map with another map's modifier indices, and then kept: the server says when it
    /// changes (`NewKeyboardNotify`, `MapNotify`), and only then is it downloaded again. A
    /// snapshot in between asks for the modifier state alone, one round trip and no grab: a
    /// change that came before that answer is in the events read after it, so the pair is still
    /// one that existed. Without events (an old server) the keymap is loaded every time.
    pub fn snapshot(&mut self) -> Option<X11LayoutSnapshot> {
        let library = lib()?;
        if let Some(loaded) = &self.keymap {
            let state = unsafe { (self.libraries.1.state)(loaded.keymap, self.connection.as_ptr(), loaded.device) };
            if !state.is_null() {
                let state = State { lib: library, state };
                if !self.keymap_changed() {
                    return Some(Self::snapshot_of(state));
                }
            }
        }
        self.reload()
    }

    fn snapshot_of(state: State) -> X11LayoutSnapshot {
        let modifiers = state.modifiers();
        X11LayoutSnapshot {
            resolver: LayoutResolver { state },
            masks: LayoutState { depressed: modifiers.depressed, latched: modifiers.latched, locked: modifiers.locked, group: modifiers.group },
        }
    }

    /// Downloads the keymap and the modifiers under a server grab.
    fn reload(&mut self) -> Option<X11LayoutSnapshot> {
        let native = self.connection.as_ptr();
        let xcb = &self.libraries.0;
        let x11 = &self.libraries.1;
        let library = lib()?;
        self.forget_keymap();
        let error = unsafe { (xcb.request_check)(native, (xcb.grab_server)(native)) };
        if !error.is_null() {
            unsafe { libc::free(error) };
            return None;
        }
        self.loads += 1;
        let grab = ServerGrab(self);
        unsafe {
            let device = (x11.core_device)(native);
            if device < 0 { return None; }
            let context = (library.context_new)(0);
            if context.is_null() { return None; }
            let keymap = (x11.keymap)(context, native, device, 0);
            (library.context_unref)(context);
            if keymap.is_null() { return None; }
            let state = (x11.state)(keymap, native, device);
            if state.is_null() {
                (library.keymap_unref)(keymap);
                return None;
            }
            let state = State { lib: library, state };
            if (xcb.has_error)(native) != 0 {
                (library.keymap_unref)(keymap);
                return None;
            }
            // Events that arrived while it was loaded are about what is loaded now.
            drop(grab);
            self.keymap_changed();
            if self.events.is_some() { self.keymap = Some(Loaded { keymap, device }); } else { (library.keymap_unref)(keymap); }
            Some(Self::snapshot_of(state))
        }
    }

    fn forget_keymap(&mut self) {
        if let (Some(loaded), Some(library)) = (self.keymap.take(), lib()) {
            // SAFETY: our reference to the keymap.
            unsafe { (library.keymap_unref)(loaded.keymap) };
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
        self.forget_keymap();
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
