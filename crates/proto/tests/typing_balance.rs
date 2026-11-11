//! Typing mode turns characters into key chords on the receiving computer. Whatever the order of
//! presses and releases (a Shift held on its own and also needed by a character, two characters
//! that share AltGr, repeats, hostile chords), the physical keys pressed over there must balance:
//! never pressed twice, never released when not down, and none left down once everything is let go.

use one_mk_proto::Control;
use one_mk_proto::typing::{Chord, Receiver, Resolution, Sender, TextBudget, valid_text};
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

/// The physical keyboard on the receiving computer.
#[derive(Default)]
struct Keyboard {
    down: BTreeSet<u32>,
}

impl Keyboard {
    fn apply(&mut self, events: &[(u32, bool)], context: &str) {
        for &(code, down) in events {
            if down {
                // A repeat of a held key is allowed (it is a repeat); it must not look like a new press.
                self.down.insert(code);
            } else {
                assert!(self.down.remove(&code), "{context}: {code} released but not down; events {events:?}");
            }
        }
    }
}

fn random_chord(rng: &mut Rng) -> Chord {
    let all = [42u32, 54, 100];
    let modifiers: Vec<u32> = all.iter().copied().filter(|_| rng.below(3) == 0).collect();
    Chord { code: [16u32, 17, 18, 30, 31, 57, 28, 2][rng.below(8)], modifiers }
}

#[test]
fn physical_keys_balance_however_characters_and_modifiers_interleave() {
    let mut rng = Rng(0x0B41_A4CE);
    for round in 0..600 {
        let mut receiver = Receiver::default();
        let mut keyboard = Keyboard::default();
        let mut held: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..200 {
            // Sources: real modifier keys (physical) and letter keys that may resolve to chords.
            let source = [42u32, 100, 30, 31, 32, 33, 34, 57, 28][rng.below(9)];
            if held.contains(&source) && rng.below(2) == 0 {
                let events = receiver.key(source, false, || unreachable!("a release needs no lookup"));
                keyboard.apply(&events, &format!("round {round}: release {source}"));
                held.remove(&source);
            } else {
                let physical = [42u32, 100].contains(&source);
                let chord = random_chord(&mut rng);
                let events = if physical {
                    receiver.physical_key(source, true)
                } else {
                    receiver.key(source, true, || chord.clone())
                };
                keyboard.apply(&events, &format!("round {round}: press {source}"));
                held.insert(source);
            }
            // The keyboard is exactly what the receiver says is held: no stuck, no missing.
            for modifier in [42u32, 100] {
                if held.contains(&modifier) {
                    assert!(keyboard.down.contains(&modifier), "physical {modifier} is still held but was released");
                }
            }
        }
        let events = receiver.release_all();
        keyboard.apply(&events, "release_all");
        assert!(keyboard.down.is_empty(), "round {round}: left down after release_all: {:?}", keyboard.down);
        assert!(receiver.release_all().is_empty(), "and a second release has nothing to do");
        // After it the receiver is as new.
        let again = receiver.physical_key(30, true);
        assert_eq!(again, [(30, true)]);
    }
}

#[test]
fn a_shared_modifier_stays_down_until_its_last_user_lets_go() {
    let mut r = Receiver::default();
    let chord = |code, modifiers: &[u32]| Chord { code, modifiers: modifiers.to_vec() };
    // Two characters on one AltGr: the AltGr goes down once, up with the second.
    assert_eq!(r.key(16, true, || chord(16, &[100])), [(100, true), (16, true)]);
    assert_eq!(r.key(17, true, || chord(17, &[100])), [(17, true)]);
    assert_eq!(r.key(16, false, || unreachable!()), [(16, false)]);
    assert_eq!(r.key(17, false, || unreachable!()), [(17, false), (100, false)]);
    // The user holds Shift as well as typing a shifted character: Shift outlives the character.
    assert_eq!(r.physical_key(42, true), [(42, true)]);
    assert_eq!(r.key(30, true, || chord(30, &[42])), [(30, true)]);
    assert_eq!(r.key(30, false, || unreachable!()), [(30, false)]);
    assert_eq!(r.physical_key(42, false), [(42, false)]);
    // And the other order: the character holds the Shift the user lets go of first.
    assert_eq!(r.key(30, true, || chord(30, &[42])), [(42, true), (30, true)]);
    assert_eq!(r.physical_key(42, true), []);
    assert_eq!(r.physical_key(42, false), []);
    assert_eq!(r.key(30, false, || unreachable!()), [(30, false), (42, false)]);
    // A repeat of a held key reports its key again (the platform repeats it), and starts nothing.
    assert_eq!(r.key(30, true, || chord(30, &[])), [(30, true)]);
    assert_eq!(r.key(30, true, || panic!("a repeat keeps its chord")), [(30, true)]);
    assert_eq!(r.key(30, false, || unreachable!()), [(30, false)]);
    assert_eq!(r.key(30, false, || unreachable!()), [], "a stray release");
}

#[test]
fn chords_from_a_hostile_resolver_fall_back_to_the_key_itself() {
    let mut r = Receiver::default();
    let hostile = [
        Chord { code: 30, modifiers: vec![29] }, // Ctrl is not a modifier typing may add
        Chord { code: 30, modifiers: vec![42, 54, 100, 42] }, // too many
        Chord { code: 0, modifiers: vec![] },    // not a key
        Chord { code: 768, modifiers: vec![] },  // past the key range
        Chord { code: u32::MAX, modifiers: vec![42] },
        Chord { code: 30, modifiers: vec![125] }, // the Super key
    ];
    for chord in hostile {
        assert_eq!(r.key(31, true, || chord.clone()), [(31, true)], "{chord:?}");
        assert_eq!(r.key(31, false, || unreachable!()), [(31, false)]);
    }
    // A chord that lists its own key as a modifier, or a modifier twice, is tidied.
    assert_eq!(r.key(42, true, || Chord { code: 42, modifiers: vec![42] }), [(42, true)]);
    assert_eq!(r.key(42, false, || unreachable!()), [(42, false)]);
    assert_eq!(
        r.key(30, true, || Chord { code: 30, modifiers: vec![100, 42, 42] }),
        [(42, true), (100, true), (30, true)]
    );
    assert_eq!(r.release_all(), [(30, false), (100, false), (42, false)]);
    // Sources outside the key range are ignored outright.
    for source in [0u32, 768, 100_000, u32::MAX] {
        assert!(r.key(source, true, || Chord { code: 30, modifiers: vec![] }).is_empty());
        assert!(r.key(source, false, || unreachable!()).is_empty());
    }
}

/// Everything the sender emits for a key, and the matching release, balances as well:
/// text keys press nothing until released, characters and shortcuts pair up, composition does not.
#[test]
fn the_sender_never_leaves_a_press_without_its_release() {
    let mut rng = Rng(0x5E4D);
    for _ in 0..800 {
        let mut sender = Sender::default();
        let mut sent_down: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..80 {
            let code = [30u32, 31, 32, 57, 28, 42, 100, 40, 2][rng.below(9)];
            let down = rng.below(5) != 0;
            let resolution = match rng.below(6) {
                0 => Resolution::Physical,
                1 => Resolution::Character(['a', 'ä', '界', '😀', '\u{7}'][rng.below(5)]),
                2 => Resolution::Text(["é", "e\u{301}", "'x", "", "\u{7}", "👍🏽"][rng.below(6)].to_owned()),
                3 => Resolution::Shortcut(['c', 'v', '\u{200d}'][rng.below(3)]),
                4 => Resolution::Pending,
                _ => Resolution::Character('x'),
            };
            for message in sender.key_events(code, down, || resolution.clone()) {
                match message {
                    Control::Key { code, down: true } | Control::CharacterKey { code, down: true, .. } => {
                        sent_down.insert(code);
                    }
                    Control::Key { code, down: false } | Control::CharacterKey { code, down: false, .. } => {
                        sent_down.remove(&code);
                    }
                    Control::Text { text } => assert!(valid_text(&text), "{text:?} is one typeable character"),
                    other => panic!("{other:?}"),
                }
            }
        }
        // The visit ends: everything sent as held is released.
        for message in sender.release_all() {
            match message {
                Control::Key { code, down: false } | Control::CharacterKey { code, down: false, .. } => {
                    sent_down.remove(&code);
                }
                other => panic!("{other:?} is not a release"),
            }
        }
        assert!(sent_down.is_empty(), "left pressed on the other computer: {sent_down:?}");
        assert!(sender.release_all().is_empty());
    }
}

#[test]
fn typed_text_is_rationed_per_second_and_only_valid_text_is_counted() {
    let t0 = Instant::now();
    let mut budget = TextBudget::new(t0);
    assert!(
        !budget.accept("", t0) && !budget.accept("ab", t0) && !budget.accept("\u{7}", t0),
        "invalid text is refused and costs nothing"
    );
    let accepted = (0..400).filter(|_| budget.accept("a", t0)).count();
    assert_eq!(accepted, 200, "two hundred characters in the first second");
    assert!(!budget.accept("a", t0 + Duration::from_millis(999)));
    assert!(budget.accept("a", t0 + Duration::from_secs(1)), "a new second, a new allowance");
    assert!(budget.accept("a", t0), "a clock that went back does not wedge it");
    for i in 0..198 {
        assert!(budget.accept("é", t0 + Duration::from_secs(1) + Duration::from_millis(i)), "{i}");
    }
    assert!(!budget.accept("é", t0 + Duration::from_millis(1500)));
}
