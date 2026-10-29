use crate::identity::Fingerprint;
use anyhow::{Context, Result};
use one_mk_proto::{Align, Side};
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
    /// How its facing edge lines up with ours along that side
    /// ([`Align`] as text: `stretch`, `center` or pixels); empty is
    /// stretch, the default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
    /// When `side` (or `align`) was last chosen (milliseconds since 1970,
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
}

fn default_side() -> String {
    Side::Right.to_string()
}

impl Device {
    pub fn side(&self) -> Side {
        self.side.parse().unwrap_or(Side::Right)
    }
    /// A value that doesn't parse (a bad hand edit) is the default.
    pub fn align(&self) -> Align {
        bounded_align(self.align.parse().unwrap_or_default())
    }
    pub fn set_align(&mut self, align: Align) {
        let align = bounded_align(align);
        self.align = if align == Align::Stretch { String::new() } else { align.to_string() };
    }
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint.parse().ok()
    }
    pub fn short_id(&self) -> String {
        self.fingerprint().map(|f| f.short()).unwrap_or_default()
    }
}

fn bounded_align(align: Align) -> Align {
    match align {
        Align::Offset(offset) => Align::Offset(offset.clamp(-100000, 100000)),
        other => other,
    }
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
            Err(e) => return Err(e.into()),
        };
        for device in &mut file.devices {
            if let Ok(Align::Offset(offset)) = device.align.parse() { device.set_align(Align::Offset(offset)); }
        }
        Ok(Self { path, file })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            crate::identity::secure_dir(dir)?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(&self.file)?)?;
        std::fs::rename(&tmp, &self.path)?;
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
            device.align = std::mem::take(&mut existing.align);
            device.layout_version = existing.layout_version;
            device.swap_ctrl_cmd = existing.swap_ctrl_cmd;
            *existing = device;
        } else {
            self.file.devices.push(device);
        }
    }

    /// Finds a device by name (case-insensitive) or fingerprint prefix.
    pub fn find(&self, query: &str) -> Option<&Device> {
        let q = query.replace('-', "").to_ascii_lowercase();
        self.file
            .devices
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(query) || (q.len() >= 4 && d.fingerprint.starts_with(&q)))
    }

    pub fn remove(&mut self, query: &str) -> Option<Device> {
        let fp = self.find(query)?.fingerprint.clone();
        let i = self.file.devices.iter().position(|d| d.fingerprint == fp)?;
        Some(self.file.devices.remove(i))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hand_edited_offsets_are_normalized_on_load() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-offset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("devices.toml"), format!("[[device]]\nname = 'peer'\nfingerprint = '{}'\nside = 'left'\nalign = '{}'\n", "ab".repeat(32), i32::MIN)).unwrap();
        let trust = TrustStore::load(&dir).unwrap();
        assert_eq!(trust.devices()[0].align, "-100000");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn hostile_alignment_is_bounded() {
        for (raw, expected) in [(i32::MAX, 100000), (i32::MIN, -100000)] {
            let mut device = Device { align: raw.to_string(), ..Device::default() };
            assert_eq!(device.align(), Align::Offset(expected));
            device.set_align(Align::Offset(raw));
            assert_eq!(device.align, expected.to_string());
        }
    }
    use super::*;

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
        assert_eq!(d.align(), Align::Stretch);
        t.get_mut(&fp.parse().unwrap()).unwrap().swap_ctrl_cmd = Swap::Off;
        t.save().unwrap();
        assert!(!std::fs::read_to_string(dir.join("devices.toml")).unwrap().contains("align"), "the default isn't written");
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].swap_ctrl_cmd, Swap::Off);
        t.get_mut(&fp.parse().unwrap()).unwrap().set_align(Align::Offset(-300));
        t.save().unwrap();
        // Pairing again keeps the layout.
        let mut t = TrustStore::load(&dir).unwrap();
        t.upsert(Device { name: "old".into(), fingerprint: fp.clone(), ..Device::default() });
        assert_eq!(t.devices()[0].align(), Align::Offset(-300));
        assert_eq!("ON".parse::<Swap>(), Ok(Swap::On));
        assert!("maybe".parse::<Swap>().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
