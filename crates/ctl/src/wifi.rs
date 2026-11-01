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

struct Shared {
    state: Mutex<State>,
    changed: Box<dyn Fn(State) + Send + Sync>,
}

impl Shared {
    fn publish(&self, mut state: State) {
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
    fn revocation_clears_names_and_duplicate_events_are_silent() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        let shared = Shared { state: Mutex::new(State::default()), changed: Box::new(move |state| output.lock().unwrap().push(state)) };
        let connected = State { ssids: vec!["Café".into(), "Café".into()], permission: Permission::Allowed, unavailable: false };
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
