//! Keyboard layouts through the real XKB tables: what each key types with Shift, AltGr and Caps
//! Lock, whether every typed character can be asked for again (the text-mode path: a character on
//! the sending keyboard must come out as the same character on the other), and layout groups.
//! These use the system's libxkbcommon and XKB data; where either is missing the test says so and
//! passes, as the existing XKB tests do.

use crate::{LayoutResolver, LayoutState};
use one_mk_proto::Mods;

const SHIFT: u32 = 42;
const ALTGR: u32 = 100;
const LAYOUTS: &[&str] = &[
    "us", "us(dvorak)", "us(colemak)", "us(intl)", "gb", "de", "de(nodeadkeys)", "fr", "es", "it", "pt", "br", "se", "no", "fi", "dk", "ch(fr)", "ch(de)",
    "be", "cz", "pl", "hu", "tr", "ru", "gr", "il", "jp", "latam", "ca(fr)", "ua", "ro", "sk", "hr", "is", "ie",
];

fn keymap(symbols: &str) -> String {
    format!(
        "xkb_keymap {{ xkb_keycodes {{ include \"evdev+aliases(qwerty)\" }}; xkb_types {{ include \"complete\" }}; \
         xkb_compat {{ include \"complete\" }}; xkb_symbols {{ include \"pc+{symbols}+inet(evdev)\" }}; }};"
    )
}

fn resolver(symbols: &str) -> Option<LayoutResolver> {
    if !std::path::Path::new("/usr/share/X11/xkb/symbols/us").exists() {
        eprintln!("SKIP: no system XKB data");
        return None;
    }
    LayoutResolver::from_keymap(keymap(symbols).as_bytes())
}

fn with(resolver: &LayoutResolver, keys: &[u32], locked: u32, group: u32) -> LayoutState {
    let base = LayoutState { locked, group, ..LayoutState::default() };
    resolver.masks_for(Mods::from_keys(keys.iter().copied()), base).expect("masks")
}

/// Every character any key of any common layout types (plain, Shift, AltGr, Shift+AltGr) can be
/// asked for again, and the keys it names type exactly that character.
#[test]
fn every_character_a_layout_types_can_be_typed_again_from_its_own_table() {
    let (mut layouts, mut pairs, mut missing) = (0, 0, Vec::new());
    for symbols in LAYOUTS {
        let Some(resolver) = resolver(symbols) else {
            missing.push(*symbols);
            continue;
        };
        layouts += 1;
        for keys in [&[][..], &[SHIFT], &[ALTGR], &[SHIFT, ALTGR]] {
            let state = with(&resolver, keys, 0, 0);
            for code in 1..=255u32 {
                let Some(text) = resolver.resolve(code, state) else { continue };
                let mut chars = text.chars();
                let (Some(character), None) = (chars.next(), chars.next()) else { continue };
                pairs += 1;
                let chord = resolver.reverse(character, 0).unwrap_or_else(|| panic!("{symbols}: {character:?} (key {code}, {keys:?}) cannot be asked for again"));
                let again = with(&resolver, &chord.modifiers, 0, 0);
                assert_eq!(
                    resolver.resolve(chord.code, again).as_deref(),
                    Some(text.as_str()),
                    "{symbols}: {character:?} -> {chord:?} types something else"
                );
                // A chord uses at most Shift and AltGr: nothing a person could not press.
                assert!(chord.modifiers.iter().all(|m| [SHIFT, ALTGR].contains(m)), "{chord:?}");
            }
        }
    }
    if layouts == 0 {
        eprintln!("SKIP: libxkbcommon unavailable");
        return;
    }
    assert!(layouts >= 20, "only {layouts} layouts compiled (missing: {missing:?})");
    assert!(pairs > 5000, "{pairs} characters checked");
}

#[test]
fn shift_altgr_and_caps_lock_choose_the_right_symbol() {
    let Some(us) = resolver("us") else { return };
    let typed = |resolver: &LayoutResolver, code: u32, keys: &[u32], caps: bool| resolver.resolve(code, with(resolver, keys, if caps { 2 } else { 0 }, 0));
    // Letters: Caps Lock capitalises, and Shift undoes it.
    assert_eq!(typed(&us, 30, &[], false).as_deref(), Some("a"));
    assert_eq!(typed(&us, 30, &[SHIFT], false).as_deref(), Some("A"));
    assert_eq!(typed(&us, 30, &[], true).as_deref(), Some("A"));
    assert_eq!(typed(&us, 30, &[SHIFT], true).as_deref(), Some("a"));
    // Digits and punctuation ignore Caps Lock.
    assert_eq!(typed(&us, 2, &[], true).as_deref(), Some("1"));
    assert_eq!(typed(&us, 2, &[SHIFT], true).as_deref(), Some("!"));
    assert_eq!(typed(&us, 26, &[], true).as_deref(), Some("["));
    // US has no AltGr characters: it types the plain one, not nothing.
    assert_eq!(typed(&us, 30, &[ALTGR], false).as_deref(), Some("a"));
    if let Some(de) = resolver("de") {
        for (code, plain, shift, altgr, shift_altgr) in [(16u32, "q", "Q", "@", "Ω"), (18, "e", "E", "€", "€"), (12, "ß", "?", "\\", "¿")] {
            assert_eq!(typed(&de, code, &[], false).as_deref(), Some(plain), "de key {code}");
            assert_eq!(typed(&de, code, &[SHIFT], false).as_deref(), Some(shift), "de key {code} shifted");
            assert_eq!(typed(&de, code, &[ALTGR], false).as_deref(), Some(altgr), "de key {code} with AltGr");
            assert_eq!(typed(&de, code, &[SHIFT, ALTGR], false).as_deref(), Some(shift_altgr), "de key {code} with Shift+AltGr");
        }
        // AltGr+E is the euro sign whatever Caps Lock says; Caps capitalises ä, ö, ü like any letter.
        assert_eq!(typed(&de, 18, &[ALTGR], true).as_deref(), Some("€"));
        assert_eq!(typed(&de, 39, &[], true).as_deref(), Some("Ö"));
        assert_eq!(typed(&de, 39, &[SHIFT], true).as_deref(), Some("ö"));
    }
    if let Some(fr) = resolver("fr") {
        // On a French keyboard the digit row needs Shift for digits and Caps Lock does not help.
        assert_eq!(typed(&fr, 2, &[], true).as_deref(), Some("&"));
        assert_eq!(typed(&fr, 2, &[SHIFT], false).as_deref(), Some("1"));
        assert_eq!(typed(&fr, 16, &[], false).as_deref(), Some("a"));
        assert_eq!(typed(&fr, 11, &[ALTGR], false).as_deref(), Some("@"));
    }
}

#[test]
fn a_second_layout_is_a_second_group_and_text_is_looked_up_in_the_one_in_use() {
    let Some(both) = resolver("us+ru:2") else { return };
    assert_eq!(both.layout_name(0).as_deref().map(|n| n.to_lowercase().contains("english") || n.to_lowercase().contains("us")), Some(true), "{:?}", both.layout_name(0));
    assert!(both.layout_name(1).is_some_and(|n| n.to_lowercase().contains("russian")), "{:?}", both.layout_name(1));
    assert!(both.layout_name(7).is_none(), "no such group");
    let at = |group: u32| both.resolve(30, with(&both, &[], 0, group));
    assert_eq!((at(0).as_deref(), at(1).as_deref()), (Some("a"), Some("ф")));
    // Asking for a character looks in the group that is active, and finds nothing in the other.
    assert!(both.reverse('ф', 1).is_some());
    assert!(both.reverse('ф', 0).is_none());
    assert!(both.reverse('a', 0).is_some() && both.reverse('a', 1).is_none());
    // The chord a Russian keyboard needs for a capital comes with Shift.
    let capital = both.reverse('Ф', 1).unwrap();
    assert_eq!((capital.code, capital.modifiers), (30, vec![SHIFT]));
}

#[test]
fn hostile_requests_to_a_layout_return_nothing_and_never_panic() {
    let Some(de) = resolver("de") else { return };
    let state = LayoutState::default();
    for code in [0u32, 256, 1000, u32::MAX - 8, u32::MAX] {
        assert_eq!(de.resolve(code, state), None, "{code}");
    }
    // Characters that can never be typed as text: controls, format characters, private use,
    // unassigned, separators other than the plain space, and ones no layout has.
    for character in ['\0', '\n', '\t', '\u{7f}', '\u{200d}', '\u{202e}', '\u{e000}', '\u{a0}', '\u{2028}', '\u{10ffff}', '\u{378}'] {
        assert_eq!(de.reverse(character, 0), None, "{character:?}");
    }
    assert!(de.reverse(' ', 0).is_some(), "a space is the space bar");
    let _ = de.reverse('a', 999); // a group that is not there: the library clamps it, and nothing breaks
    // Wild modifier masks do not crash the lookup.
    for mask in [0u32, 1, 0xFF, 0xFFFF, u32::MAX] {
        let _ = de.resolve(30, LayoutState { depressed: mask, latched: mask, locked: mask, group: mask });
    }
    // Keymaps from outside are bounded and must compile.
    assert!(LayoutResolver::from_keymap(&vec![b'x'; 1024 * 1024 + 1]).is_none(), "over a megabyte is refused outright");
    assert!(LayoutResolver::from_keymap(b"not a keymap").is_none());
    assert!(LayoutResolver::from_keymap(b"").is_none());
}
