//! Minimal panic marker. No panic payload, heap dump, raw paths or network in hook.
use super::*;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
const DIR: &str = "analytics-panic-v2";
include!(concat!(env!("OUT_DIR"), "/analytics_sources.rs"));
struct Armed { file: std::fs::File, signal: PathBuf, metadata: PathBuf }
impl Drop for Armed {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.signal); let _ = std::fs::remove_file(&self.metadata); }
}
static FILE: Mutex<Option<Armed>> = Mutex::new(None);
static HOOK: OnceLock<()> = OnceLock::new();
static WRITING: AtomicBool = AtomicBool::new(false);

pub(super) fn initialize(home: &Path, config: &Config, id: &str, origin: Origin) {
    recover(home, config);
    arm(home, config, id, origin);
}
pub(super) fn disarm() { if let Ok(mut file) = FILE.try_lock() { *file = None; } }
pub(super) fn purge(home: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(home.join(DIR)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), result => result,
    }
}
pub(super) fn arm(home: &Path, config: &Config, id: &str, origin: Origin) {
    if std::thread::panicking() { return; }
    let Ok(mut slot) = FILE.try_lock() else { return; };
    if slot.is_some() { return; }
    let dir = home.join(DIR);
    if crate::private::create_dir_all(&dir).is_err() { return; }
    if std::fs::read_dir(&dir).map(|r| r.take(65).count()).unwrap_or(65) >= 64 { return; }
    let item = spool::Stored::new(config, id, Event::PanicObserved, origin, Outcome::Failed, ErrorCode::Panic, true);
    let path = dir.join(&item.uuid);
    let Ok(bytes) = serde_json::to_vec(&item) else { return; };
    if crate::private::write(&path.with_extension("json"), bytes).is_err() { return; }
    let Ok(file) = crate::private::create_new(&path.with_extension("signal")) else { return; };
    *slot = Some(Armed { file, signal: path.with_extension("signal"), metadata: path.with_extension("json") });
    HOOK.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !WRITING.swap(true, Ordering::Relaxed) {
                // No allocation, payload inspection, blocking mutex or network.
                let mut marker = [0u8; 20];
                marker[12..].copy_from_slice(&now().to_le_bytes());
                if let Some(location) = info.location()
                    && let Some(index) = SOURCES.iter().position(|source| location.file().ends_with(source)) {
                        marker[..4].copy_from_slice(&((index + 1) as u32).to_le_bytes());
                        marker[4..8].copy_from_slice(&location.line().to_le_bytes());
                        marker[8..12].copy_from_slice(&location.column().to_le_bytes());
                }
                if let Ok(mut file) = FILE.try_lock() && let Some(file) = file.as_mut() { let _ = file.file.write_all(&marker); }
            }
            previous(info);
        }));
    });
}
pub(super) fn recover(home: &Path, config: &Config) {
    let dir = home.join(DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else { return; };
    for entry in entries.take(64).flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("signal") {
            // Reclaim orphan metadata/claims left by interruption after a write.
            if entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(24 * 60 * 60)) {
                let _ = std::fs::remove_file(path);
            }
            continue;
        }
        let meta = path.with_extension("json");
        let bytes = std::fs::File::open(&meta).ok().and_then(|f| { let mut b = Vec::new(); f.take(4097).read_to_end(&mut b).ok()?; (b.len() <= 4096).then_some(b) });
        let item = bytes.and_then(|b| serde_json::from_slice::<spool::Stored>(&b).ok());
        let mut signal = Vec::new();
        if std::fs::File::open(&path).and_then(|f| f.take(21).read_to_end(&mut signal)).is_err() { continue; }
        let Some(mut item) = item else { let _ = std::fs::remove_file(&path); let _ = std::fs::remove_file(meta); continue; };
        if signal.len() == 20 { item.occurred_at(u64::from_le_bytes(signal[12..].try_into().unwrap())); }
        if !item.permitted(home, config) { let _ = std::fs::remove_file(&path); let _ = std::fs::remove_file(meta); continue; }
        if signal.is_empty() { continue; } // An armed process may still be running.
        if signal.len() == 20 {
            let site = u32::from_le_bytes(signal[..4].try_into().unwrap());
            let line = u32::from_le_bytes(signal[4..8].try_into().unwrap());
            let column = u32::from_le_bytes(signal[8..12].try_into().unwrap());
            if site as usize <= SOURCES.len() && line <= 1_000_000 && column <= 100_000 { item.site = Some([site, line, column]); }
            let claim = path.with_extension("claimed");
            if std::fs::rename(&path, &claim).is_err() { continue; }
            if spool::append_items(home, config, vec![item]).is_err() { let _ = std::fs::rename(claim, path); continue; }
            let _ = std::fs::remove_file(claim);
        } else { let _ = std::fs::remove_file(&path); }
        let _ = std::fs::remove_file(meta);
    }
}
pub(super) fn manifest() -> String { SOURCE_MANIFEST.to_owned() }
pub(super) fn frame(site: [u32; 3]) -> Option<serde_json::Value> {
    if site[1] == 0 || site[1] > 1_000_000 || site[2] > 100_000 { return None; }
    let source = SOURCES.get(site[0].checked_sub(1)? as usize)?;
    Some(serde_json::json!({"platform": "custom", "lang": "rust", "filename": source, "lineno": site[1], "colno": site[2], "in_app": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHILD: &str = "ONE_MK_SYNTHETIC_PANIC_CHILD";
    #[test]
    fn isolated_panic_child() {
        let Ok(home) = std::env::var(CHILD) else { return; };
        // A synthetic child only: preserve abort semantics with no user app involved.
        std::panic::set_hook(Box::new(|_| std::process::abort()));
        super::super::initialize(Path::new(&home), Origin::Cli);
        panic!("SECRET_SENTINEL_private_url_and_content");
    }
    #[test]
    fn abort_marker_is_sanitized_recovers_once_and_respects_withdrawal() {
        let home = tempfile::tempdir().unwrap(); set_enabled(home.path(), true).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "analytics::crash::tests::isolated_panic_child"])
            .env_clear().env("HOME", home.path()).env("TMPDIR", home.path())
            .env(CHILD, home.path()).env("ONE_MK_POSTHOG_KEY", "phc_test_only")
            .env("ONE_MK_POSTHOG_HOST", "https://eu.i.posthog.com").output().unwrap();
        assert!(!output.status.success());
        let entries = std::fs::read_dir(home.path().join(DIR)).unwrap();
        for entry in entries.flatten() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("SECRET_SENTINEL"));
            assert!(!String::from_utf8_lossy(&bytes).contains("/Users/"));
        }
        let config = Config::new("phc_test_only", "https://eu.i.posthog.com").unwrap();
        recover(home.path(), &config);
        let spool = std::fs::read(home.path().join("analytics-spool-v2.json")).unwrap();
        let records: Vec<spool::Stored> = serde_json::from_slice(&spool).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&records[0].payload(&config)).unwrap()["event"], "$exception");
        let value: serde_json::Value = serde_json::from_slice(&records[0].payload(&config)).unwrap();
        assert_eq!(value["properties"]["$exception_list"][0]["stacktrace"]["frames"][0]["platform"], "custom");
        assert_eq!(value["properties"]["$exception_list"][0]["stacktrace"]["frames"][0]["filename"], "crates/ctl/src/analytics/crash.rs");
        recover(home.path(), &config);
        assert_eq!(std::fs::read(home.path().join("analytics-spool-v2.json")).unwrap(), spool);
        set_enabled(home.path(), false).unwrap();
        assert!(!home.path().join(DIR).exists());
        assert!(!home.path().join("analytics-spool-v2.json").exists());
    }
    #[test]
    fn frame_mapping_never_exports_arbitrary_paths() {
        assert!(frame([0, 10, 2]).is_none());
        assert!(frame([999, 10, 2]).is_none());
        assert!(frame([1, 0, 2]).is_none());
        assert!(frame([1, 1_000_001, 2]).is_none());
        assert!(frame([1, 10, 100_001]).is_none());
        assert_eq!(frame([1, 10, 2]).unwrap()["filename"], "crates/one-mk/src/main.rs");
        assert_eq!(frame([1, 10, 2]).unwrap()["platform"], "custom");
        assert_eq!(frame([1, 10, 2]).unwrap()["lang"], "rust");
    }
}
