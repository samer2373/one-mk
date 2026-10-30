use crate::settings::{self, Settings};
use anyhow::{Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

pub const UNAVAILABLE: &str = "physical (native character resolution/injection is not implemented; use matching keyboard layouts on both computers)";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Physical,
    Characters,
    Auto,
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Physical => "physical",
            Self::Characters => "characters",
            Self::Auto => "auto",
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypingSettings {
    pub keys: Mode,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub peers: BTreeMap<String, Mode>,
}

impl TypingSettings {
    pub fn requested(&self, peer: Option<&str>) -> Mode {
        peer.and_then(|peer| self.peers.get(peer)).copied().unwrap_or(self.keys)
    }

    pub fn summary(&self) -> String {
        if self.keys == Mode::Physical && self.peers.values().all(|mode| *mode == Mode::Physical) {
            "physical (same keys; receiver keyboard layout)".into()
        } else {
            format!("{UNAVAILABLE}; requested {}, {} peer override(s)", self.keys, self.peers.len())
        }
    }

    pub fn has_character_request(&self) -> bool {
        self.keys != Mode::Physical || self.peers.values().any(|mode| *mode != Mode::Physical)
    }
}

pub async fn run(dir: &Path, keys: Option<Mode>, peer: Option<&str>) -> Result<()> {
    let peer = if let Some(query) = peer {
        let store = one_mk_net::TrustStore::load(dir)?;
        let Some(device) = store.find(query) else {
            bail!("no unique paired computer matches {query:?}; use its id from `one-mk devices`");
        };
        Some(device.fingerprint.clone())
    } else {
        None
    };
    let typing = if let Some(mode) = keys {
        Settings::update(dir, |settings| {
            if let Some(peer) = &peer {
                settings.typing.peers.insert(peer.clone(), mode);
            } else {
                settings.typing.keys = mode;
            }
            Ok(settings.typing.clone())
        })?
    } else {
        Settings::load(dir)?.typing
    };
    println!("Typing preference: {}{}.", typing.requested(peer.as_deref()), peer.as_ref().map_or(String::new(), |peer| format!(" for {peer}")));
    println!("Typing effective: {UNAVAILABLE}.");
    if keys.is_some() {
        settings::apply(dir).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_settings_preserve_physical_default() {
        let settings: Settings = toml::from_str("settings_version = 2\nnotify_files = false\n").unwrap();
        assert_eq!(settings.typing, TypingSettings::default());
        assert!(!settings.notify_files);
        assert!(!settings.typing.has_character_request());
    }

    #[test]
    fn peer_override_round_trips_without_changing_other_settings() {
        let mut settings = Settings { notify_files: false, ..Settings::default() };
        settings.typing.keys = Mode::Auto;
        settings.typing.peers.insert("peer-id".into(), Mode::Characters);
        let encoded = toml::to_string_pretty(&settings).unwrap();
        let decoded: Settings = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded, settings);
        assert_eq!(decoded.typing.requested(Some("peer-id")), Mode::Characters);
        assert_eq!(decoded.typing.requested(Some("other")), Mode::Auto);
        assert!(decoded.typing.summary().contains("not implemented"));
    }

    #[test]
    fn unknown_modes_are_actionable_parse_errors() {
        assert!(toml::from_str::<Settings>("[typing]\nkeys = 'german'\n").unwrap_err().to_string().contains("physical"));
    }
}
