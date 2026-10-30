use crate::Media;
#[cfg(target_os = "linux")]
use crate::{MAX_SOURCE, decode::Decoded, sandbox};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;

pub struct ImportOptions<'a> {
    pub source: &'a Path,
    pub output: &'a Path,
    pub runtime: &'a Path,
    pub cancel: &'a AtomicBool,
    pub timeout: std::time::Duration,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRecord {
    pub schema: u32,
    pub source_blake3: String,
    pub source_bytes: u64,
    pub media: Media,
    pub poster_blake3: String,
    pub poster_file: String,
    pub timeline: String,
}

#[cfg(target_os = "linux")]
fn open_path(path: &Path, directory: bool) -> Result<std::fs::File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let absolute = if path.is_absolute() { path.to_owned() } else { std::env::current_dir().map_err(|error| error.to_string())?.join(path) };
    let components: Vec<_> = absolute.components().collect();
    let mut current = std::fs::File::open("/").map_err(|error| error.to_string())?;
    for (index, component) in components.iter().enumerate().skip(1) {
        let std::path::Component::Normal(name) = component else { return Err("dot/parent path components forbidden".into()); };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| "NUL in path")?;
        let is_directory = directory || index + 1 != components.len();
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK | if is_directory { libc::O_DIRECTORY } else { 0 };
        let descriptor = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 { return Err(format!("path must exist without symlinks: {}", std::io::Error::last_os_error())); }
        current = unsafe { std::fs::File::from_raw_fd(descriptor) };
    }
    Ok(current)
}

#[cfg(target_os = "linux")]
pub fn import(options: ImportOptions<'_>) -> Result<ImportRecord, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if options.cancel.load(Ordering::Relaxed) { return Err("import cancelled".into()); }
    let runtime = sandbox::validate_runtime(options.runtime).map_err(|error| format!("sandbox unavailable: refusing to import: {error}"))?;
    let available = sandbox::doctor(Some(&runtime));
    if !available.available { return Err(available.reason); }
    if options.cancel.load(Ordering::Relaxed) { return Err("import cancelled".into()); }
    let output = open_path(options.output, true)?;
    let metadata = output.metadata().map_err(|error| error.to_string())?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 { return Err("output directory must be owned with mode 0700".into()); }
    if unsafe { libc::flock(output.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 { return Err("another import holds this output directory; retry when it finishes".into()); }
    let directory = std::path::PathBuf::from(format!("/proc/self/fd/{}", output.as_raw_fd()));
    for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())?.take(4096) {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.file_name().as_encoded_bytes().starts_with(b".import-") { continue; }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o777 == 0o700 {
            std::fs::remove_dir_all(entry.path()).map_err(|error| format!("stale partial cleanup: {error}"))?;
        }
    }
    let staging = tempfile::Builder::new().prefix(".import-").tempdir_in(&directory).map_err(|error| error.to_string())?;
    std::fs::set_permissions(staging.path(), std::fs::Permissions::from_mode(0o700)).map_err(|error| error.to_string())?;
    let mut source = open_path(options.source, false)?;
    let metadata = source.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE { return Err("source must be a regular file <=512 MiB".into()); }
    let mut snapshot = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(staging.path().join("source")).map_err(|error| error.to_string())?;
    let mut hash = blake3::Hasher::new();
    let mut source_bytes = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        if options.cancel.load(Ordering::Relaxed) { return Err("import cancelled".into()); }
        let count = source.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 { break; }
        source_bytes += count as u64;
        if source_bytes > MAX_SOURCE { return Err("source grew beyond 512 MiB".into()); }
        snapshot.write_all(&buffer[..count]).map_err(|error| error.to_string())?;
        hash.update(&buffer[..count]);
    }
    snapshot.sync_all().map_err(|error| error.to_string())?;
    drop(snapshot);
    let source_blake3 = hash.finalize().to_hex().to_string();
    let response = sandbox::run(&runtime, Some(&staging.path().join("source")), "--worker", options.cancel, options.timeout)?;
    let decoded: Decoded = serde_json::from_slice(&response).map_err(|error| format!("invalid worker record: {error}"))?;
    validate_decoded(&decoded)?;
    let verified = hash_file(&staging.path().join("source"))?;
    if verified != source_blake3 { return Err("snapshot changed during import".into()); }
    let record = ImportRecord {
        schema: 1, source_blake3: source_blake3.clone(), source_bytes,
        media: decoded.media, poster_blake3: blake3::hash(&decoded.poster).to_hex().to_string(),
        poster_file: "poster.ppm".into(), timeline: "validated-source; normalization/playback NOT qualified".into(),
    };
    let record_bytes = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
    if source_bytes + decoded.poster.len() as u64 + record_bytes.len() as u64 > MAX_SOURCE { return Err("validation: total output exceeds 512 MiB".into()); }
    write_new(&staging.path().join("poster.ppm"), &decoded.poster)?;
    write_new(&staging.path().join("record.json"), &record_bytes)?;
    std::fs::File::open(staging.path()).and_then(|file| file.sync_all()).map_err(|error| error.to_string())?;
    if options.cancel.load(Ordering::Relaxed) { return Err("import cancelled before commit".into()); }
    let old_name = std::ffi::CString::new(staging.path().file_name().ok_or("missing staging name")?.as_encoded_bytes()).map_err(|_| "invalid staging name")?;
    let new_name = std::ffi::CString::new(source_blake3).map_err(|_| "invalid content hash")?;
    if unsafe { libc::renameat2(output.as_raw_fd(), old_name.as_ptr(), output.as_raw_fd(), new_name.as_ptr(), libc::RENAME_NOREPLACE) } != 0 { return Err(format!("atomic commit refused (existing object/symlink is never replaced): {}", std::io::Error::last_os_error())); }
    output.sync_all().map_err(|error| format!("committed, directory sync failed: {error}"))?;
    Ok(record)
}

#[cfg(target_os = "linux")]
fn validate_decoded(decoded: &Decoded) -> Result<(), String> {
    crate::admission::validate_media(&decoded.media)?;
    let (width, height) = crate::decode::poster_dimensions(decoded.media.width, decoded.media.height);
    let header = format!("P6\n{width} {height}\n255\n");
    if !decoded.poster.starts_with(header.as_bytes()) || decoded.poster.len() != header.len() + width as usize * height as usize * 3 { return Err("worker returned invalid poster header/dimensions/payload".into()); }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn record() -> Decoded {
        Decoded { media: Media { format: crate::Format::Gif, width: 2, height: 2, frames: 2, duration_ms: 200 }, poster: b"P6\n2 2\n255\nabcdefghijkl".to_vec() }
    }

    #[test]
    fn parent_rejects_forged_poster_headers_and_lengths() {
        assert!(validate_decoded(&record()).is_ok());
        for poster in [b"P6\n".as_slice(), b"P6\n99999 2\n255\n", b"P6\n2 2\n256\nabcdefghijkl", b"P6\n2 2\n255\nabcdefghijk", b"P6\n2 2\n255\nabcdefghijklm", b"P6\n1 1\n255\nabc"] {
            let mut decoded = record();
            decoded.poster = poster.to_vec();
            assert!(validate_decoded(&decoded).is_err(), "{poster:?}");
        }
    }

    #[test]
    fn parent_rejects_worker_fps_and_pixel_budget() {
        let mut decoded = record();
        decoded.media.duration_ms = 1;
        assert!(validate_decoded(&decoded).is_err());
        decoded.media.duration_ms = 60_000;
        decoded.media.width = 4096;
        decoded.media.height = 2160;
        decoded.media.frames = 114;
        assert!(validate_decoded(&decoded).is_err());
    }
}

#[cfg(target_os = "linux")]
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn hash_file(path: &Path) -> Result<String, String> {
    let mut source = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn import(_options: ImportOptions<'_>) -> Result<ImportRecord, String> {
    Err("sandbox unavailable: refusing to import; Windows AppContainer/job or macOS signed helper is UNTESTED and not implemented".into())
}
