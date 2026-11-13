//! What a settings file looks like on disk, from one `stat`: so the daemon
//! reads and parses a file again only when it changed, not on every request
//! the window sends while it is open (`status` four times a second).
//!
//! A change is any difference in size, modification or status-change time,
//! or (on Unix) the file itself: every save here writes a new file and
//! renames it over the old one, which also changes the inode. A file that
//! is missing has a stamp too, so a computer with nothing paired does not
//! keep looking either.
//!
//! A rewrite of the same length within one tick of the file system's clock
//! leaves the same stamp (on Windows there is no inode to tell it apart). So
//! a stamp taken while the file was only just modified is not trusted: the
//! file is read again on every look until a read happens when the file has
//! settled (is a little older than the clock's grain), and only that read's
//! stamp is trusted.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// How old a file's modification time must be before its stamp is trusted.
const SETTLED: Duration = Duration::from_secs(2);

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

    /// Whether the file was modified so recently (or claims to be from the
    /// future) that another change could still leave the same stamp.
    fn racy(&self) -> bool {
        match self {
            Stamp::Missing => false,
            Stamp::File { modified, .. } => {
                modified.is_none_or(|m| SystemTime::now().duration_since(m).map_or(true, |age| age < SETTLED))
            }
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
    /// Whether `stamp` was taken when the file had settled.
    trusted: bool,
    value: Option<T>,
}

impl<T> Default for Cached<T> {
    fn default() -> Self { Self { stamp: None, trusted: false, value: None } }
}

impl<T> Cached<T> {
    /// The value for `path` as it is now: `load`ed again only if the file
    /// changed since the last call (or can't be looked at).
    pub fn get(&mut self, path: &Path, load: impl FnOnce() -> T) -> &T {
        // Looked at before reading: a change while it is read shows next time.
        let now = Stamp::of(path);
        if now.is_none() || now != self.stamp || self.value.is_none() || !self.trusted {
            self.value = Some(load());
            self.stamp = now;
            self.trusted = settled(now);
        }
        self.value.as_ref().expect("just set")
    }

    /// Whether `path` is the same as when [`Cached::remember`] last noted it.
    pub fn unchanged(&self, path: &Path) -> bool {
        self.stamp.is_some() && self.trusted && Stamp::of(path) == self.stamp
    }

    /// Notes the file as it is now, with no value (for a caller that keeps
    /// what it read elsewhere). Take the stamp *before* reading.
    pub fn remember(&mut self, stamp: Option<Stamp>) {
        self.stamp = stamp;
        self.trusted = settled(stamp);
    }

    /// Forgets the stamp: the next look reads the file.
    pub fn invalidate(&mut self) {
        self.stamp = None;
        self.trusted = false;
        self.value = None;
    }
}

/// Whether a stamp taken now can be trusted to tell the file's next change.
fn settled(stamp: Option<Stamp>) -> bool {
    stamp.is_some_and(|s| !s.racy())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dates the file `hours` hours ago, so it counts as settled.
    fn age(path: &Path, hours: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600)).unwrap();
    }

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
        age(&path, 3);
        assert_eq!(read(&mut cache), "one");
        assert_eq!(read(&mut cache), "one");
        // Same length, written by renaming a new file over it (as every save does).
        let tmp = dir.join("profiles.toml.tmp");
        std::fs::write(&tmp, "two").unwrap();
        age(&tmp, 2);
        std::fs::rename(&tmp, &path).unwrap();
        assert_eq!(read(&mut cache), "two");
        // Same length, rewritten in place (an editor).
        std::fs::write(&path, "six").unwrap();
        age(&path, 1);
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

    #[test]
    fn a_file_changed_a_moment_ago_is_always_read_again() {
        let dir = crate::testdir::scratch_path("stamp-racy");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devices.toml");
        let mut cache = Cached::default();
        let mut reads = 0;
        let mut read = |cache: &mut Cached<String>| {
            cache.get(&path, || { reads += 1; std::fs::read_to_string(&path).unwrap_or_default() }).clone()
        };
        // Several same-length rewrites with no pause between them: the stamp may not differ, the reads must.
        for word in ["aaa", "bbb", "ccc", "ddd"] {
            std::fs::write(&path, word).unwrap();
            assert_eq!(read(&mut cache), word);
        }
        drop(read);
        assert_eq!(reads, 4);
        cache.remember(Stamp::of(&path));
        assert!(!cache.unchanged(&path), "a fresh file is not trusted");
        age(&path, 1);
        cache.remember(Stamp::of(&path));
        assert!(cache.unchanged(&path));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stamp_taken_while_the_file_was_fresh_is_not_trusted_once_it_ages() {
        let dir = crate::testdir::scratch_path("stamp-settle");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devices.toml");
        let mut cache = Cached::default();
        let mut reads = 0;
        let mut read = |cache: &mut Cached<String>| {
            cache.get(&path, || { reads += 1; std::fs::read_to_string(&path).unwrap_or_default() }).clone()
        };
        std::fs::write(&path, "aaa").unwrap();
        assert_eq!(read(&mut cache), "aaa");
        assert!(!cache.trusted, "read while fresh");
        // The file now looks old but nothing was read since: the next look reads, and only then trusts.
        age(&path, 1);
        assert_eq!(read(&mut cache), "aaa");
        assert!(cache.trusted, "read after it settled");
        assert_eq!(read(&mut cache), "aaa");
        drop(read);
        assert_eq!(reads, 2, "the settled read happened once, then it was kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
