#![cfg(target_os = "linux")]
//! A one-mk with a home of its own (`--home`, `ONE_MK_HOME`: a test, or a
//! second one-mk) changes nothing outside that folder when it pairs or
//! forgets. Before, it rewrote the user's file-manager "Send to" entries to
//! run the throwaway binary with the throwaway home.

use one_mk_net::{Device, Identity, TrustStore};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// A user's account: HOME and the XDG folders, all inside `root`.
struct Account {
    root: tempfile::TempDir,
}

impl Account {
    fn new() -> Self {
        let root = tempfile::Builder::new().prefix("one-mk-home-isolation-").tempdir().unwrap();
        for dir in ["user/.config", "user/.local/share/nautilus/scripts", "user/.cache", "run"] {
            std::fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path().join("run"), std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { root }
    }

    fn user(&self) -> PathBuf {
        self.root.path().join("user")
    }

    /// The default home, as one-mk finds it with no `--home`.
    fn default_home(&self) -> PathBuf {
        self.user().join(".config/one-mk")
    }

    /// A home of its own, outside the user's folders.
    fn custom_home(&self, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let home = self.root.path().join(name);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        Identity::load_or_create(&home).unwrap();
        home
    }

    fn command(&self, home: Option<&Path>) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk"));
        let user = self.user();
        command.env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default()).env("HOME", &user)
            .env("XDG_CONFIG_HOME", user.join(".config")).env("XDG_DATA_HOME", user.join(".local/share"))
            .env("XDG_CACHE_HOME", user.join(".cache")).env("XDG_RUNTIME_DIR", self.root.path().join("run"));
        if let Some(home) = home {
            command.arg("--home").arg(home);
        }
        command
    }

    fn cli(&self, home: Option<&Path>, args: &[&str]) -> String {
        let output = self.command(home).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }

    /// Every file and folder outside `except`, with its contents and time.
    fn snapshot(&self, except: &[&Path]) -> BTreeMap<PathBuf, (Vec<u8>, Option<SystemTime>)> {
        fn walk(dir: &Path, except: &[&Path], out: &mut BTreeMap<PathBuf, (Vec<u8>, Option<SystemTime>)>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if except.iter().any(|e| path.starts_with(e)) {
                    continue;
                }
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    out.insert(path.clone(), (Vec::new(), None));
                    walk(&path, except, out);
                } else {
                    out.insert(path.clone(), (std::fs::read(&path).unwrap_or_default(), meta.modified().ok()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.root.path(), except, &mut out);
        out
    }
}

fn add_device(home: &Path, name: &str, fingerprint: &str) {
    TrustStore::update(home, |t| {
        t.upsert(Device { name: name.into(), fingerprint: fingerprint.into(), ..Device::default() });
        Ok(())
    })
    .unwrap();
}

fn send_to_entries(account: &Account) -> Vec<(String, String)> {
    let dir = account.user().join(".local/share/nautilus/scripts");
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read_to_string(e.path()).unwrap_or_default())).collect();
    entries.sort();
    entries
}

#[track_caller]
fn wait_for(child: &mut Child, log: &Path, text: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let output = std::fs::read_to_string(log).unwrap_or_default();
        if output.contains(text) {
            return output;
        }
        assert!(child.try_wait().unwrap().is_none(), "exited early: {output}");
        assert!(Instant::now() < deadline, "no {text:?} within 20 s: {output}");
        std::thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn pair_and_forget_with_a_home_of_its_own_change_nothing_outside_it() {
    let account = Account::new();
    // The user's own one-mk, with "Send to" entries for its computer.
    let default = account.default_home();
    Identity::load_or_create(&default).unwrap();
    add_device(&default, "Studio", &"ab".repeat(32));
    account.cli(None, &["integrate"]);
    let entries = send_to_entries(&account);
    assert!(entries.iter().any(|(_, text)| text.contains("send")), "integrate wrote nothing: {entries:?}");

    // Two homes of their own pair with each other, then forget each other.
    let (a, b) = (account.custom_home("a"), account.custom_home("b"));
    let before = account.snapshot(&[&a, &b]);
    let spawn = |home: &Path, args: &[&str]| {
        let log = home.join("pair.log");
        let file = std::fs::File::create(&log).unwrap();
        let child = account.command(Some(home)).args(args).stdin(Stdio::piped()).stdout(file.try_clone().unwrap()).stderr(file).spawn().unwrap();
        (child, log)
    };
    let (mut host, host_log) = spawn(&a, &["pair", "--machine", "--no-discovery"]);
    let output = wait_for(&mut host, &host_log, "me\t");
    let port = output.lines().find(|line| line.starts_with("me\t")).unwrap().split('\t').nth(3).unwrap().to_owned();
    let (mut joiner, join_log) = spawn(&b, &["pair", "--machine", "--no-discovery", "--connect", &format!("127.0.0.1:{port}")]);
    for (child, log) in [(&mut host, &host_log), (&mut joiner, &join_log)] {
        wait_for(child, log, "code\t");
        writeln!(child.stdin.as_mut().unwrap(), "yes").unwrap();
    }
    for (child, log) in [(&mut host, &host_log), (&mut joiner, &join_log)] {
        let status = child.wait().unwrap();
        assert!(status.success(), "{}", std::fs::read_to_string(log).unwrap());
    }
    assert_eq!(TrustStore::load(&a).unwrap().devices().len(), 1);
    let b_id = Identity::load_or_create(&b).unwrap().fingerprint.to_string();
    account.cli(Some(&a), &["forget", &b_id[..8]]);
    assert!(TrustStore::load(&a).unwrap().devices().is_empty());
    assert_eq!(account.snapshot(&[&a, &b]), before, "a file outside the custom homes changed");
    assert_eq!(send_to_entries(&account), entries);

    // The same through ONE_MK_HOME.
    let output = account.command(None).env("ONE_MK_HOME", &b).args(["forget", &Identity::load_or_create(&a).unwrap().fingerprint.to_string()[..8]]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(account.snapshot(&[&a, &b]), before, "a file outside the custom homes changed");

    // The default home still keeps its entries in step.
    account.cli(None, &["forget", "Studio"]);
    assert_ne!(send_to_entries(&account), entries, "forgetting in the default home updates the entries");
}
