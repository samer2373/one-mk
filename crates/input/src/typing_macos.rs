use super::{KeyboardLayout, shortcut};
use one_mk_proto::{Mods, typing::{LayoutDescriptor, Resolution}};
use std::ffi::{c_char, c_void};

pub(super) type State = u32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISGetInputSourceProperty(source: *const c_void, property: *const c_void) -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    static kTISPropertyInputSourceID: *const c_void;
    fn UCKeyTranslate(layout: *const c_void, code: u16, action: u16, modifiers: u32, keyboard: u32, options: u32, dead: *mut u32, capacity: usize, length: *mut usize, text: *mut u16) -> i32;
    fn LMGetKbdType() -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFDataGetBytePtr(value: *const c_void) -> *const c_void;
    fn CFStringGetCString(value: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
}

struct Source(*const c_void);

impl Source {
    fn current() -> Option<Self> {
        let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
        (!source.is_null()).then_some(Self(source))
    }
}

impl Drop for Source {
    fn drop(&mut self) { unsafe { CFRelease(self.0) }; }
}

#[derive(Default)]
pub(super) struct Keyboard {
    dead: u32,
    source: String,
}

impl Keyboard {
    pub(super) fn new() -> Option<Self> { Some(Self::default()) }

    pub(super) fn descriptor(&mut self) -> Option<LayoutDescriptor> {
        let source = Source::current()?;
        let property = unsafe { TISGetInputSourceProperty(source.0, kTISPropertyInputSourceID) };
        let data = unsafe { TISGetInputSourceProperty(source.0, kTISPropertyUnicodeKeyLayoutData) };
        if property.is_null() || data.is_null() { return None; }
        let mut buffer = [0u8; 256];
        if !unsafe { CFStringGetCString(property, buffer.as_mut_ptr().cast(), buffer.len() as isize, 0x08000100) } { return None; }
        let length = buffer.iter().position(|byte| *byte == 0)?;
        let id = std::str::from_utf8(&buffer[..length]).ok()?;
        if self.source != id { self.reset(); self.source = id.into(); }
        let name = match id.strip_prefix("com.apple.keylayout.").unwrap_or(id) {
            "US" | "ABC" => "us", "German" => "de", "French" => "fr", "Arabic" => "ar", other => other,
        };
        Some(KeyboardLayout::descriptor_named(name))
    }

    pub(super) fn reset(&mut self) { self.dead = 0; }

    pub(super) fn resolve_with(&mut self, code: u32, mods: Mods, state: &mut Option<State>) -> Resolution {
        let state = state.get_or_insert(0);
        std::mem::swap(&mut self.dead, state);
        let resolution = self.resolve(code, mods);
        std::mem::swap(&mut self.dead, state);
        resolution
    }

    pub(super) fn resolve(&mut self, code: u32, mods: Mods) -> Resolution {
        if shortcut(mods, true) { self.reset(); return Resolution::Physical; }
        let Some(key) = crate::macos_logic::evdev_to_mac(code) else { self.reset(); return Resolution::Physical };
        let Some(source) = Source::current() else { self.reset(); return Resolution::Physical };
        let data = unsafe { TISGetInputSourceProperty(source.0, kTISPropertyUnicodeKeyLayoutData) };
        if data.is_null() { self.reset(); return Resolution::Physical; }
        let layout = unsafe { CFDataGetBytePtr(data) };
        if layout.is_null() { self.reset(); return Resolution::Physical; }
        let mut modifiers = 0;
        for modifier in mods.keys() {
            modifiers |= match modifier { 42 | 54 => 2, 56 | 100 => 8, _ => 0 };
        }
        let flags = objc2_core_graphics::CGEventSource::flags_state(objc2_core_graphics::CGEventSourceStateID::CombinedSessionState);
        if flags.0 & 0x10000 != 0 { modifiers |= 4; }
        let mut text = [0u16; 16];
        let mut length = 0;
        let result = unsafe { UCKeyTranslate(layout, key, 0, modifiers, u32::from(LMGetKbdType()), 0, &mut self.dead, text.len(), &mut length, text.as_mut_ptr()) };
        if result != 0 || length > text.len() { self.reset(); return Resolution::Physical; }
        if length == 0 && self.dead != 0 { return Resolution::Pending; }
        String::from_utf16(&text[..length]).ok().filter(|text| one_mk_proto::typing::valid_sequence(text))
            .map_or_else(|| { self.reset(); Resolution::Physical }, Resolution::Text)
    }
}
