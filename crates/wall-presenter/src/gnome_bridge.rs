// SPDX-License-Identifier: MIT OR Apache-2.0
use futures_util::StreamExt;
use one_mk_wall_sync::bridge::{self, Monitor, State};
use std::os::fd::RawFd;
use std::time::{Duration, Instant};
use zbus::{Connection, Proxy, proxy::SignalStream};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug)]
pub enum Event { Layout(u32, Vec<Monitor>), Pause(u32) }

pub struct Bridge {
    connection: Connection,
    proxy: Proxy<'static>,
    signals: SignalStream<'static>,
    owner_changes: SignalStream<'static>,
    owner: String,
    serial: u32,
    pub monitors: Vec<Monitor>,
    pub reasons: u32,
    last_state: Option<State>,
    state_times: Vec<Instant>,
}

impl Bridge {
    pub async fn connect(wayland_fd: RawFd) -> Result<Self> {
        let connection = zbus::connection::Builder::session()?
            .method_timeout(Duration::from_secs(1)).max_queued(16).build().await?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await?;
        let changes = Proxy::new_owned(connection.clone(), "org.freedesktop.DBus",
            "/org/freedesktop/DBus", "org.freedesktop.DBus").await?;
        let owner_changes = changes.receive_signal_with_args("NameOwnerChanged", &[(0, bridge::NAME)]).await?;
        let owner = bus.get_name_owner(bridge::NAME.try_into()?).await?;
        let shell = bus.get_name_owner("org.gnome.Shell".try_into()?).await?;
        let pid = bus.get_connection_unix_process_id(owner.clone().into()).await?;
        let shell_pid = bus.get_connection_unix_process_id(shell.into()).await?;
        let uid = bus.get_connection_unix_user(owner.clone().into()).await?;
        if pid != shell_pid || uid != unsafe { libc::getuid() } {
            return Err("bridge is not owned by this user's GNOME Shell".into());
        }
        authenticate_wayland(wayland_fd, pid)?;
        let proxy = Proxy::new_owned(connection.clone(), owner.to_string(), bridge::PATH, bridge::NAME).await?;
        let mut signals = proxy.receive_all_signals().await?;
        let reply = proxy.call_method("Hello", &(bridge::VERSION,)).await?;
        validate_message(&reply, &owner, "(uua(iiiid))")?;
        let (version, serial, monitors): (u32, u32, Vec<Monitor>) = reply.body().deserialize()?;
        if version != bridge::VERSION { return Err("unsupported bridge version".into()); }
        bridge::validate_layout(serial, &monitors)?;
        let snapshot = tokio::time::timeout(Duration::from_secs(1), signals.next()).await?
            .ok_or("bridge disappeared before pause snapshot")?;
        validate_message(&snapshot, &owner, "u")?;
        if snapshot.header().member().map(|member| member.as_str()) != Some("PauseHint") {
            return Err("first bridge signal was not PauseHint".into());
        }
        let (reasons,): (u32,) = snapshot.body().deserialize()?;
        bridge::validate_reasons(reasons)?;
        if bus.get_name_owner(bridge::NAME.try_into()?).await? != owner {
            return Err("bridge owner changed during Hello".into());
        }
        Ok(Self { connection, proxy, signals, owner_changes, owner: owner.to_string(), serial,
            monitors, reasons, last_state: None, state_times: Vec::new() })
    }

    pub async fn set_state(&mut self, state: State) -> Result<()> {
        if self.last_state == Some(state) { return Ok(()); }
        let now = Instant::now();
        self.state_times.retain(|time| now.duration_since(*time) < Duration::from_secs(1));
        if self.state_times.len() >= 10 { return Err("renderer state rate exceeded".into()); }
        self.state_times.push(now);
        let reply = self.proxy.call_method("SetState", &(state as u32,)).await?;
        validate_message(&reply, &self.owner, "")?;
        self.last_state = Some(state);
        Ok(())
    }

    pub async fn next(&mut self) -> Result<Event> {
        let message = tokio::select! {
            _ = self.owner_changes.next() => return Err("bridge owner changed; discard surface and reconnect".into()),
            message = self.signals.next() => message.ok_or("bridge signal stream closed")?,
        };
        match message.header().member().map(|member| member.as_str()) {
            Some("PauseHint") => {
                validate_message(&message, &self.owner, "u")?;
                let (reasons,): (u32,) = message.body().deserialize()?;
                bridge::validate_reasons(reasons)?;
                self.reasons = reasons;
                Ok(Event::Pause(reasons))
            }
            Some("Layout") => {
                validate_message(&message, &self.owner, "(ua(iiiid))")?;
                let (serial, monitors): (u32, Vec<Monitor>) = message.body().deserialize()?;
                bridge::validate_layout(serial, &monitors)?;
                if serial <= self.serial { return Err("non-increasing bridge layout serial".into()); }
                self.serial = serial;
                self.monitors = monitors.clone();
                Ok(Event::Layout(serial, monitors))
            }
            _ => Err("unexpected bridge signal".into()),
        }
    }

    pub fn connection(&self) -> &Connection { &self.connection }
    pub fn owner(&self) -> &str { &self.owner }
    pub fn serial(&self) -> u32 { self.serial }
}

fn validate_message(message: &zbus::Message, owner: &str, signature: &str) -> Result<()> {
    if message.body().len() > bridge::MAX_PAYLOAD || message.body().signature().to_string() != signature ||
        message.header().sender().map(|sender| sender.as_str()) != Some(owner) {
        return Err(format!("invalid bridge message: owner={:?} expected={owner}, signature={} expected={signature}, bytes={}",
            message.header().sender(), message.body().signature(), message.body().len()).into());
    }
    Ok(())
}

fn authenticate_wayland(descriptor: RawFd, owner_pid: u32) -> Result<()> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe { libc::getsockopt(descriptor, libc::SOL_SOCKET, libc::SO_PEERCRED,
        (&mut credentials as *mut libc::ucred).cast(), &mut length) } != 0 ||
        credentials.pid <= 0 || credentials.pid as u32 != owner_pid ||
        credentials.uid != unsafe { libc::getuid() } {
        return Err("bridge does not own the inherited Wayland connection".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn wayland_peer_is_pinned() {
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(authenticate_wayland(socket.as_raw_fd(), std::process::id()).is_ok());
        assert!(authenticate_wayland(socket.as_raw_fd(), std::process::id() + 1).is_err());
        assert!(authenticate_wayland(-1, std::process::id()).is_err());
    }
}
