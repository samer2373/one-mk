//! Event-driven Wi-Fi observations shared by the daemon and its window.

use std::sync::{Arc, Mutex};

#[cfg(all(unix, not(target_os = "macos")))]
#[path = "wifi/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "wifi/macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "wifi/windows.rs"]
mod platform;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permission {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Restricted,
    Allowed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub ssids: Vec<String>,
    pub gateways: Vec<String>,
    pub addrs: Vec<std::net::IpAddr>,
    pub permission: Permission,
    pub unavailable: bool,
}

impl State {
    pub fn explanation(&self) -> Option<&'static str> {
        match self.permission {
            Permission::NotDetermined | Permission::Denied | Permission::Restricted => Some(
                "Allow oneMK to read Wi-Fi names in Location Services to switch layouts automatically. Your location is not tracked.",
            ),
            _ if self.unavailable => Some("Wi-Fi names are unavailable right now. You can still choose a profile yourself."),
            _ => None,
        }
    }

    pub fn permission_button(&self) -> Option<&'static str> {
        match self.permission {
            Permission::NotDetermined => Some("Allow Wi-Fi access"),
            Permission::Denied | Permission::Restricted => Some("Location settings"),
            _ => None,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn apple_permission(status: i32) -> Permission {
    match status {
        0 => Permission::NotDetermined,
        1 => Permission::Restricted,
        2 => Permission::Denied,
        3 | 4 => Permission::Allowed,
        _ => Permission::Unknown,
    }
}

#[cfg(any(windows, test))]
fn windows_permission(error: u32) -> Permission {
    match error {
        0 => Permission::Allowed,
        5 => Permission::Denied,
        _ => Permission::Unknown,
    }
}

#[cfg(any(target_os = "macos", test))]
fn neighbor_mac(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|word| {
        let fields: Vec<_> = word.split(':').collect();
        if fields.len() != 6 || fields.iter().any(|field| field.is_empty() || field.len() > 2) { return None; }
        let bytes = fields.iter().map(|field| u8::from_str_radix(field, 16).ok()).collect::<Option<Vec<_>>>()?;
        bytes.iter().any(|byte| *byte != 0).then(|| bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(":"))
    })
}

struct Shared {
    state: Mutex<State>,
    changed: Box<dyn Fn(State) + Send + Sync>,
}

impl Shared {
    #[cfg(any(windows, target_os = "macos"))]
    fn publish_network(&self, mut state: State) {
        state.addrs = if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter(|interface| !interface.is_loopback()).map(|interface| interface.ip()).collect();
        state.gateways = platform::gateways();
        self.publish(state);
    }

    fn publish(&self, mut state: State) {
        state.addrs.sort();
        state.addrs.dedup();
        state.gateways.sort();
        state.gateways.dedup();
        state.ssids.sort();
        state.ssids.dedup();
        if matches!(state.permission, Permission::Denied | Permission::Restricted | Permission::NotDetermined) {
            state.ssids.clear();
        }
        let mut current = self.state.lock().unwrap();
        if *current == state {
            return;
        }
        *current = state.clone();
        drop(current);
        (self.changed)(state);
    }
}

#[derive(Clone, Copy)]
enum Action { Permission, Refresh, Stop }

pub struct Watch {
    shared: Arc<Shared>,
    action: Box<dyn Fn(Action) + Send + Sync>,
}

impl Watch {
    pub fn start(changed: impl Fn(State) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), changed: Box::new(changed) });
        let action = platform::start(shared.clone());
        Self { shared, action }
    }

    pub fn state(&self) -> State { self.shared.state.lock().unwrap().clone() }

    pub fn request_permission(&self) { (self.action)(Action::Permission); }

    pub fn refresh(&self) { (self.action)(Action::Refresh); }
}

impl Drop for Watch {
    fn drop(&mut self) { (self.action)(Action::Stop); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_states_offer_the_right_recovery() {
        assert_eq!(apple_permission(0), Permission::NotDetermined);
        assert_eq!(apple_permission(1), Permission::Restricted);
        assert_eq!(apple_permission(2), Permission::Denied);
        for status in [3, 4] { assert_eq!(apple_permission(status), Permission::Allowed); }
        assert_eq!(apple_permission(99), Permission::Unknown);
        assert_eq!(windows_permission(5), Permission::Denied);
        assert_eq!(windows_permission(0), Permission::Allowed);
        assert_eq!(windows_permission(1062), Permission::Unknown);
        for permission in [Permission::NotDetermined, Permission::Denied, Permission::Restricted] {
            let state = State { permission, ..State::default() };
            assert!(state.explanation().is_some());
            assert!(state.permission_button().is_some());
        }
        assert!(State { permission: Permission::Allowed, ..State::default() }.permission_button().is_none());
    }

    #[test]
    fn router_neighbors_require_a_real_mac_and_normalize_apple_output() {
        assert_eq!(neighbor_mac("? (192.168.1.1) at a:b:c:d:e:f on en0"), Some("0a:0b:0c:0d:0e:0f".into()));
        assert_eq!(neighbor_mac("fe80::1%en0 00:11:22:33:44:55 en0"), Some("00:11:22:33:44:55".into()));
        for output in ["? (192.168.1.1) at (incomplete)", "00:00:00:00:00:00", "fe80:12:34:56:78:90"] { assert_eq!(neighbor_mac(output), None); }
    }

    #[test]
    fn native_network_and_permission_watchers_have_no_polling_loop() {
        let macos = include_str!("wifi/macos_network.rs");
        assert!(macos.contains("SCDynamicStoreSetNotificationKeys"));
        assert!(macos.contains("State:/Network/Global/IPv4"));
        let windows = include_str!("wifi/windows.rs");
        for api in ["NotifyIpInterfaceChange", "NotifyRouteChange2", "NotifyUnicastIpAddressChange", "CancelMibChangeNotify2"] { assert!(windows.contains(api)); }
        assert!(windows.contains(r#"HSTRING::from("location")"#));
        for watcher in [macos, windows] {
            for polling in ["thread::sleep", "recv_timeout", "tokio::time::interval"] { assert!(!watcher.contains(polling)); }
        }
    }

    #[test]
    fn addresses_and_routes_republish_without_an_ssid_change() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        let shared = Shared { state: Mutex::new(State::default()), changed: Box::new(move |state| output.lock().unwrap().push(state)) };
        let mut state = State { ssids: vec!["LAN".into()], permission: Permission::Allowed, ..State::default() };
        shared.publish(state.clone());
        state.addrs = vec!["192.168.3.10".parse().unwrap(), "192.168.3.10".parse().unwrap()];
        shared.publish(state.clone());
        state.gateways = vec!["00:11:22:33:44:55".into(), "00:11:22:33:44:55".into()];
        shared.publish(state.clone());
        shared.publish(state.clone());
        state.addrs.clear();
        shared.publish(state);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[1].addrs.len(), 1);
        assert_eq!(events[2].gateways, ["00:11:22:33:44:55"]);
        assert!(events[3].addrs.is_empty());
    }

    #[test]
    fn revocation_clears_names_and_duplicate_events_are_silent() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        let shared = Shared { state: Mutex::new(State::default()), changed: Box::new(move |state| output.lock().unwrap().push(state)) };
        let connected = State { ssids: vec!["Café".into(), "Café".into()], permission: Permission::Allowed, unavailable: false, ..State::default() };
        shared.publish(connected.clone());
        shared.publish(connected.clone());
        shared.publish(State { permission: Permission::Denied, ..connected.clone() });
        shared.publish(connected);
        shared.publish(State { permission: Permission::Allowed, ..State::default() });
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].ssids, ["Café"]);
        assert!(events[1].ssids.is_empty());
        assert_eq!(events[2].ssids, ["Café"]);
        assert!(events[3].ssids.is_empty());
    }
}
