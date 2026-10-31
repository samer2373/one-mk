#![cfg(target_os = "linux")]

use one_mk_net::{Device, Identity, TrustStore};
use one_mk_proto::Side;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Lab {
    root: tempfile::TempDir,
    children: Vec<Child>,
}

impl Lab {
    fn new() -> Self { Self { root: tempfile::Builder::new().prefix("one-mk-m9-test-").tempdir().unwrap(), children: Vec::new() } }

    fn home(&self, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let home = self.root.path().join(name);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        Identity::load_or_create(&home).unwrap();
        home
    }

    fn command(home: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk"));
        command.env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", home).env("ONE_MK_HOME", home).env("XDG_RUNTIME_DIR", home);
        command
    }

    fn cli(home: &Path, args: &[&str]) -> String {
        let output = Self::command(home).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }

    fn spawn(&mut self, home: &Path, args: &[&str], log: &str) -> usize {
        let output = std::fs::File::create(home.join(log)).unwrap();
        let child = Self::command(home).args(args).stdin(Stdio::piped())
            .stdout(output.try_clone().unwrap()).stderr(output).spawn().unwrap();
        self.children.push(child);
        self.children.len() - 1
    }

    fn wait_text(home: &Path, log: &str, text: &str) -> String {
        wait(|| {
            let output = std::fs::read_to_string(home.join(log)).unwrap();
            output.contains(text).then_some(output)
        })
    }

    fn finished(&mut self, child: usize, home: &Path, log: &str) -> String {
        let status = wait(|| self.children[child].try_wait().unwrap());
        let output = std::fs::read_to_string(home.join(log)).unwrap();
        assert!(status.success(), "{output}");
        output
    }

    fn daemon(&mut self, home: &Path) -> u16 {
        let child = self.spawn(home, &["run", "--backend", "null", "--no-discovery", "--no-notify", "--no-clipboard", "--port", "0"], "daemon.log");
        wait(|| {
            assert!(self.children[child].try_wait().unwrap().is_none(), "{}", std::fs::read_to_string(home.join("daemon.log")).unwrap());
            let output = one_mk_ctl::client::request(home, one_mk_ctl::client::Request::Status, Duration::from_secs(1)).ok().flatten()?;
            one_mk_ctl::status::Status::decode(&output).map(|status| status.port)
        })
    }

    fn pair(&mut self, host: &Path, joiner: &Path, invitation: bool, host_port: u16, join_port: u16) -> (String, String) {
        if invitation {
            let address = if_addrs::get_if_addrs().unwrap().into_iter().map(|interface| interface.ip())
                .find(|address| address.is_ipv4() && one_mk_net::invite::usable_address(*address)).expect("invitation needs a local non-loopback address");
            let host_child = self.spawn(host, &["invite", "--listen", &format!("{address}:0"), "--no-discovery", "--no-confirm", "--service-port", &host_port.to_string()], "pair.log");
            let output = Self::wait_text(host, "pair.log", "onemk://");
            let link = output.lines().find(|line| line.starts_with("onemk://")).unwrap();
            let join_child = self.spawn(joiner, &["join", "--yes", "--service-port", &join_port.to_string()], "pair.log");
            writeln!(self.children[join_child].stdin.as_mut().unwrap(), "{link}").unwrap();
            (self.finished(host_child, host, "pair.log"), self.finished(join_child, joiner, "pair.log"))
        } else {
            let host_child = self.spawn(host, &["pair", "--machine", "--no-discovery"], "pair.log");
            let output = Self::wait_text(host, "pair.log", "me\t");
            let port = output.lines().find(|line| line.starts_with("me\t")).unwrap().split('\t').nth(3).unwrap();
            let join_child = self.spawn(joiner, &["pair", "--machine", "--no-discovery", "--connect", &format!("127.0.0.1:{port}")], "pair.log");
            for (child, home) in [(host_child, host), (join_child, joiner)] {
                Self::wait_text(home, "pair.log", "code\t");
                writeln!(self.children[child].stdin.as_mut().unwrap(), "yes").unwrap();
            }
            (self.finished(host_child, host, "pair.log"), self.finished(join_child, joiner, "pair.log"))
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for child in &mut self.children { let _ = child.kill(); let _ = child.wait(); }
        if std::thread::panicking() {
            for home in std::fs::read_dir(self.root.path()).unwrap().flatten() {
                for log in ["pair.log", "daemon.log"] {
                    if let Ok(output) = std::fs::read_to_string(home.path().join(log)) { eprintln!("{} / {log}: {output}", home.path().display()); }
                }
            }
        }
    }
}

#[track_caller]
fn wait<T>(mut condition: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = condition() { return value; }
        assert!(Instant::now() < deadline, "pairing/layout did not settle within 20 seconds");
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn device(home: &Path, other: &Path) -> Device {
    let identity = Identity::load_or_create(other).unwrap();
    TrustStore::load(home).unwrap().get(&identity.fingerprint).unwrap().clone()
}

fn mirrored(host: &Path, peer: &Path, side: Side) {
    wait(|| {
        let local = device(host, peer);
        let remote = device(peer, host);
        (local.side() == side && remote.side() == side.opposite()).then_some(())
    });
    wait(|| {
        let output = Lab::cli(host, &["status"]);
        (!output.contains("edges overlap")).then_some(())
    });
}

fn three_computers(invitation: bool, reverse: bool) {
    let mut lab = Lab::new();
    let host = lab.home("host");
    let second = lab.home("second");
    let third = lab.home("third");
    let host_port = lab.daemon(&host);
    let second_port = lab.daemon(&second);
    lab.pair(&host, &second, invitation, host_port, second_port);
    let first = if invitation { Side::Left } else { Side::Right };
    mirrored(&host, &second, first);
    let original = device(&host, &second);
    let third_port = lab.daemon(&third);
    let (first_home, second_home, first_port, second_port) = if reverse { (&third, &host, third_port, host_port) } else { (&host, &third, host_port, third_port) };
    lab.pair(first_home, second_home, invitation, first_port, second_port);
    mirrored(&host, &third, one_mk_proto::pairing_layout::next_side([first]).unwrap());
    assert_eq!(device(&host, &second).side(), original.side());
    Lab::cli(&host, &["layout", &Identity::load_or_create(&third).unwrap().fingerprint.to_string(), "bottom", "--offset", "50,1080"]);
    wait(|| (device(&third, &host).offset == Some((-50, -1080))).then_some(()));
    lab.pair(first_home, second_home, invitation, first_port, second_port);
    mirrored(&host, &third, Side::Bottom);
    assert_eq!(device(&host, &third).offset, Some((50, 1080)));
}

#[test]
fn numeric_pairing_chooses_free_side_and_repair_preserves_placement() {
    for reverse in [false, true] { three_computers(false, reverse); }
}

#[test]
fn invitation_pairing_chooses_free_side_and_repair_preserves_placement() {
    for reverse in [false, true] { three_computers(true, reverse); }
}

#[test]
fn full_layout_still_pairs_and_requests_manual_placement() {
    for invitation in [false, true] {
        let mut lab = Lab::new();
        let host = lab.home("full");
        let joiner = lab.home("joiner");
        let mut trust = TrustStore::load(&host).unwrap();
        for (index, side) in Side::ALL.into_iter().enumerate() {
            let peer = lab.home(&format!("occupied-{index}"));
            let identity = Identity::load_or_create(&peer).unwrap();
            trust.upsert(Device { name: format!("occupied-{index}"), fingerprint: identity.fingerprint.to_string(), side: side.to_string(), ..Device::default() });
        }
        trust.save().unwrap();
        let host_port = lab.daemon(&host);
        let join_port = lab.daemon(&joiner);
        let (output, _) = lab.pair(&host, &joiner, invitation, host_port, join_port);
        assert!(output.contains("placement needed"), "{output}");
        wait(|| Lab::cli(&host, &["status"]).contains("placement needed").then_some(()));
        assert!(Lab::cli(&host, &["devices"]).contains("placement needed"));
        assert_eq!(device(&host, &joiner).placed_by, one_mk_net::PlacedBy::Pending);
        lab.pair(&host, &joiner, invitation, host_port, join_port);
        assert_eq!(device(&host, &joiner).placed_by, one_mk_net::PlacedBy::Pending);
        assert_eq!(TrustStore::load(&host).unwrap().devices().len(), 5);
        let fingerprint = Identity::load_or_create(&joiner).unwrap().fingerprint.to_string();
        let output = Lab::command(&host).args(["switch", &fingerprint]).output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("placement needed"));
        Lab::cli(&host, &["stop"]);
        lab.finished(0, &host, "daemon.log");
        assert!(Lab::cli(&host, &["status"]).contains("placement needed"));
        Lab::cli(&host, &["layout", &Identity::load_or_create(&joiner).unwrap().fingerprint.to_string(), "right", "--offset", "1920,1080"]);
        wait(|| (!Lab::cli(&host, &["status"]).contains("placement needed")).then_some(()));
        lab.daemon(&host);
        wait(|| (device(&joiner, &host).offset == Some((-1920, -1080))).then_some(()));
    }
}

#[test]
fn populated_computers_negotiate_a_free_mirrored_edge_without_moving_existing_peers() {
    for invitation in [false, true] {
        let mut lab = Lab::new();
        let host = lab.home("host");
        let joiner = lab.home("joiner");
        for (home, name) in [(&host, "old-host-peer"), (&joiner, "old-join-peer")] {
            let old = lab.home(name);
            let identity = Identity::load_or_create(&old).unwrap();
            let mut trust = TrustStore::load(home).unwrap();
            trust.upsert(Device { name: name.into(), fingerprint: identity.fingerprint.to_string(), side: "left".into(), ..Device::default() });
            trust.save().unwrap();
        }
        let host_port = lab.daemon(&host);
        let join_port = lab.daemon(&joiner);
        lab.pair(&host, &joiner, invitation, host_port, join_port);
        wait(|| {
            let local = device(&host, &joiner);
            let remote = device(&joiner, &host);
            (local.side() == remote.side().opposite() && local.side() != Side::Left && remote.side() != Side::Left).then_some(())
        });
        for home in [&host, &joiner] {
            let trust = TrustStore::load(home).unwrap();
            assert_eq!(trust.devices()[0].side(), Side::Left);
            assert!(!Lab::cli(home, &["status"]).contains("edges overlap"));
        }
    }
}

#[test]
fn incompatible_free_edges_stop_negotiating_and_request_placement() {
    let mut lab = Lab::new();
    let host = lab.home("host");
    let joiner = lab.home("joiner");
    for (home, sides) in [(&host, [Side::Left, Side::Right]), (&joiner, [Side::Top, Side::Bottom])] {
        let mut trust = TrustStore::load(home).unwrap();
        for (index, side) in sides.into_iter().enumerate() {
            trust.upsert(Device { fingerprint: one_mk_net::Fingerprint([index as u8; 32]).to_string(), name: format!("occupied-{index}"), side: side.to_string(), ..Device::default() });
        }
        trust.save().unwrap();
    }
    lab.daemon(&host);
    lab.daemon(&joiner);
    lab.pair(&host, &joiner, false, 0, 0);
    wait(|| {
        let local = device(&host, &joiner);
        let remote = device(&joiner, &host);
        (local.placed_by == one_mk_net::PlacedBy::Pending || remote.placed_by == one_mk_net::PlacedBy::Pending).then_some(())
    });
    for (home, other, occupied) in [(&host, &joiner, [Side::Left, Side::Right]), (&joiner, &host, [Side::Top, Side::Bottom])] {
        let device = device(home, other);
        assert!(device.placed_by == one_mk_net::PlacedBy::Pending || !occupied.contains(&device.side()));
        assert!(!Lab::cli(home, &["status"]).contains("edges overlap"));
    }
}
