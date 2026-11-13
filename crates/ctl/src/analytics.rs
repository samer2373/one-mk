//! Opt-in product events only. No user-supplied properties or error strings.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};

mod transport;
mod spool;
mod crash;
pub use crash::initialize;

const CONSENT: &str = "analytics-consent-v2";
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
        let removed = match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
        // Always attempt cleanup, even if consent removal itself failed.
        let purged = spool::purge(home);
        crash::disarm();
        return removed.and(purged);
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

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    AppOpened, SettingsOpened, DaemonReady, DaemonFinished,
    PairingFinished, LayoutFinished, ProfileFinished, TransferFinished,
    ConnectionEstablished, ConnectionClosed, ConnectionAttemptFinished, UpdateCheckFinished,
    UpdateRequestFinished, UpdateInstallFinished, UpdateRollbackFinished, FeatureSettingFinished, PanicObserved,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin { Desktop, Cli, Daemon }
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome { Success, Failed, Cancelled, Rejected }
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode { None, Io, Timeout, PermissionDenied, Connection, Unknown, Panic }

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
struct Config { key: String, host: String }
impl Config {
    fn new(key: &str, host: &str) -> Option<Self> {
        if !key.starts_with("phc_") || key.len() <= 4 || key.len() > 256
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return None; }
        let host = match host {
            "https://us.i.posthog.com" => "us.i.posthog.com",
            "https://eu.i.posthog.com" => "eu.i.posthog.com",
            _ => return None,
        };
        Some(Self { key: key.into(), host: format!("https://{host}") })
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
}

/// Protect the daemon from reconnection storms: 60 events/minute/process,
/// at most one of the same event each second. Queue is separately bounded.
struct Budget { start: Instant, count: u32, last: std::collections::HashMap<(Event, bool), Instant> }
impl Budget {
    fn new(now: Instant) -> Self { Self { start: now, count: 0, last: Default::default() } }
    fn admit(&mut self, event: Event, failed: bool, now: Instant) -> bool {
        if now.duration_since(self.start) >= Duration::from_secs(60) { self.start = now; self.count = 0; }
        if self.count >= 60 || self.last.get(&(event, failed)).is_some_and(|last| now.duration_since(*last) < Duration::from_secs(1)) { return false; }
        self.count += 1;
        self.last.insert((event, failed), now);
        true
    }
}
static PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static SENDER: OnceLock<Option<mpsc::SyncSender<Pending>>> = OnceLock::new();

fn enqueue(pending: Pending) {
    let sender = SENDER.get_or_init(|| {
        let (send, receive) = mpsc::sync_channel::<Pending>(QUEUE);
        std::thread::Builder::new().name("anonymous-analytics".into()).spawn(move || {
            let mut budget = Budget::new(Instant::now());
            let mut homes = std::collections::HashMap::<PathBuf, Config>::new();
            loop {
                match receive.recv_timeout(Duration::from_secs(1)) {
                    Ok(pending) => {
                        if pending.permitted() {
                            if homes.len() < 8 || homes.contains_key(&pending.home) {
                                homes.insert(pending.home.clone(), pending.config.clone());
                            }
                            if budget.admit(pending.event, matches!(pending.outcome, Outcome::Failed), Instant::now()) {
                                let _ = spool::append(&pending.home, &pending.config, &pending.id,
                                    pending.event, pending.origin, pending.outcome, pending.error);
                            }
                        }
                        // Flush guarantees a bounded chance to persist, not network acceptance.
                        PENDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                }
                for (home, config) in &homes {
                    let _ = spool::pump(home, config);
                }
            }
        }).ok().map(|_| send)
    });
    if let Some(sender) = sender {
        PENDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if sender.try_send(pending).is_err() {
            PENDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

pub fn record(home: &Path, event: Event, origin: Origin, outcome: Outcome, error: ErrorCode) {
    let (Some(config), Some(id)) = (Config::from_env(), identity(home)) else { return; };
    crash::arm(home, &config, &id, origin);
    enqueue(Pending { home: home.into(), id, config, event, origin, outcome, error });
}

/// Only for process exit; never wait in a UI/frame/input/daemon handler.
pub fn flush() {
    let until = Instant::now() + Duration::from_millis(300);
    while PENDING.load(std::sync::atomic::Ordering::Relaxed) != 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    crash::disarm();
}

pub fn result(home: &Path, event: Event, origin: Origin, result: &anyhow::Result<()>) {
    let (outcome, error) = match result {
        Ok(()) => (Outcome::Success, ErrorCode::None),
        Err(error) => (Outcome::Failed, error_code(error)),
    };
    record(home, event, origin, outcome, error);
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}
impl Config {
    fn destination(&self) -> String {
        blake3::hash(format!("{}\n{}", self.host, self.key).as_bytes()).to_hex().to_string()
    }
}

/// Explicit synthetic verification only. Uses the real capture adapter, never
/// a production home. Returned UUIDs must be queried in PostHog before claiming
/// ingestion; HTTP acknowledgment alone cannot establish visibility/grouping.
pub fn verify_synthetic(home: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let config = Config::from_env().ok_or_else(|| anyhow::anyhow!("PostHog project key and region required"))?;
    let id = identity(home).ok_or_else(|| anyhow::anyhow!("explicit test-home consent required"))?;
    let mut receipts = Vec::new();
    for exception in [false, true] {
        let mut item = spool::Stored::new(&config, &id, Event::TransferFinished, Origin::Cli,
            if exception { Outcome::Failed } else { Outcome::Success },
            if exception { ErrorCode::Io } else { ErrorCode::None }, exception);
        item.verification = true;
        let result = transport::send(home, &config, &item);
        receipts.push((item.uuid, format!("{result:?}")));
    }
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_explicit_and_cloud_only() {
        for host in ["", "http://eu.i.posthog.com", "https://other.example", "https://eu.i.posthog.com/path"] { assert!(Config::new("phc_test", host).is_none()); }
        for key in ["", "phc_", "phc_\r\n", "private_api_token"] { assert!(Config::new(key, "https://eu.i.posthog.com").is_none()); }
        assert!(Config::new("phc_test", "https://us.i.posthog.com").is_some());
    }
    #[test]
    fn identity_is_explicit_stable_reset_and_corrupt_fail_closed() {
        let home = tempfile::tempdir().unwrap(); assert!(identity(home.path()).is_none());
        // Legacy consent does not authorize detailed error diagnostics.
        crate::private::write(&home.path().join("analytics-consent-v1"), "a".repeat(32)).unwrap();
        assert!(identity(home.path()).is_none());
        set_enabled(home.path(), true).unwrap(); let id = identity(home.path()).unwrap();
        set_enabled(home.path(), true).unwrap(); assert_eq!(identity(home.path()).unwrap(), id);
        set_enabled(home.path(), false).unwrap(); set_enabled(home.path(), true).unwrap(); assert_ne!(identity(home.path()).unwrap(), id);
        crate::private::write(&home.path().join(CONSENT), "SECRET_SENTINEL").unwrap(); assert!(identity(home.path()).is_none());
    }
    #[test]
    fn success_does_not_hide_the_first_failure() {
        let now = Instant::now(); let mut budget = Budget::new(now);
        assert!(budget.admit(Event::TransferFinished, false, now));
        assert!(budget.admit(Event::TransferFinished, true, now));
        assert!(!budget.admit(Event::TransferFinished, true, now));
    }
    #[test]
    fn rate_budget_is_bounded() {
        let now = Instant::now(); let mut budget = Budget::new(now);
        assert!(budget.admit(Event::AppOpened, false, now)); assert!(!budget.admit(Event::AppOpened, false, now));
        for second in 1..60 { assert!(budget.admit(Event::AppOpened, false, now + Duration::from_secs(second))); }
        assert!(!budget.admit(Event::SettingsOpened, false, now + Duration::from_secs(59)));
        assert!(budget.admit(Event::SettingsOpened, false, now + Duration::from_secs(60)));
    }
}
