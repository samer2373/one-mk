//! Telling the daemon when this computer changes networks, for automatic
//! layout profiles (`one-mk profile auto`).
//!
//! Event-driven everywhere, never polled, and only started when some
//! profile switches automatically, so it costs nothing otherwise.
//!
//! - Linux (and other Unix desktops with NetworkManager): NetworkManager's
//!   `PropertiesChanged` signals for `State` and `PrimaryConnection` on the
//!   system bus. After a change settles, the Wi-Fi network names come from
//!   NetworkManager, the router's MAC address from the kernel's route and
//!   neighbour tables (`/proc/net/route`, `/proc/net/arp`) and the
//!   addresses from the interfaces. Without NetworkManager (systemd-networkd,
//!   iwd alone, connman) nothing is watched and profiles only change with
//!   `one-mk profile use`.
//! - Windows and macOS: not done yet (TODO: `INetworkListManager` events or
//!   `NotifyIpInterfaceChange` on Windows; `SCDynamicStore` notifications
//!   for `State:/Network/Global/IPv4` and the AirPort SSID on macOS). The
//!   daemon says so once, and `one-mk profile use` works as everywhere.

use crate::profiles::NetworkId;

/// Calls back with what the networks look like now: once soon after
/// starting, then after every change.
pub type OnChange = Box<dyn Fn(NetworkId) + Send + Sync>;

/// A source of network-change notifications.
pub trait NetworkWatcher {
    /// Starts watching on the current tokio runtime; runs until the
    /// runtime ends.
    fn start(self: Box<Self>, on_change: OnChange);
}

/// Shown when this OS can't switch profiles by itself.
pub const UNSUPPORTED: &str = "Note: switching layout profiles by itself is not done on this system yet \
     (it needs NetworkManager on Linux); `one-mk profile use <name>` works.";

/// This OS's watcher.
pub fn system() -> Box<dyn NetworkWatcher> {
    #[cfg(test)]
    let w: Box<dyn NetworkWatcher> = Box::new(fake::Fake);
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    let w: Box<dyn NetworkWatcher> = Box::new(nm::NetworkManager);
    #[cfg(all(not(test), not(all(unix, not(target_os = "macos")))))]
    let w: Box<dyn NetworkWatcher> = Box::new(Unsupported);
    w
}

/// Tests' stand-in: every daemon that starts watching hears what
/// [`fake::announce`] says.
#[cfg(test)]
pub mod fake {
    use super::{NetworkWatcher, OnChange};
    use crate::profiles::NetworkId;
    use std::sync::Mutex;

    static WATCHERS: Mutex<Vec<OnChange>> = Mutex::new(Vec::new());

    pub(super) struct Fake;

    impl NetworkWatcher for Fake {
        fn start(self: Box<Self>, on_change: OnChange) {
            WATCHERS.lock().unwrap().push(on_change);
        }
    }

    /// Tells every watching daemon the network is now `net`.
    pub fn announce(net: &NetworkId) {
        for w in WATCHERS.lock().unwrap().iter() {
            w(net.clone());
        }
    }

    pub fn watchers() -> usize {
        WATCHERS.lock().unwrap().len()
    }
}

/// Windows and macOS for now: says so, and never calls back.
#[cfg_attr(any(test, all(unix, not(target_os = "macos"))), allow(dead_code))]
struct Unsupported;

impl NetworkWatcher for Unsupported {
    fn start(self: Box<Self>, _on_change: OnChange) {
        tracing::info!("{UNSUPPORTED}");
    }
}

/// This computer's addresses, loopback left out.
pub fn local_addrs() -> Vec<std::net::IpAddr> {
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter(|i| !i.is_loopback()).map(|i| i.ip()).collect()
}

/// The default routers in a `/proc/net/route` table (hex, little-endian
/// addresses), as IPv4 addresses.
pub fn default_gateways(route: &str) -> Vec<std::net::Ipv4Addr> {
    route
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (dest, gw) = (f.get(1)?, f.get(2)?);
            if *dest != "00000000" || *gw == "00000000" {
                return None;
            }
            let raw = u32::from_str_radix(gw, 16).ok()?;
            Some(std::net::Ipv4Addr::from(raw.to_le_bytes()))
        })
        .collect()
}

/// The MAC address of `ip` in a `/proc/net/arp` table, lowercase.
pub fn arp_mac(arp: &str, ip: std::net::Ipv4Addr) -> Option<String> {
    let ip = ip.to_string();
    arp.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let mac = f.get(3)?.to_ascii_lowercase();
        (f.first() == Some(&ip.as_str()) && mac != "00:00:00:00:00:00").then_some(mac)
    })
}

/// The MAC addresses of this computer's default routers (Linux; empty
/// elsewhere).
pub fn gateway_macs() -> Vec<String> {
    let (Ok(route), Ok(arp)) = (std::fs::read_to_string("/proc/net/route"), std::fs::read_to_string("/proc/net/arp")) else {
        return Vec::new();
    };
    let mut macs: Vec<String> = default_gateways(&route).into_iter().filter_map(|g| arp_mac(&arp, g)).collect();
    macs.dedup();
    macs
}

#[cfg(all(unix, not(target_os = "macos")))]
#[cfg_attr(test, allow(dead_code))]
mod nm {
    use super::{NetworkWatcher, OnChange};
    use crate::profiles::NetworkId;
    use futures_util::StreamExt;
    use std::time::Duration;
    use zbus::proxy::CacheProperties;
    use zbus::zvariant::OwnedObjectPath;

    const NM: &str = "org.freedesktop.NetworkManager";
    const NM_PATH: &str = "/org/freedesktop/NetworkManager";
    /// One switch of networks is a burst of changes, and addresses arrive
    /// a moment after the connection: wait for this much quiet first.
    const SETTLE: Duration = Duration::from_millis(1500);

    /// After NetworkManager's change notifications end (it restarted, the
    /// bus hiccuped), wait at least this before watching again, doubling up
    /// to `RETRY_MAX` while it keeps happening, so it can never spin.
    const RETRY_MIN: Duration = Duration::from_secs(1);
    const RETRY_MAX: Duration = Duration::from_secs(60);

    pub(super) struct NetworkManager;

    impl NetworkWatcher for NetworkManager {
        fn start(self: Box<Self>, on_change: OnChange) {
            tokio::spawn(async move {
                if let Err(e) = watch_forever(&on_change).await {
                    tracing::info!(
                        "the system bus is not available ({e}), so layout profiles don't switch by themselves; \
                         `one-mk profile use <name>` works"
                    );
                }
            });
        }
    }

    /// Watches NetworkManager for as long as the system bus is there. When
    /// it isn't running, or its notifications end, this sleeps (no timer)
    /// until it is on the bus again, then watches again.
    async fn watch_forever(on_change: &OnChange) -> zbus::Result<()> {
        let mut backoff = RETRY_MIN;
        let mut said = false;
        loop {
            let conn = zbus::Connection::system().await?;
            let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
            let mut owners = dbus.receive_name_owner_changed_with_args(&[(0, NM)]).await?;
            loop {
                let started = std::time::Instant::now();
                match watch(&conn, on_change).await {
                    Ok(()) => tracing::debug!("NetworkManager's change notifications ended; watching again"),
                    Err(e) if !said => tracing::info!(
                        "NetworkManager is not available ({e}), so layout profiles don't switch by themselves \
                         until it is; `one-mk profile use <name>` works"
                    ),
                    Err(e) => tracing::debug!("NetworkManager: {e}"),
                }
                said = true;
                if started.elapsed() > RETRY_MAX {
                    backoff = RETRY_MIN;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RETRY_MAX);
                // Until NetworkManager is on the bus (again).
                let mut bus_gone = false;
                while !dbus.name_has_owner(zbus::names::BusName::try_from(NM)?).await? {
                    if owners.next().await.is_none() {
                        bus_gone = true;
                        break;
                    }
                }
                if bus_gone {
                    break; // connect again
                }
            }
        }
    }

    async fn watch(conn: &zbus::Connection, on_change: &OnChange) -> zbus::Result<()> {
        let nm = zbus::Proxy::new(conn, NM, NM_PATH, NM).await?;
        // Fails at once if NetworkManager isn't running.
        let _: u32 = nm.get_property("State").await?;
        // Each stream starts with the current value: the first round reads
        // the network the daemon starts on.
        let mut state = nm.receive_property_changed::<u32>("State").await;
        let mut primary = nm.receive_property_changed::<OwnedObjectPath>("PrimaryConnection").await;
        loop {
            let changed = tokio::select! {
                c = state.next() => c.is_some(),
                c = primary.next() => c.is_some(),
            };
            if !changed {
                return Ok(());
            }
            loop {
                let more = tokio::time::timeout(SETTLE, async {
                    tokio::select! {
                        c = state.next() => c.is_some(),
                        c = primary.next() => c.is_some(),
                    }
                });
                match more.await {
                    Ok(true) => continue,
                    Ok(false) => return Ok(()),
                    Err(_) => break, // quiet: settled
                }
            }
            let ssids = ssids(conn).await.unwrap_or_else(|e| {
                tracing::debug!("reading Wi-Fi networks: {e}");
                Vec::new()
            });
            let net = NetworkId { ssids, gateways: super::gateway_macs(), addrs: super::local_addrs() };
            tracing::debug!(?net, "network changed");
            on_change(net);
        }
    }

    async fn proxy<'a>(conn: &zbus::Connection, path: OwnedObjectPath, interface: &'a str) -> zbus::Result<zbus::Proxy<'a>> {
        zbus::proxy::Builder::new(conn)
            .destination(NM)?
            .path(path)?
            .interface(interface)?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// The Wi-Fi networks of the active connections.
    async fn ssids(conn: &zbus::Connection) -> zbus::Result<Vec<String>> {
        let nm = proxy(conn, OwnedObjectPath::try_from(NM_PATH)?, NM).await?;
        let active: Vec<OwnedObjectPath> = nm.get_property("ActiveConnections").await?;
        let mut out = Vec::new();
        for path in active {
            let c = proxy(conn, path, "org.freedesktop.NetworkManager.Connection.Active").await?;
            let kind: String = c.get_property("Type").await?;
            if kind != "802-11-wireless" {
                continue;
            }
            let ap: OwnedObjectPath = c.get_property("SpecificObject").await?;
            if ap.as_str() == "/" {
                continue;
            }
            let ap = proxy(conn, ap, "org.freedesktop.NetworkManager.AccessPoint").await?;
            let ssid: Vec<u8> = ap.get_property("Ssid").await?;
            if !ssid.is_empty() {
                out.push(String::from_utf8_lossy(&ssid).into_owned());
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_default_router_and_its_mac() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     wlp2s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                     wlp2s0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(default_gateways(route), vec![std::net::Ipv4Addr::new(192, 168, 1, 1)]);
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n\
                   192.168.1.20     0x1         0x2         11:22:33:44:55:66     *        wlp2s0\n\
                   192.168.1.1      0x1         0x2         AA:BB:CC:00:11:22     *        wlp2s0\n";
        assert_eq!(arp_mac(arp, std::net::Ipv4Addr::new(192, 168, 1, 1)).as_deref(), Some("aa:bb:cc:00:11:22"));
        assert_eq!(arp_mac(arp, std::net::Ipv4Addr::new(10, 0, 0, 1)), None);
        assert!(default_gateways("header only\n").is_empty());
    }
}
