//! Settings that are not about one paired device: what the clipboard
//! shares, and whether received files raise a notification. They live in
//! `settings.toml` next to `devices.toml`; the command line edits the file
//! and sends the daemon a `settings` request, which applies it without a
//! restart (the same way `one-mk layout` applies `devices.toml`).

use anyhow::{Context, Result};
use one_mk_clipboard::{CEILING_BYTES, DEFAULT_MAX_BYTES, Filter};
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "settings.toml";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipSettings {
    /// Share plain text.
    pub text: bool,
    /// Share HTML (rich text).
    pub html: bool,
    /// Share images.
    pub image: bool,
    /// Largest clipboard to send or accept, in MB (1 MB = 2^20 bytes).
    pub max_mb: u32,
    /// Don't share what a password manager marked as sensitive.
    pub skip_sensitive: bool,
    /// Let a paired computer fetch this clipboard (`one-mk clip-get` there).
    pub allow_pull: bool,
}

impl Default for ClipSettings {
    fn default() -> Self {
        Self {
            text: true,
            html: true,
            image: true,
            max_mb: (DEFAULT_MAX_BYTES >> 20) as u32,
            skip_sensitive: true,
            allow_pull: true,
        }
    }
}

impl ClipSettings {
    /// The most `max_mb` can be set to.
    pub const MAX_MB_LIMIT: u32 = (CEILING_BYTES >> 20) as u32;

    pub fn filter(&self) -> Filter {
        Filter {
            text: self.text,
            html: self.html,
            image: self.image,
            max_bytes: (self.max_mb.clamp(1, Self::MAX_MB_LIMIT) as usize) << 20,
            skip_concealed: self.skip_sensitive,
        }
    }

    pub fn summary(&self) -> String {
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "text {}, html {}, images {}, size limit {} MB, skip password managers {}, others may fetch it {}",
            on(self.text),
            on(self.html),
            on(self.image),
            self.max_mb.clamp(1, Self::MAX_MB_LIMIT),
            on(self.skip_sensitive),
            on(self.allow_pull)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub clipboard: ClipSettings,
    /// Show a desktop notification when files arrive.
    pub notify_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { clipboard: ClipSettings::default(), notify_files: true }
    }
}

impl Settings {
    /// Reads `settings.toml`; a missing file means the defaults.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults_share_everything_up_to_25_mb() {
        let s = Settings::default();
        assert!(s.clipboard.text && s.clipboard.html && s.clipboard.image && s.notify_files);
        assert_eq!(s.clipboard.filter(), Filter::default());
        assert_eq!(s.clipboard.max_mb, 25);
    }

    #[test]
    fn saved_settings_come_back_and_a_missing_file_is_the_defaults() {
        let dir = temp("round");
        assert_eq!(Settings::load(&dir).unwrap(), Settings::default());
        let s = Settings {
            clipboard: ClipSettings { image: false, max_mb: 3, allow_pull: false, ..Default::default() },
            notify_files: false,
        };
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir).unwrap(), s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_and_future_files_still_load() {
        let dir = temp("partial");
        std::fs::write(dir.join(FILE), "[clipboard]\nmax_mb = 5\nsome_future_key = 1\n").unwrap();
        let s = Settings::load(&dir).unwrap();
        assert_eq!(s.clipboard.max_mb, 5);
        assert!(s.clipboard.text && s.notify_files, "what the file doesn't say keeps its default");
        std::fs::write(dir.join(FILE), "clipboard = [").unwrap();
        assert!(Settings::load(&dir).is_err(), "a broken file is reported, not silently reset");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_size_limit_is_kept_sane() {
        let f = |mb| ClipSettings { max_mb: mb, ..Default::default() }.filter().max_bytes();
        assert_eq!(f(0), 1 << 20, "never zero");
        assert_eq!(f(25), 25 << 20);
        assert_eq!(f(u32::MAX), CEILING_BYTES);
    }
}
