//! Who a Linux input device is, for per-device rules: what the X server
//! says (the XInput device name, the `Device Node` and `Device Product ID`
//! properties) completed by `/sys/class/input/eventN/device`, which any
//! user may read: the bus, vendor, product, name and `uniq` (a serial
//! number, or a Bluetooth device's address). Nothing here needs root, and
//! nothing reads `/dev/input`.

use one_mk_proto::device::{Bus, Fingerprint, linux_bus};
use std::path::Path;

/// What the kernel says about one input device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SysInfo {
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub name: String,
    pub uniq: String,
    pub phys: String,
}

/// `eventN` from a `Device Node` such as `/dev/input/event5`; anything
/// else (a path elsewhere, `..`) is refused.
pub(crate) fn event_node(device_node: &str) -> Option<&str> {
    let n = device_node.strip_prefix("/dev/input/")?;
    let digits = n.strip_prefix("event")?;
    (!digits.is_empty() && digits.len() <= 6 && digits.bytes().all(|b| b.is_ascii_digit())).then_some(n)
}

/// Reads `<sys>/class/input/<event>/device/...` (`sys` is `/sys`, or a
/// folder made by a test). Small files only: each read is capped.
pub(crate) fn read_sysfs(sys: &Path, event: &str) -> Option<SysInfo> {
    let dir = sys.join("class/input").join(event).join("device");
    let read = |name: &str| -> Option<String> {
        use std::io::Read;
        let mut s = String::new();
        std::fs::File::open(dir.join(name)).ok()?.take(4096).read_to_string(&mut s).ok()?;
        Some(s.trim().to_owned())
    };
    let hex = |name: &str| read(name).and_then(|s| u16::from_str_radix(&s, 16).ok());
    Some(SysInfo {
        bustype: hex("id/bustype")?,
        vendor: hex("id/vendor").unwrap_or(0),
        product: hex("id/product").unwrap_or(0),
        name: read("name").unwrap_or_default(),
        uniq: read("uniq").unwrap_or_default(),
        phys: read("phys").unwrap_or_default(),
    })
}

/// A device's fingerprint from the XInput name, what the kernel says (if
/// it could be read) and the `Device Product ID` property (if any).
pub(crate) fn fingerprint(xi_name: &str, sys: Option<&SysInfo>, product_id: Option<(u32, u32)>) -> Fingerprint {
    if is_xtest(xi_name) {
        return Fingerprint::new(Bus::Virtual, 0, 0, None, xi_name);
    }
    match sys {
        Some(s) => {
            let name = if s.name.is_empty() { xi_name } else { s.name.as_str() };
            Fingerprint::new(linux_bus(s.bustype, &s.phys), s.vendor, s.product, Some(&s.uniq), name)
        }
        None => {
            let (v, p) = product_id.map_or((0, 0), |(v, p)| (v as u16, p as u16));
            Fingerprint::new(Bus::Unknown, v, p, None, xi_name)
        }
    }
}

/// X11's own virtual devices, through which XTest input arrives.
pub(crate) fn is_xtest(xi_name: &str) -> bool {
    xi_name.contains("XTEST")
}

/// XWayland's devices: one merged keyboard and pointer for all real ones.
pub(crate) fn is_xwayland(xi_name: &str) -> bool {
    xi_name.starts_with("xwayland-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_nodes() {
        assert_eq!(event_node("/dev/input/event5"), Some("event5"));
        assert_eq!(event_node("/dev/input/event"), None);
        assert_eq!(event_node("/dev/input/event5/../../x"), None);
        assert_eq!(event_node("/dev/input/mouse0"), None);
        assert_eq!(event_node("/etc/passwd"), None);
        assert_eq!(event_node("/dev/input/event12345678"), None);
    }

    #[test]
    fn reads_what_the_kernel_says() {
        let dir = std::env::temp_dir().join(format!("one-mk-sysfs-{}", std::process::id()));
        let dev = dir.join("class/input/event7/device");
        std::fs::create_dir_all(dev.join("id")).unwrap();
        for (f, v) in [
            ("id/bustype", "0005\n"),
            ("id/vendor", "04e8\n"),
            ("id/product", "7021\n"),
            ("name", "Travel Keyboard\n"),
            ("uniq", "f4:73:35:aa:bb:cc\n"),
            ("phys", "00:1a:7d:da:71:13\n"),
        ] {
            std::fs::write(dev.join(f), v).unwrap();
        }
        let s = read_sysfs(&dir, "event7").unwrap();
        let fp = fingerprint("Travel Keyboard", Some(&s), Some((1, 2)));
        assert_eq!(fp.to_string(), "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard");
        assert_eq!(read_sysfs(&dir, "event8"), None, "a device that went away meanwhile");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_the_kernel_the_x_server_says_enough() {
        let fp = fingerprint("Logitech USB Receiver", None, Some((0x046d, 0xc52b)));
        assert_eq!(fp.to_string(), "unknown/046d:c52b//Logitech USB Receiver");
        let fp = fingerprint("Virtual core XTEST keyboard", None, None);
        assert_eq!(fp.to_string(), "virtual/0000:0000//Virtual core XTEST keyboard");
        let receiver = SysInfo { bustype: 3, vendor: 0x046d, product: 0x4082, name: "Logitech MX Master 3".into(), uniq: String::new(), phys: "usb-0000:00:14.0-1/input2:1".into() };
        assert_eq!(fingerprint("Logitech MX Master 3", Some(&receiver), None).to_string(), "receiver/046d:4082//Logitech MX Master 3");
        assert!(is_xwayland("xwayland-pointer:16") && !is_xwayland("Logitech"));
    }
}
