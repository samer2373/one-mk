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

impl CodeSign for SystemCodeSign {
    fn team(&self, path: &Path) -> Result<String> {
        let verified = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"])
            .arg(path).output().context("Couldn't verify the app's identity")?;
        ensure!(verified.status.success(), "The app's security check failed; nothing was replaced");
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
    fn rename(&self, source: &Path, destination: &Path) -> Result<()> { Ok(fs::rename(source, destination)?) }
}

fn swap(files: &impl BundleFiles, candidate: &Path, target: &Path, previous: &Path) -> Result<()> {
    ensure!(files.exists(target)? && files.exists(candidate)?, "The app or its update is missing");
    ensure!(!files.exists(previous)?, "The previous app is still waiting for a successful restart");
    files.rename(target, previous).context("This app's folder can't be changed by your account. Choose Download instead")?;
    if let Err(error) = files.rename(candidate, target) {
        files.rename(previous, target).context("Couldn't restore the previous app; it is saved beside the installation")?;
        return Err(error).context("Couldn't install the update; the previous app was restored");
    }
    Ok(())
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

pub(super) fn apply_bundle(manifest: &Manifest, release: &Release, origin: &Url, executable: &Path, home: &Path) -> Result<()> {
    let target = bundle(executable)?;
    ensure!(!policy::translocated(&target.to_string_lossy()), "Move oneMK to Applications before updating");
    let _lock = target_lock(&target).context("This app's folder can't be changed by your account. Choose Download instead")?;
    let previous = backup(&target)?;
    ensure!(!Disk.exists(&previous)? && !receipt(&target).try_exists()?, "The previous app is still waiting for a successful restart");
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
    if let Err(error) = stop_daemon(home).and_then(|()| swap(&Disk, &candidate, &target, &previous)) {
        if Disk.exists(&target).unwrap_or(false) { let _ = fs::remove_file(&marker); }
        return Err(error);
    }
    sync_directory(target.parent().context("The app has no parent folder")?)?;
    println!("Restarting…");
    Ok(())
}

pub(super) fn acknowledge(executable: &Path, started_version: &str) -> Result<()> {
    ensure!(started_version == env!("CARGO_PKG_VERSION"), "The previous window is still running");
    let target = bundle(executable)?;
    let marker = receipt(&target);
    if !marker.try_exists()? { return Ok(()); }
    let _lock = target_lock(&target)?;
    regular(&marker)?;
    let version = fs::read_to_string(&marker)?;
    ensure!(version == started_version, "The previous app is kept until the new window starts");
    let previous = backup(&target)?;
    if Disk.exists(&previous)? { fs::remove_dir_all(previous)?; }
    fs::remove_file(marker)?;
    Ok(())
}

pub(super) fn rollback_available(executable: &Path) -> bool {
    bundle(executable).is_ok_and(|target| receipt(&target).is_file() && backup(&target).is_ok_and(|previous| previous.is_dir()))
}

pub(super) fn rollback_bundle(executable: &Path, home: &Path) -> Result<()> {
    let target = bundle(executable)?;
    let _lock = target_lock(&target)?;
    let previous = backup(&target)?;
    matching_team(&SystemCodeSign, &target, &previous)?;
    let staging = Staging::new(target.parent().context("The app has no parent folder")?)?;
    stop_daemon(home)?;
    swap(&Disk, &previous, &target, &staging.0.join("failed.app"))?;
    let _ = fs::remove_file(receipt(&target));
    println!("Previous app restored. Restarting…");
    Ok(())
}

pub(super) fn move_to_applications(executable: &Path) -> Result<()> {
    let source = bundle(executable)?;
    let target = Path::new("/Applications/one-mk.app");
    ensure!(policy::translocated(&source.to_string_lossy()), "The app is already installed outside the disk image");
    let _lock = target_lock(target).context("Your account can't move this app to Applications. Drag it there in Finder")?;
    ensure!(!target.try_exists()?, "An app already exists in Applications. Open that copy instead");
    let staging = Staging::new(Path::new("/Applications"))?;
    let candidate = staging.0.join("one-mk.app");
    let copied = Command::new("/usr/bin/ditto").arg(&source).arg(&candidate).output()?;
    ensure!(copied.status.success(), "Couldn't move the app to Applications. Drag it there in Finder");
    matching_team(&SystemCodeSign, &source, &candidate)?;
    fs::rename(&candidate, target)?;
    println!("Moved to Applications. Restarting…");
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
    fn disk_swap_and_window_acknowledgement_keep_rollback_until_start() -> Result<()> {
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
        assert!(!previous.exists());
        assert!(!receipt(&target).exists());
        Ok(())
    }
}
