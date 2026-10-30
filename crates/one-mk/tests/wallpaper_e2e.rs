#![cfg(target_os = "linux")]

use one_mk_net::{Device, Identity, Node, TrustStore, read_frame};
use one_mk_proto::{Control, PROTOCOL_VERSION, StreamKind, caps};
use one_mk_proto::wallpaper::{Kind, Message};
use serde_json::Value;
use std::path::{Path,PathBuf};
use std::process::{Child,Command,Stdio};
use std::sync::Arc;
use std::time::Duration;

struct Lab { root: PathBuf, children: Vec<Child> }
impl Lab {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("wall-{}-{:x}",std::process::id(),rand::random::<u16>()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root,children: Vec::new() }
    }
    fn home(&self, name: &str) -> PathBuf {
        let home = self.root.join(name);
        std::fs::create_dir_all(&home).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home,std::fs::Permissions::from_mode(0o700)).unwrap();
        home
    }
    fn command(home: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_one-mk"));
        command.env_clear().env("PATH","/usr/bin:/bin").env("HOME",home).env("XDG_RUNTIME_DIR",home).env("ONE_MK_HOME",home);
        command
    }
    fn cli(home: &Path, arguments: &[&str], code: i32) -> Value {
        let output = Self::command(home).args(arguments).output().unwrap();
        assert_eq!(output.status.code(),Some(code),"{arguments:?}: {}",String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn daemon(&mut self, home: &Path, port: u16, connect: Option<u16>) -> usize {
        let mut command = Self::command(home);
        command.args(["run","--backend","null","--no-discovery","--no-clipboard","--no-notify","--port",&port.to_string()]);
        if let Some(port) = connect { command.args(["--connect",&format!("127.0.0.1:{port}")]); }
        let log = std::fs::File::create(home.join("daemon.log")).unwrap();
        let child = command.stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log)).spawn().unwrap();
        self.children.push(child);
        self.children.len()-1
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        for child in &mut self.children { let _ = child.kill(); let _ = child.wait(); }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn port() -> u16 { std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }
fn trust(home: &Path, identity: &Identity) {
    let mut store = TrustStore::load(home).unwrap();
    store.upsert(Device { fingerprint: identity.fingerprint.to_string(),name: identity.name.clone(),side: "right".into(),..Default::default() });
    store.save().unwrap();
}
async fn ready(home: &Path) {
    for _ in 0..100 {
        let value = Lab::cli(home,&["wallpaper","status","--json"],0);
        if !value["daemon"].is_null() { return; }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("daemon did not start: {}",std::fs::read_to_string(home.join("daemon.log")).unwrap());
}

struct Attacker { _node: Node, send: quinn::SendStream, recv: quinn::RecvStream, buffer: Vec<u8> }
impl Attacker {
    async fn connect(home: &Path, port: u16) -> Self {
        Self::with_caps(home,port,caps::WALLPAPER).await
    }
    async fn with_caps(home: &Path, port: u16, capabilities: u32) -> Self {
        let identity = Identity::load_or_create(home).unwrap();
        let node = Node::bind(Arc::new(identity),"127.0.0.1:0".parse().unwrap()).unwrap();
        let peer = node.connect(format!("127.0.0.1:{port}").parse().unwrap()).await.unwrap();
        let (send,recv) = peer.open_stream(StreamKind::Control).await.unwrap();
        let mut attacker = Self { _node: node,send,recv,buffer: Vec::new() };
        attacker.send(Control::Hello { version: PROTOCOL_VERSION,name: "hostile wallpaper peer".into(),screens: vec![],nonce: 0,os: 0 }).await;
        assert!(matches!(attacker.next().await,Control::Hello { .. }));
        assert!(matches!(attacker.next().await,Control::Caps(_)));
        assert!(matches!(attacker.next().await,Control::Layout { .. }));
        attacker.send(Control::Caps(capabilities)).await;
        attacker
    }
    async fn send(&mut self, message: Control) {
        let mut bytes = Vec::new(); message.encode_frame(&mut bytes);
        self.send.write_all(&bytes).await.unwrap();
    }
    async fn next(&mut self) -> Control {
        tokio::time::timeout(Duration::from_secs(5),read_frame(&mut self.recv,&mut self.buffer)).await.unwrap().unwrap().unwrap()
    }
    async fn challenge(&mut self) -> [u8;16] {
        let mut epoch = None;
        for _ in 0..3 {
            let Control::Wallpaper(message) = self.next().await else { panic!("wallpaper handshake") };
            epoch = Some(message.epoch);
        }
        epoch.unwrap()
    }
}

#[tokio::test]
async fn old_peer_gets_no_wallpaper_and_forgetting_removes_consent() {
    let mut lab = Lab::new();
    let home = lab.home("r"); let remote = lab.home("l");
    let identity = Identity::load_or_create(&remote).unwrap();
    trust(&home,&identity);
    let port = port(); lab.daemon(&home,port,None); ready(&home).await;
    let mut peer = Attacker::with_caps(&remote,port,0).await;
    Lab::cli(&home,&["wallpaper","enable","--json"],0);
    Lab::cli(&home,&["wallpaper","consent","grant",&identity.fingerprint.to_string(),"--json"],0);
    assert!(tokio::time::timeout(Duration::from_millis(250),read_frame(&mut peer.recv,&mut peer.buffer)).await.is_err());
    let result = Lab::command(&home).args(["forget",&identity.fingerprint.to_string()]).output().unwrap();
    assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    let status = Lab::cli(&home,&["wallpaper","status","--json"],0);
    assert_eq!(status["consent"],serde_json::json!([]));
    assert_eq!(status["daemon"]["receivers"],0);
}

async fn observed(home: &Path, predicate: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..100 {
        let value = Lab::cli(home,&["wallpaper","status","--json"],0);
        if predicate(&value["daemon"]) { return value; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("wallpaper status did not converge");
}

#[tokio::test]
async fn three_real_null_daemons_consent_hostility_kill_restart_and_upgrade() {
    let mut lab = Lab::new();
    let (home_a,home_b,home_c,home_bad) = (lab.home("a"),lab.home("b"),lab.home("c"),lab.home("d"));
    let (identity_a,identity_b,identity_c,identity_bad) = (Identity::load_or_create(&home_a).unwrap(),Identity::load_or_create(&home_b).unwrap(),Identity::load_or_create(&home_c).unwrap(),Identity::load_or_create(&home_bad).unwrap());
    trust(&home_a,&identity_b); trust(&home_b,&identity_a);
    trust(&home_b,&identity_c); trust(&home_c,&identity_b); trust(&home_b,&identity_bad);
    std::fs::write(home_b.join("settings.toml"),"settings_version = 1\nnotify_files = false\n").unwrap();
    let port_b = port();
    let receiver = lab.daemon(&home_b,port_b,None); ready(&home_b).await;
    lab.daemon(&home_a,port(),Some(port_b)); ready(&home_a).await;
    lab.daemon(&home_c,port(),Some(port_b)); ready(&home_c).await;
    assert_eq!(Lab::cli(&home_b,&["wallpaper","status","--json"],0)["enabled"],false);
    Lab::cli(&home_a,&["wallpaper","consent","grant",&identity_c.fingerprint.to_string(),"--json"],4);
    let mut attacker = Attacker::connect(&home_bad,port_b).await;
    let mut message = Message { kind: Kind::Renderer,coordinator: identity_bad.fingerprint.0,epoch: [8;16],sequence: 1,descriptor: [0;48] };
    attacker.send(Control::Wallpaper(message)).await;
    observed(&home_b,|status| status["rejected"] == 1).await;
    Lab::cli(&home_b,&["wallpaper","enable","--json"],0);
    attacker.send(Control::Wallpaper(message)).await;
    observed(&home_b,|status| status["rejected"] == 2).await;
    Lab::cli(&home_b,&["wallpaper","consent","grant",&identity_bad.fingerprint.to_string(),"--json"],0);
    message.epoch = attacker.challenge().await;
    attacker.send(Control::Wallpaper(message)).await;
    observed(&home_b,|status| status["accepted"] == 1).await;
    let mut canvas = message; canvas.kind = Kind::Canvas; canvas.sequence = 2;
    canvas.descriptor[..4].copy_from_slice(&3300u32.to_le_bytes());
    canvas.descriptor[4..8].copy_from_slice(&1200u32.to_le_bytes());
    canvas.descriptor[8] = 2; canvas.descriptor[9] = 2;
    attacker.send(Control::Wallpaper(canvas)).await;
    observed(&home_b,|status| status["pending_canvases"] == 1).await;
    attacker.send(Control::Wallpaper(message)).await;
    let mut spoofed = message; spoofed.sequence = 2; spoofed.coordinator = identity_a.fingerprint.0;
    attacker.send(Control::Wallpaper(spoofed)).await;
    let mut wrong_epoch = message; wrong_epoch.sequence = 2; wrong_epoch.epoch = [4;16];
    attacker.send(Control::Wallpaper(wrong_epoch)).await;
    for sequence in 2..=40 { let mut flood = message; flood.sequence = sequence; attacker.send(Control::Wallpaper(flood)).await; }
    observed(&home_b,|status| status["rejected"].as_u64().unwrap_or(0) >= 10).await;
    Lab::cli(&home_b,&["wallpaper","consent","revoke",&identity_bad.fingerprint.to_string(),"--json"],0);
    let before = Lab::cli(&home_b,&["wallpaper","status","--json"],0);
    message.sequence = 100; attacker.send(Control::Wallpaper(message)).await;
    observed(&home_b,|status| status["rejected"].as_u64() > before["daemon"]["rejected"].as_u64()).await;
    assert_eq!(Lab::cli(&home_b,&["wallpaper","status","--json"],0)["daemon"]["pending_canvases"],0);
    Lab::cli(&home_b,&["wallpaper","consent","grant",&identity_bad.fingerprint.to_string(),"--json"],0);
    let old_epoch = attacker.challenge().await;
    lab.children[receiver].kill().unwrap(); lab.children[receiver].wait().unwrap();
    drop(attacker);
    lab.daemon(&home_b,port_b,None); ready(&home_b).await;
    let mut restarted = Attacker::connect(&home_bad,port_b).await;
    assert_ne!(restarted.challenge().await,old_epoch);
    message.epoch = old_epoch; restarted.send(Control::Wallpaper(message)).await;
    observed(&home_b,|status| status["rejected"] == 1 && status["accepted"] == 0).await;
    let mut oversized = vec![25]; oversized.extend_from_slice(&message.encode()); oversized.push(0);
    let mut framed = (oversized.len() as u16).to_le_bytes().to_vec(); framed.extend_from_slice(&oversized);
    restarted.send.write_all(&framed).await.unwrap();
    assert!(!matches!(tokio::time::timeout(Duration::from_secs(5),read_frame(&mut restarted.recv,&mut restarted.buffer)).await.unwrap(),Ok(Some(_))));
    Lab::cli(&home_b,&["wallpaper","doctor","--json"],3);
    Lab::cli(&home_b,&["wallpaper","disable","--json"],0);
    let status = Lab::cli(&home_b,&["wallpaper","status","--json"],0);
    assert_eq!(status["daemon"]["receivers"],0);
    assert_eq!(status["daemon"]["timers"],0);
    Lab::cli(&home_b,&["wallpaper","doctor","--json"],0);
    let saved = std::fs::read_to_string(home_b.join("settings.toml")).unwrap();
    assert!(saved.contains("notify_files = false"));
    assert!(saved.contains("[wallpaper]"));
}

fn ticks(child: &Child) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat",child.id())).unwrap();
    let fields: Vec<_> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
    fields[11].parse::<u64>().unwrap()+fields[12].parse::<u64>().unwrap()
}

#[tokio::test]
async fn isolated_off_and_enabled_scaffolding_have_zero_idle_cpu_ticks() {
    let mut lab = Lab::new(); let home = lab.home("idle");
    let daemon = lab.daemon(&home,port(),None); ready(&home).await;
    for enabled in [false,true,false] {
        Lab::cli(&home,&["wallpaper",if enabled { "enable" } else { "disable" },"--json"],0);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let before = ticks(&lab.children[daemon]);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let delta = ticks(&lab.children[daemon])-before;
        eprintln!("wallpaper enabled={enabled}: {delta} CPU ticks / 2 seconds");
        assert_eq!(delta,0);
        let status = Lab::cli(&home,&["wallpaper","status","--json"],0);
        assert_eq!(status["daemon"]["timers"],0);
        assert_eq!(status["daemon"]["children"],0);
    }
}
