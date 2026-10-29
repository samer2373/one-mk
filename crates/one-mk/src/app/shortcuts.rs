//! Keyboard shortcuts in the daemon: the system's listener while the
//! pointer is here, the matcher on captured keys while it is away (or held
//! at an edge), and both turned into the requests `one-mk switch` and
//! `one-mk lock-pointer` send, handled by the same code.

use super::App;
use crate::hotkeys::{self, HotkeySettings};
use one_mk_input::hotkeys::{Action, Availability, Bindings, Debounce, Listener, Matcher, Source, Verdict};
use one_mk_input::{Event, Kind};
use tokio::sync::{mpsc, oneshot};

pub(super) struct Shortcuts {
    listener: Listener,
    bindings: Bindings,
    /// Why `[hotkeys]` in settings.toml can't be used.
    problem: Option<String>,
    availability: Option<Availability>,
    matcher: Matcher,
    /// The capture the matcher's keys belong to.
    capture: Option<u32>,
    debounce: Debounce,
}

/// The shortcuts `settings` asks for (none, with the reason, if they can't
/// be used).
fn bindings(settings: &HotkeySettings) -> (Bindings, Option<String>) {
    match settings.bindings() {
        Ok(b) => (b, None),
        Err(e) => {
            tracing::warn!("keyboard shortcuts in settings.toml: {e}");
            (Bindings::default(), Some(format!("settings.toml: {e}")))
        }
    }
}

impl Shortcuts {
    /// Starts listening; the listener's events come from the receiver.
    pub(super) fn start(kind: Kind, settings: &HotkeySettings) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (bindings, problem) = bindings(settings);
        let (listener, rx) = one_mk_input::hotkeys::listen(kind, bindings.clone());
        let s = Shortcuts {
            listener,
            bindings,
            problem,
            availability: None,
            matcher: Matcher::default(),
            capture: None,
            debounce: Debounce::default(),
        };
        (s, rx)
    }

    /// `settings.toml` was reloaded. The system is asked again only if the
    /// shortcuts changed (on GNOME that shows a dialog).
    pub(super) fn reload(&mut self, settings: &HotkeySettings) {
        let (bindings, problem) = bindings(settings);
        self.problem = problem;
        if bindings != self.bindings {
            self.bindings = bindings.clone();
            self.listener.set(bindings);
        }
    }

    /// For `one-mk status`: the state word and the details.
    pub(super) fn status(&self) -> (String, String) {
        let clean = |s: &str| s.replace(['\t', '\n'], " ");
        if self.bindings.is_empty() {
            return ("off".into(), self.problem.as_deref().map(clean).unwrap_or_default());
        }
        match &self.availability {
            None => ("starting".into(), "asking the system".into()),
            Some(Availability::Starting(s)) => ("starting".into(), clean(s)),
            Some(Availability::Active(s)) => ("on".into(), clean(s)),
            Some(Availability::Unavailable(s)) => ("unavailable".into(), clean(s)),
        }
    }
}

impl App {
    /// `Event::Hotkey` and `Event::Hotkeys`, from the system's listener.
    pub(super) fn on_shortcut_event(&mut self, ev: Event) {
        match ev {
            Event::Hotkey(a) => self.shortcut(a, Source::System),
            Event::Hotkeys(a) => {
                match &a {
                    Availability::Unavailable(why) => tracing::info!(
                        "keyboard shortcuts work only while the pointer is on another computer ({why}); here, {}",
                        one_mk_ctl::status::HOTKEYS_FALLBACK
                    ),
                    a => tracing::debug!(?a, "keyboard shortcuts"),
                }
                self.shortcuts.availability = Some(a);
            }
            _ => {}
        }
    }

    /// A key captured while the pointer is on another computer or held at an
    /// edge. True if it belongs to a shortcut and must go no further.
    pub(super) fn captured_shortcut(&mut self, code: u32, down: bool) -> bool {
        let capture = self.remote.as_ref().map(|r| r.activation).or(self.approach.as_ref().map(|(id, _)| *id));
        let s = &mut self.shortcuts;
        if capture.is_none() || s.bindings.is_empty() {
            return false;
        }
        if s.capture != capture {
            s.capture = capture;
            s.matcher.reset();
        }
        match s.matcher.key(&s.bindings, self.mods, code, down) {
            Verdict::Pass => false,
            Verdict::Swallow => true,
            Verdict::Fire(a) => {
                self.shortcut(a, Source::Captured);
                true
            }
        }
    }

    fn shortcut(&mut self, action: Action, source: Source) {
        if !self.shortcuts.debounce.admit(action, source, std::time::Instant::now()) {
            return;
        }
        let order = hotkeys::numbered_from(self.node.identity(), &self.trust);
        let req = match hotkeys::request(action, &order) {
            Ok(r) => r,
            Err(e) => return tracing::warn!("keyboard shortcut ({}): {e}", action.describe().to_lowercase()),
        };
        let (tx, mut rx) = oneshot::channel();
        self.on_request(req, tx);
        let reply = rx.try_recv().unwrap_or_default();
        match reply.strip_prefix("error ") {
            Some(e) => tracing::warn!("keyboard shortcut ({}): {}", action.describe().to_lowercase(), e.trim()),
            None => tracing::info!(?action, ?source, reply = reply.lines().next().unwrap_or(""), "keyboard shortcut"),
        }
    }
}
