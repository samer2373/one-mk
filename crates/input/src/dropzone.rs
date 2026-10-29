//! OS-owned edge drop targets. No clipboard polling and no input injection.

use anyhow::{Result, bail};
use one_mk_proto::Rect;
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};

#[cfg(target_os = "linux")]
#[path = "dropzone_x11.rs"]
mod x11;
#[cfg(windows)]
#[path = "dropzone_windows.rs"]
mod win;
#[cfg(target_os = "macos")]
#[path = "dropzone_macos.rs"]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::with_main_loop;

pub const MAX_URI_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Zone { pub rect: Rect, pub peer: String }

#[derive(Debug)]
pub struct Drop { pub peer: String, pub paths: Vec<PathBuf> }

pub trait DropTarget {
    fn set_zones(&self, zones: Vec<Zone>);
}

pub struct Handle { zones: watch::Sender<Vec<Zone>> }

impl DropTarget for Handle {
    fn set_zones(&self, zones: Vec<Zone>) {
        self.zones.send_if_modified(|current| {
            if *current == zones { false } else { *current = zones; true }
        });
    }
}

pub fn available_by_default() -> bool {
    cfg!(any(windows, target_os = "macos")) || (cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some())
}

pub fn availability() -> &'static str {
    if cfg!(windows) { "Available with the Windows OLE edge target (interactive desktop only)." }
    else if cfg!(target_os = "macos") { "Available with the AppKit edge target (Accessibility/Input Monitoring permission required)." }
    else if available_by_default() { "Available on X11." }
    else { "File edge drops are not available on this desktop; use one-mk send or the file manager's Send to entry." }
}

pub fn start(backend: &str) -> Result<(Handle, mpsc::Receiver<Drop>)> {
    let (zones, changed) = watch::channel(Vec::new());
    let (dropped, events) = mpsc::channel(4);
    #[cfg(target_os = "macos")]
    if backend == "native" {
        mac::start(changed, dropped)?;
        return Ok((Handle { zones }, events));
    }
    #[cfg(windows)]
    if backend == "native" {
        win::start(changed, dropped)?;
        return Ok((Handle { zones }, events));
    }
    #[cfg(target_os = "linux")]
    if backend == "x11" && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        x11::start(changed, dropped)?;
        return Ok((Handle { zones }, events));
    }
    let _ = (backend, changed, dropped);
    Ok((Handle { zones }, events))
}

pub fn uri_list(bytes: &[u8]) -> Result<Vec<PathBuf>> {
    if bytes.len() > MAX_URI_BYTES { bail!("drop URI list exceeds 1 MiB"); }
    let text = std::str::from_utf8(bytes)?;
    let mut paths = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty() && !line.starts_with('#')) {
        if paths.len() == 10_000 { bail!("too many dropped paths"); }
        let uri = line.strip_prefix("file://").ok_or_else(|| anyhow::anyhow!("only local file URLs are accepted"))?;
        let path = if uri.starts_with('/') { uri } else {
            uri.strip_prefix("localhost/").map(|rest| &uri[uri.len() - rest.len() - 1..])
                .ok_or_else(|| anyhow::anyhow!("remote file URL host is not accepted"))?
        };
        if path.contains(['?', '#', '\0', '\\']) { bail!("invalid local file URL"); }
        let mut decoded = Vec::with_capacity(path.len());
        let mut input = path.as_bytes().iter().copied();
        while let Some(byte) = input.next() {
            let byte = if byte == b'%' {
                let high = input.next().and_then(|byte| (byte as char).to_digit(16));
                let low = input.next().and_then(|byte| (byte as char).to_digit(16));
                match (high, low) { (Some(high), Some(low)) => (high * 16 + low) as u8, _ => bail!("invalid percent escape") }
            } else { byte };
            if byte == 0 { bail!("NUL in file URL"); }
            decoded.push(byte);
        }
        #[cfg(unix)]
        let path = { use std::os::unix::ffi::OsStringExt; PathBuf::from(std::ffi::OsString::from_vec(decoded)) };
        #[cfg(not(unix))]
        let path = PathBuf::from(String::from_utf8(decoded)?.trim_start_matches('/'));
        paths.push(path);
    }
    if paths.is_empty() { bail!("empty file drop"); }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_intake_is_local_bounded_and_strict() {
        assert!(uri_list(b"# a comment\r\nfile:///tmp/hello%20world\r\n").is_ok());
        assert!(uri_list(b"file://localhost/tmp/a").is_ok());
        for bad in ["file://server/a", "https://example.com/a", "file:///a%", "file:///a%XZ", "file:///a%00", "file:///a?query", "file:///a#part", ""] {
            assert!(uri_list(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(uri_list(&vec![b'a'; MAX_URI_BYTES + 1]).is_err());
    }
}
