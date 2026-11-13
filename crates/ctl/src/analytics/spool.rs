//! Bounded private retry storage, never raw diagnostics. Keys are not persisted.
use super::*;
use transport::Delivery;

const FILE: &str = "analytics-spool-v2.json";
const MAX_EVENTS: usize = 256;
const MAX_BYTES: u64 = 512 * 1024;
const TTL: u64 = 24 * 60 * 60;
const ATTEMPTS: u8 = 5;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Stored {
    pub(super) uuid: String,
    pub(super) id: String,
    destination: String,
    created: u64,
    version: String,
    pub(super) event: Event,
    origin: Origin,
    outcome: Outcome,
    error: ErrorCode,
    exception: bool,
    #[serde(default)]
    pub(super) site: Option<[u32; 3]>,
    #[serde(default)]
    site_manifest: Option<String>,
    #[serde(default)]
    panic_aborts: bool,
    #[serde(default)]
    pub(super) verification: bool,
    #[serde(default)]
    pub(super) verification_run: Option<String>,
    attempts: u8,
    next: u64,
}
impl Stored {
    pub(super) fn new(config: &Config, id: &str, event: Event, origin: Origin, outcome: Outcome, error: ErrorCode, exception: bool) -> Self {
        let bytes: [u8; 16] = rand::random();
        let mut bytes = bytes;
        bytes[6] = (bytes[6] & 15) | 0x40;
        bytes[8] = (bytes[8] & 63) | 0x80;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Self { uuid: format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]),
            id: id.into(), destination: config.destination(), created: now(), version: env!("CARGO_PKG_VERSION").into(),
            event, origin, outcome, error, exception, site: None, verification: false, verification_run: None, panic_aborts: cfg!(panic = "abort"), site_manifest: matches!(event, Event::PanicObserved).then(crash::manifest), attempts: 0, next: 0 }
    }
    pub(super) fn occurred_at(&mut self, stamp: u64) { self.created = stamp; }
    pub(super) fn permitted(&self, home: &Path, config: &Config) -> bool {
        self.valid() && identity_for(home, config).as_deref() == Some(&self.id) && self.destination == config.destination()
            && self.created <= now().saturating_add(60) && now().saturating_sub(self.created) <= TTL
    }
    fn valid(&self) -> bool {
        (!matches!(self.event, Event::PanicObserved) || (self.version == env!("CARGO_PKG_VERSION") && self.site_manifest.as_deref() == Some(crash::manifest().as_str())))
            && self.verification_run.as_ref().is_none_or(|v| v.len() == 36 && v.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() }))
            && self.id.len() == 32 && self.id.bytes().all(|b| b.is_ascii_hexdigit())
            && self.destination.len() == 64 && self.destination.bytes().all(|b| b.is_ascii_hexdigit())
            && self.uuid.len() == 36 && self.uuid.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
            && !self.version.is_empty() && self.version.len() <= 32 && self.version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    }
    pub(super) fn payload(&self, config: &Config) -> Vec<u8> {
        let stamp = time::OffsetDateTime::from_unix_timestamp(self.created as i64)
            .ok().and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok());
        let mut properties = serde_json::json!({"$process_person_profile": false, "$geoip_disable": true,
            "app": "onemk", "app_id": APP_ID, "environment": config.environment, "schema_version": 2, "app_version": self.version, "os_family": std::env::consts::OS,
            "architecture": std::env::consts::ARCH, "origin": self.origin, "operation": self.event,
            "outcome": self.outcome, "error_code": self.error });
        if self.verification { properties["synthetic_verification"] = true.into(); }
        if let Some(marker) = &self.verification_run { properties["verification_run_id"] = marker.clone().into(); }
        let event = if self.exception {
            let panic = matches!(self.event, Event::PanicObserved);
            let kind = if panic { "RustPanicObserved" } else { "OneMKOperationError" };
            properties["$exception_list"] = serde_json::json!([{
                "type": kind,
                "value": if panic { "Rust panic observed; payload intentionally omitted" } else { "Operation failed; inspect operation and error_code" },
                "mechanism": {"type": if panic { "rust_panic_hook" } else { "handled_result" }, "handled": !panic, "synthetic": true},
            }]);
            if let Some(frame) = self.site.and_then(crash::frame) {
                properties["$exception_list"][0]["stacktrace"] = serde_json::json!({"type": "raw", "frames": [frame]});
            }
            // Group only by typed diagnostic fields, never payload/message/paths.
            properties["$exception_fingerprint"] = format!("{APP_ID}:{}:{:?}:{:?}:{}", config.environment, self.event, self.site,
                serde_json::to_string(&self.error).unwrap_or_default()).into();
            properties["$exception_level"] = (if panic && self.panic_aborts { "fatal" } else { "error" }).into();
            "$exception".to_owned()
        } else { serde_json::to_value(self.event).unwrap().as_str().unwrap().to_owned() };
        serde_json::to_vec(&serde_json::json!({"api_key": config.key, "uuid": self.uuid, "timestamp": stamp,
            "distinct_id": format!("{APP_ID}:{}:{}", config.environment, self.id), "event": event, "properties": properties})).expect("typed analytics envelope")
    }
}

fn lock(home: &Path) -> std::io::Result<std::fs::File> {
    crate::private::create_dir_all(home)?;
    let file = crate::private::lock_file(&home.join("analytics-consent.lock"))?;
    // Telemetry must not stall behind another process or a failing filesystem.
    file.try_lock().map_err(std::io::Error::other)?;
    Ok(file)
}
pub(super) fn read(home: &Path) -> Vec<Stored> {
    let Ok(file) = std::fs::File::open(home.join(FILE)) else { return vec![]; };
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_BYTES { return vec![]; }
    serde_json::from_slice::<Vec<Stored>>(&bytes).unwrap_or_default().into_iter().take(MAX_EVENTS).collect()
}
fn write(home: &Path, items: &[Stored]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(items).map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_BYTES { return Err(std::io::Error::other("analytics spool capacity")); }
    let temp = home.join("analytics-spool-v2.tmp");
    let mut file = crate::private::write_open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(temp, home.join(FILE))
}
pub(super) fn purge(home: &Path) -> std::io::Result<()> {
    let mut error = None;
    for name in [FILE, "analytics-spool-v2.tmp"] {
        if let Err(e) = std::fs::remove_file(home.join(name)) && e.kind() != std::io::ErrorKind::NotFound { error = Some(e); }
    }
    if let Err(e) = crash::purge(home) { error = Some(e); }
    match error { Some(error) => Err(error), None => Ok(()) }
}
pub(super) fn append(home: &Path, config: &Config, id: &str, event: Event, origin: Origin, outcome: Outcome, error: ErrorCode) -> std::io::Result<()> {
    let mut additions = vec![Stored::new(config, id, event, origin, outcome, error, false)];
    if matches!(outcome, Outcome::Failed) { additions.push(Stored::new(config, id, event, origin, outcome, error, true)); }
    append_items(home, config, additions)
}
pub(super) fn append_items(home: &Path, config: &Config, additions: Vec<Stored>) -> std::io::Result<()> {
    let _lock = lock(home)?;
    let mut items = read(home);
    items.retain(|v| v.permitted(home, config) && v.attempts < ATTEMPTS);
    for item in additions {
        if item.permitted(home, config) { items.push(item); }
    }
    if items.len() > MAX_EVENTS { items.drain(..items.len() - MAX_EVENTS); }
    write(home, &items)
}
pub(super) fn pump(home: &Path, config: &Config) -> std::io::Result<()> {
    let item = {
        let _lock = lock(home)?;
        let mut items = read(home);
        items.retain(|v| v.permitted(home, config) && v.attempts < ATTEMPTS);
        let selected = items.iter_mut().find(|v| v.next <= now()).map(|v| {
            v.attempts += 1;
            // Lease while HTTP runs, preventing simultaneous process sends.
            v.next = now() + 10;
            v.clone()
        });
        write(home, &items)?;
        selected
    };
    let Some(item) = item else { return Ok(()); };
    let delivery = transport::send(home, config, &item);
    let _lock = lock(home)?;
    let mut items = read(home);
    if let Some(stored) = items.iter_mut().find(|v| v.uuid == item.uuid) {
        match delivery {
            Delivery::Retry(delay) if stored.attempts < ATTEMPTS => {
                let backoff = (1u64 << stored.attempts.min(8)) + u64::from(rand::random::<u8>() % 5);
                stored.next = now() + delay.max(backoff).min(3600);
            }
            _ => { items.retain(|v| v.uuid != item.uuid); },
        }
    }
    items.retain(|v| v.permitted(home, config));
    write(home, &items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::consent_test::{identity, identity_for, set_enabled, set_enabled_for};
    fn config() -> Config { Config::new("phc_test_only", "https://eu.i.posthog.com").unwrap() }
    #[test]
    fn all_payloads_and_fingerprints_are_app_and_environment_scoped() {
        let home = tempfile::tempdir().unwrap();
        set_enabled_for(home.path(), true, "production").unwrap();
        let id = identity_for(home.path(), "production").unwrap();
        let production = Config::scoped("phc_test_only", "https://eu.i.posthog.com", "production").unwrap();
        let verification = Config::scoped("phc_test_only", "https://eu.i.posthog.com", "verification").unwrap();
        for exception in [false, true] {
            let item = Stored::new(&production, &id, Event::TransferFinished, Origin::Cli, Outcome::Failed, ErrorCode::Io, exception);
            assert!(item.permitted(home.path(), &production));
            assert!(!item.permitted(home.path(), &verification));
            let value: serde_json::Value = serde_json::from_slice(&item.payload(&production)).unwrap();
            assert_eq!(value["properties"]["app_id"], "one_mk");
            assert_eq!(value["properties"]["app"], "onemk");
            assert_eq!(value["properties"]["environment"], "production");
            assert_eq!(value["distinct_id"], format!("one_mk:production:{id}"));
            if exception {
                let other = Stored::new(&verification, &id, Event::TransferFinished, Origin::Cli, Outcome::Failed, ErrorCode::Io, true);
                let other: serde_json::Value = serde_json::from_slice(&other.payload(&verification)).unwrap();
                assert_ne!(value["properties"]["$exception_fingerprint"], other["properties"]["$exception_fingerprint"]);
            }
        }
    }
    #[test]
    fn retry_identity_timestamp_and_error_contract() {
        let home = tempfile::tempdir().unwrap(); set_enabled(home.path(), true).unwrap();
        let config = config(); let id = identity(home.path()).unwrap();
        append(home.path(), &config, &id, Event::TransferFinished, Origin::Cli, Outcome::Failed, ErrorCode::Io).unwrap();
        let items = read(home.path()); assert_eq!(items.len(), 2);
        let mut retry = items[1].clone(); retry.attempts += 1; retry.next = now() + 10;
        assert_eq!(items[1].payload(&config), retry.payload(&config));
        let payload: serde_json::Value = serde_json::from_slice(&retry.payload(&config)).unwrap();
        assert_eq!(payload["event"], "$exception");
        assert_eq!(payload["properties"]["$exception_list"][0]["mechanism"]["handled"], true);
        assert!(!retry.payload(&config).windows(6).any(|v| v == b"/Users"));
        assert_eq!(payload["uuid"], retry.uuid);
    }
    #[test]
    fn revoke_reenable_destination_expiry_and_corruption_fail_closed() {
        let home = tempfile::tempdir().unwrap(); set_enabled(home.path(), true).unwrap();
        let config = config(); let id = identity(home.path()).unwrap();
        let mut item = Stored::new(&config, &id, Event::AppOpened, Origin::Desktop, Outcome::Success, ErrorCode::None, false);
        assert!(item.permitted(home.path(), &config));
        assert!(!item.permitted(home.path(), &Config::new("phc_other", "https://eu.i.posthog.com").unwrap()));
        item.created = now() - TTL - 1; assert!(!item.permitted(home.path(), &config));
        item.created = now(); item.uuid = "-".repeat(36); assert!(!item.permitted(home.path(), &config));
        item.uuid = Stored::new(&config, &id, Event::AppOpened, Origin::Desktop, Outcome::Success, ErrorCode::None, false).uuid;
        set_enabled(home.path(), false).unwrap(); set_enabled(home.path(), true).unwrap();
        assert!(!item.permitted(home.path(), &config));
    }
    #[test]
    fn crash_build_and_source_map_changes_are_not_misinterpreted() {
        let home = tempfile::tempdir().unwrap(); set_enabled(home.path(), true).unwrap();
        let config = config(); let id = identity(home.path()).unwrap();
        let mut item = Stored::new(&config, &id, Event::PanicObserved, Origin::Daemon, Outcome::Failed, ErrorCode::Panic, true);
        assert!(item.permitted(home.path(), &config));
        item.site_manifest = Some("0".repeat(64)); assert!(!item.permitted(home.path(), &config));
        item.site_manifest = Some(crash::manifest()); item.version = "0.0.0".into();
        assert!(!item.permitted(home.path(), &config));
    }
    // The production worker retries opportunistic lock contention on its next
    // tick. A parallel test's process spawn can transiently retain a lock FD.
    // Retry only that explicit WouldBlock case, never a transport/I/O failure.
    fn pump_after_transient_lock(home: &Path, config: &Config) {
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            match pump(home, config) {
                Ok(()) => return,
                Err(error) if Instant::now() < until && error.get_ref().is_some_and(|inner|
                    matches!(inner.downcast_ref::<std::fs::TryLockError>(), Some(std::fs::TryLockError::WouldBlock))) => {
                    std::thread::sleep(Duration::from_millis(2));
                },
                Err(error) => panic!("pump failed: {error}"),
            }
        }
    }
    #[test]
    fn retry_pump_preserves_identity_and_removes_only_after_ack() {
        use std::net::TcpListener;
        let home = tempfile::tempdir().unwrap(); set_enabled(home.path(), true).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = Config { key: "phc_test".into(), host: format!("http://{}", listener.local_addr().unwrap()), environment: "development".into() };
        let server = std::thread::spawn(move || {
            for status in [503, 200] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut data = Vec::new(); let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap(); assert!(n > 0); data.extend_from_slice(&buffer[..n]);
                    if let Some(i) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..i]).to_lowercase();
                        let len: usize = header.lines().find_map(|l| l.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
                        if data.len() >= i + 4 + len { break; }
                    }
                }
                let body = "{\"status\":1}";
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        super::super::set_enabled_for(home.path(), true, &config.environment, Some(&config)).unwrap();
        let id = super::super::identity_for(home.path(), &config).unwrap();
        append(home.path(), &config, &id, Event::AppOpened, Origin::Desktop, Outcome::Success, ErrorCode::None).unwrap();
        let before = read(home.path())[0].clone();
        pump_after_transient_lock(home.path(), &config);
        let mut after = read(home.path()); assert_eq!(after.len(), 1); assert_eq!(after[0].attempts, 1);
        assert_eq!(after[0].payload(&config), before.payload(&config)); assert!(after[0].next > now());
        after[0].next = 0; write(home.path(), &after).unwrap();
        pump_after_transient_lock(home.path(), &config); assert!(read(home.path()).is_empty());
        server.join().unwrap();
    }
    #[test]
    fn capacity_private_permissions_and_purge() {
        let home = tempfile::tempdir().unwrap(); set_enabled(home.path(), true).unwrap();
        let config = config(); let id = identity(home.path()).unwrap();
        let items = (0..300).map(|_| Stored::new(&config, &id, Event::AppOpened, Origin::Desktop, Outcome::Success, ErrorCode::None, false)).collect();
        append_items(home.path(), &config, items).unwrap();
        assert_eq!(read(home.path()).len(), MAX_EVENTS);
        assert!(std::fs::metadata(home.path().join(FILE)).unwrap().len() <= MAX_BYTES);
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(home.path().join(FILE)).unwrap().permissions().mode() & 0o777, 0o600); }
        set_enabled(home.path(), false).unwrap(); assert!(!home.path().join(FILE).exists());
    }
    #[test]
    fn oversized_and_unknown_field_storage_is_rejected() {
        let home = tempfile::tempdir().unwrap();
        crate::private::write(&home.path().join(FILE), vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(read(home.path()).is_empty());
        crate::private::write(&home.path().join(FILE), b"[{\"raw_message\":\"SECRET_SENTINEL\"}]").unwrap();
        assert!(read(home.path()).is_empty());
    }
}
