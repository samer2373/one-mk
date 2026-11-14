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
//! a stamp records whether the file had *settled* (was a little older than
//! the clock's grain) at the moment it was taken, and that is part of what
//! stamps are compared by: a stamp taken while the file was fresh is never
//! trusted, the file is read again on every look until a stamp is taken when
//! it has settled, and only that read's stamp is trusted. A clock that moves
//! back makes the stamp of the same file unsettled again, so it differs too.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// How old a file's modification time must be before its stamp is trusted.
const SETTLED: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    Missing,
    File { len: u64, modified: Option<SystemTime>, changed: (i64, i64), id: (u64, u64), settled: bool },
}

impl Stamp {
    /// `None` when it can't be told (an error other than "not found"): the
    /// caller then reads the file, as it would have without a stamp.
    pub fn of(path: &Path) -> Option<Stamp> { Self::of_at(path, SystemTime::now()) }

    /// [`Stamp::of`] with the clock told, to tell whether the file has settled.
    fn of_at(path: &Path, now: SystemTime) -> Option<Stamp> {
        match std::fs::metadata(path) {
            Ok(meta) => Some(Self::from_meta(&meta, now)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Stamp::Missing),
            Err(_) => None,
        }
    }

    /// Whether this stamp can be trusted to tell the file's next change.
    fn trusted(&self) -> bool {
        match self {
            Stamp::Missing => true,
            Stamp::File { settled, .. } => *settled,
        }
    }

    #[cfg(unix)]
    fn from_meta(meta: &std::fs::Metadata, now: SystemTime) -> Stamp {
        use std::os::unix::fs::MetadataExt as _;
        let modified = meta.modified().ok();
        Stamp::File { len: meta.len(), modified, changed: (meta.ctime(), meta.ctime_nsec()), id: (meta.dev(), meta.ino()), settled: settled(modified, now) }
    }

    #[cfg(not(unix))]
    fn from_meta(meta: &std::fs::Metadata, now: SystemTime) -> Stamp {
        let created = meta.created().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok());
        let created = created.map_or((0, 0), |d| (d.as_secs() as i64, i64::from(d.subsec_nanos())));
        let modified = meta.modified().ok();
        Stamp::File { len: meta.len(), modified, changed: created, id: (0, 0), settled: settled(modified, now) }
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
        self.get_at(path, SystemTime::now(), load)
    }

    fn get_at(&mut self, path: &Path, clock: SystemTime, load: impl FnOnce() -> T) -> &T {
        // Looked at before reading: a change while it is read shows next time.
        let now = Stamp::of_at(path, clock);
        let kept = now.is_some() && now == self.stamp && self.value.is_some() && now.is_some_and(|s| s.trusted());
        if !kept {
            self.value = Some(load());
            self.stamp = now;
        }
        self.value.as_ref().expect("just set")
    }

    /// Whether `path` is the same as when [`Cached::remember`] last noted it.
    pub fn unchanged(&self, path: &Path) -> bool { self.unchanged_at(path, SystemTime::now()) }

    fn unchanged_at(&self, path: &Path, clock: SystemTime) -> bool {
        let now = Stamp::of_at(path, clock);
        now.is_some() && now == self.stamp && now.is_some_and(|s| s.trusted())
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

/// Whether a file last modified at `modified` had settled at `now`: it is
/// older than the clock's grain, and not from the future.
fn settled(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified.is_some_and(|m| now.duration_since(m).is_ok_and(|age| age >= SETTLED))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mtime(path: &Path) -> Option<SystemTime> { std::fs::metadata(path).unwrap().modified().ok() }

    /// A clock far enough after the files written in a test for them to count as settled.
    fn later() -> SystemTime { SystemTime::now() + Duration::from_secs(60) }

    #[test]
    fn a_file_is_read_again_only_after_it_changed() {
        let dir = crate::testdir::scratch_path("stamp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profiles.toml");
        let mut cache = Cached::default();
        let mut reads = 0;
        let mut read = |cache: &mut Cached<String>| {
            cache.get_at(&path, later(), || { reads += 1; std::fs::read_to_string(&path).unwrap_or_default() }).clone()
        };
        assert_eq!(read(&mut cache), "");
        assert_eq!(read(&mut cache), "", "a missing file is not looked for again");
        std::fs::write(&path, "one").unwrap();
        assert_eq!(read(&mut cache), "one");
        assert_eq!(read(&mut cache), "one");
        // Same length, written by renaming a new file over it (as every save does). The new file is
        // written until the file system's clock says it is a different moment from the old one: on a
        // coarse clock (Windows) two writes can otherwise share a timestamp, which is what stamps
        // cannot tell apart and what the unsettled-stamp rule exists for (tested below).
        let tmp = dir.join("profiles.toml.tmp");
        let before = mtime(&path);
        while {
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(&tmp, "two").unwrap();
            mtime(&tmp) == before
        } {}
        std::fs::rename(&tmp, &path).unwrap();
        assert_eq!(read(&mut cache), "two");
        // Same length, rewritten in place (an editor), a clock tick later.
        let before = mtime(&path);
        while {
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(&path, "six").unwrap();
            mtime(&path) == before
        } {}
        assert_eq!(read(&mut cache), "six");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read(&mut cache), "");
        drop(read);
        assert_eq!(reads, 5);
        cache.invalidate();
        assert!(!cache.unchanged(&path));
        std::fs::write(&path, "kept").unwrap();
        cache.remember(Stamp::of_at(&path, later()));
        assert!(cache.stamp.is_some_and(|s| s.trusted()));
        assert!(cache.unchanged_at(&path, later()), "the same file at a settled time is the same");
        std::fs::write(&path, "kept!").unwrap();
        assert!(!cache.unchanged_at(&path, later()), "a longer file is a change");
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
            cache.get_at(&path, SystemTime::now(), || { reads += 1; std::fs::read_to_string(&path).unwrap_or_default() }).clone()
        };
        // Same-length rewrites with no pause: the stamp may not differ, the reads must.
        for word in ["aaa", "bbb", "ccc", "ddd"] {
            std::fs::write(&path, word).unwrap();
            assert_eq!(read(&mut cache), word);
        }
        // Unchanged and still fresh: read every time.
        assert_eq!(read(&mut cache), "ddd");
        drop(read);
        assert_eq!(reads, 5);
        cache.remember(Stamp::of(&path));
        assert!(!cache.unchanged(&path), "a fresh file is not trusted");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stamp_taken_while_the_file_was_fresh_is_not_trusted_after_time_passes() {
        let dir = crate::testdir::scratch_path("stamp-settle");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devices.toml");
        let mut cache = Cached::default();
        let reads = std::cell::Cell::new(0);
        let read = |cache: &mut Cached<String>, clock: SystemTime| {
            cache.get_at(&path, clock, || { reads.set(reads.get() + 1); std::fs::read_to_string(&path).unwrap_or_default() }).clone()
        };
        std::fs::write(&path, "aaa").unwrap();
        let written = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(read(&mut cache, written), "aaa");
        // Nothing changed on disk, but the earlier stamp was taken fresh: one more read, taken settled.
        assert_eq!(read(&mut cache, written + SETTLED + Duration::from_secs(1)), "aaa");
        assert_eq!(read(&mut cache, written + SETTLED + Duration::from_secs(5)), "aaa");
        assert_eq!(reads.get(), 2, "the settled read happened once, then it was kept");
        // The clock moves back into the file's own moment: the stamp is no longer a settled one.
        assert_eq!(read(&mut cache, written), "aaa");
        assert_eq!(reads.get(), 3, "a clock that went back is not trusted");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
