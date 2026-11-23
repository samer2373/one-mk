//! Telling the daemon when this computer changes networks, for automatic
//! layout profiles (`one-mk profile auto`).
//!
//! Event-driven everywhere, never polled, and only started when some
//! profile switches automatically, so it costs nothing otherwise.
//!
//! - Linux (and other Unix desktops with NetworkManager): NetworkManager's
//!   `PropertiesChanged` signals for `State`, `PrimaryConnection` and
//!   `ActiveConnections` on the system bus (a second link coming up, such as
//!   Wi-Fi next to Ethernet, changes only the last), and the kernel's
//!   address and route notifications (netlink). After a change settles, the
//!   Wi-Fi network names come from NetworkManager, the router's MAC address
//!   from the kernel's route and neighbour tables (`/proc/net/route`,
//!   `/proc/net/arp`, matched per interface) and the addresses from the
//!   interfaces. Without NetworkManager (systemd-networkd, iwd alone,
//!   connman) nothing is watched and profiles only change with `one-mk
//!   profile use`.
//! - Windows and macOS: WlanAPI and CoreWLAN notifications through ctl's
//!   cross-checkable Wi-Fi module. Location permission changes are events too.

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
    #[cfg(all(not(test), any(target_os = "macos", windows)))]
    let w: Box<dyn NetworkWatcher> = Box::new(NativeWifi);
    #[cfg(all(not(test), not(any(unix, windows))))]
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

/// Other platforms: says so, and never calls back.
#[cfg(not(any(unix, windows)))]
struct Unsupported;

#[cfg(not(any(unix, windows)))]
impl NetworkWatcher for Unsupported {
    fn start(self: Box<Self>, _on_change: OnChange) {
        tracing::info!("{UNSUPPORTED}");
    }
}

#[cfg(any(target_os = "macos", windows))]
#[cfg_attr(test, allow(dead_code))] // tests use the fake watcher
struct NativeWifi;

#[cfg(any(target_os = "macos", windows))]
impl NetworkWatcher for NativeWifi {
    fn start(self: Box<Self>, on_change: OnChange) {
        let watch = one_mk_ctl::wifi::Watch::start(move |state| {
            on_change(NetworkId { ssids: state.ssids, gateways: state.gateways, addrs: state.addrs });
        });
        std::mem::forget(watch);
    }
}

/// This computer's addresses, loopback left out.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))] // NetworkManager's
pub fn local_addrs() -> Vec<std::net::IpAddr> {
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter(|i| !i.is_loopback()).map(|i| i.ip()).collect()
}

/// The default routes in a `/proc/net/route` table (hex, little-endian
/// addresses): each one's interface and router, as an IPv4 address.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))] // NetworkManager's
pub fn default_gateways(route: &str) -> Vec<(String, std::net::Ipv4Addr)> {
    route
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (iface, dest, gw) = (f.first()?, f.get(1)?, f.get(2)?);
            if *dest != "00000000" || *gw == "00000000" {
                return None;
            }
            let raw = u32::from_str_radix(gw, 16).ok()?;
            Some(((*iface).to_owned(), std::net::Ipv4Addr::from(raw.to_le_bytes())))
        })
        .collect()
}

/// The MAC address of `ip` on interface `iface` in a `/proc/net/arp` table,
/// lowercase. Two interfaces can have routers at the same address (two
/// home networks, both 192.168.1.1): only the row of `iface` counts.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))] // NetworkManager's
pub fn arp_mac(arp: &str, ip: std::net::Ipv4Addr, iface: &str) -> Option<String> {
    let ip = ip.to_string();
    arp.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let mac = f.get(3)?.to_ascii_lowercase();
        (f.first() == Some(&ip.as_str()) && f.get(5) == Some(&iface) && mac != "00:00:00:00:00:00").then_some(mac)
    })
}

/// The MAC addresses of the default routers in these two tables, each once.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))] // NetworkManager's
fn routers_in(route: &str, arp: &str) -> Vec<String> {
    let mut macs: Vec<String> = Vec::new();
    for mac in default_gateways(route).into_iter().filter_map(|(iface, gateway)| arp_mac(arp, gateway, &iface)) {
        if !macs.contains(&mac) {
            macs.push(mac);
        }
    }
    macs
}

/// The MAC addresses of this computer's default routers (Linux; empty
/// elsewhere).
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))] // NetworkManager's
pub fn gateway_macs() -> Vec<String> {
    let (Ok(route), Ok(arp)) = (std::fs::read_to_string("/proc/net/route"), std::fs::read_to_string("/proc/net/arp")) else {
        return Vec::new();
    };
    routers_in(&route, &arp)
}

/// The kernel's notifications that an address or a route was added or
/// removed (a netlink socket, read without polling). NetworkManager's
/// signals say when a connection comes and goes, not when the address or
/// the router of one that stays connected changes (a lease that moves to
/// another subnet, an address added by hand). Linux only; elsewhere it never
/// reports anything.
#[cfg(target_os = "linux")]
#[cfg_attr(test, allow(dead_code))]
struct RouteEvents(tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>);

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "linux")))]
struct RouteEvents;

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "linux")))]
impl RouteEvents {
    fn open() -> std::io::Result<Self> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    async fn changed(&mut self) -> std::io::Result<()> {
        std::future::pending().await
    }
}

#[cfg(target_os = "linux")]
#[cfg_attr(test, allow(dead_code))]
impl RouteEvents {
    /// Subscribes to IPv4 and IPv6 address and route changes. Needs the
    /// runtime.
    fn open() -> std::io::Result<Self> {
        use std::os::fd::{FromRawFd, OwnedFd};
        // SAFETY: plain syscalls; the descriptor is owned right away.
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `fd` is a new, valid descriptor nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: an all-zero `sockaddr_nl` is valid; the fields are set below.
        let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        address.nl_groups = (libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV4_ROUTE | libc::RTMGRP_IPV6_IFADDR | libc::RTMGRP_IPV6_ROUTE) as u32;
        // SAFETY: `address` outlives the call and the length is its size.
        let bound = unsafe { libc::bind(std::os::fd::AsRawFd::as_raw_fd(&fd), (&raw const address).cast(), std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t) };
        if bound < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Self::from_fd(fd)
    }

    /// Reads from `fd` (a socket, already non-blocking).
    fn from_fd(fd: std::os::fd::OwnedFd) -> std::io::Result<Self> {
        // SAFETY: `fd` is a valid, open descriptor that stays open as long as the `AsyncFd` owns it.
        unsafe { tokio::io::unix::AsyncFd::register_with_interest(fd, tokio::io::Interest::READABLE) }.map(Self).map_err(|e| e.into_parts().1)
    }

    /// Waits until something was reported, and drains what is queued (a
    /// burst is one change).
    async fn changed(&mut self) -> std::io::Result<()> {
        let mut buffer = [0u8; 8192];
        let mut got = false;
        loop {
            let mut ready = self.0.readable().await?;
            loop {
                let read = ready.try_io(|fd| {
                    // SAFETY: reads into `buffer`, at most its length.
                    let n = unsafe { libc::recv(std::os::fd::AsRawFd::as_raw_fd(fd.get_ref()), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
                    if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n) }
                });
                match read {
                    Ok(Ok(_)) => got = true,
                    // The kernel's queue overflowed: certainly something changed.
                    Ok(Err(e)) if e.raw_os_error() == Some(libc::ENOBUFS) => got = true,
                    Ok(Err(e)) => return Err(e),
                    Err(_would_block) => break,
                }
            }
            if got {
                return Ok(());
            }
        }
    }
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

    struct Retry(Duration);

    impl Default for Retry {
        fn default() -> Self { Self(RETRY_MIN) }
    }

    impl Retry {
        fn failed(&mut self, stable: Duration) -> Duration {
            if stable >= RETRY_MAX { self.0 = RETRY_MIN; }
            let delay = self.0;
            self.0 = (delay * 2).min(RETRY_MAX);
            delay
        }
    }

    pub(super) struct NetworkManager;

    impl NetworkWatcher for NetworkManager {
        fn start(self: Box<Self>, on_change: OnChange) {
            tokio::spawn(async move {
                watch_forever(&on_change).await;
            });
        }
    }

    /// Reconnects after any bus failure. On a healthy bus without
    /// NetworkManager, waits for its owner notification without polling.
    async fn watch_forever(on_change: &OnChange) {
        watch_forever_using(on_change, zbus::Connection::system).await;
    }

    async fn watch_forever_using<F, Fut>(on_change: &OnChange, mut connect: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = zbus::Result<zbus::Connection>>,
    {
        let mut retry = Retry::default();
        let mut said = false;
        let mut connected_before = false;
        let mut connection_error = String::new();
        let mut identical_failures = 0;
        loop {
            let started = std::time::Instant::now();
            let result = match connect().await {
                Ok(conn) => {
                    connected_before = true;
                    identical_failures = 0;
                    watch_connection(&conn, on_change).await
                }
                Err(error) => {
                    let text = error.to_string();
                    identical_failures = if text == connection_error { identical_failures + 1 } else { 1 };
                    connection_error = text;
                    if !connected_before && identical_failures >= 5 {
                        tracing::info!("no system bus: network profiles will not switch automatically");
                        return std::future::pending().await;
                    }
                    Err(error)
                }
            };
            match result {
                Ok(()) => tracing::debug!("NetworkManager's change notifications ended; reconnecting"),
                Err(error) if !said => tracing::info!("NetworkManager or the system bus is not available ({error}); retrying; `one-mk profile use <name>` works"),
                Err(error) => tracing::debug!("NetworkManager: {error}; reconnecting"),
            }
            said = true;
            tokio::time::sleep(retry.failed(started.elapsed())).await;
        }
    }

    async fn watch_connection(conn: &zbus::Connection, on_change: &OnChange) -> zbus::Result<()> {
        let dbus = zbus::fdo::DBusProxy::new(conn).await?;
        let mut owners = dbus.receive_name_owner_changed_with_args(&[(0, NM)]).await?;
        while !dbus.name_has_owner(zbus::names::BusName::try_from(NM)?).await? {
            if owners.next().await.is_none() { return Ok(()); }
        }
        watch(conn, owners, on_change).await
    }

    /// What woke the watcher.
    #[derive(Debug, PartialEq, Eq)]
    enum Wake {
        /// Something about the networks changed.
        Change,
        /// NetworkManager or the bus went away or changed hands: subscribe
        /// again.
        Restart,
    }

    /// Everything that can say the networks changed.
    struct Streams<'a> {
        state: zbus::proxy::PropertyStream<'a, u32>,
        primary: zbus::proxy::PropertyStream<'a, OwnedObjectPath>,
        /// A second link coming up (Wi-Fi next to Ethernet) leaves `State`
        /// and `PrimaryConnection` as they were; this one changes.
        active: zbus::proxy::PropertyStream<'a, Vec<OwnedObjectPath>>,
        /// NetworkManager's owner on the bus, and the bus connection itself
        /// (its streams end with it). The property streams above never end
        /// when the bus dies: they wait on a cache nobody updates any more.
        owners: zbus::fdo::NameOwnerChangedStream,
        /// Addresses and routes changing on a link that stays connected.
        route: Option<super::RouteEvents>,
    }

    impl Streams<'_> {
        async fn next(&mut self) -> Wake {
            tokio::select! {
                c = self.state.next() => if c.is_some() { Wake::Change } else { Wake::Restart },
                c = self.primary.next() => if c.is_some() { Wake::Change } else { Wake::Restart },
                c = self.active.next() => if c.is_some() { Wake::Change } else { Wake::Restart },
                _ = self.owners.next() => Wake::Restart,
                changed = async {
                    match self.route.as_mut() {
                        Some(route) => route.changed().await,
                        None => std::future::pending().await,
                    }
                } => if changed.is_ok() { Wake::Change } else {
                    // Without it the rest still works.
                    self.route = None;
                    Wake::Change
                },
            }
        }
    }

    async fn watch(conn: &zbus::Connection, owners: zbus::fdo::NameOwnerChangedStream, on_change: &OnChange) -> zbus::Result<()> {
        let nm = zbus::Proxy::new(conn, NM, NM_PATH, NM).await?;
        // Fails at once if NetworkManager isn't running.
        let _: u32 = nm.get_property("State").await?;
        // Each stream starts with the current value: the first round reads
        // the network the daemon starts on.
        let mut streams = Streams {
            state: nm.receive_property_changed::<u32>("State").await,
            primary: nm.receive_property_changed::<OwnedObjectPath>("PrimaryConnection").await,
            active: nm.receive_property_changed::<Vec<OwnedObjectPath>>("ActiveConnections").await,
            owners,
            route: super::RouteEvents::open().map_err(|e| tracing::debug!("watching address changes: {e}")).ok(),
        };
        loop {
            if streams.next().await == Wake::Restart {
                return Ok(());
            }
            loop {
                match tokio::time::timeout(SETTLE, streams.next()).await {
                    Ok(Wake::Change) => continue,
                    Ok(Wake::Restart) => return Ok(()),
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test(start_paused = true)]
        async fn absent_system_bus_stops_retrying() {
            let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = attempts.clone();
            let task = tokio::spawn(async move {
                let callback: OnChange = Box::new(|_| {});
                watch_forever_using(&callback, move || {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Err(zbus::Error::Failure("no bus".into())) }
                }).await;
            });
            for _ in 0..20 {
                tokio::task::yield_now().await;
                tokio::time::advance(Duration::from_secs(60)).await;
            }
            assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 5);
            task.abort();
            let _ = task.await;
        }

        /// A NetworkManager stand-in on the bus: the three properties the
        /// watcher subscribes to.
        struct FakeNm {
            active: Vec<OwnedObjectPath>,
        }

        #[zbus::interface(name = "org.freedesktop.NetworkManager")]
        impl FakeNm {
            #[zbus(property)]
            fn state(&self) -> u32 { 70 }

            #[zbus(property)]
            fn primary_connection(&self) -> OwnedObjectPath { OwnedObjectPath::try_from("/").unwrap() }

            #[zbus(property)]
            fn active_connections(&self) -> Vec<OwnedObjectPath> { self.active.clone() }
        }

        async fn private_bus(address: &str) -> Option<tokio::process::Child> {
            use std::process::Stdio;
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut bus = match tokio::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1", &format!("--address={address}")])
                .stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn() {
                Ok(bus) => bus,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => panic!("starting private dbus-daemon: {error}"),
            };
            let mut ready = String::new();
            BufReader::new(bus.stdout.take().unwrap()).read_line(&mut ready).await.unwrap();
            assert!(!ready.is_empty());
            Some(bus)
        }

        async fn fake_nm(address: &str) -> zbus::Connection {
            zbus::connection::Builder::address(address).unwrap()
                .name(NM).unwrap()
                .serve_at(NM_PATH, FakeNm { active: Vec::new() }).unwrap()
                .build().await.unwrap()
        }

        /// A second link coming up changes only `ActiveConnections`, and the
        /// system bus restarting under a running NetworkManager ends the
        /// property streams of neither: both must still reach the watcher.
        #[tokio::test]
        async fn follows_active_connections_and_survives_a_bus_restart() {
            let path = std::env::temp_dir().join(format!("one-mk-nm-live-{}", std::process::id()));
            let address = format!("unix:path={}", path.display());
            let Some(mut bus) = private_bus(&address).await else {
                return eprintln!("skipped: dbus-daemon missing (private-bus integration requires dbus-daemon on PATH)");
            };
            let nm = fake_nm(&address).await;
            let (changed, mut reports) = tokio::sync::mpsc::unbounded_channel();
            let watch_address = address.clone();
            let task = tokio::spawn(async move {
                let callback: OnChange = Box::new(move |_| { let _ = changed.send(()); });
                watch_forever_using(&callback, move || {
                    let address = watch_address.clone();
                    async move { zbus::connection::Builder::address(address.as_str())?.build().await }
                }).await;
            });
            let wait = Duration::from_secs(8);
            tokio::time::timeout(wait, reports.recv()).await.expect("the starting network is reported").unwrap();
            // Only `ActiveConnections` changes.
            let iface = nm.object_server().interface::<_, FakeNm>(NM_PATH).await.unwrap();
            {
                let mut fake = iface.get_mut().await;
                fake.active = vec![OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/ActiveConnection/1").unwrap()];
                fake.active_connections_changed(iface.signal_emitter()).await.unwrap();
            }
            tokio::time::timeout(wait, reports.recv()).await.expect("a second connection is noticed").unwrap();
            // The bus goes away and comes back with NetworkManager on it.
            drop(iface);
            drop(nm);
            bus.kill().await.unwrap();
            bus.wait().await.unwrap();
            let mut bus = private_bus(&address).await.unwrap();
            let _nm = fake_nm(&address).await;
            tokio::time::timeout(Duration::from_secs(20), reports.recv()).await.expect("watching again after the bus restarted").unwrap();
            task.abort();
            let _ = task.await;
            bus.kill().await.unwrap();
            bus.wait().await.unwrap();
            let _ = std::fs::remove_file(path);
        }

        /// Address and route notifications wake the watcher: a burst of
        /// them is read at once and counts as one change.
        #[cfg(target_os = "linux")]
        #[tokio::test]
        async fn route_notifications_wake_the_watcher() {
            let (kernel, socket) = std::os::unix::net::UnixDatagram::pair().unwrap();
            socket.set_nonblocking(true).unwrap();
            let mut events = crate::netwatch::RouteEvents::from_fd(socket.into()).unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(50), events.changed()).await.is_err(), "quiet while nothing happens");
            for _ in 0..5 {
                kernel.send(&[0u8; 16]).unwrap();
            }
            tokio::time::timeout(Duration::from_secs(5), events.changed()).await.expect("woken by the notification").unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(100), events.changed()).await.is_err(), "the whole burst was drained");
            kernel.send(&[0u8; 16]).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.changed()).await.expect("and the next one wakes it again").unwrap();
        }

        /// The real subscription opens (when this system has netlink).
        #[cfg(target_os = "linux")]
        #[tokio::test]
        async fn the_kernel_subscription_opens() {
            if let Err(error) = crate::netwatch::RouteEvents::open() {
                eprintln!("skipped: no netlink socket here ({error})");
            }
        }

        #[test]
        fn reconnect_backoff_is_bounded_and_resets_after_stability() {
            let mut retry = Retry::default();
            for seconds in [1, 2, 4, 8, 16, 32, 60, 60] {
                assert_eq!(retry.failed(Duration::ZERO), Duration::from_secs(seconds));
            }
            assert_eq!(retry.failed(Duration::from_secs(60)), RETRY_MIN);
            assert_eq!(retry.failed(Duration::ZERO), Duration::from_secs(2));
        }

        #[tokio::test]
        async fn reconnects_when_a_private_bus_restarts() {
            use std::process::Stdio;
            use tokio::io::{AsyncBufReadExt, BufReader};

            let path = std::env::temp_dir().join(format!("one-mk-nm-bus-{}", std::process::id()));
            let address = format!("unix:path={}", path.display());
            let start = || tokio::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1", &format!("--address={address}")])
                .stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn();
            let mut bus = match start() {
                Ok(bus) => bus,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("skipped: dbus-daemon missing (private-bus restart integration requires dbus-daemon on PATH)");
                    return;
                }
                Err(error) => panic!("starting private dbus-daemon: {error}"),
            };
            let mut ready = String::new();
            BufReader::new(bus.stdout.take().unwrap()).read_line(&mut ready).await.unwrap();
            assert!(!ready.is_empty());
            let (sent, mut attempts) = tokio::sync::mpsc::unbounded_channel();
            let watch_address = address.clone();
            let task = tokio::spawn(async move {
                let callback: OnChange = Box::new(|_| {});
                watch_forever_using(&callback, move || {
                    let address = watch_address.clone();
                    let sent = sent.clone();
                    async move {
                        let result = zbus::connection::Builder::address(address.as_str())?.build().await;
                        let _ = sent.send(result.is_ok());
                        result
                    }
                }).await;
            });
            assert_eq!(attempts.recv().await, Some(true));
            bus.kill().await.unwrap();
            bus.wait().await.unwrap();
            assert_eq!(tokio::time::timeout(Duration::from_secs(5), attempts.recv()).await.unwrap(), Some(false));
            bus = start().unwrap();
            ready.clear();
            BufReader::new(bus.stdout.take().unwrap()).read_line(&mut ready).await.unwrap();
            assert!(!ready.is_empty());
            assert_eq!(tokio::time::timeout(Duration::from_secs(5), attempts.recv()).await.unwrap(), Some(true));
            task.abort();
            let _ = task.await;
            bus.kill().await.unwrap();
            bus.wait().await.unwrap();
            let _ = std::fs::remove_file(path);
        }
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
        assert_eq!(default_gateways(route), vec![("wlp2s0".to_owned(), std::net::Ipv4Addr::new(192, 168, 1, 1))]);
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n\
                   192.168.1.20     0x1         0x2         11:22:33:44:55:66     *        wlp2s0\n\
                   192.168.1.1      0x1         0x2         AA:BB:CC:00:11:22     *        wlp2s0\n";
        assert_eq!(arp_mac(arp, std::net::Ipv4Addr::new(192, 168, 1, 1), "wlp2s0").as_deref(), Some("aa:bb:cc:00:11:22"));
        assert_eq!(arp_mac(arp, std::net::Ipv4Addr::new(10, 0, 0, 1), "wlp2s0"), None);
        assert!(default_gateways("header only\n").is_empty());
    }

    /// Two networks whose routers share an address: each route gets the
    /// MAC of its own interface, not the first row for that address.
    #[test]
    fn routers_sharing_an_address_are_told_apart_by_interface() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                     wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n";
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n\
                   192.168.1.1      0x1         0x2         22:22:22:22:22:22     *        wlan0\n\
                   192.168.1.1      0x1         0x2         11:11:11:11:11:11     *        eth0\n";
        let gateway = std::net::Ipv4Addr::new(192, 168, 1, 1);
        assert_eq!(arp_mac(arp, gateway, "eth0").as_deref(), Some("11:11:11:11:11:11"));
        assert_eq!(arp_mac(arp, gateway, "wlan0").as_deref(), Some("22:22:22:22:22:22"));
        assert_eq!(arp_mac(arp, gateway, "enp3s0"), None, "no row for that interface");
        assert_eq!(routers_in(route, arp), ["11:11:11:11:11:11", "22:22:22:22:22:22"]);
        // A stale all-zero row on the interface does not count either.
        let stale = "IP address       HW type     Flags       HW address            Mask     Device\n\
                     192.168.1.1      0x1         0x0         00:00:00:00:00:00     *        eth0\n";
        assert_eq!(routers_in(route, stale), Vec::<String>::new());
    }
}
