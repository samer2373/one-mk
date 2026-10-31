use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
pub const TASKS: u64 = 64;
const LIMITS: [(&str, u64); 4] = [
    ("memory.max", MEMORY_BYTES), ("memory.swap.max", 0),
    ("memory.oom.group", 1), ("pids.max", TASKS),
];
static NEXT_GROUP: AtomicU64 = AtomicU64::new(0);
/// A playback worker may use at most one CPU (`cpu.max`), when the delegation
/// offers the cpu controller; decoding slower than real time drops frames.
const PLAYBACK_CPU_MAX: &str = "100000 100000";

/// Which worker a group is for; the two never share a delegation lock holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose { Import, Playback }

impl Purpose {
    fn prefix(self) -> &'static str {
        match self { Self::Import => "one-mk-import-", Self::Playback => "one-mk-play-" }
    }
}

pub(crate) struct Group {
    parent: File,
    directory: File,
    name: std::ffi::CString,
    membership: File,
    kill: File,
    removed: bool,
}

fn open_at(directory: RawFd, name: &std::ffi::CStr, flags: libc::c_int) -> Result<File, String> {
    let descriptor = unsafe { libc::openat(directory, name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if descriptor < 0 { return Err(format!("cgroup {}: {}", name.to_string_lossy(), std::io::Error::last_os_error())); }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn control(directory: &File, name: &str, write: bool) -> Result<File, String> {
    let name = std::ffi::CString::new(name).map_err(|_| "invalid cgroup control")?;
    open_at(directory.as_raw_fd(), &name, if write { libc::O_WRONLY } else { libc::O_RDONLY })
}

fn read_control(directory: &File, name: &str) -> Result<String, String> {
    let mut text = String::new();
    control(directory, name, false)?.take(4097).read_to_string(&mut text).map_err(|error| error.to_string())?;
    if text.len() > 4096 { return Err("oversized cgroup control".into()); }
    Ok(text)
}

fn set_limit(directory: &File, name: &str, value: u64) -> Result<(), String> {
    control(directory, name, true)?.write_all(value.to_string().as_bytes()).map_err(|error| format!("cgroup {name}: {error}"))?;
    if read_control(directory, name)?.trim().parse::<u64>() != Ok(value) {
        return Err(format!("cgroup {name} ceiling readback mismatch"));
    }
    Ok(())
}

fn open_root(path: &Path) -> Result<File, String> {
    if !path.is_absolute() || path.as_os_str().as_bytes().split(|byte| *byte == b'/').any(|part| part == b"." || part == b"..") {
        return Err("cgroup root must be absolute without dot/parent components".into());
    }
    let mut directory = File::open("/").map_err(|error| error.to_string())?;
    for component in path.components() {
        if component == Component::RootDir { continue; }
        let Component::Normal(name) = component else { return Err("invalid cgroup root".into()); };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| "NUL in cgroup root")?;
        directory = open_at(directory.as_raw_fd(), &name, libc::O_RDONLY | libc::O_DIRECTORY)?;
        let metadata = directory.metadata().map_err(|error| error.to_string())?;
        if metadata.mode() & 0o022 != 0 || (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }) {
            return Err("untrusted cgroup root ancestor".into());
        }
    }
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(directory.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { filesystem.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err("cgroup root is not a cgroup v2 filesystem".into());
    }
    Ok(directory)
}

fn populated(events: &str) -> Result<bool, String> {
    let mut values = events.lines().filter_map(|line| line.strip_prefix("populated "));
    let value = match values.next() {
        Some("0") => false,
        Some("1") => true,
        _ => return Err("invalid cgroup.events populated state".into()),
    };
    if values.next().is_some() { return Err("duplicate cgroup.events populated state".into()); }
    Ok(value)
}

impl Group {
    pub(crate) fn create(root: Option<&Path>, purpose: Purpose) -> Result<Self, String> {
        let parent = open_root(root.ok_or("select an explicitly delegated --cgroup-root (memory and pids controllers required)")?)?;
        if unsafe { libc::flock(parent.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("another importer holds this cgroup delegation; retry when it finishes".into());
        }
        let controllers = read_control(&parent, "cgroup.subtree_control")?;
        if !["memory", "pids"].iter().all(|required| controllers.split_whitespace().any(|entry| entry == *required)) {
            return Err("delegated cgroup root must already enable memory and pids controllers; no host controllers were changed".into());
        }
        if read_control(&parent, "cgroup.type")?.trim() != "domain" { return Err("cgroup root must be a domain".into()); }
        if purpose == Purpose::Playback { remove_stale(&parent, purpose)?; }
        let cpu = purpose == Purpose::Playback && controllers.split_whitespace().any(|entry| entry == "cpu");
        let name = std::ffi::CString::new(format!("{}{}-{}", purpose.prefix(), std::process::id(), NEXT_GROUP.fetch_add(1, Ordering::Relaxed))).map_err(|_| "invalid group name")?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(format!("create delegated import cgroup: {}", std::io::Error::last_os_error()));
        }
        let setup = (|| {
            let directory = open_at(parent.as_raw_fd(), &name, libc::O_RDONLY | libc::O_DIRECTORY)?;
            for (control, value) in LIMITS { set_limit(&directory, control, value)?; }
            if cpu {
                control(&directory, "cpu.max", true)?.write_all(PLAYBACK_CPU_MAX.as_bytes()).map_err(|error| format!("cgroup cpu.max: {error}"))?;
                if read_control(&directory, "cpu.max")?.trim() != PLAYBACK_CPU_MAX { return Err("cgroup cpu.max readback mismatch".into()); }
            }
            if populated(&read_control(&directory, "cgroup.events")?)? { return Err("new import cgroup is unexpectedly populated".into()); }
            let membership = control(&directory, "cgroup.procs", true)?;
            let kill = control(&directory, "cgroup.kill", true)?;
            Ok((directory, membership, kill))
        })();
        match setup {
            Ok((directory, membership, kill)) => Ok(Self { parent, directory, name, membership, kill, removed: false }),
            Err(error) => {
                unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR); }
                Err(error)
            }
        }
    }

    pub(crate) fn membership_fd(&self) -> RawFd { self.membership.as_raw_fd() }

    pub(crate) fn finish(&mut self) -> Result<(), String> {
        if self.removed { return Ok(()); }
        self.kill.write_all(b"1").map_err(|error| format!("kill import cgroup: {error}"))?;
        let started = std::time::Instant::now();
        while populated(&read_control(&self.directory, "cgroup.events")?)? {
            if started.elapsed() >= std::time::Duration::from_secs(2) { return Err("import cgroup remains populated after kill".into()); }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err(format!("remove import cgroup: {}", std::io::Error::last_os_error()));
        }
        self.removed = true;
        Ok(())
    }
}

impl Drop for Group {
    fn drop(&mut self) { let _ = self.finish(); }
}

/// Removes groups of this purpose left behind by a renderer that was killed
/// before it could clean up. Safe only because the caller holds the root's
/// exclusive lock: every live owner of such a group would hold that lock.
fn remove_stale(parent: &File, purpose: Purpose) -> Result<(), String> {
    let listing = std::path::PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()));
    for entry in std::fs::read_dir(&listing).map_err(|error| error.to_string())?.take(4096) {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        let Some(rest) = text.strip_prefix(purpose.prefix()) else { continue };
        if !rest.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-') { continue; }
        let name = std::ffi::CString::new(text).map_err(|_| "invalid stale group name")?;
        let directory = open_at(parent.as_raw_fd(), &name, libc::O_RDONLY | libc::O_DIRECTORY)?;
        if populated(&read_control(&directory, "cgroup.events")?)? {
            control(&directory, "cgroup.kill", true)?.write_all(b"1").map_err(|error| format!("kill stale group: {error}"))?;
            let started = std::time::Instant::now();
            while populated(&read_control(&directory, "cgroup.events")?)? {
                if started.elapsed() >= std::time::Duration::from_secs(2) { return Err("stale playback group stays populated".into()); }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err(format!("remove stale playback group: {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

pub(crate) fn attach(descriptor: RawFd) -> std::io::Result<()> {
    if unsafe { libc::write(descriptor, b"0".as_ptr().cast(), 1) } != 1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_delegation_means_no_worker() {
        assert!(Group::create(None, Purpose::Import).err().unwrap().contains("--cgroup-root"));
        assert!(Group::create(None, Purpose::Playback).err().unwrap().contains("--cgroup-root"));
        for root in ["relative", "/sys/../sys/fs/cgroup", "/sys/./fs/cgroup"] {
            assert!(open_root(Path::new(root)).is_err(), "{root}");
        }
        assert!(open_root(Path::new("/")).unwrap_err().contains("not a cgroup v2"));
    }

    #[test]
    fn events_are_strict_and_bounded() {
        assert!(!populated("populated 0\nfrozen 0\n").unwrap());
        assert!(populated("frozen 0\npopulated 1\n").unwrap());
        for events in ["", "populated max", "populated 2", "populated 0\npopulated 1"] {
            assert!(populated(events).is_err(), "{events}");
        }
    }

    #[test]
    fn controls_never_follow_symlinks_or_create_missing_files() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let directory = File::open(temporary.path()).unwrap();
        let target = temporary.path().join("target");
        std::fs::write(&target, "unchanged").unwrap();
        symlink(&target, temporary.path().join("memory.max")).unwrap();
        assert!(set_limit(&directory, "memory.max", MEMORY_BYTES).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
        assert!(set_limit(&directory, "pids.max", TASKS).is_err());
        assert!(!temporary.path().join("pids.max").exists());
        std::fs::write(temporary.path().join("cgroup.events"), "x".repeat(4097)).unwrap();
        assert!(read_control(&directory, "cgroup.events").is_err());
    }

    #[test]
    fn failed_membership_write_refuses_exec() {
        let file = File::open("/dev/null").unwrap();
        assert!(attach(file.as_raw_fd()).is_err());
    }

    #[test]
    fn limit_readback_must_match_exactly() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = File::open(temporary.path()).unwrap();
        std::fs::write(temporary.path().join("memory.max"), "max\n").unwrap();
        assert!(set_limit(&directory, "memory.max", 0).unwrap_err().contains("readback mismatch"));
    }

    #[test]
    #[ignore = "requires an explicit private writable cgroup v2 delegation; never use the live session"]
    fn delegated_kernel_limits_attach_and_cleanup() {
        use std::os::unix::process::CommandExt;
        let root = std::env::var_os("ONE_MK_WALL_TEST_CGROUP_ROOT").expect("explicit private delegation required");
        let mut group = Group::create(Some(Path::new(&root)), Purpose::Import).unwrap();
        assert!(Group::create(Some(Path::new(&root)), Purpose::Import).err().unwrap().contains("another importer"));
        for (control, value) in LIMITS {
            assert_eq!(read_control(&group.directory, control).unwrap().trim().parse::<u64>().unwrap(), value);
        }
        let descriptor = group.membership_fd();
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30").env_clear();
        unsafe { command.pre_exec(move || attach(descriptor)); }
        let mut child = command.spawn().unwrap();
        let membership = read_control(&group.directory, "cgroup.procs");
        let cleanup = group.finish();
        let status = child.wait().unwrap();
        assert!(membership.unwrap().lines().any(|pid| pid == child.id().to_string()));
        cleanup.unwrap();
        assert!(!status.success());
        assert!(group.removed);
        group.finish().unwrap();
    }

    #[test]
    #[ignore = "requires explicit private cgroup delegation and /usr/bin/python3; allocates at most 96 MiB"]
    fn delegated_kernel_memory_and_task_pressure() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let root = std::env::var_os("ONE_MK_WALL_TEST_CGROUP_ROOT").expect("explicit private delegation required");
        for (limit, value, script, event) in [
            ("memory.max", 32 * 1024 * 1024, "memory = bytearray(96 * 1024 * 1024)", "memory.events"),
            ("pids.max", 4, "import os, errno, signal\nchildren = []\ntry:\n for attempt in range(32):\n  child = os.fork()\n  if child == 0:\n   signal.pause()\n   os._exit(0)\n  children.append(child)\n raise RuntimeError('task ceiling not enforced')\nexcept OSError as error:\n if error.errno != errno.EAGAIN: raise\nfinally:\n for child in children: os.kill(child, signal.SIGKILL)\n for child in children: os.waitpid(child, 0)", "pids.events"),
        ] {
            let mut group = Group::create(Some(Path::new(&root)), Purpose::Import).unwrap();
            set_limit(&group.directory, limit, value).unwrap();
            let descriptor = group.membership_fd();
            let mut command = Command::new("/usr/bin/python3");
            command.args(["-I", "-S", "-c", script]).env_clear().stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            unsafe { command.pre_exec(move || attach(descriptor)); }
            let mut child = command.spawn().unwrap();
            let started = std::time::Instant::now();
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if started.elapsed() < std::time::Duration::from_secs(5) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    _ => { let _ = child.kill(); let _ = child.wait(); break Err(std::io::Error::other("pressure worker timed out or wait failed")); }
                }
            };
            let events = read_control(&group.directory, event);
            let cleanup = group.finish();
            let status = status.unwrap();
            let events = events.unwrap();
            cleanup.unwrap();
            let counter = if event == "memory.events" { "oom_kill " } else { "max " };
            assert!(events.lines().any(|line| line.strip_prefix(counter).and_then(|value| value.parse::<u64>().ok()).is_some_and(|value| value > 0)), "{events}");
            assert_eq!(status.success(), event == "pids.events");
        }
    }
}
