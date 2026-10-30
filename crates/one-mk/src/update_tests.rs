use super::*;
use ring::signature::KeyPair;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::AtomicBool;

struct Server { address: SocketAddr, stop: Arc<AtomicBool>, thread: Option<std::thread::JoinHandle<()>> }
impl Server {
    fn new(routes: impl FnOnce(&Url) -> Result<HashMap<String, Vec<u8>>>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let routes = routes(&Url::parse(&format!("http://{address}/latest"))?)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            for incoming in listener.incoming() {
                if stopped.load(Ordering::Acquire) { break; }
                if let Ok(mut socket) = incoming {
                    let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request = Vec::new();
                    while request.len() < 4096 && !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        if socket.read_exact(&mut byte).is_err() { break; }
                        request.push(byte[0]);
                    }
                    if !request.is_empty() {
                        let path = std::str::from_utf8(&request).ok().and_then(|text| text.split_whitespace().nth(1));
                        if let Some(response) = path.and_then(|path| routes.get(path)) { let _ = socket.write_all(response); }
                    }
                }
            }
        });
        Ok(Self { address, stop, thread: Some(thread) })
    }
    fn origin(&self) -> Result<Url> { Ok(Url::parse(&format!("http://{}/latest", self.address))?) }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("one-mk-update-test-{}-{}", std::process::id(), TEMP_ID.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

fn pair() -> Result<signature::Ed25519KeyPair> {
    signature::Ed25519KeyPair::from_seed_unchecked(&policy::hex_bytes::<32>("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")?)
        .map_err(|_| anyhow::anyhow!("test key invalid"))
}

fn response(bytes: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bytes.len()).into_bytes();
    response.extend(bytes);
    response
}

fn fixture(bytes: &[u8]) -> Manifest {
    Manifest { version: "0.1.13".into(), release_time: "2026-10-30T00:00:00Z".into(), sequence: u64::MAX - 1,
        minimum_supported_version: "0.1.0".into(), assets: vec![policy::Asset { name: "portable.bin".into(), size: bytes.len() as u64,
            sha256: hex(digest::digest(&digest::SHA256, bytes).as_ref()), os: std::env::consts::OS.into(), arch: std::env::consts::ARCH.into(), kind: "binary".into() }],
        revoked: vec![], next_key: None }
}

#[test]
fn signatures_refuse_every_mutation() -> Result<()> {
    let pair = pair()?;
    let message = b"signed release manifest";
    let signed = pair.sign(message);
    verify(pair.public_key().as_ref(), message, signed.as_ref())?;
    for index in 0..signed.as_ref().len() {
        let mut changed = signed.as_ref().to_vec();
        changed[index] ^= 1;
        assert!(verify(pair.public_key().as_ref(), message, &changed).is_err());
        assert!(verify(pair.public_key().as_ref(), message, &signed.as_ref()[..index]).is_err());
    }
    assert!(verify(&[0; 32], message, signed.as_ref()).is_err());
    assert!(verify(&[], message, signed.as_ref()).is_err());
    assert!(verify(pair.public_key().as_ref(), b"tampered", signed.as_ref()).is_err());
    assert!(verify(pair.public_key().as_ref(), &[], signed.as_ref()).is_err());
    Ok(())
}

#[test]
fn local_signed_check_and_state_replay() -> Result<()> {
    let scratch = Scratch::new()?;
    let pair = pair()?;
    let bytes = serde_json::to_vec(&fixture(b"stand-in"))?;
    let signed = pair.sign(&bytes);
    let server = Server::new(|origin| {
        let api = serde_json::json!({"tag_name":"v0.1.13", "assets":[
            {"name":"update-manifest.json", "browser_download_url":origin.join("/manifest")?.as_str()},
            {"name":"update-manifest.json.sig", "browser_download_url":origin.join("/signature")?.as_str()}
        ]});
        Ok(HashMap::from([
            ("/latest".into(), response(&serde_json::to_vec(&api)?)),
            ("/manifest".into(), response(&bytes)), ("/signature".into(), response(signed.as_ref())),
        ]))
    })?;
    let endpoint = server.origin()?;
    let mut state = State::default();
    checked(&scratch.0, endpoint.as_str(), pair.public_key().as_ref(), "0.1.12", &mut state)?;
    assert_eq!(load_state(&scratch.0)?.available.as_deref(), Some("0.1.13"));
    checked(&scratch.0, endpoint.as_str(), pair.public_key().as_ref(), "0.1.12", &mut state)?;
    assert!(checked(&scratch.0, endpoint.as_str(), &[0; 32], "0.1.12", &mut state).is_err());
    state.sequence = u64::MAX;
    assert!(checked(&scratch.0, endpoint.as_str(), pair.public_key().as_ref(), "0.1.12", &mut state).is_err());
    Ok(())
}

#[test]
fn local_download_replace_rollback_and_failures() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new()?;
    let target = scratch.0.join("one-mk");
    let original = b"#!/bin/sh\necho 'one-mk 0.1.12'\n";
    let candidate = b"#!/bin/sh\necho 'one-mk 0.1.13'\n";
    fs::write(&target, original)?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    let server = Server::new(|_| Ok(HashMap::from([("/binary".into(), response(candidate))])))?;
    let origin = server.origin()?;
    let release = Release { tag_name: "v0.1.13".into(), assets: vec![ReleaseAsset { name: "portable.bin".into(), browser_download_url: origin.join("/binary")?.to_string() }] };
    apply(&fixture(candidate), &release, &origin, &target, InstallKind::Portable)?;
    assert_eq!(fs::read(&target)?, candidate);
    assert_eq!(fs::read(scratch.0.join("one-mk.old"))?, original);
    rollback(&target)?;
    assert_eq!(fs::read(&target)?, original);
    let mut bad = fixture(candidate);
    bad.assets[0].sha256 = "00".repeat(32);
    assert!(apply(&bad, &release, &origin, &target, InstallKind::Portable).is_err());
    assert_eq!(fs::read(&target)?, original);
    bad = fixture(candidate);
    bad.assets[0].size -= 1;
    assert!(apply(&bad, &release, &origin, &target, InstallKind::Portable).is_err());
    assert_eq!(fs::read(&target)?, original);
    assert!(!fs::read_dir(&scratch.0)?.filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with(".one-mk-update-")));
    Ok(())
}

#[test]
fn interrupted_download_and_redirect_refused() -> Result<()> {
    let mut interrupted = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n".to_vec();
    interrupted.extend(b"tiny");
    let server = Server::new(|_| Ok(HashMap::from([
        ("/latest".into(), interrupted),
        ("/redirect".into(), b"HTTP/1.1 302 Found\r\nLocation: https://attacker.invalid/payload\r\n\r\n".to_vec()),
        ("/chunk".into(), b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\ntest\r\n0\r\n\r\n".to_vec()),
    ])))?;
    let origin = server.origin()?;
    assert!(fetch(&origin, &origin, 8).is_err());
    assert!(fetch(&origin.join("/redirect")?, &origin, 8).is_err());
    assert_eq!(fetch(&origin.join("/chunk")?, &origin, 4)?, b"test");
    assert!(validate_url(&Url::parse("http://github.com/release")?).is_err());
    assert!(validate_url(&Url::parse("https://user:pass@github.com/release")?).is_err());
    Ok(())
}

#[test]
fn subprocess_apply() -> Result<()> {
    let Some(target) = std::env::var_os("ONE_MK_UPDATE_TEST_TARGET") else { return Ok(()); };
    let target = PathBuf::from(target);
    let parent = target.parent().context("missing test directory")?;
    ensure!(parent.parent() == Some(std::env::temp_dir().as_path()) && parent.file_name().is_some_and(|name| name.to_string_lossy().starts_with("one-mk-update-test-")), "test target must be in a throwaway updater fixture");
    let origin = Url::parse(&std::env::var("ONE_MK_UPDATE_TEST_URL")?)?;
    ensure!(origin.host_str() == Some("127.0.0.1"), "test server must be loopback");
    let release = Release { tag_name: "v0.1.13".into(), assets: vec![ReleaseAsset { name: "portable.bin".into(), browser_download_url: origin.join("/binary")?.to_string() }] };
    apply(&fixture(&[0; 100]), &release, &origin, &target, InstallKind::Portable)
}

#[test]
fn hostile_http_framing_is_refused() -> Result<()> {
    let malformed = [
        "HTTP/1.1 nope\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx",
        "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nfffffffffffffffff\r\n",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx!!",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n",
        "HTTP/1.1 302 Found\r\nLocation: https://attacker.invalid/\r\n\r\n",
    ];
    for response in malformed {
        let server = Server::new(|_| Ok(HashMap::from([("/latest".into(), response.as_bytes().to_vec())])))?;
        let origin = server.origin()?;
        assert!(fetch(&origin, &origin, 8).is_err(), "accepted {response:?}");
    }
    let valid = b"HTTP/1.1 200 OK\r\n";
    for length in 0..valid.len() {
        let mut input = std::io::Cursor::new(&valid[..length]);
        assert!(line(&mut input, &mut 32).is_err());
    }
    for value in 0..=u8::MAX {
        let bytes = vec![value; 65];
        let mut input = std::io::Cursor::new(bytes);
        assert!(line(&mut input, &mut 64).is_err());
    }
    Ok(())
}

#[test]
fn kill_mid_download_preserves_original_and_cleans_on_retry() -> Result<()> {
    let scratch = Scratch::new()?;
    let target = scratch.0.join("one-mk");
    fs::write(&target, b"original executable is never replaced")?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}/latest", listener.local_addr()?);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        loop {
            let mut header = String::new();
            reader.read_line(&mut header)?;
            if header == "\r\n" || header.is_empty() { break; }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npart")?;
        let _ = ready_tx.send(());
        let _ = finish_rx.recv_timeout(Duration::from_secs(10));
        Ok(())
    });
    let mut child = Command::new(std::env::current_exe()?).args(["--exact", "update::tests::subprocess_apply", "--nocapture"])
        .env("ONE_MK_UPDATE_TEST_TARGET", &target).env("ONE_MK_UPDATE_TEST_URL", origin)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let ready = ready_rx.recv_timeout(Duration::from_secs(10));
    let killed = child.kill();
    let _ = child.wait();
    let _ = finish_tx.send(());
    server.join().map_err(|_| anyhow::anyhow!("local server failed"))??;
    ready?;
    killed?;
    assert_eq!(fs::read(&target)?, b"original executable is never replaced");
    let _lock = target_lock(&target)?;
    assert!(!fs::read_dir(&scratch.0)?.filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with(".one-mk-update-payload-")));
    Ok(())
}

#[test]
fn wrong_version_failed_execution_and_recovery_are_safe() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new()?;
    let original = b"#!/bin/sh\necho 'one-mk 0.1.12'\n";
    let target = scratch.0.join("one-mk");
    fs::write(&target, original)?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    for bytes in [b"#!/bin/sh\nexit 1\n".as_slice(), b"#!/bin/sh\necho wrong\n", b"#!/bin/sh\necho 'one-mk 0.1.14'\n"] {
        let server = Server::new(|_| Ok(HashMap::from([("/binary".into(), response(bytes))])))?;
        let origin = server.origin()?;
        let release = Release { tag_name: "v0.1.13".into(), assets: vec![ReleaseAsset { name: "portable.bin".into(), browser_download_url: origin.join("/binary")?.to_string() }] };
        assert!(apply(&fixture(bytes), &release, &origin, &target, InstallKind::Portable).is_err());
        assert_eq!(fs::read(&target)?, original);
    }
    fs::rename(&target, scratch.0.join("one-mk.old"))?;
    rollback(&target)?;
    assert_eq!(fs::read(&target)?, original);
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn hung_version_probe_terminates_its_process_group() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new()?;
    let target = scratch.0.join("probe");
    let child_id = scratch.0.join("child-id");
    fs::write(&target, format!("#!/bin/sh\nsleep 30 &\necho $! > '{}'\nwait\n", child_id.display()))?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    assert!(version_check(&target, Some("0.1.13")).is_err());
    let process_id: u32 = fs::read_to_string(child_id)?.trim().parse()?;
    for _ in 0..100 {
        match fs::read_to_string(format!("/proc/{process_id}/stat")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Ok(status) if status.split_whitespace().nth(2) == Some("Z") => return Ok(()),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    bail!("version probe left a running descendant")
}

#[test]
fn downloads_may_only_come_from_the_origin_or_github() -> Result<()> {
    let origin = Url::parse("https://api.github.com/repos/o/r/releases/latest")?;
    for ok in ["https://api.github.com/x", "https://github.com/o/r/releases/download/v1/a", "https://objects.githubusercontent.com/a",
        "https://release-assets.githubusercontent.com/a"] {
        assert!(allowed(&Url::parse(ok)?, &origin), "{ok}");
    }
    for bad in ["https://attacker.example/a", "https://github.com.attacker.example/a", "http://github.com/a", "https://github.com:8443/a",
        "https://evil.githubusercontent.com/a", "https://api.github.com:444/x"] {
        assert!(!allowed(&Url::parse(bad)?, &origin), "{bad}");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlink_is_never_an_update_target_or_backup() -> Result<()> {
    let scratch = Scratch::new()?;
    let real = scratch.0.join("real");
    fs::write(&real, "x")?;
    let link = scratch.0.join("link");
    std::os::unix::fs::symlink(&real, &link)?;
    assert!(regular(&real).is_ok());
    assert!(regular(&link).is_err());
    assert!(regular(&scratch.0.join("missing")).is_err());
    let candidate = scratch.0.join("candidate");
    fs::write(&candidate, "new")?;
    assert!(replace(&candidate, &link).is_err());
    assert_eq!(fs::read_to_string(&real)?, "x", "the file behind the link is untouched");
    Ok(())
}
