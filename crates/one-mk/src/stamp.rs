//! What a settings file looks like on disk, from one `stat`: so the daemon
//! reads and parses a file again only when it changed, not on every request
//! the window sends while it is open (`status` four times a second).
//!
//! A change is any difference in size, modification or status-change time,
//! or (on Unix) the file itself: every save here writes a new file and
//! renames it over the old one, which also changes the inode. A file that
//! is missing has a stamp too, so a computer with nothing paired does not
//! keep looking either.

use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    Missing,
    File { len: u64, modified: Option<SystemTime>, changed: (i64, i64), id: (u64, u64) },
}

impl Stamp {
    /// `None` when it can't be told (an error other than "not found"): the
    /// caller then reads the file, as it would have without a stamp.
    pub fn of(path: &Path) -> Option<Stamp> {
        match std::fs::metadata(path) {
            Ok(meta) => Some(Self::from_meta(&meta)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Stamp::Missing),
            Err(_) => None,
        }
    }

    #[cfg(unix)]
    fn from_meta(meta: &std::fs::Metadata) -> Stamp {
        use std::os::unix::fs::MetadataExt as _;
        Stamp::File { len: meta.len(), modified: meta.modified().ok(), changed: (meta.ctime(), meta.ctime_nsec()), id: (meta.dev(), meta.ino()) }
    }

    #[cfg(not(unix))]
    fn from_meta(meta: &std::fs::Metadata) -> Stamp {
        let created = meta.created().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok());
        let created = created.map_or((0, 0), |d| (d.as_secs() as i64, i64::from(d.subsec_nanos())));
        Stamp::File { len: meta.len(), modified: meta.modified().ok(), changed: created, id: (0, 0) }
    }
}

/// A value read from a file, kept until the file changes.
#[derive(Debug)]
pub struct Cached<T> {
    stamp: Option<Stamp>,
    value: Option<T>,
}

impl<T> Default for Cached<T> {
    fn default() -> Self { Self { stamp: None, value: None } }
}

impl<T> Cached<T> {
    /// The value for `path` as it is now: `load`ed again only if the file
    /// changed since the last call (or can't be looked at).
    pub fn get(&mut self, path: &Path, load: impl FnOnce() -> T) -> &T {
        // Looked at before reading: a change while it is read shows next time.
        let now = Stamp::of(path);
        if now.is_none() || now != self.stamp || self.value.is_none() {
            self.value = Some(load());
            self.stamp = now;
        }
        self.value.as_ref().expect("just set")
    }

    /// Whether `path` is the same as when [`Cached::remember`] last noted it.
    pub fn unchanged(&self, path: &Path) -> bool {
        self.stamp.is_some() && Stamp::of(path) == self.stamp
    }

    /// Notes the file as it is now, with no value (for a caller that keeps
    /// what it read elsewhere). Take the stamp *before* reading.
    pub fn remember(&mut self, stamp: Option<Stamp>) { self.stamp = stamp; }

    /// Forgets the stamp: the next look reads the file.
    pub fn invalidate(&mut self) {
        self.stamp = None;
        self.value = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_read_again_only_after_it_changed() {
        let dir = crate::testdir::scratch_path("stamp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profiles.toml");
        let mut cache = Cached::default();
        let mut reads = 0;
        let mut read = |cache: &mut Cached<String>| {
            cache.get(&path, || { reads += 1; std::fs::read_to_string(&path).unwrap_or_default() }).clone()
        };
        assert_eq!(read(&mut cache), "");
        assert_eq!(read(&mut cache), "", "a missing file is not looked for again");
        std::fs::write(&path, "one").unwrap();
        assert_eq!(read(&mut cache), "one");
        assert_eq!(read(&mut cache), "one");
        // Same length, written by renaming a new file over it (as every save does).
        let tmp = dir.join("profiles.toml.tmp");
        std::fs::write(&tmp, "two").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert_eq!(read(&mut cache), "two");
        // Same length, rewritten in place (an editor), a clock tick later:
        // a rewrite within the same tick of the file system's clock and of
        // the same length is the one change a stamp can miss.
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&path, "six").unwrap();
        assert_eq!(read(&mut cache), "six");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read(&mut cache), "");
        drop(read);
        assert_eq!(reads, 5);
        cache.invalidate();
        assert!(!cache.unchanged(&path));
        cache.remember(Stamp::of(&path));
        assert!(cache.unchanged(&path));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
