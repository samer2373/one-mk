use one_mk_proto::typing::{Chord, LayoutDescriptor, Resolution};
use one_mk_proto::Mods;

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
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::new(text).expect("one bounded printable grapheme")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardLayout {
    descriptor: LayoutDescriptor,
    table: Vec<(u32, [Option<char>; 4])>,
}

impl KeyboardLayout {
    pub fn descriptor(&self) -> &LayoutDescriptor {
        &self.descriptor
    }

    pub fn resolve(&self, code: u32, mods: Mods) -> Resolution {
        let held: Vec<_> = mods.keys().collect();
        let shortcut = held.iter().any(|code| matches!(code, 29 | 97 | 56 | 125 | 126));
        let level = usize::from(held.iter().any(|code| matches!(code, 42 | 54)))
            + 2 * usize::from(held.contains(&100));
        let Some((_, levels)) = self.table.iter().find(|(key, _)| *key == code) else {
            return Resolution::Physical;
        };
        if shortcut {
            levels[0].map_or(Resolution::Physical, Resolution::Shortcut)
        } else {
            levels[level].map_or(Resolution::Physical, |character| Resolution::Text(character.to_string()))
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

    #[cfg(all(feature = "null-typing-tests", debug_assertions))]
    pub fn fake(name: &str) -> Self {
        assert!(matches!(name, "us" | "de" | "fr" | "pl" | "tr"));
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
            _ => {}
        }
        Self {
            descriptor: LayoutDescriptor { name: name.into(), variant: "test-fixture".into(), rules: "fixture-v1".into(), model: "pc105".into() },
            table,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
