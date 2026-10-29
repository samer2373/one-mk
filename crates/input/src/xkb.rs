//! The modifier state of the wlroots backend's virtual keyboard, worked
//! out with the keymap the keyboard was given: wlroots takes modifiers
//! from the client, and what a key does depends on the keymap (with
//! `ctrl:nocaps`, Caps Lock is a Control key, not a lock). libxkbcommon is
//! loaded with `dlopen` the first time it is needed, so nothing else links
//! to it; without it (or with a keymap it can't compile) the modifiers
//! fall back to what the keys mean in a standard keymap ([`XkbMods`]).

use crate::linux_logic::XkbMods;
use std::ffi::{c_char, c_int, c_void};
use std::sync::OnceLock;

/// `xkb_state_component` bits.
const MODS_DEPRESSED: c_int = 1 << 0;
const MODS_LATCHED: c_int = 1 << 1;
const MODS_LOCKED: c_int = 1 << 2;
const LAYOUT_LOCKED: c_int = 1 << 6;
const LAYOUT_EFFECTIVE: c_int = 1 << 7;
const KEYMAP_FORMAT_TEXT_V1: c_int = 1;
/// X keycodes are evdev codes plus 8.
const EVDEV_OFFSET: u32 = 8;

/// The libxkbcommon functions used: a struct of function pointers and
/// `load`, which opens the library (never closed) and looks them up.
macro_rules! library {
    ($($field:ident = $sym:literal: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        struct Lib {
            $($field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        unsafe fn load() -> Option<Lib> {
            unsafe {
                let handle = libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if handle.is_null() {
                    return None;
                }
                Some(Lib {
                    $($field: {
                        let p = libc::dlsym(handle, $sym.as_ptr());
                        if p.is_null() {
                            return None;
                        }
                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(p)
                    },)*
                })
            }
        }
    };
}

library! {
    context_new = c"xkb_context_new": fn(c_int) -> *mut c_void;
    context_unref = c"xkb_context_unref": fn(*mut c_void);
    keymap_new_from_buffer = c"xkb_keymap_new_from_buffer": fn(*mut c_void, *const c_char, usize, c_int, c_int) -> *mut c_void;
    keymap_unref = c"xkb_keymap_unref": fn(*mut c_void);
    state_new = c"xkb_state_new": fn(*mut c_void) -> *mut c_void;
    state_unref = c"xkb_state_unref": fn(*mut c_void);
    state_update_key = c"xkb_state_update_key": fn(*mut c_void, u32, c_int) -> c_int;
    state_update_mask = c"xkb_state_update_mask": fn(*mut c_void, u32, u32, u32, u32, u32, u32) -> c_int;
    state_serialize_mods = c"xkb_state_serialize_mods": fn(*mut c_void, c_int) -> u32;
    state_serialize_layout = c"xkb_state_serialize_layout": fn(*mut c_void, c_int) -> u32;
}

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(|| {
        let lib = unsafe { load() };
        if lib.is_none() {
            tracing::warn!("libxkbcommon.so.0 not found: the virtual keyboard's modifiers assume a standard keymap");
        }
        lib
    })
    .as_ref()
}

/// A libxkbcommon keyboard state for one keymap.
struct State {
    lib: &'static Lib,
    state: *mut c_void,
}

// Owned and used by one thread at a time; libxkbcommon objects have no
// thread affinity.
unsafe impl Send for State {}

impl State {
    fn new(keymap: &[u8]) -> Option<State> {
        let lib = lib()?;
        unsafe {
            let ctx = (lib.context_new)(0);
            if ctx.is_null() {
                return None;
            }
            let map = (lib.keymap_new_from_buffer)(ctx, keymap.as_ptr().cast(), keymap.len(), KEYMAP_FORMAT_TEXT_V1, 0);
            // The keymap and state hold their own references.
            (lib.context_unref)(ctx);
            if map.is_null() {
                return None;
            }
            let state = (lib.state_new)(map);
            (lib.keymap_unref)(map);
            if state.is_null() {
                return None;
            }
            Some(State { lib, state })
        }
    }

    fn key(&mut self, code: u32, down: bool) {
        unsafe { (self.lib.state_update_key)(self.state, code + EVDEV_OFFSET, c_int::from(down)) };
    }

    fn modifiers(&self) -> Modifiers {
        let mods = |c| unsafe { (self.lib.state_serialize_mods)(self.state, c) };
        Modifiers {
            depressed: mods(MODS_DEPRESSED),
            latched: mods(MODS_LATCHED),
            locked: mods(MODS_LOCKED),
            group: unsafe { (self.lib.state_serialize_layout)(self.state, LAYOUT_EFFECTIVE) },
        }
    }

    /// Locks the layout `group` (the seat's), keeping the modifiers.
    fn lock_group(&mut self, group: u32) {
        let locked = unsafe { (self.lib.state_serialize_layout)(self.state, LAYOUT_LOCKED) };
        if locked != group {
            let m = self.modifiers();
            unsafe { (self.lib.state_update_mask)(self.state, m.depressed, m.latched, m.locked, 0, 0, group) };
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe { (self.lib.state_unref)(self.state) };
    }
}

/// What `zwp_virtual_keyboard_v1.modifiers` takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Modifiers {
    pub(crate) depressed: u32,
    pub(crate) latched: u32,
    pub(crate) locked: u32,
    pub(crate) group: u32,
}

/// The virtual keyboard's modifiers: from its keymap, or failing that
/// from a standard one.
pub(crate) struct KeyboardMods {
    real: Option<State>,
    fallback: XkbMods,
    group: u32,
    /// Keys down: a repeated press must not count twice (libxkbcommon
    /// would count a modifier pressed twice as held until released twice).
    down: Vec<u32>,
}

impl KeyboardMods {
    /// For the keymap the virtual keyboard was just given, with the keys
    /// `held` still down on it and the seat's layout `group`.
    pub(crate) fn new(keymap: &[u8], held: &[u32], group: u32) -> KeyboardMods {
        let real = State::new(keymap);
        if real.is_none() && lib().is_some() {
            tracing::warn!("libxkbcommon could not compile the keymap: the virtual keyboard's modifiers assume a standard one");
        }
        let mut m = KeyboardMods { real, fallback: XkbMods::default(), group: 0, down: Vec::new() };
        m.set_group(group);
        for &code in held {
            m.key(code, true);
        }
        m
    }

    /// Whether the modifiers come from the keymap itself.
    #[cfg(test)]
    fn is_real(&self) -> bool {
        self.real.is_some()
    }

    /// The keys down.
    pub(crate) fn held(&self) -> &[u32] {
        &self.down
    }

    /// A key sent to the keyboard; a press of a key already down (a
    /// repeat) changes nothing.
    pub(crate) fn key(&mut self, code: u32, down: bool) {
        if down == self.down.contains(&code) {
            return;
        }
        if down {
            self.down.push(code);
        } else {
            self.down.retain(|&c| c != code);
        }
        match &mut self.real {
            Some(s) => s.key(code, down),
            None => {
                self.fallback.key(code, down);
            }
        }
    }

    /// The seat's layout changed (another layout picked here).
    pub(crate) fn set_group(&mut self, group: u32) {
        self.group = group;
        if let Some(s) = &mut self.real {
            s.lock_group(group);
        }
    }

    pub(crate) fn get(&self) -> Modifiers {
        match &self.real {
            Some(s) => s.modifiers(),
            None => Modifiers { depressed: self.fallback.depressed(), latched: 0, locked: self.fallback.locked, group: self.group },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: u32 = 58;
    const LCTRL: u32 = 29;
    const LSHIFT: u32 = 42;
    const SHIFT: u32 = 1;
    const LOCK: u32 = 1 << 1;
    const CONTROL: u32 = 1 << 2;

    /// The least of a keymap with `ctrl:nocaps`: Caps Lock is a Control key.
    const NOCAPS: &str = r#"xkb_keymap {
    xkb_keycodes "min" {
        minimum = 8;
        maximum = 255;
        <LCTL> = 37;
        <AC01> = 38;
        <LFSH> = 50;
        <CAPS> = 66;
    };
    xkb_types "min" {
        type "ONE_LEVEL" { modifiers = none; level_name[Level1] = "Any"; };
        type "ALPHABETIC" {
            modifiers = Shift+Lock;
            map[Shift] = Level2;
            map[Lock] = Level2;
            level_name[Level1] = "Base";
            level_name[Level2] = "Caps";
        };
    };
    xkb_compat "min" {
        interpret Shift_L { action = SetMods(modifiers=modMapMods); };
        interpret Control_L { action = SetMods(modifiers=modMapMods); };
        interpret Caps_Lock { action = LockMods(modifiers=Lock); };
    };
    xkb_symbols "min" {
        key <LCTL> { [ Control_L ] };
        key <AC01> { [ a, A ] };
        key <LFSH> { [ Shift_L ] };
        key <CAPS> { [ Control_L ] };
        modifier_map Shift { <LFSH> };
        modifier_map Control { <LCTL>, <CAPS> };
    };
};
"#;

    fn real(keymap: &str) -> Option<KeyboardMods> {
        let m = KeyboardMods::new(keymap.as_bytes(), &[], 0);
        if lib().is_none() {
            eprintln!("libxkbcommon.so.0 not found: skipped");
            return None;
        }
        assert!(m.is_real(), "the keymap compiles");
        Some(m)
    }

    fn mods(depressed: u32, locked: u32) -> Modifiers {
        Modifiers { depressed, latched: 0, locked, group: 0 }
    }

    #[test]
    fn caps_lock_is_control_with_ctrl_nocaps() {
        let Some(mut m) = real(NOCAPS) else { return };
        m.key(CAPS, true);
        assert_eq!(m.get(), mods(CONTROL, 0), "Control depressed, nothing locked");
        m.key(CAPS, true);
        m.key(CAPS, false);
        assert_eq!(m.get(), mods(0, 0), "a repeat doesn't count twice");
        m.key(CAPS, false);
        assert_eq!(m.get(), mods(0, 0));
        m.key(LSHIFT, true);
        m.key(LCTRL, true);
        m.key(CAPS, true);
        m.key(LCTRL, false);
        assert_eq!(m.get(), mods(SHIFT | CONTROL, 0), "still held by Caps Lock");
        m.key(CAPS, false);
        m.key(LSHIFT, false);
        assert_eq!(m.get(), mods(0, 0));
    }

    /// The keys still down on the keyboard when its keymap changes count.
    #[test]
    fn a_new_keymap_starts_with_the_keys_held() {
        if lib().is_none() {
            return;
        }
        let mut m = KeyboardMods::new(NOCAPS.as_bytes(), &[CAPS, 30], 0);
        assert_eq!(m.get(), mods(CONTROL, 0));
        assert_eq!(m.held(), [CAPS, 30]);
        m.key(CAPS, false);
        assert_eq!(m.get(), mods(0, 0));
    }

    /// The real thing, compiled from the system's XKB data as a seat's
    /// keymap would be: a standard US layout, then with `ctrl:nocaps`.
    #[test]
    fn system_keymaps() {
        if !std::path::Path::new("/usr/share/X11/xkb/symbols/ctrl").exists() {
            eprintln!("no XKB data: skipped");
            return;
        }
        let keymap = |symbols: &str| {
            format!(
                "xkb_keymap {{ xkb_keycodes {{ include \"evdev+aliases(qwerty)\" }}; xkb_types {{ include \"complete\" }}; \
                 xkb_compat {{ include \"complete\" }}; xkb_symbols {{ include \"{symbols}\" }}; }};"
            )
        };
        let Some(mut us) = real(&keymap("pc+us+inet(evdev)")) else { return };
        us.key(CAPS, true);
        us.key(CAPS, false);
        assert_eq!(us.get(), mods(0, LOCK), "Caps Lock locks");
        us.key(LCTRL, true);
        assert_eq!(us.get(), mods(CONTROL, LOCK));

        let Some(mut nocaps) = real(&keymap("pc+us+inet(evdev)+ctrl(nocaps)")) else { return };
        nocaps.key(CAPS, true);
        assert_eq!(nocaps.get(), mods(CONTROL, 0), "depressed Control 4, locked 0");
        nocaps.key(CAPS, false);
        assert_eq!(nocaps.get(), mods(0, 0));

        // Two layouts: the seat's layout is locked, keys keep it.
        let Some(mut two) = real(&keymap("pc+us+de:2+inet(evdev)")) else { return };
        two.set_group(1);
        assert_eq!(two.get().group, 1);
        two.key(LSHIFT, true);
        assert_eq!(two.get(), Modifiers { depressed: SHIFT, latched: 0, locked: 0, group: 1 });
    }

    #[test]
    fn a_keymap_that_does_not_compile_falls_back() {
        let mut m = KeyboardMods::new(b"not a keymap", &[], 3);
        assert!(!m.is_real());
        m.key(CAPS, true);
        assert_eq!(m.get(), Modifiers { depressed: 0, latched: 0, locked: LOCK, group: 3 });
        m.key(LCTRL, true);
        assert_eq!(m.get().depressed, CONTROL);
    }
}
