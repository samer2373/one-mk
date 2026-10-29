//! The portal backend against a fake desktop: a fake
//! `org.freedesktop.portal.Desktop` (RemoteDesktop, InputCapture, Session)
//! on a private `dbus-daemon`, and a fake libei server that remembers which
//! keys and buttons are held down. Each failure path is driven on purpose:
//! the desktop closing a session, the portal going away, the session bus
//! dying, a hung call, a refused restore token, the screen locking and the
//! computer sleeping. (Skipped where `dbus-daemon` isn't installed.)

use super::*;
use ::zbus;
use reis::PendingRequestResult;
use reis::request::{EisRequest, EisRequestConverter};
use std::collections::BTreeSet;
use std::io::BufRead;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Opts = HashMap<String, OwnedValue>;
type Results = HashMap<&'static str, Value<'static>>;

// ------------------------------------------------------------ unit tests

#[test]
fn held_keys_and_buttons_are_let_go_last_first() {
    let mut h = Held::default();
    for m in [
        Inject::Key { code: 29, down: true },
        Inject::Key { code: 30, down: true },
        Inject::Key { code: 30, down: true }, // a repeat holds it once
        Inject::Button { button: 0x110, down: true },
        Inject::Key { code: 31, down: true },
        Inject::Key { code: 31, down: false },
        Inject::MoveTo(Pos::new(1.0, 2.0)),
    ] {
        h.update(&m);
    }
    assert_eq!(
        h.release(),
        vec![
            Inject::Button { button: 0x110, down: false },
            Inject::Key { code: 30, down: false },
            Inject::Key { code: 29, down: false },
        ]
    );
    assert_eq!(h.release(), vec![], "forgotten once let go");
}

#[test]
fn token_file_is_private_atomic_and_forgettable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir();
    let path = dir.join("sub").join(REMOTE_DESKTOP_TOKEN);
    let tokens = TokenFile(path.clone());
    assert_eq!(tokens.load(), None);
    // A leftover temporary file from before, readable by all.
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp = path.with_file_name(format!("{REMOTE_DESKTOP_TOKEN}.tmp"));
    std::fs::write(&tmp, "old").unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();

    tokens.store(Some("abc-123"));
    assert_eq!(tokens.load().as_deref(), Some("abc-123"));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!tmp.exists(), "renamed into place");
    // Edited by hand: whitespace doesn't count.
    std::fs::write(&path, "  abc-123\n").unwrap();
    assert_eq!(tokens.load().as_deref(), Some("abc-123"));
    std::fs::write(&path, "\n").unwrap();
    assert_eq!(tokens.load(), None);
    tokens.store(None);
    assert!(!path.exists());
    tokens.store(None); // nothing to forget is fine
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn retry_backs_off_and_starts_over_after_a_stable_session() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut r = Retry::new(tx, "control", "Can't do it", "Waiting");
    let lost = || Err(anyhow!("lost"));
    let secs: Vec<u64> = (0..7)
        .map(|_| match r.after(lost(), Duration::ZERO) {
            Next::Retry(d) => d.as_secs(),
            Next::Stop => panic!("transient errors are retried"),
        })
        .collect();
    assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30]);
    // Told once per run of failures, as a status line.
    assert_eq!(
        rx.try_recv().unwrap(),
        Event::Status { part: "control", problem: Some("Can't do it: lost. Retrying in the background.".into()) }
    );
    assert!(rx.try_recv().is_err());
    r.ready();
    assert_eq!(rx.try_recv().unwrap(), Event::Status { part: "control", problem: None });
    // A session that ran for a minute starts the backoff over, and its
    // loss is routine: nothing to tell unless getting it back fails.
    assert_eq!(r.after(lost(), STABLE), Next::Retry(Duration::from_secs(1)));
    assert!(rx.try_recv().is_err());
    assert_eq!(r.after(lost(), Duration::ZERO), Next::Retry(Duration::from_secs(2)));
    assert!(matches!(rx.try_recv(), Ok(Event::Status { problem: Some(_), .. })));
    // The daemon gone: stop.
    assert_eq!(r.after(Ok(()), Duration::ZERO), Next::Stop);
}

#[test]
fn refusals_and_rejected_tokens() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut r = Retry::new(tx, "control", "Can't do it", "Waiting");
    let declined = anyhow::Error::new(ashpd::Error::Response(ResponseError::Cancelled)).context("not granted");
    assert_eq!(r.after(Err(declined), Duration::ZERO), Next::Stop);
    let Ok(Event::Status { problem: Some(p), .. }) = rx.try_recv() else { panic!("a status line") };
    assert!(p.contains("you declined") && p.contains("Restart one-mk"), "{p}");

    let rejected = anyhow::Error::new(ashpd::Error::Response(ResponseError::Other)).context(TokenRejected);
    assert_eq!(r.after(Err(rejected), Duration::ZERO), Next::Retry(Duration::ZERO), "asks again at once");

    // Refused: the portal answered no. Not refused: it didn't answer.
    assert!(refused(&ashpd::Error::Response(ResponseError::Other).into()));
    let invalid = zbus::Error::FDO(Box::new(zbus::fdo::Error::InvalidArgs("Invalid restore token".into())));
    assert!(refused(&ashpd::Error::Zbus(invalid).into()));
    assert!(!refused(&ashpd::Error::Zbus(zbus::Error::InputOutput(std::io::Error::other("gone").into())).into()));
    assert!(!refused(&anyhow!("the desktop portal did not answer (selecting devices)")));
}

#[tokio::test]
async fn a_panic_is_an_error() {
    let r = guarded("the test", async { panic!("boom") }).await;
    assert_eq!(r.unwrap_err().to_string(), "the test crashed");
    assert!(guarded("the test", async { Ok(()) }).await.is_ok());
}

// ------------------------------------------------------- fake desktop

/// What the fake desktop does, and what it saw.
#[derive(Debug)]
struct World {
    /// Calls, in order: "rd.CreateSession", "ic.Release", ...
    log: Vec<String>,
    /// Restore tokens the desktop accepts (each once).
    tokens: Vec<String>,
    next_token: u32,
    /// An unknown token makes SelectDevices fail, as xdg-desktop-portal
    /// does for one it can't parse; otherwise the dialog just comes up.
    strict_tokens: bool,
    /// The user declines every dialog.
    deny: bool,
    /// How long the user takes in a dialog.
    dialog: Duration,
    /// InputCapture.Release never answers.
    hang_release: bool,
    /// InputCapture.Disable never answers.
    hang_disable: bool,
    ic_version: u32,
    /// Restore tokens given to SelectDevices / InputCapture.Start, by session.
    restoring: HashMap<String, Option<String>>,
    /// Sessions, "rd" or "ic", in creation order.
    sessions: Vec<(&'static str, String)>,
    /// Barrier ids of the last SetPointerBarriers.
    barriers: Vec<u32>,
    /// And their positions (x1, y1, x2, y2).
    positions: Vec<(i32, i32, i32, i32)>,
    /// ('k', key) and ('b', button) held down through libei.
    pressed: BTreeSet<(char, u32)>,
    /// Devices the injector started emulating on.
    emulating: usize,
}

impl Default for World {
    fn default() -> Self {
        World {
            log: Vec::new(),
            tokens: Vec::new(),
            next_token: 1,
            strict_tokens: false,
            deny: false,
            dialog: Duration::ZERO,
            hang_release: false,
            hang_disable: false,
            ic_version: 1,
            restoring: HashMap::new(),
            sessions: Vec::new(),
            barriers: Vec::new(),
            positions: Vec::new(),
            pressed: BTreeSet::new(),
            emulating: 0,
        }
    }
}

impl World {
    fn count(&self, call: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(call)).count()
    }

    fn session(&self, kind: &str) -> String {
        self.sessions.iter().rev().find(|(k, _)| *k == kind).expect("a session").1.clone()
    }

    fn new_token(&mut self) -> String {
        let t = format!("tok-{}", self.next_token);
        self.next_token += 1;
        self.tokens.push(t.clone());
        t
    }

    /// Whether `token` restores the permission (using it up).
    fn honor(&mut self, token: Option<&str>) -> bool {
        let Some(i) = token.and_then(|t| self.tokens.iter().position(|x| x == t)) else { return false };
        self.tokens.remove(i);
        true
    }
}

type Shared = Arc<Mutex<World>>;

fn lock(w: &Shared) -> std::sync::MutexGuard<'_, World> {
    w.lock().unwrap_or_else(|e| e.into_inner())
}

fn opt_str(o: &Opts, key: &str) -> Option<String> {
    o.get(key).and_then(|v| <&str>::try_from(&**v).ok()).map(str::to_owned)
}

fn opt_u32(o: &Opts, key: &str) -> Option<u32> {
    o.get(key).and_then(|v| u32::try_from(&**v).ok())
}

/// The object path of a request or session, as the portal spec makes it.
fn handle_path(hdr: &zbus::message::Header<'_>, kind: &str, token: Option<String>) -> String {
    let sender = hdr.sender().map(|s| s.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    format!("/org/freedesktop/portal/desktop/{kind}/{sender}/{}", token.unwrap_or_default())
}

fn path(p: &str) -> zbus::fdo::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(p.to_owned()).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))
}

/// Answers request `handle` after `delay`, as a portal does once the user
/// is done (0 = ok, 1 = cancelled).
fn respond(conn: &zbus::Connection, handle: String, delay: Duration, code: u32, results: Results) {
    let conn = conn.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay.max(Duration::from_millis(5))).await;
        let _ = conn
            .emit_signal(None::<&str>, handle.as_str(), "org.freedesktop.portal.Request", "Response", &(code, results))
            .await;
    });
}

async fn new_session(world: &Shared, conn: &zbus::Connection, kind: &'static str, at: &str) -> zbus::fdo::Result<()> {
    conn.object_server().at(at, FakeSession { world: world.clone(), kind }).await?;
    lock(world).sessions.push((kind, at.to_owned()));
    Ok(())
}

struct FakeSession {
    world: Shared,
    kind: &'static str,
}

#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl FakeSession {
    async fn close(&self) {
        lock(&self.world).log.push(format!("{}.Close", self.kind));
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

struct FakeRemoteDesktop(Shared);

#[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
impl FakeRemoteDesktop {
    #[zbus(name = "CreateSession")]
    async fn create_session(
        &self,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        let session = handle_path(&hdr, "session", opt_str(&options, "session_handle_token"));
        new_session(&self.0, conn, "rd", &session).await?;
        lock(&self.0).log.push("rd.CreateSession".into());
        respond(conn, handle.clone(), Duration::ZERO, 0, Results::from([("session_handle", Value::from(session))]));
        path(&handle)
    }

    #[zbus(name = "SelectDevices")]
    async fn select_devices(
        &self,
        session: OwnedObjectPath,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let token = opt_str(&options, "restore_token");
        let mut w = lock(&self.0);
        w.log.push(format!("rd.SelectDevices token={token:?} persist={:?}", opt_u32(&options, "persist_mode")));
        if let Some(t) = &token
            && w.strict_tokens
            && !w.tokens.contains(t)
        {
            return Err(zbus::fdo::Error::InvalidArgs("Invalid restore token".into()));
        }
        w.restoring.insert(session.to_string(), token);
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        respond(conn, handle.clone(), Duration::ZERO, 0, Results::new());
        path(&handle)
    }

    #[zbus(name = "Start")]
    async fn start(
        &self,
        session: OwnedObjectPath,
        _parent: String,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        let mut w = lock(&self.0);
        let token = w.restoring.remove(session.as_str()).flatten();
        let restored = w.honor(token.as_deref());
        w.log.push(format!("rd.Start dialog={}", !restored));
        let (code, delay) = match (restored, w.deny) {
            (true, _) => (0, Duration::ZERO),
            (false, true) => (1, w.dialog),
            (false, false) => (0, w.dialog),
        };
        let mut results = Results::from([("devices", Value::from(3u32))]);
        if code == 0 {
            results.insert("restore_token", Value::from(w.new_token()));
        }
        respond(conn, handle.clone(), delay, code, results);
        path(&handle)
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(&self, _session: OwnedObjectPath, _options: Opts) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        lock(&self.0).log.push("rd.ConnectToEIS".into());
        fake_eis(self.0.clone(), true)
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(property, name = "AvailableDeviceTypes")]
    fn available_device_types(&self) -> u32 {
        3
    }
}

struct FakeInputCapture(Shared);

#[zbus::interface(name = "org.freedesktop.portal.InputCapture")]
impl FakeInputCapture {
    /// Version 1: asks the user every time.
    #[zbus(name = "CreateSession")]
    async fn create_session(
        &self,
        _parent: String,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        let session = handle_path(&hdr, "session", opt_str(&options, "session_handle_token"));
        new_session(&self.0, conn, "ic", &session).await?;
        let mut w = lock(&self.0);
        w.log.push("ic.CreateSession".into());
        let code = u32::from(w.deny);
        // InputCapture gives the session as an object path (RemoteDesktop as a string).
        let handle_value = Value::from(path(&session)?.into_inner());
        let results = Results::from([("session_handle", handle_value), ("capabilities", Value::from(3u32))]);
        respond(conn, handle.clone(), w.dialog, code, results);
        path(&handle)
    }

    #[zbus(name = "CreateSession2")]
    async fn create_session2(
        &self,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let session = handle_path(&hdr, "session", opt_str(&options, "session_handle_token"));
        new_session(&self.0, conn, "ic", &session).await?;
        lock(&self.0).log.push("ic.CreateSession2".into());
        let value = OwnedValue::try_from(Value::from(path(&session)?)).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(HashMap::from([("session_handle".to_owned(), value)]))
    }

    /// Version 2: restores with a token.
    #[zbus(name = "Start")]
    async fn start(
        &self,
        _session: OwnedObjectPath,
        _parent: String,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        let mut w = lock(&self.0);
        let token = opt_str(&options, "restore_token");
        let restored = w.honor(token.as_deref());
        w.log.push(format!("ic.Start token={token:?} dialog={}", !restored));
        let delay = if restored { Duration::ZERO } else { w.dialog };
        let results = Results::from([("capabilities", Value::from(3u32)), ("restore_token", Value::from(w.new_token()))]);
        respond(conn, handle.clone(), delay, 0, results);
        path(&handle)
    }

    #[zbus(name = "GetZones")]
    async fn get_zones(
        &self,
        _session: OwnedObjectPath,
        options: Opts,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        lock(&self.0).log.push("ic.GetZones".into());
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        let zones = vec![(1920u32, 1080u32, 0i32, 0i32)];
        respond(conn, handle.clone(), Duration::ZERO, 0, Results::from([("zones", Value::from(zones)), ("zone_set", Value::from(1u32))]));
        path(&handle)
    }

    #[zbus(name = "SetPointerBarriers")]
    async fn set_pointer_barriers(
        &self,
        _session: OwnedObjectPath,
        options: Opts,
        barriers: Vec<Opts>,
        _zone_set: u32,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let mut w = lock(&self.0);
        w.barriers = barriers.iter().filter_map(|b| opt_u32(b, "barrier_id")).collect();
        w.positions = barriers
            .iter()
            .filter_map(|b| <(i32, i32, i32, i32)>::try_from(b.get("position")?.try_clone().ok()?).ok())
            .collect();
        let line = format!("ic.SetPointerBarriers {:?}", w.barriers);
        w.log.push(line);
        let handle = handle_path(&hdr, "request", opt_str(&options, "handle_token"));
        respond(conn, handle.clone(), Duration::ZERO, 0, Results::from([("failed_barriers", Value::from(Vec::<u32>::new()))]));
        path(&handle)
    }

    #[zbus(name = "Enable")]
    async fn enable(&self, _session: OwnedObjectPath, _options: Opts) {
        lock(&self.0).log.push("ic.Enable".into());
    }

    #[zbus(name = "Disable")]
    async fn disable(&self, _session: OwnedObjectPath, _options: Opts) {
        let hang = {
            let mut w = lock(&self.0);
            w.log.push("ic.Disable".into());
            w.hang_disable
        };
        if hang {
            std::future::pending::<()>().await;
        }
    }

    #[zbus(name = "Release")]
    async fn release(&self, _session: OwnedObjectPath, options: Opts) {
        let hang = {
            let mut w = lock(&self.0);
            w.log.push(format!("ic.Release id={:?}", opt_u32(&options, "activation_id")));
            w.hang_release
        };
        if hang {
            std::future::pending::<()>().await;
        }
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(&self, _session: OwnedObjectPath, _options: Opts) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        lock(&self.0).log.push("ic.ConnectToEIS".into());
        fake_eis(self.0.clone(), false)
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        lock(&self.0).ic_version
    }

    #[zbus(property, name = "SupportedCapabilities")]
    fn supported_capabilities(&self) -> u32 {
        3
    }
}

/// A libei server for one connection: offers a keyboard and a pointer and
/// keeps track of what is held down. `sender`: the client injects
/// (RemoteDesktop); otherwise it receives (InputCapture).
fn fake_eis(world: Shared, sender: bool) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
    let (ours, theirs) = UnixStream::pair().map_err(|e| zbus::fdo::Error::IOError(e.to_string()))?;
    std::thread::spawn(move || {
        let fd = ours.as_raw_fd();
        let Ok(context) = reis::eis::Context::new(ours) else { return };
        let mut handshaker = reis::handshake::EisHandshaker::new(&context, 1);
        let mut converter: Option<EisRequestConverter> = None;
        let (mut seats, mut devices) = (Vec::new(), Vec::new());
        loop {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd; the socket outlives the loop.
            unsafe { libc::poll(&mut pfd, 1, 1000) };
            match context.read() {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => return, // the client hung up
            }
            while let Some(r) = context.pending_request() {
                let PendingRequestResult::Request(req) = r else { return };
                let Some(conv) = converter.as_mut() else {
                    if let Ok(Some(resp)) = handshaker.handle_request(req) {
                        let conv = EisRequestConverter::new(&context, resp, 1);
                        let caps = DeviceCapability::Pointer
                            | DeviceCapability::PointerAbsolute
                            | DeviceCapability::Keyboard
                            | DeviceCapability::Scroll
                            | DeviceCapability::Button;
                        seats.push(conv.handle().add_seat(Some("seat0"), caps));
                        converter = Some(conv);
                    }
                    continue;
                };
                if conv.handle_request(req).is_err() {
                    return;
                }
                while let Some(req) = conv.next_request() {
                    let mut w = lock(&world);
                    match req {
                        EisRequest::Bind(b) => {
                            for (name, caps) in [
                                ("keyboard", DeviceCapability::Keyboard.into()),
                                ("pointer", DeviceCapability::Pointer | DeviceCapability::Button | DeviceCapability::Scroll),
                            ] {
                                let dev = b.seat.add_device(Some(name), reis::eis::device::DeviceType::Virtual, caps, |_| {});
                                dev.resumed();
                                if !sender {
                                    dev.start_emulating(1);
                                }
                                devices.push(dev);
                            }
                        }
                        EisRequest::DeviceStartEmulating(_) => w.emulating += 1,
                        EisRequest::KeyboardKey(k) => {
                            if k.state == reis::eis::keyboard::KeyState::Press {
                                w.pressed.insert(('k', k.key));
                            } else {
                                w.pressed.remove(&('k', k.key));
                            }
                        }
                        EisRequest::Button(b) => {
                            if b.state == reis::eis::button::ButtonState::Press {
                                w.pressed.insert(('b', b.button));
                            } else {
                                w.pressed.remove(&('b', b.button));
                            }
                        }
                        EisRequest::Disconnect => return,
                        _ => {}
                    }
                }
            }
            let _ = context.flush();
        }
    });
    Ok(std::os::fd::OwnedFd::from(theirs).into())
}

// ------------------------------------------------------------- harness

fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("one-mk-portal-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A private session bus at a fixed address, so it can be restarted.
struct Bus {
    child: std::process::Child,
    addr: String,
}

impl Bus {
    fn start(socket: &Path) -> Option<Bus> {
        let _ = std::fs::remove_file(socket);
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .arg(format!("--address=unix:path={}", socket.display()))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut addr = String::new();
        std::io::BufReader::new(child.stdout.take()?).read_line(&mut addr).ok()?;
        // Without the server's guid, which changes when it restarts (the
        // user's real bus address has none either).
        let addr = addr.trim().split(',').filter(|kv| !kv.starts_with("guid=")).collect::<Vec<_>>().join(",");
        Some(Bus { child, addr })
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn serve_fake(addr: &str, world: &Shared) -> zbus::Connection {
    zbus::connection::Builder::address(addr)
        .unwrap()
        .name("org.freedesktop.portal.Desktop")
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", FakeRemoteDesktop(world.clone()))
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", FakeInputCapture(world.clone()))
        .unwrap()
        .build()
        .await
        .unwrap()
}

struct Harness {
    dir: PathBuf,
    bus: Option<Bus>,
    fake: Option<zbus::Connection>,
    world: Shared,
    events: mpsc::UnboundedReceiver<Event>,
    capture: Option<mpsc::UnboundedSender<CaptureCmd>>,
    inject: Option<mpsc::UnboundedSender<Inject>>,
    power: watch::Sender<Power>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const WAIT: Duration = Duration::from_secs(10);

impl Harness {
    /// The backend against a fake desktop set up by `setup`, with `files`
    /// (restore tokens saved from before) in its state directory. `None`
    /// without `dbus-daemon`.
    async fn start(files: &[(&str, &str)], setup: impl FnOnce(&mut World)) -> Option<Harness> {
        let dir = temp_dir();
        let Some(bus) = Bus::start(&dir.join("bus")) else {
            eprintln!("skipped: no dbus-daemon");
            return None;
        };
        let mut world = World::default();
        setup(&mut world);
        let world = Arc::new(Mutex::new(world));
        let fake = serve_fake(&bus.addr, &world).await;
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).unwrap();
        }
        let (power, power_rx) = watch::channel(Power::default());
        let env = Env { bus: Some(bus.addr.clone()), unit: Duration::from_millis(10), power: power_rx };
        let (ev_tx, events) = mpsc::unbounded_channel();
        let (capture, inject) = start(env, ev_tx, dir.clone()).unwrap();
        Some(Harness {
            dir,
            bus: Some(bus),
            fake: Some(fake),
            world,
            events,
            capture: Some(capture),
            inject: Some(inject),
            power,
        })
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        lock(&self.world)
    }

    /// Waits until the fake desktop's state satisfies `f`.
    async fn until(&self, what: &str, f: impl Fn(&World) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !f(&self.world()) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}; log: {:#?}", self.world().log);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The next event that satisfies `f`, skipping others.
    async fn event(&mut self, what: &str, f: impl Fn(&Event) -> bool) -> Event {
        let found = tokio::time::timeout(WAIT, async {
            loop {
                let ev = self.events.recv().await.expect("the backend is running");
                if f(&ev) {
                    return ev;
                }
            }
        });
        match found.await {
            Ok(ev) => ev,
            Err(_) => panic!("timed out waiting for {what}; log: {:#?}", self.world().log),
        }
    }

    /// Both sides set up, with a barrier on the right and the injector's
    /// devices ready.
    async fn ready(&mut self) {
        self.capture().send(CaptureCmd::SetSides(vec![Armed::whole(Side::Right)])).unwrap();
        self.until("injection", |w| w.emulating >= 2).await;
        self.until("barriers", |w| !w.barriers.is_empty() && w.count("ic.Enable") > 0).await;
    }

    fn capture(&self) -> &mpsc::UnboundedSender<CaptureCmd> {
        self.capture.as_ref().unwrap()
    }

    fn inject(&self, m: Inject) {
        self.inject.as_ref().unwrap().send(m).unwrap();
    }

    /// Hold a key and a button down through the injector.
    async fn hold(&self) {
        self.inject(Inject::Key { code: 30, down: true });
        self.inject(Inject::Button { button: 0x110, down: true });
        self.until("keys held", |w| w.pressed.len() == 2).await;
    }

    /// The pointer hits the right barrier; returns the activation id.
    async fn activate(&mut self) -> u32 {
        let (session, barrier) = {
            let w = self.world();
            (w.session("ic"), w.barriers[0])
        };
        let mut opts = Results::new();
        opts.insert("activation_id", Value::from(41u32));
        opts.insert("barrier_id", Value::from(barrier));
        opts.insert("cursor_position", Value::from((1919.0f64, 500.0f64)));
        self.signal("Activated", &session, opts).await;
        match self.event("Activated", |e| matches!(e, Event::Activated { .. })).await {
            Event::Activated { id, side, .. } => {
                assert_eq!(side, Side::Right);
                id
            }
            _ => unreachable!(),
        }
    }

    async fn signal(&self, name: &str, session: &str, opts: Results) {
        let session = OwnedObjectPath::try_from(session.to_owned()).unwrap();
        let fake = self.fake.as_ref().unwrap();
        let iface = "org.freedesktop.portal.InputCapture";
        fake.emit_signal(None::<&str>, "/org/freedesktop/portal/desktop", iface, name, &(session, opts)).await.unwrap();
    }

    /// The desktop closes the newest session of `kind`.
    async fn close_session(&self, kind: &str) {
        let session = self.world().session(kind);
        let fake = self.fake.as_ref().unwrap();
        let details = HashMap::<&str, Value>::new();
        fake.emit_signal(None::<&str>, session.as_str(), "org.freedesktop.portal.Session", "Closed", &details)
            .await
            .unwrap();
    }

    fn log(&self) -> Vec<String> {
        self.world().log.clone()
    }

    fn count(&self, call: &str) -> usize {
        self.world().count(call)
    }

    fn token(&self) -> Option<String> {
        TokenFile(self.dir.join(REMOTE_DESKTOP_TOKEN)).load()
    }
}

// ----------------------------------------------------------- the tests

#[tokio::test]
async fn first_start_waits_for_the_dialog_then_the_token_is_reused() {
    let Some(mut h) = Harness::start(&[], |w| w.dialog = Duration::from_millis(600)).await else { return };
    // Nobody allowed one-mk yet: the dialog comes up, and the user is told.
    let ev = h.event("waiting", |e| matches!(e, Event::Status { part: "control", problem: Some(_) })).await;
    let Event::Status { problem: Some(text), .. } = ev else { unreachable!() };
    assert!(text.starts_with("Waiting for your approval") && text.contains("remote desktop dialog"), "{text}");
    h.event("approved", |e| *e == Event::Status { part: "control", problem: None }).await;
    h.ready().await;
    assert_eq!(h.token().as_deref(), Some("tok-1"));
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(h.dir.join(REMOTE_DESKTOP_TOKEN)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(h.log().contains(&"rd.SelectDevices token=None persist=Some(2)".to_owned()), "{:#?}", h.log());

    // The daemon stops: both sessions are closed.
    h.capture = None;
    h.inject = None;
    h.until("sessions closed", |w| w.count("rd.Close") == 1 && w.count("ic.Close") == 1).await;

    // The next start restores the permission: no dialog, a fresh token.
    let (power, power_rx) = watch::channel(Power::default());
    let env = Env { bus: Some(h.bus.as_ref().unwrap().addr.clone()), unit: Duration::from_millis(10), power: power_rx };
    let (ev_tx, events) = mpsc::unbounded_channel();
    let (capture, inject) = start(env, ev_tx, h.dir.clone()).unwrap();
    (h.events, h.capture, h.inject, h.power) = (events, Some(capture), Some(inject), power);
    h.capture().send(CaptureCmd::SetSides(vec![Armed::whole(Side::Right)])).unwrap();
    h.until("started again", |w| w.count("rd.ConnectToEIS") == 2 && w.count("ic.Enable") == 2).await;
    assert_eq!(h.count("rd.Start dialog=false"), 1, "{:#?}", h.log());
    assert_eq!(h.token().as_deref(), Some("tok-2"));
    while let Ok(ev) = h.events.try_recv() {
        assert!(!matches!(ev, Event::Status { part: "control", problem: Some(_) }), "{ev:?}");
    }
}

#[tokio::test]
async fn a_rejected_token_is_replaced_by_asking_again() {
    let setup = |w: &mut World| {
        w.strict_tokens = true;
        w.dialog = Duration::from_millis(300);
    };
    let Some(mut h) = Harness::start(&[(REMOTE_DESKTOP_TOKEN, "from-another-life\n")], setup).await else { return };
    h.ready().await;
    let log = h.world().log.clone();
    let selects: Vec<_> = log.iter().filter(|l| l.starts_with("rd.SelectDevices")).collect();
    assert_eq!(
        selects,
        ["rd.SelectDevices token=Some(\"from-another-life\") persist=Some(2)", "rd.SelectDevices token=None persist=Some(2)"]
    );
    assert_eq!(h.count("rd.Close"), 1, "the refused session was closed");
    assert_eq!(h.token().as_deref(), Some("tok-1"), "replaced by the new permission");
}

#[tokio::test]
async fn declining_stops_with_a_clear_line_and_forgets_the_token() {
    // The saved token isn't honored, the dialog comes up, the user says no.
    let Some(mut h) = Harness::start(&[(REMOTE_DESKTOP_TOKEN, "stale")], |w| w.deny = true).await else { return };
    let ev = h.event("declined", |e| matches!(e, Event::Status { problem: Some(p), .. } if p.contains("declined"))).await;
    let Event::Status { problem: Some(text), .. } = ev else { unreachable!() };
    assert!(text.contains("Restart one-mk to be asked again"), "{text}");
    h.until("both declined", |w| w.count("rd.Start") == 1 && w.count("ic.CreateSession") == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.count("rd.CreateSession"), 1, "no new dialog after a no");
    assert_eq!(h.count("ic.CreateSession"), 1, "no new dialog after a no");
    assert_eq!(h.token(), None);
}

#[tokio::test]
async fn a_closed_session_lets_go_and_comes_back() {
    let Some(mut h) = Harness::start(&[], |_| {}).await else { return };
    h.ready().await;
    h.hold().await;
    let id = h.activate().await;

    // The desktop closes both sessions (as GNOME does on some monitor
    // changes, or when permission is revoked).
    h.close_session("rd").await;
    h.close_session("ic").await;
    assert_eq!(h.event("deactivated", |e| matches!(e, Event::Deactivated { .. })).await, Event::Deactivated { id });
    h.until("keys let go", |w| w.pressed.is_empty()).await;
    h.until("closed", |w| w.count("rd.Close") == 1 && w.count("ic.Close") == 1).await;

    // Both come back by themselves; the remote desktop one without asking.
    h.until("reconnected", |w| w.count("rd.ConnectToEIS") == 2 && w.count("ic.ConnectToEIS") == 2).await;
    h.until("barriers again", |w| w.count("ic.Enable") == 2).await;
    assert_eq!(h.count("rd.Start dialog=false"), 1, "{:#?}", h.log());
    // And work again.
    h.until("injection", |w| w.emulating >= 4).await;
    h.hold().await;
    let next = h.activate().await;
    assert_ne!(next, id);
}

#[tokio::test]
async fn the_portal_going_away_lets_go_and_reconnects_when_it_is_back() {
    let Some(mut h) = Harness::start(&[], |_| {}).await else { return };
    h.ready().await;
    h.hold().await;
    let id = h.activate().await;

    // xdg-desktop-portal crashes: its sessions are gone without a Closed.
    drop(h.fake.take());
    assert_eq!(h.event("deactivated", |e| matches!(e, Event::Deactivated { .. })).await, Event::Deactivated { id });
    h.until("keys let go", |w| w.pressed.is_empty()).await;
    // Retrying meanwhile, with backoff: a few attempts, not a busy loop.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let attempts = h.world().count("rd.CreateSession");
    assert_eq!(attempts, 1, "nobody to talk to: no session was created");

    h.fake = Some(serve_fake(&h.bus.as_ref().unwrap().addr.clone(), &h.world).await);
    h.until("reconnected", |w| w.count("rd.ConnectToEIS") == 2 && w.count("ic.ConnectToEIS") == 2).await;
    h.until("barriers again", |w| w.count("ic.Enable") == 2).await;
}

#[tokio::test]
async fn losing_the_session_bus_lets_go_and_reconnects() {
    let Some(mut h) = Harness::start(&[], |_| {}).await else { return };
    h.ready().await;
    h.hold().await;
    let id = h.activate().await;

    // The bus itself dies (the user's session bus restarting).
    drop(h.fake.take());
    drop(h.bus.take());
    assert_eq!(h.event("deactivated", |e| matches!(e, Event::Deactivated { .. })).await, Event::Deactivated { id });
    h.until("keys let go", |w| w.pressed.is_empty()).await;
    let ev = h.event("status", |e| matches!(e, Event::Status { part: "capture", problem: Some(_) })).await;
    let Event::Status { problem: Some(text), .. } = ev else { unreachable!() };
    assert!(text.contains("Retrying in the background"), "{text}");

    let bus = Bus::start(&h.dir.join("bus")).unwrap();
    h.fake = Some(serve_fake(&bus.addr, &h.world).await);
    h.bus = Some(bus);
    h.until("reconnected", |w| w.count("rd.ConnectToEIS") == 2 && w.count("ic.ConnectToEIS") == 2).await;
    h.event("capture back", |e| *e == Event::Status { part: "capture", problem: None }).await;
}

#[tokio::test]
async fn a_hung_release_falls_back_to_disabling() {
    let Some(mut h) = Harness::start(&[], |w| w.hang_release = true).await else { return };
    h.ready().await;
    let id = h.activate().await;
    // The daemon hands the pointer back, but the portal never answers:
    // disabling capture ends it too, and the barriers come back.
    h.capture().send(CaptureCmd::Release { id, pos: None }).unwrap();
    h.until("disabled", |w| w.count("ic.Disable") == 1 && w.count("ic.Enable") == 2).await;
    assert_eq!(h.count("ic.Close"), 0, "the session stays");
}

#[tokio::test]
async fn a_hung_portal_ends_the_session() {
    let setup = |w: &mut World| {
        w.hang_release = true;
        w.hang_disable = true;
    };
    let Some(mut h) = Harness::start(&[], setup).await else { return };
    h.ready().await;
    let id = h.activate().await;
    h.capture().send(CaptureCmd::Release { id, pos: None }).unwrap();
    // Neither answers: the session is given up (closing it, and dropping
    // its connection, makes the desktop release), then re-created.
    h.until("closed", |w| w.count("ic.Close") == 1).await;
    h.until("reconnected", |w| w.count("ic.ConnectToEIS") == 2).await;
}

#[tokio::test]
async fn locking_and_sleeping_let_go_and_waking_reads_the_screens_again() {
    let Some(mut h) = Harness::start(&[], |_| {}).await else { return };
    h.ready().await;
    h.hold().await;
    let id = h.activate().await;

    h.power.send_modify(|p| p.locked = true);
    assert_eq!(h.event("deactivated", |e| matches!(e, Event::Deactivated { .. })).await, Event::Deactivated { id });
    h.until("pointer released", |w| w.count("ic.Release id=Some(41)") == 1).await;
    h.until("keys let go", |w| w.pressed.is_empty()).await;
    // While locked the pointer doesn't leave: an activation is handed back.
    let session = h.world().session("ic");
    let barrier = h.world().barriers[0];
    let mut opts = Results::new();
    opts.insert("activation_id", Value::from(42u32));
    opts.insert("barrier_id", Value::from(barrier));
    opts.insert("cursor_position", Value::from((1919.0f64, 500.0f64)));
    h.signal("Activated", &session, opts).await;
    h.until("handed back", |w| w.count("ic.Release id=Some(42)") == 1).await;
    h.power.send_modify(|p| p.locked = false);

    // Sleep: the same; on waking the screen layout is read again.
    h.hold().await;
    let zones = h.world().count("ic.GetZones");
    h.power.send_modify(|p| p.asleep = true);
    h.until("keys let go", |w| w.pressed.is_empty()).await;
    h.power.send_modify(|p| p.asleep = false);
    h.until("zones again", |w| w.count("ic.GetZones") == zones + 1).await;
    // The sessions stayed: no new permission dialog after waking.
    assert_eq!(h.count("ic.CreateSession"), 1);
    assert_eq!(h.count("rd.CreateSession"), 1);
    while let Ok(ev) = h.events.try_recv() {
        assert!(!matches!(ev, Event::Activated { .. }), "{ev:?}");
    }
}

#[tokio::test]
async fn input_capture_version_2_restores_its_permission_too() {
    let setup = |w: &mut World| {
        w.ic_version = 2;
        w.tokens.push("ic-saved".into());
    };
    let Some(mut h) = Harness::start(&[(INPUT_CAPTURE_TOKEN, "ic-saved")], setup).await else { return };
    h.ready().await;
    assert_eq!(h.count("ic.CreateSession2"), 1);
    assert_eq!(h.count("ic.Start token=Some(\"ic-saved\") dialog=false"), 1, "{:#?}", h.log());
    let saved = TokenFile(h.dir.join(INPUT_CAPTURE_TOKEN)).load().unwrap();
    assert!(saved.starts_with("tok-"), "{saved}");
}

/// Barriers are line segments: with only part of an edge leading to a
/// peer (an offset layout), only that part gets one, so the rest is a wall
/// that never activates capture. `Activate` (`one-mk switch`) pushes the
/// pointer into the middle of that part.
#[tokio::test]
async fn barriers_cover_only_the_armed_part_of_an_edge() {
    let Some(mut h) = Harness::start(&[], |_| {}).await else { return };
    h.ready().await;
    assert_eq!(h.world().positions, [(1920, 0, 1920, 1079)], "all of the right edge");
    let part = Armed { side: Side::Right, spans: Some(vec![(300.0, 1080.0)]) };
    let bottom = Armed { side: Side::Bottom, spans: Some(vec![(100.5, 200.0), (5000.0, 6000.0)]) };
    h.capture().send(CaptureCmd::SetSides(vec![part, bottom])).unwrap();
    h.until("partial barriers", |w| w.positions.len() == 2).await;
    assert_eq!(h.world().positions, [(1920, 300, 1920, 1079), (101, 1080, 199, 1080)]);
    h.capture().send(CaptureCmd::SetSides(vec![Armed { side: Side::Left, spans: Some(vec![]) }])).unwrap();
    h.until("no barriers", |w| w.positions.is_empty()).await;
}
