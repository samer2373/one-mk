//! Files and folders only this account can read, from the first byte.
//!
//! The settings folder holds the paired computers, Wi-Fi names, router
//! addresses and a log that names them all. Whatever the process's umask
//! (often 002 on Linux desktops), what oneMK creates there is made private
//! (folders 0700, files 0600) as it is created, not fixed up afterwards.
//! Existing files keep their mode. Elsewhere than Unix this changes nothing.

use std::fs::OpenOptions;
use std::io;
use std::path::Path;

/// Folder mode for what oneMK creates.
pub const DIR_MODE: u32 = 0o700;
/// File mode for what oneMK creates.
pub const FILE_MODE: u32 = 0o600;

/// `OpenOptions` whose newly created file is readable by this account only.
pub fn options() -> OpenOptions {
    #[allow(unused_mut)] // set on Unix only
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, FILE_MODE);
    options
}

/// Creates `dir` and any missing parents, each private; fine if it exists.
pub fn create_dir_all(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, DIR_MODE);
    builder.create(dir)
}

/// Writes `bytes` to `path` (replacing what is there), creating it private.
pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    use std::io::Write;
    options().write(true).create(true).truncate(true).open(path)?.write_all(bytes.as_ref())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn what_is_created_is_private() {
        let base = std::env::temp_dir().join(format!("one-mk-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // Whatever the umask lets through (often group read and write).
        let dir = base.join("a/b");
        create_dir_all(&dir).unwrap();
        write(&dir.join("file"), "x").unwrap();
        let appended = dir.join("log");
        options().create(true).append(true).open(&appended).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&base.join("a")), 0o700);
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("file")), 0o600);
        assert_eq!(mode(&appended), 0o600);
        // An existing file keeps its mode; its contents are replaced.
        std::fs::set_permissions(dir.join("file"), std::fs::Permissions::from_mode(0o644)).unwrap();
        write(&dir.join("file"), "y").unwrap();
        assert_eq!(mode(&dir.join("file")), 0o644);
        assert_eq!(std::fs::read_to_string(dir.join("file")).unwrap(), "y");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
