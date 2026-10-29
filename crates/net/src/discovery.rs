//! Zero-configuration LAN discovery over mDNS / DNS-SD.
//!
//! Anyone on the network can send mDNS answers, so everything discovery
//! reports is an untrusted hint: where a device with some fingerprint
//! *might* be. Who a connection really reaches is decided only by the TLS
//! handshake, which proves the peer holds the key for its fingerprint, and
//! by the trust store. Names from discovery are never shown to the user.

use crate::identity::{Fingerprint, Identity};
use anyhow::Result;
use mdns_sd::{IfKind, IfPredicate, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;

const SERVICE: &str = "_one-mk._udp.local.";
/// Addresses taken from one announcement, at most.
const MAX_ADDRS: usize = 8;

/// A device seen on the network. Unauthenticated: see the module docs.
#[derive(Clone, Debug)]
pub struct Found {
    /// Claimed fingerprint; only a connection's TLS handshake proves it.
    pub fingerprint: Fingerprint,
    /// Claimed name, cleaned with `sanitize_name`.
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
    pub async fn stop(self) {
        if let Ok(stopped) = self.daemon.shutdown() {
            let _ = tokio::time::timeout(std::time::Duration::from_millis(200), stopped.recv_async()).await;
        }
    }

    pub fn start(id: &Identity, port: u16, pairing: bool) -> Result<(Self, mpsc::UnboundedReceiver<Found>)> {
        let daemon = ServiceDaemon::new()?;
        skip_virtual_interfaces(&daemon)?;
        let me = id.fingerprint.to_string();
        let name = one_mk_proto::sanitize_name(&id.name);
        let info = ServiceInfo::new(
            SERVICE,
            &format!("{name} ({}{})", id.fingerprint.short(), if pairing { " pairing" } else { "" }),
            &host_name(&id.fingerprint),
            "",
            port,
            &[
                ("id", me.as_str()),
                ("name", name.as_str()),
                ("v", "1"),
                ("mode", if pairing { "pair" } else { "run" }),
            ][..],
        )?
        .enable_addr_auto();
        daemon.register(info)?;
        let rx = Self::spawn_browser(&daemon, me)?;
        Ok((Self { daemon }, rx))
    }

    /// Only looks for other devices, without advertising this one. For
    /// short-lived tools (like `one-mk send`) that share the identity of
    /// the long-running service on this machine and must not be mistaken
    /// for it.
    pub fn browse(id: &Identity) -> Result<(Self, mpsc::UnboundedReceiver<Found>)> {
        let daemon = ServiceDaemon::new()?;
        skip_virtual_interfaces(&daemon)?;
        let rx = Self::spawn_browser(&daemon, id.fingerprint.to_string())?;
        Ok((Self { daemon }, rx))
    }

    fn spawn_browser(daemon: &ServiceDaemon, me: String) -> Result<mpsc::UnboundedReceiver<Found>> {
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
                if info.get_port() == 0 {
                    continue;
                }
                addrs.sort_by_key(|a| !a.is_ipv4()); // IPv4 first: most reliable on home LANs
                addrs.truncate(MAX_ADDRS);
                let name = one_mk_proto::sanitize_name(info.get_property_val_str("name").unwrap_or(""));
                let pairing = info.get_property_val_str("mode") == Some("pair");
                if tx.send(Found { fingerprint, name, addrs, pairing }).is_err() {
                    break;
                }
            }
        })?;
        Ok(rx)
    }
}

/// `one-mk-<short fingerprint>.local.`: unique per device, unlike the
/// machine's host name (two "MacBook-Pro"s) and always valid ASCII.
fn host_name(fp: &Fingerprint) -> String {
    format!("one-mk-{}.local.", fp.short())
}

/// Interfaces that only lead to containers, VMs or this machine itself.
/// Advertising their addresses made peers dial e.g. Docker's 172.17.0.1,
/// which on their side is their own Docker bridge.
pub fn is_virtual(name: &str) -> bool {
    const PREFIXES: [&str; 15] = [
        "docker", "br-", "veth", "virbr", "vnet", "vmnet", "vboxnet", "lxcbr", "lxdbr", "incusbr", "podman",
        "cni", "flannel", "cali", "vEthernet",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

fn skip_virtual_interfaces(daemon: &ServiceDaemon) -> Result<()> {
    daemon.disable_interface(IfKind::Predicate(IfPredicate::new(|intf| is_virtual(&intf.name))))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_name_is_unique_ascii() {
        let a = host_name(&Fingerprint([0xab; 32]));
        assert_eq!(a, "one-mk-abab-abab.local.");
        assert_ne!(a, host_name(&Fingerprint([0xcd; 32])));
    }

    #[test]
    fn virtual_interfaces() {
        for v in ["docker0", "br-1a2b3c", "veth12ab", "virbr0", "vEthernet (WSL)", "vboxnet0"] {
            assert!(is_virtual(v), "{v}");
        }
        for real in ["eth0", "enp3s0", "wlan0", "wlp2s0", "en0", "Wi-Fi", "Ethernet"] {
            assert!(!is_virtual(real), "{real}");
        }
    }
}
