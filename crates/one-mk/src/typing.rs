use crate::settings::{self, Settings};
use anyhow::Result;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Said wherever letters (characters, auto) are offered.
pub const EXPERIMENTAL: &str = "experimental: it works only toward Windows and Mac computers, and has not been tested on real hardware";

pub const UNAVAILABLE: &str = "physical (letters follow the receiving computer's keyboard layout)";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The same physical keys; letters follow the other computer's layout (default).
    #[default]
    Physical,
    /// The letters printed on your keys (experimental).
    Characters,
    /// Characters only when the two layouts differ (experimental).
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
            format!("requested {} ({EXPERIMENTAL}); characters need a supported sender and receiver, otherwise {UNAVAILABLE}; {} peer override(s)", self.keys, self.peers.len())
        }
    }

    pub fn has_character_request(&self) -> bool {
        self.keys != Mode::Physical || self.peers.values().any(|mode| *mode != Mode::Physical)
    }
}

/// What `mode` will do toward one paired computer, as its own words for it:
/// letters are only ever typed by Windows and Mac receivers, and then only
/// if that computer runs a version that can (it says so when it connects; this
/// command does not know) and this one can read its own keyboard layout. So the
/// line says what is asked for and when it applies, and "Right now" below it
/// says what is in effect.
pub fn receiver_line(name: &str, os: u8, mode: Mode) -> String {
    use one_mk_proto::os;
    const WHEN: &str = "when that computer runs a version that supports it and this one can read its keyboard layout; otherwise the same physical keys";
    let (system, what) = match os {
        os::WINDOWS => ("Windows", true),
        os::MACOS => ("Mac", true),
        os::LINUX => ("Linux", false),
        _ => ("unknown system", false),
    };
    match (mode, what) {
        (Mode::Physical, _) => format!("{name} ({system}): the same physical keys, so letters follow its own keyboard layout."),
        (Mode::Characters, true) => format!("{name} ({system}): asks to type the letters printed on your keys, {WHEN} (experimental: not yet tested on a real {system} computer; shortcuts still use the keys)."),
        (Mode::Auto, true) => format!("{name} ({system}): asks to type the letters printed on your keys only when your two keyboard layouts differ, otherwise the same keys; it applies {WHEN} (experimental: not yet tested on a real {system} computer)."),
        (_, false) if os == os::LINUX => format!("{name} (Linux): keeps using the same physical keys: Linux computers can't type letters yet, so this changes nothing for it."),
        _ => format!("{name}: its system is not known yet, so it keeps using the same physical keys until it connects."),
    }
}

pub async fn run(dir: &Path, keys: Option<Mode>, peer: Option<&str>) -> Result<()> {
    let mut peer_name = None;
    let mut peer_os = None;
    let peer = if let Some(query) = peer {
        let store = one_mk_net::TrustStore::load(dir)?;
        let Some(device) = store.find(query) else { return Err(crate::manage::not_found(&store, query)) };
        peer_name = Some(store.display(device));
        peer_os = Some(device.os);
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
        Settings::view(dir)?.typing
    };
    let requested = typing.requested(peer.as_deref());
    println!("Typing preference: {requested}{}.", peer_name.as_ref().map_or(String::new(), |name| format!(" for {name}")));
    if typing.has_character_request() {
        println!("Letters (characters, auto) are {EXPERIMENTAL}.");
    }
    // What each paired computer will really do with it.
    match (&peer_name, peer_os) {
        (Some(name), Some(os)) => println!("  {}", receiver_line(name, os, requested)),
        _ => {
            if let Ok(store) = one_mk_net::TrustStore::load(dir) {
                for device in store.devices() {
                    println!("  {}", receiver_line(&store.display(device), device.os, typing.requested(Some(&device.fingerprint))));
                }
            }
        }
    }
    // Tell a running one-mk first, so "Right now" is what it does after this change.
    if keys.is_some() {
        settings::apply(dir).await?;
    }
    let effective = crate::control::request(dir, crate::control::Request::Status).await.ok().flatten()
        .and_then(|text| crate::manage::Status::decode(&text)).map(|status| status.typing);
    match effective {
        Some(effective) => println!("Right now: {effective}."),
        // `apply` has already said so after a change.
        None if keys.is_some() => {}
        None => println!("one-mk is not running; this applies the next time it starts."),
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
        assert!(decoded.typing.summary().contains("supported sender and receiver"));
    }

    #[test]
    fn each_receiver_is_told_what_will_really_happen() {
        use one_mk_proto::os;
        assert!(receiver_line("Desk", os::LINUX, Mode::Characters).contains("keeps using the same physical keys"));
        assert!(receiver_line("Air", os::MACOS, Mode::Characters).contains("experimental"));
        assert!(receiver_line("Air", os::MACOS, Mode::Auto).contains("only when your two keyboard layouts differ"));
        assert!(receiver_line("Box", os::WINDOWS, Mode::Physical).contains("physical keys"));
        assert!(receiver_line("New", os::UNKNOWN, Mode::Characters).contains("not known"));
    }

    /// A Windows or Mac computer may run a version without letter typing, and
    /// the sender may not read its own layout: the per-computer line must not
    /// promise letters, only ask for them and say when they apply.
    #[test]
    fn a_windows_or_mac_line_does_not_promise_letters() {
        use one_mk_proto::os;
        for (system, mode) in [(os::WINDOWS, Mode::Characters), (os::MACOS, Mode::Characters), (os::WINDOWS, Mode::Auto)] {
            let line = receiver_line("Desk", system, mode);
            assert!(line.contains("asks to type") && line.contains("version that supports it") && line.contains("otherwise the same physical keys"), "{line}");
            assert!(!line.contains(": types the letters"), "{line}");
        }
    }

    #[test]
    fn unknown_modes_are_actionable_parse_errors() {
        assert!(toml::from_str::<Settings>("[typing]\nkeys = 'german'\n").unwrap_err().to_string().contains("physical"));
    }
}
