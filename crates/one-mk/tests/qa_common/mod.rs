//! Shared lab for the end-to-end QA tests (`qa_daemon_e2e.rs`, `qa_cli_surface.rs`,
//! `qa_clipboard_e2e.rs`): real `one-mk` binaries, private homes, `--backend null`.
//!
//! Every test calls [`isolated`] first. It re-runs the test inside a private
//! network namespace (loopback only, one fresh `lo`) when `unshare` allows it,
//! so a test can never reach the real network, mDNS group or a real oneMK; where
//! `unshare` is not allowed it carries on on plain loopback, which every test
//! is written to be safe on as well (no discovery, private homes, a dead D-Bus).
#![allow(dead_code)]

use one_mk_ctl::client::{Request, request};
use one_mk_ctl::status::Status;
use one_mk_net::{Fingerprint, Identity, TrustStore};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const BIN: &str = env!("CARGO_BIN_EXE_one-mk");
const NETNS: &str = "ONE_MK_QA_NETNS";

fn path_env() -> std::ffi::OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

fn uid_gid() -> (u32, u32) {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata("/proc/self").expect("/proc/self");
    (meta.uid(), meta.gid())
}

/// Whether the calling test should go on in this process. `false`: it was
/// re-run in a private network namespace and that run passed (or failed the
/// assertion here); the caller returns at once.
pub fn isolated(test: &str) -> bool {
    if std::env::var_os(NETNS).is_some() {
        return true;
    }
    let (uid, gid) = uid_gid();
    let wrap = |args: &[&str]| {
        let mut command = Command::new("unshare");
        command
            .args([
                "-Un",
                &format!("--map-user={uid}"),
                &format!("--map-group={gid}"),
                "--keep-caps",
                "sh",
                "-c",
                "ip link set lo up && ip addr add 192.168.77.1/32 dev lo && exec \"$@\"",
                "qa",
            ])
            .args(args);
        command
    };
    match wrap(&["true"]).stdin(Stdio::null()).output() {
        Ok(output) if output.status.success() => {}
        _ => return true,
    }
    let exe = std::env::current_exe().unwrap();
    let output = wrap(&[exe.to_str().unwrap(), "--exact", test, "--nocapture", "--test-threads=1"])
        .env(NETNS, "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{test} failed in a private network namespace:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

/// An address the other computer could dial that is not loopback (an invitation
/// refuses loopback): the one given to `lo` inside the private namespace, else a
/// real interface's.
pub fn reachable_address() -> Option<String> {
    if std::env::var_os(NETNS).is_some() {
        return Some("192.168.77.1".to_owned());
    }
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .map(|interface| interface.ip())
        .find(|address| address.is_ipv4() && one_mk_net::invite::usable_address(*address))
        .map(|address| address.to_string())
}

/// Whether each process can have a host name of its own (so two computers are
/// "alpha" and "beta", not twins).
pub fn names_work() -> bool {
    static NAMES: OnceLock<bool> = OnceLock::new();
    *NAMES.get_or_init(|| {
        std::env::var_os(NETNS).is_some()
            && Command::new("unshare")
                .args(["-u", "sh", "-c", "hostname qaprobe && hostname"])
                .stdin(Stdio::null())
                .output()
                .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "qaprobe")
    })
}

#[track_caller]
pub fn wait<T>(what: &str, seconds: u64, mut condition: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(value) = condition() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out after {seconds} s waiting for: {what}");
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// What a finished command printed.
#[derive(Debug)]
pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn both(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    #[track_caller]
    pub fn success(self) -> String {
        assert!(self.ok(), "failed ({:?}): {}{}", self.code, self.stderr, self.stdout);
        self.stdout
    }

    /// Non-zero exit with a message on stderr, never a panic or a backtrace.
    #[track_caller]
    pub fn failed(&self) -> &str {
        assert!(
            !self.ok() && self.code.is_some(),
            "expected a clean failure, got {:?}: {}{}",
            self.code,
            self.stderr,
            self.stdout
        );
        assert!(
            !self.stderr.contains("panicked") && !self.stderr.contains("RUST_BACKTRACE"),
            "a panic: {}",
            self.stderr
        );
        assert!(!self.stderr.trim().is_empty(), "failed without saying why (stdout: {})", self.stdout);
        &self.stderr
    }
}

/// A command line process whose stdout is read line by line.
pub struct Proc {
    pub child: Child,
    lines: Receiver<String>,
    seen: Vec<String>,
    stderr: Arc<Mutex<String>>,
}

impl Proc {
    pub fn spawn(mut command: Command) -> Self {
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let (tx, lines) = channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let (sink, err) = (stderr.clone(), child.stderr.take().unwrap());
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let mut sink = sink.lock().unwrap();
                sink.push_str(&line);
                sink.push('\n');
            }
        });
        Self { child, lines, seen: Vec::new(), stderr }
    }

    /// The next stdout line that starts with `prefix` (earlier ones are kept in `seen`).
    #[track_caller]
    pub fn expect(&mut self, prefix: &str, seconds: u64) -> String {
        self.try_expect(prefix, seconds).unwrap_or_else(|| {
            panic!("no `{prefix}` line within {seconds} s; stdout: {:?}; stderr: {}", self.seen, self.stderr())
        })
    }

    pub fn try_expect(&mut self, prefix: &str, seconds: u64) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if line.starts_with(prefix) {
                        return Some(line);
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    pub fn say(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        let _ = writeln!(stdin, "{line}").and_then(|()| stdin.flush());
    }

    pub fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    pub fn seen(&self) -> &[String] {
        &self.seen
    }

    /// Waits for the end (killing it after `seconds`); everything it printed.
    #[track_caller]
    pub fn finish(mut self, seconds: u64) -> Out {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        std::thread::sleep(Duration::from_millis(60));
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push(line);
        }
        let stdout = self.seen.iter().map(|line| format!("{line}\n")).collect::<String>();
        assert!(status.is_some(), "did not finish within {seconds} s; stdout: {stdout}; stderr: {}", self.stderr());
        Out { code: status.and_then(|status| status.code()), stdout, stderr: self.stderr() }
    }

    pub fn signal(&self, signal: i32) {
        unsafe { libc::kill(self.child.id() as i32, signal) };
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One computer: a private settings folder, and a daemon when started.
#[derive(Clone)]
pub struct Node {
    pub name: String,
    pub home: PathBuf,
    pub downloads: PathBuf,
    pub port: u16,
    /// Extra environment for every process of this computer (a private `DISPLAY`).
    pub env: Vec<(String, String)>,
}

pub struct Lab {
    pub root: tempfile::TempDir,
    daemons: Vec<(String, Child)>,
    extra: Vec<Child>,
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

impl Lab {
    pub fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::Builder::new().prefix("qa-").tempdir().unwrap();
        // Hooks are refused when the settings folder or any folder above it is writable by a group.
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { root, daemons: Vec::new(), extra: Vec::new() }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// A computer with a private settings folder and an identity (no daemon yet).
    pub fn node(&self, name: &str) -> Node {
        use std::os::unix::fs::PermissionsExt;
        let home = self.root.path().join(name);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        // A daemon checks for a new release when it starts: never from a test, whatever network it is on.
        std::fs::write(home.join("settings.toml"), "[update]\ncheck = false\n").unwrap();
        std::fs::set_permissions(home.join("settings.toml"), std::fs::Permissions::from_mode(0o600)).unwrap();
        let downloads = self.root.path().join(format!("{name}-dl"));
        std::fs::create_dir_all(&downloads).unwrap();
        Node { name: name.to_owned(), home, downloads, port: free_udp_port(), env: Vec::new() }
    }

    /// A command as `node` would run it: its own home, its own host name, no desktop.
    pub fn command(node: &Node, args: &[&str]) -> Command {
        let mut command = if names_work() {
            let mut command = Command::new("unshare");
            // `hostname` refuses names with odd characters; the system call does not.
            command.args([
                "-u",
                "sh",
                "-c",
                "hostname \"$0\" 2>/dev/null || python3 -c 'import socket,sys; socket.sethostname(sys.argv[1])' \"$0\" && exec \"$@\"",
                &node.name,
                BIN,
            ]);
            command
        } else {
            Command::new(BIN)
        };
        command
            .env_clear()
            .env("PATH", path_env())
            .env("HOME", &node.home)
            .env("ONE_MK_HOME", &node.home)
            .env("XDG_RUNTIME_DIR", &node.home)
            .env("NO_COLOR", "1")
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/dead");
        for (name, value) in &node.env {
            command.env(name, value);
        }
        command.args(args);
        command
    }

    /// Runs a command to the end (killed after 90 s).
    #[track_caller]
    pub fn run(node: &Node, args: &[&str]) -> Out {
        Self::run_with(node, args, |_| {})
    }

    #[track_caller]
    pub fn run_with(node: &Node, args: &[&str], tweak: impl FnOnce(&mut Command)) -> Out {
        let mut command = Self::command(node, args);
        tweak(&mut command);
        let mut proc = Proc::spawn(command);
        drop(proc.child.stdin.take());
        proc.finish(90)
    }

    #[track_caller]
    pub fn ok(node: &Node, args: &[&str]) -> String {
        Self::run(node, args).success()
    }

    /// Starts `one-mk run` on the null backend, no discovery, nothing shared with the desktop.
    pub fn start(&mut self, node: &Node, extra: &[&str]) {
        let log = std::fs::OpenOptions::new().create(true).append(true).open(node.home.join("daemon.log")).unwrap();
        let port = node.port.to_string();
        let downloads = node.downloads.to_str().unwrap().to_owned();
        let mut args = vec![
            "run",
            "--backend",
            "null",
            "--no-discovery",
            "--no-notify",
            "--port",
            &port,
            "--downloads",
            &downloads,
        ];
        args.extend_from_slice(extra);
        let child = Self::command(node, &args)
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        self.daemons.push((node.name.clone(), child));
        let index = self.daemons.len() - 1;
        wait(&format!("{} to answer status", node.name), 30, || {
            assert!(
                self.daemons[index].1.try_wait().unwrap().is_none(),
                "daemon {} exited: {}",
                node.name,
                std::fs::read_to_string(node.home.join("daemon.log")).unwrap_or_default()
            );
            self.status_of(node)
        });
    }

    /// A daemon with no clipboard and no friends: the usual test computer.
    pub fn start_plain(&mut self, node: &Node) {
        self.start(node, &["--no-clipboard"]);
    }

    pub fn status_of(&self, node: &Node) -> Option<Status> {
        let reply = request(&node.home, Request::Status, Duration::from_secs(2)).ok().flatten()?;
        Status::decode(&reply)
    }

    #[track_caller]
    pub fn status(&self, node: &Node) -> Status {
        self.status_of(node).unwrap_or_else(|| panic!("{} is not answering status", node.name))
    }

    fn daemon_index(&self, node: &Node) -> usize {
        self.daemons
            .iter()
            .rposition(|(name, _)| *name == node.name)
            .unwrap_or_else(|| panic!("{} was never started", node.name))
    }

    pub fn daemon_pid(&self, node: &Node) -> u32 {
        self.daemons[self.daemon_index(node)].1.id()
    }

    /// `kill -9`: no chance to clean up.
    pub fn kill9(&mut self, node: &Node) {
        let index = self.daemon_index(node);
        let child = &mut self.daemons[index].1;
        let _ = child.kill();
        let _ = child.wait();
    }

    /// SIGTERM, then wait for it to go.
    pub fn terminate(&mut self, node: &Node) {
        let index = self.daemon_index(node);
        let child = &mut self.daemons[index].1;
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "{} did not stop on SIGTERM", node.name);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn daemon_alive(&mut self, node: &Node) -> bool {
        let index = self.daemon_index(node);
        self.daemons[index].1.try_wait().unwrap().is_none()
    }

    pub fn log(node: &Node) -> String {
        std::fs::read_to_string(node.home.join("daemon.log")).unwrap_or_default()
    }

    /// Adopts a process the test started so it is killed at the end.
    pub fn adopt(&mut self, child: Child) {
        self.extra.push(child);
    }

    /// Gives `node` a private X server (Xvfb) and clipboard tools, so its daemon shares a real X
    /// clipboard that nothing else can see. `false` when Xvfb or xclip is not installed.
    pub fn display(&mut self, node: &mut Node) -> bool {
        let have = |tool: &str| {
            Command::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if !have("Xvfb") || !have("xclip") {
            return false;
        }
        let first = 300 + u32::from(node.port) % 500;
        for attempt in 0..40 {
            let number = 300 + (first - 300 + attempt * 7) % 500;
            let socket = format!("/tmp/.X11-unix/X{number}");
            if Path::new(&socket).exists() || Path::new(&format!("/tmp/.X{number}-lock")).exists() {
                continue;
            }
            let mut child = Command::new("Xvfb")
                .args([&format!(":{number}"), "-screen", "0", "640x480x24", "-nolisten", "tcp"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let up = (0..50).any(|_| {
                std::thread::sleep(Duration::from_millis(100));
                child.try_wait().unwrap().is_some() || Path::new(&socket).exists()
            });
            if up && child.try_wait().unwrap().is_none() {
                self.extra.push(child);
                node.env.push(("DISPLAY".into(), format!(":{number}")));
                return true;
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("no free X display number");
    }

    /// Pairs two computers over loopback with `pair --machine`, both answering `yes`.
    #[track_caller]
    pub fn pair(&self, a: &Node, b: &Node) {
        let (mut first, mut second) = self.pair_start(a, b);
        first.expect("code", 20);
        second.expect("code", 20);
        first.say("yes");
        second.say("yes");
        first.expect("paired", 20);
        second.expect("paired", 20);
        let (one, two) = (first.finish(20), second.finish(20));
        assert!(one.ok() && two.ok(), "{one:?} {two:?}");
        wait("both daemons to see each other online", 30, || {
            let up =
                |node: &Node| self.status_of(node).is_none_or(|status| status.peers.iter().all(|peer| peer.online));
            (up(a) && up(b)).then_some(())
        });
    }

    /// `a` waits for a pairing, `b` connects to it; both ready for an answer.
    #[track_caller]
    pub fn pair_start(&self, a: &Node, b: &Node) -> (Proc, Proc) {
        let mut host = Proc::spawn(Self::command(a, &["pair", "--machine", "--no-discovery"]));
        let me = host.expect("me\t", 20);
        let port = me.split('\t').nth(3).unwrap().to_owned();
        let joiner = Proc::spawn(Self::command(
            b,
            &["pair", "--machine", "--no-discovery", "--connect", &format!("127.0.0.1:{port}")],
        ));
        (host, joiner)
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for (_, child) in &mut self.daemons {
            let _ = child.kill();
            let _ = child.wait();
        }
        for child in &mut self.extra {
            let _ = child.kill();
            let _ = child.wait();
        }
        if std::thread::panicking() {
            if let Ok(entries) = std::fs::read_dir(self.root.path()) {
                for entry in entries.flatten() {
                    if let Ok(log) = std::fs::read_to_string(entry.path().join("daemon.log")) {
                        eprintln!("--- {} daemon.log\n{log}", entry.path().display());
                    }
                }
            }
        }
    }
}

pub fn fingerprint(node: &Node) -> Fingerprint {
    Identity::load_or_create(&node.home).unwrap().fingerprint
}

pub fn short_id(node: &Node) -> String {
    fingerprint(node).short()
}

pub fn trusted(node: &Node) -> TrustStore {
    TrustStore::load(&node.home).unwrap()
}

/// The text of a file as lossless UTF-8 or a hash-like summary for binary.
pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Deterministic pseudo-random bytes (so a corrupted transfer cannot match by accident).
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}
