use super::*;

pub(super) fn bundle(executable: &Path) -> Result<PathBuf> {
    executable.ancestors().find(|path| path.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("app")))
        .map(Path::to_path_buf).context("Couldn't find the running app")
}

fn backup(target: &Path) -> Result<PathBuf> {
    let name = target.file_stem().context("The app has no name")?.to_string_lossy();
    Ok(target.with_file_name(format!(".{name}.previous.app")))
}

fn receipt(target: &Path) -> PathBuf { target.with_extension("update-pending") }

trait CodeSign {
    fn team(&self, path: &Path) -> Result<String>;
}

struct SystemCodeSign;

/// codesign reads a requirement that doesn't start with `=` as the path of a file holding one, so the text
/// needs the `=` ("--test-requirement==anchor apple generic" on the command line). Without it the check
/// always failed and no app update could ever be installed.
const VERIFY_ARGS: [&str; 4] = ["--verify", "--deep", "--strict", "--test-requirement==anchor apple generic"];

impl CodeSign for SystemCodeSign {
    fn team(&self, path: &Path) -> Result<String> {
        // Signed through Apple's chain (a Developer ID), not just any
        // signature with a team string in it: only then is the team the
        // publisher's.
        let verified = Command::new("/usr/bin/codesign").args(VERIFY_ARGS)
            .arg(path).output().context("Couldn't verify the app's identity")?;
        if !verified.status.success() {
            let reason = String::from_utf8_lossy(&verified.stderr).lines().next().unwrap_or("").trim().to_owned();
            tracing::warn!("update: the security check failed for {}: {reason}", path.display());
            bail!("The app's security check failed ({}); nothing was replaced", path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
        }
        let identity = Command::new("/usr/bin/codesign").args(["--display", "--verbose=4"])
            .arg(path).output().context("Couldn't read the app's identity")?;
        ensure!(identity.status.success(), "Couldn't read the app's identity");
        let text = String::from_utf8_lossy(&identity.stderr);
        let team = text.lines().find_map(|line| line.strip_prefix("TeamIdentifier="))
            .filter(|team| team.len() == 10 && team.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .context("This app has no verified publisher identity; download the signed app instead")?;
        Ok(team.to_owned())
    }
}

fn matching_team(runner: &impl CodeSign, running: &Path, candidate: &Path) -> Result<()> {
    let expected = runner.team(running)?;
    let offered = runner.team(candidate)?;
    ensure!(!expected.is_empty() && expected == offered, "The new app is from a different publisher; nothing was replaced");
    Ok(())
}

trait BundleFiles {
    fn exists(&self, path: &Path) -> Result<bool>;
    fn rename(&self, source: &Path, destination: &Path) -> Result<()>;
    /// Swaps two folders in one step: there is never a moment with neither.
    fn exchange(&self, a: &Path, b: &Path) -> Result<()>;
}

struct Disk;

impl BundleFiles for Disk {
    fn exists(&self, path: &Path) -> Result<bool> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => { ensure!(metadata.file_type().is_dir(), "The app or backup is not a safe folder"); Ok(true) }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    fn rename(&self, source: &Path, destination: &Path) -> Result<()> { rename_new(source, destination) }
    fn exchange(&self, a: &Path, b: &Path) -> Result<()> { exchange(a, b) }
}

#[cfg(target_os = "macos")]
fn renamex(from: &Path, to: &Path, flags: libc::c_uint) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let (from, to) = (std::ffi::CString::new(from.as_os_str().as_bytes())?, std::ffi::CString::new(to.as_os_str().as_bytes())?);
    // SAFETY: two valid NUL-terminated paths; renamex_np only reads them.
    let r = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), flags) };
    if r != 0 { return Err(std::io::Error::last_os_error().into()); }
    Ok(())
}

/// Renames, refusing to replace anything already at `destination` (a plain
/// rename would silently replace an empty folder made there meanwhile).
fn rename_new(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    return renamex(source, destination, libc::RENAME_EXCL);
    #[cfg(not(target_os = "macos"))]
    {
        ensure!(!destination.try_exists()?, "{} already exists", destination.display());
        Ok(fs::rename(source, destination)?)
    }
}

fn exchange(a: &Path, b: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    return renamex(a, b, libc::RENAME_SWAP);
    // Elsewhere only the tests run this: three renames will do.
    #[cfg(not(target_os = "macos"))]
    {
        let side = a.with_extension("exchange");
        fs::rename(a, &side)?;
        fs::rename(b, a)?;
        Ok(fs::rename(side, b)?)
    }
}

/// Puts `candidate` where `target` is and keeps the old one at `previous`.
/// The swap itself is one step, so an interruption never leaves no app; a
/// stop between it and the second step leaves the old app in the staging
/// folder (only going back is lost, never the app).
fn swap(files: &impl BundleFiles, candidate: &Path, target: &Path, previous: &Path) -> Result<()> {
    ensure!(files.exists(target)? && files.exists(candidate)?, "The app or its update is missing");
    ensure!(!files.exists(previous)?, "The previous app is still waiting for a successful restart");
    files.exchange(candidate, target).context("This app's folder can't be changed by your account. Choose Download instead")?;
    if let Err(error) = files.rename(candidate, previous) {
        files.exchange(candidate, target).context("Couldn't restore the previous app; it is saved beside the installation")?;
        return Err(error).context("Couldn't install the update; the previous app was restored");
    }
    Ok(())
}

/// Staging folders left by an update that was stopped (a crash, a forced
/// quit): with the lock held no other update is running, so they are
/// leftovers.
fn clean_staging(parent: &Path) {
    let Ok(entries) = fs::read_dir(parent) else { return };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with(".one-mk-bundle-") && e.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

struct Staging(PathBuf);
impl Staging {
    fn new(parent: &Path) -> Result<Self> {
        for _ in 0..100 {
            let path = parent.join(format!(".one-mk-bundle-{}-{}", std::process::id(), TEMP_ID.fetch_add(1, Ordering::Relaxed)));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("This app's folder can't be changed by your account. Choose Download instead"),
            }
        }
        bail!("Couldn't prepare a folder for the update")
    }
}
impl Drop for Staging { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

fn stop_daemon(home: &Path) -> Result<()> {
    let stopped = Command::new(std::env::current_exe()?).arg("stop").arg("--home").arg(home)
        .stdin(Stdio::null()).output().context("Couldn't stop sharing before the update")?;
    ensure!(stopped.status.success(), "Couldn't stop sharing before the update; nothing was replaced");
    Ok(())
}

/// Stops sharing, then swaps `candidate` in. The version before the running
/// one, kept since the last update for "Go back", is moved to `older` (inside
/// the staging folder, deleted once the update is done) to make room for the
/// running version: when stopping or swapping fails nothing was updated, so it
/// goes back to `previous` instead of being deleted with the staging folder.
fn install(files: &impl BundleFiles, stop: impl FnOnce() -> Result<()>, candidate: &Path, target: &Path, previous: &Path, older: &Path) -> Result<()> {
    let had_previous = files.exists(previous)?;
    if had_previous { files.rename(previous, older)?; }
    let result = stop().and_then(|()| swap(files, candidate, target, previous));
    if result.is_err() && had_previous && !files.exists(previous).unwrap_or(true) {
        if let Err(error) = files.rename(older, previous) {
            tracing::warn!("update: couldn't put the previous version back for Go back: {error:#}");
        }
    }
    result
}

/// A receipt whose version is not the installed app's belongs to an update
/// that stopped before its swap (killed, power loss, the window closed): the
/// app is still the old one, so the receipt guards nothing and would block
/// every later update. One whose version is the installed app's is a real
/// swap waiting for the new app to start, and stays.
fn clear_stale_receipt(target: &Path) -> Result<()> {
    let marker = receipt(target);
    if !marker.try_exists()? { return Ok(()); }
    regular(&marker)?;
    let pending = fs::read_to_string(&marker)?;
    let installed = target.join("Contents/MacOS/one-mk");
    // Only a readable, different version proves the swap never happened.
    if version_check(&installed, None).is_ok() && version_check(&installed, Some(pending.trim())).is_err() {
        fs::remove_file(&marker)?;
    }
    Ok(())
}

pub(super) fn apply_bundle(manifest: &Manifest, release: &Release, origin: &Url, executable: &Path, home: &Path) -> Result<()> {
    let target = bundle(executable)?;
    ensure!(!policy::translocated(&target.to_string_lossy()), "Move oneMK to Applications before updating");
    let _lock = target_lock(&target, "This app's folder can't be changed by your account. Choose Download instead")?;
    clean_staging(target.parent().context("The app has no parent folder")?);
    let previous = backup(&target)?;
    clear_stale_receipt(&target)?;
    ensure!(!receipt(&target).try_exists()?, "The previous app is still waiting for a successful restart. Open oneMK once, then update");
    ensure!(Disk.exists(&target)?, "The running app is missing");
    let staging = Staging::new(target.parent().context("The app has no parent folder")?)?;
    let asset = policy::select_install(manifest, "macos", std::env::consts::ARCH, InstallKind::MacApp)?;
    let payload = download(asset, release, origin, &staging.0)?;
    println!("Checking the new app…");
    let extracted = Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&payload.path).arg(&staging.0)
        .output().context("Couldn't unpack the app update")?;
    ensure!(extracted.status.success(), "Couldn't unpack the app update; nothing was replaced");
    let candidate = staging.0.join("one-mk.app");
    ensure!(Disk.exists(&candidate)?, "The update does not contain the app");
    matching_team(&SystemCodeSign, &target, &candidate)?;
    let candidate_executable = candidate.join("Contents/MacOS/one-mk");
    regular(&candidate_executable)?;
    version_check(&candidate_executable, Some(&manifest.version))?;
    println!("Installing…");
    let marker = receipt(&target);
    let mut pending = OpenOptions::new().create_new(true).write(true).open(&marker)?;
    pending.write_all(manifest.version.as_bytes())?;
    pending.sync_all()?;
    if let Err(error) = install(&Disk, || stop_daemon(home), &candidate, &target, &previous, &staging.0.join("older.app")) {
        if Disk.exists(&target).unwrap_or(false) { let _ = fs::remove_file(&marker); }
        return Err(error);
    }
    sync_directory(target.parent().context("The app has no parent folder")?)?;
    restarting(&format!("oneMK {} installed", manifest.version), "Open oneMK to start sharing again");
    Ok(())
}

pub(super) fn acknowledge(executable: &Path, started_version: &str) -> Result<()> {
    ensure!(started_version == env!("CARGO_PKG_VERSION"), "The previous window is still running");
    let target = bundle(executable)?;
    let marker = receipt(&target);
    if !marker.try_exists()? { return Ok(()); }
    let _lock = target_lock(&target, "This app's folder can't be changed by your account")?;
    regular(&marker)?;
    let version = fs::read_to_string(&marker)?;
    ensure!(version == started_version, "The previous app is kept until the new window starts");
    // The previous app stays for "Go back to the previous version" until
    // the next update replaces it.
    fs::remove_file(marker)?;
    // An update's staging folder (with the oldest app in it) can outlive the
    // update when something in it was in use: the new window starting is the
    // moment it is certain nobody needs it, and the lock is held.
    if let Some(parent) = target.parent() { clean_staging(parent); }
    Ok(())
}

pub(super) fn rollback_available(executable: &Path) -> bool {
    bundle(executable).is_ok_and(|target| backup(&target).is_ok_and(|previous| previous.is_dir()))
}

pub(super) fn rollback_bundle(executable: &Path, home: &Path) -> Result<()> {
    let target = bundle(executable)?;
    let _lock = target_lock(&target, "This app's folder can't be changed by your account")?;
    clean_staging(target.parent().context("The app has no parent folder")?);
    let previous = backup(&target)?;
    matching_team(&SystemCodeSign, &target, &previous)?;
    let staging = Staging::new(target.parent().context("The app has no parent folder")?)?;
    stop_daemon(home)?;
    swap(&Disk, &previous, &target, &staging.0.join("failed.app"))?;
    let _ = fs::remove_file(receipt(&target));
    restarting("Previous app restored", "Open oneMK to start sharing again");
    Ok(())
}

pub(super) fn move_to_applications(executable: &Path) -> Result<()> {
    let source = bundle(executable)?;
    let target = Path::new("/Applications/one-mk.app");
    ensure!(policy::translocated(&source.to_string_lossy()), "The app is already installed outside the disk image");
    let _lock = target_lock(target, "Your account can't move this app to Applications. Drag it there in Finder")?;
    clean_staging(Path::new("/Applications"));
    ensure!(!target.try_exists()?, "An app already exists in Applications. Open that copy instead");
    let staging = Staging::new(Path::new("/Applications"))?;
    let candidate = staging.0.join("one-mk.app");
    // Without the quarantine mark of the disk image: a quarantined app moved
    // by anything but Finder is translocated again when it starts.
    let copied = Command::new("/usr/bin/ditto").arg("--noqtn").arg(&source).arg(&candidate).output()?;
    ensure!(copied.status.success(), "Couldn't move the app to Applications. Drag it there in Finder");
    let _ = Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(&candidate).output();
    matching_team(&SystemCodeSign, &source, &candidate)?;
    rename_new(&candidate, target).context("An app already exists in Applications. Open that copy instead")?;
    restarting("Moved to Applications", "Open oneMK from Applications");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;

    struct FakeSign { candidate: Result<String, &'static str> }
    impl CodeSign for FakeSign {
        fn team(&self, path: &Path) -> Result<String> {
            if path == Path::new("running") { return Ok("ABCDEFGHIJ".into()); }
            self.candidate.as_ref().cloned().map_err(|error| anyhow::anyhow!("{error}"))
        }
    }

    #[test]
    fn leftover_staging_folders_go_and_nothing_else_does() {
        let parent = tempfile::tempdir().unwrap();
        let old = parent.path().join(".one-mk-bundle-60812-2/older.app/Contents/MacOS");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("one-mk-tray"), b"x").unwrap();
        fs::create_dir_all(parent.path().join("one-mk.app/Contents")).unwrap();
        fs::create_dir_all(parent.path().join(".one-mk.previous.app")).unwrap();
        fs::write(parent.path().join(".one-mk-bundle-note"), b"a file, not a staging folder").unwrap();
        clean_staging(parent.path());
        assert!(!parent.path().join(".one-mk-bundle-60812-2").exists());
        assert!(parent.path().join("one-mk.app/Contents").is_dir(), "the app stays");
        assert!(parent.path().join(".one-mk.previous.app").is_dir(), "the app kept for Go back stays");
        assert!(parent.path().join(".one-mk-bundle-note").is_file());
    }

    #[test]
    fn the_signature_requirement_is_given_to_codesign_as_text_not_as_a_file_name() {
        // "--test-requirement=anchor apple generic" makes codesign look for a file of that name and fail.
        assert!(VERIFY_ARGS.contains(&"--test-requirement==anchor apple generic"));
    }

    #[test]
    fn refuses_wrong_unsigned_and_invalid_teams() {
        for candidate in [Ok("OTHERTEAM1".into()), Ok(String::new()), Err("bad signature")] {
            assert!(matching_team(&FakeSign { candidate }, Path::new("running"), Path::new("new")).is_err());
        }
        assert!(matching_team(&FakeSign { candidate: Ok("ABCDEFGHIJ".into()) }, Path::new("running"), Path::new("new")).is_ok());
    }

    struct FakeFiles { paths: RefCell<HashSet<PathBuf>>, fail: bool }
    impl BundleFiles for FakeFiles {
        fn exists(&self, path: &Path) -> Result<bool> { Ok(self.paths.borrow().contains(path)) }
        fn rename(&self, source: &Path, destination: &Path) -> Result<()> {
            ensure!(!(self.fail && source == Path::new("new")), "injected failure");
            let mut paths = self.paths.borrow_mut();
            ensure!(!paths.contains(destination) && paths.remove(source), "invalid rename");
            paths.insert(destination.into());
            Ok(())
        }
        fn exchange(&self, a: &Path, b: &Path) -> Result<()> {
            let paths = self.paths.borrow();
            ensure!(paths.contains(a) && paths.contains(b), "invalid exchange");
            Ok(())
        }
    }

    #[test]
    fn swap_retains_previous_and_restores_on_failure() -> Result<()> {
        for fail in [true, false] {
            let files = FakeFiles { paths: RefCell::new([PathBuf::from("app"), PathBuf::from("new")].into()), fail };
            let result = swap(&files, Path::new("new"), Path::new("app"), Path::new("old"));
            assert_eq!(result.is_err(), fail);
            assert!(files.exists(Path::new("app"))?);
            assert_eq!(files.exists(Path::new("old"))?, !fail);
            assert_eq!(files.exists(Path::new("new"))?, fail);
        }
        Ok(())
    }

    #[test]
    fn a_failed_stop_or_swap_keeps_the_app_kept_for_go_back() -> Result<()> {
        for stop_fails in [true, false] {
            let files = FakeFiles { paths: RefCell::new(["app", "new", "old"].map(PathBuf::from).into()), fail: !stop_fails };
            let result = install(&files, || { ensure!(!stop_fails, "stop failed"); Ok(()) },
                Path::new("new"), Path::new("app"), Path::new("old"), Path::new("staging/older"));
            assert!(result.is_err());
            assert!(files.exists(Path::new("old"))?, "the backup from the last update is still there (stop_fails={stop_fails})");
            assert!(!files.exists(Path::new("staging/older"))?);
        }
        // A good update puts the older one aside for the staging folder to delete.
        let files = FakeFiles { paths: RefCell::new(["app", "new", "old"].map(PathBuf::from).into()), fail: false };
        install(&files, || Ok(()), Path::new("new"), Path::new("app"), Path::new("old"), Path::new("staging/older"))?;
        assert!(files.exists(Path::new("old"))? && files.exists(Path::new("staging/older"))?);
        Ok(())
    }

    #[cfg(unix)]
    fn fake_app(target: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        let executable = target.join("Contents/MacOS/one-mk");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, format!("#!/bin/sh\necho 'one-mk {version}'\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_receipt_of_an_update_that_never_swapped_is_cleared_and_a_real_one_stays() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let target = temporary.path().join("one-mk.app");
        fake_app(&target, "1.0.0");
        // Interrupted before the swap: the installed app is still 1.0.0.
        fs::write(receipt(&target), "1.1.0")?;
        clear_stale_receipt(&target)?;
        assert!(!receipt(&target).exists());
        // Swapped, new app not yet started: the receipt stays.
        fake_app(&target, "1.1.0");
        fs::write(receipt(&target), "1.1.0")?;
        clear_stale_receipt(&target)?;
        assert!(receipt(&target).exists());
        Ok(())
    }

    #[test]
    fn bundle_location_and_translocation() -> Result<()> {
        assert_eq!(bundle(Path::new("/Applications/one-mk.app/Contents/MacOS/one-mk"))?, Path::new("/Applications/one-mk.app"));
        assert_eq!(bundle(Path::new("/Applications/one-mk.APP/Contents/MacOS/one-mk"))?, Path::new("/Applications/one-mk.APP"));
        for path in ["/private/var/folders/x/AppTranslocation/id/d/one-mk.app", "/Volumes/one-mk/one-mk.app"] {
            assert!(policy::translocated(path));
        }
        assert!(!policy::translocated("/Applications/one-mk.app"));
        assert!(!policy::translocated("/Users/me/Apps/one-mk.app"));
        Ok(())
    }

    #[test]
    fn disk_swap_and_window_acknowledgement_keep_rollback_after_start() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let target = temporary.path().join("one-mk.app");
        let candidate = temporary.path().join("new.app");
        fs::create_dir(&target)?;
        fs::create_dir(&candidate)?;
        fs::write(target.join("version"), "old")?;
        fs::write(candidate.join("version"), "new")?;
        let previous = backup(&target)?;
        swap(&Disk, &candidate, &target, &previous)?;
        assert_eq!(fs::read_to_string(target.join("version"))?, "new");
        assert_eq!(fs::read_to_string(previous.join("version"))?, "old");
        fs::write(receipt(&target), env!("CARGO_PKG_VERSION"))?;
        let executable = target.join("Contents/MacOS/one-mk");
        assert!(acknowledge(&executable, "0.0.0").is_err());
        assert!(previous.is_dir());
        assert!(rollback_available(&executable));
        acknowledge(&executable, env!("CARGO_PKG_VERSION"))?;
        // Started: the previous app stays for "Go back".
        assert!(previous.is_dir());
        assert!(!receipt(&target).exists());
        assert!(rollback_available(&executable));
        Ok(())
    }
}
