use super::{DeadKeys, KeyboardLayout, shortcut};
use one_mk_proto::{Mods, typing::{LayoutDescriptor, Resolution}};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, MapVirtualKeyExW, ToUnicodeEx, MAPVK_VSC_TO_VK_EX, VK_CAPITAL, VK_NUMLOCK};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub(super) type State = DeadKeys;

#[repr(C)]
struct Version {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut Version) -> i32;
}

#[derive(Default)]
pub(super) struct Keyboard {
    dead: DeadKeys,
    layout: usize,
}

impl Keyboard {
    pub(super) fn new() -> Option<Self> {
        let mut version = Version { size: std::mem::size_of::<Version>() as u32, major: 0, minor: 0, build: 0, platform: 0, service_pack: [0; 128] };
        (unsafe { RtlGetVersion(&mut version) } == 0 && (version.major > 10 || (version.major == 10 && version.build >= 14393)))
            .then(Self::default)
    }

    fn current(&mut self) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL {
        let thread = unsafe { GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut()) };
        let layout = unsafe { GetKeyboardLayout(thread) };
        if self.layout != layout as usize { self.reset(); self.layout = layout as usize; }
        layout
    }

    pub(super) fn descriptor(&mut self) -> Option<LayoutDescriptor> {
        let layout = self.current() as usize;
        if layout == 0 { return None; }
        let language = layout & 0xffff;
        let name = match language { 0x0409 => "us".into(), 0x0407 => "de".into(), 0x040c => "fr".into(), 0x0401 => "ar".into(), _ => format!("windows-{language:04x}") };
        let mut descriptor = KeyboardLayout::descriptor_named(&name);
        let variant = (layout >> 16) & 0xffff;
        if variant != language { descriptor.variant = format!("{variant:04x}"); }
        Some(descriptor)
    }

    pub(super) fn reset(&mut self) { self.dead = DeadKeys::default(); }

    pub(super) fn resolve_with(&mut self, code: u32, mods: Mods, state: &mut Option<State>) -> Resolution {
        let state = state.get_or_insert_with(DeadKeys::default);
        std::mem::swap(&mut self.dead, state);
        let resolution = self.resolve(code, mods);
        std::mem::swap(&mut self.dead, state);
        resolution
    }

    pub(super) fn resolve(&mut self, code: u32, mods: Mods) -> Resolution {
        if shortcut(mods, false) { self.reset(); return Resolution::Physical; }
        let layout = self.current();
        if layout.is_null() { return Resolution::Physical; }
        let Some(key) = crate::win_logic::evdev_to_win(code) else { self.reset(); return Resolution::Physical };
        let (virtual_key, scan) = match key {
            crate::win_logic::WinKey::Scan { scan, extended } => {
                let scan = u32::from(scan) | if extended { 0xe000 } else { 0 };
                (unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, layout) }, scan)
            }
            crate::win_logic::WinKey::Vk { vk, scan, .. } => (u32::from(vk), u32::from(scan)),
        };
        if virtual_key == 0 || virtual_key > 255 { return Resolution::Physical; }
        let mut state = [0u8; 256];
        for modifier in mods.keys() {
            match modifier {
                42 | 54 => { state[0x10] = 0x80; state[if modifier == 42 { 0xa0 } else { 0xa1 }] = 0x80; }
                100 => { state[0x11] = 0x80; state[0x12] = 0x80; state[0xa5] = 0x80; }
                _ => {}
            }
        }
        state[usize::from(VK_CAPITAL)] = unsafe { GetKeyState(i32::from(VK_CAPITAL)) } as u8 & 1;
        state[usize::from(VK_NUMLOCK)] = unsafe { GetKeyState(i32::from(VK_NUMLOCK)) } as u8 & 1;
        state[virtual_key as usize] = 0x80;
        let mut text = [0u16; 16];
        let count = unsafe { ToUnicodeEx(virtual_key, scan, state.as_ptr(), text.as_mut_ptr(), text.len() as i32, 0x4, layout) };
        if count == 0 { self.reset(); return Resolution::Physical; }
        let length = count.unsigned_abs() as usize;
        if length > text.len() { self.reset(); return Resolution::Physical; }
        let Some(text) = String::from_utf16(&text[..length]).ok().filter(|text| one_mk_proto::typing::valid_sequence(text)) else { self.reset(); return Resolution::Physical };
        self.dead.feed(text, count < 0)
    }
}
