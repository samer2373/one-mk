mod fixtures;

#[cfg(target_os = "linux")]
use std::path::Path;
use std::process::{Command, Output};

fn invoke(arguments: &[&std::ffi::OsStr]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk-wall-import"));
    command.args(arguments).env_clear();
    if arguments.first().is_some_and(|operation| ["import", "doctor", "storage-check"].iter().any(|name| operation == name))
        && let Some(root) = std::env::var_os("ONE_MK_WALL_TEST_CGROUP_ROOT") {
        command.arg("--cgroup-root").arg(root);
    }
    command.output().expect("finite helper reaped")
}

#[test]
#[cfg(all(target_os = "linux", feature = "test-faults"))]
fn killed_workers_at_each_checkpoint_and_cancel_leave_no_partials_or_descendants() {
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = std::env::var_os("ONE_MK_WALL_TEST_RUNTIME") else { eprintln!("kill-stage sandbox test not qualified without explicit lab runtime"); return; };
    let lab = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wall-import-lab").canonicalize().unwrap();
    let temporary = tempfile::tempdir_in(&lab).unwrap();
    let output = temporary.path().join("objects");
    std::fs::create_dir(&output).unwrap();
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = temporary.path().join("two.gif");
    std::fs::write(&source, fixtures::gif(2, 2, 2)).unwrap();
    struct ChildGuard(Option<std::process::Child>, Vec<u32>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 { let _ = child.kill(); let _ = child.wait(); }
            for child in &self.1 {
                let started = std::time::Instant::now();
                while started.elapsed() < std::time::Duration::from_secs(5) {
                    let result = unsafe { libc::waitpid(*child as i32, std::ptr::null_mut(), libc::WNOHANG) };
                    if result == *child as i32 || !Path::new(&format!("/proc/{child}")).exists() { break; }
                    if result == 0 { unsafe { libc::kill(*child as i32, libc::SIGKILL); } }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }
    fn descendants(parent: u32, result: &mut Vec<u32>) {
        let children = std::fs::read_to_string(format!("/proc/{parent}/task/{parent}/children")).unwrap_or_default();
        for child in children.split_whitespace().filter_map(|value| value.parse::<u32>().ok()) {
            result.push(child);
            descendants(child, result);
        }
    }
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) }, 0);
    for stage in ["start", "probe", "decode", "decoder", "poster", "respond", "cancel", "parent-sigkill"] {
        std::fs::write(&source, if stage == "decoder" { std::fs::read(lab.join("media/six.webm")).unwrap() } else { fixtures::gif(2, 2, 2) }).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk-wall-import"));
        command.arg("import").arg(&source).arg("--output").arg(&output).arg("--runtime").arg(&runtime).arg("--json")
            .env_clear().env("ONE_MK_WALL_TEST_STOP", if stage == "cancel" || stage == "parent-sigkill" { "decode" } else { stage })
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
        command.arg("--cgroup-root").arg(std::env::var_os("ONE_MK_WALL_TEST_CGROUP_ROOT").expect("explicit private cgroup delegation required"));
        let mut guard = ChildGuard(Some(command.spawn().unwrap()), Vec::new());
        let parent = guard.0.as_ref().unwrap().id();
        let started = std::time::Instant::now();
        let worker = loop {
            let mut children = Vec::new();
            descendants(parent, &mut children);
            guard.1 = children.clone();
            if let Some(worker) = children.into_iter().find(|child| std::fs::read_to_string(format!("/proc/{child}/status")).is_ok_and(|text| text.lines().any(|line| line.starts_with("State:") && line.contains('T')))) { break worker; }
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "no stopped worker at {stage}");
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        let mut children = Vec::new();
        descendants(parent, &mut children);
        assert_eq!(unsafe { libc::kill(if stage == "cancel" || stage == "parent-sigkill" { parent } else { worker } as i32, if stage == "cancel" { libc::SIGTERM } else { libc::SIGKILL }) }, 0);
        let result = guard.0.take().unwrap().wait_with_output().unwrap();
        if stage == "parent-sigkill" {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(result.status.signal(), Some(libc::SIGKILL));
            assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1, "SIGKILL must leave an unpublished partial for recovery");
            let recovered = invoke(&["import".as_ref(), source.as_os_str(), "--output".as_ref(), output.as_os_str(), "--runtime".as_ref(), &runtime, "--json".as_ref()]);
            assert!(recovered.status.success(), "{}", String::from_utf8_lossy(&recovered.stdout));
            let record: one_mk_wall_import::ImportRecord = serde_json::from_slice(&recovered.stdout).unwrap();
            std::fs::remove_dir_all(output.join(record.source_blake3)).unwrap();
        } else { assert_eq!(result.status.code(), Some(if stage == "cancel" { 130 } else { 5 }), "{}", String::from_utf8_lossy(&result.stdout)); }
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0, "partial files after {stage}");
        for child in children {
            let started = std::time::Instant::now();
            while Path::new(&format!("/proc/{child}")).exists() {
                unsafe { libc::waitpid(child as i32, std::ptr::null_mut(), libc::WNOHANG); }
                assert!(started.elapsed() < std::time::Duration::from_secs(5), "unreaped child {child} after {stage}");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}

#[test]
fn finite_refusal_without_runtime_or_owner_environment() {
    assert!(invoke(&["import".as_ref(), "--help".as_ref()]).status.success());
    assert_eq!(invoke(&["import".as_ref(), "--unknown".as_ref(), "--json".as_ref()]).status.code(), Some(2));
    let status = invoke(&["doctor".as_ref(), "--json".as_ref()]);
    assert_eq!(status.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&status.stdout).contains("sandbox unavailable: refusing to import"));
    let status = invoke(&["--worker".as_ref()]);
    assert_eq!(status.status.code(), Some(4));
    let status = invoke(&["import".as_ref()]);
    assert_eq!(status.status.code(), Some(2));
}

#[test]
#[cfg(all(target_os = "linux", feature = "test-faults"))]
fn capped_real_worker_exhausts_only_private_scratch() {
    let Some(runtime) = std::env::var_os("ONE_MK_WALL_TEST_RUNTIME") else { eprintln!("NOT QUALIFIED: explicit audited lab runtime required"); return; };
    let result = invoke(&["storage-check".as_ref(), "--runtime".as_ref(), &runtime]);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
}

#[test]
fn generated_images_have_structural_admission() {
    for bytes in [fixtures::gif(2, 2, 2), fixtures::apng(), fixtures::webp()] {
        assert_eq!(one_mk_wall_import::inspect(&bytes).unwrap().frames, 2);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn isolated_real_binary_import_and_failure_paths() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let Some(runtime) = std::env::var_os("ONE_MK_WALL_TEST_RUNTIME") else {
        eprintln!("sandbox end-to-end not requested; set ONE_MK_WALL_TEST_RUNTIME to an explicit pinned lab root (not a passing sandbox qualification)");
        return;
    };
    let runtime = Path::new(&runtime);
    let status = invoke(&["doctor".as_ref(), "--runtime".as_ref(), runtime.as_os_str(), "--json".as_ref()]);
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stdout));
    let lab = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wall-import-lab");
    std::fs::create_dir_all(&lab).unwrap();
    let lab = lab.canonicalize().unwrap();
    let temporary = tempfile::tempdir_in(&lab).unwrap();
    let output = temporary.path().join("objects");
    std::fs::create_dir(&output).unwrap();
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = temporary.path().join("untrusted");
    let stale = output.join(".import-stale-fixture");
    std::fs::create_dir(&stale).unwrap();
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(stale.join("source"), b"interrupted parent snapshot").unwrap();
    std::fs::File::open(&stale).unwrap().set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH)).unwrap();
    let run = || invoke(&["import".as_ref(), source.as_os_str(), "--runtime".as_ref(), runtime.as_os_str(), "--output".as_ref(), output.as_os_str(), "--json".as_ref()]);
    for (name, expected) in [("six.webm", one_mk_wall_import::Format::Vp9Webm), ("six-av1.webm", one_mk_wall_import::Format::Av1Webm)] {
        let bytes = std::fs::read(lab.join("media").join(name)).expect("generate six-frame VP9/AV1 lab fixtures before sandbox qualification");
        std::fs::write(&source, &bytes).unwrap();
        let result = run();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
        let record: one_mk_wall_import::ImportRecord = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(record.media.format, expected);
        assert_eq!(record.media.frames, 6);
        assert_eq!(record.media.duration_ms, 200);
        assert_eq!(record.source_blake3, blake3::hash(&bytes).to_hex().to_string());
        assert!(!stale.exists(), "expired interrupted-parent partial must be reaped on activation");
        std::fs::remove_dir_all(output.join(record.source_blake3)).unwrap();
    }
    for bytes in [fixtures::gif(2, 2, 2), fixtures::apng(), fixtures::webp()] {
        std::fs::write(&source, &bytes).unwrap();
        let result = run();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
        let record: one_mk_wall_import::ImportRecord = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(record.source_blake3, blake3::hash(&bytes).to_hex().to_string());
        assert_eq!(record.media.frames, 2);
        assert_eq!(record.media.duration_ms, 200);
        let object = output.join(&record.source_blake3);
        assert!(object.join("poster.ppm").is_file());
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(object.join("record.json")).unwrap()).unwrap(), serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap());
        let duplicate = run();
        assert!(!duplicate.status.success(), "existing objects must never be overwritten");
        std::fs::remove_dir_all(object).unwrap();
        let repeated = run();
        assert!(repeated.status.success());
        assert_eq!(result.stdout, repeated.stdout, "record must be deterministic");
        std::fs::remove_dir_all(output.join(record.source_blake3)).unwrap();
    }
    let mut attacks = vec![b"GIF89a\xff\xff\xff\xff\0\0\0;".to_vec(), b"\0\0\0\x18ftypmp42".to_vec(), b"RIFF\x04\0\0\0AVI ".to_vec(), b"GIF89a".to_vec()];
    let mut polyglot = fixtures::gif(2, 2, 2);
    polyglot.extend_from_slice(b"PK\x03\x04");
    attacks.push(polyglot);
    let mut bomb = fixtures::gif(2, 2, 2);
    bomb[6..10].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    attacks.push(bomb);
    for bytes in attacks {
        std::fs::write(&source, bytes).unwrap();
        let result = run();
        assert!(!result.status.success());
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    }
    std::fs::write(&source, fixtures::gif(2, 2, 2)).unwrap();
    let hash = blake3::hash(&std::fs::read(&source).unwrap()).to_hex().to_string();
    symlink(temporary.path(), output.join(&hash)).unwrap();
    assert!(!run().status.success());
    std::fs::remove_file(output.join(&hash)).unwrap();
    let link = temporary.path().join("input-link");
    symlink(&source, &link).unwrap();
    let result = invoke(&["import".as_ref(), link.as_os_str(), "--runtime".as_ref(), runtime.as_os_str(), "--output".as_ref(), output.as_os_str(), "--json".as_ref()]);
    assert!(!result.status.success());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    assert!(one_mk_wall_import::import(one_mk_wall_import::ImportOptions { source: &source, output: &output, runtime, cgroup_root: None, cancel: &cancelled, timeout: std::time::Duration::ZERO }).unwrap_err().contains("cancelled"));
}
