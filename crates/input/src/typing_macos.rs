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
    fn CFDataGetBytePtr(value: *const c_void) -> *const u8;
    fn CFDataGetLength(value: *const c_void) -> isize;
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

#[derive(Clone)]
struct Layout {
    source: String,
    data: Vec<u32>,
    keyboard: u32,
}

static LAYOUT: std::sync::Mutex<Option<Layout>> = std::sync::Mutex::new(None);
static EVENTS: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<crate::Event>>> = std::sync::Mutex::new(None);
static WATCHING: std::sync::Once = std::sync::Once::new();
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) fn watch(events: tokio::sync::mpsc::UnboundedSender<crate::Event>) {
    *EVENTS.lock().unwrap_or_else(|error| error.into_inner()) = Some(events);
}

pub(super) fn set_enabled(enabled: bool) {
    if ENABLED.swap(enabled, std::sync::atomic::Ordering::AcqRel) == enabled || !enabled { return; }
    WATCHING.call_once(|| dispatch2::DispatchQueue::main().exec_async(|| {
        if let Some(center) = objc2_core_foundation::CFNotificationCenter::distributed_center() {
            let name = objc2_core_foundation::CFString::from_static_str("com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged");
            unsafe { center.add_observer(std::ptr::null_mut(), Some(changed), Some(&name), std::ptr::null(), objc2_core_foundation::CFNotificationSuspensionBehavior::DeliverImmediately); }
        }
    }));
    dispatch2::DispatchQueue::main().exec_async(update);
}

unsafe extern "C-unwind" fn changed(
    _center: *mut objc2_core_foundation::CFNotificationCenter, _observer: *mut c_void,
    _name: *const objc2_core_foundation::CFString, _object: *const c_void,
    _info: *const objc2_core_foundation::CFDictionary,
) {
    dispatch2::DispatchQueue::main().exec_async(update);
}

fn read_layout() -> Option<Layout> {
    let source = Source::current()?;
    let property = unsafe { TISGetInputSourceProperty(source.0, kTISPropertyInputSourceID) };
    let data = unsafe { TISGetInputSourceProperty(source.0, kTISPropertyUnicodeKeyLayoutData) };
    if property.is_null() || data.is_null() { return None; }
    let mut buffer = [0u8; 256];
    if !unsafe { CFStringGetCString(property, buffer.as_mut_ptr().cast(), buffer.len() as isize, 0x08000100) } { return None; }
    let length = buffer.iter().position(|byte| *byte == 0)?;
    let identifier = std::str::from_utf8(&buffer[..length]).ok()?.to_owned();
    let length = usize::try_from(unsafe { CFDataGetLength(data) }).ok().filter(|length| *length > 0 && *length <= 1024 * 1024)?;
    let bytes = unsafe { CFDataGetBytePtr(data) };
    if bytes.is_null() { return None; }
    let mut owned = vec![0u32; length.div_ceil(4)];
    unsafe { std::ptr::copy_nonoverlapping(bytes, owned.as_mut_ptr().cast(), length); }
    Some(Layout { source: identifier, data: owned, keyboard: u32::from(unsafe { LMGetKbdType() }) })
}

fn update() {
    if !ENABLED.load(std::sync::atomic::Ordering::Acquire) { return; }
    *LAYOUT.lock().unwrap_or_else(|error| error.into_inner()) = read_layout();
    if let Some(events) = EVENTS.lock().unwrap_or_else(|error| error.into_inner()).as_ref()
        && let Some(keyboard) = KeyboardLayout::native() { let _ = events.send(crate::Event::KeyboardLayout(keyboard)); }
}

#[derive(Default)]
pub(super) struct Keyboard {
    dead: u32,
    source: String,
}

impl Keyboard {
    pub(super) fn new() -> Option<Self> { Some(Self::default()) }

    pub(super) fn descriptor(&mut self) -> Option<LayoutDescriptor> {
        let layout = LAYOUT.lock().unwrap_or_else(|error| error.into_inner());
        let id = layout.as_ref().map_or("unknown", |layout| layout.source.as_str());
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
        let cached = LAYOUT.lock().unwrap_or_else(|error| error.into_inner());
        let Some(layout) = cached.as_ref() else { self.reset(); return Resolution::Physical };
        if self.source != layout.source { self.reset(); self.source.clone_from(&layout.source); }
        let mut modifiers = 0;
        for modifier in mods.keys() {
            modifiers |= match modifier { 42 | 54 => 2, 56 | 100 => 8, _ => 0 };
        }
        let flags = objc2_core_graphics::CGEventSource::flags_state(objc2_core_graphics::CGEventSourceStateID::CombinedSessionState);
        if flags.0 & 0x10000 != 0 { modifiers |= 4; }
        let mut text = [0u16; 16];
        let mut length = 0;
        let result = unsafe { UCKeyTranslate(layout.data.as_ptr().cast(), key, 0, modifiers, layout.keyboard, 0, &mut self.dead, text.len(), &mut length, text.as_mut_ptr()) };
        if result != 0 || length > text.len() { self.reset(); return Resolution::Physical; }
        if length == 0 && self.dead != 0 { return Resolution::Pending; }
        String::from_utf16(&text[..length]).ok().filter(|text| one_mk_proto::typing::valid_sequence(text))
            .map_or_else(|| { self.reset(); Resolution::Physical }, Resolution::Text)
    }
}
