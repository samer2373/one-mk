//! `one-mk input-devices detect` in the daemon (see `crate::detect`): the
//! backend reports this computer's own presses while a detection runs, and
//! the input this computer sends to another one is looked at too. Nothing
//! is held back or changed: the press still does what it does.

use super::App;
use crate::detect::{End, Session, ended};
use one_mk_input::{CaptureCmd, DeviceRef, Event};
use tokio::sync::oneshot;
use tokio::time::Instant;

impl App {
    /// `Request::Detect`: a detection begins, in place of any still running.
    pub(super) fn start_detect(&mut self, seconds: u32, reply: oneshot::Sender<String>) {
        if let Some(old) = self.detect.take() {
            old.finish(End::Replaced);
        }
        // Windows and macOS say which device a press came from on another path.
        let hints = cfg!(any(windows, target_os = "macos")) && self.backend_name == "native";
        let unseen = one_mk_input::detect_support(self.backend_name).err();
        tracing::debug!(seconds, "detecting the next key or button pressed");
        self.detect = Some(Session::new(reply, seconds, hints, unseen));
        let _ = self.capture.send(CaptureCmd::Detect(true));
    }

    /// The detection ends (with a press, a timeout, Esc ...); the backend
    /// stops reporting presses.
    pub(super) fn end_detect(&mut self, end: End) {
        let Some(session) = self.detect.take() else { return };
        let end = match end {
            End::Found { found, .. } => End::Found { found, device: self.device_identity(found.device) },
            other => other,
        };
        tracing::debug!(?end, "detection over");
        session.finish(end);
        let _ = self.capture.send(CaptureCmd::Detect(false));
    }

    /// A connected device's fingerprint and name.
    fn device_identity(&self, device: Option<DeviceRef>) -> Option<(String, String)> {
        let d = self.devices.list().iter().find(|d| Some(d.id) == device)?;
        Some((d.fingerprint.to_string(), crate::tuning::device_name(&d.fingerprint.to_string())))
    }

    /// Looks at an input event while detecting. True if it was only for
    /// detecting (`LocalPress`, `PressDevice`), which go nowhere else.
    pub(super) fn detect_input(&mut self, ev: &Event) -> bool {
        let away = self.remote.is_some();
        let Some(session) = self.detect.as_mut() else {
            return matches!(ev, Event::LocalPress { .. } | Event::PressDevice { .. });
        };
        let now = session.ms();
        let step = match *ev {
            Event::LocalPress { code, down, device } => session.detector.press(code, down, device, false, now),
            Event::PressDevice { code, down, device } => session.detector.device(code, down, device, now),
            // Input on its way to another computer (or held for a crossing): the keys and
            // buttons as the rules see them.
            Event::Key { code, down, device } => session.detector.press(code, down, device, away, now),
            Event::Button { button, down, device } => session.detector.press(button, down, device, away, now),
            _ => return false,
        };
        if let Some(end) = ended(step) {
            self.end_detect(end);
        }
        matches!(ev, Event::LocalPress { .. } | Event::PressDevice { .. })
    }

    /// When the detection must be looked at next.
    pub(super) fn detect_deadline(&self) -> Option<Instant> {
        self.detect.as_ref().map(Session::deadline)
    }

    pub(super) fn detect_timer(&mut self, now: Instant) {
        let Some(session) = self.detect.as_mut() else { return };
        if session.abandoned() {
            self.end_detect(End::Cancelled);
            return;
        }
        let ms = session.ms();
        if let Some(end) = ended(session.detector.tick(ms)) {
            self.end_detect(end);
        } else if now >= session.until {
            self.end_detect(End::Timeout);
        }
    }

    /// Whoever asked hung up (the command was stopped, the window cancelled).
    pub(super) fn detect_abandoned(&mut self) {
        tracing::debug!("detection cancelled by its caller");
        self.end_detect(End::Cancelled);
    }
}

/// Waits until a running detection's caller hangs up (never, without one).
pub(super) async fn hung_up(detect: &mut Option<Session>) {
    match detect {
        Some(session) => session.hung_up().await,
        None => std::future::pending().await,
    }
}
