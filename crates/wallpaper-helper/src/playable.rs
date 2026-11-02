//! AVFoundation (macOS) and Media Foundation (Windows) choose how to read a
//! file from its extension, and the wallpaper cache names every video
//! `<hash>.video`: opened as it is, no video plays ("no playable video
//! track"). What they open instead is a hard link named with the type the
//! file's first bytes show, in a folder beside the cache: no copy, no extra
//! space, and the cache's own size limits never count it twice.

use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The extension for a video whose file starts with `head`.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some(if &head[8..12] == b"qt  " { "mov" } else { "mp4" });
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(if head.windows(4).any(|window| window == b"webm") { "webm" } else { "mkv" });
    }
    // Older QuickTime files start with an atom other than `ftyp`.
    if head.len() >= 8 && matches!(&head[4..8], b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot") { return Some("mov"); }
    None
}

/// The path to open for `path`: itself unless it is a cached `.video`.
pub fn path(path: &Path) -> Result<PathBuf> {
    if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("video")) { return Ok(path.to_owned()); }
    let mut head = [0u8; 64];
    let read = fs::File::open(path).context("The wallpaper video is missing")?.read(&mut head)?;
    let Some(extension) = sniff(&head[..read]) else { return Ok(path.to_owned()) };
    let cache = path.parent().context("The wallpaper video has no folder")?;
    let links = one_mk_wallpaper::links_dir(cache);
    fs::create_dir_all(&links)?;
    // Links to videos the cache no longer has: their wallpaper is gone.
    for entry in fs::read_dir(&links)?.flatten() {
        let link = entry.path();
        let kept = link.file_stem().is_some_and(|stem| cache.join(stem).with_extension("video").is_file());
        if !kept { let _ = fs::remove_file(link); }
    }
    let stem = path.file_stem().context("The wallpaper video has no name")?;
    let link = links.join(stem).with_extension(extension);
    if !same_file(&link, path) {
        let _ = fs::remove_file(&link);
        fs::hard_link(path, &link).context("Couldn't open the wallpaper video")?;
    }
    Ok(link)
}

fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(a), Ok(b)) = (fs::metadata(a), fs::metadata(b)) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    { a.len() == b.len() && a.modified().ok() == b.modified().ok() && a.created().ok() == b.created().ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_containers_players_open() {
        let mp4 = [0, 0, 0, 24, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm'];
        let mov = [0, 0, 0, 20, b'f', b't', b'y', b'p', b'q', b't', b' ', b' '];
        let old_mov = [0, 0, 0, 8, b'w', b'i', b'd', b'e'];
        let mut webm = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x82, 0x84];
        webm.extend_from_slice(b"webm");
        let mut mkv = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x82, 0x88];
        mkv.extend_from_slice(b"matroska");
        assert_eq!(sniff(&mp4), Some("mp4"));
        assert_eq!(sniff(&mov), Some("mov"));
        assert_eq!(sniff(&old_mov), Some("mov"));
        assert_eq!(sniff(&webm), Some("webm"));
        assert_eq!(sniff(&mkv), Some("mkv"));
        assert_eq!(sniff(b"not a video at all"), None);
        assert_eq!(sniff(&[]), None);
    }

    #[test]
    fn a_cached_video_opens_through_a_typed_link_and_stale_links_go() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join("wallpaper-video");
        fs::create_dir(&cache).unwrap();
        let hash = "a".repeat(64);
        let video = cache.join(format!("{hash}.video"));
        fs::write(&video, [0, 0, 0, 24, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 1, 2, 3]).unwrap();
        let links = home.path().join("wallpaper-links");
        fs::create_dir(&links).unwrap();
        let stale = links.join(format!("{}.mp4", "b".repeat(64)));
        fs::write(&stale, b"gone").unwrap();

        let opened = path(&video).unwrap();
        assert_eq!(opened, links.join(format!("{hash}.mp4")));
        assert_eq!(fs::read(&opened).unwrap(), fs::read(&video).unwrap());
        assert!(!stale.exists());
        // Again: the same link, not a new one.
        assert_eq!(path(&video).unwrap(), opened);
        assert!(same_file(&opened, &video));

        // Anything that isn't a cached `.video` opens as it is.
        let original = home.path().join("holiday.mov");
        fs::write(&original, b"x").unwrap();
        assert_eq!(path(&original).unwrap(), original);
    }
}
