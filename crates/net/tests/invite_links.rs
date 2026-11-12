//! Invitation links come from outside (typed, pasted, scanned from a QR code): parsing must refuse
//! everything malformed without panicking, and whatever it accepts must be a sound invitation
//! that prints and parses back to itself.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use one_mk_net::Fingerprint;
use one_mk_net::invite::{Invitation, MAX_LIFETIME, MAX_LINK, unix_now};
use std::net::SocketAddr;
use std::time::Duration;

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

fn invitation(addrs: &[&str]) -> Invitation {
    Invitation::new(
        Fingerprint([0xAB; 32]),
        addrs.iter().map(|a| a.parse().unwrap()).collect(),
        24901,
        Duration::from_secs(600),
    )
    .unwrap()
}

fn link(addrs: &[&str]) -> String {
    invitation(addrs).link().to_string()
}

#[test]
fn a_link_round_trips_with_one_or_several_addresses_of_either_family() {
    for addrs in [
        vec!["192.168.1.20:24900"],
        vec!["192.168.1.20:24900", "10.0.0.7:24900", "172.16.5.5:24900"],
        vec!["[fd00::1234]:24900", "192.168.1.20:24900"],
        vec!["[2001:db8::1]:1"],
        vec!["192.168.1.20:65535"],
        vec!["10.0.0.1:24900"; 8],
    ] {
        let original = invitation(&addrs);
        let text = original.link().to_string();
        assert!(
            text.starts_with("onemk://pair?v=1&d=") && text.is_ascii() && text.len() <= MAX_LINK,
            "{} bytes",
            text.len()
        );
        let parsed = Invitation::parse(&text).unwrap_or_else(|e| panic!("{addrs:?}: {e:#}"));
        assert_eq!(parsed.fingerprint, original.fingerprint);
        assert_eq!(parsed.addrs, original.addrs);
        assert_eq!(
            (parsed.service_port, parsed.expires, parsed.mdns.as_str()),
            (original.service_port, original.expires, original.mdns.as_str())
        );
        assert_eq!(parsed.link().to_string(), text, "the secret and every field survive");
        assert!(!parsed.is_expired());
        assert!(parsed.expires > unix_now() && parsed.expires <= unix_now() + MAX_LIFETIME.as_secs());
    }
}

#[test]
fn invitations_that_cannot_work_are_refused_when_made() {
    let fp = Fingerprint([1; 32]);
    let new = |addrs: &[&str], port: u16, ttl: Duration| {
        Invitation::new(fp, addrs.iter().map(|a| a.parse::<SocketAddr>().unwrap()).collect(), port, ttl)
    };
    let ok = Duration::from_secs(60);
    assert!(new(&["192.168.1.2:24900"], 24900, ok).is_ok());
    assert!(new(&[], 24900, ok).is_err(), "no address");
    assert!(new(&["10.0.0.1:24900"; 9], 24900, ok).is_err(), "too many addresses");
    assert!(new(&["192.168.1.2:24900"], 0, ok).is_err(), "no service port");
    assert!(new(&["192.168.1.2:0"], 24900, ok).is_err(), "no port to dial");
    assert!(new(&["192.168.1.2:24900", "192.168.1.3:24901"], 24900, ok).is_err(), "addresses on different ports");
    for unusable in [
        "127.0.0.1:24900",
        "0.0.0.0:24900",
        "224.0.0.1:24900",
        "255.255.255.255:24900",
        "[::1]:24900",
        "[::]:24900",
        "[fe80::1]:24900",
        "[ff02::1]:24900",
        "[::ffff:127.0.0.1]:24900",
    ] {
        let err = new(&[unusable], 24900, ok).err().unwrap_or_else(|| panic!("{unusable} was accepted"));
        assert!(err.to_string().contains("invalid invitation address"), "{err}");
    }
    assert!(new(&["192.168.1.2:24900"], 24900, Duration::ZERO).is_err());
    assert!(new(&["192.168.1.2:24900"], 24900, Duration::from_millis(900)).is_err(), "under a second");
    assert!(new(&["192.168.1.2:24900"], 24900, MAX_LIFETIME).is_ok());
    assert!(new(&["192.168.1.2:24900"], 24900, MAX_LIFETIME + Duration::from_secs(1)).is_err());
}

#[test]
fn every_truncation_and_every_changed_character_is_refused_or_sound() {
    let good = link(&["192.168.1.20:24900", "[fd00::2]:24900"]);
    for cut in 0..good.len() {
        assert!(Invitation::parse(&good[..cut]).is_err(), "cut at {cut}");
    }
    let bytes = good.as_bytes();
    let mut rng = Rng(0x11A7);
    let (mut accepted, mut refused) = (0, 0);
    for index in 0..bytes.len() {
        for _ in 0..8 {
            let mut changed = bytes.to_vec();
            changed[index] = b"-_AZaz09=+/ %\n\0~"[rng.below(16)];
            let Ok(text) = String::from_utf8(changed) else { continue };
            match Invitation::parse(&text) {
                Err(_) => refused += 1,
                Ok(parsed) => {
                    accepted += 1;
                    // Whatever got through is a complete, usable invitation.
                    assert!((1..=8).contains(&parsed.addrs.len()) && parsed.service_port != 0, "{text}");
                    assert!(parsed.addrs.iter().all(|a| a.port() != 0 && one_mk_net::invite::usable_address(a.ip())));
                    assert_eq!(parsed.mdns, format!("one-mk-{}.local.", parsed.fingerprint.short()));
                    assert_eq!(
                        Invitation::parse(&parsed.link()).unwrap().link().to_string(),
                        parsed.link().to_string(),
                        "stable"
                    );
                }
            }
        }
    }
    assert!(
        refused > accepted * 3,
        "{refused} refused, {accepted} accepted (only changes to the secret, the expiry or an address digit are sound)"
    );
}

#[test]
fn hostile_links_of_every_shape_are_refused_without_panicking() {
    let good = link(&["192.168.1.20:24900"]);
    let payload = good.strip_prefix("onemk://pair?v=1&d=").unwrap();
    let raw = URL_SAFE_NO_PAD.decode(payload).unwrap();
    let wrap = |bytes: &[u8]| format!("onemk://pair?v=1&d={}", URL_SAFE_NO_PAD.encode(bytes));
    assert!(Invitation::parse(&wrap(&raw)).is_ok());
    // Trailing bytes, a wrong inner version, and counts that do not match.
    let mut longer = raw.clone();
    longer.push(0);
    assert!(Invitation::parse(&wrap(&longer)).is_err());
    let mut other_version = raw.clone();
    other_version[0] = 2;
    assert!(Invitation::parse(&wrap(&other_version)).is_err());
    let count_at = 1 + 32 + 8 + 2 + 2 + 23; // version, fingerprint, expiry, port, mdns length and the name
    for count in [0u8, 2, 9, 255] {
        let mut changed = raw.clone();
        changed[count_at] = count;
        assert!(Invitation::parse(&wrap(&changed)).is_err(), "count {count}");
    }
    // Wrong prefixes, versions, schemes.
    for text in [
        String::new(),
        "onemk://pair".into(),
        format!("onemk://pair?v=2&d={payload}"),
        format!("ONEMK://pair?v=1&d={payload}"),
        format!("http://pair?v=1&d={payload}"),
        format!(" onemk://pair?v=1&d={payload}"),
        format!("onemk://pair?v=1&d={payload}\n"),
        format!("onemk://pair?v=1&d={payload}&x=1"),
        format!("onemk://pair?v=1&d={payload}="),
        format!("onemk://pair?v=1&d={}é", &payload[..payload.len() - 1]),
        format!("onemk://pair?v=1&d={}", "A".repeat(MAX_LINK)),
        format!("onemk://pair?v=1&d={}", "Z".repeat(10)),
        "onemk://pair?v=1&d=".into(),
    ] {
        assert!(Invitation::parse(&text).is_err(), "{:?}", &text[..text.len().min(60)]);
    }
    // Random bytes, wrapped as a valid link, never panic.
    let mut rng = Rng(0xFA11);
    for _ in 0..20_000 {
        let len = rng.below(180);
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if rng.below(2) == 0 && !bytes.is_empty() {
            bytes[0] = 1;
        }
        let _ = Invitation::parse(&wrap(&bytes));
    }
    // A link at the size limit is refused for its size, one bigger as well.
    assert!(Invitation::parse(&format!("onemk://pair?v=1&d={}", "A".repeat(MAX_LINK + 1))).is_err());
    let _ = Fingerprint([0; 32]);
}
