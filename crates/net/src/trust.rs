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
    /// When `side` was last chosen (milliseconds since 1970, but always
    /// above the previous value). Both computers keep the newer layout.
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
        let file = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("reading {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(e) => return Err(e.into()),
        };
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
            device.layout_version = existing.layout_version;
            device.swap_ctrl_cmd = existing.swap_ctrl_cmd;
            device.offset = existing.offset;
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
        t.get_mut(&fp.parse().unwrap()).unwrap().swap_ctrl_cmd = Swap::Off;
        t.save().unwrap();
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].swap_ctrl_cmd, Swap::Off);
        assert_eq!(t.devices()[0].offset, None);
        t.get_mut(&fp.parse().unwrap()).unwrap().offset = Some((-3000, -900));
        t.save().unwrap();
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].offset, Some((-3000, -900)));
        assert_eq!("ON".parse::<Swap>(), Ok(Swap::On));
        assert!("maybe".parse::<Swap>().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
