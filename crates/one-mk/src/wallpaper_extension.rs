// SPDX-License-Identifier: MIT OR Apache-2.0
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use one_mk_wallpaper::{ExtensionAction, GnomeSetup, GnomeStage};
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
    /// Copy oneMK's GNOME extension into your extensions folder (nothing is
    /// downloaded) and switch it on for your next login; then log out and
    /// back in once. Also updates an older copy oneMK installed.
    Install {
        /// Use this wallpaper helper (an absolute path) instead of the one
        /// installed with oneMK.
        #[arg(long, value_name = "PATH")] renderer: Option<PathBuf>,
    },
    /// Switch the installed extension on (at your next login if GNOME has
    /// not loaded it yet). It never starts a video by itself.
    Enable,
    /// Switch the extension off and remove oneMK's copy of it (a copy you
    /// changed is left alone).
    Uninstall,
    /// Show whether the extension is installed, switched on and supported
    /// by this GNOME version.
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

fn qualified(version: &str) -> bool { one_mk_wallpaper::supported_gnome_shell(version) }

/// Whether the managed copy at `destination` differs from the extension this
/// oneMK carries (installed by an older version): Install then updates it.
fn outdated(destination: &Path) -> bool {
    FILES.iter().filter(|(name, _)| !name.starts_with("LICENSE")).any(|(name, contents)|
        std::fs::read(destination.join(name)).is_ok_and(|installed| installed != contents.as_bytes()))
}

struct ShellState { version: String, state: Option<u32>, path: Option<String>, user_extensions: bool, listed: bool }

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
/// GNOME's list of switched-on extensions is GVariant text: `['a', 'b']` or
/// `@as []`. Keeps the other entries exactly as they are.
fn list_entries(text: &str) -> Vec<String> {
    text.trim().trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']')
        .split(',').map(|entry| entry.trim().trim_matches('\'').to_owned()).filter(|entry| !entry.is_empty()).collect()
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn list_text(entries: &[String]) -> String {
    format!("[{}]", entries.iter().map(|entry| format!("'{entry}'")).collect::<Vec<_>>().join(", "))
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
/// The list with `UUID` switched on, or `None` if it already is.
fn list_with(text: &str) -> Option<String> {
    let mut entries = list_entries(text);
    if entries.iter().any(|entry| entry == UUID) { return None; }
    entries.push(UUID.to_owned());
    Some(list_text(&entries))
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
/// The list without `UUID`, or `None` if it isn't there.
fn list_without(text: &str) -> Option<String> {
    let entries = list_entries(text);
    if !entries.iter().any(|entry| entry == UUID) { return None; }
    Some(list_text(&entries.into_iter().filter(|entry| entry != UUID).collect::<Vec<_>>()))
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn gsettings(arguments: &[&str]) -> Result<String> {
    let output = tokio::time::timeout(std::time::Duration::from_secs(3),
        tokio::process::Command::new("gsettings").args(arguments).kill_on_drop(true).output()).await
        .context("GNOME settings took too long")?.context("GNOME settings could not start")?;
    ensure!(output.status.success(), "GNOME settings said: {}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Switches the extension on for the next login. GNOME refuses a live
/// enable of an extension it hasn't seen yet, but loads one that is already on
/// the list when the session starts, so a person only logs out and in.
#[cfg(all(unix, not(target_os = "macos")))]
async fn switch_on_for_next_login() -> Result<()> {
    let current = gsettings(&["get", "org.gnome.shell", "enabled-extensions"]).await?;
    if let Some(next) = list_with(&current) { gsettings(&["set", "org.gnome.shell", "enabled-extensions", &next]).await?; }
    Ok(())
}
#[cfg(not(all(unix, not(target_os = "macos"))))]
async fn switch_on_for_next_login() -> Result<()> { bail!("GNOME wallpaper extensions are Linux-only") }

#[cfg(all(unix, not(target_os = "macos")))]
async fn forget_from_login_list() -> Result<()> {
    let current = gsettings(&["get", "org.gnome.shell", "enabled-extensions"]).await?;
    if let Some(next) = list_without(&current) { gsettings(&["set", "org.gnome.shell", "enabled-extensions", &next]).await?; }
    Ok(())
}
#[cfg(not(all(unix, not(target_os = "macos"))))]
async fn forget_from_login_list() -> Result<()> { Ok(()) }

fn setup_state(installed: bool, occupied: bool, renderer: bool, old: bool, expected_path: &Path, shell: Option<&ShellState>) -> GnomeSetup {
    let (stage, message, can_install, can_enable) = match shell {
        None => (GnomeStage::Unavailable, "GNOME session unavailable. Open oneMK in your GNOME Wayland session.".into(), false, false),
        Some(shell) if !qualified(&shell.version) => (GnomeStage::Unsupported, one_mk_wallpaper::unsupported_gnome_shell(&shell.version), false, false),
        _ if occupied && !installed => (GnomeStage::Unmanaged,
            "Another copy of the oneMK extension exists. It will not be overwritten. Manage it in GNOME Extensions first.".into(), false, false),
        Some(shell) if shell.state.is_some_and(|state| state != 99) && shell.path.as_deref() != expected_path.to_str() => (GnomeStage::Unmanaged,
            "GNOME loaded a different copy of the oneMK extension. It will not be replaced or enabled from this window.".into(), false, false),
        _ if !installed => (GnomeStage::NotInstalled,
            if renderer { "GNOME needs a small oneMK extension to play video behind your windows. Install copies it into your user extensions folder and switches it on for your next login; nothing is downloaded. Then log out and back in once." }
            else { "The GNOME extension is not installed and the bundled renderer is missing. Reinstall oneMK; nothing will be downloaded." }.into(), renderer, false),
        _ if old => (GnomeStage::NotInstalled,
            if renderer { "An older copy of oneMK's GNOME extension is installed. Choose Install GNOME extension (or run `one-mk wallpaper extension install`) to update it, then log out and back in once." }
            else { "An older copy of oneMK's GNOME extension is installed and the bundled renderer is missing. Reinstall oneMK; nothing will be downloaded." }.into(), renderer, false),
        Some(shell) if shell.state.is_none() || shell.state == Some(99) => if shell.listed {
            (GnomeStage::LogOutRequired, "One step left: log out and back in once. GNOME loads the extension at login, and it turns itself on. Then choose a video here.".into(), false, false)
        } else {
            (GnomeStage::LogOutRequired, "The extension is installed but not switched on for your next login. Choose Switch on, then log out and back in once.".into(), false, true)
        },
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
    Ok(setup_state(installed, occupied, bundled_renderer().is_ok(), installed && outdated(&destination), &destination, shell.as_ref()))
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
            tokio::task::spawn_blocking(move || {
                // An older copy oneMK installed (and nobody changed) is replaced.
                if regular(&target.join(MARKER)).is_ok() && outdated(&target) { uninstall(&target)?; }
                install(&target, &renderer)
            }
                .with_context(|| format!("installing renderer {} into {}", renderer.display(), target.display()))).await??;
            match switch_on_for_next_login().await {
                Ok(()) => format!("Installed in {}. One step left: log out and back in once. The extension turns itself on, and video can start after that.", destination.display()),
                Err(error) => format!("Installed in {}, but GNOME could not switch it on for your next login ({error:#}). Log out and back in once, then choose Switch on here.", destination.display()),
            }
        }
        Command::Enable => {
            let setup = setup().await?;
            ensure!(setup.can_enable || setup.stage == GnomeStage::Active, "{}", setup.message);
            if setup.stage == GnomeStage::LogOutRequired {
                switch_on_for_next_login().await?;
                return Ok(json!({"ok": true, "message": "Switched on for your next login. Log out and back in once; the extension then turns itself on."}));
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            if setup.stage != GnomeStage::Active {
                let connection = session_bus()?.method_timeout(std::time::Duration::from_secs(1)).build().await?;
                let extensions = zbus::Proxy::new(&connection, "org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions").await?;
                let enabled: bool = extensions.call("EnableExtension", &(UUID,)).await?;
                ensure!(enabled, "GNOME could not enable the extension. Log out and back in once, then try again.");
            }
            "GNOME accepted the enable request. When the extension is active, choose a video on the Wallpaper page to start playback.".into()
        }
        Command::Uninstall => remove().await?,
        Command::Status => return status().await,
    };
    Ok(json!({"ok": true, "message": message}))
}

/// Switches the extension off (now, if GNOME runs it, and at the next login)
/// and removes oneMK's copy; used by `extension uninstall` and by `one-mk
/// uninstall`. A copy oneMK did not install, or one that was changed, is
/// left where it is and the error says so.
pub async fn remove() -> Result<String> {
    if !cfg!(target_os = "linux") { bail!("GNOME wallpaper extensions are Linux-only"); }
    let destination = destination()?;
    let existed = destination.try_exists().with_context(|| format!("checking extension {}", destination.display()))?;
    if !existed {
        let _ = forget_from_login_list().await;
        return Ok("oneMK's GNOME extension is not installed; nothing changed.".into());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    disable_now().await;
    let _ = forget_from_login_list().await;
    uninstall(&destination).with_context(|| format!("Could not remove the GNOME extension in {}", destination.display()))?;
    Ok("Switched off and removed oneMK's GNOME extension.".into())
}

/// The session bus this process was started in, and only that one. Without
/// `DBUS_SESSION_BUS_ADDRESS` or `XDG_RUNTIME_DIR` zbus would fall back to
/// `/run/user/<uid>/bus`: the desktop session of whoever owns the uid, which
/// a process started outside that session (a test, a service, a script with
/// a cleared environment) must never change.
#[cfg(all(unix, not(target_os = "macos")))]
fn session_bus() -> Result<zbus::connection::Builder<'static>> {
    ensure!(session_bus_named(std::env::var_os("DBUS_SESSION_BUS_ADDRESS"), std::env::var_os("XDG_RUNTIME_DIR")), "This is not running inside a desktop session");
    Ok(zbus::connection::Builder::session()?)
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn session_bus_named(address: Option<std::ffi::OsString>, runtime: Option<std::ffi::OsString>) -> bool {
    address.is_some_and(|value| !value.is_empty()) || runtime.is_some_and(|value| !value.is_empty())
}

/// Asks a running GNOME Shell to switch the extension off; outside a GNOME
/// session there is nothing running to switch off.
#[cfg(all(unix, not(target_os = "macos")))]
async fn disable_now() {
    let Ok(builder) = session_bus() else { return };
    let Ok(Ok(connection)) = tokio::time::timeout(std::time::Duration::from_secs(2), builder.method_timeout(std::time::Duration::from_secs(2)).build()).await else { return };
    let Ok(extensions) = zbus::Proxy::new(&connection, "org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions").await else { return };
    let _ = extensions.call::<_, _, bool>("DisableExtension", &(UUID,)).await;
}

pub async fn status() -> Result<Value> {
    let destination = destination()?;
    let installed = destination.join(MARKER).is_file();
    let metadata: Value = serde_json::from_str(FILES[0].1)?;
    let mut result = json!({"installed": installed, "outdated": installed && outdated(&destination), "enabled": null, "qualified": null,
        "restart_limit_reached": null, "qualified_shell_versions": metadata["one-mk-qualified-shell-versions"],
        "path": destination, "reason": "GNOME session unavailable; runtime state unknown"});
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Ok((version, enabled)) = shell_state().await {
        result["enabled"] = json!(enabled);
        result["shell_version"] = json!(version);
        result["qualified"] = json!(qualified(&version));
        result["reason"] = json!(if enabled { "ready for a video selected on the Wallpaper page" } else { "extension not enabled" });
    }
    result["ok"] = json!(true);
    result["setup"] = serde_json::to_value(setup().await?)?;
    // What to do next, in words, then the facts behind it.
    result["message"] = json!(format!("{}\nInstalled: {}. Switched on: {}. Supported GNOME version: {}.",
        result["setup"]["message"].as_str().unwrap_or("unknown"), yes_no(&result["installed"]), yes_no(&result["enabled"]), yes_no(&result["qualified"])));
    Ok(result)
}

fn yes_no(value: &Value) -> &'static str {
    match value.as_bool() { Some(true) => "yes", Some(false) => "no", None => "unknown (not in a GNOME session)" }
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn shell_state() -> Result<(String, bool)> {
    let state = observe_shell().await?;
    Ok((state.version, state.state == Some(1) && state.user_extensions))
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn observe_shell() -> Result<ShellState> {
    let connection = session_bus()?.method_timeout(std::time::Duration::from_secs(1)).build().await?;
    let extensions = zbus::Proxy::new(&connection, "org.gnome.Shell.Extensions", "/org/gnome/Shell/Extensions", "org.gnome.Shell.Extensions").await?;
    let version: String = extensions.get_property("ShellVersion").await?;
    let info: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = extensions.call("GetExtensionInfo", &(UUID,)).await?;
    let state = info.get("state").and_then(|state| f64::try_from(state).ok())
        .filter(|value| value.is_finite() && value.fract() == 0.0 && (0.0..=99.0).contains(value)).map(|value| value as u32);
    let path = info.get("path").and_then(|value| <&str>::try_from(value).ok()).map(str::to_owned);
    let user_extensions: bool = extensions.get_property("UserExtensionsEnabled").await?;
    let listed = gsettings(&["get", "org.gnome.shell", "enabled-extensions"]).await.is_ok_and(|text| list_entries(&text).iter().any(|entry| entry == UUID));
    Ok(ShellState { version, state, path, user_extensions, listed })
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
        let mut shell = ShellState { version: "50.1".into(), state: None, path: None, user_extensions: true, listed: true };
        let check = |installed, occupied, renderer, shell: &ShellState, expected, install, enable| {
            let setup = setup_state(installed, occupied, renderer, false, path, Some(shell));
            assert_eq!((setup.stage, setup.can_install, setup.can_enable), (expected, install, enable));
            assert!(!setup.message.contains("not connected to the production service"));
            setup
        };
        check(false, false, true, &shell, GnomeStage::NotInstalled, true, false);
        check(false, false, false, &shell, GnomeStage::NotInstalled, false, false);
        check(false, true, true, &shell, GnomeStage::Unmanaged, false, false);
        let setup = check(true, true, true, &shell, GnomeStage::LogOutRequired, false, false);
        assert!(setup.message.contains("log out and back in once") && setup.message.contains("turns itself on"));
        // Installed but not on GNOME's login list: one click puts it there.
        shell.listed = false;
        let setup = check(true, true, true, &shell, GnomeStage::LogOutRequired, false, true);
        assert!(setup.message.contains("Switch on"));
        shell.listed = true;
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
        // The whole GNOME 50 series is supported; other series say so plainly.
        shell.state = Some(1);
        shell.version = "50.2".into();
        check(true, true, true, &shell, GnomeStage::Active, false, false);
        for version in ["49.4", "51.0"] {
            shell.version = version.into();
            let setup = check(true, true, true, &shell, GnomeStage::Unsupported, false, false);
            assert!(setup.message.contains(&format!("GNOME Shell {version} is not supported")) && setup.message.contains("GNOME 50"), "{}", setup.message);
        }
        // An older copy oneMK installed: Install updates it.
        shell.version = "50.1".into();
        let old = setup_state(true, true, true, true, path, Some(&shell));
        assert_eq!((old.stage, old.can_install), (GnomeStage::NotInstalled, true));
        assert!(old.message.contains("older copy") && old.message.contains("one-mk wallpaper extension install"));
        let absent = setup_state(true, true, true, false, path, None);
        assert_eq!(absent.stage, GnomeStage::Unavailable);
        assert!(!absent.can_enable && !absent.can_install);
    }

    #[test]
    fn a_process_outside_a_desktop_session_never_reaches_one() {
        assert!(!session_bus_named(None, None));
        assert!(!session_bus_named(Some("".into()), Some("".into())));
        assert!(session_bus_named(Some("unix:path=/run/user/1000/bus".into()), None));
        assert!(session_bus_named(None, Some("/run/user/1000".into())));
    }

    #[test]
    fn login_list_keeps_other_extensions_and_adds_or_removes_only_ours() {
        assert_eq!(list_with("@as []").unwrap(), format!("['{UUID}']"));
        assert_eq!(list_with("['ding@rastersoft.com', 'ubuntu-dock@ubuntu.com']").unwrap(), format!("['ding@rastersoft.com', 'ubuntu-dock@ubuntu.com', '{UUID}']"));
        assert!(list_with(&format!("['{UUID}']")).is_none());
        assert_eq!(list_without(&format!("['a@b', '{UUID}']")).unwrap(), "['a@b']");
        assert_eq!(list_without(&format!("['{UUID}']")).unwrap(), "[]");
        assert!(list_without("['a@b']").is_none());
        assert!(list_entries("@as []").is_empty());
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
    fn an_older_managed_copy_is_detected_and_replaced_when_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let renderer = root.path().join("renderer");
        std::fs::write(&renderer, "renderer").unwrap();
        let destination = root.path().join(UUID);
        install(&destination, &renderer).unwrap();
        assert!(!outdated(&destination));
        // As 0.1.28 installed it: same layout, older extension code.
        let manifest_path = destination.join(MARKER);
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        let older = FILES[1].1.replace("split('.')[0] !== '50'", "!== '50.1'");
        std::fs::write(destination.join("extension.js"), &older).unwrap();
        manifest["files"]["extension.js"] = json!(blake3::hash(older.as_bytes()).to_hex().to_string());
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(outdated(&destination));
        uninstall(&destination).unwrap();
        install(&destination, &renderer).unwrap();
        assert_eq!(std::fs::read_to_string(destination.join("extension.js")).unwrap(), FILES[1].1);
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
        assert_eq!(metadata["one-mk-qualified-shell-versions"], json!(["50"]));
        assert_eq!(metadata["shell-version"], json!(["50"]));
        let source = FILES[1].1;
        assert!(source.contains("this._client = null"));
        assert!(source.contains("window.get_pid() !== client.pid"));
        assert!(source.contains("uid !== new Gio.Credentials().get_unix_user()"));
        assert!(source.contains("Config.PACKAGE_VERSION.split('.')[0] !== '50'"));
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
