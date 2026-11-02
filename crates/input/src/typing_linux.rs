use super::{KeyboardLayout, shortcut};
use crate::{Composition, LayoutComposer, X11Keyboard, X11LayoutSnapshot};
use one_mk_proto::{Mods, typing::{LayoutDescriptor, Resolution}};

pub(super) type State = LayoutComposer;

pub(super) struct Keyboard {
    keyboard: Option<X11Keyboard>,
    snapshot: Option<X11LayoutSnapshot>,
    composer: LayoutComposer,
}

impl Keyboard {
    fn composer() -> Option<LayoutComposer> {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"].into_iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .unwrap_or_else(|| "C.UTF-8".into());
        LayoutComposer::new(&std::ffi::CString::new(locale).ok()?)
    }

    pub(super) fn new() -> Option<Self> {
        Some(Self {
            keyboard: Some(X11Keyboard::connect(&std::env::var("DISPLAY").ok()?)?), snapshot: None,
            composer: Self::composer()?,
        })
    }

    pub(super) fn from_keymap(keymap: &[u8], masks: crate::LayoutState) -> Option<Self> {
        Some(Self {
            keyboard: None,
            snapshot: Some(X11LayoutSnapshot { resolver: crate::LayoutResolver::from_keymap(keymap)?, masks }),
            composer: Self::composer()?,
        })
    }

    pub(super) fn modifiers(&mut self, masks: crate::LayoutState) {
        if let Some(snapshot) = &mut self.snapshot {
            if snapshot.masks.group != masks.group { self.composer.reset(); }
            snapshot.masks = masks;
        }
    }

    pub(super) fn descriptor(&mut self) -> Option<LayoutDescriptor> {
        if let Some(keyboard) = &mut self.keyboard { self.snapshot = keyboard.snapshot(); }
        let snapshot = self.snapshot.as_ref()?;
        let name = snapshot.resolver.layout_name(snapshot.masks.group)?;
        let canonical = match name.as_str() {
            "English (US)" => "us", "German" => "de", "French" => "fr", "Arabic" => "ar", other => other,
        };
        let descriptor = KeyboardLayout::descriptor_named(canonical);
        Some(descriptor)
    }

    pub(super) fn reset(&mut self) { self.composer.reset(); }

    pub(super) fn resolve_with(&mut self, code: u32, mods: Mods, state: &mut Option<State>) -> Resolution {
        if state.is_none() { *state = Self::composer(); }
        let Some(state) = state else { return Resolution::Physical };
        std::mem::swap(&mut self.composer, state);
        let resolution = self.resolve(code, mods);
        std::mem::swap(&mut self.composer, state);
        resolution
    }

    pub(super) fn resolve(&mut self, code: u32, mods: Mods) -> Resolution {
        if shortcut(mods, false) { self.reset(); return Resolution::Physical; }
        let Some(snapshot) = &self.snapshot else { return Resolution::Physical };
        let Some(masks) = snapshot.resolver.masks_for(mods, snapshot.masks) else { return Resolution::Physical };
        match self.composer.feed(&snapshot.resolver, code, masks) {
            Composition::Text(text) => Resolution::Text(text),
            Composition::Pending => Resolution::Pending,
            Composition::Cancelled => {
                if one_mk_proto::typing::character_key_code(code) { Resolution::Pending }
                else { Resolution::Physical }
            }
            Composition::Physical => Resolution::Physical,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(symbols: &str) -> Option<KeyboardLayout> {
        if !std::path::Path::new("/usr/share/X11/xkb/symbols/us").exists() { return None; }
        let keymap = format!("xkb_keymap {{ xkb_keycodes {{ include \"evdev+aliases(qwerty)\" }}; xkb_types {{ include \"complete\" }}; xkb_compat {{ include \"complete\" }}; xkb_symbols {{ include \"pc+{symbols}+inet(evdev)\" }}; }};");
        KeyboardLayout::from_keymap(keymap.as_bytes(), crate::LayoutState::default())
    }

    #[test]
    fn typing_supplied_keymap_follows_group_and_modifiers() {
        let Some(mut source) = layout("us+ara:2") else { return };
        assert_eq!(source.descriptor().name, "us");
        assert_eq!(source.resolve(16, Mods::default()), Resolution::Text("q".into()));
        let mut mods = Mods::default();
        mods.update(42, true);
        assert_eq!(source.resolve(16, mods), Resolution::Text("Q".into()));
        source.modifiers(crate::LayoutState { group: 1, ..Default::default() });
        assert!(source.refresh());
        assert_eq!(source.descriptor().name, "ar");
        assert_eq!(source.resolve(16, Mods::default()), Resolution::Text("ض".into()));
        mods.update(29, true);
        assert_eq!(source.resolve(16, mods), Resolution::Physical);
        assert!(!source.can_inject());
    }

    #[test]
    fn typing_supplied_keymap_composes_and_resets_per_visit() {
        let Some(source) = layout("us(intl)") else { return };
        assert_eq!(source.resolve(40, Mods::default()), Resolution::Pending);
        assert_eq!(source.resolve(18, Mods::default()), Resolution::Text("é".into()));
        assert_eq!(source.resolve(40, Mods::default()), Resolution::Pending);
        source.reset();
        assert_eq!(source.resolve(18, Mods::default()), Resolution::Text("e".into()));
        assert_eq!(source.resolve(40, Mods::default()), Resolution::Pending);
        assert_eq!(source.resolve(1, Mods::default()), Resolution::Physical);
        assert_eq!(source.resolve(18, Mods::default()), Resolution::Text("e".into()));
    }

    #[test]
    fn typing_native_compose_sessions_do_not_consume_each_other() {
        let Some(source) = layout("us(intl)") else { return };
        let mut first = super::super::Translation::default();
        let mut second = super::super::Translation::default();
        for state in [&mut first, &mut second] {
            assert_eq!(source.resolve_with(40, Mods::default(), state), Resolution::Pending);
        }
        first.reset();
        assert_eq!(source.resolve_with(18, Mods::default(), &mut first), Resolution::Text("e".into()));
        assert_eq!(source.resolve_with(18, Mods::default(), &mut second), Resolution::Text("é".into()));
    }
}
