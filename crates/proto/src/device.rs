//! Physical keyboards and mice: who they are (a fingerprint that survives
//! sleep, reconnecting and replugging) and which one an input event came
//! from (a [`DeviceRef`] into the backend's table of connected devices).
//! Pure data; the input backends fill it in, the tuning rules match it.
//! See "Per-device rules" in `docs/INPUT-TUNING.md`.

use std::fmt;
use std::str::FromStr;

/// The longest device name kept, in characters (longer ones are cut).
pub const MAX_NAME: usize = 128;
/// The longest serial number or address kept, in characters.
pub const MAX_SERIAL: usize = 64;

/// Which connected device an event came from: an index the backend hands
/// out, valid only together with its latest [`InputDevice`] table. Never
/// stored in a rule (it changes when a device reconnects).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceRef(pub u32);

/// How a device is connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bus {
    Usb,
    Bluetooth,
    /// Paired to a USB receiver (dongle) and shown by the OS as a device
    /// of its own.
    Receiver,
    /// Built in: a laptop's keyboard and touchpad (I2C, ACPI, PS/2).
    Internal,
    /// Made by software (X11's XTEST devices, remote desktop tools).
    Virtual,
    #[default]
    Unknown,
}

impl Bus {
    pub const ALL: [Bus; 6] = [Bus::Usb, Bus::Bluetooth, Bus::Receiver, Bus::Internal, Bus::Virtual, Bus::Unknown];

    pub fn as_str(self) -> &'static str {
        match self {
            Bus::Usb => "usb",
            Bus::Bluetooth => "bluetooth",
            Bus::Receiver => "receiver",
            Bus::Internal => "internal",
            Bus::Virtual => "virtual",
            Bus::Unknown => "unknown",
        }
    }

    /// In words, for people: "USB", "Bluetooth", "USB receiver"...
    pub fn words(self) -> &'static str {
        match self {
            Bus::Usb => "USB",
            Bus::Bluetooth => "Bluetooth",
            Bus::Receiver => "USB receiver",
            Bus::Internal => "built in",
            Bus::Virtual => "virtual (made by software)",
            Bus::Unknown => "connection unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Bus> {
        Bus::ALL.into_iter().find(|b| b.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// From a Linux input bus type (`/sys/.../id/bustype`, `BUS_*`).
    pub fn from_linux(bustype: u16) -> Bus {
        match bustype {
            0x03 => Bus::Usb,
            0x05 => Bus::Bluetooth,
            0x06 => Bus::Virtual,
            // I2C, PS/2 (i8042), host (ACPI), SPI, Intel ISHTP.
            0x18 | 0x11 | 0x19 | 0x1c | 0x1f => Bus::Internal,
            _ => Bus::Unknown,
        }
    }
}

/// Who a device is, as far as the OS tells: what a per-device rule names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub bus: Bus,
    pub vendor: u16,
    pub product: u16,
    /// A serial number, or a Bluetooth address, when the OS gives one.
    pub serial: Option<String>,
    pub name: String,
}

/// Removes what has no place in a name: control and invisible characters,
/// surrounding blanks; cuts it to `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    let kept: String = s
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'))
        .take(max)
        .collect();
    kept.trim().to_owned()
}

impl Fingerprint {
    /// A fingerprint from what an OS reported, cleaned (names cut to
    /// [`MAX_NAME`], serials to [`MAX_SERIAL`], an empty serial is none).
    pub fn new(bus: Bus, vendor: u16, product: u16, serial: Option<&str>, name: &str) -> Fingerprint {
        let serial = serial.map(|s| clean(s, MAX_SERIAL)).filter(|s| !s.is_empty());
        Fingerprint { bus, vendor, product, serial, name: clean(name, MAX_NAME) }
    }

    /// Whether this fingerprint, as written in a rule, names `device`: the
    /// same vendor:product and serial if the rule has a serial, else the
    /// same vendor:product and name (letter case ignored). The bus never
    /// counts: a keyboard keeps its rules over cable and receiver alike
    /// when it reports the same identity.
    pub fn matches(&self, device: &Fingerprint) -> bool {
        if (self.vendor, self.product) != (device.vendor, device.product) {
            return false;
        }
        match &self.serial {
            Some(s) => device.serial.as_deref() == Some(s.as_str()),
            None => self.name.to_lowercase() == device.name.to_lowercase(),
        }
    }

    /// Whether a rule could never tell `self` and `other` apart.
    pub fn indistinguishable(&self, other: &Fingerprint) -> bool {
        self.matches(other) && other.matches(self)
    }

    /// `046d:c52b`.
    pub fn ids(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor, self.product)
    }
}

fn escape(serial: &str) -> String {
    serial.replace('%', "%25").replace('/', "%2F")
}

fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let code = rest.get(i + 1..i + 3).ok_or("a `%` in the serial must be followed by 25 or 2F")?;
        match code.to_ascii_uppercase().as_str() {
            "25" => out.push('%'),
            "2F" => out.push('/'),
            _ => return Err(format!("`%{code}` in the serial: only %25 (%) and %2F (/) are written so")),
        }
        rest = &rest[i + 3..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `bus/vendor:product/serial/name`, the form rules are written in.
impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let serial = self.serial.as_deref().map(escape).unwrap_or_default();
        write!(f, "{}/{}/{serial}/{}", self.bus.as_str(), self.ids(), self.name)
    }
}

impl FromStr for Fingerprint {
    type Err = String;

    fn from_str(s: &str) -> Result<Fingerprint, String> {
        const FORM: &str = "write a device as bus/vendor:product/serial/name, as `one-mk input-devices` lists it";
        let s = s.trim();
        if s.chars().any(char::is_control) {
            return Err("a device can't contain control characters".into());
        }
        let mut parts = s.splitn(4, '/');
        let (Some(bus), Some(ids), Some(serial), Some(name)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(format!("{s:?}: {FORM}"));
        };
        let bus = Bus::parse(bus).ok_or_else(|| {
            let all: Vec<_> = Bus::ALL.iter().map(|b| b.as_str()).collect();
            format!("{bus:?} is not a connection: say {}", all.join(", "))
        })?;
        let hex = |h: &str| (h.len() == 4 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| u16::from_str_radix(h, 16).ok()).flatten();
        let (vendor, product) = match ids.split_once(':') {
            Some((v, p)) => (hex(v), hex(p)),
            None => (None, None),
        };
        let (Some(vendor), Some(product)) = (vendor, product) else {
            return Err(format!("{ids:?}: vendor and product are written as four hex digits each, like 046d:c52b"));
        };
        let serial = unescape(serial)?;
        if serial.chars().count() > MAX_SERIAL {
            return Err(format!("the serial is longer than {MAX_SERIAL} characters"));
        }
        let name = name.trim();
        if name.chars().count() > MAX_NAME {
            return Err(format!("the name is longer than {MAX_NAME} characters"));
        }
        let fp = Fingerprint::new(bus, vendor, product, Some(&serial), name);
        if fp.serial.is_none() && fp.name.is_empty() {
            return Err(format!("{s:?}: a device needs a serial or a name, or a rule would match every device {}", fp.ids()));
        }
        Ok(fp)
    }
}

/// What a device is good for (one device may be both: many mice have
/// keys, many keyboards a touchpad).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Kinds {
    pub keyboard: bool,
    pub pointer: bool,
}

impl Kinds {
    /// "keyboard", "mouse", "keyboard and mouse" or "input device".
    pub fn words(self) -> &'static str {
        match (self.keyboard, self.pointer) {
            (true, true) => "keyboard and mouse",
            (true, false) => "keyboard",
            (false, true) => "mouse",
            (false, false) => "input device",
        }
    }

    pub fn union(self, other: Kinds) -> Kinds {
        Kinds { keyboard: self.keyboard || other.keyboard, pointer: self.pointer || other.pointer }
    }
}

/// A connected keyboard or mouse, as the backend reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDevice {
    /// What its events carry.
    pub id: DeviceRef,
    pub fingerprint: Fingerprint,
    pub kinds: Kinds,
}

/// Linux: what `/sys/class/input/eventN/device` says about an input
/// device. `phys` tells a device paired to a receiver (`.../input2:1`: the
/// `:1` is its slot on the receiver) from the receiver's own interface.
pub fn linux_bus(bustype: u16, phys: &str) -> Bus {
    let bus = Bus::from_linux(bustype);
    let on_receiver = phys.rsplit_once(':').is_some_and(|(head, slot)| {
        head.rsplit('/').next().is_some_and(|last| last.starts_with("input")) && !slot.is_empty() && slot.chars().all(|c| c.is_ascii_digit())
    });
    if bus == Bus::Usb && on_receiver { Bus::Receiver } else { bus }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remap::test_support::Rng;

    fn fp(s: &str) -> Fingerprint {
        s.parse().unwrap()
    }

    #[test]
    fn fingerprints_print_and_parse() {
        let a = Fingerprint::new(Bus::Bluetooth, 0x04e8, 0x7021, Some("f4:73:35:aa:bb:cc"), "Travel Keyboard");
        assert_eq!(a.to_string(), "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard");
        assert_eq!(fp(&a.to_string()), a);
        let b = Fingerprint::new(Bus::Usb, 0x046d, 0xc52b, None, "Logitech USB Receiver / Mouse");
        assert_eq!(b.to_string(), "usb/046d:c52b//Logitech USB Receiver / Mouse");
        assert_eq!(fp(&b.to_string()), b, "a / in the name is kept");
        let c = Fingerprint::new(Bus::Usb, 1, 2, Some("a/b%c"), "x");
        assert_eq!(c.to_string(), "usb/0001:0002/a%2Fb%25c/x");
        assert_eq!(fp(&c.to_string()), c);
        assert_eq!(fp(" USB/046D:C52B//Name ").ids(), "046d:c52b", "case and blanks don't matter");
    }

    #[test]
    fn bad_fingerprints_say_what_is_wrong() {
        for (bad, says) in [
            ("", "bus/vendor:product"),
            ("usb/046d:c52b", "bus/vendor:product"),
            ("floppy/046d:c52b//x", "not a connection"),
            ("usb/46d:c52b//x", "four hex digits"),
            ("usb/046d-c52b//x", "four hex digits"),
            ("usb/+46d:c52b//x", "four hex digits"),
            ("usb/046d:c52b/%zz/x", "%zz"),
            ("usb/046d:c52b/%2/x", "followed by"),
            ("usb/046d:c52b//", "needs a serial or a name"),
            ("usb/046d:c52b//a\nb", "control"),
        ] {
            let e = bad.parse::<Fingerprint>().unwrap_err();
            assert!(e.contains(says), "{bad:?}: {e}");
        }
        assert!(format!("usb/0001:0002//{}", "n".repeat(MAX_NAME + 1)).parse::<Fingerprint>().is_err());
        assert!(format!("usb/0001:0002/{}/n", "s".repeat(MAX_SERIAL + 1)).parse::<Fingerprint>().is_err());
    }

    #[test]
    fn matching_uses_the_serial_when_the_rule_has_one() {
        let rule = fp("bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard");
        let renamed = fp("usb/04e8:7021/f4:73:35:aa:bb:cc/My keyboard");
        assert!(rule.matches(&renamed), "the serial counts, not the name or bus");
        let twin = fp("bluetooth/04e8:7021/00:00:00:00:00:01/Travel Keyboard");
        assert!(!rule.matches(&twin), "another device of the same model");
        let other_vendor = fp("bluetooth/1234:7021/f4:73:35:aa:bb:cc/Travel Keyboard");
        assert!(!rule.matches(&other_vendor));
        assert!(!rule.matches(&fp("bluetooth/04e8:7021//Travel Keyboard")), "a device without its serial");
    }

    #[test]
    fn matching_without_a_serial_uses_the_model_and_name() {
        let rule = fp("usb/046d:c52b//Logitech USB Receiver");
        assert!(rule.matches(&fp("usb/046d:c52b//logitech usb receiver")));
        assert!(rule.matches(&fp("receiver/046d:c52b/123/Logitech USB Receiver")), "any serial, any bus");
        assert!(!rule.matches(&fp("usb/046d:c52b//Logitech USB Receiver Mouse")));
        assert!(!rule.matches(&fp("usb/046d:c52c//Logitech USB Receiver")));
        let twin = fp("usb/046d:c52b//Logitech USB Receiver");
        assert!(rule.indistinguishable(&twin), "two identical models without a serial");
        assert!(!rule.indistinguishable(&fp("usb/046d:c52b/1/Logitech USB Receiver")));
    }

    #[test]
    fn names_are_cleaned() {
        let f = Fingerprint::new(Bus::Usb, 1, 2, Some("  "), " Key\u{7}board\u{202e} ");
        assert_eq!((f.name.as_str(), f.serial), ("Keyboard", None));
        let long = Fingerprint::new(Bus::Usb, 1, 2, Some(&"s".repeat(500)), &"é".repeat(500));
        assert_eq!((long.name.chars().count(), long.serial.unwrap().chars().count()), (MAX_NAME, MAX_SERIAL));
    }

    #[test]
    fn linux_buses() {
        assert_eq!(linux_bus(0x03, "usb-0000:00:14.0-2/input0"), Bus::Usb);
        assert_eq!(linux_bus(0x03, "usb-0000:00:14.0-2/input2:1"), Bus::Receiver);
        assert_eq!(linux_bus(0x05, "f4:73:35:aa:bb:cc"), Bus::Bluetooth);
        assert_eq!(linux_bus(0x18, "i2c-ELAN0001:00"), Bus::Internal);
        assert_eq!(linux_bus(0x11, "isa0060/serio0/input0"), Bus::Internal);
        assert_eq!(linux_bus(0x06, ""), Bus::Virtual);
        assert_eq!(linux_bus(0x99, ""), Bus::Unknown);
    }

    /// Hostile text never panics, and whatever parses prints back to
    /// something that parses to the same fingerprint.
    #[test]
    fn random_text_never_panics_and_round_trips() {
        let alphabet: Vec<char> = "usbluetoothreceiver/:%2F25abcdef0123456789 \u{7}é漢\u{202e}-+.".chars().collect();
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let mut parsed = 0;
        for _ in 0..200_000 {
            let mut s: String = match rng.below(3) {
                0 => String::new(),
                1 => "usb/046d:c52b/".into(),
                _ => "bluetooth/".into(),
            };
            for _ in 0..rng.below(40) {
                s.push(alphabet[rng.below(alphabet.len())]);
            }
            if let Ok(f) = s.parse::<Fingerprint>() {
                parsed += 1;
                assert_eq!(f.to_string().parse::<Fingerprint>().as_ref(), Ok(&f), "{s:?}");
                assert!(f.matches(&f));
            }
        }
        assert!(parsed > 1000, "the generator should hit valid fingerprints too ({parsed})");
    }
}
