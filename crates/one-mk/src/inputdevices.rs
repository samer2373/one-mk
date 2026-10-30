//! The keyboards and mice this computer has seen (`input-devices.toml` in
//! the config folder), for per-device rules: written by the daemon when a
//! device is plugged in, unplugged, falls asleep or wakes, read by `one-mk
//! input-devices`, by `--device` and by the window, so a rule can be made
//! for a keyboard that is asleep right now. See "Per-device rules" in
//! `docs/INPUT-TUNING.md`.

use anyhow::{Context, Result, bail};
use one_mk_proto::device::{Bus, Fingerprint, InputDevice};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE: &str = "input-devices.toml";
/// Devices remembered at most; the ones seen longest ago go first.
pub const MAX: usize = 64;
/// A bigger file isn't ours (or is broken): it is not read.
const MAX_BYTES: u64 = 256 * 1024;

/// What `input-devices.toml` holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Remembered {
    /// What the daemon that wrote it could tell.
    #[serde(default)]
    pub daemon: DaemonNote,
    #[serde(default, rename = "device", skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Seen>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonNote {
    /// Its input backend (`x11`, `portal`, ...).
    #[serde(default)]
    pub backend: String,
    /// Why it can't tell devices apart, if it can't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

/// One device, by fingerprint.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub fingerprint: String,
    #[serde(default)]
    pub keyboard: bool,
    #[serde(default)]
    pub pointer: bool,
    /// Seconds since 1970.
    #[serde(default)]
    pub first_seen: u64,
    /// When it last came or went (seconds since 1970).
    #[serde(default)]
    pub last_seen: u64,
    #[serde(default)]
    pub connected: bool,
    /// More than one connected device has this fingerprint: two of the
    /// same model without a serial, or one device the OS shows twice.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub twins: bool,
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Seen {
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint.parse().ok()
    }

    /// Its name, or what it is when it has none.
    pub fn name(&self) -> String {
        crate::tuning::device_name(&self.fingerprint)
    }

    pub fn kinds(&self) -> &'static str {
        one_mk_proto::device::Kinds { keyboard: self.keyboard, pointer: self.pointer }.words()
    }
}

impl Remembered {
    /// The file, or nothing if there is none yet. One that can't be read
    /// is an error saying so (the daemon then starts a new one).
    pub fn load(dir: &Path) -> Result<Remembered> {
        let p = path(dir);
        let meta = match std::fs::metadata(&p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Remembered::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", p.display())),
        };
        if meta.len() > MAX_BYTES {
            bail!("{} is larger than {} KB, so it isn't read", p.display(), MAX_BYTES / 1024);
        }
        let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let mut r: Remembered = toml::from_str(&text).map_err(|e| anyhow::anyhow!("{} can't be read: {}", p.display(), e.message()))?;
        // Entries that don't say a usable device are dropped, and the list
        // is cut to size, whatever the file says.
        r.devices.retain(|d| d.fingerprint().is_some());
        r.bound();
        Ok(r)
    }

    /// Writes it through a file of its own renamed over the old one, so a
    /// reader never sees half of it.
    pub fn save(&self, dir: &Path) -> Result<()> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let write = || -> Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            let head = "# Keyboards and mice one-mk has seen on this computer, for per-device rules.\n\
                        # Written by the one-mk daemon; `one-mk input-devices` shows them.\n\n";
            file.write_all(head.as_bytes())?;
            file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path(dir))?;
            Ok(())
        };
        write().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// The devices connected now are `now` (at `at`): new ones are added,
    /// the others marked connected or not, and when that changed, when.
    /// Returns whether anything changed.
    pub fn update(&mut self, now: &[InputDevice], at: u64) -> bool {
        let before = self.clone();
        // Per fingerprint: keyboard, pointer, and whether two connected
        // devices of the same kind have it.
        let mut connected: Vec<(String, bool, bool, bool)> = Vec::new();
        for d in now {
            let fp = d.fingerprint.to_string();
            let twin = now.iter().filter(|x| x.fingerprint == d.fingerprint && x.kinds == d.kinds).count() > 1;
            match connected.iter_mut().find(|c| c.0 == fp) {
                Some(c) => {
                    c.1 |= d.kinds.keyboard;
                    c.2 |= d.kinds.pointer;
                    c.3 |= twin;
                }
                None => connected.push((fp, d.kinds.keyboard, d.kinds.pointer, twin)),
            }
        }
        for seen in &mut self.devices {
            let here = connected.iter().find(|c| c.0 == seen.fingerprint);
            if here.is_some() != seen.connected {
                seen.connected = here.is_some();
                seen.last_seen = at;
            }
            if let Some(&(_, k, p, twins)) = here {
                seen.keyboard |= k;
                seen.pointer |= p;
                seen.twins = twins;
            } else {
                seen.twins = false;
            }
        }
        for (fp, keyboard, pointer, twins) in connected {
            if !self.devices.iter().any(|s| s.fingerprint == fp) {
                self.devices.push(Seen { fingerprint: fp, keyboard, pointer, first_seen: at, last_seen: at, connected: true, twins });
            }
        }
        self.bound();
        *self != before
    }

    /// The daemon stops (or can't see devices): none is known connected.
    pub fn disconnect_all(&mut self, at: u64) -> bool {
        self.update(&[], at)
    }

    /// At most [`MAX`]: the devices seen longest ago go first, connected
    /// ones last.
    fn bound(&mut self) {
        while self.devices.len() > MAX {
            let Some(oldest) = self.devices.iter().enumerate().min_by_key(|(_, d)| (d.connected, d.last_seen)).map(|(i, _)| i) else {
                break;
            };
            self.devices.remove(oldest);
        }
    }

    /// The device `query` names: a fingerprint (as `input-devices` lists
    /// it), or a name or part of one (letter case ignored) that picks
    /// exactly one device seen. The error says what to do.
    pub fn find(&self, query: &str) -> Result<String> {
        let q = query.trim();
        if q.is_empty() {
            bail!("--device needs a keyboard's or mouse's name, as `one-mk input-devices` lists it");
        }
        if q.contains('/')
            && let Ok(fp) = q.parse::<Fingerprint>()
        {
            return Ok(fp.to_string());
        }
        let lower = q.to_lowercase();
        let exact: Vec<&Seen> = self.devices.iter().filter(|d| d.name().to_lowercase() == lower).collect();
        let found = if exact.is_empty() { self.devices.iter().filter(|d| d.name().to_lowercase().contains(&lower)).collect() } else { exact };
        match found.as_slice() {
            [one] => Ok(one.fingerprint.clone()),
            [] if q.contains('/') => {
                let e = q.parse::<Fingerprint>().err().unwrap_or_default();
                bail!("--device {q:?}: {e}")
            }
            [] if self.devices.is_empty() => bail!(
                "no keyboard or mouse has been seen on this computer yet. A device is listed (`one-mk input-devices`) once it \
                 has been connected while one-mk runs; then use its name or fingerprint with --device"
            ),
            [] => bail!("no keyboard or mouse called {q:?} has been seen on this computer: see `one-mk input-devices` for their names"),
            many => {
                let list: Vec<String> = many.iter().map(|d| format!("  {}  ({})", d.fingerprint, d.kinds())).collect();
                bail!("{q:?} could be any of these devices; say which with its fingerprint (--device \"...\"):\n{}", list.join("\n"))
            }
        }
    }
}

// ------------------------------------------------------------- the command

/// Whether this computer can tell devices apart, when no daemon says.
fn guess_support() -> Result<Option<&'static str>, &'static str> {
    if cfg!(target_os = "linux") {
        let set = |v: &str| std::env::var_os(v).is_some_and(|x| !x.is_empty());
        if set("WAYLAND_DISPLAY") {
            return Err(one_mk_input::MERGED_DEVICES);
        }
        if set("DISPLAY") {
            return one_mk_input::device_support("x11");
        }
        return Err("no desktop session was found (neither Wayland nor X11)");
    }
    one_mk_input::device_support("native")
}

/// Whether per-device rules work here: `Ok(untested note)` or `Err(why
/// not)`, from the running daemon's note or, without one, a guess.
pub fn support(r: &Remembered, running: bool) -> Result<Option<String>, String> {
    if running && !r.daemon.backend.is_empty() {
        if let Some(why) = &r.daemon.unavailable {
            return Err(why.clone());
        }
        return one_mk_input::device_support(&r.daemon.backend).map(|n| n.map(str::to_owned)).map_err(str::to_owned);
    }
    guess_support().map(|n| n.map(str::to_owned)).map_err(str::to_owned)
}

/// "just now", "5 minutes ago", "3 days ago".
fn ago(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    let n = |k: u64, one: &str| format!("{k} {one}{} ago", if k == 1 { "" } else { "s" });
    match s {
        0..60 => "just now".into(),
        60..3600 => n(s / 60, "minute"),
        3600..86400 => n(s / 3600, "hour"),
        _ => n(s / 86400, "day"),
    }
}

/// `one-mk input-devices [--json]`.
pub async fn command(dir: &Path, json: bool) -> Result<()> {
    let running = matches!(crate::control::request(dir, crate::control::Request::Status).await, Ok(Some(_)));
    let (mut r, problem) = match Remembered::load(dir) {
        Ok(r) => (r, None),
        Err(e) => (Remembered::default(), Some(format!("{e:#}"))),
    };
    if !running {
        // A daemon that died can't have said what was unplugged since.
        r.devices.iter_mut().for_each(|d| d.connected = false);
    }
    let support = support(&r, running);
    if json {
        println!("{}", render_json(&r, running, &support, problem.as_deref()));
    } else {
        print!("{}", render_text(&r, running, &support, problem.as_deref(), now()));
    }
    Ok(())
}

pub fn render_text(r: &Remembered, running: bool, support: &Result<Option<String>, String>, problem: Option<&str>, now: u64) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    match support {
        Ok(None) => out.push_str("Per-device rules: available on this computer.\n"),
        Ok(Some(note)) => {
            let _ = writeln!(out, "Per-device rules: available on this computer, but {note}.");
        }
        Err(why) => {
            let mut why = why.clone();
            if let Some(first) = why.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            let _ = writeln!(out, "{why}.");
        }
    }
    if let Some(p) = problem {
        let _ = writeln!(out, "The list of devices seen can't be read ({p}); one-mk writes a new one when it next sees a device.");
    }
    if !running {
        out.push_str("one-mk isn't running, so it can't tell which devices are connected now (start it with `one-mk run`).\n");
    }
    if r.devices.is_empty() {
        out.push_str(if support.is_ok() {
            "No keyboard or mouse seen yet: a device is listed once it is connected while one-mk runs.\n"
        } else {
            "No keyboard or mouse can be listed here.\n"
        });
        return out;
    }
    out.push_str("\nKeyboards and mice seen on this computer:\n");
    let mut list: Vec<&Seen> = r.devices.iter().collect();
    list.sort_by_key(|d| (!d.connected, std::cmp::Reverse(d.last_seen)));
    for d in list {
        let Some(fp) = d.fingerprint() else { continue };
        let mut about = vec![d.kinds().to_owned(), fp.bus.words().to_owned(), fp.ids()];
        if let Some(s) = &fp.serial {
            about.push(if fp.bus == Bus::Bluetooth { format!("address {s}") } else { format!("serial {s}") });
        }
        let state = if d.connected {
            "connected".to_owned()
        } else if d.last_seen > 0 {
            format!("not connected (asleep or unplugged), last seen {}", ago(d.last_seen, now))
        } else {
            "not connected".to_owned()
        };
        let _ = writeln!(out, "\n  {} ({})\n    {state}\n    --device \"{}\"", d.name(), about.join(", "), d.fingerprint);
        if d.twins {
            out.push_str("    More than one device here has exactly this identity (two of the same model without a serial \
                          number, or one device shown twice): a rule for it applies to all of them.\n");
        }
    }
    if cfg!(windows) && r.devices.iter().filter_map(Seen::fingerprint).any(|f| f.bus == Bus::Usb) {
        out.push_str("\nOn Windows, devices paired to one USB receiver (dongle) may show up as the receiver itself: \
                      one-mk then can't tell them apart, and a rule for the receiver applies to all of them.\n");
    }
    out
}

pub fn render_json(r: &Remembered, running: bool, support: &Result<Option<String>, String>, problem: Option<&str>) -> String {
    let s = |v: &str| crate::doctor::json_str(v);
    let opt = |v: Option<&str>| v.map_or_else(|| "null".to_owned(), s);
    let devices: Vec<String> = r
        .devices
        .iter()
        .filter_map(|d| {
            let fp = d.fingerprint()?;
            Some(format!(
                "{{\"fingerprint\":{},\"name\":{},\"bus\":{},\"connection\":{},\"ids\":{},\"serial\":{},\"keyboard\":{},\"pointer\":{},\"connected\":{},\"first_seen\":{},\"last_seen\":{},\"twins\":{}}}",
                s(&d.fingerprint),
                s(&d.name()),
                s(fp.bus.as_str()),
                s(fp.bus.words()),
                s(&fp.ids()),
                opt(fp.serial.as_deref()),
                d.keyboard,
                d.pointer,
                d.connected,
                d.first_seen,
                d.last_seen,
                d.twins
            ))
        })
        .collect();
    let (available, note) = match support {
        Ok(n) => (true, n.as_deref()),
        Err(why) => (false, Some(why.as_str())),
    };
    format!(
        "{{\"available\":{available},\"note\":{},\"running\":{running},\"problem\":{},\"devices\":[{}]}}",
        opt(note),
        opt(problem),
        devices.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_proto::device::{DeviceRef, Kinds};

    fn dev(id: u32, fp: &str, keyboard: bool) -> InputDevice {
        InputDevice { id: DeviceRef(id), fingerprint: fp.parse().unwrap(), kinds: Kinds { keyboard, pointer: !keyboard } }
    }

    const KB: &str = "bluetooth/04e8:7021/f4:73:35:aa:bb:cc/Travel Keyboard";
    const MOUSE: &str = "usb/046d:c077//Logitech USB Optical Mouse";

    #[test]
    fn devices_are_remembered_with_when_they_came_and_went() {
        let mut r = Remembered::default();
        assert!(r.update(&[dev(10, KB, true), dev(11, MOUSE, false), dev(12, MOUSE, true)], 100));
        assert_eq!(r.devices.len(), 2, "one entry per fingerprint");
        let mouse = &r.devices[1];
        assert!(mouse.keyboard && mouse.pointer && mouse.connected && !mouse.twins, "{mouse:?}");
        assert!(!r.update(&[dev(10, KB, true), dev(11, MOUSE, false), dev(12, MOUSE, true)], 150), "nothing changed");
        assert!(r.update(&[dev(11, MOUSE, false), dev(12, MOUSE, true)], 200), "the keyboard sleeps");
        assert_eq!((r.devices[0].connected, r.devices[0].last_seen, r.devices[0].first_seen), (false, 200, 100));
        assert!(r.update(&[dev(13, KB, true), dev(11, MOUSE, false)], 300), "it wakes, with another id");
        assert_eq!((r.devices[0].connected, r.devices[0].last_seen), (true, 300));
        assert!(r.update(&[dev(13, KB, true), dev(14, KB, true)], 400));
        assert!(r.devices[0].twins, "two connected devices with one identity");
        assert!(r.disconnect_all(500));
        assert!(r.devices.iter().all(|d| !d.connected && !d.twins));
    }

    #[test]
    fn the_list_is_bounded_and_forgets_the_oldest_first() {
        let mut r = Remembered::default();
        for i in 0..(MAX as u32 + 10) {
            let fp = format!("usb/1234:{i:04x}//Keyboard {i}");
            r.update(&[dev(i, &fp, true)], u64::from(i));
        }
        r.update(&[dev(0, "usb/1234:0000//Keyboard 0", true)], 1000);
        assert_eq!(r.devices.len(), MAX);
        assert!(r.devices.iter().any(|d| d.fingerprint.ends_with("Keyboard 0")), "a connected device stays");
        assert!(!r.devices.iter().any(|d| d.fingerprint.ends_with("Keyboard 1")), "the oldest go");
    }

    #[test]
    fn devices_are_found_by_name_or_fingerprint() {
        let mut r = Remembered::default();
        r.update(&[dev(1, KB, true), dev(2, MOUSE, false), dev(3, "usb/046d:c52b//Logitech USB Receiver", true)], 1);
        assert_eq!(r.find("travel keyboard").unwrap(), KB);
        assert_eq!(r.find("Travel").unwrap(), KB, "a part of the name");
        assert_eq!(r.find(MOUSE).unwrap(), MOUSE);
        assert_eq!(r.find(" usb/046D:C999//Never Seen ").unwrap(), "usb/046d:c999//Never Seen", "any fingerprint, written canonically");
        let e = r.find("logitech").unwrap_err().to_string();
        assert!(e.contains("could be any of") && e.contains(MOUSE) && e.contains("Logitech USB Receiver"), "{e}");
        let e = r.find("trackball").unwrap_err().to_string();
        assert!(e.contains("no keyboard or mouse called \"trackball\"") && e.contains("one-mk input-devices"), "{e}");
        let e = r.find("usb/zz").unwrap_err().to_string();
        assert!(e.contains("--device"), "{e}");
        assert!(Remembered::default().find("x").unwrap_err().to_string().contains("seen on this computer yet"));
        assert!(r.find("  ").is_err());
    }

    #[test]
    fn the_file_round_trips_and_bad_files_are_survived() {
        let dir = std::env::temp_dir().join(format!("one-mk-inputdevices-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Remembered::load(&dir).unwrap(), Remembered::default(), "no file yet");
        let mut r = Remembered { daemon: DaemonNote { backend: "x11".into(), unavailable: None }, ..Default::default() };
        r.update(&[dev(1, KB, true)], 7);
        r.save(&dir).unwrap();
        assert_eq!(Remembered::load(&dir).unwrap(), r);
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());
        std::fs::write(path(&dir), "[[device]]\nfingerprint = \"nonsense\"\n[[device]]\nfingerprint = \"usb/0001:0002//ok\"\n").unwrap();
        assert_eq!(Remembered::load(&dir).unwrap().devices.len(), 1, "an unusable entry is dropped");
        std::fs::write(path(&dir), "this is = = not toml").unwrap();
        assert!(Remembered::load(&dir).unwrap_err().to_string().contains("can't be read"));
        std::fs::write(path(&dir), vec![b'#'; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(Remembered::load(&dir).unwrap_err().to_string().contains("larger than"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_list_reads_plainly() {
        let mut r = Remembered::default();
        r.update(&[dev(1, KB, true), dev(2, MOUSE, false), dev(3, MOUSE, false)], 1000);
        r.update(&[dev(2, MOUSE, false), dev(3, MOUSE, false)], 2000);
        let text = render_text(&r, true, &Ok(None), None, 2000 + 7200);
        assert!(text.starts_with("Per-device rules: available on this computer.\n"), "{text}");
        assert!(text.contains("Logitech USB Optical Mouse (mouse, USB, 046d:c077)\n    connected\n"), "{text}");
        assert!(text.contains("Travel Keyboard (keyboard, Bluetooth, 04e8:7021, address f4:73:35:aa:bb:cc)\n    not connected (asleep or unplugged), last seen 2 hours ago"), "{text}");
        assert!(text.contains("a rule for it applies to all of them"), "twins are said: {text}");
        assert!(text.find("Optical Mouse").unwrap() < text.find("Travel").unwrap(), "connected ones first");
        let off = render_text(&Remembered::default(), false, &Err(one_mk_input::MERGED_DEVICES.into()), None, 0);
        assert!(off.starts_with("Per-device rules are not available on this desktop: the desktop merges all keyboards and mice"), "{off}");
        assert!(off.contains("one-mk isn't running"));
        let json = render_json(&r, true, &Ok(Some("untested".into())), Some("bad \"file\""));
        assert!(json.starts_with("{\"available\":true,\"note\":\"untested\",\"running\":true,\"problem\":\"bad \\\"file\\\"\",\"devices\":[{"), "{json}");
        assert!(json.contains("\"connection\":\"Bluetooth\""), "{json}");
    }

    #[test]
    fn ages_read_plainly() {
        assert_eq!(ago(100, 100), "just now");
        assert_eq!(ago(100, 220), "2 minutes ago");
        assert_eq!(ago(0, 3600), "1 hour ago");
        assert_eq!(ago(0, 3 * 86400), "3 days ago");
        assert_eq!(ago(500, 100), "just now", "a clock that went back");
    }
}
