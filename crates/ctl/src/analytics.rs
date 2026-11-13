//! Opt-in product events only. No user-supplied properties or error strings.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};

mod transport;
mod spool;
mod crash;
/// Pin consent before any work starts. Enabling/resetting requires a new process;
/// old in-flight operations can never acquire a later consent generation.
pub fn initialize(home: &Path, origin: Origin) {
    let session = SESSION.get_or_init(|| Session::snapshot(home));
    if let Some((config, id)) = session.current(home) { crash::initialize(home, &config, &id, origin); }
}

struct Session { home: PathBuf, identity: Option<String>, config: Option<Config> }
impl Session {
    fn snapshot(home: &Path) -> Self { Self { home: home.into(), identity: identity(home), config: Config::from_env() } }
    fn current(&self, home: &Path) -> Option<(Config, String)> {
        self.for_generation(home, Config::from_env().as_ref(), process_revoked())
    }
    fn for_generation(&self, home: &Path, current: Option<&Config>, revoked: bool) -> Option<(Config, String)> {
        if revoked || home != self.home { return None; }
        let id = self.identity.as_ref()?;
        let config = self.config.as_ref()?;
        if identity(home).as_ref() != Some(id) || current?.destination() != config.destination() { return None; }
        Some((config.clone(), id.clone()))
    }
}
static SESSION: OnceLock<Session> = OnceLock::new();
fn process_revoked() -> bool { REVOKED.load(std::sync::atomic::Ordering::SeqCst) }
static REVOKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const CONSENT: &str = "analytics-consent-v3";
const APP_ID: &str = "one_mk";
fn environment(value: &str) -> Option<&str> {
    matches!(value, "development" | "staging" | "production" | "verification").then_some(value)
}
fn environment_from_env() -> Option<String> {
    let value = match std::env::var("ONE_MK_POSTHOG_ENVIRONMENT") {
        Ok(value) => value,
        Err(std::env::VarError::NotUnicode(_)) => return None,
        Err(std::env::VarError::NotPresent) => match std::env::var("POSTHOG_ENVIRONMENT") {
            Ok(value) => value,
            Err(std::env::VarError::NotUnicode(_)) => return None,
            Err(std::env::VarError::NotPresent) => if cfg!(debug_assertions) { "development" } else { "production" }.into(),
        },
    };
    environment(&value).map(str::to_owned)
}
fn consent_path(home: &Path, scope: &str) -> PathBuf { home.join(format!("{CONSENT}-{scope}")) }
const QUEUE: usize = 32;

/// The consent file itself is the random installation ID. Missing, malformed or
/// unreadable consent fails closed. Never generates an ID on a read.
pub fn identity(home: &Path) -> Option<String> { identity_for(home, &environment_from_env()?) }
fn identity_for(home: &Path, scope: &str) -> Option<String> {
    environment(scope)?;
    let mut data = String::new();
    std::fs::File::open(consent_path(home, scope)).ok()?.take(33).read_to_string(&mut data).ok()?;
    (data.len() == 32 && data.bytes().all(|b| b.is_ascii_hexdigit())).then_some(data)
}

/// Explicit local consent per environment; configuration alone never grants it.
/// Repeated enable preserves identity. Off/reset removes it; re-enable starts anew.
pub fn set_enabled(home: &Path, enabled: bool) -> std::io::Result<()> {
    let scope = environment_from_env().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid analytics environment"))?;
    set_enabled_for(home, enabled, &scope)
}
fn set_enabled_for(home: &Path, enabled: bool, scope: &str) -> std::io::Result<()> {
    if !enabled && SESSION.get().is_some_and(|s| s.home == home) { REVOKED.store(true, std::sync::atomic::Ordering::SeqCst); crash::disarm(); }
    crate::private::create_dir_all(home)?;
    let lock = crate::private::lock_file(&home.join("analytics-consent.lock"))?;
    lock.lock()?;
    let path = consent_path(home, scope);
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
    if identity_for(home, scope).is_some() { return Ok(()); }
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
struct Config { key: String, host: String, environment: String }
impl Config {
    #[cfg(test)]
    fn new(key: &str, host: &str) -> Option<Self> { Self::scoped(key, host, "development") }
    fn scoped(key: &str, host: &str, scope: &str) -> Option<Self> {
        environment(scope)?;
        if !key.starts_with("phc_") || key.len() <= 4 || key.len() > 256
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return None; }
        let host = match host {
            "https://us.i.posthog.com" => "us.i.posthog.com",
            "https://eu.i.posthog.com" => "eu.i.posthog.com",
            _ => return None,
        };
        Some(Self { key: key.into(), host: format!("https://{host}"), environment: scope.into() })
    }
    fn from_env() -> Option<Self> {
        Self::scoped(&std::env::var("ONE_MK_POSTHOG_KEY").ok()?, &std::env::var("ONE_MK_POSTHOG_HOST").ok()?, &environment_from_env()?)
    }
}
pub fn configured() -> bool { Config::from_env().is_some() }

struct Pending {
    home: PathBuf, id: String, config: Config, event: Event,
    origin: Origin, outcome: Outcome, error: ErrorCode,
}
impl Pending {
    fn permitted(&self) -> bool { !process_revoked() && identity(&self.home).as_deref() == Some(&self.id) }
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
    let Some((config, id)) = SESSION.get().and_then(|session| session.current(home)) else { return; };
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
        blake3::hash(format!("{APP_ID}\n{}\n{}\n{}", self.environment, self.host, self.key).as_bytes()).to_hex().to_string()
    }
}

/// Explicit synthetic verification only. Uses the real capture adapter, never
/// a production home. Returned UUIDs must be queried in PostHog before claiming
/// ingestion; HTTP acknowledgment alone cannot establish visibility/grouping.
pub fn verify_synthetic(home: &Path) -> anyhow::Result<(String, Vec<(String, String)>)> {
    let config = Config::from_env().ok_or_else(|| anyhow::anyhow!("PostHog project key and region required"))?;
    anyhow::ensure!(config.environment == "verification", "synthetic verification requires environment=verification");
    let mut receipts = Vec::new();
    let first_id = identity(home).ok_or_else(|| anyhow::anyhow!("explicit test-home consent required"))?;
    let marker = spool::Stored::new(&config, &first_id, Event::TransferFinished, Origin::Cli,
        Outcome::Success, ErrorCode::None, false).uuid;
    for event in [Event::AppOpened, Event::SettingsOpened] {
        let mut item = spool::Stored::new(&config, &first_id, event, Origin::Desktop,
            Outcome::Success, ErrorCode::None, false);
        item.verification = true;
        item.verification_run = Some(marker.clone());
        eprintln!("verification_run_id={marker} uuid={} event={event:?}", item.uuid);
        let result = transport::send(home, &config, &item);
        anyhow::ensure!(result == transport::Delivery::Accepted, "synthetic activity transport failed: {result:?}");
        receipts.push((item.uuid, format!("{result:?}")));
    }
    for generation in 0..2 {
        let id = identity(home).ok_or_else(|| anyhow::anyhow!("test consent required"))?;
        if generation == 1 { anyhow::ensure!(id != first_id, "reset must rotate identity"); }
        for (outcome, exception) in [(Outcome::Success, false), (Outcome::Failed, false), (Outcome::Failed, true)] {
            let mut item = spool::Stored::new(&config, &id, Event::TransferFinished, Origin::Cli,
                outcome, if matches!(outcome, Outcome::Failed) { ErrorCode::Io } else { ErrorCode::None }, exception);
            item.verification = true;
            item.verification_run = Some(marker.clone());
            eprintln!("verification_run_id={marker} uuid={} exception={exception}", item.uuid);
            let result = transport::send(home, &config, &item);
            anyhow::ensure!(result == transport::Delivery::Accepted, "synthetic transport failed: {result:?}");
            receipts.push((item.uuid.clone(), format!("{result:?}")));
            if exception {
                set_enabled(home, false)?;
                anyhow::ensure!(transport::send(home, &config, &item) == transport::Delivery::Revoked, "withdrawal must block replay");
                set_enabled(home, true)?;
                anyhow::ensure!(transport::send(home, &config, &item) == transport::Delivery::Revoked, "reset must block old identity");
            }
        }
    }
    Ok((marker, receipts))
}

/// Operator-only recovery of one real panic from the isolated verifier child.
/// No production home or raw panic payload may be passed to this helper.
pub fn verify_synthetic_panic(home: &Path) -> anyhow::Result<(String, String)> {
    let config = Config::from_env().ok_or_else(|| anyhow::anyhow!("PostHog configuration required"))?;
    anyhow::ensure!(config.environment == "verification", "panic verification requires environment=verification");
    crash::recover(home, &config);
    let mut items = spool::read(home);
    anyhow::ensure!(items.len() == 1, "exactly one isolated panic marker required");
    let mut item = items.remove(0);
    anyhow::ensure!(matches!(item.event, Event::PanicObserved), "panic marker required");
    anyhow::ensure!(item.site.and_then(crash::frame).is_some(), "real owned source site required");
    item.verification = true;
    item.verification_run = Some(item.uuid.clone());
    eprintln!("verification_run_id={} uuid={} real_panic_source_frame=true", item.uuid, item.uuid);
    let result = transport::send(home, &config, &item);
    anyhow::ensure!(result == transport::Delivery::Accepted, "panic verification transport failed: {result:?}");
    spool::purge(home)?;
    Ok((item.uuid, format!("{result:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consent_and_configuration_are_environment_scoped() {
        let home = tempfile::tempdir().unwrap();
        set_enabled_for(home.path(), true, "production").unwrap();
        let production = identity_for(home.path(), "production").unwrap();
        assert!(identity_for(home.path(), "verification").is_none());
        set_enabled_for(home.path(), true, "verification").unwrap();
        let verification = identity_for(home.path(), "verification").unwrap();
        assert_ne!(production, verification);
        set_enabled_for(home.path(), false, "verification").unwrap();
        assert!(identity_for(home.path(), "verification").is_none());
        assert_eq!(identity_for(home.path(), "production"), Some(production));
        for invalid in ["", "Production", "../production", "test", "other"] {
            assert!(Config::scoped("phc_test", "https://eu.i.posthog.com", invalid).is_none());
        }
        let a = Config::scoped("phc_test", "https://eu.i.posthog.com", "production").unwrap();
        let b = Config::scoped("phc_test", "https://eu.i.posthog.com", "verification").unwrap();
        assert_ne!(a.destination(), b.destination());
    }
    #[test]
    fn process_generation_never_adopts_later_consent_or_destination() {
        let home = tempfile::tempdir().unwrap();
        let config = Config::new("phc_test", "https://eu.i.posthog.com").unwrap();
        let disabled = Session { home: home.path().into(), identity: None, config: Some(config.clone()) };
        set_enabled(home.path(), true).unwrap();
        assert!(disabled.for_generation(home.path(), Some(&config), false).is_none());
        let active = Session { home: home.path().into(), identity: identity(home.path()), config: Some(config.clone()) };
        assert!(active.for_generation(home.path(), Some(&config), false).is_some());
        // A failed local withdrawal is fail-closed even if the file survived.
        assert!(active.for_generation(home.path(), Some(&config), true).is_none());
        let other = Config::new("phc_other", "https://eu.i.posthog.com").unwrap();
        assert!(active.for_generation(home.path(), Some(&other), false).is_none());
        set_enabled(home.path(), false).unwrap();
        assert!(active.for_generation(home.path(), Some(&config), false).is_none());
        set_enabled(home.path(), true).unwrap();
        assert!(active.for_generation(home.path(), Some(&config), false).is_none());
    }
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
        crate::private::write(&consent_path(home.path(), "development"), "SECRET_SENTINEL").unwrap(); assert!(identity(home.path()).is_none());
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
