//! 6D-M1 laboratory renderer session (feature `m1-lab`, never in release
//! builds). The Shell spawns this process; it authenticates the Shell bridge,
//! then the daemon authority named by the private runtime record, and admits
//! the daemon's plans through `Gate::accept_encoded`. While the admitted plan
//! is active (not paused, nothing pausing locally) the presenter plays its
//! imported media (step B, `playback.rs`); otherwise there is no surface and
//! no decoder. Any teardown (revoked consent, daemon exit/crash, forged reply,
//! repeated faults, unanswered request, playback failure) ends the process.
//!
//! Lifecycle (step C): the Shell ends a renderer with SIGTERM (lock, sleep,
//! cover, disable, layout failure) and SIGKILL only after a grace period.
//! SIGTERM is a clean teardown: the decode worker and its cgroup are removed
//! and the surface is destroyed before the process exits. A layout change
//! drops the admitted plan and the surface at once; only a fresh plan for the
//! new layout is presented again.

use crate::gnome_bridge::{Bridge, Event, Result};
use crate::playback::{self, Presenter, Report, Request};
use futures_util::StreamExt;
use one_mk_wall_sync::bridge::State;
use one_mk_wall_sync::bridge::Monitor;
use one_mk_wall_sync::presentation::{Owner, Rect, Slice};
use one_mk_wall_sync::presentation_auth::{self as auth, Outcome, Record, Reply, Session};
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::Duration;
use zbus::{Connection, MatchRule, MessageStream, message::Type};

const DEADLINE: Duration = Duration::from_millis(750);
/// Safety net for a lost `Changed` signal; owner loss is observed directly.
const HEARTBEAT: Duration = Duration::from_secs(2);

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

fn log(message: std::fmt::Arguments<'_>) {
    eprintln!("wallpaper-m1-renderer[{}]: {message}", std::process::id());
}

/// Reads the daemon's record: private directories, a private regular file
/// owned by this user, never through a symlink, bounded.
fn read_record() -> Result<Record> {
    let base = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?);
    let metadata = std::fs::symlink_metadata(&base)?;
    if !base.is_absolute() || !metadata.is_dir() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
        return Err("XDG_RUNTIME_DIR is not private to this user".into());
    }
    let directory = base.join(auth::RUNTIME_DIRECTORY);
    let metadata = std::fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.uid() != uid() || metadata.mode() & 0o777 != 0o700 {
        return Err("authority directory is not private to this user".into());
    }
    let file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory.join(auth::RECORD_FILE))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
        return Err("authority record is not a private file owned by this user".into());
    }
    let mut bytes = Vec::new();
    file.take(auth::MAX_RECORD as u64 + 1).read_to_end(&mut bytes)?;
    Record::decode(&bytes).map_err(|error| format!("invalid authority record: {error:?}").into())
}

fn nonce() -> Result<[u8; auth::NONCE_BYTES]> {
    let mut bytes = [0_u8; auth::NONCE_BYTES];
    // SAFETY: the buffer is valid for its length.
    let filled = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    if filled != bytes.len() as isize { return Err("no randomness for a request nonce".into()); }
    Ok(bytes)
}

fn describe(session: &Session) -> String {
    let Some(plan) = session.admitted() else { return "admitted_sequence=none active=false".into() };
    let media: String = plan.media_id[..8].iter().map(|byte| format!("{byte:02x}")).collect();
    let slices: Vec<String> = plan.slices.iter().map(|slice| format!("{}@{},{},{}x{}", slice.screen,
        slice.canvas.x, slice.canvas.y, slice.canvas.width, slice.canvas.height)).collect();
    format!("admitted_sequence={} paused={} active={} revision={} media={media} canvas={},{},{}x{} slices={}",
        plan.sequence, plan.paused, session.active(), plan.consent_revision, plan.canvas.x, plan.canvas.y,
        plan.canvas.width, plan.canvas.height, slices.join(";"))
}

/// One authenticated `Plan` round trip; `Err` means the presentation is over.
async fn fetch(connection: &Connection, session: &mut Session, owner: &Owner) -> Result<()> {
    let nonce = nonce()?;
    let tag = session.request_tag(&nonce);
    let body = (nonce.to_vec(), session.serial(), session.monitors().to_vec(), tag.to_vec());
    let call = connection.call_method(Some(owner.unique_name.as_str()), auth::PATH, Some(auth::INTERFACE), auth::METHOD,
        &body);
    let reply = match tokio::time::timeout(DEADLINE, call).await {
        Err(_) => { session.owner_lost(); return Err("authority did not answer in time; presentation destroyed".into()); }
        Ok(Err(zbus::Error::MethodError(_, message, _))) => {
            session.owner_lost();
            return Err(format!("authority refused this renderer ({}); presentation destroyed",
                message.unwrap_or_default()).into());
        }
        Ok(Err(error)) => { session.owner_lost(); return Err(error.into()); }
        Ok(Ok(reply)) => reply,
    };
    let header = reply.header();
    let outcome = if header.sender().map(|sender| sender.as_str()) != Some(owner.unique_name.as_str()) {
        session.owner_lost();
        return Err("reply from another connection; presentation destroyed".into());
    } else if reply.body().len() > auth::MAX_REPLY_BODY || reply.body().signature().to_string() != auth::REPLY_SIGNATURE {
        session.invalid_envelope()
    } else {
        match reply.body().deserialize::<(bool, u64, Vec<u8>, Vec<u8>)>() {
            Ok((enabled, revision, payload, tag)) =>
                session.reply(owner, &nonce, Reply { enabled, revision, payload: &payload, tag: &tag }),
            Err(_) => session.invalid_envelope(),
        }
    };
    match outcome {
        Outcome::Accepted { sequence, .. } => log(format_args!("event=accepted sequence={sequence} {}", describe(session))),
        Outcome::Unchanged => {}
        Outcome::Refused(refusal) => log(format_args!("event=refused reason={refusal:?} {}", describe(session))),
        Outcome::Teardown(reason) => return Err(format!("presentation torn down: {reason:?}").into()),
    }
    Ok(())
}

/// What is on screen: the media and exactly where each slice goes.
#[derive(Clone, Debug, PartialEq)]
struct Showing { media: [u8; 32], canvas: Rect, slices: Vec<Slice>, monitors: Vec<Monitor> }

/// The playback the session is showing (its presenter generation), if any.
#[derive(Default)]
struct Screen { showing: Option<Showing>, generation: Option<u64> }

/// Starts, replaces or stops playback so the screen follows the session.
/// Stopping reports Paused first, so the Shell hides the clones before the
/// surface and decoder go away.
async fn reconcile(session: &Session, bridge: &mut Bridge, presenter: &mut Presenter, screen: &mut Screen) -> Result<()> {
    let plan = session.active().then(|| session.admitted()).flatten();
    let wanted = plan.map(|plan| Showing { media: plan.media_id, canvas: plan.canvas, slices: plan.slices.clone(),
        monitors: session.monitors().to_vec() });
    if wanted == screen.showing { return Ok(()); }
    match plan {
        Some(plan) => {
            log(format_args!("event=play sequence={}", plan.sequence));
            screen.generation = Some(presenter.play(Request { plan: plan.clone(), monitors: session.monitors().to_vec() }));
        }
        None => {
            bridge.set_state(State::Paused).await?;
            presenter.stop();
            screen.generation = None;
            log(format_args!("event=stopped surfaces=0 decoders=0 {}", describe(session)));
        }
    }
    screen.showing = wanted;
    Ok(())
}

/// Runs the presentation until a teardown, or until the Shell sends SIGTERM.
/// Either way the presenter is dropped before returning: the decode worker is
/// killed, its cgroup removed and the surface destroyed.
pub async fn present() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    // Installed first, so a SIGTERM at any point is a clean teardown.
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        biased;
        _ = terminate.recv() => {}
        result = run() => return result,
    }
    // The session future, and with it the presenter, was dropped above.
    log(format_args!("event=terminated signal=SIGTERM surfaces=0 decoders=0"));
    Err("terminated by the Shell (SIGTERM); presentation destroyed".into())
}

async fn run() -> Result<()> {
    let descriptor = std::env::var("WAYLAND_SOCKET")?.parse()?;
    let config = playback::Config::from_env().map_err(|error| format!("playback unavailable: {error}"))?;
    // A previous renderer killed outright cannot remove its worker's group.
    match one_mk_wall_import::sandbox::reclaim_playback_groups(&config.cgroup_root) {
        Ok(0) => {}
        Ok(count) => log(format_args!("event=reclaimed stale_playback_groups={count}")),
        Err(error) => log(format_args!("event=reclaim-failed error={error:?}")),
    }
    let mut bridge = Bridge::connect(descriptor).await?;
    bridge.set_state(State::Paused).await?;
    let record = read_record().map_err(|error| format!("no usable authority record: {error}"))?;
    let connection = bridge.connection().clone();
    // Subscribe before checking the owner so a loss in between is still seen.
    let names = zbus::Proxy::new_owned(connection.clone(), "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus").await?;
    let mut owners = names.receive_signal_with_args("NameOwnerChanged", &[(0, record.unique_name.as_str())]).await?;
    let rule = MatchRule::builder().msg_type(Type::Signal).sender(record.unique_name.as_str())?.path(auth::PATH)?
        .interface(auth::INTERFACE)?.member(auth::CHANGED)?.build();
    let mut changes = MessageStream::for_match_rule(rule, &connection, Some(4)).await?;
    let bus = zbus::fdo::DBusProxy::new(&connection).await?;
    let name: zbus::names::BusName<'_> = record.unique_name.as_str().try_into()?;
    let pid = bus.get_connection_unix_process_id(name.clone()).await
        .map_err(|_| "stale authority record: its bus name has no owner")?;
    let owner_uid = bus.get_connection_unix_user(name).await?;
    if pid != record.pid || owner_uid != uid() {
        return Err("authority record does not match the connection that owns its name".into());
    }
    let owner = Owner { unique_name: record.unique_name.clone(), uid: owner_uid, pid };
    let me = connection.unique_name().ok_or("no unique bus name")?.to_string();
    let mut session = Session::new(owner.clone(), record, me, bridge.serial(), bridge.monitors.clone(), bridge.reasons)
        .map_err(|error| format!("cannot open a presentation session: {error:?}"))?;
    log(format_args!("event=authenticating authority={} pid={pid}", owner.unique_name));
    // SAFETY: the Shell gave this process the connection (its peer was pinned
    // to the bridge owner above); nothing else uses the descriptor.
    let socket = unsafe { std::os::unix::net::UnixStream::from_raw_fd(descriptor) };
    let mut presenter = Presenter::start(socket, config)?;
    let mut screen = Screen::default();
    fetch(&connection, &mut session, &owner).await?;
    reconcile(&session, &mut bridge, &mut presenter, &mut screen).await?;
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = owners.next() => {
                session.owner_lost();
                return Err("authority connection lost (daemon exit, crash or kill); presentation destroyed".into());
            }
            event = bridge.next() => match event? {
                Event::Pause(reasons) => {
                    session.pause_hint(reasons).map_err(|reason| format!("presentation torn down: {reason:?}"))?;
                    log(format_args!("event=pause reasons={reasons} {}", describe(&session)));
                }
                Event::Layout(serial, monitors) => {
                    session.layout(serial, monitors).map_err(|reason| format!("presentation torn down: {reason:?}"))?;
                    log(format_args!("event=layout serial={serial} {}", describe(&session)));
                    fetch(&connection, &mut session, &owner).await?;
                }
            },
            change = changes.next() => {
                if change.is_none() { return Err("authority signal stream closed".into()); }
                fetch(&connection, &mut session, &owner).await?;
            }
            _ = heartbeat.tick() => fetch(&connection, &mut session, &owner).await?,
            report = presenter.reports.recv() => match report {
                Some(Report::FirstFrame { generation, latency }) if screen.generation == Some(generation) => {
                    bridge.set_state(State::Playing).await?;
                    log(format_args!("event=playing first_frame_ms={}", latency.as_millis()));
                }
                Some(Report::Holding { generation }) if screen.generation == Some(generation) => {
                    bridge.set_state(State::Holding).await?;
                    log(format_args!("event=holding"));
                }
                Some(Report::Moving { generation }) if screen.generation == Some(generation) => {
                    bridge.set_state(State::Playing).await?;
                    log(format_args!("event=moving"));
                }
                // A report from a playback that was replaced or stopped.
                Some(Report::FirstFrame { .. } | Report::Holding { .. } | Report::Moving { .. }) => {}
                Some(Report::Failed(error)) => return Err(format!("playback failed: {error}").into()),
                None => return Err("presenter ended".into()),
            },
        }
        reconcile(&session, &mut bridge, &mut presenter, &mut screen).await?;
    }
}
