use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const GST_VERSION: &str = "1.28.2";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema: u32,
    pub gstreamer: String,
    pub files: std::collections::BTreeMap<String, String>,
    pub licenses: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct Availability { pub available: bool, pub reason: String }

pub fn doctor(runtime: Option<&Path>) -> Availability {
    let result = runtime.ok_or_else(|| "no audited runtime selected (--runtime DIR)".to_string())
        .and_then(validate_runtime)
        .and_then(|root| run(&root, None, "--sandbox-check", &std::sync::atomic::AtomicBool::new(false), std::time::Duration::from_secs(5)).map(|_| ()));
    match result {
        Ok(()) => Availability { available: true, reason: "bubblewrap namespaces, denied sockets, read-only runtime and resource limits passed; native hardware unqualified".into() },
        Err(reason) => Availability { available: false, reason: format!("sandbox unavailable: refusing to import: {reason}") },
    }
}

pub fn validate_runtime(root: &Path) -> Result<PathBuf, String> {
    use std::io::Read;
    use sha2::{Digest, Sha256};
    if !root.is_absolute() || root.canonicalize().map_err(|error| error.to_string())? != root { return Err("runtime must be an absolute nonsymlink canonical directory".into()); }
    let manifest_path = root.join("runtime.json");
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if root.metadata().map_err(|error| error.to_string())?.mode() & 0o022 != 0 { return Err("runtime root must not be group/world writable".into()); }
    }
    let mut manifest_bytes = Vec::new();
    std::fs::File::open(&manifest_path).map_err(|error| format!("audited runtime manifest: {error}"))?.take(crate::MAX_METADATA as u64 + 1).read_to_end(&mut manifest_bytes).map_err(|error| error.to_string())?;
    if manifest_bytes.len() > crate::MAX_METADATA { return Err("runtime manifest too large".into()); }
    let manifest: RuntimeManifest = serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    if manifest.schema != 1 || manifest.gstreamer != GST_VERSION || manifest.files.is_empty() { return Err("runtime version/manifest mismatch".into()); }
    fn walk(root: &Path, directory: &Path, actual: &mut std::collections::BTreeMap<String, String>) -> Result<(), String> {
        for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            #[cfg(unix)] {
                use std::os::unix::fs::MetadataExt;
                if metadata.mode() & 0o022 != 0 { return Err("runtime contains group/world-writable entries".into()); }
            }
            if metadata.is_dir() { walk(root, &path, actual)?; }
            else if metadata.is_file() && path != root.join("runtime.json") {
                if metadata.len() > 128 * 1024 * 1024 || actual.len() >= 1024 { return Err("runtime size/file limit".into()); }
                let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
                let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
                actual.insert(path.strip_prefix(root).map_err(|error| error.to_string())?.to_string_lossy().into_owned(), hash);
            } else if !metadata.is_file() { return Err("runtime symlink/device forbidden".into()); }
        }
        Ok(())
    }
    let mut actual = std::collections::BTreeMap::new();
    walk(root, root, &mut actual)?;
    if actual != manifest.files || manifest.licenses.keys().ne(manifest.files.keys()) { return Err("runtime closure hash/licence inventory mismatch (including unlisted files)".into()); }
    for (name, license) in &manifest.licenses {
        let lower = name.to_ascii_lowercase();
        if ["ffmpeg", "libavcodec", "libavformat", "libavfilter", "libavdevice", "libavutil", "libswscale", "libswresample", "gstlibav", "x264", "x265", "libmpv"].iter().any(|forbidden| lower.contains(forbidden)) {
            return Err("forbidden codec in runtime closure".into());
        }
        if !["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "bzip2-1.0.6", "LGPL-2.0-or-later", "LGPL-2.1-or-later", "GPL-3.0-or-later WITH GCC-exception-3.1"].contains(&license.as_str()) {
            return Err("unknown or incompatible runtime licence; refusing import".into());
        }
    }
    Ok(root.to_owned())
}

#[cfg(target_os = "linux")]
pub fn worker_restrictions() -> Result<(), String> {
    if std::env::var("ONE_MK_WALL_SANDBOX").as_deref() != Ok("1") { return Err("worker must run inside the import sandbox".into()); }
    let architecture = if cfg!(target_arch = "x86_64") { 0xc000003e } else if cfg!(target_arch = "aarch64") { 0xc00000b7 } else { return Err("seccomp architecture unqualified".into()); };
    let instruction = |code, jump_true, jump_false, value| libc::sock_filter { code, jt: jump_true, jf: jump_false, k: value };
    let mut filter = vec![
        instruction(0x20, 0, 0, 4), instruction(0x15, 1, 0, architecture), instruction(0x06, 0, 0, 0x80000000), instruction(0x20, 0, 0, 0),
        instruction(0x35, 0, 1, 0x40000000), instruction(0x06, 0, 0, 0x80000000),
    ];
    for syscall in [libc::SYS_socket, libc::SYS_connect, libc::SYS_bind, libc::SYS_listen, libc::SYS_accept, libc::SYS_ptrace, libc::SYS_mount, libc::SYS_umount2, libc::SYS_unshare, libc::SYS_setns, libc::SYS_bpf, libc::SYS_keyctl, libc::SYS_perf_event_open, libc::SYS_process_vm_readv, libc::SYS_process_vm_writev, libc::SYS_io_uring_setup, libc::SYS_io_uring_enter, libc::SYS_io_uring_register, libc::SYS_userfaultfd, libc::SYS_open_by_handle_at] {
        filter.push(instruction(0x15, 0, 1, syscall as u32));
        filter.push(instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32));
    }
    filter.extend([
        instruction(0x15, 0, 4, libc::SYS_socketpair as u32), instruction(0x20, 0, 0, 16),
        instruction(0x15, 1, 0, libc::AF_UNIX as u32), instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
        instruction(0x06, 0, 0, 0x7fff0000),
    ]);
    filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    let program = libc::sock_fprog { len: filter.len() as u16, filter: filter.as_mut_ptr() };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 || unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program) } != 0 { return Err(format!("seccomp: {}", std::io::Error::last_os_error())); }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn worker_restrictions() -> Result<(), String> {
    Err("sandbox unavailable: Windows AppContainer/job and macOS signed sandbox helper are designed but UNTESTED/unimplemented; refusing to import".into())
}

#[cfg(target_os = "linux")]
pub fn check_worker() -> Result<(), String> {
    worker_restrictions()?;
    let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if socket >= 0 { unsafe { libc::close(socket); } return Err("network denial failed".into()); }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) { return Err("seccomp socket denial unverified".into()); }
    if std::fs::write("/sandbox-write-test", b"denied").is_ok() || std::fs::read("/etc/passwd").is_ok() || std::fs::read_dir("/home").is_ok() { return Err("filesystem denial failed".into()); }
    for (resource, ceiling) in [(libc::RLIMIT_AS, 1024 * 1024 * 1024), (libc::RLIMIT_CPU, 120), (libc::RLIMIT_NOFILE, 64)] {
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if unsafe { libc::getrlimit(resource, &mut limit) } != 0 || limit.rlim_max > ceiling { return Err("resource ceiling not installed".into()); }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn check_worker() -> Result<(), String> { worker_restrictions() }

#[cfg(target_os = "linux")]
pub fn deny_subprocesses() -> Result<(), String> {
    let instruction = |code, jump_true, jump_false, value| libc::sock_filter { code, jt: jump_true, jf: jump_false, k: value };
    let mut filter = vec![instruction(0x20, 0, 0, 0)];
    #[cfg(target_arch = "x86_64")]
    for syscall in [libc::SYS_fork, libc::SYS_vfork] {
        filter.push(instruction(0x15, 0, 1, syscall as u32));
        filter.push(instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32));
    }
    filter.extend([
        instruction(0x15, 0, 1, libc::SYS_clone3 as u32), instruction(0x06, 0, 0, 0x00050000 | libc::ENOSYS as u32),
        instruction(0x15, 0, 4, libc::SYS_clone as u32), instruction(0x20, 0, 0, 16),
        instruction(0x45, 1, 0, libc::CLONE_THREAD as u32), instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
        instruction(0x06, 0, 0, 0x7fff0000), instruction(0x06, 0, 0, 0x7fff0000),
    ]);
    let program = libc::sock_fprog { len: filter.len() as u16, filter: filter.as_mut_ptr() };
    if unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program) } != 0 { return Err("cannot restrict decoder descendants".into()); }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn deny_subprocesses() -> Result<(), String> { worker_restrictions() }

#[cfg(target_os = "linux")]
pub(crate) fn run(root: &Path, input: Option<&Path>, operation: &str, cancel: &std::sync::atomic::AtomicBool, timeout: std::time::Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    use std::process::{Command, Stdio};
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new("/usr/bin/bwrap");
    command.args(["--unshare-all", "--die-with-parent", "--new-session", "--cap-drop", "ALL", "--ro-bind"])
        .arg(root).arg("/").arg("--ro-bind").arg(executable).arg("/worker")
        .args(["--proc", "/proc", "--dev", "/dev", "--size", "16777216", "--tmpfs", "/tmp", "--chdir", "/tmp", "--clearenv",
            "--setenv", "ONE_MK_WALL_SANDBOX", "1", "--setenv", "HOME", "/nonexistent",
            "--setenv", "GST_PLUGIN_SYSTEM_PATH_1_0", "", "--setenv", "GST_PLUGIN_PATH_1_0", "/plugins",
            "--setenv", "GST_REGISTRY_1_0", "/tmp/registry.bin", "--setenv", "GST_REGISTRY_FORK", "no",
            "--setenv", "LD_LIBRARY_PATH", "/lib", "--setenv", "LC_ALL", "C"]);
    #[cfg(feature = "test-faults")]
    if operation == "--worker" && let Some(stage) = std::env::var_os("ONE_MK_WALL_TEST_STOP") {
        command.args(["--setenv", "ONE_MK_WALL_TEST_STOP"]).arg(stage);
    }
    let input_stream = if let Some(input) = input {
        command.args(["--ro-bind", "/proc/self/fd/0", "/input"]);
        Stdio::from(std::fs::File::open(input).map_err(|error| error.to_string())?)
    } else { Stdio::null() };
    command.arg("/worker").arg(operation).env_clear().stdin(input_stream).stdout(Stdio::piped()).stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            for (resource, ceiling) in [(libc::RLIMIT_AS, 1024 * 1024 * 1024), (libc::RLIMIT_CPU, 120), (libc::RLIMIT_FSIZE, crate::MAX_SOURCE), (libc::RLIMIT_NOFILE, 64), (libc::RLIMIT_CORE, 0)] {
                let limit = libc::rlimit { rlim_cur: ceiling, rlim_max: ceiling };
                if libc::setrlimit(resource, &limit) != 0 { return Err(std::io::Error::last_os_error()); }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|error| format!("bubblewrap: {error}"))?;
    let output = child.stdout.take().ok_or("worker stdout missing")?;
    let overflow = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&overflow);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output.take(crate::MAX_RESPONSE + 1).read_to_end(&mut bytes);
        if bytes.len() as u64 > crate::MAX_RESPONSE { observed.store(true, Ordering::Relaxed); }
        result.map(|_| bytes)
    });
    let stderr = child.stderr.take().ok_or("worker stderr missing")?;
    let observed = Arc::clone(&overflow);
    let error_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(16 * 1024 + 1).read_to_end(&mut bytes);
        if bytes.len() > 16 * 1024 { observed.store(true, Ordering::Relaxed); }
        result.map(|_| bytes)
    });
    let started = std::time::Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= timeout || overflow.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            break Err("import cancelled, timed out or worker response exceeded limit".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break if status.success() { Ok(()) } else { Err(format!("isolated worker refused media or sandbox setup failed ({status}; doctor checks availability)")) },
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(error) => { let _ = child.kill(); let _ = child.wait(); break Err(error.to_string()); }
        }
    };
    let bytes = reader.join().map_err(|_| "worker pipe reader failed")?.map_err(|error| error.to_string())?;
    let stderr = error_reader.join().map_err(|_| "worker error reader failed")?.map_err(|error| error.to_string())?;
    if let Err(error) = result {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(message) = value.get("error").and_then(|value| value.as_str()) {
            return Err(message.chars().take(1024).collect());
        }
        let detail: String = String::from_utf8_lossy(&stderr).chars().filter(|character| !character.is_control()).take(512).collect();
        return Err(if detail.is_empty() { error } else { format!("{error}: {detail}") });
    }
    if bytes.len() as u64 > crate::MAX_RESPONSE { return Err("worker response limit".into()); }
    Ok(bytes)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn run(_root: &Path, _input: Option<&Path>, _operation: &str, _cancel: &std::sync::atomic::AtomicBool, _timeout: std::time::Duration) -> Result<Vec<u8>, String> {
    worker_restrictions()?;
    unreachable!()
}
