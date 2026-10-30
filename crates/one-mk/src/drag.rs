#[cfg(test)]
mod tests {
    use super::*;

    struct Daemons { children: Vec<std::process::Child>, root: PathBuf }

    impl Drop for Daemons {
        fn drop(&mut self) {
            for child in &mut self.children { let _ = child.kill(); let _ = child.wait(); }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    #[ignore = "requires ONE_MK_TEST_BINARY and permission to bind private loopback sockets"]
    async fn two_null_daemons_receive_a_simulated_edge_drop() {
        use one_mk_net::Identity;
        use one_mk_proto::drag::{Drag, Change};
        use std::{net::UdpSocket, process::{Command, Stdio}, time::Duration};
        let binary = std::env::var_os("ONE_MK_TEST_BINARY").expect("build one-mk and set ONE_MK_TEST_BINARY");
        let root = std::env::temp_dir().join(format!("one-mk-edge-loop-{}", std::process::id()));
        let mut daemons = Daemons { children: vec![], root: root.clone() };
        let homes = [root.join("sender"), root.join("receiver")];
        for home in &homes { std::fs::create_dir_all(home).unwrap(); }
        let identities = homes.each_ref().map(|home| Identity::load_or_create(home).unwrap());
        let sockets = [UdpSocket::bind("127.0.0.1:0").unwrap(), UdpSocket::bind("127.0.0.1:0").unwrap()];
        let ports = sockets.each_ref().map(|socket| socket.local_addr().unwrap().port());
        for (index, home) in homes.iter().enumerate() {
            let other = 1 - index;
            std::fs::write(home.join("devices.toml"), format!(
                "[[device]]\nname = \"computer-{other}\"\nfingerprint = \"{}\"\naddrs = [\"127.0.0.1:{}\"]\n", identities[other].fingerprint, ports[other])).unwrap();
        }
        std::mem::drop(sockets);
        for (index, home) in homes.iter().enumerate() {
            let log = std::fs::File::create(root.join(format!("daemon-{index}.log"))).unwrap();
            let child = Command::new(&binary).arg("--home").arg(home)
                .args(["run", "--backend", "null", "--no-discovery", "--no-notify", "--no-clipboard", "--port"])
                .arg(ports[index].to_string()).arg("--downloads").arg(home.join("downloads"))
                .stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log)).spawn().unwrap();
            daemons.children.push(child);
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                for child in &mut daemons.children { assert!(child.try_wait().unwrap().is_none(), "null daemon exited early"); }
                let mut ready = true;
                for home in &homes {
                    ready &= crate::control::request(home, crate::control::Request::Status).await.ok().flatten().is_some();
                }
                if ready { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.expect("null daemons ready");
        let file = root.join("edge-drop.txt");
        let contents = b"edge drop through the existing resumable QUIC transfer\n";
        std::fs::write(&file, contents).unwrap();
        let mut state = Drag::default();
        assert_eq!(state.pointer(true, Some(0)), Change::Show(0));
        assert_eq!(state.drop_on(0), Some(0));
        crate::notify::disable();
        send(&homes[0], vec![file], identities[1].fingerprint.to_string()).await.unwrap();
        assert_eq!(std::fs::read(homes[1].join("downloads/edge-drop.txt")).unwrap(), contents);
        assert_eq!(state.drop_on(0), None);
        println!("PASS: two real --backend null daemons; armed edge -> simulated drop -> safe intake -> QUIC -> receiver Downloads; {} bytes verified", contents.len());
    }

    #[test]
    fn safety_rejects_missing_special_and_symlink_paths() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("one-mk-drop-safety-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("hello.txt");
        std::fs::write(&file, "hello").unwrap();
        assert_eq!(safe_paths(vec![file.clone(), file.clone()]).unwrap(), vec![file]);
        assert!(safe_paths(vec![root.join("missing")]).is_err());
        assert!(safe_paths(vec![PathBuf::from("relative")]).is_err());
        #[cfg(unix)]
        {
            let link = root.join("escape");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            assert!(safe_paths(vec![link.join("hello.txt")]).is_err());
            assert!(safe_paths(vec![root.clone()]).is_err());
            assert!(safe_paths(vec![PathBuf::from("/dev/null")]).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn safety_bounds_counts_size_depth_and_empty_folders() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("one-mk-drop-bounds-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(safe_paths(vec![]).is_err());
        assert!(safe_paths(vec![root.clone(); MAX_FILES + 1]).is_err());
        assert!(safe_paths(vec![root.clone()]).is_err());
        let file = root.join("file");
        std::fs::write(&file, b"hello").unwrap();
        assert_eq!(safe_paths(vec![root.clone(), file.clone()]).unwrap(), vec![root.clone()]);
        assert!(safe_paths(vec![root.join("../file")]).is_err());
        let mut budget = Budget { files: MAX_FILES, ..Default::default() };
        assert!(inspect(&file, 0, &mut budget).is_err());
        let mut budget = Budget { bytes: MAX_BYTES, ..Default::default() };
        assert!(inspect(&file, 0, &mut budget).is_err());
        assert!(inspect(&root, 65, &mut Budget::default()).is_err());
        let mut budget = Budget { entries: 20_000, ..Default::default() };
        assert!(inspect(&root, 0, &mut budget).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
use anyhow::{Result, bail, Context};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 1 << 40;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub zone_px: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: one_mk_input::dropzone::available_by_default(), zone_px: 12 }
    }
}

pub async fn configure(dir: &Path, on: bool, off: bool, zone: Option<u16>) -> Result<()> {
    if zone.is_some_and(|width| !(1..=128).contains(&width)) {
        bail!("drop-zone width must be between 1 and 128 pixels");
    }
    if on || off || zone.is_some() {
        crate::settings::Settings::update(dir, |settings| {
            if on || off { settings.drag.enabled = on; }
            if let Some(width) = zone { settings.drag.zone_px = width; }
            Ok(())
        })?;
        let _ = crate::control::request(dir, crate::control::Request::Settings).await?;
    }
    let settings = crate::settings::Settings::load_or_default(dir).drag;
    println!("File edge drops: {}; zone {} px. {}", if settings.enabled { "on" } else { "off" }, settings.zone_px,
        one_mk_input::dropzone::availability());
    Ok(())
}

pub fn safe_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    if paths.is_empty() || paths.len() > MAX_FILES { bail!("drop must contain 1 to 10000 local paths"); }
    let mut unique = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) {
            bail!("drop paths must be absolute without parent traversal");
        }
        #[cfg(windows)]
        if !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        { bail!("network and device paths are not accepted in file drops"); }
        check_ancestors(&path)?;
        let path = std::fs::canonicalize(path)?;
        if seen.insert(path.clone()) { unique.push(path); }
    }
    unique.sort();
    let roots: Vec<_> = unique.iter().filter(|path| !unique.iter().any(|other| other != *path && path.starts_with(other))).cloned().collect();
    let mut budget = Budget::default();
    for path in &roots { inspect(path, 0, &mut budget)?; }
    let files = crate::transfer::collect(&roots)?;
    if files.is_empty() { bail!("nothing to send (the folders are empty)"); }
    if files.len() > MAX_FILES || files.iter().try_fold(0u64, |sum, file| sum.checked_add(file.size)).is_none_or(|sum| sum > MAX_BYTES) {
        bail!("file batch exceeds the limit of 10000 files or 1 TiB");
    }
    for file in files {
        check_ancestors(&file.path)?;
        crate::transfer::sanitize(&file.rel).map_err(anyhow::Error::msg)?;
        if file.rel.len() > one_mk_proto::FileMsg::MAX_PATH { bail!("drop path is too long"); }
    }
    Ok(roots)
}

fn check_ancestors(path: &Path) -> Result<()> {
    for parent in path.ancestors() {
        if redirected(&std::fs::symlink_metadata(parent)?) {
            bail!("symlinks are not accepted in dropped paths: {}", parent.display());
        }
    }
    Ok(())
}

fn redirected(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

#[derive(Default)]
struct Budget { files: usize, bytes: u64, entries: usize }

fn inspect(path: &Path, depth: usize, budget: &mut Budget) -> Result<()> {
    budget.entries += 1;
    if depth > 64 || budget.entries > 20_000 { bail!("drop folder tree is too large or deep"); }
    let metadata = std::fs::symlink_metadata(path)?;
    if redirected(&metadata) { bail!("drop contains a symlink or reparse point"); }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? { inspect(&entry?.path(), depth + 1, budget)?; }
    } else if metadata.is_file() {
        budget.files += 1;
        budget.bytes = budget.bytes.checked_add(metadata.len()).context("drop size overflow")?;
        if budget.files > MAX_FILES || budget.bytes > MAX_BYTES { bail!("file batch exceeds the limit of 10000 files or 1 TiB"); }
        crate::transfer::open_regular(path, metadata.len()).with_context(|| format!("cannot read {}", path.display()))?;
    } else { bail!("only regular files and directories may be dropped: {}", path.display()); }
    Ok(())
}

pub async fn send(dir: &Path, paths: Vec<PathBuf>, peer: String) -> Result<()> {
    let paths = tokio::task::spawn_blocking(move || safe_paths(paths)).await??;
    crate::send::dropped(dir, paths, peer).await
}
