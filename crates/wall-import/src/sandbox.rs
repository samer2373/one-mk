use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const GST_VERSION: &str = "1.28.2";
pub const SCRATCH_BYTES: u64 = 16 * 1024 * 1024;
pub const DEV_BYTES: u64 = 64 * 1024;

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

pub fn doctor(runtime: Option<&Path>, cgroup_root: Option<&Path>) -> Availability {
    let result = runtime.ok_or_else(|| "no audited runtime selected (--runtime DIR)".to_string())
        .and_then(validate_runtime)
        .and_then(|root| run(&root, cgroup_root, None, "--sandbox-check", &std::sync::atomic::AtomicBool::new(false), std::time::Duration::from_secs(5)).map(|_| ()));
    match result {
        Ok(()) => Availability { available: true, reason: "bubblewrap namespaces, external seccomp, read-only bounded /dev, 16 MiB scratch, per-process rlimits and mandatory cgroup v2 1 GiB memory / zero swap / 64 tasks configured; native hardware and pressure qualification still required".into() },
        Err(reason) => Availability { available: false, reason: format!("sandbox unavailable: refusing to import: {reason}") },
    }
}

pub fn validate_runtime(root: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        let directory = pin_runtime(root)?;
        use std::os::fd::AsRawFd;
        validate_contents(Path::new(&format!("/proc/self/fd/{}", directory.as_raw_fd())))?;
        Ok(root.to_owned())
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = root; Err("runtime sandbox is only implemented on Linux".into()) }
}

#[cfg(target_os = "linux")]
fn pin_runtime(root: &Path) -> Result<std::fs::File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    if !root.is_absolute() { return Err("runtime must be an absolute nonsymlink canonical directory".into()); }
    let mut directory = std::fs::File::open("/").map_err(|error| error.to_string())?;
    for component in root.components() {
        if component == std::path::Component::RootDir { continue; }
        let std::path::Component::Normal(name) = component else { return Err("runtime dot/parent components forbidden".into()); };
        let parent = directory.metadata().map_err(|error| error.to_string())?;
        trusted_owner(&parent)?;
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| "NUL runtime path")?;
        let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if descriptor < 0 { return Err(format!("runtime directory: {}", std::io::Error::last_os_error())); }
        directory = unsafe { std::fs::File::from_raw_fd(descriptor) };
    }
    let metadata = directory.metadata().map_err(|error| error.to_string())?;
    trusted_owner(&metadata)?;
    if !metadata.is_dir() || metadata.mode() & 0o022 != 0 { return Err("untrusted runtime directory".into()); }
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn trusted_owner(metadata: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    if metadata.mode() & 0o022 != 0 || (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }) {
        return Err("runtime entries and every ancestor must be owned by root or caller and not group/world writable".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_contents(root: &Path) -> Result<RuntimeManifest, String> {
    use std::io::Read;
    use sha2::{Digest, Sha256};
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
            trusted_owner(&metadata)?;
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
    let plugins: std::collections::BTreeSet<_> = actual.keys().filter(|name| name.starts_with("plugins/")).map(String::as_str).collect();
    if plugins != std::collections::BTreeSet::from(["plugins/libgstcoreelements.so", "plugins/libgstmatroska.so", "plugins/libgstvpx.so", "plugins/libgstaom.so", "plugins/libgstvideoconvertscale.so"]) { return Err("runtime plugin allowlist must match exactly".into()); }
    let names: std::collections::BTreeSet<_> = actual.keys().filter_map(|name| Path::new(name).file_name()?.to_str()).collect();
    for (name, license) in &manifest.licenses {
        let lower = name.to_ascii_lowercase();
        if ["ffmpeg", "libavcodec", "libavformat", "libavfilter", "libavdevice", "libavutil", "libswscale", "libswresample", "gstlibav", "x264", "x265", "libmpv"].iter().any(|forbidden| lower.contains(forbidden)) {
            return Err("forbidden codec in runtime closure".into());
        }
        if !["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "bzip2-1.0.6", "LGPL-2.0-or-later", "LGPL-2.1-or-later", "GPL-3.0-or-later WITH GCC-exception-3.1"].contains(&license.as_str()) {
            return Err("unknown or incompatible runtime licence; refusing import".into());
        }
        crate::runtime_elf::validate(&std::fs::read(root.join(name)).map_err(|error| error.to_string())?, &names)?;
    }
    Ok(manifest)
}

pub fn runtime_inventory(root: &Path) -> Result<RuntimeManifest, String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let directory = pin_runtime(root)?;
        validate_contents(Path::new(&format!("/proc/self/fd/{}", directory.as_raw_fd())))
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = root; Err("runtime inventory requires Linux".into()) }
}

#[cfg(target_os = "linux")]
pub fn worker_restrictions() -> Result<(), String> {
    if std::env::var("ONE_MK_WALL_SANDBOX").as_deref() != Ok("1") { return Err("worker must run inside the import sandbox".into()); }
    if unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } != 1 || unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) } != 2 { return Err("external seccomp/no_new_privs missing".into()); }
    Ok(())
}

#[cfg(target_os = "linux")]
fn external_filter() -> Result<std::fs::File, String> {
    use std::io::{Seek, Write};
    let architecture = if cfg!(target_arch = "x86_64") { 0xc000003e } else if cfg!(target_arch = "aarch64") { 0xc00000b7 } else { return Err("seccomp architecture unqualified".into()); };
    let instruction = |code, jump_true, jump_false, value| libc::sock_filter { code, jt: jump_true, jf: jump_false, k: value };
    let mut filter = vec![
        instruction(0x20, 0, 0, 4), instruction(0x15, 1, 0, architecture), instruction(0x06, 0, 0, 0x80000000), instruction(0x20, 0, 0, 0),
        instruction(0x35, 0, 1, 0x40000000), instruction(0x06, 0, 0, 0x80000000),
    ];
    for syscall in [libc::SYS_socket, libc::SYS_connect, libc::SYS_bind, libc::SYS_listen, libc::SYS_accept, libc::SYS_accept4, libc::SYS_ptrace, libc::SYS_mount, libc::SYS_umount2, libc::SYS_fsopen, libc::SYS_fsmount, libc::SYS_move_mount, libc::SYS_open_tree, libc::SYS_mount_setattr, libc::SYS_fspick, libc::SYS_unshare, libc::SYS_setns, libc::SYS_bpf, libc::SYS_keyctl, libc::SYS_add_key, libc::SYS_request_key, libc::SYS_perf_event_open, libc::SYS_process_vm_readv, libc::SYS_process_vm_writev, libc::SYS_io_uring_setup, libc::SYS_io_uring_enter, libc::SYS_io_uring_register, libc::SYS_userfaultfd, libc::SYS_open_by_handle_at, libc::SYS_memfd_create, libc::SYS_shmget, libc::SYS_shmat] {
        filter.push(instruction(0x15, 0, 1, syscall as u32));
        filter.push(instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32));
    }
    filter.extend([
        instruction(0x15, 0, 4, libc::SYS_mmap as u32), instruction(0x20, 0, 0, 40),
        instruction(0x45, 0, 1, libc::MAP_HUGETLB as u32), instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
        instruction(0x20, 0, 0, 0),
        instruction(0x15, 0, 1, libc::SYS_clone3 as u32), instruction(0x06, 0, 0, 0x00050000 | libc::ENOSYS as u32),
        instruction(0x15, 0, 3, libc::SYS_clone as u32), instruction(0x20, 0, 0, 16),
        instruction(0x45, 0, 1, 0x7e020080), instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
        instruction(0x20, 0, 0, 0),
    ]);
    filter.extend([
        instruction(0x15, 0, 4, libc::SYS_socketpair as u32), instruction(0x20, 0, 0, 16),
        instruction(0x15, 1, 0, libc::AF_UNIX as u32), instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
        instruction(0x06, 0, 0, 0x7fff0000),
    ]);
    filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    use std::os::fd::FromRawFd;
    let descriptor = unsafe { libc::memfd_create(c"wall-import-seccomp".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if descriptor < 0 { return Err(std::io::Error::last_os_error().to_string()); }
    let mut file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    for entry in filter {
        file.write_all(&entry.code.to_ne_bytes()).and_then(|()| file.write_all(&[entry.jt, entry.jf])).and_then(|()| file.write_all(&entry.k.to_ne_bytes())).map_err(|error| error.to_string())?;
    }
    file.rewind().map_err(|error| error.to_string())?;
    if unsafe { libc::fcntl(descriptor, libc::F_ADD_SEALS, libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL) } < 0 { return Err("cannot seal seccomp program".into()); }
    Ok(file)
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
    let memory = unsafe { libc::memfd_create(c"wall-import-denial-check".as_ptr(), libc::MFD_CLOEXEC) };
    if memory >= 0 { unsafe { libc::close(memory); } return Err("anonymous shmem bypasses scratch ceiling".into()); }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) { return Err("memfd denial unverified".into()); }
    let memory = unsafe { libc::shmget(libc::IPC_PRIVATE, 4096, libc::IPC_CREAT | 0o600) };
    if memory >= 0 { unsafe { libc::shmctl(memory, libc::IPC_RMID, std::ptr::null_mut()); } return Err("SysV shmem bypasses scratch ceiling".into()); }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) { return Err("SysV shmem denial unverified".into()); }
    let huge = unsafe { libc::mmap(std::ptr::null_mut(), 4096, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_HUGETLB, -1, 0) };
    if huge != libc::MAP_FAILED { unsafe { libc::munmap(huge, 4096); } return Err("explicit huge pages bypass the memory controller".into()); }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) { return Err("huge-page denial unverified".into()); }
    if unsafe { libc::unshare(libc::CLONE_NEWUSER) } == 0 { return Err("nested user namespace allowed".into()); }
    for path in ["/dev", "/dev/shm", "/proc"] {
        if std::fs::write(format!("{path}/wall-import-write-check"), b"denied").is_ok() { return Err(format!("writable sandbox mount: {path}")); }
    }
    for (path, ceiling, readonly) in [(c"/dev", DEV_BYTES, true), (c"/tmp", SCRATCH_BYTES, false)] {
        let mut status = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), status.as_mut_ptr()) } != 0 { return Err("cannot inspect sandbox filesystem ceilings".into()); }
        let status = unsafe { status.assume_init() };
        if status.f_blocks.saturating_mul(status.f_frsize) > ceiling || (status.f_flag & libc::ST_RDONLY != 0) != readonly { return Err("sandbox tmpfs oversized or incorrect read-only flag".into()); }
    }
    if std::fs::write("/sandbox-write-test", b"denied").is_ok() || std::fs::read("/etc/passwd").is_ok() || std::fs::read_dir("/home").is_ok() { return Err("filesystem denial failed".into()); }
    for (resource, ceiling) in [(libc::RLIMIT_AS, 1024 * 1024 * 1024), (libc::RLIMIT_CPU, 120), (libc::RLIMIT_NOFILE, 64)] {
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if unsafe { libc::getrlimit(resource, &mut limit) } != 0 || limit.rlim_max > ceiling { return Err("resource ceiling not installed".into()); }
    }
    Ok(())
}

#[cfg(all(target_os = "linux", feature = "test-faults"))]
pub fn check_storage_pressure() -> Result<(), String> {
    use std::io::Write;
    check_worker()?;
    let block = [0x5a; 64 * 1024];
    let mut total = 0u64;
    for index in 0..=SCRATCH_BYTES / block.len() as u64 {
        let mut file = std::fs::File::create(format!("/tmp/pressure-{index}")).map_err(|error| error.to_string())?;
        match file.write_all(&block) {
            Ok(()) => total += block.len() as u64,
            Err(error) if error.raw_os_error() == Some(libc::ENOSPC) => return Ok(()),
            Err(error) => return Err(format!("unexpected scratch pressure error: {error}")),
        }
        if total > SCRATCH_BYTES { return Err("worker exceeded aggregate scratch ceiling".into()); }
    }
    Err("scratch pressure did not reach explicit ENOSPC ceiling".into())
}

#[cfg(all(target_os = "linux", feature = "test-faults"))]
pub fn storage_check(root: &Path, cgroup_root: Option<&Path>) -> Result<(), String> {
    run(root, cgroup_root, None, "--sandbox-storage-check", &std::sync::atomic::AtomicBool::new(false), std::time::Duration::from_secs(10)).map(|_| ())
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
fn read_pipe(mut pipe: impl std::io::Read + std::os::fd::AsRawFd, limit: usize, overflow: &std::sync::atomic::AtomicBool, stopped: &std::sync::atomic::AtomicBool) -> std::io::Result<Vec<u8>> {
    use std::sync::atomic::Ordering;
    let result = (|| {
        let descriptor = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return Err(std::io::Error::last_os_error()); }
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let available = buffer.len().min(limit + 1 - bytes.len());
            match pipe.read(&mut buffer[..available]) {
                Ok(0) => return Ok(bytes),
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.len() > limit { overflow.store(true, Ordering::Relaxed); return Ok(bytes); }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopped.load(Ordering::Relaxed) { return Ok(bytes); }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        }
    })();
    if result.is_err() { overflow.store(true, Ordering::Relaxed); }
    result
}

#[cfg(target_os = "linux")]
pub(crate) fn run(root: &Path, cgroup_root: Option<&Path>, input: Option<&Path>, operation: &str, cancel: &std::sync::atomic::AtomicBool, timeout: std::time::Duration) -> Result<Vec<u8>, String> {
    use std::os::unix::process::CommandExt;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    use std::process::{Command, Stdio};
    use std::os::fd::AsRawFd;
    let mut group = crate::cgroup::Group::create(cgroup_root).map_err(|error| format!("sandbox unavailable: refusing to import: {error}"))?;
    let membership_fd = group.membership_fd();
    let directory = pin_runtime(root)?;
    validate_contents(Path::new(&format!("/proc/self/fd/{}", directory.as_raw_fd())))?;
    let filter = external_filter()?;
    let directory_fd = directory.as_raw_fd();
    let filter_fd = filter.as_raw_fd();
    let parent_pid = unsafe { libc::getpid() };
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new("/usr/bin/bwrap");
    command.args(["--unshare-all", "--unshare-user", "--disable-userns", "--assert-userns-disabled", "--die-with-parent", "--new-session", "--cap-drop", "ALL", "--ro-bind-fd"])
        .arg(directory_fd.to_string()).arg("/").arg("--ro-bind").arg(executable).arg("/worker")
        .arg("--seccomp").arg(filter_fd.to_string())
        .args(["--proc", "/proc", "--remount-ro", "/proc", "--size", "65536", "--tmpfs", "/dev",
            "--dev-bind", "/dev/null", "/dev/null", "--dev-bind", "/dev/zero", "/dev/zero", "--dev-bind", "/dev/urandom", "/dev/urandom",
            "--dir", "/dev/shm", "--remount-ro", "/dev", "--size", "16777216", "--tmpfs", "/tmp", "--chdir", "/tmp", "--clearenv",
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
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 || libc::getppid() != parent_pid { return Err(std::io::Error::other("import parent died")); }
            crate::cgroup::attach(membership_fd)?;
            for descriptor in [directory_fd, filter_fd] {
                if libc::fcntl(descriptor, libc::F_SETFD, 0) < 0 { return Err(std::io::Error::last_os_error()); }
            }
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
    let stopped = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&overflow);
    let reader_stopped = Arc::clone(&stopped);
    let reader = std::thread::spawn(move || read_pipe(output, crate::MAX_RESPONSE as usize, &observed, &reader_stopped));
    let stderr = child.stderr.take().ok_or("worker stderr missing")?;
    let observed = Arc::clone(&overflow);
    let reader_stopped = Arc::clone(&stopped);
    let error_reader = std::thread::spawn(move || read_pipe(stderr, 16 * 1024, &observed, &reader_stopped));
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
    let cleanup = group.finish();
    stopped.store(true, Ordering::Relaxed);
    let bytes = reader.join().map_err(|_| "worker pipe reader failed")?.map_err(|error| error.to_string())?;
    let stderr = error_reader.join().map_err(|_| "worker error reader failed")?.map_err(|error| error.to_string())?;
    cleanup?;
    if overflow.load(Ordering::Relaxed) { return Err("worker output or stderr exceeded limit, or pipe read failed".into()); }
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
pub(crate) fn run(_root: &Path, _cgroup_root: Option<&Path>, _input: Option<&Path>, _operation: &str, _cancel: &std::sync::atomic::AtomicBool, _timeout: std::time::Duration) -> Result<Vec<u8>, String> {
    worker_restrictions()?;
    unreachable!()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn external_filter_denies_hugetlb_without_denying_normal_mappings() {
        use std::io::Read;
        let mut filter = Vec::new();
        external_filter().unwrap().read_to_end(&mut filter).unwrap();
        for (flags, expected) in [(libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, 0x7fff0000), (libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_HUGETLB, 0x00050000 | libc::EPERM as u32)] {
            let mut data = [0u8; 64];
            data[..4].copy_from_slice(&(libc::SYS_mmap as u32).to_ne_bytes());
            let architecture: u32 = if cfg!(target_arch = "x86_64") { 0xc000003e } else { 0xc00000b7 };
            data[4..8].copy_from_slice(&architecture.to_ne_bytes());
            data[40..44].copy_from_slice(&(flags as u32).to_ne_bytes());
            let mut accumulator = 0;
            let mut position = 0;
            let result = loop {
                let entry = &filter[position * 8..position * 8 + 8];
                let value = u32::from_ne_bytes(entry[4..8].try_into().unwrap());
                match u16::from_ne_bytes(entry[..2].try_into().unwrap()) {
                    0x20 => accumulator = u32::from_ne_bytes(data[value as usize..value as usize + 4].try_into().unwrap()),
                    0x06 => break value,
                    code => {
                        let condition = match code { 0x15 => accumulator == value, 0x35 => accumulator >= value, 0x45 => accumulator & value != 0, _ => panic!("unsupported filter instruction") };
                        position += usize::from(if condition { entry[2] } else { entry[3] });
                    }
                }
                position += 1;
            };
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn worker_pipes_are_bounded_even_when_a_descendant_holds_them_open() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"bounded").unwrap();
        let overflow = AtomicBool::new(false);
        let stopped = AtomicBool::new(true);
        assert_eq!(read_pipe(reader, 8, &overflow, &stopped).unwrap(), b"bounded");
        assert!(!overflow.load(Ordering::Relaxed));
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"oversized").unwrap();
        assert_eq!(read_pipe(reader, 3, &overflow, &stopped).unwrap(), b"over");
        assert!(overflow.load(Ordering::Relaxed));
    }

    #[test]
    fn pipe_io_failure_stops_the_supervisor() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let overflow = AtomicBool::new(false);
        let stopped = AtomicBool::new(false);
        let file = std::fs::OpenOptions::new().write(true).open("/dev/null").unwrap();
        assert!(read_pipe(file, 8, &overflow, &stopped).is_err());
        assert!(overflow.load(Ordering::Relaxed));
    }

    #[test]
    fn missing_cgroup_is_refused_before_runtime_or_spawn() {
        let result = run(Path::new("/no-runtime"), None, None, "--sandbox-check", &std::sync::atomic::AtomicBool::new(false), std::time::Duration::ZERO);
        assert!(result.unwrap_err().contains("--cgroup-root"));
    }

    fn runtime(extra: &str) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let lab = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/runtime-policy-tests");
        std::fs::create_dir_all(&lab).unwrap();
        let root = tempfile::tempdir_in(lab.canonicalize().unwrap()).unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut files = std::collections::BTreeMap::new();
        for name in ["plugins/libgstcoreelements.so", "plugins/libgstmatroska.so", "plugins/libgstvpx.so", "plugins/libgstaom.so", "plugins/libgstvideoconvertscale.so", extra] {
            let path = root.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::set_permissions(path.parent().unwrap(), std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(&path, b"policy fixture").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            files.insert(name.to_owned(), Sha256::digest(b"policy fixture").iter().map(|byte| format!("{byte:02x}")).collect::<String>());
        }
        let licenses = files.keys().map(|name| (name.clone(), "MIT".to_owned())).collect();
        std::fs::write(root.path().join("runtime.json"), serde_json::to_vec(&RuntimeManifest { schema: 1, gstreamer: GST_VERSION.into(), files, licenses }).unwrap()).unwrap();
        std::fs::set_permissions(root.path().join("runtime.json"), std::fs::Permissions::from_mode(0o600)).unwrap();
        root
    }

    #[test]
    fn import_time_plugin_allowlist_rejects_openh264() {
        let root = runtime("plugins/libgstopenh264.so");
        let error = validate_contents(root.path()).unwrap_err();
        assert!(error.contains("plugin allowlist"), "{error}");
    }

    #[test]
    fn runtime_rejects_writable_ancestor() {
        use std::os::unix::fs::PermissionsExt;
        let root = runtime("lib/libfixture.so");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_runtime(root.path()).unwrap_err().contains("ancestor"));
    }

}
