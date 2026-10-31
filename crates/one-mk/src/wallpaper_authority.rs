//! 6D-M1 step A laboratory only (feature `wallpaper-m1-lab`, never in release
//! builds): the daemon's presentation authority on the session bus.
//!
//! On start it creates a fresh key and authority generation, serves
//! `org.onemk.WallpaperAuthority1.Plan` on its own unique connection and
//! publishes a private record in `$XDG_RUNTIME_DIR/one-mk-wallpaper-m1/`.
//! Only a caller that proves the key (and runs as this user) gets a plan, and
//! every plan is tagged for that caller's nonce. Consent comes from the live
//! `[wallpaper]` settings the daemon applies; a change is announced to the
//! pinned renderer at once. Exit removes the record; a crash leaves a stale
//! record whose name has no owner. Production authorization stays false.

use crate::wallpaper::WallpaperSettings;
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use one_mk_wall_sync::bridge::Monitor;
use one_mk_wall_sync::presentation::Rect;
use one_mk_wall_sync::presentation_auth::{self as auth, Publisher, Record};
use one_mk_wall_sync::presentation_wire;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use zbus::{MatchRule, MessageStream, message::Type};

/// An inherited descriptor carrying the laboratory configuration.
pub const LAB_FD: &str = "ONE_MK_M1_LAB_FD";
/// In the daemon's home; read on SIGUSR1 to pause/play or inject one fault.
const DIRECTIVE: &str = "wallpaper-m1-lab-directive";
const REQUESTS_PER_SECOND: u32 = 20;
const REFUSALS_LOGGED_PER_SECOND: u32 = 8;

#[derive(Default)]
struct Window { start: Option<Instant>, count: u32 }

impl Window {
    fn admit(&mut self, limit: u32) -> bool {
        let now = Instant::now();
        if self.start.is_none_or(|start| now.duration_since(start) >= Duration::from_secs(1)) {
            *self = Self { start: Some(now), count: 0 };
        }
        self.count += 1;
        self.count <= limit
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LabConfig {
    /// The coordinator whose consent this receiver's presentation needs.
    peer: String,
    /// Imported media content hash (hex).
    media: String,
    /// The shared canvas: x, y, width, height.
    canvas: [i64; 4],
    /// Where this computer's (0, 0) sits on that canvas.
    origin: [i32; 2],
}

struct Policy { peer: String, consent: watch::Sender<Option<u64>> }
static POLICY: OnceLock<Policy> = OnceLock::new();

fn consent(peer: &str, settings: &WallpaperSettings) -> Option<u64> {
    settings.grants.get(peer).copied().filter(|_| settings.enabled)
}

/// Called whenever the daemon applies `[wallpaper]` settings.
pub fn publish(settings: &WallpaperSettings) {
    if let Some(policy) = POLICY.get() {
        let live = consent(&policy.peer, settings);
        policy.consent.send_if_modified(|current| std::mem::replace(current, live) != live);
    }
}

pub fn requested() -> bool { std::env::var_os(LAB_FD).is_some() }

fn hex<const N: usize>(text: &str) -> Result<[u8; N]> {
    ensure!(text.len() == N * 2 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected {} lowercase hex digits", N * 2);
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)?;
    }
    Ok(bytes)
}

fn read_config() -> Result<LabConfig> {
    let descriptor: i32 = std::env::var(LAB_FD)?.parse().context("laboratory descriptor")?;
    ensure!(descriptor >= 3, "the laboratory configuration must be an inherited descriptor");
    // SAFETY: the descriptor was handed to this process for this purpose and is
    // used (then closed) only here.
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4096, "laboratory configuration exceeds 4096 bytes");
    Ok(serde_json::from_slice(&bytes)?)
}

/// A mode-0700 directory owned by this user, never through a symlink.
fn private_directory(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    ensure!(metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o777 == 0o700,
        "{} is not a private directory owned by this user", path.display());
    Ok(())
}

fn runtime_directory() -> Result<PathBuf> {
    let base = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?);
    ensure!(base.is_absolute(), "XDG_RUNTIME_DIR must be absolute");
    let metadata = std::fs::symlink_metadata(&base)?;
    // SAFETY: getuid has no preconditions.
    ensure!(metadata.is_dir() && metadata.uid() == unsafe { libc::getuid() } && metadata.mode() & 0o077 == 0,
        "XDG_RUNTIME_DIR is not a private directory owned by this user");
    let directory = base.join(auth::RUNTIME_DIRECTORY);
    private_directory(&directory)?;
    Ok(directory)
}

fn random<const N: usize>() -> [u8; N] {
    loop {
        let bytes: [u8; N] = rand::random();
        if bytes != [0; N] { return bytes; }
    }
}

pub struct Authority {
    task: tokio::task::JoinHandle<()>,
    record: PathBuf,
    generation: [u8; 16],
    _lock: std::fs::File,
}

impl Drop for Authority {
    fn drop(&mut self) {
        self.task.abort();
        // Remove the record only while it is still ours.
        let ours = std::fs::read(&self.record).ok().and_then(|bytes| Record::decode(&bytes).ok())
            .is_some_and(|record| record.generation == self.generation);
        if ours && std::fs::remove_file(&self.record).is_ok() {
            eprintln!("wallpaper-m1-authority: stopped; record removed");
        }
    }
}

pub async fn start(dir: &Path, settings: &WallpaperSettings) -> Result<Authority> {
    let config = read_config().context("wallpaper M1 laboratory configuration")?;
    hex::<32>(&config.peer).context("peer fingerprint")?;
    let media = hex::<32>(&config.media).context("media content hash")?;
    let [x, y, width, height] = config.canvas;
    let canvas = Rect { x: i32::try_from(x)?, y: i32::try_from(y)?, width: u32::try_from(width)?, height: u32::try_from(height)? };
    let generation = random::<16>();
    let mut publisher = Publisher::new(generation, media, canvas, (config.origin[0], config.origin[1]))
        .map_err(|error| anyhow::anyhow!("invalid laboratory plan: {error:?}"))?;
    let directory = runtime_directory()?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(directory.join("authority.lock"))?;
    // SAFETY: flock on a descriptor this function owns.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("another wallpaper authority is running for this user");
    }
    let (sender, mut live) = watch::channel(consent(&config.peer, settings));
    POLICY.set(Policy { peer: config.peer, consent: sender }).map_err(|_| anyhow::anyhow!("authority already started"))?;
    let connection = zbus::connection::Builder::session()?.max_queued(16).build().await
        .context("session bus for the wallpaper authority")?;
    let authority = connection.unique_name().context("no unique bus name")?.to_string();
    let rule = MatchRule::builder().msg_type(Type::MethodCall).path(auth::PATH)?.interface(auth::INTERFACE)?.build();
    let mut messages = MessageStream::for_match_rule(rule, &connection, Some(16)).await?;
    let mut directives = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())?;
    let key = random::<32>();
    let record = Record { key, generation, pid: std::process::id(), unique_name: authority.clone() };
    let path = directory.join(auth::RECORD_FILE);
    let staging = directory.join(format!("{}.{}", auth::RECORD_FILE, std::process::id()));
    let _ = std::fs::remove_file(&staging);
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&staging)?;
    file.write_all(&record.encode().map_err(|error| anyhow::anyhow!("authority record: {error:?}"))?)?;
    file.sync_all()?;
    std::fs::rename(&staging, &path)?;
    eprintln!("wallpaper-m1-authority: ready name={authority} pid={} consent={}", std::process::id(),
        live.borrow().is_some());
    let directive_path = dir.join(DIRECTIVE);
    let task = tokio::spawn(async move {
        let bus = match zbus::fdo::DBusProxy::new(&connection).await {
            Ok(bus) => bus,
            Err(error) => { eprintln!("wallpaper-m1-authority: bus proxy failed: {error}"); return; }
        };
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let mut pinned: Option<String> = None;
        let mut paused = false;
        let mut fault: Option<String> = None;
        // Callers that cannot prove the key have their own budget (and stop
        // being logged past it), so they cannot starve the genuine renderer.
        let mut authenticated_window = Window::default();
        let mut refused_window = Window::default();
        loop {
            tokio::select! {
                message = messages.next() => {
                    let Some(Ok(message)) = message else { break };
                    let header = message.header();
                    let Some(sender) = header.sender().map(|sender| sender.to_string()) else { continue };
                    let mut authenticated = false;
                    let result: Result<(bool, u64, Vec<u8>, [u8; 32])> = async {
                        ensure!(header.member().map(|member| member.as_str()) == Some(auth::METHOD) &&
                            message.body().len() <= auth::MAX_REQUEST_BODY &&
                            message.body().signature().to_string() == auth::REQUEST_SIGNATURE, "invalid request envelope");
                        let (nonce, serial, monitors, tag): (Vec<u8>, u32, Vec<Monitor>, Vec<u8>) = message.body().deserialize()?;
                        let nonce: [u8; auth::NONCE_BYTES] = nonce.as_slice().try_into().context("invalid nonce")?;
                        ensure!(auth::tag_matches(&auth::request_tag(&key, &sender, &authority, &nonce, serial, &monitors), &tag),
                            "credential refused");
                        authenticated = true;
                        ensure!(authenticated_window.admit(REQUESTS_PER_SECOND), "request rate exceeded");
                        let name: zbus::names::BusName<'_> = sender.as_str().try_into()?;
                        ensure!(bus.get_connection_unix_user(name).await? == uid, "caller is another user");
                        if let Some(previous) = pinned.as_ref().filter(|previous| **previous != sender) {
                            let previous: zbus::names::BusName<'_> = previous.as_str().try_into()?;
                            ensure!(!bus.name_has_owner(previous).await?, "another renderer holds this authority");
                        }
                        if pinned.as_ref() != Some(&sender) {
                            eprintln!("wallpaper-m1-authority: renderer {sender} authenticated");
                            pinned = Some(sender.clone());
                        }
                        let consent = *live.borrow();
                        let (enabled, revision, mut payload) = match consent {
                            Some(revision) => (true, revision, publisher.plan(revision, serial, &monitors, paused)
                                .map_err(|error| anyhow::anyhow!("cannot plan for this layout: {error:?}"))?),
                            None => (false, 0, Vec::new()),
                        };
                        let mut forge = false;
                        if let Some(kind) = fault.take().filter(|_| enabled) {
                            forge = inject(&kind, &mut payload, &publisher, revision);
                            eprintln!("wallpaper-m1-authority: injected {kind} ({} bytes)", payload.len());
                        }
                        let tag = if forge { [0; 32] } else {
                            auth::reply_tag(&key, &sender, &authority, &nonce, enabled, revision, &payload)
                        };
                        Ok((enabled, revision, payload, tag))
                    }.await;
                    match result {
                        Ok((enabled, revision, payload, tag)) => {
                            let _ = connection.reply(&header, &(enabled, revision, payload, tag.to_vec())).await;
                        }
                        Err(error) => {
                            if authenticated || refused_window.admit(REFUSALS_LOGGED_PER_SECOND) {
                                eprintln!("wallpaper-m1-authority: refused {sender}: {error:#}");
                            }
                            let _ = connection.reply_error(&header, "org.freedesktop.DBus.Error.AccessDenied", &format!("{error:#}")).await;
                        }
                    }
                }
                changed = live.changed() => {
                    if changed.is_err() { break; }
                    eprintln!("wallpaper-m1-authority: consent {}", if live.borrow().is_some() { "granted" } else { "revoked" });
                    notify(&connection, pinned.as_deref()).await;
                }
                _ = directives.recv() => {
                    let text = std::fs::File::open(&directive_path).and_then(|file| {
                        let mut text = String::new();
                        file.take(64).read_to_string(&mut text).map(|_| text)
                    }).unwrap_or_default();
                    let _ = std::fs::remove_file(&directive_path);
                    match text.trim() {
                        "pause" => paused = true,
                        "play" => paused = false,
                        kind @ ("replay" | "stale-sequence" | "stale-revision" | "oversize" | "oversize-envelope" |
                            "malformed" | "forge") => fault = Some(kind.to_owned()),
                        other => { eprintln!("wallpaper-m1-authority: unknown directive {other:?}"); continue; }
                    }
                    eprintln!("wallpaper-m1-authority: directive {}", text.trim());
                    notify(&connection, pinned.as_deref()).await;
                }
            }
        }
    });
    Ok(Authority { task, record: path, generation, _lock: lock })
}

/// Tells the pinned renderer to fetch a fresh plan (the signal carries nothing).
async fn notify(connection: &zbus::Connection, renderer: Option<&str>) {
    if let Some(renderer) = renderer {
        let _ = connection.emit_signal(Some(renderer), auth::PATH, auth::INTERFACE, auth::CHANGED, &()).await;
    }
}

/// Laboratory fault injection; returns whether to forge the reply tag.
fn inject(kind: &str, payload: &mut Vec<u8>, publisher: &Publisher, revision: u64) -> bool {
    let current = presentation_wire::decode(payload).ok();
    match kind {
        "replay" => if let Some(previous) = publisher.previous() { *payload = previous.to_vec() },
        "stale-sequence" => if let Some(mut plan) = current {
            plan.sequence = 1;
            plan.media_id[0] ^= 0xff;
            *payload = presentation_wire::encode(&plan).unwrap_or_default();
        },
        "stale-revision" => if let Some(mut plan) = current {
            plan.sequence = publisher.sequence() + 1;
            plan.consent_revision = if revision > 1 { revision - 1 } else { revision + 1 };
            *payload = presentation_wire::encode(&plan).unwrap_or_default();
        },
        "oversize" => payload.resize(presentation_wire::MAX_PAYLOAD + 1, 0),
        "oversize-envelope" => *payload = vec![0; 65536],
        "malformed" => if payload.len() > 6 { payload[6] = 1 },
        "forge" => return true,
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_mk_wall_sync::presentation::PresentationPlan;

    #[test]
    fn consent_follows_live_policy_for_the_configured_peer_only() {
        let peer = "ab".repeat(32);
        let mut settings = WallpaperSettings { enabled: true, ..Default::default() };
        assert_eq!(consent(&peer, &settings), None);
        settings.grants.insert("cd".repeat(32), 4);
        assert_eq!(consent(&peer, &settings), None);
        settings.grants.insert(peer.clone(), 9);
        assert_eq!(consent(&peer, &settings), Some(9));
        settings.enabled = false;
        assert_eq!(consent(&peer, &settings), None);
    }

    #[test]
    fn laboratory_hex_and_budgets_are_strict() {
        assert_eq!(hex::<2>("0aff").unwrap(), [10, 255]);
        for bad in ["0AFF", "0af", "0affe0", "0agf", ""] { assert!(hex::<2>(bad).is_err(), "{bad}"); }
        let mut window = Window::default();
        assert!((0..3).all(|_| window.admit(3)));
        assert!(!window.admit(3));
    }

    #[test]
    fn injected_faults_change_exactly_what_they_name() {
        let mut publisher = Publisher::new([1; 16], [2; 32], Rect { x: 0, y: 0, width: 100, height: 100 }, (0, 0)).unwrap();
        let monitors = [(0, 0, 100, 100, 1.0)];
        let first = publisher.plan(7, 1, &monitors, false).unwrap();
        let second = publisher.plan(7, 1, &monitors, true).unwrap();
        let mut payload = second.clone();
        assert!(!inject("replay", &mut payload, &publisher, 7));
        assert_eq!(payload, first);
        type Check = fn(&PresentationPlan) -> bool;
        let checks: [(&str, Check); 2] = [
            ("stale-sequence", |plan| plan.sequence == 1 && plan.media_id != [2; 32]),
            ("stale-revision", |plan| plan.sequence == 3 && plan.consent_revision == 6),
        ];
        for (kind, check) in checks {
            let mut payload = second.clone();
            assert!(!inject(kind, &mut payload, &publisher, 7));
            assert!(check(&presentation_wire::decode(&payload).unwrap()), "{kind}");
        }
        let mut payload = second.clone();
        inject("oversize", &mut payload, &publisher, 7);
        assert_eq!(payload.len(), presentation_wire::MAX_PAYLOAD + 1);
        let mut payload = second.clone();
        inject("malformed", &mut payload, &publisher, 7);
        assert!(presentation_wire::decode(&payload).is_err());
        let mut payload = second.clone();
        assert!(inject("forge", &mut payload, &publisher, 7));
        assert_eq!(payload, second);
    }
}
