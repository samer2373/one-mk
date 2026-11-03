// SPDX-License-Identifier: MIT OR Apache-2.0
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use one_mk_wallpaper::{ExtensionAction, GnomeSetup, GnomeStage, GNOME_PLAYBACK_UNAVAILABLE};
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
    /// Enable the installed extension, without enabling video playback.
    Enable,
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

fn bundled_renderer_at(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let sibling = directory.join("one-mk-wallpaper");
    if regular(&sibling).is_ok_and(|metadata| metadata.len() > 0) { return Some(sibling); }
    // The .deb keeps optional components outside PATH, and never places the
    // extension in GNOME's system discovery directory (install is consented).
    let packaged = directory.parent()?.join("lib/one-mk/one-mk-wallpaper");
    regular(&packaged).is_ok_and(|metadata| metadata.len() > 0).then_some(packaged)
}

fn bundled_renderer() -> Result<PathBuf> {
    bundled_renderer_at(&std::env::current_exe()?).context("The bundled GNOME renderer is missing. Reinstall oneMK; nothing will be downloaded.")
}

fn qualified(version: &str) -> bool {
    serde_json::from_str::<Value>(FILES[0].1).ok().is_some_and(|metadata|
        metadata["one-mk-qualified-shell-versions"].as_array().is_some_and(|versions| versions.contains(&json!(version))))
}

struct ShellState { version: String, state: Option<u32>, path: Option<String>, user_extensions: bool }

fn setup_state(installed: bool, occupied: bool, renderer: bool, expected_path: &Path, shell: Option<&ShellState>) -> GnomeSetup {
    let (stage, message, can_install, can_enable) = match shell {
        None => (GnomeStage::Unavailable, "GNOME session unavailable. Open oneMK in your GNOME Wayland session.".into(), false, false),
        Some(shell) if !qualified(&shell.version) => (GnomeStage::Unsupported,
            format!("GNOME Shell {} has not been qualified. This extension supports GNOME Shell 50.1 only.", shell.version), false, false),
        _ if occupied && !installed => (GnomeStage::Unmanaged,
            "Another copy of the oneMK extension exists. It will not be overwritten. Manage it in GNOME Extensions first.".into(), false, false),
        Some(shell) if shell.state.is_some_and(|state| state != 99) && shell.path.as_deref() != expected_path.to_str() => (GnomeStage::Unmanaged,
            "GNOME loaded a different copy of the oneMK extension. It will not be replaced or enabled from this window.".into(), false, false),
        _ if !installed => (GnomeStage::NotInstalled,
            if renderer { "The GNOME extension is not installed. Install copies bundled files into your user extensions folder, without downloading or enabling anything." }
            else { "The GNOME extension is not installed and the bundled renderer is missing. Reinstall oneMK; nothing will be downloaded." }.into(), renderer, false),
        Some(shell) if shell.state.is_none() || shell.state == Some(99) => (GnomeStage::LogOutRequired,
            "Log out and back in once to finish. GNOME Wayland loads a newly installed extension in your next session. Then choose Enable here.".into(), false, false),
        Some(shell) if shell.state == Some(1) && shell.user_extensions => (GnomeStage::Active,
            "The GNOME extension is active. Choose a video on this page to play it. Off restores your desktop; enabling alone never changes your wallpaper.".into(), false, false),
        Some(shell) if !shell.user_extensions => (GnomeStage::Disabled,
            "GNOME has disabled user extensions. Turn them on in GNOME Extensions, then return here.".into(), false, false),
        Some(shell) if matches!(shell.state, Some(2 | 6)) => (GnomeStage::Disabled,
            "The GNOME extension is installed but disabled. Enable allows its desktop bridge to run; it does not grant video playback or peer access.".into(), false, true),
        Some(shell) => (GnomeStage::Failed, format!("GNOME extension state {} is not active. Check GNOME Extensions for errors.", shell.state.unwrap_or_default()), false, false),
    };
    GnomeSetup { stage, message, can_install, can_enable }
}

pub async fn setup() -> Result<GnomeSetup> {
    let destination = destination()?;
    let occupied = std::fs::symlink_metadata(&destination).is_ok();
    let installed = regular(&destination.join(MARKER)).is_ok();
    #[cfg(all(unix, not(target_os = "macos")))]
    let shell = observe_shell().await.ok();
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let shell: Option<ShellState> = None;
    Ok(setup_state(installed, occupied, bundled_renderer().is_ok(), &destination, shell.as_ref()))
}

pub async fn consented_action(action: ExtensionAction) -> Result<Value> {
    ensure!(cfg!(target_os = "linux") && one_mk_wallpaper::gnome_wayland(), "Open oneMK in your GNOME Wayland session to manage this extension");
    let setup = setup().await?;
    match action {
        ExtensionAction::Install => { ensure!(setup.can_install, "{}", setup.message); command(&Command::Install { renderer: None }).await }
        ExtensionAction::Enable => { ensure!(setup.can_enable, "{}", setup.message); command(&Command::Enable).await }
    }
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
        let copied = stage.join("libexec/one-mk-wallpaper");
        std::fs::copy(renderer, &copied)?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&copied, std::fs::Permissions::from_mode(0o755))?;
        }
        hashes.insert("libexec/one-mk-wallpaper".into(), json!(blake3::hash(&std::fs::read(copied)?).to_hex().to_string()));
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
    let hashes = manifest["files"].as_object().context("invalid installed file list")?;
    expected.push(if hashes.contains_key("libexec/one-mk-wall") { "libexec/one-mk-wall" } else { "libexec/one-mk-wallpaper" });
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
            let renderer = renderer.clone().map_or_else(bundled_renderer, Ok)?;
            let target = destination.clone();
            tokio::task::spawn_blocking(move || install(&target, &renderer)
                .with_context(|| format!("installing renderer {} into {}", renderer.display(), target.display()))).await??;
            format!("Installed {}. Not enabled. Log out and back in once to finish, then choose Enable in the Wallpaper page. {GNOME_PLAYBACK_UNAVAILABLE}", destination.display())
        }
        Command::Enable => {
            let setup = setup().await?;
            ensure!(setup.can_enable || setup.stage == GnomeStage::Active, "{}", setup.message);
            #[cfg(all(unix, not(target_os = "macos")))]
            if setup.stage != GnomeStage::Active {
                let connection = zbus::connection::Builder::session()?.method_timeout(std::time::Duration::from_secs(1)).build().await?;
                let extensions = zbus::Proxy::new(&connection, "org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions").await?;
                let enabled: bool = extensions.call("EnableExtension", &(UUID,)).await?;
                ensure!(enabled, "GNOME could not enable the extension. Log out and back in once, then try again.");
            }
            "GNOME accepted the enable request. When the extension is active, choose a video on the Wallpaper page to start playback.".into()
        }
        Command::Uninstall => {
            #[cfg(all(unix, not(target_os = "macos")))]
            if let Ok((_, enabled)) = shell_state().await && enabled {
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
    if let Ok((version, enabled)) = shell_state().await {
        result["enabled"] = json!(enabled);
        result["shell_version"] = json!(version);
        result["qualified"] = json!(metadata["one-mk-qualified-shell-versions"].as_array()
            .is_some_and(|versions| versions.contains(&json!(version))));
        result["reason"] = json!(if enabled { "ready for a video selected on the Wallpaper page" } else { "extension not enabled" });
    }
    result["ok"] = json!(true);
    result["setup"] = serde_json::to_value(setup().await?)?;
    result["message"] = json!(format!("GNOME extension: installed={}; enabled={}; qualified={}; restart-limit-reached={}. {}",
        result["installed"], result["enabled"], result["qualified"], result["restart_limit_reached"],
        if result["restart_limit_reached"] == true { "restart limit reached; static wallpaper" }
        else { result["reason"].as_str().unwrap_or("unknown") }));
    Ok(result)
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn shell_state() -> Result<(String, bool)> {
    let state = observe_shell().await?;
    Ok((state.version, state.state == Some(1) && state.user_extensions))
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn observe_shell() -> Result<ShellState> {
    let connection = zbus::connection::Builder::session()?.method_timeout(std::time::Duration::from_secs(1)).build().await?;
    let extensions = zbus::Proxy::new(&connection, "org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions").await?;
    let version: String = extensions.get_property("ShellVersion").await?;
    let info: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = extensions.call("GetExtensionInfo", &(UUID,)).await?;
    let state = info.get("state").and_then(|state| f64::try_from(state).ok())
        .filter(|value| value.is_finite() && value.fract() == 0.0 && (0.0..=99.0).contains(value)).map(|value| value as u32);
    let path = info.get("path").and_then(|value| <&str>::try_from(value).ok()).map(str::to_owned);
    let user_extensions: bool = extensions.get_property("UserExtensionsEnabled").await?;
    Ok(ShellState { version, state, path, user_extensions })
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
        assert!(destination.join("libexec/one-mk-wallpaper").is_file());
        assert!(install(&destination, &renderer).is_err());
        std::fs::write(destination.join("foreign"), "keep me").unwrap();
        assert!(uninstall(&destination).is_err());
        std::fs::remove_file(destination.join("foreign")).unwrap();
        uninstall(&destination).unwrap();
        assert!(!destination.exists());
        uninstall(&destination).unwrap();
    }

    #[test]
    fn setup_distinguishes_install_logout_enable_and_active_without_authorizing_playback() {
        let path = Path::new("/private/extensions/one-mk-wallpaper@one-mk.org");
        let mut shell = ShellState { version: "50.1".into(), state: None, path: None, user_extensions: true };
        let check = |installed, occupied, renderer, shell: &ShellState, expected, install, enable| {
            let setup = setup_state(installed, occupied, renderer, path, Some(shell));
            assert_eq!((setup.stage, setup.can_install, setup.can_enable), (expected, install, enable));
            assert!(!setup.message.contains("not connected to the production service"));
            setup
        };
        check(false, false, true, &shell, GnomeStage::NotInstalled, true, false);
        check(false, false, false, &shell, GnomeStage::NotInstalled, false, false);
        check(false, true, true, &shell, GnomeStage::Unmanaged, false, false);
        let setup = check(true, true, true, &shell, GnomeStage::LogOutRequired, false, false);
        assert!(setup.message.contains("Log out and back in once to finish"));
        shell.state = Some(99);
        check(true, true, true, &shell, GnomeStage::LogOutRequired, false, false);
        shell.state = Some(2);
        shell.path = Some("/another/copy".into());
        check(true, true, true, &shell, GnomeStage::Unmanaged, false, false);
        check(false, false, true, &shell, GnomeStage::Unmanaged, false, false);
        shell.path = Some(path.to_string_lossy().into_owned());
        check(true, true, true, &shell, GnomeStage::Disabled, false, true);
        shell.state = Some(1);
        check(true, true, true, &shell, GnomeStage::Active, false, false);
        shell.user_extensions = false;
        check(true, true, true, &shell, GnomeStage::Disabled, false, false);
        shell.user_extensions = true;
        for state in [3, 4, 5, 7, 8] {
            shell.state = Some(state);
            check(true, true, true, &shell, GnomeStage::Failed, false, false);
        }
        shell.version = "50.2".into();
        check(true, true, true, &shell, GnomeStage::Unsupported, false, false);
        let absent = setup_state(true, true, true, path, None);
        assert_eq!(absent.stage, GnomeStage::Unavailable);
        assert!(!absent.can_enable && !absent.can_install);
    }

    #[test]
    fn renderer_discovery_supports_unpacked_deb_and_tar_without_path_search() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("usr/bin/one-mk");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        assert!(bundled_renderer_at(&binary).is_none());
        let packaged = root.path().join("usr/lib/one-mk/one-mk-wallpaper");
        std::fs::create_dir_all(packaged.parent().unwrap()).unwrap();
        std::fs::write(&packaged, "renderer").unwrap();
        assert_eq!(bundled_renderer_at(&binary), Some(packaged));
        let sibling = binary.with_file_name("one-mk-wallpaper");
        std::fs::write(&sibling, "renderer").unwrap();
        assert_eq!(bundled_renderer_at(&binary), Some(sibling));
    }

    #[test]
    fn modified_managed_files_are_never_removed_or_replaced() {
        let root = tempfile::tempdir().unwrap();
        let renderer = root.path().join("renderer");
        std::fs::write(&renderer, "renderer").unwrap();
        let destination = root.path().join(UUID);
        install(&destination, &renderer).unwrap();
        std::fs::write(destination.join("extension.js"), "user changes").unwrap();
        assert!(uninstall(&destination).is_err());
        assert!(install(&destination, &renderer).is_err());
        assert_eq!(std::fs::read_to_string(destination.join("extension.js")).unwrap(), "user changes");
    }

    #[test]
    fn qualification_is_exact_and_default_is_off() {
        let metadata: Value = serde_json::from_str(FILES[0].1).unwrap();
        assert_eq!(metadata["one-mk-qualified-shell-versions"], json!(["50.1"]));
        let source = FILES[1].1;
        assert!(source.contains("this._client = null"));
        assert!(source.contains("window.get_pid() !== client.pid"));
        assert!(source.contains("uid !== new Gio.Credentials().get_unix_user()"));
        assert!(source.contains("Config.PACKAGE_VERSION !== '50.1'"));
        for forbidden in ["unsafe_mode", "FrameReady", "startRenderer(argv)", "damageTimes", "unlock-dialog"] {
            assert!(!source.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn shell_only_presents_windows_and_cleans_up_on_sender_loss_or_disable() {
        let source = FILES[1].1;
        for required in ["'NameOwnerChanged'", "name === this._client?.sender && !owner", "disable() {",
            "this._stop();", "entry.box?.destroy()", "this._object.unexport()"] {
            assert!(source.contains(required), "{required}");
        }
        for forbidden in ["Gio.Subprocess", "force_exit", "send_signal", "gsettings", "DConf", "Gst."] {
            assert!(!source.contains(forbidden), "{forbidden}");
        }
    }
}
