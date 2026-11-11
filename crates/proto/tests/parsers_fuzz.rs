//! Everything a person types (rules in settings.toml, command-line words,
//! device names) goes through these parsers: none may panic on any input,
//! and what prints must parse back to the same thing.

use one_mk_proto::device::{Bus, Fingerprint as Device};
use one_mk_proto::gesture::Direction;
use one_mk_proto::keynames::{self, Chord, Trigger};
use one_mk_proto::overlay::DrawMode;
use one_mk_proto::remap::Output;
use one_mk_proto::{Modifier, Side, appmatch::AppPattern, sanitize_name};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const TOKENS: &[&str] = &[
    "ctrl", "shift", "alt", "meta", "lctrl", "rmeta", "esc", "capslock", "a", "kp5", "f13", "+", "++", " ", "  ", "\t", "\n", "\u{0}",
    "code:", "code:58", "code:0x3a", "code:99999", "evdev:", "evdev:KEY_A", "0x", "-1", "layer:", "layer:nav", "run:", "switch", "switch next",
    "none", "back", "forward", "scroll", "scroll down 3", "up", "down", "lock-pointer", "on", "toggle", "!", "*", "**", "/", "\\", "%", "%2F",
    "%25", "%zz", ":", "046d:c52b", "usb", "Bluetooth", "é", "ß", "界", "😀", "\u{202e}", "\u{200b}", "\u{feff}", "\u{301}", "\u{0}", "9999999999999",
    "button4", "volume up", "ÿ", "İ", "ǅ", "ﬃ", "ẞ",
];

fn junk(rng: &mut Rng) -> String {
    let mut s = String::new();
    for _ in 0..rng.below(7) {
        if rng.below(8) == 0 {
            // A random scalar value, including non-characters and astral planes.
            s.push(char::from_u32((rng.next() % 0x11_0000) as u32).unwrap_or('\u{fffd}'));
        } else {
            s.push_str(TOKENS[rng.below(TOKENS.len())]);
        }
    }
    s
}

#[test]
fn no_parser_panics_on_junk_and_what_parses_prints_and_parses_again() {
    let mut rng = Rng(0xA5A5_1234_9876_5432);
    let (mut chords, mut triggers, mut outputs, mut patterns, mut devices) = (0, 0, 0, 0, 0);
    for _ in 0..6_000 {
        let s = junk(&mut rng);
        let _ = keynames::parse_key(&s);
        let _ = s.parse::<Side>();
        let _ = s.parse::<Modifier>();
        let _ = s.parse::<Direction>();
        let _ = s.parse::<DrawMode>();
        let _ = Bus::parse(&s);
        if let Ok(chord) = s.parse::<Chord>() {
            chords += 1;
            assert_eq!(chord.to_string().parse::<Chord>().as_ref(), Ok(&chord), "{s:?} prints as {chord}");
        }
        if let Ok(trigger) = s.parse::<Trigger>() {
            triggers += 1;
            assert_eq!(trigger.to_string().parse::<Trigger>().as_ref(), Ok(&trigger), "{s:?} prints as {trigger}");
        }
        if let Ok(output) = s.parse::<Output>() {
            outputs += 1;
            let printed = output.to_string();
            match &output {
                // A sequence is written as an array in settings, not as one word.
                Output::Sequence(_) => {}
                // A computer's name may hold anything but a line break, and prints verbatim.
                _ => assert_eq!(printed.parse::<Output>().as_ref(), Ok(&output), "{s:?} prints as {printed:?}"),
            }
        }
        if let Ok(pattern) = s.parse::<AppPattern>() {
            patterns += 1;
            assert_eq!(pattern.to_string().parse::<AppPattern>().as_ref(), Ok(&pattern), "{s:?}");
            // Matching never panics, whatever the names.
            let _ = pattern.matches(&[junk(&mut rng), junk(&mut rng)]);
            let _ = pattern.matches::<&str>(&[]);
        }
        if let Ok(device) = s.parse::<Device>() {
            devices += 1;
            assert_eq!(device.to_string().parse::<Device>().as_ref(), Ok(&device), "{s:?} prints as {device}");
        }
        let cleaned = sanitize_name(&s);
        assert!(!cleaned.is_empty() && cleaned.chars().count() <= 64);
        assert_eq!(sanitize_name(&cleaned), cleaned, "cleaning twice changes nothing: {s:?}");
    }
    assert!(chords > 30 && triggers > 30 && outputs > 30 && patterns > 30, "{chords} {triggers} {outputs} {patterns} {devices}");
}

#[test]
fn device_names_with_every_awkward_character_round_trip() {
    let mut rng = Rng(77);
    let pieces = ["/", "%", "%2F", "%25", ":", " ", "é", "界", "😀", "a", "0", "\u{a0}", "\u{3000}", "\\", "+", "!"];
    for _ in 0..20_000 {
        let word = |rng: &mut Rng| (0..rng.below(6)).map(|_| pieces[rng.below(pieces.len())]).collect::<String>();
        let (serial, name) = (word(&mut rng), word(&mut rng));
        let bus = Bus::ALL[rng.below(Bus::ALL.len())];
        let device = Device::new(bus, rng.next() as u16, rng.next() as u16, if rng.below(3) == 0 { None } else { Some(&serial) }, &name);
        if device.serial.is_none() && device.name.is_empty() {
            // Nothing to tell it from the same model: refused, with a reason.
            assert!(device.to_string().parse::<Device>().is_err());
            continue;
        }
        let back = device.to_string().parse::<Device>().unwrap_or_else(|e| panic!("{device:?} -> {device} -> {e}"));
        assert_eq!(back, device, "{device}");
    }
}

#[test]
fn every_key_name_prints_and_parses_back_alone_and_with_modifiers() {
    for (code, printed, names) in keynames::all_names() {
        assert_eq!(keynames::parse_key(printed), Ok(code), "{printed}");
        for name in names {
            assert_eq!(keynames::parse_key(name), Ok(code), "{name}");
        }
        for mods in 0..16u8 {
            let chord = Chord { mods, key: code as u16 };
            let text = chord.to_string();
            if text.contains("++") || text.ends_with('+') && names[0] == "+" {
                continue; // a plus key is written another way; asserted below
            }
            assert_eq!(text.parse::<Chord>(), Ok(chord), "{text}");
        }
    }
    // The documented way to write a plus key.
    assert!(keynames::parse_key("kpplus").is_ok() && keynames::parse_key("equal").is_ok());
    assert!("ctrl++".parse::<Chord>().is_err() && "+".parse::<Chord>().is_err() && "a+".parse::<Chord>().is_err());
    // Past the table: numbers are bounded, not wrapped.
    for bad in ["code:99999", "code:4294967296", "code:-1", "code:", "code:0x", "evdev:KEY_NOPE"] {
        assert!(keynames::parse_key(bad).is_err(), "{bad}");
    }
    assert_eq!(keynames::parse_key("CODE:0X3A"), keynames::parse_key("code:58"));
}
