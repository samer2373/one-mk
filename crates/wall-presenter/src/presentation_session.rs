use crate::gnome_bridge::{Bridge, Event, Result};
use futures_util::StreamExt;
use one_mk_wall_sync::{bridge::{Monitor, State}, presentation::{Gate, Owner, PresentationPlan, Rect, Slice}, presentation_wire};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Duration;
use zbus::{Connection, MatchRule, MessageStream, Proxy, message::Type};

const INTERFACE: &str = "org.onemk.WallpaperAuthority1";
const PATH: &str = "/org/onemk/WallpaperAuthority1";
const DEADLINE: Duration = Duration::from_millis(750);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    pub root: PathBuf,
    pub credential: String,
    pub generation: [u8; 16],
    pub revision: u64,
    pub peer: String,
    pub media: [u8; 32],
    pub canvas: [i32; 4],
    pub authority: String,
    pub pid: u32,
}

impl Bootstrap {
    pub fn read_environment() -> Result<Self> {
        let descriptor: i32 = std::env::var("ONE_MK_M1_BOOTSTRAP_FD")?.parse()?;
        if descriptor < 3 { return Err("bootstrap must be an explicit inherited descriptor".into()); }
        let mut bytes = Vec::new();
        unsafe { std::fs::File::from_raw_fd(descriptor) }.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 { return Err("bootstrap exceeds budget".into()); }
        let bootstrap: Self = serde_json::from_slice(&bytes)?;
        bootstrap.validate()?;
        Ok(bootstrap)
    }

    fn validate(&self) -> Result<()> {
        let root = &self.root;
        let metadata = std::fs::symlink_metadata(root)?;
        if !root.is_absolute() || root.canonicalize()? != *root || !metadata.is_dir() ||
            metadata.mode() & 0o777 != 0o700 || metadata.uid() != unsafe { libc::getuid() } {
            return Err("bootstrap root is not canonical, private and owned".into());
        }
        for (name, child) in [("HOME", "home"), ("XDG_RUNTIME_DIR", "run"), ("ONE_MK_HOME", "one-mk")] {
            if PathBuf::from(std::env::var(name)?) != root.join(child) {
                return Err("bootstrap environment escapes private laboratory".into());
            }
        }
        if std::env::var_os("DISPLAY").is_some() ||
            std::env::var("DBUS_SESSION_BUS_ADDRESS")? != std::fs::read_to_string(root.join("session-bus"))?.trim() ||
            std::env::var("DBUS_SYSTEM_BUS_ADDRESS")? != format!("unix:path={}/run/system_bus", root.display()) ||
            std::env::var("WAYLAND_DISPLAY")? != "spike-wl-0" ||
            self.credential.len() != 64 || !self.credential.bytes().all(|byte| byte.is_ascii_hexdigit()) ||
            self.generation == [0; 16] || self.revision == 0 || self.media == [0; 32] ||
            self.canvas[2] <= 0 || self.canvas[3] <= 0 {
            return Err("invalid or non-private laboratory bootstrap".into());
        }
        Ok(())
    }

    fn canvas(&self) -> Rect {
        Rect { x: self.canvas[0], y: self.canvas[1], width: self.canvas[2] as u32, height: self.canvas[3] as u32 }
    }
}

pub struct Authority {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Authority {
    fn drop(&mut self) { self.task.abort(); }
}

pub async fn authority(bootstrap: Bootstrap, policy: impl Fn() -> bool + Send + 'static) -> Result<Authority> {
    let connection = zbus::connection::Builder::session()?.max_queued(8).method_timeout(DEADLINE).build().await?;
    let rule = MatchRule::builder().msg_type(Type::MethodCall).path(PATH)?.interface(INTERFACE)?.build();
    let mut messages = MessageStream::for_match_rule(rule, &connection, Some(8)).await?;
    println!("wallpaper M1 authority {} {}", connection.unique_name().ok_or("no unique bus name")?, std::process::id());
    let task = tokio::spawn(async move {
        let mut pinned = None;
        let mut sequence = 0_u64;
        let mut revoked = false;
        let mut previous = Vec::new();
        let mut last_request = None;
        while let Some(Ok(message)) = messages.next().await {
            let result: Result<()> = async {
                let header = message.header();
                let sender = header.sender().ok_or("missing caller")?;
                if message.body().len() > 2048 || message.body().signature().to_string() != "sua(iiiid)" ||
                    header.member().map(|member| member.as_str()) != Some("Snapshot") {
                    return Err("invalid authority envelope".into());
                }
                let (credential, serial, monitors): (String, u32, Vec<Monitor>) = message.body().deserialize()?;
                if credential != bootstrap.credential || pinned.as_ref().is_some_and(|owner| owner != sender) {
                    return Err("wrong, stale or already consumed credential".into());
                }
                let bus = zbus::fdo::DBusProxy::new(&connection).await?;
                if bus.get_connection_unix_user(sender.clone().into()).await? != unsafe { libc::getuid() } {
                    return Err("wrong caller uid".into());
                }
                let now = std::time::Instant::now();
                if last_request.is_some_and(|previous: std::time::Instant| now.duration_since(previous) < Duration::from_millis(40)) {
                    return Err("authority request rate exceeded".into());
                }
                last_request = Some(now);
                one_mk_wall_sync::bridge::validate_layout(serial, &monitors)?;
                pinned = Some(sender.clone());
                revoked |= !policy();
                sequence = sequence.checked_add(1).ok_or("sequence exhausted")?;
                let mode = std::fs::read_to_string(bootstrap.root.join("plan-fault")).unwrap_or_default();
                let plan = PresentationPlan { generation: bootstrap.generation, consent_revision: bootstrap.revision,
                    sequence, layout_serial: serial, media_id: bootstrap.media, canvas: bootstrap.canvas(),
                    slices: monitors.iter().enumerate().map(|(index, &(x, y, width, height, _))| Slice {
                        screen: format!("monitor-{index}"), canvas: Rect { x, y, width: width as u32, height: height as u32 },
                    }).collect(), looping: true, paused: mode.trim() == "pause" };
                let mut payload = presentation_wire::encode(&plan).map_err(|_| "cannot encode presentation plan")?;
                match mode.trim() {
                    "replay" => payload = previous.clone(),
                    "stale" => { payload[24..32].copy_from_slice(&0_u64.to_le_bytes()); },
                    "oversize" => payload.resize(presentation_wire::MAX_PAYLOAD + 1, 0),
                    "malformed" => payload = b"bad".to_vec(),
                    _ => previous = payload.clone(),
                }
                connection.reply(&header, &(bootstrap.credential.clone(), !revoked, bootstrap.revision, payload)).await?;
                Ok(())
            }.await;
            if let Err(error) = result {
                let _ = connection.reply_error(&message.header(), "org.freedesktop.DBus.Error.AccessDenied", &error.to_string()).await;
            }
        }
    });
    Ok(Authority { task })
}

pub async fn present(bootstrap: Bootstrap) -> Result<()> {
    let descriptor = std::env::var("WAYLAND_SOCKET")?.parse()?;
    let mut bridge = Bridge::connect(descriptor).await?;
    let owner = Owner { unique_name: bootstrap.authority.clone(), uid: unsafe { libc::getuid() }, pid: bootstrap.pid };
    let bus = zbus::fdo::DBusProxy::new(bridge.connection()).await?;
    let name: zbus::names::BusName<'_> = bootstrap.authority.as_str().try_into()?;
    if !bootstrap.authority.starts_with(':') || bus.get_connection_unix_process_id(name.clone()).await? != owner.pid ||
        bus.get_connection_unix_user(name).await? != owner.uid {
        return Err("authority transport identity does not match inherited bootstrap".into());
    }
    let proxy = Proxy::new_owned(bridge.connection().clone(), bootstrap.authority.clone(), PATH, INTERFACE).await?;
    let changes = Proxy::new_owned(bridge.connection().clone(), "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").await?;
    let mut owners = changes.receive_signal_with_args("NameOwnerChanged", &[(0, &bootstrap.authority)]).await?;
    let mut gate = Gate::new(owner.clone(), bootstrap.revision, bootstrap.generation, bootstrap.canvas(),
        bridge.serial(), bridge.monitors.clone()).map_err(|error| format!("invalid bootstrap gate: {error:?}"))?;
    gate.pause_hint(bridge.reasons).map_err(|error| format!("invalid initial pause: {error:?}"))?;
    bridge.set_state(State::Paused).await?;
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = owners.next() => { gate.owner_lost(); return Err("authority owner lost; presentation destroyed".into()); },
            event = bridge.next() => match event? {
                Event::Pause(reasons) => gate.pause_hint(reasons).map_err(|error| format!("pause refused: {error:?}"))?,
                Event::Layout(serial, monitors) => gate.layout_changed(serial, monitors).map_err(|error| format!("layout refused: {error:?}"))?,
            },
            _ = interval.tick() => {
                let reply = tokio::time::timeout(DEADLINE, proxy.call_method("Snapshot",
                    &(bootstrap.credential.clone(), bridge.serial(), bridge.monitors.clone()))).await??;
                if reply.header().sender().map(|sender| sender.as_str()) != Some(&bootstrap.authority) ||
                    reply.body().len() > 4352 || reply.body().signature().to_string() != "sbtay" {
                    return Err("invalid presentation envelope".into());
                }
                let (proof, enabled, revision, payload): (String, bool, u64, Vec<u8>) = reply.body().deserialize()?;
                if proof != bootstrap.credential { return Err("authority credential proof refused".into()); }
                gate.reconcile_consent(enabled, Some(revision));
                if !enabled || revision != bootstrap.revision { return Err("consent revoked; presentation destroyed".into()); }
                match gate.accept_encoded(&owner, &payload, enabled, Some(revision)) {
                    Ok(()) => println!("M1 plan accepted sequence={} active={}",
                        presentation_wire::decode(&payload).map_err(|_| "accepted invalid wire")?.sequence, gate.active().is_some()),
                    Err(error) => println!("M1 plan refused {error:?} previous_active={}", gate.active().is_some()),
                }
            },
        }
    }
}
