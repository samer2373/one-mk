// SPDX-License-Identifier: MIT OR Apache-2.0
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const UUID: &str = "one-mk-wallpaper@one-mk.org";
const MARKER: &str = ".one-mk-install.json";
const FILES: &[(&str, &str)] = &[
    ("metadata.json", include_str!("../../../extensions/gnome/one-mk-wallpaper@one-mk.org/metadata.json")),
    ("extension.js", include_str!("../../../extensions/gnome/one-mk-wallpaper@one-mk.org/extension.js")),
    ("bridge.xml", include_str!("../../../extensions/gnome/one-mk-wallpaper@one-mk.org/bridge.xml")),
    ("LICENSE-MIT", include_str!("../../../LICENSE-MIT")),
    ("LICENSE-APACHE", include_str!("../../../LICENSE-APACHE")),
];

#[derive(Subcommand)]
pub enum Command {
    /// Install the managed extension files; enabling it is a separate step.
    Install {
        /// Absolute path to the separately installed renderer executable.
        #[arg(long)] renderer: Option<PathBuf>,
    },
    /// Remove managed extension files after disabling the extension.
    Uninstall,
    /// Show installation, session and qualification status.
    Status,
}

fn destination() -> Result<PathBuf> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(data) => PathBuf::from(data),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".local/share"),
    };
    if !data.is_absolute() { bail!("HOME/XDG_DATA_HOME must be absolute"); }
    Ok(data.join("gnome-shell/extensions").join(UUID))
}

fn regular(path: &Path) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path).with_context(|| format!("inspecting {}", path.display()))?;
    if !metadata.is_file() { bail!("refusing non-regular file {}", path.display()); }
    Ok(metadata)
}

fn install(destination: &Path, renderer: &Path) -> Result<()> {
    if std::fs::symlink_metadata(destination).is_ok() { bail!("extension path exists; uninstall the managed copy first"); }
    if !renderer.is_absolute() { bail!("--renderer must name an absolute path: {}", renderer.display()); }
    if regular(renderer)?.len() == 0 { bail!("renderer is empty: {}", renderer.display()); }
    let parent = destination.parent().context("missing extension parent")?;
    std::fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".one-mk-install-{}-{}", std::process::id(), rand::random::<u64>()));
    std::fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        let mut hashes = serde_json::Map::new();
        for &(name, contents) in FILES {
            std::fs::write(stage.join(name), contents)?;
            hashes.insert(name.into(), json!(blake3::hash(contents.as_bytes()).to_hex().to_string()));
        }
        std::fs::create_dir(stage.join("libexec"))?;
        let copied = stage.join("libexec/one-mk-wall");
        std::fs::copy(renderer, &copied)?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&copied, std::fs::Permissions::from_mode(0o755))?;
        }
        hashes.insert("libexec/one-mk-wall".into(), json!(blake3::hash(&std::fs::read(copied)?).to_hex().to_string()));
        std::fs::write(stage.join(MARKER), serde_json::to_vec(&json!({"uuid": UUID, "version": 1, "files": hashes}))?)?;
        if std::fs::symlink_metadata(destination).is_ok() { bail!("extension appeared during install"); }
        std::fs::rename(&stage, destination)?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_dir_all(stage); }
    result
}

fn uninstall(destination: &Path) -> Result<()> {
    if !destination.try_exists()? { return Ok(()); }
    if !std::fs::symlink_metadata(destination)?.is_dir() { bail!("refusing unmanaged extension path"); }
    let marker = destination.join(MARKER);
    if regular(&marker)?.len() > 4096 { bail!("invalid install manifest"); }
    let manifest: Value = serde_json::from_slice(&std::fs::read(marker)?)?;
    if manifest["uuid"] != UUID || manifest["version"] != 1 { bail!("not a managed one-mk extension"); }
    let mut expected: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
    expected.push("libexec/one-mk-wall");
    let hashes = manifest["files"].as_object().context("invalid installed file list")?;
    if hashes.len() != expected.len() { bail!("unexpected installed files"); }
    for name in expected {
        let path = destination.join(name);
        regular(&path)?;
        if hashes.get(name).and_then(Value::as_str) != Some(blake3::hash(&std::fs::read(&path)?).to_hex().as_str()) {
            bail!("installed file modified: {}; preserve or remove it manually", path.display());
        }
    }
    if std::fs::read_dir(destination)?.count() != FILES.len() + 2 ||
        !std::fs::symlink_metadata(destination.join("libexec"))?.is_dir() ||
        std::fs::read_dir(destination.join("libexec"))?.count() != 1 {
        bail!("extension contains unmanaged files; refusing to remove them");
    }
    std::fs::remove_dir_all(destination)?;
    Ok(())
}

pub async fn command(command: &Command) -> Result<Value> {
    if !cfg!(target_os = "linux") { bail!("GNOME wallpaper extensions are Linux-only"); }
    let destination = destination()?;
    let message = match command {
        Command::Install { renderer } => {
            let renderer = renderer.as_ref().context("--renderer /absolute/path is required")?;
            install(&destination, renderer).with_context(|| format!("installing renderer {} into {}", renderer.display(), destination.display()))?;
            format!("Installed {}. Not enabled. Enable explicitly: gnome-extensions enable {UUID}\nA logout/login may be required. Playback remains unavailable until daemon-plan integration is qualified.", destination.display())
        }
        Command::Uninstall => {
            #[cfg(all(unix, not(target_os = "macos")))]
            if let Ok((_, enabled, _)) = shell_state().await && enabled {
                bail!("disable first: gnome-extensions disable {UUID}");
            }
            let existed = destination.try_exists().with_context(|| format!("checking extension {}", destination.display()))?;
            uninstall(&destination).with_context(|| format!("uninstalling extension {}", destination.display()))?;
            if existed { "Removed managed extension files; no GNOME settings changed." } else { "No managed extension is installed; nothing changed." }.into()
        }
        Command::Status => return status().await,
    };
    Ok(json!({"ok": true, "message": message}))
}

pub async fn status() -> Result<Value> {
    let destination = destination()?;
    let installed = destination.join(MARKER).is_file();
    let metadata: Value = serde_json::from_str(FILES[0].1)?;
    let mut result = json!({"installed": installed, "enabled": null, "qualified": null,
        "restart_limit_reached": null, "qualified_shell_versions": metadata["one-mk-qualified-shell-versions"],
        "path": destination, "reason": "GNOME session unavailable; runtime state unknown"});
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Ok((version, enabled, owner)) = shell_state().await {
        result["enabled"] = json!(enabled);
        result["shell_version"] = json!(version);
        result["qualified"] = json!(metadata["one-mk-qualified-shell-versions"].as_array()
            .is_some_and(|versions| versions.contains(&json!(version))));
        result["reason"] = json!(if enabled { "static: daemon presentation plan unavailable" } else { "extension not enabled" });
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            let path = PathBuf::from(runtime).join("one-mk-wallpaper/status.json");
            if regular(&path).is_ok_and(|metadata| metadata.len() <= 4096) &&
                let Ok(contents) = std::fs::read(path) && let Ok(state) = serde_json::from_slice::<Value>(&contents) &&
                state["shell_owner"] == owner {
                result["restart_limit_reached"] = state["restart_limit_reached"].clone();
                result["reason"] = state["reason"].clone();
            }
        }
    }
    result["ok"] = json!(true);
    result["message"] = json!(format!("GNOME extension: installed={}; enabled={}; qualified={}; restart-limit-reached={}. {}",
        result["installed"], result["enabled"], result["qualified"], result["restart_limit_reached"],
        if result["restart_limit_reached"] == true { "restart limit reached; static wallpaper" }
        else { result["reason"].as_str().unwrap_or("unknown") }));
    Ok(result)
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn shell_state() -> Result<(String, bool, String)> {
    let connection = zbus::connection::Builder::session()?.method_timeout(std::time::Duration::from_secs(1)).build().await?;
    let extensions = zbus::Proxy::new(&connection, "org.gnome.Shell", "/org/gnome/Shell", "org.gnome.Shell.Extensions").await?;
    let version: String = extensions.get_property("ShellVersion").await?;
    let info: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = extensions.call("GetExtensionInfo", &(UUID,)).await?;
    let enabled = info.get("state").and_then(|state| f64::try_from(state).ok()).is_some_and(|state| state == 1.0);
    let bus = zbus::fdo::DBusProxy::new(&connection).await?;
    let owner = bus.get_name_owner("org.gnome.Shell".try_into()?).await?.to_string();
    Ok((version, enabled, owner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_install_uninstall_preserves_foreign_files() {
        let home = tempfile::tempdir().unwrap();
        let renderer = home.path().join("renderer");
        std::fs::write(&renderer, "test renderer").unwrap();
        let destination = home.path().join("data/gnome-shell/extensions").join(UUID);
        install(&destination, &renderer).unwrap();
        assert!(destination.join("libexec/one-mk-wall").is_file());
        assert!(install(&destination, &renderer).is_err());
        std::fs::write(destination.join("foreign"), "keep me").unwrap();
        assert!(uninstall(&destination).is_err());
        std::fs::remove_file(destination.join("foreign")).unwrap();
        uninstall(&destination).unwrap();
        assert!(!destination.exists());
        uninstall(&destination).unwrap();
    }

    #[test]
    fn qualification_is_exact_and_default_is_off() {
        let metadata: Value = serde_json::from_str(FILES[0].1).unwrap();
        assert_eq!(metadata["one-mk-qualified-shell-versions"], json!(["50.1"]));
        let source = FILES[1].1;
        assert!(source.contains("this._authorized = false"));
        for forbidden in ["unsafe_mode", "FrameReady", "startRenderer(argv)", "damageTimes", "unlock-dialog"] {
            assert!(!source.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn lifecycle_stops_are_cooperative_and_sleep_or_holding_are_understood() {
        let source = FILES[1].1;
        // SIGTERM first (the renderer removes its worker and cgroup), SIGKILL
        // only after the grace period; never an immediate force-exit.
        for required in ["process.send_signal(15)", "TERMINATE_GRACE_MS = 2000", "process.force_exit()",
            "'prepare-for-sleep'", "this._login.preparingForSleep ? 8", "const PLAYING = 1, HOLDING = 4",
            "value > HOLDING", "renderer.serial !== this._serial"] {
            assert!(source.contains(required), "{required}");
        }
        assert!(!source.contains("process?.force_exit()"));
    }
}
