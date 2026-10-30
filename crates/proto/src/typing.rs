use crate::Control;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_TEXT_BYTES: usize = 8;
pub const TEXT_PER_SECOND: u32 = 200;
pub const MAX_KEY_CODE: u32 = 767;

pub fn valid_key_code(code: u32) -> bool {
    (1..=MAX_KEY_CODE).contains(&code)
}

pub fn printable(character: char) -> bool {
    !matches!(
        get_general_category(character),
        GeneralCategory::Control | GeneralCategory::Format | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator | GeneralCategory::ParagraphSeparator
    ) && (get_general_category(character) != GeneralCategory::SpaceSeparator || character == ' ')
}

pub fn valid_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TEXT_BYTES
        && text.chars().all(printable) && text.graphemes(true).count() == 1
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutDescriptor {
    pub name: String,
    pub variant: String,
    pub rules: String,
    pub model: String,
}

impl LayoutDescriptor {
    pub fn valid(&self) -> bool {
        !self.name.is_empty() && [&self.name, &self.variant, &self.rules, &self.model]
            .into_iter().all(|field| field.len() <= 128 && field.chars().all(printable))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Physical,
    Characters,
    Auto,
}

pub fn use_characters(mode: Mode, peer_caps: u32, supported: bool, local: Option<&LayoutDescriptor>, peer: Option<&LayoutDescriptor>) -> bool {
    supported && peer_caps & crate::caps::TEXT != 0 && match mode {
        Mode::Physical => false,
        Mode::Characters => true,
        Mode::Auto => matches!((local, peer), (Some(local), Some(peer)) if local.valid() && peer.valid() && local != peer),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Physical,
    Text(String),
    Shortcut(char),
}

#[derive(Default)]
pub struct Sender {
    held: HashMap<u32, Resolution>,
}

impl Sender {
    pub fn key(&mut self, code: u32, down: bool, resolve: impl FnOnce() -> Resolution) -> Option<Control> {
        if !valid_key_code(code) {
            return None;
        }
        let resolution = if down {
            self.held.entry(code).or_insert_with(|| match if matches!(code, 29 | 42 | 54 | 56 | 58 | 69 | 70 | 97 | 100 | 125 | 126) { Resolution::Physical } else { resolve() } {
                Resolution::Text(text) if valid_text(&text) => Resolution::Text(text),
                Resolution::Shortcut(character) if printable(character) => Resolution::Shortcut(character),
                _ => Resolution::Physical,
            }).clone()
        } else {
            self.held.remove(&code)?
        };
        match resolution {
            Resolution::Physical => Some(Control::Key { code, down }),
            Resolution::Text(text) => down.then_some(Control::Text { text }),
            Resolution::Shortcut(character) => Some(Control::CharacterKey { code, down, character }),
        }
    }

    pub fn release_all(&mut self) -> Vec<Control> {
        let mut codes: Vec<_> = self.held.keys().copied().collect();
        codes.sort_unstable();
        codes.into_iter().filter_map(|code| self.key(code, false, || Resolution::Physical)).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub code: u32,
    pub modifiers: Vec<u32>,
}

impl Chord {
    fn normalized(mut self) -> Self {
        self.modifiers.retain(|modifier| *modifier != self.code);
        self.modifiers.sort_unstable();
        self.modifiers.dedup();
        self
    }
}

#[derive(Default)]
pub struct Receiver {
    held: HashMap<u32, Chord>,
    counts: BTreeMap<u32, usize>,
}

impl Receiver {
    pub fn key(&mut self, source: u32, down: bool, resolve: impl FnOnce() -> Chord) -> Vec<(u32, bool)> {
        if !valid_key_code(source) {
            return Vec::new();
        }
        if down {
            if let Some(chord) = self.held.get(&source) {
                return vec![(chord.code, true)];
            }
            let resolved = resolve();
            let chord = if valid_key_code(resolved.code) && resolved.modifiers.len() <= 3
                && resolved.modifiers.iter().all(|code| matches!(code, 42 | 54 | 100))
            {
                resolved.normalized()
            } else {
                Chord { code: source, modifiers: Vec::new() }
            };
            let mut events = Vec::new();
            for code in chord.modifiers.iter().copied().chain(std::iter::once(chord.code)) {
                let count = self.counts.entry(code).or_default();
                if *count == 0 {
                    events.push((code, true));
                }
                *count += 1;
            }
            self.held.insert(source, chord);
            events
        } else {
            let Some(chord) = self.held.remove(&source) else { return Vec::new() };
            let mut events = Vec::new();
            for code in std::iter::once(chord.code).chain(chord.modifiers.into_iter().rev()) {
                if let Some(count) = self.counts.get_mut(&code) {
                    *count -= 1;
                    if *count == 0 {
                        self.counts.remove(&code);
                        events.push((code, false));
                    }
                }
            }
            events
        }
    }

    pub fn release_all(&mut self) -> Vec<(u32, bool)> {
        let mut sources: Vec<_> = self.held.keys().copied().collect();
        sources.sort_unstable();
        sources.into_iter().flat_map(|source| self.key(source, false, || unreachable!())).collect()
    }
}

pub struct TextBudget {
    start: Instant,
    used: u32,
}

impl TextBudget {
    pub fn new(now: Instant) -> Self {
        Self { start: now, used: 0 }
    }

    pub fn accept(&mut self, text: &str, now: Instant) -> bool {
        if !valid_text(text) {
            return false;
        }
        if now.saturating_duration_since(self.start) >= Duration::from_secs(1) {
            self.start = now;
            self.used = 0;
        }
        if self.used >= TEXT_PER_SECOND {
            return false;
        }
        self.used += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_one_printable_bounded_cluster() {
        for text in ["@", "é", "e\u{301}", "ı", "ł", "€", " ", "😀"] {
            assert!(valid_text(text), "{text:?}");
        }
        for text in ["", "ab", "\n", "\r", "\t", "\x1b", "\0", "\u{202e}", "\u{2066}", "\u{200d}", "\u{2028}", "\u{e000}", "\u{a0}", "\u{feff}", "e\u{301}\u{301}\u{301}\u{301}"] {
            assert!(!valid_text(text), "{text:?}");
        }
    }

    #[test]
    fn all_unicode_controls_and_formats_are_refused() {
        for scalar in 0..=0x10ffff {
            if let Some(character) = char::from_u32(scalar)
                && matches!(get_general_category(character), GeneralCategory::Control | GeneralCategory::Format)
            {
                assert!(!valid_text(&character.to_string()), "{scalar:x}");
            }
        }
    }

    #[test]
    fn sender_remembers_text_and_physical_pairing_across_layout_changes() {
        let mut sender = Sender::default();
        assert_eq!(sender.key(21, true, || Resolution::Text("z".into())), Some(Control::Text { text: "z".into() }));
        assert_eq!(sender.key(21, true, || Resolution::Physical), Some(Control::Text { text: "z".into() }));
        assert_eq!(sender.key(21, false, || panic!("release never resolves")), None);
        assert_eq!(sender.key(30, true, || Resolution::Physical), Some(Control::Key { code: 30, down: true }));
        assert_eq!(sender.release_all(), vec![Control::Key { code: 30, down: false }]);
        assert!(sender.release_all().is_empty());
    }

    #[test]
    fn malformed_resolution_falls_back_for_entire_hold() {
        let mut sender = Sender::default();
        assert_eq!(sender.key(28, true, || Resolution::Text("\n".into())), Some(Control::Key { code: 28, down: true }));
        assert_eq!(sender.key(28, false, || unreachable!()), Some(Control::Key { code: 28, down: false }));
        assert!(sender.key(28, false, || unreachable!()).is_none());
    }

    #[test]
    fn modifiers_and_locks_always_keep_their_physical_pairing() {
        let mut sender = Sender::default();
        for code in [29, 42, 54, 56, 58, 69, 70, 97, 100, 125, 126] {
            assert_eq!(sender.key(code, true, || panic!("modifiers do not resolve")), Some(Control::Key { code, down: true }));
            assert_eq!(sender.key(code, false, || unreachable!()), Some(Control::Key { code, down: false }));
        }
    }

    #[test]
    fn receiver_retains_resolved_chord_and_shared_modifiers() {
        let mut receiver = Receiver::default();
        assert_eq!(receiver.key(21, true, || Chord { code: 44, modifiers: vec![42, 42] }), vec![(42, true), (44, true)]);
        assert_eq!(receiver.key(20, true, || Chord { code: 45, modifiers: vec![42] }), vec![(45, true)]);
        assert_eq!(receiver.key(21, true, || panic!("no repeat re-resolution")), vec![(44, true)]);
        assert_eq!(receiver.key(21, false, || panic!("no release re-resolution")), vec![(44, false)]);
        assert_eq!(receiver.release_all(), vec![(45, false), (42, false)]);
        assert!(receiver.release_all().is_empty());
    }

    #[test]
    fn random_layout_changes_preserve_held_invariant() {
        let mut receiver = Receiver::default();
        let mut random = 0x5e_u64;
        let mut held = BTreeMap::<u32, bool>::new();
        for _ in 0..100_000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let source = ((random >> 32) % 20) as u32;
            let down = random & 1 != 0;
            let target = ((random >> 40) % 30) as u32 + 100;
            let events = if random.is_multiple_of(101) {
                receiver.release_all()
            } else {
                receiver.key(source, down, || Chord { code: target, modifiers: vec![42] })
            };
            for (code, pressed) in events {
                if pressed {
                    held.insert(code, true);
                } else {
                    assert!(held.remove(&code).is_some(), "up without down: {code}");
                }
            }
            assert_eq!(held.keys().copied().collect::<Vec<_>>(), receiver.counts.keys().copied().collect::<Vec<_>>());
        }
        for (code, down) in receiver.release_all() {
            assert!(!down);
            assert!(held.remove(&code).is_some());
        }
        assert!(held.is_empty());
    }

    #[test]
    fn text_rate_limit_has_no_timer_and_charges_repeats() {
        let now = Instant::now();
        let mut budget = TextBudget::new(now);
        assert!(!budget.accept("\n", now));
        for _ in 0..TEXT_PER_SECOND {
            assert!(budget.accept("z", now));
        }
        assert!(!budget.accept("z", now));
        assert!(!budget.accept("z", now + Duration::from_millis(999)));
        assert!(budget.accept("z", now + Duration::from_secs(1)));
    }

    #[test]
    fn capabilities_and_valid_descriptors_are_required() {
        let german = LayoutDescriptor { name: "de".into(), variant: String::new(), rules: "evdev".into(), model: "pc105".into() };
        let us = LayoutDescriptor { name: "us".into(), ..german.clone() };
        assert!(use_characters(Mode::Auto, crate::caps::TEXT, true, Some(&german), Some(&us)));
        assert!(!use_characters(Mode::Auto, crate::caps::TEXT, true, Some(&us), Some(&us)));
        assert!(!use_characters(Mode::Auto, crate::caps::TEXT, true, Some(&german), None));
        assert!(!use_characters(Mode::Characters, 0, true, None, None));
        assert!(!use_characters(Mode::Characters, crate::caps::TEXT, false, None, None));
        assert_eq!(crate::caps::LOCAL & crate::caps::TEXT, 0);
    }

    #[test]
    fn wire_roundtrip_and_physical_key_compatibility() {
        let messages = [
            Control::Text { text: "e\u{301}".into() },
            Control::CharacterKey { code: 21, down: true, character: 'z' },
            Control::CharacterKey { code: 21, down: false, character: 'z' },
            Control::KeyboardLayout(Box::new(LayoutDescriptor { name: "de".into(), variant: "nodeadkeys".into(), rules: "evdev".into(), model: "pc105".into() })),
        ];
        for message in messages {
            let mut frame = Vec::new();
            message.encode_frame(&mut frame);
            assert_eq!(Control::decode(&frame[2..]).unwrap(), message);
            for length in 0..frame.len() - 2 {
                assert!(Control::decode(&frame[2..][..length]).is_err());
            }
            frame.push(0);
            assert!(Control::decode(&frame[2..]).is_err());
        }
        let mut frame = Vec::new();
        Control::Key { code: 21, down: true }.encode_frame(&mut frame);
        assert_eq!(frame, [6, 0, 5, 21, 0, 0, 0, 1]);
    }

    #[test]
    fn hostile_wire_text_and_shortcuts_are_rejected() {
        for bytes in [vec![21, 1, 0, 0xff], vec![21, 9, 0], vec![21, 1, 0, b'\n'], vec![21, 2, 0, b'a', b'b']] {
            assert!(Control::decode(&bytes).is_err(), "{bytes:?}");
        }
        for scalar in [0, 9, 10, 27, 0xd800, 0x110000, 0x202e, 0x2066] {
            let mut bytes = vec![22, 21, 0, 0, 0, 1];
            bytes.extend_from_slice(&u32::to_le_bytes(scalar));
            assert!(Control::decode(&bytes).is_err());
        }
        let mut bytes = vec![22, 21, 0, 0, 0, 2];
        bytes.extend_from_slice(&u32::to_le_bytes('z' as u32));
        assert!(Control::decode(&bytes).is_err());
        let message = Control::KeyboardLayout(Box::new(LayoutDescriptor { name: "x".repeat(129), variant: String::new(), rules: String::new(), model: String::new() }));
        let mut frame = Vec::new();
        message.encode_frame(&mut frame);
        assert!(Control::decode(&frame[2..]).is_err());
    }

    #[test]
    fn random_new_protocol_frames_never_panic() {
        let mut random = 0x5e_u64;
        for length in 0..128 {
            for tag in 21..=23 {
                for _ in 0..100 {
                    let mut bytes = vec![tag];
                    for _ in 0..length {
                        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                        bytes.push((random >> 32) as u8);
                    }
                    let _ = Control::decode(&bytes);
                }
            }
        }
    }

    #[test]
    fn invalid_sources_and_chords_cannot_grow_held_state() {
        let mut sender = Sender::default();
        let mut receiver = Receiver::default();
        for code in [0, MAX_KEY_CODE + 1, u32::MAX] {
            assert!(sender.key(code, true, || panic!("invalid source never resolves")).is_none());
            assert!(receiver.key(code, true, || panic!("invalid source never resolves")).is_empty());
        }
        for chord in [
            Chord { code: u32::MAX, modifiers: vec![] },
            Chord { code: 44, modifiers: vec![29] },
            Chord { code: 44, modifiers: vec![42; 4] },
        ] {
            assert_eq!(receiver.key(21, true, || chord), vec![(21, true)]);
            assert_eq!(receiver.release_all(), vec![(21, false)]);
        }
        assert!(sender.held.is_empty());
        assert!(receiver.held.is_empty());
        assert!(receiver.counts.is_empty());
    }

    #[test]
    fn oversized_outgoing_typing_values_stay_bounded_and_are_refused() {
        for message in [
            Control::Text { text: "x".repeat(100_000) },
            Control::KeyboardLayout(Box::new(LayoutDescriptor {
                name: "x".repeat(100_000), variant: String::new(), rules: String::new(), model: String::new(),
            })),
        ] {
            let mut frame = Vec::new();
            message.encode_frame(&mut frame);
            assert!(frame.len() <= 523);
            assert!(Control::decode(&frame[2..]).is_err());
        }
    }
}
