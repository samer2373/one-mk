// SPDX-License-Identifier: MIT OR Apache-2.0
//! `one-mk pointer-extension`: the optional GNOME Shell extension that hides
//! this computer's pointer while the mouse is on another computer (see
//! `crates/input/src/portal_pointer.rs`). Installing copies three files into
//! the user's extensions folder; nothing is downloaded, enabled or changed
//! until asked, and GNOME Wayland loads a new extension at the next login.
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const UUID: &str = "one-mk-pointer@one-mk.org";
const MARKER: &str = ".one-mk-install.json";
const FILES: &[(&str, &str)] = &[
    ("metadata.json", include_str!("../../../extensions/gnome/one-mk-pointer@one-mk.org/metadata.json")),
    ("extension.js", include_str!("../../../extensions/gnome/one-mk-pointer@one-mk.org/extension.js")),
    ("bridge.xml", include_str!("../../../extensions/gnome/one-mk-pointer@one-mk.org/bridge.xml")),
];

#[derive(Subcommand)]
pub enum Action {
    /// Copy the extension into your GNOME extensions folder (it is not enabled).
    Install,
    /// Ask GNOME to enable the installed extension.
    Enable,
    /// Show whether it is installed and active.
    Status,
    /// Remove the installed files (disable it first: `gnome-extensions disable one-mk-pointer@one-mk.org`).
    Uninstall,
}

fn destination() -> Result<PathBuf> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(data) => PathBuf::from(data),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".local/share"),
    };
    if !data.is_absolute() {
        bail!("HOME/XDG_DATA_HOME must be absolute");
    }
    Ok(data.join("gnome-shell/extensions").join(UUID))
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn regular(path: &Path) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path).with_context(|| format!("inspecting {}", path.display()))?;
    if !metadata.is_file() {
        bail!("refusing non-regular file {}", path.display());
    }
    Ok(metadata)
}

/// Whether `dir` holds exactly the files this command installed, unmodified
/// (`Ok(true)`), nothing (`Ok(false)`), or something else (an error).
fn managed(dir: &Path) -> Result<bool> {
    if std::fs::symlink_metadata(dir).is_err() {
        return Ok(false);
    }
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        bail!("{} is not a folder; leaving it alone", dir.display());
    }
    let marker = dir.join(MARKER);
    if regular(&marker).is_err() || regular(&marker)?.len() > 4096 {
        bail!("{} was not installed by one-mk; leaving it alone", dir.display());
    }
    let manifest: Value = serde_json::from_slice(&std::fs::read(&marker)?).context("invalid install manifest")?;
    if manifest["uuid"] != UUID {
        bail!("{} belongs to another extension; leaving it alone", dir.display());
    }
    let hashes = manifest["files"].as_object().context("invalid installed file list")?;
    if hashes.len() != FILES.len() || std::fs::read_dir(dir)?.count() != FILES.len() + 1 {
        bail!("{} has files one-mk did not install; leaving it alone", dir.display());
    }
    for (name, expected) in hashes {
        let path = dir.join(name);
        regular(&path)?;
        if expected.as_str() != Some(hash(&std::fs::read(&path)?).as_str()) {
            bail!("{} was modified; keep or remove it yourself", path.display());
        }
    }
    Ok(true)
}

fn write(dir: &Path) -> Result<()> {
    let parent = dir.parent().context("missing extensions folder")?;
    std::fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".one-mk-install-{}-{}", std::process::id(), rand::random::<u64>()));
    std::fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        let mut hashes = serde_json::Map::new();
        for &(name, contents) in FILES {
            std::fs::write(stage.join(name), contents)?;
            hashes.insert(name.into(), json!(hash(contents.as_bytes())));
        }
        std::fs::write(stage.join(MARKER), serde_json::to_vec(&json!({"uuid": UUID, "version": 1, "files": hashes}))?)?;
        std::fs::rename(&stage, dir)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

pub async fn command(action: &Action) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("The pointer extension is for GNOME on Linux only.");
    }
    let dir = destination()?;
    match action {
        Action::Install => {
            let upgrade = managed(&dir)?;
            if upgrade {
                std::fs::remove_dir_all(&dir)?;
            }
            write(&dir)?;
            println!("{} {}.", if upgrade { "Updated" } else { "Installed" }, dir.display());
            if upgrade {
                println!("GNOME loads an extension's code once per login: log out and back in once to use the new version.");
            } else {
                println!("Not enabled yet. GNOME on Wayland loads a new extension at your next login: log out and back in,");
                println!("then run `one-mk pointer-extension enable`. The pointer on this computer is then hidden while");
                println!("the mouse is on another one, and comes back by itself if one-mk stops.");
            }
        }
        Action::Uninstall => {
            if !managed(&dir)? {
                println!("Nothing installed.");
                return Ok(());
            }
            if let Some(state) = linux::state().await.ok().flatten()
                && state == 1
            {
                bail!("It is enabled. Disable it first: gnome-extensions disable {UUID}");
            }
            std::fs::remove_dir_all(&dir)?;
            println!("Removed {}.", dir.display());
        }
        Action::Enable => {
            if !managed(&dir)? {
                bail!("Not installed. Run `one-mk pointer-extension install` first.");
            }
            linux::enable().await?;
            println!("GNOME enabled the extension. The pointer is hidden while the mouse is on another computer.");
        }
        Action::Status => {
            let installed = managed(&dir)?;
            println!("Installed: {}", if installed { "yes" } else { "no" });
            if installed {
                match linux::state().await {
                    Ok(Some(1)) => println!("GNOME: active. The pointer is hidden while the mouse is on another computer."),
                    Ok(Some(state)) => println!("GNOME: not active (state {state}). Run `one-mk pointer-extension enable`."),
                    Ok(None) => println!("GNOME doesn't know it yet. Log out and back in, then run `one-mk pointer-extension enable`."),
                    Err(e) => println!("GNOME: could not ask ({e:#})."),
                }
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::UUID;
    use anyhow::{Result, ensure};
    use std::collections::HashMap;
    use std::time::Duration;

    async fn extensions() -> Result<zbus::Proxy<'static>> {
        let connection = zbus::connection::Builder::session()?.method_timeout(Duration::from_secs(2)).build().await?;
        Ok(zbus::Proxy::new(&connection, "org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions").await?)
    }

    /// GNOME's state number for the extension (1 is active), if it knows it.
    pub async fn state() -> Result<Option<u32>> {
        let info: HashMap<String, zbus::zvariant::OwnedValue> = extensions().await?.call("GetExtensionInfo", &(UUID,)).await?;
        Ok(info.get("state").and_then(|s| f64::try_from(s).ok()).filter(|s| s.is_finite() && (0.0..=99.0).contains(s)).map(|s| s as u32))
    }

    pub async fn enable() -> Result<()> {
        ensure!(state().await?.is_some(), "GNOME doesn't know the extension yet. Log out and back in once, then try again.");
        let enabled: bool = extensions().await?.call("EnableExtension", &(UUID,)).await?;
        ensure!(enabled, "GNOME could not enable the extension. Log out and back in once, then try again.");
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
mod linux {
    pub async fn state() -> anyhow::Result<Option<u32>> {
        Ok(None)
    }
    pub async fn enable() -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-pointer-ext-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_bundled_extension_is_complete() {
        let metadata: Value = serde_json::from_str(FILES[0].1).unwrap();
        assert_eq!(metadata["uuid"], UUID);
        assert!(FILES[1].1.contains("org.onemk.Pointer1") && FILES[2].1.contains("org.onemk.Pointer1"));
    }

    #[test]
    fn install_is_recognised_changed_files_and_strangers_are_left_alone() {
        let root = temp("managed");
        let dir = root.join("ext").join(UUID);
        assert!(!managed(&dir).unwrap(), "nothing there yet");
        write(&dir).unwrap();
        assert!(managed(&dir).unwrap());
        for (name, contents) in FILES {
            assert_eq!(&std::fs::read_to_string(dir.join(name)).unwrap(), contents);
        }
        // Edited by hand: not ours to replace or remove.
        std::fs::write(dir.join("extension.js"), "// mine").unwrap();
        assert!(managed(&dir).unwrap_err().to_string().contains("modified"));
        // A folder that isn't ours at all.
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("extension.js"), "x").unwrap();
        assert!(managed(&other).unwrap_err().to_string().contains("not installed by one-mk"));
        // An extra file.
        write(&root.join("again")).unwrap();
        std::fs::write(root.join("again/extra.txt"), "x").unwrap();
        assert!(managed(&root.join("again")).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
