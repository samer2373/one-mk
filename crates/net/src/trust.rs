use crate::identity::Fingerprint;
use anyhow::{Context, Result};
use one_mk_proto::Side;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Whether Ctrl and Cmd (Meta) trade places for keys sent to a device, so
/// copy and paste shortcuts work between a Mac and a PC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Swap {
    /// Swap when exactly one of the two computers is a Mac.
    #[default]
    Auto,
    On,
    Off,
}

impl fmt::Display for Swap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Swap::Auto => "auto",
            Swap::On => "on",
            Swap::Off => "off",
        })
    }
}

impl FromStr for Swap {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Swap::Auto),
            "on" => Ok(Swap::On),
            "off" => Ok(Swap::Off),
            _ => Err(format!("expected on, off or auto, not {s:?}")),
        }
    }
}

/// A paired device.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub fingerprint: String,
    /// Which side of this machine the device sits on.
    #[serde(default = "default_side")]
    pub side: String,
    /// Last addresses it was reached at, for networks that block discovery.
    #[serde(default)]
    pub addrs: Vec<String>,
    /// When `side` (or `offset`) was last chosen (milliseconds since 1970,
    /// but always above the previous value, and above any version received
    /// from the device). Both computers keep the newer layout; a tie is
    /// broken by the larger fingerprint (`one_mk::manage::resolve_layout`).
    #[serde(default)]
    pub layout_version: u64,
    /// Ctrl/Cmd swapping for keys sent to this device.
    #[serde(default)]
    pub swap_ctrl_cmd: Swap,
    /// The device's OS when last connected (see `one_mk_proto::os`).
    #[serde(default)]
    pub os: u8,
    /// Where the device's desktop origin sits in this machine's logical
    /// pixels; the pointer crosses only where the screens then touch.
    /// None: anywhere along `side`, scaled to fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<(i32, i32)>,
    /// Who chose `side` and `offset` (see [`PlacedBy`]); only a guess may
    /// be moved by layout learning.
    #[serde(default, skip_serializing_if = "PlacedBy::is_default")]
    pub placed_by: PlacedBy,
}

/// Who chose where a device sits. Pushing the pointer toward a computer
/// (`[edge] learn`) may move it only while its place is a guess.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlacedBy {
    /// Not said: pairing's guess while `layout_version` is 0, the user's
    /// choice otherwise (files from before this field have only that).
    #[default]
    Default,
    /// `one-mk layout`, the window, a layout profile, or the other
    /// computer (whose user may have chosen it).
    User,
    /// Layout learning, from the pointer pushed toward it.
    Learned,
    /// The guessed or learned place has been used: the pointer crossed to
    /// the device through that edge.
    Confirmed,
    Pairing,
    Pending,
}

impl PlacedBy {
    fn is_default(&self) -> bool {
        *self == PlacedBy::Default
    }
}

impl<'de> Deserialize<'de> for PlacedBy {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "user" => PlacedBy::User,
            "learned" => PlacedBy::Learned,
            "confirmed" => PlacedBy::Confirmed,
            "pairing" => PlacedBy::Pairing,
            "pending" => PlacedBy::Pending,
            _ => PlacedBy::Default,
        })
    }
}

fn default_side() -> String {
    Side::Right.to_string()
}

impl Device {
    pub fn side(&self) -> Side {
        self.side.parse().unwrap_or(Side::Right)
    }
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint.parse().ok()
    }
    pub fn short_id(&self) -> String {
        self.fingerprint().map(|f| f.short()).unwrap_or_default()
    }
    /// Its place is only a guess that layout learning may change: pairing's
    /// default, or learned and not used since, and not at an exact offset.
    pub fn learnable(&self) -> bool {
        self.offset.is_none()
            && match self.placed_by {
                PlacedBy::Default => self.layout_version == 0,
                PlacedBy::Learned | PlacedBy::Pairing => true,
                PlacedBy::User | PlacedBy::Confirmed | PlacedBy::Pending => false,
            }
    }
}

/// Largest offset (either axis) kept from `devices.toml`, as on the wire
/// (`Control::Layout`): far beyond any desktop, and small enough that the
/// geometry never overflows. A hand edit past it is cut down to it.
pub const MAX_OFFSET: i32 = 1 << 24;

fn bounded_offset((x, y): (i32, i32)) -> (i32, i32) {
    (x.clamp(-MAX_OFFSET, MAX_OFFSET), y.clamp(-MAX_OFFSET, MAX_OFFSET))
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default, rename = "device")]
    devices: Vec<Device>,
}

/// Devices this machine has paired with, in `devices.toml`.
pub struct TrustStore {
    path: PathBuf,
    file: File,
}

impl TrustStore {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("devices.toml");
        let mut file: File = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("reading {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(e) => return Err(e).with_context(|| format!("reading paired computers {}", path.display())),
        };
        for device in &mut file.devices {
            device.offset = device.offset.map(bounded_offset);
        }
        Ok(Self { path, file })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            crate::identity::secure_dir(dir)?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        crate::identity::write_private(&tmp, toml::to_string_pretty(&self.file)?.as_bytes()).with_context(|| format!("writing paired computers {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).with_context(|| format!("saving paired computers {}", self.path.display()))?;
        Ok(())
    }

    pub fn devices(&self) -> &[Device] {
        &self.file.devices
    }

    pub fn get(&self, fp: &Fingerprint) -> Option<&Device> {
        let hex = fp.to_string();
        self.file.devices.iter().find(|d| d.fingerprint == hex)
    }

    pub fn get_mut(&mut self, fp: &Fingerprint) -> Option<&mut Device> {
        let hex = fp.to_string();
        self.file.devices.iter_mut().find(|d| d.fingerprint == hex)
    }

    pub fn is_trusted(&self, fp: &Fingerprint) -> bool {
        self.get(fp).is_some()
    }

    /// Adds or replaces a device, keeping its settings if already known.
    pub fn upsert(&mut self, mut device: Device) {
        if let Some(existing) = self.file.devices.iter_mut().find(|d| d.fingerprint == device.fingerprint) {
            device.side = std::mem::take(&mut existing.side);
            device.layout_version = existing.layout_version;
            device.swap_ctrl_cmd = existing.swap_ctrl_cmd;
            device.offset = existing.offset;
            device.placed_by = existing.placed_by;
            *existing = device;
        } else {
            self.file.devices.push(device);
        }
    }

    /// Finds a device by name (case-insensitive) or fingerprint prefix.
    /// A query that fits two devices finds neither: a computer's name is its
    /// own to choose, and must not stand in for another's ID.
    pub fn find(&self, query: &str) -> Option<&Device> {
        let q = query.replace('-', "").to_ascii_lowercase();
        let mut hits = self.file.devices.iter().filter(|d| d.name.eq_ignore_ascii_case(query) || (q.len() >= 4 && d.fingerprint.starts_with(&q)));
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    }

    pub fn remove(&mut self, query: &str) -> Option<Device> {
        let fp = self.find(query)?.fingerprint.clone();
        let i = self.file.devices.iter().position(|d| d.fingerprint == fp)?;
        Some(self.file.devices.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_and_pending_placements_round_trip_without_becoming_user_choices() {
        for origin in [PlacedBy::Pairing, PlacedBy::Pending] {
            let device = Device { placed_by: origin, layout_version: 16, ..Device::default() };
            let saved = toml::to_string(&device).unwrap();
            let restored: Device = toml::from_str(&saved).unwrap();
            assert_eq!(restored.placed_by, origin);
            assert_eq!(restored.learnable(), origin == PlacedBy::Pairing);
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_devices_are_owner_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("one-mk-private-devices-{}-{}", std::process::id(), rand::random::<u64>()));
        TrustStore::load(&dir).unwrap().save().unwrap();
        assert_eq!(std::fs::metadata(dir.join("devices.toml")).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A hand-edited offset too large for the geometry is cut down on load;
    /// the `align` of builds before the placement model is ignored (it was
    /// never released), so that device has no offset.
    #[test]
    fn hand_edited_offsets_are_normalized_on_load() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-offset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fp = "ab".repeat(32);
        let text = format!("[[device]]\nname = 'peer'\nfingerprint = '{fp}'\nside = 'left'\noffset = [{}, 5]\n[[device]]\nname = 'old'\nfingerprint = '{}'\nalign = '300'\n", i32::MIN, "cd".repeat(32));
        std::fs::write(dir.join("devices.toml"), text).unwrap();
        let trust = TrustStore::load(&dir).unwrap();
        assert_eq!(trust.devices()[0].offset, Some((-MAX_OFFSET, 5)));
        assert_eq!(trust.devices()[1].offset, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Who placed a device: a file from before `placed_by` reads as the
    /// user's choice once a layout was chosen, pairing's guess before;
    /// the field is written only when it says more than that.
    #[test]
    fn placement_origin_and_what_learning_may_move() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-placed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = ("ab".repeat(32), "cd".repeat(32), "ef".repeat(32));
        let text = format!(
            "[[device]]\nname = 'guess'\nfingerprint = '{a}'\nside = 'right'\n\
             [[device]]\nname = 'chosen'\nfingerprint = '{b}'\nside = 'left'\nlayout_version = 1700000000000\n\
             [[device]]\nname = 'learned'\nfingerprint = '{c}'\nside = 'top'\nlayout_version = 5\nplaced_by = 'learned'\n"
        );
        std::fs::write(dir.join("devices.toml"), text).unwrap();
        let mut t = TrustStore::load(&dir).unwrap();
        let learnable: Vec<bool> = t.devices().iter().map(Device::learnable).collect();
        assert_eq!(learnable, [true, false, true]);
        let guess = t.get_mut(&a.parse().unwrap()).unwrap();
        guess.offset = Some((1920, 0));
        assert!(!guess.learnable(), "an exact place is never a guess");
        guess.offset = None;
        guess.placed_by = PlacedBy::Confirmed;
        assert!(!guess.learnable());
        t.save().unwrap();
        let saved = std::fs::read_to_string(dir.join("devices.toml")).unwrap();
        assert_eq!(saved.matches("placed_by").count(), 2, "{saved}");
        assert!(saved.contains("placed_by = \"confirmed\"") && saved.contains("placed_by = \"learned\""));
        assert!(TrustStore::load(&dir).is_ok());
        // A value this build doesn't know (a newer build's, a typo) is no
        // reason to lose the paired computers: it reads as not said.
        std::fs::write(dir.join("devices.toml"), format!("[[device]]\nname = 'x'\nfingerprint = '{a}'\nplaced_by = 'robot'\n")).unwrap();
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].placed_by, PlacedBy::Default);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A name that is also another computer's ID never picks either.
    #[test]
    fn an_ambiguous_query_finds_nothing() {
        let dev = |name: &str, fp: &str| Device { name: name.into(), fingerprint: fp.into(), ..Default::default() };
        let dir = std::env::temp_dir().join(format!("one-mk-trust-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = TrustStore::load(&dir).unwrap();
        t.upsert(dev("abcd1234", &"ee".repeat(32)));
        t.upsert(dev("victim", &format!("abcd1234{}", "00".repeat(28))));
        assert!(t.find("abcd1234").is_none());
        assert_eq!(t.find("victim").unwrap().name, "victim");
        assert_eq!(t.find("eeeeeeee").unwrap().name, "abcd1234");
    }

    /// A `devices.toml` from before layout versions and Ctrl/Cmd settings
    /// still loads, with defaults.
    #[test]
    fn old_files_still_load() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fp = "ab".repeat(32);
        std::fs::write(dir.join("devices.toml"), format!("[[device]]\nname = \"old\"\nfingerprint = \"{fp}\"\nside = \"left\"\n")).unwrap();
        let mut t = TrustStore::load(&dir).unwrap();
        let d = &t.devices()[0];
        assert_eq!((d.side(), d.layout_version, d.swap_ctrl_cmd, d.os), (Side::Left, 0, Swap::Auto, 0));
        assert_eq!(d.offset, None);
        t.get_mut(&fp.parse().unwrap()).unwrap().swap_ctrl_cmd = Swap::Off;
        t.save().unwrap();
        assert!(!std::fs::read_to_string(dir.join("devices.toml")).unwrap().contains("offset"), "the default isn't written");
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].swap_ctrl_cmd, Swap::Off);
        assert_eq!(t.devices()[0].offset, None);
        t.get_mut(&fp.parse().unwrap()).unwrap().offset = Some((-3000, -900));
        t.save().unwrap();
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].offset, Some((-3000, -900)));
        // Pairing again keeps the layout.
        let mut t = TrustStore::load(&dir).unwrap();
        t.get_mut(&fp.parse().unwrap()).unwrap().placed_by = PlacedBy::Learned;
        t.upsert(Device { name: "old".into(), fingerprint: fp.clone(), ..Device::default() });
        assert_eq!(t.devices()[0].offset, Some((-3000, -900)));
        assert_eq!(t.devices()[0].placed_by, PlacedBy::Learned);
        assert_eq!("ON".parse::<Swap>(), Ok(Swap::On));
        assert!("maybe".parse::<Swap>().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
