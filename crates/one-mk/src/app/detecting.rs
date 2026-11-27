//! `one-mk input-devices detect` in the daemon (see `crate::detect`): the
//! backend reports this computer's own presses while a detection runs (here
//! or captured), tagged with the detection's session, so a late report of
//! an earlier detection never counts for the next. Backends that can't see
//! this computer's own input (the Wayland portals) are read through the
//! input this computer sends to another one. Nothing is held back or
//! changed: the press still does what it does.

use super::App;
use crate::detect::{End, Session, ended};
use one_mk_input::{CaptureCmd, DeviceRef, Event};
use one_mk_proto::detect::{Press, Step};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::oneshot;
use tokio::time::Instant;

/// The last detection session handed out.
static SESSIONS: AtomicU32 = AtomicU32::new(0);

impl App {
    /// `Request::Detect`: a detection begins, in place of any still running.
    /// A request whose asker already hung up (it waited in the queue behind
    /// other traffic) is dropped, and replaces nothing.
    pub(super) fn start_detect(&mut self, seconds: u32, keys_only: bool, reply: oneshot::Sender<String>) {
        if reply.is_closed() {
            tracing::debug!("a detection whose asker had already gone was dropped");
            return;
        }
        if let Some(old) = self.detect.take() {
            old.finish(End::Replaced);
        }
        let id = SESSIONS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        // Windows and macOS say which device a press came from on another path.
        let hints = cfg!(any(windows, target_os = "macos")) && self.backend_name == "native";
        let support = one_mk_input::detect_support(self.backend_name);
        tracing::debug!(seconds, id, "detecting the next key or button pressed");
        let mut session = Session::new(id, reply, seconds, keys_only, hints, support.is_ok(), support.err());
        if !session.local_presses
            && let Some(r) = &self.remote
        {
            // Read from what goes to the peer: what is held there now is no press.
            let held: Vec<u32> = r.held.iter().collect();
            let now = session.ms();
            session.detector.seed(&held, now);
        }
        self.detect = Some(session);
        let _ = self.capture.send(CaptureCmd::Detect(Some(id)));
    }

    /// The detection ends (with a press, a timeout, Esc ...); the backend
    /// stops reporting presses.
    pub(super) fn end_detect(&mut self, end: End) {
        let Some(session) = self.detect.take() else { return };
        let end = match end {
            End::Found { found, .. } => End::Found {
                found,
                device: self.device_identity(found.device),
                unusable: found.unusable().or_else(|| {
                    one_mk_input::detect_unusable(self.backend_name, found.code, found.mods, self.settings.input.local_rules)
                }),
            },
            other => other,
        };
        tracing::debug!(?end, "detection over");
        session.finish(end);
        let _ = self.capture.send(CaptureCmd::Detect(None));
    }

    /// A connected device's fingerprint and name.
    fn device_identity(&self, device: Option<DeviceRef>) -> Option<(String, String)> {
        let d = self.devices.list().iter().find(|d| Some(d.id) == device)?;
        Some((d.fingerprint.to_string(), crate::tuning::device_name(&d.fingerprint.to_string())))
    }

    /// Looks at an input event while detecting. True if it was only for
    /// detecting (`LocalPress`, `PressDevice`, `DetectHeld`), which go
    /// nowhere else, and never count as this computer's own input.
    pub(super) fn detect_input(&mut self, ev: &Event) -> bool {
        let only = matches!(ev, Event::LocalPress { .. } | Event::PressDevice { .. } | Event::DetectHeld { .. });
        let away = self.remote.is_some();
        let Some(session) = self.detect.as_mut() else { return only };
        let now = session.ms();
        let step = match ev {
            Event::LocalPress { session: id, press } if *id == session.id => session.detector.press(*press, away, now),
            Event::PressDevice { session: id, press, device } if *id == session.id => {
                session.detector.device(*press, *device, now)
            }
            Event::DetectHeld { session: id, codes } if *id == session.id => {
                session.detector.seed(codes, now);
                Step::Wait
            }
            // Where the backend can't report this computer's own presses: input on its way
            // to another computer (or held for a crossing), as the rules see it.
            &Event::Key { code, down, device } if !session.local_presses => {
                session.detector.press(Press { code, down, device, ..Press::default() }, away, now)
            }
            &Event::Button { button, down, device } if !session.local_presses => {
                session.detector.press(Press { code: button, down, device, ..Press::default() }, away, now)
            }
            _ => return only,
        };
        if let Some(end) = ended(step) {
            self.end_detect(end);
        }
        only
    }

    /// When the detection must be looked at next.
    pub(super) fn detect_deadline(&self) -> Option<Instant> {
        self.detect.as_ref().map(Session::deadline)
    }

    pub(super) fn detect_timer(&mut self, now: Instant) {
        let Some(session) = self.detect.as_mut() else { return };
        if session.abandoned() {
            self.detect_abandoned();
            return;
        }
        let ms = session.ms();
        if let Some(end) = ended(session.detector.tick(ms)) {
            self.end_detect(end);
        } else if now >= session.until {
            self.end_detect(End::Timeout);
        }
    }

    /// Called at the top of each pass of the daemon's loop, before it takes
    /// the next event: the select prefers input and link traffic, so a
    /// hang-up seen only by its own branch could wait behind a busy stream.
    pub(super) fn detect_check_abandoned(&mut self) {
        if self.detect.as_ref().is_some_and(Session::abandoned) {
            self.detect_abandoned();
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
