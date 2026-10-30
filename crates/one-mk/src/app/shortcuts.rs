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
    settings: HotkeySettings,
    /// Computers that can be jumped to, this one included.
    computers: usize,
    /// A peer controls this computer: the system's shortcuts are paused.
    controlled: bool,
}

/// The shortcuts `settings` asks for with `computers` computers (none,
/// with the reason, if they can't be used).
fn bindings(settings: &HotkeySettings, computers: usize) -> (Bindings, Option<String>) {
    match settings.bindings_for(computers) {
        Ok(b) => (b, None),
        Err(e) => {
            tracing::warn!("keyboard shortcuts in settings.toml: {e}");
            (Bindings::default(), Some(format!("settings.toml: {e}")))
        }
    }
}

impl Shortcuts {
    /// Starts listening; the listener's events come from the receiver.
    pub(super) fn start(kind: Kind, settings: &HotkeySettings, computers: usize) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (bindings, problem) = bindings(settings, computers);
        let (listener, rx) = one_mk_input::hotkeys::listen(kind, bindings.clone());
        let s = Shortcuts {
            listener,
            bindings,
            problem,
            availability: None,
            matcher: Matcher::default(),
            capture: None,
            debounce: Debounce::default(),
            settings: settings.clone(),
            computers,
            controlled: false,
        };
        (s, rx)
    }

    /// `settings.toml` was reloaded.
    pub(super) fn reload(&mut self, settings: &HotkeySettings) {
        self.settings = settings.clone();
        self.apply();
    }

    /// The number of computers (this one and the paired ones) may have
    /// changed: jumps are bound only to computers that exist.
    pub(super) fn set_computers(&mut self, computers: usize) {
        if std::mem::replace(&mut self.computers, computers) != computers {
            self.apply();
        }
    }

    /// The system is asked again only if the shortcuts changed (on GNOME
    /// that shows a dialog).
    fn apply(&mut self) {
        let (bindings, problem) = bindings(&self.settings, self.computers);
        self.problem = problem;
        if bindings != self.bindings {
            self.bindings = bindings.clone();
            self.listener.set(bindings);
        }
    }

    /// Whether a peer controls this computer. Its keys arrive through the
    /// injector, and must not run this computer's shortcuts: the system's
    /// listener is paused where that's possible (Windows, macOS) and what
    /// it reports meanwhile is ignored (see `App::on_shortcut_event`).
    pub(super) fn set_controlled(&mut self, controlled: bool) {
        if std::mem::replace(&mut self.controlled, controlled) != controlled {
            self.listener.pause(controlled);
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
            Event::Hotkey(a) if !system_shortcut_applies(self.owner.is_some()) => {
                tracing::debug!(?a, "keyboard shortcut ignored: another computer controls this one");
            }
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

    /// A key captured while the pointer is on another computer, held at an
    /// edge, or broadcast. True if it belongs to a shortcut and must go no
    /// further.
    pub(super) fn captured_shortcut(&mut self, code: u32, down: bool) -> bool {
        let capture =
            self.remote.as_ref().map(|r| r.activation).or(self.approach.as_ref().map(|(id, _)| *id)).or(self.broadcast_activation());
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
        if action == Action::FetchClipboard {
            tokio::spawn(async move {
                let Ok(reply) = rx.await else { return; };
                let body = if let Some(error) = reply.strip_prefix("error ") {
                    error.trim().to_owned()
                } else {
                    let mut lines = reply.lines().skip(1);
                    let name = lines.next().unwrap_or("another computer");
                    let kind = lines.next().unwrap_or("clipboard");
                    format!("Got {kind} from {name}")
                };
                tracing::info!("{body}");
                crate::notify::failure("one-mk clipboard", &body).await;
            });
            return;
        }
        let reply = rx.try_recv().unwrap_or_default();
        match reply.strip_prefix("error ") {
            Some(e) => tracing::warn!("keyboard shortcut ({}): {}", action.describe().to_lowercase(), e.trim()),
            None => tracing::info!(?action, ?source, reply = reply.lines().next().unwrap_or(""), "keyboard shortcut"),
        }
    }
}

/// Whether a shortcut the system reports is this computer's to act on:
/// not while a peer controls it. Its keys arrive through the injector, and
/// the system's shortcut service fires on them as on any (`RegisterHotKey`
/// and the Linux portal can't tell them apart), so a chord typed on the
/// peer for a program here would jump, lock the pointer or take control
/// back. The peer's own shortcuts are handled on the peer.
fn system_shortcut_applies(controlled: bool) -> bool {
    !controlled
}

#[cfg(test)]
mod tests {
    #[test]
    fn system_shortcuts_wait_while_controlled() {
        assert!(super::system_shortcut_applies(false));
        assert!(!super::system_shortcut_applies(true));
    }
}
