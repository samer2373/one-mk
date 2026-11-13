//! Opt-in product events only. No user-supplied properties or error strings.
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};
use serde::Serialize;

const CONSENT: &str = "analytics-consent-v1";
const QUEUE: usize = 32;

/// The consent file itself is the random installation ID. Missing, malformed or
/// unreadable consent fails closed. Never generates an ID on a read.
pub fn identity(home: &Path) -> Option<String> {
    let mut data = String::new();
    std::fs::File::open(home.join(CONSENT)).ok()?.take(33).read_to_string(&mut data).ok()?;
    (data.len() == 32 && data.bytes().all(|b| b.is_ascii_hexdigit())).then_some(data)
}

/// Explicit local consent; independent of environment/other product preferences.
/// Repeated enable preserves identity. Off/reset removes it; re-enable starts anew.
pub fn set_enabled(home: &Path, enabled: bool) -> std::io::Result<()> {
    crate::private::create_dir_all(home)?;
    let lock = crate::private::lock_file(&home.join("analytics-consent.lock"))?;
    lock.lock()?;
    let path = home.join(CONSENT);
    if !enabled {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    if identity(home).is_some() { return Ok(()); }
    crate::private::create_dir_all(home)?;
    // Replace incomplete consent fail-closed; no live consent is overwritten.
    if path.exists() { std::fs::remove_file(&path)?; }
    match crate::private::create_new(&path) {
        Ok(mut file) => { file.write_all(format!("{:032x}", rand::random::<u128>()).as_bytes())?; file.sync_all() },
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    AppOpened, SettingsOpened, DaemonReady, DaemonFinished,
    PairingFinished, LayoutFinished, ProfileFinished, TransferFinished,
    ConnectionEstablished, ConnectionClosed, ConnectionAttemptFinished, UpdateCheckFinished,
    UpdateRequestFinished, UpdateInstallFinished, UpdateRollbackFinished, FeatureSettingFinished,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin { Desktop, Cli, Daemon }
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome { Success, Failed, Cancelled, Rejected }
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode { None, Io, Timeout, PermissionDenied, Connection, Unknown }

/// Classify types only, never message text (which may contain content/secrets).
pub fn error_code(error: &anyhow::Error) -> ErrorCode {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        match error.kind() {
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            std::io::ErrorKind::TimedOut => ErrorCode::Timeout,
            _ => ErrorCode::Io,
        }
    } else { ErrorCode::Unknown }
}

#[derive(Clone)]
struct Config { key: String, host: &'static str }
impl Config {
    fn new(key: &str, host: &str) -> Option<Self> {
        if !key.starts_with("phc_") || key.len() <= 4 || key.len() > 256
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return None; }
        let host = match host {
            "https://us.i.posthog.com" => "us.i.posthog.com",
            "https://eu.i.posthog.com" => "eu.i.posthog.com",
            _ => return None,
        };
        Some(Self { key: key.into(), host })
    }
    fn from_env() -> Option<Self> {
        Self::new(&std::env::var("ONE_MK_POSTHOG_KEY").ok()?, &std::env::var("ONE_MK_POSTHOG_HOST").ok()?)
    }
}
pub fn configured() -> bool { Config::from_env().is_some() }

struct Pending {
    home: PathBuf, id: String, config: Config, event: Event,
    origin: Origin, outcome: Outcome, error: ErrorCode,
}
impl Pending {
    fn permitted(&self) -> bool { identity(&self.home).as_deref() == Some(&self.id) }
    fn payload(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "api_key": self.config.key, "event": self.event,
            "properties": {"distinct_id": self.id, "$process_person_profile": false,
                "$geoip_disable": true, "app": "onemk", "app_version": env!("CARGO_PKG_VERSION"),
                "os_family": std::env::consts::OS, "origin": self.origin,
                "outcome": self.outcome, "error_code": self.error},
        })).expect("fixed event schema")
    }
}

/// Protect the daemon from reconnection storms: 60 events/minute/process,
/// at most one of the same event each second. Queue is separately bounded.
struct Budget { start: Instant, count: u32, last: std::collections::HashMap<Event, Instant> }
impl Budget {
    fn new(now: Instant) -> Self { Self { start: now, count: 0, last: Default::default() } }
    fn admit(&mut self, event: Event, now: Instant) -> bool {
        if now.duration_since(self.start) >= Duration::from_secs(60) { self.start = now; self.count = 0; }
        if self.count >= 60 || self.last.get(&event).is_some_and(|last| now.duration_since(*last) < Duration::from_secs(1)) { return false; }
        self.count += 1;
        self.last.insert(event, now);
        true
    }
}
static PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static SENDER: OnceLock<Option<mpsc::SyncSender<Pending>>> = OnceLock::new();

pub fn record(home: &Path, event: Event, origin: Origin, outcome: Outcome, error: ErrorCode) {
    let (Some(config), Some(id)) = (Config::from_env(), identity(home)) else { return; };
    let sender = SENDER.get_or_init(|| {
        let (send, receive) = mpsc::sync_channel::<Pending>(QUEUE);
        std::thread::Builder::new().name("anonymous-analytics".into()).spawn(move || {
            let mut budget = Budget::new(Instant::now());
            while let Ok(pending) = receive.recv() {
                if pending.permitted() && budget.admit(pending.event, Instant::now()) {
                    let _ = pending.config.send_if_permitted(&pending);
                }
                PENDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            }
        }).ok().map(|_| send)
    });
    if let Some(sender) = sender {
        PENDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if sender.try_send(Pending { home: home.into(), id, config, event, origin, outcome, error }).is_err() {
            PENDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Only for process exit; never wait in a UI/frame/input/daemon handler.
pub fn flush() {
    let until = Instant::now() + Duration::from_millis(300);
    while PENDING.load(std::sync::atomic::Ordering::Relaxed) != 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn result(home: &Path, event: Event, origin: Origin, result: &anyhow::Result<()>) {
    let (outcome, error) = match result {
        Ok(()) => (Outcome::Success, ErrorCode::None),
        Err(error) => (Outcome::Failed, error_code(error)),
    };
    record(home, event, origin, outcome, error);
}

impl Config {
    fn send_if_permitted(&self, pending: &Pending) -> anyhow::Result<()> {
        if !pending.permitted() { return Ok(()); }
        let body = pending.payload();
        let address = (self.host, 443).to_socket_addrs()?.next()
            .ok_or_else(|| anyhow::anyhow!("analytics host unavailable"))?;
        let socket = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        socket.set_write_timeout(Some(Duration::from_secs(1)))?;
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?.with_root_certificates(roots).with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(self.host)?;
        let mut stream = rustls::StreamOwned::new(rustls::ClientConnection::new(Arc::new(config), name)?, socket);
        // DNS/connect may have taken time: withdrawal invalidates queued IDs.
        if !pending.permitted() { return Ok(()); }
        write!(stream, "POST /capture/ HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", self.host, body.len())?;
        stream.write_all(&body)?;
        stream.flush()?;
        // Wait for a response without storing its body, following redirects,
        // logging failures or allowing analytics errors to affect the command.
        let mut response = [0; 128];
        let _ = stream.read(&mut response)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending(home: &Path) -> Pending {
        Pending { home: home.into(), id: identity(home).unwrap(),
            config: Config::new("phc_test", "https://eu.i.posthog.com").unwrap(),
            event: Event::TransferFinished, origin: Origin::Desktop,
            outcome: Outcome::Failed, error: ErrorCode::Unknown }
    }
    #[test]
    fn consent_is_explicit_stable_and_resettable() {
        let home = tempfile::tempdir().unwrap();
        assert!(identity(home.path()).is_none());
        set_enabled(home.path(), true).unwrap();
        let original = identity(home.path()).unwrap();
        set_enabled(home.path(), true).unwrap();
        assert_eq!(identity(home.path()).unwrap(), original);
        let event = pending(home.path());
        assert!(event.permitted());
        set_enabled(home.path(), false).unwrap();
        assert!(identity(home.path()).is_none());
        assert!(!event.permitted());
        set_enabled(home.path(), true).unwrap();
        assert_ne!(identity(home.path()).unwrap(), original);
        assert!(!event.permitted(), "old queued events never use fresh consent");
        // This exits before DNS/network, even with a syntactically valid key.
        event.config.send_if_permitted(&event).unwrap();
    }
    #[test]
    fn corrupt_or_unreadable_consent_fails_closed() {
        let home = tempfile::tempdir().unwrap();
        for value in ["", "yes", "0123456789abcdef0123456789abcdef-extra", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"] {
            crate::private::write(&home.path().join(CONSENT), value).unwrap();
            assert!(identity(home.path()).is_none());
        }
        set_enabled(home.path(), false).unwrap();
        std::fs::create_dir(home.path().join(CONSENT)).unwrap();
        assert!(identity(home.path()).is_none());
        assert!(set_enabled(home.path(), false).is_err());
    }
    #[test]
    fn configuration_requires_valid_public_key_and_cloud_region() {
        for key in ["", "phc_", "secret_personal_key", "phc_bad\r\n"] {
            assert!(Config::new(key, "https://us.i.posthog.com").is_none());
        }
        for host in ["", "http://eu.i.posthog.com", "https://evil.example", "https://eu.i.posthog.com/path"] {
            assert!(Config::new("phc_test", host).is_none());
        }
        assert!(Config::new("phc_test", "https://eu.i.posthog.com").is_some());
    }
    #[test]
    fn exact_payload_has_no_user_content_or_error_text() {
        let home = tempfile::tempdir().unwrap();
        set_enabled(home.path(), true).unwrap();
        let event = pending(home.path());
        let value: serde_json::Value = serde_json::from_slice(&event.payload()).unwrap();
        assert_eq!(value["event"], "transfer_finished");
        let props = value["properties"].as_object().unwrap();
        assert_eq!(props.len(), 9);
        assert_eq!(props["error_code"], "unknown");
        assert_eq!(props["$process_person_profile"], false);
        assert_eq!(props["$geoip_disable"], true);
        assert!(!String::from_utf8(event.payload()).unwrap().contains(home.path().to_str().unwrap()));
    }
    #[test]
    fn error_classification_uses_types_not_message_strings() {
        let private = anyhow::anyhow!("permission denied: /secret/path token=private");
        assert!(matches!(error_code(&private), ErrorCode::Unknown));
        let io = anyhow::Error::new(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/secret"));
        assert!(matches!(error_code(&io), ErrorCode::PermissionDenied));
    }
    #[test]
    fn bounded_queue_drops_excess_without_blocking() {
        let (send, _receive) = mpsc::sync_channel(QUEUE);
        for n in 0..QUEUE { send.try_send(n).unwrap(); }
        assert!(matches!(send.try_send(QUEUE), Err(mpsc::TrySendError::Full(_))));
    }
    #[test]
    fn event_storms_are_throttled_and_budget_recovers() {
        let start = Instant::now();
        let mut budget = Budget::new(start);
        assert!(budget.admit(Event::ConnectionEstablished, start));
        assert!(!budget.admit(Event::ConnectionEstablished, start));
        budget.count = 60;
        assert!(!budget.admit(Event::TransferFinished, start + Duration::from_secs(2)));
        assert!(budget.admit(Event::TransferFinished, start + Duration::from_secs(61)));
    }
    #[cfg(unix)]
    #[test]
    fn identity_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        set_enabled(home.path(), true).unwrap();
        assert_eq!(std::fs::metadata(home.path().join(CONSENT)).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
