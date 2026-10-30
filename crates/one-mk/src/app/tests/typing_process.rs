use super::*;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use tokio::io::AsyncBufReadExt;

#[tokio::test]
#[ignore = "subprocess fixture; launched only by the isolated lifecycle test"]
async fn fixture_daemon() {
    let Ok(directory) = std::env::var("ONE_MK_TYPING_FIXTURE") else { return };
    let dir = PathBuf::from(directory);
    let port = std::env::var("ONE_MK_TYPING_PORT").unwrap().parse().unwrap();
    let connect = std::env::var("ONE_MK_TYPING_CONNECT").ok().map(|port| SocketAddr::from(([127, 0, 0, 1], port.parse().unwrap()))).into_iter().collect();
    let (mut backend, mut handles) = one_mk_input::null(Desktop::new(vec![Rect::new(0, 0, 1920, 1080)]));
    backend.keyboard = Some(KeyboardLayout::fake(&std::env::var("ONE_MK_TYPING_LAYOUT").unwrap()));
    let events = handles.events.clone();
    let _capture = handles.capture;
    let output = dir.join("injected");
    tokio::spawn(async move {
        let mut output = std::fs::File::create(output).unwrap();
        while let Some(event) = handles.inject.recv().await {
            match event {
                Inject::Text { text } => writeln!(output, "text={}", text.as_str()).unwrap(),
                _ => writeln!(output, "{event:?}").unwrap(),
            }
            output.flush().unwrap();
        }
    });
    let (stop, mut stopped) = watch::channel(false);
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let words: Vec<_> = line.split_whitespace().collect();
            let event = match words.as_slice() {
                ["enter"] => Event::Activated { id: 1, side: Side::Right, pos: Pos::new(1919.0, 540.0) },
                ["key", code, down] => Event::Key { code: code.parse().unwrap(), down: *down == "down", device: None },
                ["stop"] => { stop.send_replace(true); break; }
                _ => panic!("invalid fixture command"),
            };
            events.send(event).unwrap();
        }
    });
    let (power, _) = one_mk_input::fake_power();
    serve(dir.clone(), Options { backend: one_mk_input::Kind::Null, port, connect, discovery: false, clipboard: None, downloads: dir.join("downloads"), start_locked: false, power: Some(power) }, backend, async move {
        let _ = stopped.wait_for(|stop| *stop).await;
    }).await.unwrap();
}

struct Process {
    child: Child,
    dir: PathBuf,
}

impl Process {
    fn spawn(dir: PathBuf, port: u16, connect: Option<u16>, layout: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let home = dir.join("home");
        let runtime = dir.join("runtime");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "app::tests::typing::processes::fixture_daemon", "--ignored", "--nocapture"])
            .env_clear().env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", home).env("XDG_RUNTIME_DIR", runtime).env("ONE_MK_HOME", &dir)
            .env("ONE_MK_TYPING_FIXTURE", &dir).env("ONE_MK_TYPING_PORT", port.to_string()).env("ONE_MK_TYPING_LAYOUT", layout)
            .stdin(Stdio::piped()).stdout(Stdio::from(std::fs::File::create(dir.join("stdout")).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(dir.join("stderr")).unwrap()));
        if let Some(connect) = connect { command.env("ONE_MK_TYPING_CONNECT", connect.to_string()); }
        Self { child: command.spawn().unwrap(), dir }
    }

    fn send(&mut self, command: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{command}").unwrap();
    }

    fn ticks(&self) -> u64 {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.child.id())).unwrap();
        let fields: Vec<_> = stat.rsplit_once(") ").unwrap().1.split_whitespace().collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    }

    fn injected(&self, event: &str) -> bool {
        std::fs::read_to_string(self.dir.join("injected")).unwrap_or_default().contains(event)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn connected_pair() -> (Process, Process) {
    let (da, db) = (temp_dir("typing-process-a"), temp_dir("typing-process-b"));
    let (identity_a, identity_b) = (Identity::load_or_create(&da).unwrap(), Identity::load_or_create(&db).unwrap());
    trust(&da, &identity_b, Side::Right);
    trust(&db, &identity_a, Side::Left);
    Settings::update(&da, |settings| { settings.typing.keys = crate::typing::Mode::Auto; Ok(()) }).unwrap();
    let port = free_port();
    let sender = Process::spawn(da, port, None, "de");
    let receiver = Process::spawn(db, free_port(), Some(port), "us");
    eventually_async("process layout negotiation", || async {
        control::request(&sender.dir, Request::Status).await.ok().flatten().and_then(|text| Status::decode(&text)).is_some_and(|status| status.typing == "characters (German to US)")
    }).await;
    (sender, receiver)
}

#[tokio::test]
#[ignore = "manual CPU measurement; coarse process accounting includes existing QUIC keepalives"]
async fn enabled_connected_idle_sample() {
    let (sender, receiver) = connected_pair().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let before = (sender.ticks(), receiver.ticks());
    tokio::time::sleep(Duration::from_secs(3)).await;
    let ticks = (sender.ticks() - before.0, receiver.ticks() - before.1);
    eprintln!("TEXT enabled, connected idle over 3 seconds: {ticks:?} CPU ticks");
}

#[tokio::test]
async fn two_process_character_path_kill9_releases() {
    let (mut sender, receiver) = connected_pair().await;
    sender.send("enter");
    eventually_async("process visit", || async { receiver.injected("MoveTo") }).await;
    sender.send("key 21 down");
    sender.send("key 21 up");
    eventually_async("process German text", || async { receiver.injected("text=z") }).await;
    sender.send("key 29 down");
    sender.send("key 21 down");
    eventually_async("process Ctrl Z", || async { receiver.injected("Key { code: 44, down: true }") }).await;
    sender.child.kill().unwrap();
    sender.child.wait().unwrap();
    eventually_async("release after SIGKILL", || async {
        receiver.injected("Key { code: 44, down: false }") && receiver.injected("Key { code: 29, down: false }")
    }).await;
}
