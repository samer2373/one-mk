//! Where a Linux desktop keeps per-user configuration and data: the base
//! directories of the XDG specification. Every place oneMK writes an
//! autostart entry, a menu entry, a script or an extension asks here, so
//! none of them ends up under `~/.config` while the file manager's entries go
//! to `$XDG_DATA_HOME` (as happened when only some honoured the variables).
//!
//! A value that is empty or not an absolute path is ignored, as the
//! specification says, and the default under the home folder is used.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::path::PathBuf;

/// `$XDG_CONFIG_HOME`, or `~/.config`.
pub fn config_home() -> Result<PathBuf> {
    base("XDG_CONFIG_HOME", std::env::var_os("XDG_CONFIG_HOME"), ".config")
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
pub fn data_home() -> Result<PathBuf> {
    base("XDG_DATA_HOME", std::env::var_os("XDG_DATA_HOME"), ".local/share")
}

fn base(name: &str, value: Option<OsString>, default: &str) -> Result<PathBuf> {
    if let Some(path) = chosen(value) {
        return Ok(path);
    }
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()).with_context(|| format!("no home directory, and {name} is not set"))?;
    Ok(home.join(default))
}

/// The value of an XDG variable if it can be used.
fn chosen(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    // The values are Unix paths: `/data/x` is not absolute on Windows.
    #[test]
    fn only_an_absolute_value_is_used() {
        assert_eq!(chosen(Some("/data/x".into())), Some(PathBuf::from("/data/x")));
        for bad in ["", "relative/dir", "./here", "~/data"] {
            assert_eq!(chosen(Some(bad.into())), None, "{bad:?}");
        }
        assert_eq!(chosen(None), None);
    }

    #[test]
    fn the_default_is_under_the_home_folder() {
        let home = directories::BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(base("XDG_DATA_HOME", None, ".local/share").unwrap(), home.join(".local/share"));
        assert_eq!(base("XDG_DATA_HOME", Some("/elsewhere".into()), ".local/share").unwrap(), PathBuf::from("/elsewhere"));
        assert_eq!(base("XDG_CONFIG_HOME", Some("rel".into()), ".config").unwrap(), home.join(".config"));
    }
}
