use crate::{ClipData, Digest};
use anyhow::{Result, bail};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, AtomicU64, Ordering}};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_BYTES: usize = 64 << 20;
const MAX_ITEMS: usize = 1000;
const MAX_BYTES: usize = 256 << 20;
pub(crate) const SAVE_PREFIX: &str = ".clipboard-history-";

pub fn clean_unfinished(directory: &std::path::Path) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(SAVE_PREFIX) {
            let kind = entry.file_type()?;
            if kind.is_file() || kind.is_symlink() { std::fs::remove_file(entry.path())?; }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub data: Arc<ClipData>,
    pub timestamp: u64,
    pub origin: String,
    pub digest: Digest,
}

impl Entry {
    pub fn kind(&self) -> &'static str {
        if self.data.html.is_some() { "html" } else if self.data.text.is_some() { "text" } else { "image" }
    }

    pub fn preview(&self) -> String {
        if let Some(text) = self.data.text.as_ref().or(self.data.html.as_ref()) {
            safe_preview(text)
        } else {
            self.data.png.as_deref().and_then(|bytes| {
                let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
                let reader = decoder.read_info().ok()?;
                Some(format!("image {}x{}", reader.info().width, reader.info().height))
            }).unwrap_or_else(|| "image".into())
        }
    }
}

pub fn safe_preview(text: &str) -> String {
    text.chars().filter(|ch| !invisible(*ch)).take(80).collect()
}

pub fn invisible(ch: char) -> bool {
    use unicode_general_category::{get_general_category, GeneralCategory};
    matches!(get_general_category(ch), GeneralCategory::Control | GeneralCategory::Format | GeneralCategory::LineSeparator | GeneralCategory::ParagraphSeparator)
        || matches!(ch, '\u{034f}' | '\u{115f}'..='\u{1160}' | '\u{17b4}'..='\u{17b5}' | '\u{180b}'..='\u{180f}' | '\u{2065}' | '\u{3164}' | '\u{fe00}'..='\u{fe0f}' | '\u{ffa0}' | '\u{fff0}'..='\u{fff8}' | '\u{1bca0}'..='\u{1bca3}' | '\u{1d173}'..='\u{1d17a}' | '\u{e0000}'..='\u{e0fff}')
}

#[derive(Default)]
pub struct Status {
    pub len: AtomicUsize,
    pub revision: AtomicU64,
    pub disabled: AtomicBool,
    pub monitoring_unavailable: AtomicBool,
    note: Mutex<Option<String>>,
    saver: Mutex<Option<crate::history_save::Saver>>,
}

impl Status {
    pub fn note(&self) -> Option<String> {
        self.note.try_lock().ok().and_then(|note| note.clone()).or_else(|| self.saver.try_lock().ok().and_then(|saver| saver.as_ref().and_then(|saver| saver.error())))
    }
}

pub fn digest(data: &ClipData) -> Digest {
    let mut hash = blake3::Hasher::new();
    for (tag, bytes) in [(1u8, data.text.as_deref().map(str::as_bytes)), (2, data.html.as_deref().map(str::as_bytes)), (3, data.png.as_deref())] {
        if let Some(bytes) = bytes {
            hash.update(&[tag]);
            hash.update(&(bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    *hash.finalize().as_bytes()
}

pub struct History {
    entries: VecDeque<Entry>,
    bytes: usize,
    max: usize,
    byte_cap: usize,
    item_cap: usize,
    next: u64,
    revision: u64,
    path: Option<PathBuf>,
    pub error: Option<String>,
    pub monitoring_unavailable: bool,
    status: Arc<Status>,
    saver: Option<crate::history_save::Saver>,
    last_saved: Option<(Option<PathBuf>, u64)>,
    blocked: bool,
    remove_path: Option<PathBuf>,
    removed_path: Option<PathBuf>,
}

impl Default for History {
    fn default() -> Self {
        Self { entries: VecDeque::new(), bytes: 0, max: 20, byte_cap: DEFAULT_BYTES,
            item_cap: crate::DEFAULT_MAX_BYTES, next: 1, revision: 0, path: None, error: None, monitoring_unavailable: false,
            status: Arc::default(), saver: None, last_saved: None, blocked: false, remove_path: None, removed_path: None }
    }
}

impl History {
    pub fn configure(&mut self, max: usize, bytes: usize, item_cap: usize, path: Option<PathBuf>) {
        self.max = max.min(MAX_ITEMS);
        self.byte_cap = bytes.min(MAX_BYTES);
        self.item_cap = item_cap.min(crate::CEILING_BYTES);
        let load = !self.disabled() && self.path.is_none() && path.is_some() && self.entries.is_empty() && self.saver.is_none();
        if self.disabled() { self.blocked = false; self.error = None; }
        if self.path != path { self.remove_path = self.path.take(); self.blocked = false; self.error = None; }
        self.path = path;
        if self.path.is_some() { self.removed_path = None; }
        if load && let Err(error) = self.load() {
            self.error = Some(format!("Could not load clipboard history: {error}; clear it with one-mk clipboard history clear."));
            self.blocked = true;
            self.trim();
            self.publish();
            return;
        }
        if load && !self.disabled() { self.last_saved = Some((self.path.clone(), self.revision)); }
        self.trim();
        self.save_checked();
    }

    pub fn list(&self) -> Vec<Entry> { self.entries.iter().rev().cloned().collect() }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn disabled(&self) -> bool { self.max == 0 || self.byte_cap == 0 }
    pub fn get(&self, id: u64) -> Option<Entry> { self.entries.iter().find(|item| item.id == id).cloned() }
    pub fn status(&self) -> Arc<Status> { self.status.clone() }
    pub fn remove_saved_file(&mut self, path: PathBuf) {
        if self.path.is_none() && self.removed_path.as_ref() == Some(&path) && self.saver.as_ref().is_none_or(|saver| saver.error().is_none()) { return; }
        if self.saver.is_some() || path.exists() { self.remove_path = Some(path.clone()); }
        self.removed_path = Some(path);
    }
    pub(crate) fn saver(&self) -> Option<crate::history_save::Saver> { self.saver.clone() }

    fn publish(&self) {
        self.status.len.store(self.len(), Ordering::Relaxed);
        self.status.revision.store(self.revision, Ordering::Relaxed);
        self.status.disabled.store(self.disabled(), Ordering::Relaxed);
        if self.monitoring_unavailable { self.status.monitoring_unavailable.store(true, Ordering::Relaxed); }
        *self.status.note.lock().unwrap_or_else(|error| error.into_inner()) = self.error.clone();
    }

    #[cfg(test)]
    fn flush(&self) { if let Some(saver) = &self.saver { saver.flush().unwrap(); } }

    pub fn record(&mut self, data: Arc<ClipData>, concealed: bool, origin: &str) {
        let digest = digest(&data);
        self.record_hashed(data, digest, concealed, origin);
    }

    pub fn record_hashed(&mut self, data: Arc<ClipData>, digest: Digest, concealed: bool, origin: &str) {
        if concealed || self.max == 0 || self.next == u64::MAX || data.is_empty() || data.len() > self.item_cap || data.len() > self.byte_cap { return; }
        if self.entries.back().is_some_and(|entry| entry.digest == digest) { return; }
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        self.bytes += data.len();
        self.entries.push_back(Entry { id: self.next, data, timestamp, origin: safe_preview(origin), digest });
        self.next = self.next.saturating_add(1);
        self.revision = self.revision.wrapping_add(1);
        self.trim();
        self.save_checked();
    }

    fn remove(&mut self, position: usize) {
        if let Some(entry) = self.entries.remove(position) { self.bytes -= entry.data.len(); self.revision = self.revision.wrapping_add(1); }
    }

    fn trim(&mut self) {
        let mut position = 0;
        while position < self.entries.len() {
            if self.entries.get(position).is_some_and(|entry| entry.data.len() > self.item_cap) { self.remove(position); }
            else { position += 1; }
        }
        while self.entries.len() > self.max || self.bytes > self.byte_cap {
            let position = self.entries.iter().position(|entry| entry.kind() == "image").unwrap_or(0);
            self.remove(position);
        }
    }

    pub fn delete(&mut self, id: u64) -> bool {
        let Some(position) = self.entries.iter().position(|entry| entry.id == id) else { return false; };
        self.remove(position);
        self.save_checked();
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.revision = self.revision.wrapping_add(1);
        self.error = None;
        self.blocked = false;
        self.save_checked();
    }

    fn save_checked(&mut self) {
        self.publish();
        if self.blocked { return; }
        let state = (self.path.clone(), self.revision);
        if self.remove_path.is_none() && self.last_saved.as_ref() == Some(&state) && self.saver.as_ref().is_none_or(|saver| saver.error().is_none()) { return; }
        if self.path.is_none() && self.remove_path.is_none() && self.saver.is_none() { return; }
        let saver = self.saver.get_or_insert_with(crate::history_save::Saver::new);
        *self.status.saver.lock().unwrap_or_else(|error| error.into_inner()) = Some(saver.clone());
        saver.submit(crate::history_save::Snapshot { path: self.path.clone(), entries: self.entries.iter().cloned().collect() }, self.remove_path.take());
        self.last_saved = Some(state);
    }

    fn load(&mut self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()); };
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > (MAX_BYTES + MAX_ITEMS * 1024) as u64 { bail!("saved file is over the safety limit"); }
        let mut magic = [0; 5];
        file.read_exact(&mut magic)?;
        if &magic != b"OMKH1" { bail!("unrecognised saved file"); }
        let count = read_u64(&mut file)?;
        if count > MAX_ITEMS as u64 { bail!("too many saved items"); }
        let mut entries = VecDeque::new();
        let mut bytes = 0usize;
        for _ in 0..count {
            let id = read_u64(&mut file)?;
            let timestamp = read_u64(&mut file)?;
            let origin = read_field(&mut file, 1024)?.map(String::from_utf8).transpose()?.unwrap_or_default();
            let text = read_field(&mut file, crate::CEILING_BYTES)?.map(String::from_utf8).transpose()?;
            let html = read_field(&mut file, crate::CEILING_BYTES)?.map(String::from_utf8).transpose()?;
            let png = read_field(&mut file, crate::CEILING_BYTES)?;
            let data = Arc::new(ClipData { text, html, png });
            bytes = bytes.checked_add(data.len()).ok_or_else(|| anyhow::anyhow!("saved history is too large"))?;
            if bytes > MAX_BYTES || id == 0 || id == u64::MAX || entries.iter().any(|entry: &Entry| entry.id == id) { bail!("invalid saved history"); }
            if let Some(png) = data.png.as_deref() { crate::decode_png(png)?; }
            entries.push_back(Entry { id, timestamp, origin: safe_preview(&origin), digest: digest(&data), data });
        }
        let mut trailing = [0];
        if file.read(&mut trailing)? != 0 { bail!("unexpected saved data"); }
        self.next = entries.iter().map(|entry| entry.id).max().unwrap_or(0) + 1;
        self.entries = entries;
        self.bytes = bytes;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

pub(crate) fn write_field(file: &mut impl Write, bytes: Option<&[u8]>) -> Result<()> {
    file.write_all(&bytes.map_or(u64::MAX, |bytes| bytes.len() as u64).to_le_bytes())?;
    if let Some(bytes) = bytes { file.write_all(bytes)?; }
    Ok(())
}

fn read_u64(file: &mut impl Read) -> Result<u64> {
    let mut bytes = [0; 8];
    file.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_field(file: &mut impl Read, max: usize) -> Result<Option<Vec<u8>>> {
    let length = read_u64(file)?;
    if length == u64::MAX { return Ok(None); }
    if length > max as u64 { bail!("saved item is over the safety limit"); }
    let mut bytes = Vec::new();
    file.take(length).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != length { bail!("incomplete saved item"); }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    #[test]
    fn disabled_history_removes_corrupt_container_without_loading() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard-history.bin");
        std::fs::write(&path, b"corrupt").unwrap();
        let mut history = History::default();
        history.configure(0, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.flush();
        assert!(!path.exists());
        assert!(history.error.is_none());
    }
    #[test]
    fn disabled_history_removes_existing_empty_container() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard-history.bin");
        std::fs::write(&path, [b"OMKH1".as_slice(), &0u64.to_le_bytes()].concat()).unwrap();
        let mut history = History::default();
        history.configure(0, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.flush();
        assert!(!path.exists());
    }
    use super::*;
    fn text(value: &str) -> Arc<ClipData> { Arc::new(ClipData { text: Some(value.into()), ..ClipData::default() }) }

    #[test]
    fn interrupted_saves_are_removed_without_touching_other_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(".clipboard-history-interrupted"), b"unfinished").unwrap();
        std::fs::write(directory.path().join("clipboard-history.bin"), b"saved").unwrap();
        clean_unfinished(directory.path()).unwrap();
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(std::fs::read(directory.path().join("clipboard-history.bin")).unwrap(), b"saved");
    }

    #[test]
    fn privacy_dedup_and_caps() {
        let mut history = History::default();
        history.configure(2, 6, 5, None);
        history.record(text("pwd"), true, "here");
        assert!(history.is_empty());
        history.record(text("first"), false, "here");
        history.record(text("first"), false, "peer");
        assert_eq!(history.len(), 1);
        history.record(text("new"), false, "peer");
        assert_eq!(history.len(), 1);
        assert_eq!(history.list()[0].preview(), "new");
        history.configure(0, 6, 5, None);
        assert!(history.is_empty());
        history.record(text("new"), false, "here");
        assert!(history.is_empty());
    }

    #[test]
    fn images_evicted_before_text_and_ids_are_stable() {
        let mut history = History::default();
        history.configure(2, 1000, 1000, None);
        history.record(text("first"), false, "here");
        history.record(Arc::new(ClipData { png: Some(vec![1]), ..ClipData::default() }), false, "peer");
        history.record(text("last"), false, "peer");
        assert_eq!(history.list().iter().map(Entry::preview).collect::<Vec<_>>(), ["last", "first"]);
        assert!(history.delete(1));
        assert!(history.get(3).is_some());
    }

    #[test]
    fn previews_strip_controls_bidi_and_bound_unicode() {
        assert_eq!(safe_preview("a\u{e0001}\u{e007f}\u{fff9}\u{180e}\u{ad}\u{34f}\u{115f}\u{1160}\u{3164}\u{ffa0}\u{fe0f}b"), "ab");
        assert_eq!(safe_preview("a\n\r\t\u{1b}\u{202e}\u{2066}b"), "ab");
        assert_eq!(safe_preview(&"🦀".repeat(10000)).chars().count(), 80);
        assert!(String::from_utf8(vec![0xff]).is_err());
        for code in 0..=0x10ffff {
            if let Some(ch) = char::from_u32(code) {
                let preview = safe_preview(&ch.to_string());
                assert!(preview.chars().all(|ch| !ch.is_control()));
                if matches!(unicode_general_category::get_general_category(ch), unicode_general_category::GeneralCategory::Format | unicode_general_category::GeneralCategory::LineSeparator | unicode_general_category::GeneralCategory::ParagraphSeparator) {
                    assert!(preview.is_empty(), "format scalar U+{code:04X} escaped filtering");
                }
            }
        }
    }

    #[test]
    fn disabled_persistence_removes_file_and_clears_load_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard-history.bin");
        std::fs::write(&path, b"corrupt").unwrap();
        let mut history = History::default();
        history.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        assert!(history.error.is_some());
        history.configure(0, DEFAULT_BYTES, 1024, None);
        history.flush();
        assert!(history.error.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn disabled_history_does_not_leave_an_empty_container() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard-history.bin");
        let mut history = History::default();
        history.configure(0, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.flush();
        assert!(!path.exists());
    }

    #[test]
    fn unchanged_configuration_does_not_rewrite_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard-history.bin");
        let mut history = History::default();
        history.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.record(text("retained"), false, "peer");
        history.flush();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        history.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.flush();
        assert_eq!(before, std::fs::metadata(path).unwrap().modified().unwrap());
    }

    #[test]
    fn transient_save_failure_retries_after_the_next_change() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("later");
        let path = parent.join("clipboard-history.bin");
        let mut history = History::default();
        history.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.record(text("first"), false, "peer");
        assert!(history.saver.as_ref().unwrap().flush().is_err());
        std::fs::create_dir(&parent).unwrap();
        history.record(text("second"), false, "peer");
        history.flush();
        assert!(path.exists());
    }

    #[test]
    fn persistence_is_opt_in_private_atomic_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard-history.bin");
        let mut history = History::default();
        history.record(text("in memory"), false, "here");
        assert!(!path.exists());
        history.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        history.flush();
        assert!(history.error.is_none(), "{:?}", history.error);
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600); }
        history.record(text("second"), false, "peer");
        history.flush();
        assert!(history.error.is_none());
        let mut loaded = History::default();
        loaded.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.list()[0].origin, "peer");
        loaded.clear();
        loaded.flush();
        let mut empty = History::default();
        empty.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
        assert!(empty.is_empty());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        loaded.configure(20, DEFAULT_BYTES, 1024, None);
        loaded.flush();
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_and_hostile_saved_data_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard-history.bin");
        for bytes in [b"garbage".to_vec(), [b"OMKH1".as_slice(), &u64::MAX.to_le_bytes()].concat()] {
            std::fs::write(&path, &bytes).unwrap();
            let mut history = History::default();
            history.configure(20, DEFAULT_BYTES, 1024, Some(path.clone()));
            assert!(history.error.is_some());
            history.record(text("later"), false, "here");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            history.clear();
            history.flush();
            assert!(history.error.is_none());
        }
        let mut encoded = Vec::new();
        encoded.extend_from_slice(b"OMKH1");
        for number in [1u64, 1, 0] { encoded.extend_from_slice(&number.to_le_bytes()); }
        write_field(&mut encoded, Some(b"here")).unwrap();
        write_field(&mut encoded, Some(&[0xff])).unwrap();
        write_field(&mut encoded, None).unwrap();
        write_field(&mut encoded, None).unwrap();
        std::fs::write(&path, &encoded).unwrap();
        let mut history = History::default();
        history.configure(20, DEFAULT_BYTES, 1024, Some(path));
        assert!(history.error.is_some());
        assert!(history.is_empty());
    }

    #[test]
    fn generated_saved_file_mutations_are_bounded_and_transactional() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard-history.bin");
        let mut original = History::default();
        original.configure(20, 1024, 1024, Some(path.clone()));
        original.record(text("sample 日本語"), false, "peer");
        original.flush();
        let bytes = std::fs::read(&path).unwrap();
        for position in 0..bytes.len() {
            for candidate in [bytes[..position].to_vec(), {
                let mut mutated = bytes.clone();
                mutated[position] ^= 0xff;
                mutated
            }] {
                std::fs::write(&path, &candidate).unwrap();
                let mut history = History::default();
                history.configure(20, 1024, 1024, Some(path.clone()));
                assert!(history.len() <= 20 && history.bytes <= 1024);
                if history.error.is_some() {
                    assert!(history.is_empty());
                    assert_eq!(std::fs::read(&path).unwrap(), candidate);
                }
            }
        }
    }

    #[test]
    fn generated_operations_preserve_all_bounds() {
        let mut history = History::default();
        history.configure(7, 100, 40, None);
        let mut random = 42u64;
        for _ in 0..10000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let length = (random % 50) as usize;
            history.record(text(&"x".repeat(length)), random.is_multiple_of(5), "peer");
            if random.is_multiple_of(13) { history.delete(random % 100); }
            if random.is_multiple_of(101) { history.clear(); }
            let items = history.list();
            assert!(items.len() <= 7);
            assert!(items.iter().map(|item| item.data.len()).sum::<usize>() <= 100);
            assert!(items.iter().all(|item| item.data.len() <= 40));
            assert_eq!(history.bytes, items.iter().map(|item| item.data.len()).sum::<usize>());
        }
    }
}
