//! Zero-configuration LAN discovery over mDNS / DNS-SD.

use crate::identity::{Fingerprint, Identity};
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;

const SERVICE: &str = "_one-mk._udp.local.";

/// A device seen on the network.
#[derive(Clone, Debug)]
pub struct Found {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub addrs: Vec<SocketAddr>,
    /// The device is waiting to pair (as opposed to its normal service).
    pub pairing: bool,
}

/// Advertises this device and reports others. Stops when dropped.
pub struct Discovery {
    daemon: ServiceDaemon,
}

impl Discovery {
    pub fn start(id: &Identity, port: u16, pairing: bool) -> Result<(Self, mpsc::UnboundedReceiver<Found>)> {
        let daemon = ServiceDaemon::new()?;
        let me = id.fingerprint.to_string();
        let host: String = id
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let info = ServiceInfo::new(
            SERVICE,
            &format!("{} ({}{})", id.name, id.fingerprint.short(), if pairing { " pairing" } else { "" }),
            &format!("{host}.local."),
            "",
            port,
            &[
                ("id", me.as_str()),
                ("name", id.name.as_str()),
                ("v", "1"),
                ("mode", if pairing { "pair" } else { "run" }),
            ][..],
        )?
        .enable_addr_auto();
        daemon.register(info)?;

        let events = daemon.browse(SERVICE)?;
        let (tx, rx) = mpsc::unbounded_channel();
        // mdns-sd is runtime-agnostic and blocks; keep it off the async runtime.
        std::thread::Builder::new().name("one-mk-mdns".into()).spawn(move || {
            while let Ok(event) = events.recv() {
                let ServiceEvent::ServiceResolved(info) = event else { continue };
                let Some(fingerprint) = info.get_property_val_str("id").and_then(|s| s.parse().ok()) else {
                    continue;
                };
                if info.get_property_val_str("id") == Some(me.as_str()) {
                    continue;
                }
                let mut addrs: Vec<SocketAddr> = info
                    .get_addresses()
                    .iter()
                    .map(|ip| ip.to_ip_addr())
                    .filter(usable)
                    .map(|ip| SocketAddr::new(ip, info.get_port()))
                    .collect();
                addrs.sort_by_key(|a| !a.is_ipv4()); // IPv4 first: most reliable on home LANs
                let name = info.get_property_val_str("name").unwrap_or("unknown").to_owned();
                let pairing = info.get_property_val_str("mode") == Some("pair");
                if tx.send(Found { fingerprint, name, addrs, pairing }).is_err() {
                    break;
                }
            }
        })?;
        Ok((Self { daemon }, rx))
    }
}

/// Link-local IPv6 needs a scope id we cannot carry in a plain `SocketAddr`.
fn usable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_unspecified(),
        IpAddr::V6(v6) => !v6.is_loopback() && (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}
