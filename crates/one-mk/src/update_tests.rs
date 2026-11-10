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
    assert!(verify_embedded(message, signed.as_ref()).is_err(), "the shipped key must never trust the published test seed");
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
fn update_preferences_default_to_check_and_ask() -> Result<()> {
    let defaults = Settings::default();
    assert!(defaults.check);
    assert!(!defaults.install_automatically);
    let legacy: Settings = toml::from_str("check = true")?;
    assert_eq!(legacy, defaults);
    let automatic: Settings = toml::from_str("check = true\ninstall_automatically = true")?;
    assert!(automatic.install_automatically);
    assert_eq!(toml::from_str::<Settings>(&toml::to_string(&automatic)?)?, automatic);
    Ok(())
}

#[test]
fn doctor_uses_only_plain_cached_results() -> Result<()> {
    let scratch = Scratch::new()?;
    assert_eq!(doctor(&scratch.0), (false, "Couldn't check: no completed update check yet".into()));
    let current = env!("CARGO_PKG_VERSION");
    let mut state = State { sequence: 1, digest: "00".repeat(32), highest_version: Some(current.into()), ..State::default() };
    save_state(&scratch.0, &state)?;
    assert_eq!(doctor(&scratch.0), (false, format!("Up to date ({current})")));
    let current = policy::version(current)?;
    state.available = Some(format!("{}.0.0", current.major + 1));
    save_state(&scratch.0, &state)?;
    assert_eq!(doctor(&scratch.0), (true, "Update available — Update now".into()));
    state.error = Some("Check your internet connection and try again".into());
    save_state(&scratch.0, &state)?;
    assert_eq!(doctor(&scratch.0), (false, "Couldn't check: Check your internet connection and try again".into()));
    fs::write(scratch.0.join(STATE), b"broken")?;
    assert_eq!(doctor(&scratch.0), (false, "Couldn't check: saved update information couldn't be read".into()));
    Ok(())
}

#[test]
fn failed_checks_are_said_in_plain_words() -> Result<()> {
    // What `update --url http://…` and GitHub's anonymous rate limit say.
    let insecure = validate_url(&Url::parse("http://example.com/latest.json")?).unwrap_err().to_string();
    assert_eq!(insecure, "The update address must start with https://; oneMK only downloads updates over a secure connection");
    assert_eq!(refused(403, Some("api.github.com")), "GitHub is limiting requests from your network; try again in a while");
    assert_eq!(refused(429, Some("github.com")), "GitHub is limiting requests from your network; try again in a while");
    assert_eq!(refused(403, Some("updates.example.com")), "The update server is limiting requests from your network; try again in a while");
    assert!(refused(404, Some("github.com")).contains("no signed oneMK release"));
    for code in [400, 403, 404, 429, 500, 503] {
        let said = refused(code, Some("github.com"));
        assert!(!said.contains("unit test") && !said.contains("check for a published"), "{said}");
    }
    // Saved by an older version: shown in today's words, in doctor and the window.
    let scratch = Scratch::new()?;
    for (old, now) in [
        ("update URL must use HTTPS (HTTP is allowed only for 127.0.0.1 in unit tests)", insecure.as_str()),
        ("update server returned HTTP 403; check for a published signed release and retry later", "GitHub is limiting requests from your network; try again in a while"),
    ] {
        let state = State { error: Some(old.into()), ..State::default() };
        save_state(&scratch.0, &state)?;
        assert_eq!(doctor(&scratch.0), (false, format!("Couldn't check: {now}")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(scratch.0.join(STATE))?.permissions().mode() & 0o777, 0o600, "the saved state is private");
    }
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
    let _lock = target_lock(&target, "denied")?;
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
#[cfg(unix)]
fn version_probe_writes_nothing_inside_a_mac_app() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new()?;
    let macos = scratch.0.join("one-mk.app/Contents/MacOS");
    fs::create_dir_all(&macos)?;
    let target = macos.join("one-mk");
    let listing = scratch.0.join("listing");
    // The program lists its own folder while it runs: macOS kills an app that has a file added beside it.
    fs::write(&target, format!("#!/bin/sh\nls -A \"$(dirname \"$0\")\" > '{}'\necho one-mk 0.1.13\n", listing.display()))?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    version_check(&target, Some("0.1.13"))?;
    assert_eq!(fs::read_to_string(listing)?.trim(), "one-mk");
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

#[test]
#[cfg(target_os = "linux")]
#[ignore = "starts a local Python HTTP server; run explicitly"]
fn python_http_portable_end_to_end() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new()?;
    let release_dir = scratch.0.join("release");
    let installation = scratch.0.join("portable");
    let home = scratch.0.join("state");
    fs::create_dir(&release_dir)?;
    fs::create_dir(&installation)?;
    fs::create_dir(&home)?;
    let target = installation.join("one-mk");
    let original = b"#!/bin/sh\necho 'one-mk 0.1.19'\n";
    let replacement = b"#!/bin/sh\necho 'one-mk 0.1.20'\n";
    fs::write(&target, original)?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    fs::write(release_dir.join("portable.bin"), replacement)?;
    let mut manifest = fixture(replacement);
    manifest.version = "0.1.20".into();
    let bytes = serde_json::to_vec(&manifest)?;
    let key = pair()?;
    let signed = key.sign(&bytes);
    fs::write(release_dir.join("update-manifest.json"), &bytes)?;
    fs::write(release_dir.join("update-manifest.json.sig"), signed.as_ref())?;
    let reservation = TcpListener::bind("127.0.0.1:0")?;
    let port = reservation.local_addr()?.port();
    drop(reservation);
    let endpoint = format!("http://127.0.0.1:{port}/latest.json");
    let assets = ["portable.bin", "update-manifest.json", "update-manifest.json.sig"].map(|name|
        serde_json::json!({"name":name, "browser_download_url":format!("http://127.0.0.1:{port}/{name}")}));
    fs::write(release_dir.join("latest.json"), serde_json::to_vec(&serde_json::json!({"tag_name":"v0.1.20", "assets":assets}))?)?;
    struct ServerProcess(std::process::Child);
    impl Drop for ServerProcess { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    let mut server = ServerProcess(Command::new("python3").args(["-m", "http.server", &port.to_string(), "--bind", "127.0.0.1", "--directory"])
        .arg(&release_dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?);
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        ensure!(server.0.try_wait()?.is_none() && Instant::now() < deadline, "local Python server did not start");
        std::thread::sleep(Duration::from_millis(25));
    }
    println!("E2E: python3 -m http.server bound to 127.0.0.1 (temporary release directory)");
    assert!(verify_embedded(&bytes, signed.as_ref()).is_err());
    println!("E2E: production verification refuses the RFC 8032 test signature");
    let mut state = State::default();
    let (manifest, release, origin, _) = checked(&home, &endpoint, key.public_key().as_ref(), "0.1.19", &mut state)?;
    println!("E2E: TEST-ONLY explicit key verified signed 0.1.20 manifest; installed version 0.1.19");
    apply(&manifest, &release, &origin, &target, InstallKind::Portable)?;
    version_check(&target, Some("0.1.20"))?;
    assert_eq!(fs::read(installation.join("one-mk.old"))?, original);
    println!("E2E: signed SHA-256 and size verified; portable 0.1.20 starts; 0.1.19 backup retained");
    rollback(&target)?;
    version_check(&target, Some("0.1.19"))?;
    assert_eq!(load_state(&home)?.highest_version.as_deref(), Some("0.1.20"));
    println!("E2E: rollback starts 0.1.19; signed 0.1.20 high-water mark retained; PASS");
    Ok(())
}

#[test]
fn a_package_file_list_names_the_executable_exactly() {
    let list = "/.\n/usr\n/usr/bin\n/usr/bin/one-mk\n/usr/bin/one-mk-app\n";
    assert!(package_lists(list, "/usr/bin/one-mk"));
    assert!(!package_lists(list, "/usr/bin/one"));
    assert!(!package_lists(list, "/usr/local/bin/one-mk"));
}

#[cfg(unix)]
#[test]
fn only_one_mk_at_the_signed_version_is_installed() {
    let ok = "Package: one-mk\nVersion: 0.1.24\n";
    assert!(check_deb_fields(ok, "0.1.24").is_ok());
    assert!(check_deb_fields(ok, "0.1.25").is_err());
    assert!(check_deb_fields("Package: openssh-server\nVersion: 0.1.24\n", "0.1.24").is_err());
    assert!(check_deb_fields("Package: one-mk-evil\nVersion: 0.1.24\n", "0.1.24").is_err());
    assert!(check_deb_fields("", "0.1.24").is_err());
}


/// A staged package update, as `apply_deb` leaves it for the root step: a
/// signed list and the package the list names, in a private folder.
#[cfg(unix)]
struct RootFixture { _sandbox: tempfile::TempDir, staging: PathBuf, state: PathBuf, key: signature::Ed25519KeyPair }

#[cfg(unix)]
const DEB_NAME: &str = "one-mk-linux.deb";

#[cfg(unix)]
fn fake_deb(version: &str) -> Vec<u8> { format!("Package: one-mk\nVersion: {version}\n").into_bytes() }

#[cfg(unix)]
fn deb_manifest(package: &[u8], version: &str, sequence: u64) -> Manifest {
    Manifest { version: version.into(), release_time: "2026-11-01T00:00:00Z".into(), sequence, minimum_supported_version: "0.1.0".into(),
        assets: vec![policy::Asset { name: DEB_NAME.into(), size: package.len() as u64, sha256: hex(digest::digest(&digest::SHA256, package).as_ref()),
            os: "linux".into(), arch: std::env::consts::ARCH.into(), kind: "deb".into() }], revoked: vec![], next_key: None }
}

#[cfg(unix)]
impl RootFixture {
    fn new() -> Result<Self> {
        let sandbox = tempfile::tempdir()?;
        let staging = sandbox.path().join(DEB_STAGING);
        let state = sandbox.path().join("root-state");
        Ok(Self { staging, state, _sandbox: sandbox, key: pair()? })
    }

    /// Stages `manifest` signed by the test key (or `signature`), with `package` beside it.
    fn stage(&self, manifest: &[u8], signature: Option<&[u8]>, package: &[u8]) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::remove_dir_all(&self.staging);
        fs::create_dir(&self.staging)?;
        fs::set_permissions(&self.staging, fs::Permissions::from_mode(0o700))?;
        let signed = self.key.sign(manifest);
        fs::write(self.staging.join(STAGED_MANIFEST), manifest)?;
        fs::write(self.staging.join(STAGED_SIGNATURE), signature.unwrap_or(signed.as_ref()))?;
        fs::write(self.staging.join(DEB_NAME), package)?;
        Ok(())
    }

    fn stage_release(&self, version: &str, sequence: u64) -> Result<Vec<u8>> {
        let package = fake_deb(version);
        self.stage(&serde_json::to_vec(&deb_manifest(&package, version, sequence))?, None, &package)?;
        Ok(package)
    }

    /// Runs the root step as it runs under pkexec, with the test key and a
    /// fake installer. Returns what the installer was given: its path and bytes.
    fn run(&self, installed: &str, during: &dyn Fn(&Path)) -> (Result<()>, Option<(PathBuf, Vec<u8>)>) {
        let installed_copy = std::cell::RefCell::new(None);
        let public = self.key.public_key().as_ref().to_vec();
        let verify = move |message: &[u8], signed: &[u8]| verify(&public, message, signed);
        let fields = |copy: &Path| -> Result<String> { during(&self.staging); Ok(String::from_utf8_lossy(&fs::read(copy)?).into_owned()) };
        let install = |copy: &Path| -> Result<()> { *installed_copy.borrow_mut() = Some((copy.to_owned(), fs::read(copy)?)); Ok(()) };
        let result = install_staged(&self.staging, &RootStep {
            owner: unsafe { libc::geteuid() }, installed, floor: 0, state: &self.state, verify: &verify, fields: &fields, install: &install,
        });
        (result, installed_copy.into_inner())
    }
}

#[cfg(unix)]
fn nothing(_: &Path) {}

#[cfg(unix)]
#[test]
fn the_root_step_installs_only_its_own_verified_copy_of_a_signed_newer_package() -> Result<()> {
    let fixture = RootFixture::new()?;
    let package = fixture.stage_release("0.1.30", 10)?;
    let (result, installed) = fixture.run("0.1.29", &nothing);
    result?;
    let (path, bytes) = installed.context("nothing was installed")?;
    assert_eq!(bytes, package);
    assert!(path.starts_with(&fixture.state), "installs a root-only copy, never the staged file: {}", path.display());
    let state = load_state(&fixture.state)?;
    assert_eq!((state.sequence, state.highest_version.as_deref()), (10, Some("0.1.30")));
    assert!(!fs::read_dir(&fixture.state)?.flatten().any(|entry| entry.file_name().to_string_lossy().starts_with(".one-mk-update-payload-")),
        "the private copy is removed");
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_root_step_refuses_a_tampered_or_foreign_signed_list() -> Result<()> {
    let fixture = RootFixture::new()?;
    let package = fake_deb("0.1.30");
    let manifest = serde_json::to_vec(&deb_manifest(&package, "0.1.30", 10))?;
    // Signed, then changed: the version, then the checksum of the package.
    let signed = fixture.key.sign(&manifest);
    for changed in [String::from_utf8(manifest.clone())?.replace("0.1.30", "9.9.9"),
        String::from_utf8(manifest.clone())?.replace(&hex(digest::digest(&digest::SHA256, &package).as_ref()), &"ab".repeat(32))] {
        fixture.stage(changed.as_bytes(), Some(signed.as_ref()), &package)?;
        let (result, installed) = fixture.run("0.1.29", &nothing);
        assert!(result.is_err() && installed.is_none());
    }
    // Signed by another key.
    let other = signature::Ed25519KeyPair::from_seed_unchecked(&[7; 32]).map_err(|_| anyhow::anyhow!("test key"))?;
    fixture.stage(&manifest, Some(other.sign(&manifest).as_ref()), &package)?;
    let (result, installed) = fixture.run("0.1.29", &nothing);
    assert!(result.is_err() && installed.is_none());
    // A truncated signature, then none at all.
    fixture.stage(&manifest, Some(&signed.as_ref()[..63]), &package)?;
    assert!(fixture.run("0.1.29", &nothing).1.is_none());
    fs::remove_file(fixture.staging.join(STAGED_SIGNATURE))?;
    assert!(fixture.run("0.1.29", &nothing).1.is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_root_step_refuses_a_downgrade_or_the_installed_version() -> Result<()> {
    let fixture = RootFixture::new()?;
    for version in ["0.1.29", "0.1.28", "0.1.0"] {
        fixture.stage_release(version, 10)?;
        let (result, installed) = fixture.run("0.1.29", &nothing);
        assert!(result.is_err() && installed.is_none(), "{version} must not be installed over 0.1.29");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_root_step_refuses_a_package_that_is_not_the_signed_one() -> Result<()> {
    let fixture = RootFixture::new()?;
    let package = fake_deb("0.1.30");
    let manifest = serde_json::to_vec(&deb_manifest(&package, "0.1.30", 10))?;
    // Same size, other bytes (still a valid one-mk package at that version,
    // so only the checksum can tell); longer; shorter.
    let mut same_size = package.clone();
    *same_size.last_mut().context("empty package")? = b' ';
    assert!(check_deb_fields(&String::from_utf8_lossy(&same_size), "0.1.30").is_ok());
    let mut longer = package.clone();
    longer.push(b'\n');
    for wrong in [same_size, longer, package[..package.len() - 1].to_vec()] {
        fixture.stage(&manifest, None, &wrong)?;
        let (result, installed) = fixture.run("0.1.29", &nothing);
        assert!(result.is_err() && installed.is_none());
    }
    // The signed bytes, but not the one-mk package: still refused by the package's own fields.
    let evil = b"Package: openssh-server\nVersion: 0.1.30\n".to_vec();
    fixture.stage(&serde_json::to_vec(&deb_manifest(&evil, "0.1.30", 10))?, None, &evil)?;
    let (result, installed) = fixture.run("0.1.29", &nothing);
    assert!(result.is_err() && installed.is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_package_swapped_after_the_check_is_never_installed() -> Result<()> {
    use std::os::unix::fs::symlink;
    let fixture = RootFixture::new()?;
    let package = fixture.stage_release("0.1.30", 10)?;
    // While the root step works, the staged file is replaced by another one,
    // then by a link: what is installed is still the verified copy.
    let swap = |staging: &Path| {
        let _ = fs::write(staging.join(DEB_NAME), b"Package: one-mk\nVersion: 0.1.30\nroot shell\n");
        let _ = fs::remove_file(staging.join(DEB_NAME));
        let _ = symlink("/etc/passwd", staging.join(DEB_NAME));
    };
    let (result, installed) = fixture.run("0.1.29", &swap);
    result?;
    assert_eq!(installed.context("nothing was installed")?.1, package);
    // A link or a pipe staged in the first place is refused, without blocking.
    fixture.stage_release("0.1.31", 11)?;
    fs::remove_file(fixture.staging.join(DEB_NAME))?;
    symlink(fixture.state.join("anything"), fixture.staging.join(DEB_NAME))?;
    assert!(fixture.run("0.1.30", &nothing).1.is_none());
    fs::remove_file(fixture.staging.join(DEB_NAME))?;
    let name = std::ffi::CString::new(fixture.staging.join(DEB_NAME).as_os_str().as_encoded_bytes())?;
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(fixture.run("0.1.30", &nothing).1.is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_root_step_refuses_a_replayed_older_signed_list() -> Result<()> {
    let fixture = RootFixture::new()?;
    fixture.stage_release("0.1.31", 20)?;
    fixture.run("0.1.29", &nothing).0?;
    // The installed version went back (by hand, say): an older genuine list,
    // even for a version newer than the installed one, is refused...
    fixture.stage_release("0.1.30", 19)?;
    let (result, installed) = fixture.run("0.1.29", &nothing);
    assert!(result.is_err() && installed.is_none());
    // ...and so is the same sequence with other contents.
    let package = fake_deb("0.1.32");
    fixture.stage(&serde_json::to_vec(&deb_manifest(&package, "0.1.32", 20))?, None, &package)?;
    assert!(fixture.run("0.1.29", &nothing).1.is_none());
    // The very list accepted before is fine again (same bytes), and so is a newer one.
    fixture.stage_release("0.1.31", 20)?;
    fixture.run("0.1.29", &nothing).0?;
    fixture.stage_release("0.1.32", 21)?;
    fixture.run("0.1.31", &nothing).0?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_root_step_takes_only_a_private_staging_folder_of_the_account_that_asked() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = RootFixture::new()?;
    fixture.stage_release("0.1.30", 10)?;
    fs::set_permissions(&fixture.staging, fs::Permissions::from_mode(0o770))?;
    assert!(fixture.run("0.1.29", &nothing).1.is_none(), "a folder others can write to");
    fs::set_permissions(&fixture.staging, fs::Permissions::from_mode(0o700))?;
    let public = fixture.key.public_key().as_ref().to_vec();
    let verify = move |message: &[u8], signed: &[u8]| verify(&public, message, signed);
    let fields = |copy: &Path| -> Result<String> { Ok(String::from_utf8_lossy(&fs::read(copy)?).into_owned()) };
    let install = |_: &Path| -> Result<()> { bail!("must not install") };
    let step = |owner| RootStep { owner, installed: "0.1.29", floor: 0, state: &fixture.state, verify: &verify, fields: &fields, install: &install };
    let me = unsafe { libc::geteuid() };
    assert!(install_staged(&fixture.staging, &step(me + 1)).is_err(), "files of another account");
    let renamed = fixture.staging.with_file_name("elsewhere");
    fs::rename(&fixture.staging, &renamed)?;
    assert!(install_staged(&renamed, &step(me)).is_err(), "another folder name");
    symlink(&renamed, &fixture.staging)?;
    assert!(install_staged(&fixture.staging, &step(me)).is_err(), "a link to a folder");
    assert!(install_staged(Path::new(DEB_STAGING), &step(me)).is_err(), "a relative path");
    // A release floor (the sequence this build was released with) above the list: refused too.
    fs::remove_file(&fixture.staging)?;
    fs::rename(&renamed, &fixture.staging)?;
    let floored = RootStep { floor: 11, ..step(me) };
    assert!(install_staged(&fixture.staging, &floored).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_password_prompt_runs_the_root_step_with_only_the_folder() {
    let command = root_command(Path::new("/usr/bin/one-mk"), Path::new("/home/me/.config/one-mk/update-staging"));
    assert_eq!(command.get_program(), "pkexec");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, ["/usr/bin/one-mk", "update", "--install-deb", "/home/me/.config/one-mk/update-staging"]);
    // Run by hand, not through the password prompt, it does nothing.
    if unsafe { libc::geteuid() } != 0 {
        assert!(install_deb_as_root(Path::new("/nonexistent/update-staging")).is_err());
    }
}

/// The real, production-signed v0.1.28 update list, through the root step
/// with the key compiled into this build: it verifies, then is refused only
/// for what it is (not newer than 0.1.28; its package not staged), and one
/// changed byte makes it untrusted. (x86_64: the release's only .deb.)
#[cfg(all(unix, target_arch = "x86_64"))]
#[test]
fn the_root_step_verifies_a_real_release_with_the_compiled_in_key() -> Result<()> {
    let manifest = include_bytes!("update_fixtures/v0.1.28-update-manifest.json");
    let signature = include_bytes!("update_fixtures/v0.1.28-update-manifest.json.sig");
    let fixture = RootFixture::new()?;
    let fields = |_: &Path| -> Result<String> { bail!("not reached") };
    let install = |_: &Path| -> Result<()> { bail!("must not install") };
    let step = |installed| RootStep { owner: unsafe { libc::geteuid() }, installed, floor: 0, state: &fixture.state,
        verify: &verify_embedded, fields: &fields, install: &install };
    fixture.stage(manifest, Some(signature), b"")?;
    fs::remove_file(fixture.staging.join(DEB_NAME))?;
    let older = install_staged(&fixture.staging, &step("0.1.27")).expect_err("the package is not staged");
    assert!(format!("{older:#}").contains("one-mk-x86_64-unknown-linux-gnu.deb"), "verified, then looked for the signed package: {older:#}");
    let same = install_staged(&fixture.staging, &step("0.1.28")).expect_err("not newer");
    assert!(format!("{same:#}").contains("not a newer version"), "{same:#}");
    let mut changed = manifest.to_vec();
    *changed.last_mut().context("empty manifest")? = b' ';
    fixture.stage(&changed, Some(signature), b"")?;
    let tampered = install_staged(&fixture.staging, &step("0.1.27")).expect_err("tampered");
    assert!(format!("{tampered:#}").contains("not signed by the publisher"), "{tampered:#}");
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn the_root_step_reads_a_real_package_with_dpkg_deb() -> Result<()> {
    if !Path::new("/usr/bin/dpkg-deb").exists() { return Ok(()); }
    let fixture = RootFixture::new()?;
    let build = fixture.state.with_file_name("build");
    fs::create_dir_all(build.join("DEBIAN"))?;
    fs::write(build.join("DEBIAN/control"), "Package: one-mk\nVersion: 0.1.30\nArchitecture: all\nMaintainer: test <t@example.invalid>\nDescription: test\n")?;
    let deb = fixture.state.with_file_name("one-mk.deb");
    ensure!(Command::new("/usr/bin/dpkg-deb").args(["--root-owner-group", "--build"]).arg(&build).arg(&deb).stdout(Stdio::null()).status()?.success());
    let package = fs::read(&deb)?;
    fixture.stage(&serde_json::to_vec(&deb_manifest(&package, "0.1.30", 10))?, None, &package)?;
    let public = fixture.key.public_key().as_ref().to_vec();
    let verify = move |message: &[u8], signed: &[u8]| verify(&public, message, signed);
    let installed = std::cell::RefCell::new(None);
    let install = |copy: &Path| -> Result<()> { *installed.borrow_mut() = Some(fs::read(copy)?); Ok(()) };
    install_staged(&fixture.staging, &RootStep { owner: unsafe { libc::geteuid() }, installed: "0.1.29", floor: 0, state: &fixture.state,
        verify: &verify, fields: &deb_fields, install: &install })?;
    assert_eq!(installed.borrow().as_deref(), Some(package.as_slice()));
    Ok(())
}
