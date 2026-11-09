//! Files and folders oneMK keeps for its own account. They are created
//! private (`0600` files, `0700` folders) from the first byte, whatever the
//! process umask is (Ubuntu's default, `002`, would make them
//! group-writable): a file made with the default mode and tightened after
//! is readable by the group for a moment, and a crash in between leaves it
//! so. On Windows and macOS these are plain file operations (their folders
//! are per-user already).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

fn private() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

/// Creates `path` and any missing parents as private folders (`0700`).
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Creates one folder others may enter (`0755`) but only its owner may
/// change, whatever the umask (a folder of installed program files).
pub fn create_dir_readable(path: &Path) -> io::Result<()> {
    #[allow(unused_mut)]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o755);
    builder.create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if std::fs::metadata(path).is_ok_and(|metadata| metadata.mode() & 0o022 != 0) {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// A new private file for writing; fails if it exists.
pub fn create_new(path: &Path) -> io::Result<File> {
    private().write(true).create_new(true).open(path)
}

/// A private file opened for appending, created if missing (a log).
pub fn append(path: &Path) -> io::Result<File> {
    private().create(true).append(true).open(path)
}

/// A private file opened for reading and writing, created if missing and
/// never truncated (a lock file).
pub fn lock_file(path: &Path) -> io::Result<File> {
    private().read(true).write(true).create(true).truncate(false).open(path)
}

/// A private file opened for writing from the start (created or emptied).
pub fn write_open(path: &Path) -> io::Result<File> {
    let file = private().write(true).create(true).truncate(true).open(path)?;
    tighten_file(&file);
    Ok(file)
}

/// Writes `contents` to a private file, creating or replacing it.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let mut file = private().write(true).create(true).truncate(true).open(path)?;
    tighten_file(&file);
    file.write_all(contents.as_ref())
}

/// Writes `contents` to a file others may read (an autostart or menu entry
/// the desktop has to find) but nobody but the owner may change: `0644`,
/// whatever the umask, and never group-writable.
pub fn write_readable(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o644);
    let mut file = options.write(true).create(true).truncate(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if file.metadata().is_ok_and(|metadata| metadata.is_file() && metadata.mode() & 0o022 != 0) {
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o644));
        }
    }
    file.write_all(contents.as_ref())
}

/// Makes an existing file that group or others can reach private. Files an
/// older oneMK made with the process umask are fixed this way; failing to
/// (not ours, read-only mount) is not worth stopping for. A link is left
/// alone: it is never followed to somebody else's file.
pub fn tighten(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if let Ok(metadata) = std::fs::symlink_metadata(path)
            && metadata.is_file() && metadata.mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn tighten_file(file: &File) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if let Ok(metadata) = file.metadata()
            && metadata.is_file() && metadata.mode() & 0o077 != 0
        {
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = file;
}

/// The files oneMK itself keeps in a settings folder, which older versions
/// created with the process umask: made private when the program starts,
/// so a computer that already ran an earlier version is tightened too.
/// Nothing else in the folder is touched (a `--home` may hold other files).
pub fn tighten_home(dir: &Path) {
    for name in ["daemon.log", "setup-done", "setup.lock", "tray.lock", "update-state.json", "update.lock", "profiles.toml", "profiles.toml.lock",
        "devices.toml.lock", "settings.toml.lock"] {
        tighten(&dir.join(name));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let cache = dir.join("wallpaper-video");
        if let Ok(metadata) = std::fs::symlink_metadata(&cache)
            && metadata.is_dir() && metadata.mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 { std::fs::metadata(path).unwrap().permissions().mode() & 0o777 }

    /// Whatever the umask, what is created is private; this test's own umask is
    /// not changed (other tests run beside it), the modes are asked for.
    #[test]
    fn files_and_folders_are_created_private() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("a/b");
        create_dir_all(&folder).unwrap();
        assert_eq!((mode(&root.path().join("a")), mode(&folder)), (0o700, 0o700));
        let file = folder.join("log");
        append(&file).unwrap();
        assert_eq!(mode(&file), 0o600);
        lock_file(&folder.join("lock")).unwrap();
        assert_eq!(mode(&folder.join("lock")), 0o600);
        create_new(&folder.join("new")).unwrap();
        assert!(create_new(&folder.join("new")).is_err());
        write(&folder.join("text"), b"hello").unwrap();
        assert_eq!((mode(&folder.join("new")), mode(&folder.join("text"))), (0o600, 0o600));
        assert_eq!(std::fs::read(folder.join("text")).unwrap(), b"hello");
        // A shared entry is readable, never group-writable, even over an older loose copy.
        let entry = folder.join("entry.desktop");
        std::fs::write(&entry, "old").unwrap();
        std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o666)).unwrap();
        write_readable(&entry, "new").unwrap();
        assert_eq!((mode(&entry) & 0o022, std::fs::read(&entry).unwrap()), (0, b"new".to_vec()));
        write_readable(&folder.join("fresh.desktop"), "x").unwrap();
        assert_eq!(mode(&folder.join("fresh.desktop")) & 0o133, 0);
    }

    #[test]
    fn an_older_loose_file_is_tightened_and_nothing_else_is() {
        let root = tempfile::tempdir().unwrap();
        for name in ["daemon.log", "profiles.toml", "unrelated.txt"] {
            std::fs::write(root.path().join(name), "x").unwrap();
            std::fs::set_permissions(root.path().join(name), std::fs::Permissions::from_mode(0o664)).unwrap();
        }
        std::fs::create_dir(root.path().join("wallpaper-video")).unwrap();
        std::fs::set_permissions(root.path().join("wallpaper-video"), std::fs::Permissions::from_mode(0o775)).unwrap();
        tighten_home(root.path());
        assert_eq!((mode(&root.path().join("daemon.log")), mode(&root.path().join("profiles.toml"))), (0o600, 0o600));
        assert_eq!(mode(&root.path().join("wallpaper-video")), 0o700);
        assert_eq!(mode(&root.path().join("unrelated.txt")), 0o664, "a file oneMK did not make is left alone");
        // A link is never followed.
        let target = root.path().join("target");
        std::fs::write(&target, "x").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(root.path().join("daemon.log")).unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("daemon.log")).unwrap();
        tighten_home(root.path());
        assert_eq!(mode(&target), 0o644);
    }
}
