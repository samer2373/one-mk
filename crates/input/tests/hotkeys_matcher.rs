//! Keyboard shortcuts: parsing whatever a person types, and the matcher that spots shortcuts in
//! captured input. The matcher must fire on exactly the configured chord, never in the middle of
//! other typing, swallow what it fired on (press, repeats and release) and nothing else, and
//! never leave a key swallowed that nobody will release.

use one_mk_input::hotkeys::{
    Action, Bindings, Chord, ChordMods, Debounce, EMERGENCY, JumpChords, Matcher, Source, Verdict,
};
use one_mk_proto::Mods;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

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

fn chord(s: &str) -> Chord {
    s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
}

#[test]
fn shortcut_text_never_panics_and_what_parses_prints_and_parses_again() {
    let mut rng = Rng(0x040C_4E75);
    let words = [
        "ctrl", "alt", "shift", "super", "cmd", "option", "control", "win", "meta", "logo", "1", "9", "a", "z", "f1",
        "f24", "esc", "space", "pagedown", "left", "{n}", "+", "++", " ", "", "é", "界", "😀", "\u{0}", "ctrl+",
        "+alt", "CTRL", "Alt", "kp5", "enter", "tab",
    ];
    let (mut parsed, mut jumps) = (0, 0);
    for _ in 0..40_000 {
        let text: String = (0..1 + rng.below(5))
            .map(|_| words[rng.below(words.len())])
            .collect::<Vec<_>>()
            .join(if rng.below(6) == 0 { "" } else { "+" });
        if let Ok(c) = text.parse::<Chord>() {
            parsed += 1;
            let printed = c.to_string();
            assert_eq!(printed.parse::<Chord>(), Ok(c), "{text:?} prints as {printed:?}");
            // Every printed form can be shown to people and handed to the desktop and to Windows.
            assert!(!c.label().is_empty() && !c.xdg_trigger().is_empty());
            assert!(c.vk().is_some(), "{printed} has a Windows key");
            // A shortcut can never swallow plain typing.
            let key = printed.rsplit('+').next().unwrap();
            let function_key =
                key.strip_prefix('f').is_some_and(|n| n.parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)));
            assert!(
                !(c.mods.is_empty() || c.mods == ChordMods::SHIFT)
                    || function_key
                    || ["print", "pause", "sysrq"].contains(&key),
                "{printed}"
            );
        }
        if let Ok(j) = text.parse::<JumpChords>() {
            jumps += 1;
            assert_eq!(j.to_string().parse::<JumpChords>(), Ok(j), "{text:?}");
            for n in 1..=9u8 {
                assert_eq!(j.chord(n).to_string().parse::<Chord>(), Ok(j.chord(n)));
            }
        }
    }
    assert!(parsed > 500 && jumps > 30, "{parsed} chords, {jumps} jump templates");
    assert!("a".repeat(65).parse::<Chord>().is_err() && format!("ctrl+{}", "a".repeat(70)).parse::<Chord>().is_err());
}

#[test]
fn chords_mean_what_the_documentation_says() {
    // A Mac's keycaps are called Control and Option.
    let (ctrl, alt) = if cfg!(target_os = "macos") { ("Control", "Option") } else { ("Ctrl", "Alt") };
    for (text, label) in [("ctrl+alt+1", format!("{ctrl}+{alt}+1")), ("CTRL + ALT + PageDown", format!("{ctrl}+{alt}+PageDown"))] {
        assert_eq!(chord(text).label(), label.as_str(), "{text}");
    }
    assert_eq!(chord("cmd+1"), chord("super+1"));
    assert_eq!(chord("option+1"), chord("alt+1"));
    assert_eq!(chord("ctrl+alt+1").xdg_trigger(), "CTRL+ALT+1");
    assert_eq!(chord("super+1").xdg_trigger(), "LOGO+1");
    for refused in [
        "",
        "1",
        "shift+1",
        "a",
        "ctrl",
        "ctrl+alt",
        "ctrl+ctrl+1",
        "ctrl+alt+nope",
        "ctrl++1",
        "+1",
        "1+",
        "ctrl+alt+shift+super+esc+1",
    ] {
        let result = refused.parse::<Chord>();
        assert!(result.is_err() || refused.contains("esc"), "{refused:?} was accepted as {result:?}");
    }
    // Function keys are allowed on their own: nothing else in typing uses them.
    assert!("f13".parse::<Chord>().is_ok() && "print".parse::<Chord>().is_ok());
    assert!("{n}".parse::<JumpChords>().is_err(), "a bare digit would take over typing");
    assert_eq!("ctrl+alt+{n}".parse::<JumpChords>().unwrap().chord(3), chord("ctrl+alt+3"));
    assert_eq!(
        "ctrl+alt+{n}".parse::<JumpChords>().unwrap().chord(0),
        chord("ctrl+alt+1"),
        "out-of-range numbers are held to 1..9"
    );
    assert_eq!("ctrl+alt+{n}".parse::<JumpChords>().unwrap().chord(200), chord("ctrl+alt+9"));
    assert!("ctrl+alt+1".parse::<JumpChords>().is_err());
}

#[test]
fn bindings_refuse_what_cannot_work() {
    let ok = Bindings::new(vec![(Action::Cycle, chord("ctrl+alt+n")), (Action::Home, chord("ctrl+alt+h"))]).unwrap();
    assert_eq!(ok.action(chord("ctrl+alt+n")), Some(Action::Cycle));
    assert_eq!(ok.action(chord("ctrl+alt+m")), None);
    assert!(
        Bindings::new(vec![(Action::Cycle, chord("ctrl+alt+n")), (Action::Home, chord("ctrl+alt+n"))]).is_err(),
        "one chord, two actions"
    );
    assert!(Bindings::new(vec![(Action::Home, EMERGENCY)]).is_err(), "the emergency chord is not for rebinding");
    assert!(Bindings::new(vec![]).unwrap().is_empty());
    for action in (1..=9).map(Action::Jump).chain([
        Action::Cycle,
        Action::LockPointer,
        Action::Home,
        Action::Broadcast,
        Action::FetchClipboard,
    ]) {
        assert_eq!(Action::from_id(&action.id()), Some(action));
        assert!(!action.describe().is_empty());
    }
    for bad in ["", "jump-0", "jump-10", "jump--1", "jump-1x", "JUMP-1", "cycle ", "jump-256", "jump-", "home\n"] {
        assert_eq!(Action::from_id(bad), None, "{bad:?}");
    }
}

/// A held-modifier tracker the way a backend keeps one, then the matcher on top, over random typing.
#[test]
fn the_matcher_fires_only_on_an_exact_chord_and_swallows_only_what_it_fired() {
    let mut rng = Rng(0x04A7_C4E5);
    let bindings = Bindings::new(vec![
        (Action::Jump(1), chord("ctrl+alt+1")),
        (Action::Jump(2), chord("ctrl+alt+2")),
        (Action::Cycle, chord("ctrl+alt+n")),
        (Action::Home, chord("super+h")),
        (Action::Broadcast, chord("f9")),
    ])
    .unwrap();
    let keys = [2u32, 3, 49, 35, 67, 30, 31, 57, 28];
    let modifiers = [29u32, 97, 56, 100, 42, 54, 125, 126];
    let (mut fired, mut swallowed_total) = (0, 0);
    for _ in 0..400 {
        let mut matcher = Matcher::default();
        let mut mods = Mods::default();
        let mut down: BTreeSet<u32> = BTreeSet::new();
        let mut fired_keys: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..300 {
            let (code, is_down) = match rng.below(5) {
                0 | 1 => {
                    let code = keys[rng.below(keys.len())];
                    (code, !down.contains(&code) || rng.below(4) == 0) // sometimes a repeat
                }
                2 => {
                    let held: Vec<u32> = down.iter().copied().collect();
                    match held.len() {
                        0 => continue,
                        n => (held[rng.below(n)], false),
                    }
                }
                _ => {
                    let code = modifiers[rng.below(modifiers.len())];
                    (code, !down.contains(&code))
                }
            };
            let held_before = mods;
            if Mods::is_modifier(code) {
                mods.update(code, is_down);
                assert_eq!(
                    matcher.key(&bindings, held_before, code, is_down),
                    Verdict::Pass,
                    "a modifier is never taken"
                );
                if is_down {
                    down.insert(code)
                } else {
                    down.remove(&code)
                };
                continue;
            }
            let other_keys_down = down.iter().any(|k| *k != code && !Mods::is_modifier(*k));
            let verdict = matcher.key(&bindings, held_before, code, is_down);
            let is_repeat = is_down && down.contains(&code);
            match verdict {
                Verdict::Fire(action) => {
                    fired += 1;
                    assert!(is_down && !is_repeat, "only a fresh press fires");
                    assert!(!other_keys_down, "never in the middle of other typing");
                    let chord = Chord { mods: one_mk_input::hotkeys::ChordMods::from_held(held_before), key: code };
                    assert_eq!(bindings.action(chord), Some(action), "exactly the held modifiers and this key");
                    assert!(fired_keys.insert(code), "{code} fired while its earlier press is still swallowed");
                }
                Verdict::Swallow => {
                    swallowed_total += 1;
                    assert!(fired_keys.contains(&code), "{code} swallowed but it never fired");
                    if !is_down {
                        fired_keys.remove(&code);
                    }
                }
                Verdict::Pass => {
                    assert!(
                        !fired_keys.contains(&code),
                        "{code} fired and its next event was let through (a stuck key on the other side)"
                    );
                }
            }
            if is_down {
                down.insert(code)
            } else {
                down.remove(&code)
            };
        }
        // Capture ends with keys held: the matcher forgets them, and the next capture starts clean.
        matcher.reset();
        assert_eq!(
            matcher.key(&bindings, Mods::default(), 49, false),
            Verdict::Pass,
            "a release nobody pressed passes"
        );
    }
    assert!(fired > 20 && swallowed_total > 20, "{fired} fired, {swallowed_total} swallowed");
}

#[test]
fn a_shortcut_pressed_mid_chord_or_with_extra_modifiers_does_nothing() {
    let bindings = Bindings::new(vec![(Action::Jump(1), chord("ctrl+alt+1"))]).unwrap();
    let held = |keys: &[u32]| Mods::from_keys(keys.iter().copied());
    let mut m = Matcher::default();
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, true), Verdict::Fire(Action::Jump(1)));
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, true), Verdict::Swallow, "its repeat");
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, false), Verdict::Swallow, "its release");
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, false), Verdict::Pass, "a second release is a stray");
    // Right Ctrl counts like the left one; the right Alt does not (it is AltGr, and AltGr+1 is typing).
    assert_eq!(m.key(&bindings, held(&[97, 56]), 2, true), Verdict::Fire(Action::Jump(1)));
    m.key(&bindings, held(&[97, 56]), 2, false);
    assert_eq!(m.key(&bindings, held(&[29, 100]), 2, true), Verdict::Pass);
    m.key(&bindings, held(&[29, 100]), 2, false);
    // An extra Shift, a missing Alt: another chord.
    assert_eq!(m.key(&bindings, held(&[29, 56, 42]), 2, true), Verdict::Pass);
    m.key(&bindings, held(&[29, 56, 42]), 2, false);
    assert_eq!(m.key(&bindings, held(&[29]), 2, true), Verdict::Pass);
    m.key(&bindings, held(&[29]), 2, false);
    // Another key is already down: typing, not a shortcut (Ctrl+Alt+A then 1).
    assert_eq!(m.key(&bindings, held(&[29, 56]), 30, true), Verdict::Pass);
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, true), Verdict::Pass);
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, false), Verdict::Pass);
    assert_eq!(m.key(&bindings, held(&[29, 56]), 30, false), Verdict::Pass);
    // Hostile codes are passed and remembered harmlessly.
    for code in [0u32, 767, 768, 100_000, u32::MAX] {
        assert_eq!(m.key(&bindings, Mods::default(), code, true), Verdict::Pass);
        assert_eq!(m.key(&bindings, Mods::default(), code, false), Verdict::Pass);
    }
    // After all of it the matcher still works.
    assert_eq!(m.key(&bindings, held(&[29, 56]), 2, true), Verdict::Fire(Action::Jump(1)));
}

#[test]
fn one_press_reported_by_the_system_and_by_the_matcher_counts_once() {
    let t0 = Instant::now();
    let mut debounce = Debounce::default();
    // The system reports, then the matcher, 20 ms apart: once.
    assert!(debounce.admit(Action::Cycle, Source::System, t0));
    assert!(!debounce.admit(Action::Cycle, Source::Captured, t0 + Duration::from_millis(20)));
    // A later deliberate press is a new one, from either.
    assert!(debounce.admit(Action::Cycle, Source::Captured, t0 + Duration::from_millis(900)));
    assert!(
        debounce.admit(Action::Cycle, Source::Captured, t0 + Duration::from_millis(1000)),
        "two presses from the same source are two presses"
    );
    // A different action is never swallowed by another's report.
    assert!(debounce.admit(Action::Home, Source::System, t0 + Duration::from_millis(1010)));
    assert!(debounce.admit(Action::Cycle, Source::Captured, t0 + Duration::from_millis(1020)));
    // A clock that went backwards does not wedge it.
    assert!(debounce.admit(Action::Home, Source::Captured, t0));
}
