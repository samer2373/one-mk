use one_mk_proto::typing::{Chord, LayoutDescriptor, Resolution};
use one_mk_proto::Mods;
use std::sync::{Arc, Mutex};
use unicode_normalization::UnicodeNormalization;

#[cfg(target_os = "macos")]
#[path = "typing_macos.rs"]
mod native;
#[cfg(windows)]
#[path = "typing_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "typing_linux.rs"]
mod native;

#[cfg(target_os = "macos")]
pub(crate) fn watch_native(events: tokio::sync::mpsc::UnboundedSender<crate::Event>) { native::watch(events); }

pub fn character_mode(enabled: bool) {
    #[cfg(target_os = "macos")]
    native::set_enabled(enabled);
    #[cfg(not(target_os = "macos"))]
    let _ = enabled;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text {
    bytes: [u8; one_mk_proto::typing::MAX_TEXT_BYTES],
    length: u8,
}

impl Text {
    pub fn new(text: &str) -> Option<Self> {
        if !one_mk_proto::typing::valid_text(text) { return None; }
        let mut bytes = [0; one_mk_proto::typing::MAX_TEXT_BYTES];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Some(Self { bytes, length: text.len() as u8 })
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)]).expect("validated text")
    }

    #[cfg(any(windows, test))]
    pub(crate) fn unicode_keys(&self) -> Vec<(u16, bool)> {
        self.as_str().encode_utf16().flat_map(|unit| [(unit, true), (unit, false)]).collect()
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::new(text).expect("one bounded printable grapheme")
    }
}

#[derive(Clone)]
pub struct KeyboardLayout {
    descriptor: LayoutDescriptor,
    table: Vec<(u32, [Option<char>; 4])>,
    runtime: Option<Arc<Mutex<native::Keyboard>>>,
    dead: Arc<Mutex<DeadKeys>>,
}

impl std::fmt::Debug for KeyboardLayout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("KeyboardLayout").field("descriptor", &self.descriptor).finish()
    }
}

impl PartialEq for KeyboardLayout {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor == other.descriptor && self.table == other.table && self.runtime.is_some() == other.runtime.is_some()
    }
}

pub fn shortcut(mods: Mods, option_is_text: bool) -> bool {
    mods.keys().any(|code| matches!(code, 29 | 97 | 125 | 126) || (!option_is_text && code == 56))
}

#[derive(Default)]
pub(super) struct DeadKeys(Option<char>);

#[derive(Default)]
pub struct Translation {
    descriptor: Option<LayoutDescriptor>,
    native: Option<native::State>,
    dead: DeadKeys,
}

impl Translation {
    pub fn reset(&mut self) { *self = Self::default(); }
}

impl DeadKeys {
    pub(super) fn feed(&mut self, text: String, dead: bool) -> Resolution {
        if dead {
            let Some(character) = text.chars().next() else { return Resolution::Physical };
            return match self.0.replace(character) {
                Some(previous) => {
                    self.0 = None;
                    Resolution::Text(if previous == character { previous.to_string() } else { format!("{previous}{character}") })
                }
                None => Resolution::Pending,
            };
        }
        let Some(accent) = self.0.take() else { return Resolution::Text(text) };
        if text == " " || text == accent.to_string() { return Resolution::Text(accent.to_string()); }
        let mark = match accent {
            '\'' | '´' => '\u{301}', '`' => '\u{300}', '^' => '\u{302}', '~' => '\u{303}',
            '"' | '¨' => '\u{308}', '˚' | '°' => '\u{30a}', '¸' => '\u{327}',
            'ˇ' => '\u{30c}', '˘' => '\u{306}', '˛' => '\u{328}', '˙' => '\u{307}',
            '˝' => '\u{30b}', '¯' => '\u{304}',
            _ => return Resolution::Text(format!("{accent}{text}")),
        };
        let combined: String = format!("{text}{mark}").nfc().collect();
        if combined.chars().count() == 1 { Resolution::Text(combined) }
        else { Resolution::Text(format!("{accent}{text}")) }
    }
}

#[cfg(any(windows, target_os = "macos", test))]
#[derive(Default)]
pub(crate) struct NativeText {
    held: std::collections::HashMap<u32, char>,
    mods: Mods,
}

#[cfg(any(windows, target_os = "macos", test))]
impl NativeText {
    pub(crate) fn clear(&mut self) { self.held.clear(); self.mods = Mods::default(); }

    fn text(&self, text: Text) -> Vec<crate::Inject> {
        if shortcut(self.mods, cfg!(target_os = "macos")) { return Vec::new(); }
        let modifiers: Vec<_> = self.mods.keys().filter(|code| matches!(code, 42 | 54 | 56 | 100)).collect();
        let mut output: Vec<_> = modifiers.iter().map(|code| crate::Inject::Key { code: *code, down: false }).collect();
        output.push(crate::Inject::Text { text });
        output.extend(modifiers.into_iter().rev().map(|code| crate::Inject::Key { code, down: true }));
        output
    }

    pub(crate) fn input(&mut self, input: crate::Inject) -> Vec<crate::Inject> {
        match input {
            crate::Inject::Controlled(false) => { self.clear(); vec![input] }
            crate::Inject::CharacterKey { code, down: true, character } => {
                if !one_mk_proto::typing::character_key_code(code) || !one_mk_proto::typing::printable(character) { return Vec::new(); }
                self.held.entry(code).or_insert(character);
                self.repeat(code).unwrap_or_default()
            }
            crate::Inject::CharacterKey { code, down: false, character } => {
                if self.held.get(&code) == Some(&character) { self.held.remove(&code); }
                Vec::new()
            }
            crate::Inject::Key { code, down } if self.held.contains_key(&code) => {
                if !down { self.held.remove(&code); }
                Vec::new()
            }
            crate::Inject::Key { code, down } => { self.mods.update(code, down); vec![input] }
            crate::Inject::Text { text } => self.text(text),
            _ => vec![input],
        }
    }

    pub(crate) fn repeat(&self, code: u32) -> Option<Vec<crate::Inject>> {
        self.held.get(&code).map(|character| self.text(Text::from(character.to_string().as_str())))
    }
}

impl KeyboardLayout {
    pub(super) fn descriptor_named(name: &str) -> LayoutDescriptor {
        LayoutDescriptor { name: name.into(), variant: String::new(), rules: std::env::consts::OS.into(), model: String::new() }
    }

    pub fn native() -> Option<Self> {
        let mut keyboard = native::Keyboard::new()?;
        Some(Self {
            descriptor: keyboard.descriptor()?, table: Vec::new(),
            runtime: Some(Arc::new(Mutex::new(keyboard))), dead: Arc::default(),
        })
    }

    #[cfg(target_os = "linux")]
    pub fn from_keymap(keymap: &[u8], masks: crate::LayoutState) -> Option<Self> {
        let mut keyboard = native::Keyboard::from_keymap(keymap, masks)?;
        Some(Self {
            descriptor: keyboard.descriptor()?, table: Vec::new(),
            runtime: Some(Arc::new(Mutex::new(keyboard))), dead: Arc::default(),
        })
    }

    #[cfg(target_os = "linux")]
    pub fn modifiers(&self, masks: crate::LayoutState) {
        if let Some(runtime) = &self.runtime && let Ok(mut keyboard) = runtime.lock() { keyboard.modifiers(masks); }
    }

    pub fn can_inject(&self) -> bool {
        self.runtime.is_none() || cfg!(any(windows, target_os = "macos"))
    }

    pub fn native_text(&self) -> bool { self.runtime.is_some() && self.can_inject() }

    pub fn is_shortcut(&self, mods: Mods) -> bool {
        shortcut(mods, self.runtime.is_some() && cfg!(target_os = "macos"))
    }

    pub fn refresh(&mut self) -> bool {
        let Some(runtime) = &self.runtime else { return false };
        let Ok(mut keyboard) = runtime.lock() else { return false };
        let Some(descriptor) = keyboard.descriptor() else { return false };
        if descriptor == self.descriptor { return false; }
        keyboard.reset();
        self.descriptor = descriptor;
        true
    }

    pub fn reset(&self) {
        if let Some(runtime) = &self.runtime && let Ok(mut keyboard) = runtime.lock() { keyboard.reset(); }
        if let Ok(mut dead) = self.dead.lock() { dead.0 = None; }
    }

    pub fn descriptor(&self) -> &LayoutDescriptor {
        &self.descriptor
    }

    pub fn resolve(&self, code: u32, mods: Mods) -> Resolution {
        if let Some(runtime) = &self.runtime {
            return runtime.lock().map_or(Resolution::Physical, |mut keyboard| keyboard.resolve(code, mods));
        }
        self.dead.lock().map_or(Resolution::Physical, |mut dead| self.resolve_fixture(code, mods, &mut dead))
    }

    pub fn resolve_with(&self, code: u32, mods: Mods, translation: &mut Translation) -> Resolution {
        if translation.descriptor.as_ref() != Some(&self.descriptor) {
            translation.reset();
            translation.descriptor = Some(self.descriptor.clone());
        }
        if let Some(runtime) = &self.runtime {
            return runtime.lock().map_or(Resolution::Physical, |mut keyboard| keyboard.resolve_with(code, mods, &mut translation.native));
        }
        self.resolve_fixture(code, mods, &mut translation.dead)
    }

    fn resolve_fixture(&self, code: u32, mods: Mods, compose: &mut DeadKeys) -> Resolution {
        let held: Vec<_> = mods.keys().collect();
        let level = usize::from(held.iter().any(|code| matches!(code, 42 | 54)))
            + 2 * usize::from(held.contains(&100));
        let Some((_, levels)) = self.table.iter().find(|(key, _)| *key == code) else {
            *compose = DeadKeys::default();
            return Resolution::Physical;
        };
        if shortcut(mods, false) {
            *compose = DeadKeys::default();
            Resolution::Physical
        } else {
            levels[level].map_or(Resolution::Physical, |character| {
                let dead = self.descriptor.name == "us-intl" && matches!(code, 40 | 41);
                compose.feed(character.to_string(), dead)
            })
        }
    }

    pub fn reverse(&self, character: char) -> Option<Chord> {
        for level in 0..4 {
            if let Some((code, _)) = self.table.iter().find(|(_, levels)| levels[level] == Some(character)) {
                let mut modifiers = Vec::new();
                if level & 1 != 0 { modifiers.push(42); }
                if level & 2 != 0 { modifiers.push(100); }
                return Some(Chord { code: *code, modifiers });
            }
        }
        None
    }

    #[cfg(all(any(feature = "null-typing-tests", test), debug_assertions))]
    pub fn fake(name: &str) -> Self {
        assert!(matches!(name, "us" | "de" | "fr" | "pl" | "tr" | "us-intl" | "ar"));
        let mut table = Vec::new();
        for (codes, letters) in [
            (&[16, 17, 18, 19, 20, 21, 22, 23, 24, 25][..], "qwertyuiop"),
            (&[30, 31, 32, 33, 34, 35, 36, 37, 38][..], "asdfghjkl"),
            (&[44, 45, 46, 47, 48, 49, 50][..], "zxcvbnm"),
        ] {
            for (code, character) in codes.iter().zip(letters.chars()) {
                table.push((*code, [Some(character), character.to_uppercase().next(), None, None]));
            }
        }
        table.push((57, [Some(' '), Some(' '), None, None]));
        for (code, plain, shifted) in [(2, '1', '!'), (3, '2', '@'), (4, '3', '#'), (5, '4', '$'), (6, '5', '%'), (7, '6', '^'), (8, '7', '&'), (9, '8', '*'), (10, '9', '('), (11, '0', ')'), (12, '-', '_'), (13, '=', '+'), (26, '[', '{'), (27, ']', '}'), (39, ';', ':'), (40, '\'', '"'), (41, '`', '~'), (43, '\\', '|'), (51, ',', '<'), (52, '.', '>'), (53, '/', '?')] {
            table.push((code, [Some(plain), Some(shifted), None, None]));
        }
        let mut set = |code, plain, shifted, altgr, altgr_shift| {
            let levels = [Some(plain), Some(shifted), altgr, altgr_shift];
            if let Some(entry) = table.iter_mut().find(|(key, _)| *key == code) {
                entry.1 = levels;
            } else {
                table.push((code, levels));
            }
        };
        match name {
            "de" => {
                set(21, 'z', 'Z', None, None);
                set(44, 'y', 'Y', None, None);
                set(16, 'q', 'Q', Some('@'), None);
                set(18, 'e', 'E', Some('€'), None);
                set(26, 'ü', 'Ü', None, None);
                set(39, 'ö', 'Ö', None, None);
                set(40, 'ä', 'Ä', None, None);
                set(12, 'ß', '?', Some('\\'), None);
            }
            "fr" => {
                for (code, plain, shifted) in [(16, 'a', 'A'), (30, 'q', 'Q'), (17, 'z', 'Z'), (44, 'w', 'W'), (39, 'm', 'M'), (50, ',', '?'), (51, ';', '.'), (52, ':', '/'), (53, '!', '§')] {
                    set(code, plain, shifted, None, None);
                }
                for (code, plain, shifted, altgr) in [(3, 'é', '2', '~'), (4, '"', '3', '#'), (5, '\'', '4', '{'), (6, '(', '5', '['), (7, '-', '6', '|'), (8, 'è', '7', '`'), (9, '_', '8', '\\'), (10, 'ç', '9', '^'), (11, 'à', '0', '@'), (12, ')', '°', ']'), (13, '=', '+', '}')] {
                    set(code, plain, shifted, Some(altgr), None);
                }
            }
            "pl" => {
                for (code, plain, shifted, altgr, upper) in [(30, 'a', 'A', 'ą', 'Ą'), (18, 'e', 'E', 'ę', 'Ę'), (38, 'l', 'L', 'ł', 'Ł'), (44, 'z', 'Z', 'ż', 'Ż'), (45, 'x', 'X', 'ź', 'Ź'), (49, 'n', 'N', 'ń', 'Ń'), (24, 'o', 'O', 'ó', 'Ó'), (31, 's', 'S', 'ś', 'Ś'), (46, 'c', 'C', 'ć', 'Ć')] {
                    set(code, plain, shifted, Some(altgr), Some(upper));
                }
            }
            "tr" => {
                set(23, 'ı', 'I', None, None);
                set(40, 'i', 'İ', None, None);
                set(16, 'q', 'Q', Some('@'), None);
            }
            "ar" => {
                for (code, character) in [(16, 'ض'), (17, 'ص'), (18, 'ث'), (30, 'ش'), (31, 'س'), (32, 'ي'), (33, 'ب')] {
                    set(code, character, character, None, None);
                }
            }
            _ => {}
        }
        Self {
            descriptor: LayoutDescriptor { name: name.into(), variant: "test-fixture".into(), rules: "fixture-v1".into(), model: "pc105".into() },
            table,
            runtime: None,
            dead: Arc::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Inject;

    #[test]
    fn typing_native_descriptors_distinguish_layout_families() {
        let local = KeyboardLayout::descriptor_named("de");
        assert_eq!(local.rules, std::env::consts::OS);
        assert!(local.valid());
        let mut peer = local.clone();
        assert!(!one_mk_proto::typing::use_characters(one_mk_proto::typing::Mode::Auto, one_mk_proto::caps::TEXT, true, Some(&local), Some(&peer)));
        peer.rules = "another-platform".into();
        assert!(one_mk_proto::typing::use_characters(one_mk_proto::typing::Mode::Auto, one_mk_proto::caps::TEXT, true, Some(&local), Some(&peer)));
    }

    #[test]
    fn typing_composition_is_isolated_per_visit() {
        let source = KeyboardLayout::fake("us-intl");
        let mut first = Translation::default();
        let mut second = Translation::default();
        for state in [&mut first, &mut second] {
            assert_eq!(source.resolve_with(40, Mods::default(), state), Resolution::Pending);
        }
        first.reset();
        assert_eq!(source.resolve_with(18, Mods::default(), &mut first), Resolution::Text("e".into()));
        assert_eq!(source.resolve_with(18, Mods::default(), &mut second), Resolution::Text("é".into()));
        assert_eq!(source.resolve_with(40, Mods::default(), &mut second), Resolution::Pending);
        let changed = KeyboardLayout::fake("us");
        assert_eq!(changed.resolve_with(18, Mods::default(), &mut second), Resolution::Text("e".into()));
    }
    fn modifiers(codes: &[u32]) -> Mods {
        let mut mods = Mods::default();
        for code in codes { mods.update(*code, true); }
        mods
    }

    #[test]
    fn fixture_layouts_translate_current_levels_and_shortcuts() {
        for (layout, code, held, text) in [
            ("us", 21, &[][..], "y"), ("us", 21, &[42], "Y"),
            ("de", 40, &[], "ä"), ("de", 39, &[], "ö"), ("de", 26, &[], "ü"),
            ("de", 12, &[], "ß"), ("de", 16, &[100], "@"), ("de", 18, &[100], "€"),
            ("fr", 16, &[], "a"), ("fr", 30, &[42], "Q"), ("fr", 11, &[100], "@"),
            ("ar", 16, &[], "ض"), ("ar", 30, &[], "ش"), ("us-intl", 18, &[], "e"),
        ] {
            let keyboard = KeyboardLayout::fake(layout);
            assert_eq!(keyboard.resolve(code, modifiers(held)), Resolution::Text(text.into()), "{layout} {code}");
            for modifier in [29, 97, 56, 125, 126] {
                assert_eq!(keyboard.resolve(code, modifiers(&[modifier])), Resolution::Physical);
            }
        }
        assert!(shortcut(modifiers(&[29, 100]), false));
        assert!(!shortcut(modifiers(&[100]), false));
        assert!(!shortcut(modifiers(&[56]), true));
        assert!(shortcut(modifiers(&[56, 125]), true));
    }

    #[test]
    fn dead_key_sequences_cancel_on_shortcut_and_visit_end() {
        let keyboard = KeyboardLayout::fake("us-intl");
        for (accent, code, expected) in [(40, 18, "é"), (41, 30, "à"), (40, 57, "'"), (40, 45, "'x")] {
            assert_eq!(keyboard.resolve(accent, Mods::default()), Resolution::Pending);
            assert_eq!(keyboard.resolve(code, Mods::default()), Resolution::Text(expected.into()));
        }
        assert_eq!(keyboard.resolve(40, modifiers(&[42])), Resolution::Pending);
        assert_eq!(keyboard.resolve(22, Mods::default()), Resolution::Text("ü".into()));
        assert_eq!(keyboard.resolve(40, Mods::default()), Resolution::Pending);
        assert_eq!(keyboard.resolve(18, modifiers(&[29])), Resolution::Physical);
        assert_eq!(keyboard.resolve(18, Mods::default()), Resolution::Text("e".into()));
        assert_eq!(keyboard.resolve(40, Mods::default()), Resolution::Pending);
        keyboard.reset();
        assert_eq!(keyboard.resolve(18, Mods::default()), Resolution::Text("e".into()));
        assert_eq!(keyboard.resolve(40, Mods::default()), Resolution::Pending);
        assert_eq!(keyboard.resolve(1, Mods::default()), Resolution::Physical);
        assert_eq!(keyboard.resolve(18, Mods::default()), Resolution::Text("e".into()));
    }

    #[test]
    fn unicode_packets_pair_bmp_combining_and_surrogate_units() {
        for (text, units) in [("ä", vec![0xe4]), ("e\u{301}", vec![0x65, 0x301]), ("😀", vec![0xd83d, 0xde00])] {
            let expected: Vec<_> = units.into_iter().flat_map(|unit| [(unit, true), (unit, false)]).collect();
            assert_eq!(Text::from(text).unicode_keys(), expected);
        }
    }

    #[test]
    fn native_text_repeat_shortcuts_modifiers_and_release_cleanup() {
        let mut native = NativeText::default();
        let press = Inject::CharacterKey { code: 40, down: true, character: 'ä' };
        let text = Inject::Text { text: "ä".into() };
        assert_eq!(native.input(press), vec![text]);
        assert_eq!(native.repeat(40), Some(vec![text]));
        for modifier in [42, 100] {
            let key = Inject::Key { code: modifier, down: true };
            assert_eq!(native.input(key), vec![key]);
        }
        assert_eq!(native.repeat(40), Some(vec![
            Inject::Key { code: 42, down: false }, Inject::Key { code: 100, down: false }, text,
            Inject::Key { code: 100, down: true }, Inject::Key { code: 42, down: true },
        ]));
        native.input(Inject::Key { code: 29, down: true });
        assert_eq!(native.repeat(40), Some(Vec::new()));
        assert!(native.input(Inject::Text { text: "x".into() }).is_empty());
        native.input(Inject::Key { code: 29, down: false });
        assert!(!native.repeat(40).unwrap().is_empty());
        assert!(native.input(Inject::Key { code: 40, down: false }).is_empty());
        assert_eq!(native.repeat(40), None);
        native.input(press);
        native.clear();
        assert_eq!(native.repeat(40), None);
        assert_eq!(native.input(Inject::Text { text: "x".into() }), vec![Inject::Text { text: "x".into() }]);
    }

    #[test]
    fn native_typing_coalesced_visits_preserve_release_order() {
        let mut native = NativeText::default();
        let first = Inject::CharacterKey { code: 40, down: true, character: 'ä' };
        let second = Inject::CharacterKey { code: 26, down: true, character: 'ü' };
        let events = [
            Inject::Controlled(true), first, Inject::Key { code: 40, down: false },
            Inject::Controlled(false), Inject::Controlled(true), second,
        ];
        let output: Vec<_> = events.into_iter().flat_map(|event| native.input(event)).collect();
        assert_eq!(output, vec![
            Inject::Controlled(true), Inject::Text { text: "ä".into() },
            Inject::Controlled(false), Inject::Controlled(true), Inject::Text { text: "ü".into() },
        ]);
        assert_eq!(native.repeat(40), None);
        assert_eq!(native.repeat(26), Some(vec![Inject::Text { text: "ü".into() }]));
        native.input(Inject::CharacterKey { code: 26, down: false, character: 'x' });
        assert!(native.repeat(26).is_some());
        native.input(Inject::Controlled(false));
        assert_eq!(native.repeat(26), None);
        for (code, character) in [(29, 'x'), (30, '\n'), (767, 'x')] {
            assert!(native.input(Inject::CharacterKey { code, down: true, character }).is_empty());
            assert_eq!(native.repeat(code), None);
        }
    }

    #[test]
    fn injection_text_is_bounded_and_printable() {
        for text in ["@", "e\u{301}", "ı", "ł"] {
            assert_eq!(Text::new(text).unwrap().as_str(), text);
        }
        for text in ["", "ab", "\n", "\t", "\x1b", "\u{202e}", "\u{200d}", "123456789"] {
            assert!(Text::new(text).is_none());
        }
    }

    #[tokio::test]
    async fn null_records_both_character_primitives_without_native_injection() {
        let (backend, mut handles) = crate::null(one_mk_proto::Desktop::new(vec![one_mk_proto::Rect::new(0, 0, 100, 100)]));
        assert!(backend.keyboard.is_none());
        for event in [crate::Inject::Text { text: "@".into() }, crate::Inject::CharacterKey { code: 21, down: true, character: 'z' }] {
            backend.inject.send(event).unwrap();
            assert_eq!(handles.inject.recv().await.unwrap(), event);
        }
    }
}
