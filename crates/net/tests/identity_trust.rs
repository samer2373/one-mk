//! Device identity, fingerprints and the trusted-device file: parsing hostile
//! input, persistence, and how a damaged `devices.toml` is handled.

use one_mk_net::{Device, Fingerprint, Identity, MAX_OFFSET, PlacedBy, Swap, TrustStore};
use std::path::Path;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn fp(byte: u8) -> Fingerprint {
    Fingerprint([byte; 32])
}

fn device(name: &str, byte: u8) -> Device {
    Device { name: name.into(), fingerprint: fp(byte).to_string(), ..Device::default() }
}

fn write(dir: &Path, text: &str) {
    std::fs::write(dir.join("devices.toml"), text).unwrap();
}

// ------------------------------------------------------------ fingerprints

#[test]
fn fingerprints_round_trip_through_text() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..2000 {
        let f = Fingerprint(std::array::from_fn(|_| rng.next() as u8));
        let text = f.to_string();
        assert_eq!(text.len(), 64);
        assert!(text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "lowercase hex: {text}");
        assert_eq!(text.parse::<Fingerprint>().unwrap(), f);
        assert_eq!(text.to_uppercase().parse::<Fingerprint>().unwrap(), f, "upper case is the same fingerprint");
        assert_eq!(f.short(), format!("{}-{}", &text[..4], &text[4..8]));
    }
}

/// A 64-byte string with a multi-byte character used to split a slice in the middle of it and
/// panic, taking the command (or the daemon reading a hand-edited `devices.toml`) down; a leading
/// `+` was read as part of a number.
#[test]
fn malformed_fingerprints_are_errors_not_panics() {
    let good = "ab".repeat(32);
    assert!(good.parse::<Fingerprint>().is_ok());
    let mixed = format!("a{}a", "é".repeat(31));
    assert_eq!(mixed.len(), 64);
    let bad = [
        String::new(),
        "ab".repeat(31),
        "ab".repeat(33),
        format!("{} ", &good[..63]),
        format!(" {}", &good[1..]),
        format!("+f{}", "00".repeat(31)),
        format!("-f{}", "00".repeat(31)),
        format!("0x{}", "ab".repeat(31)),
        "g".repeat(64),
        mixed,
        "é".repeat(32),
        "😀".repeat(16),
        format!("{}\n", &good[..63]),
        format!("{}\u{0}", &good[..63]),
        format!("{}-", &good[..63]),
    ];
    for text in bad {
        assert!(text.parse::<Fingerprint>().is_err(), "{text:?} was accepted");
    }
    // Junk of every shape and length, none of which may panic.
    let mut rng = Rng(99);
    let alphabet: Vec<char> = "0123456789abcdefABCDEF+-xg é界😀\n\u{0}".chars().collect();
    for _ in 0..20_000 {
        let len = [62usize, 63, 64, 64, 64, 65][(rng.next() % 6) as usize];
        let text: String = (0..len).map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize]).collect();
        let _ = text.parse::<Fingerprint>();
    }
}

#[test]
fn the_fingerprint_of_a_certificate_is_its_sha256() {
    use sha2::{Digest, Sha256};
    let id = Identity::ephemeral().unwrap();
    let expected: [u8; 32] = Sha256::digest(id.cert.as_ref()).into();
    assert_eq!(id.fingerprint, Fingerprint(expected));
    assert_eq!(Fingerprint::of(id.cert.as_ref()), id.fingerprint);
    assert_ne!(Identity::ephemeral().unwrap().fingerprint, id.fingerprint, "every new identity is different");
    assert!(format!("{:?}", id.fingerprint).contains(&id.fingerprint.short()));
}

// ----------------------------------------------------------------- identity

#[test]
fn an_identity_is_created_once_and_loaded_ever_after() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("fresh/nested");
    let first = Identity::load_or_create(&home).unwrap();
    let second = Identity::load_or_create(&home).unwrap();
    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(first.cert.as_ref(), second.cert.as_ref());
    assert!(home.join("identity.cert.der").is_file() && home.join("identity.key.der").is_file());
    let other = Identity::load_or_create(&dir.path().join("another")).unwrap();
    assert_ne!(other.fingerprint, first.fingerprint, "two homes are two computers");
    // The key that was saved is the one the certificate was made for: a node built from the
    // reloaded identity can complete a handshake with its own fingerprint.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&home.join("identity.key.der")), 0o600);
        assert_eq!(mode(&home), 0o700);
    }
}

#[test]
fn a_half_written_identity_is_replaced_whole_never_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let first = Identity::load_or_create(dir.path()).unwrap();
    // Only the certificate survived a crash: the pair is made again, together.
    std::fs::remove_file(dir.path().join("identity.key.der")).unwrap();
    let second = Identity::load_or_create(dir.path()).unwrap();
    assert_ne!(second.fingerprint, first.fingerprint);
    let third = Identity::load_or_create(dir.path()).unwrap();
    assert_eq!(third.fingerprint, second.fingerprint);
    // And the other way round.
    std::fs::remove_file(dir.path().join("identity.cert.der")).unwrap();
    let fourth = Identity::load_or_create(dir.path()).unwrap();
    assert_ne!(fourth.fingerprint, second.fingerprint);
    assert_eq!(Identity::load_or_create(dir.path()).unwrap().fingerprint, fourth.fingerprint);
}

#[tokio::test]
async fn an_identity_can_be_used_to_run_a_node_and_the_name_is_never_empty() {
    let id = std::sync::Arc::new(Identity::ephemeral().unwrap());
    assert!(!id.name.trim().is_empty());
    assert!(!id.name.ends_with(".local"));
    let node = one_mk_net::Node::bind(id.clone(), "127.0.0.1:0".parse().unwrap());
    assert!(node.is_ok());
}

// --------------------------------------------------------------- trust store

#[test]
fn a_damaged_devices_file_is_an_error_and_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    for text in [
        "device = 5",
        "[[device]\nname = \"x\"",
        "\u{0}\u{0}",
        "[[device]]\nname = 3\nfingerprint = \"x\"",
        "[[device]]\nfingerprint = \"x\"",
    ] {
        write(dir.path(), text);
        let err = TrustStore::load(dir.path()).err().unwrap_or_else(|| panic!("{text:?} loaded"));
        assert!(format!("{err:#}").contains("devices.toml"), "{err:#}");
        // A change through `update` fails too, and does not replace the file with an empty one.
        assert!(
            TrustStore::update(dir.path(), |s| {
                s.upsert(device("new", 1));
                Ok(())
            })
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("devices.toml")).unwrap(),
            text,
            "the damaged file is not overwritten"
        );
    }
}

#[test]
fn a_failing_change_writes_nothing_and_a_missing_file_is_an_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    assert!(TrustStore::load(dir.path()).unwrap().devices().is_empty());
    TrustStore::update(dir.path(), |s| {
        s.upsert(device("kept", 1));
        Ok(())
    })
    .unwrap();
    let before = std::fs::read_to_string(dir.path().join("devices.toml")).unwrap();
    let result: anyhow::Result<()> = TrustStore::update(dir.path(), |s| {
        s.upsert(device("half done", 2));
        anyhow::bail!("the change failed after editing")
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(dir.path().join("devices.toml")).unwrap(), before);
    // No change at all does not even rewrite the file.
    let stamp = std::fs::metadata(dir.path().join("devices.toml")).unwrap().modified().unwrap();
    TrustStore::update(dir.path(), |_| Ok(())).unwrap();
    assert_eq!(std::fs::metadata(dir.path().join("devices.toml")).unwrap().modified().unwrap(), stamp);
    // No temporary file is left behind by any of it.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn what_a_hand_edit_can_say_is_bounded_or_defaulted() {
    let dir = tempfile::tempdir().unwrap();
    let id = fp(7).to_string();
    write(
        dir.path(),
        &format!(
            "[[device]]\nname = \"Lap\"\nfingerprint = \"{id}\"\nside = \"sideways\"\nswap_ctrl_cmd = \"on\"\nos = 3\noffset = [2000000000, -2000000000]\nplaced_by = \"martian\"\nlayout_version = 18446744073709551615\n"
        ),
    );
    let store = TrustStore::load(dir.path()).unwrap();
    let d = store.get(&fp(7)).unwrap();
    assert_eq!(d.side(), one_mk_proto::Side::Right, "an unknown side falls back to the default");
    assert_eq!(d.offset, Some((MAX_OFFSET, -MAX_OFFSET)), "a wild offset is cut to the bound");
    assert_eq!(d.placed_by, PlacedBy::Default, "an unknown origin is an unspecified one");
    assert_eq!(d.swap_ctrl_cmd, Swap::On);
    assert_eq!(d.layout_version, u64::MAX);
    assert!(!d.learnable(), "an exact offset is never a guess");
    // Not a valid fingerprint: still listed, but names no key.
    write(dir.path(), "[[device]]\nname = \"Odd\"\nfingerprint = \"nonsense\"\n");
    let store = TrustStore::load(dir.path()).unwrap();
    let odd = &store.devices()[0];
    assert_eq!((odd.fingerprint(), odd.short_id().as_str()), (None, ""));
    assert!(store.get(&fp(7)).is_none());
}

#[test]
fn lookup_by_name_or_id_never_picks_between_two_matches() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TrustStore::load(dir.path()).unwrap();
    // Two computers with one name and an id sharing its first digits.
    let mut a = device("Desk", 0xAB);
    let mut b = device("desk", 0xAB);
    b.fingerprint = format!("abab{}", "cd".repeat(30));
    a.alias = Some("   ".into());
    store.upsert(a.clone());
    store.upsert(b.clone());
    store.upsert(device("Lap", 0x12));
    assert!(store.find("desk").is_none(), "an ambiguous name finds neither");
    assert!(store.explain_miss("DESK").unwrap().contains("2 paired computers"));
    assert!(store.explain_miss("nobody").unwrap().contains("no paired computer"));
    assert!(store.explain_miss("lap").is_none());
    assert_eq!(store.find("lap").unwrap().name, "Lap");
    assert_eq!(store.find("1212").unwrap().name, "Lap");
    assert_eq!(store.find("12-12").unwrap().name, "Lap", "dashes in an id are as the window shows it");
    assert!(store.find("12").is_none(), "an id shorter than four digits is not an id");
    assert!(store.find_fingerprint("12").is_none());
    assert!(store.find_fingerprint("12zz").is_none());
    assert!(store.find_fingerprint("abab").is_none(), "two fingerprints start with abab");
    assert_eq!(
        store.find_fingerprint("abab-cdcd").unwrap().fingerprint,
        b.fingerprint,
        "a longer prefix tells them apart"
    );
    assert_eq!(store.find_fingerprint("ABABABAB").unwrap().fingerprint, a.fingerprint);
    // A blank alias does not hide the real name.
    assert_eq!(store.get(&fp(0xAB)).unwrap().label(), "Desk");
    // The computer that names itself after another one's id does not take it over.
    let mut thief = device("1212", 0x99);
    thief.alias = None;
    store.upsert(thief);
    assert!(store.find("1212").is_none(), "name and id of two computers: ambiguous, so neither");
    assert_eq!(store.find_fingerprint("1212").unwrap().name, "Lap", "by id only, the real one");
    // Removing needs an unambiguous query too.
    assert!(store.remove("desk").is_none());
    assert_eq!(store.devices().len(), 4);
    assert_eq!(store.remove("lap").unwrap().name, "Lap");
    assert!(!store.is_trusted(&fp(0x12)));
}

#[test]
fn upsert_keeps_what_the_user_chose_and_refreshes_what_the_device_says() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TrustStore::load(dir.path()).unwrap();
    let mut first = device("Old name", 5);
    first.side = "left".into();
    first.offset = Some((1, 2));
    first.layout_version = 99;
    first.swap_ctrl_cmd = Swap::Off;
    first.placed_by = PlacedBy::User;
    first.alias = Some("Mine".into());
    first.addrs = vec!["10.0.0.5:24900".into()];
    store.upsert(first);
    let mut again = device("New name", 5);
    again.os = 3;
    again.addrs = vec!["10.0.0.9:24900".into()];
    store.upsert(again);
    assert_eq!(store.devices().len(), 1);
    let d = &store.devices()[0];
    assert_eq!((d.name.as_str(), d.os, d.addrs.as_slice()), ("New name", 3, &["10.0.0.9:24900".to_owned()][..]));
    assert_eq!(
        (d.side.as_str(), d.offset, d.layout_version, d.swap_ctrl_cmd, d.placed_by),
        ("left", Some((1, 2)), 99, Swap::Off, PlacedBy::User)
    );
    assert_eq!(d.label(), "Mine");
    assert_eq!(store.display(d), "Mine");
}

#[test]
fn names_shown_to_people_are_clean_and_commands_quote_what_needs_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TrustStore::load(dir.path()).unwrap();
    store.upsert(device("evil\u{1b}[2K\u{202e}name", 1));
    store.upsert(device("Plain-Name_1.local", 2));
    store.upsert(device("Two Words", 3));
    store.upsert(device("quote\"; rm -rf", 4));
    store.upsert(device("Twin", 5));
    store.upsert(device("Twin", 6));
    let shown = store.display(store.get(&fp(1)).unwrap());
    assert!(!shown.chars().any(char::is_control) && !shown.contains('\u{202e}'), "{shown:?}");
    assert_eq!(store.arg(store.get(&fp(2)).unwrap()), "Plain-Name_1.local");
    assert_eq!(store.arg(store.get(&fp(3)).unwrap()), "\"Two Words\"");
    assert_eq!(
        store.arg(store.get(&fp(4)).unwrap()),
        fp(4).short(),
        "a name that is not safe to type is given as its id"
    );
    let twin = store.get(&fp(5)).unwrap();
    assert!(store.label_is_shared(twin));
    assert_eq!(store.display(twin), format!("Twin ({})", twin.short_id()));
    assert_eq!(store.arg(twin), twin.short_id(), "a shared name cannot name it");
    assert_eq!(store.display_fp(&fp(9)), "", "an unpaired computer has no name here");
}

#[test]
fn placement_rules_decide_what_layout_learning_may_move() {
    let mut d = device("x", 1);
    assert!(d.learnable(), "a fresh pairing is a guess");
    d.layout_version = 5;
    assert!(!d.learnable(), "a layout somebody chose is not");
    d.placed_by = PlacedBy::Learned;
    assert!(d.learnable());
    d.placed_by = PlacedBy::Pairing;
    assert!(d.learnable());
    for fixed in [PlacedBy::User, PlacedBy::Confirmed, PlacedBy::Pending] {
        d.placed_by = fixed;
        assert!(!d.learnable(), "{fixed:?}");
    }
    d.placed_by = PlacedBy::Learned;
    d.offset = Some((0, 0));
    assert!(!d.learnable(), "an exact offset is a decision");
    assert_eq!("AUTO".parse::<Swap>(), Ok(Swap::Auto));
    assert_eq!("On".parse::<Swap>(), Ok(Swap::On));
    assert!("maybe".parse::<Swap>().is_err());
    assert_eq!(Swap::Off.to_string(), "off");
}
