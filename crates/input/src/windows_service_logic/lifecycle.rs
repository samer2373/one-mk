use super::{HelperKind, LogicError};
use std::time::Duration;

pub const DESKTOP_DEBOUNCE: Duration = Duration::from_millis(150);
pub const HELPER_START_TIMEOUT: Duration = Duration::from_secs(5);
pub const STABLE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopKind { Unknown, Default, Winlogon, Screensaver }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub session: Option<u32>,
    pub logged_on: bool,
    pub locked: bool,
    pub console_connected: bool,
    pub desktop: DesktopKind,
}

pub trait SessionSource {
    fn snapshot(&mut self) -> Result<SessionSnapshot, LogicError>;
}

pub trait HelperControl {
    fn start(&mut self, request: StartRequest) -> Result<(), LogicError>;
    fn stop_and_release(&mut self, generation: u64) -> Result<(), LogicError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartRequest {
    pub session: u32,
    pub kind: HelperKind,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Snapshot(SessionSnapshot),
    SourceFailed,
    Logon(u32),
    Logoff(u32),
    Lock(u32),
    Unlock(u32),
    ConsoleConnect(u32),
    ConsoleDisconnect(u32),
    DesktopChanged { session: u32, desktop: DesktopKind },
    HelperStarted { generation: u64 },
    HelperExited { generation: u64 },
    Timer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Start(StartRequest),
    StopAndRelease { generation: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Update {
    pub decision: Option<Decision>,
    pub wake_at: Option<Duration>,
}

#[derive(Clone, Copy)]
enum HelperState {
    Absent,
    Starting { request: StartRequest, deadline: Duration },
    Running { request: StartRequest, since: Duration },
    Stopping { generation: u64 },
}

/// Event-driven policy for one provisioned user session. No polling or I/O.
pub struct SessionMachine {
    session: u32,
    facts: SessionSnapshot,
    desired: HelperKind,
    changed_at: Duration,
    retry_at: Duration,
    failures: u8,
    generation: u64,
    helper: HelperState,
    last_time: Duration,
    closed: bool,
}

impl SessionMachine {
    pub fn new(session: u32, now: Duration) -> Self {
        Self {
            session,
            facts: SessionSnapshot { session: None, logged_on: false, locked: true,
                console_connected: false, desktop: DesktopKind::Unknown },
            desired: HelperKind::None, changed_at: now, retry_at: now, failures: 0,
            generation: 0, helper: HelperState::Absent, last_time: now, closed: false,
        }
    }

    pub fn desired(&self) -> HelperKind { self.desired }

    fn choose(&self) -> HelperKind {
        if self.closed || self.facts.session != Some(self.session) || !self.facts.logged_on || !self.facts.console_connected {
            return HelperKind::None;
        }
        match self.facts.desktop {
            DesktopKind::Default if !self.facts.locked => HelperKind::UserDesktop,
            DesktopKind::Winlogon => HelperKind::SecureDesktop,
            _ => HelperKind::None,
        }
    }

    pub fn step(&mut self, now: Duration, event: SessionEvent) -> Update {
        if now < self.last_time { self.closed = true; }
        self.last_time = now;
        if let HelperState::Running { since, .. } = self.helper
            && now.saturating_sub(since) >= STABLE_INTERVAL { self.failures = 0; }
        match event {
            SessionEvent::Snapshot(facts) => self.facts = facts,
            SessionEvent::SourceFailed => {
                self.facts.session = None;
                self.facts.desktop = DesktopKind::Unknown;
            }
            SessionEvent::Logon(session) if session == self.session => {
                self.facts.session = Some(session);
                self.facts.logged_on = true;
                self.facts.locked = true;
                self.facts.desktop = DesktopKind::Unknown;
            }
            SessionEvent::Logoff(session) if session == self.session => {
                self.facts.logged_on = false;
                self.facts.console_connected = false;
                self.facts.desktop = DesktopKind::Unknown;
            }
            SessionEvent::Lock(session) if session == self.session => self.facts.locked = true,
            SessionEvent::Unlock(session) if session == self.session => self.facts.locked = false,
            SessionEvent::ConsoleConnect(session) => {
                self.facts.session = Some(session);
                self.facts.console_connected = session == self.session;
                self.facts.desktop = DesktopKind::Unknown;
            }
            SessionEvent::ConsoleDisconnect(session) if session == self.session => {
                self.facts.console_connected = false;
                self.facts.desktop = DesktopKind::Unknown;
            }
            SessionEvent::DesktopChanged { session, desktop } if session == self.session => self.facts.desktop = desktop,
            SessionEvent::HelperStarted { generation } => {
                if let HelperState::Starting { request, deadline } = self.helper
                    && request.generation == generation && now < deadline {
                    self.helper = HelperState::Running { request, since: now };
                }
            }
            SessionEvent::HelperExited { generation } => {
                let unexpected = match self.helper {
                    HelperState::Starting { request, .. } | HelperState::Running { request, .. }
                        if request.generation == generation => Some(true),
                    HelperState::Stopping { generation: current } if current == generation => Some(false),
                    _ => None,
                };
                if let Some(unexpected) = unexpected {
                    self.helper = HelperState::Absent;
                    if unexpected {
                        let seconds = (1u64 << self.failures.min(5)).min(30);
                        self.failures = self.failures.saturating_add(1);
                        self.retry_at = now.saturating_add(Duration::from_secs(seconds));
                    }
                }
            }
            _ => {}
        }
        let desired = self.choose();
        if desired != self.desired { self.desired = desired; self.changed_at = now; }
        match self.helper {
            HelperState::Starting { request, deadline }
                if request.kind != desired || now >= deadline => {
                if now >= deadline {
                    let seconds = (1u64 << self.failures.min(5)).min(30);
                    self.failures = self.failures.saturating_add(1);
                    self.retry_at = now.saturating_add(Duration::from_secs(seconds));
                }
                self.helper = HelperState::Stopping { generation: request.generation };
                Update { decision: Some(Decision::StopAndRelease { generation: request.generation }), wake_at: None }
            }
            HelperState::Running { request, .. } if request.kind != desired => {
                self.helper = HelperState::Stopping { generation: request.generation };
                Update { decision: Some(Decision::StopAndRelease { generation: request.generation }), wake_at: None }
            }
            HelperState::Absent if desired != HelperKind::None => {
                let ready = self.changed_at.saturating_add(DESKTOP_DEBOUNCE).max(self.retry_at);
                if now < ready { return Update { decision: None, wake_at: Some(ready) }; }
                let Some(generation) = self.generation.checked_add(1) else {
                    self.closed = true;
                    self.desired = HelperKind::None;
                    return Update { decision: None, wake_at: None };
                };
                self.generation = generation;
                let request = StartRequest { session: self.session, kind: desired, generation };
                let deadline = now.saturating_add(HELPER_START_TIMEOUT);
                self.helper = HelperState::Starting { request, deadline };
                Update { decision: Some(Decision::Start(request)), wake_at: Some(deadline) }
            }
            HelperState::Starting { deadline, .. } => Update { decision: None, wake_at: Some(deadline) },
            _ => Update { decision: None, wake_at: None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> SessionSnapshot {
        SessionSnapshot { session: Some(10), logged_on: true, locked: false,
            console_connected: true, desktop: DesktopKind::Default }
    }
    fn start(machine: &mut SessionMachine, now: Duration) -> StartRequest {
        let Some(Decision::Start(request)) = machine.step(now, SessionEvent::Timer).decision else { panic!("expected start"); };
        request
    }
    fn running() -> (SessionMachine, StartRequest, Duration) {
        let mut machine = SessionMachine::new(10, Duration::ZERO);
        let update = machine.step(Duration::ZERO, SessionEvent::Snapshot(facts()));
        assert_eq!(update.wake_at, Some(DESKTOP_DEBOUNCE));
        let request = start(&mut machine, DESKTOP_DEBOUNCE);
        machine.step(DESKTOP_DEBOUNCE, SessionEvent::HelperStarted { generation: request.generation });
        (machine, request, DESKTOP_DEBOUNCE)
    }

    #[test]
    fn unknown_signin_disconnected_and_other_user_fail_closed() {
        let mut machine = SessionMachine::new(10, Duration::ZERO);
        assert_eq!(machine.step(Duration::ZERO, SessionEvent::Timer), Update { decision: None, wake_at: None });
        for changed in [SessionSnapshot { session: None, ..facts() },
            SessionSnapshot { logged_on: false, desktop: DesktopKind::Winlogon, ..facts() },
            SessionSnapshot { session: Some(11), ..facts() },
            SessionSnapshot { console_connected: false, ..facts() },
            SessionSnapshot { desktop: DesktopKind::Unknown, ..facts() },
            SessionSnapshot { desktop: DesktopKind::Screensaver, ..facts() },
            SessionSnapshot { locked: true, ..facts() }] {
            assert_eq!(machine.step(Duration::ZERO, SessionEvent::Snapshot(changed)).decision, None);
            assert_eq!(machine.desired(), HelperKind::None);
        }
    }

    #[test]
    fn uac_switch_stops_immediately_but_starts_only_after_exit_and_debounce() {
        let (mut machine, request, now) = running();
        assert_eq!(machine.step(now, SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Winlogon }).decision,
            Some(Decision::StopAndRelease { generation: request.generation }));
        assert_eq!(machine.desired(), HelperKind::SecureDesktop);
        assert_eq!(machine.step(now + DESKTOP_DEBOUNCE, SessionEvent::Timer).decision, None);
        let update = machine.step(now + DESKTOP_DEBOUNCE, SessionEvent::HelperExited { generation: request.generation });
        let Some(Decision::Start(secure)) = update.decision else { panic!("secure helper"); };
        assert_eq!(secure.kind, HelperKind::SecureDesktop);
        assert!(secure.generation > request.generation);
        machine.step(now + DESKTOP_DEBOUNCE, SessionEvent::HelperStarted { generation: secure.generation });
        assert!(machine.step(now + DESKTOP_DEBOUNCE, SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Default }).decision.is_some());
    }

    #[test]
    fn rapid_desktop_changes_coalesce() {
        let (mut machine, request, now) = running();
        machine.step(now, SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Winlogon });
        machine.step(now, SessionEvent::HelperExited { generation: request.generation });
        for millis in 1..100 {
            let desktop = if millis % 2 == 0 { DesktopKind::Default } else { DesktopKind::Winlogon };
            assert_eq!(machine.step(now + Duration::from_millis(millis), SessionEvent::DesktopChanged { session: 10, desktop }).decision, None);
        }
        assert_eq!(start(&mut machine, now + Duration::from_millis(99) + DESKTOP_DEBOUNCE).kind, HelperKind::SecureDesktop);
    }

    #[test]
    fn crashes_back_off_cap_and_stable_minute_resets() {
        let (mut machine, mut request, mut now) = running();
        for seconds in [1, 2, 4, 8, 16, 30, 30] {
            let update = machine.step(now, SessionEvent::HelperExited { generation: request.generation });
            assert_eq!(update.wake_at, Some(now + Duration::from_secs(seconds)));
            assert_eq!(machine.step(now, SessionEvent::HelperExited { generation: request.generation }).decision, None);
            assert_eq!(machine.step(now + Duration::from_secs(seconds) - Duration::from_nanos(1), SessionEvent::Timer).decision, None);
            now += Duration::from_secs(seconds);
            request = start(&mut machine, now);
            machine.step(now, SessionEvent::HelperStarted { generation: request.generation });
        }
        now += STABLE_INTERVAL;
        assert_eq!(machine.step(now, SessionEvent::HelperExited { generation: request.generation }).wake_at, Some(now + Duration::from_secs(1)));
    }

    #[test]
    fn lock_disconnect_logoff_and_source_failure_release_once() {
        for event in [SessionEvent::Lock(10), SessionEvent::ConsoleDisconnect(10), SessionEvent::Logoff(10), SessionEvent::SourceFailed,
            SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Screensaver }] {
            let (mut machine, request, now) = running();
            assert_eq!(machine.step(now, event).decision, Some(Decision::StopAndRelease { generation: request.generation }));
            assert_eq!(machine.step(now, event).decision, None);
            assert_eq!(machine.step(now, SessionEvent::HelperExited { generation: request.generation }).wake_at, None);
        }
    }

    #[test]
    fn session_events_recover_only_with_known_facts() {
        let mut machine = SessionMachine::new(10, Duration::ZERO);
        for event in [SessionEvent::Logon(10), SessionEvent::ConsoleConnect(10), SessionEvent::Unlock(10)] {
            assert_eq!(machine.step(Duration::ZERO, event).decision, None);
            assert_eq!(machine.desired(), HelperKind::None);
        }
        machine.step(Duration::ZERO, SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Default });
        let request = start(&mut machine, DESKTOP_DEBOUNCE);
        machine.step(DESKTOP_DEBOUNCE, SessionEvent::HelperStarted { generation: request.generation });
        machine.step(DESKTOP_DEBOUNCE, SessionEvent::Lock(10));
        machine.step(DESKTOP_DEBOUNCE, SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Winlogon });
        machine.step(DESKTOP_DEBOUNCE, SessionEvent::HelperExited { generation: request.generation });
        assert_eq!(start(&mut machine, DESKTOP_DEBOUNCE * 2).kind, HelperKind::SecureDesktop);
    }

    #[test]
    fn duplicate_stale_and_foreign_events_do_not_restart() {
        let (mut machine, request, now) = running();
        for event in [SessionEvent::Logoff(11), SessionEvent::Logon(11), SessionEvent::Lock(11), SessionEvent::Unlock(11),
            SessionEvent::ConsoleDisconnect(11),
            SessionEvent::DesktopChanged { session: 11, desktop: DesktopKind::Winlogon },
            SessionEvent::HelperStarted { generation: request.generation + 1 },
            SessionEvent::HelperExited { generation: request.generation + 1 }, SessionEvent::Snapshot(facts())] {
            assert_eq!(machine.step(now, event), Update { decision: None, wake_at: None });
        }
    }

    #[test]
    fn startup_timeout_waits_for_exit_and_never_spins() {
        let mut machine = SessionMachine::new(10, Duration::ZERO);
        machine.step(Duration::ZERO, SessionEvent::Snapshot(facts()));
        let request = start(&mut machine, DESKTOP_DEBOUNCE);
        let deadline = DESKTOP_DEBOUNCE + HELPER_START_TIMEOUT;
        assert_eq!(machine.step(deadline, SessionEvent::HelperStarted { generation: request.generation }).decision,
            Some(Decision::StopAndRelease { generation: request.generation }));
        assert_eq!(machine.step(deadline, SessionEvent::Timer).wake_at, None);
        assert_eq!(machine.step(deadline, SessionEvent::HelperExited { generation: request.generation }).wake_at, Some(deadline + Duration::from_secs(1)));
    }

    #[test]
    fn another_console_session_revokes_the_current_helper() {
        let (mut machine, request, now) = running();
        assert_eq!(machine.step(now, SessionEvent::ConsoleConnect(11)).decision,
            Some(Decision::StopAndRelease { generation: request.generation }));
        machine.step(now, SessionEvent::HelperExited { generation: request.generation });
        assert_eq!(machine.desired(), HelperKind::None);
        assert_eq!(machine.step(now, SessionEvent::ConsoleConnect(10)).decision, None);
        assert_eq!(machine.desired(), HelperKind::None);
        machine.step(now, SessionEvent::DesktopChanged { session: 10, desktop: DesktopKind::Default });
        assert_eq!(start(&mut machine, now + DESKTOP_DEBOUNCE).kind, HelperKind::UserDesktop);
    }

    #[test]
    fn clock_rollback_is_permanent_and_idle_has_no_timer() {
        let (mut machine, request, _) = running();
        assert_eq!(machine.step(Duration::ZERO, SessionEvent::Timer).decision, Some(Decision::StopAndRelease { generation: request.generation }));
        machine.step(Duration::ZERO, SessionEvent::HelperExited { generation: request.generation });
        assert_eq!(machine.step(Duration::from_secs(100), SessionEvent::Snapshot(facts())), Update { decision: None, wake_at: None });
        let (mut idle, _, now) = running();
        assert_eq!(idle.step(now + STABLE_INTERVAL, SessionEvent::Timer).wake_at, None);
    }
}
