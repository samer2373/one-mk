use super::*;
use one_mk_ctl::update_logic::UpdateStatus;

#[derive(Default)]
pub(super) struct UpdateUi {
    pub status: Option<UpdateStatus>,
    pub failed: bool,
    /// A check made from this window finished: "up to date" may be said.
    pub checked: bool,
    acknowledged: bool,
    attempted: Option<String>,
}

impl App {
    pub(super) fn begin_update(&mut self) {
        if self.busy.contains(&Job::Update) { return; }
        self.update_ui.failed = false;
        self.update_output = Some("Preparing the update…".into());
        self.busy.push(Job::Update);
        self.link.update();
    }

    /// Once a frame: tells the daemon the new window started (so a kept
    /// previous version can go), and installs by itself when asked to.
    pub(super) fn update_notice(&mut self, ctx: &egui::Context) {
        if !self.update_ui.acknowledged {
            self.update_ui.acknowledged = true;
            self.link.acknowledge_app_start();
        }
        let Some(status) = self.update_ui.status.clone() else { return; };
        if status.check && status.install_automatically && status.can_update && !status.needs_password && status.error.is_none()
            && let Some(version) = &self.update_available
            && status.skipped.as_ref() != Some(version)
            && self.update_ui.attempted.as_ref() != Some(version) && !self.busy.contains(&Job::Update)
        {
            self.update_ui.attempted = Some(version.clone());
            self.begin_update();
        }
        let _ = ctx;
        // What to show lives in the sidebar's foot (shell.rs, update_box),
        // and the choices in Settings.
    }
}

/// What the update box in the sidebar shows.
pub(super) enum UpdateBox {
    /// A newer version, and whether this install can replace itself.
    Ready { version: String, can: bool, advice: String, password: bool },
    /// Running from the disk image (or translocated): move it first.
    MoveFirst,
    /// Downloading or installing: how far (0–1, when known) and what now.
    Working { done: Option<f32>, said: String },
    /// It didn't work: what happened, and a way to get it by hand.
    Failed { said: String },
}

impl App {
    pub(super) fn update_box(&self) -> Option<UpdateBox> {
        let status = self.update_ui.status.as_ref();
        let said = self.update_output.as_deref().map(str::trim).unwrap_or("").to_owned();
        if self.busy.contains(&Job::Update) {
            let done = said.strip_prefix("Downloading… ").and_then(|t| t.strip_suffix('%')).and_then(|t| t.parse::<f32>().ok()).map(|p| p / 100.0);
            return Some(UpdateBox::Working { done, said: if said.is_empty() { "Preparing…".into() } else { said } });
        }
        if self.update_ui.failed {
            return Some(UpdateBox::Failed { said });
        }
        if status.is_some_and(|s| s.move_to_applications) {
            return Some(UpdateBox::MoveFirst);
        }
        let version = self.update_available.clone()?;
        Some(UpdateBox::Ready { version, can: status.is_none_or(|s| s.can_update), advice: status.map(|s| s.advice.clone()).unwrap_or_default(), password: status.is_some_and(|s| s.needs_password) })
    }
}

impl App {
    pub(super) fn updates_card_h(&self, ui: &Ui, width: f32) -> f32 {
        let rollback = self.update_ui.status.as_ref().is_some_and(|s| s.rollback_available);
        let note_h = self.update_ui.status.as_ref().map_or(26.0, |status| ui.painter().layout(update_note(status).to_owned(), regular(T_META), INK3, (width - 36.0).max(1.0)).size().y);
        48.0 + 2.0 * 36.0 + 44.0 + if rollback { 44.0 } else { 0.0 } + note_h.max(26.0) + 14.0
    }

    /// Settings > Updates: whether to look for new versions (at most once
    /// a day) and whether to install them without asking; going back to
    /// the previous version when one is kept.
    pub(super) fn updates_card(&mut self, ui: &mut Ui, r: Rect) {
        let c = super::pages::card(ui, r, "Updates");
        let Some(status) = self.update_ui.status.clone() else {
            ui.painter().text(pos2(c.left(), c.top() + 10.0), Align2::LEFT_CENTER, "Checking how updates work here…", regular(T_BODY), INK2);
            return;
        };
        let busy = self.busy.contains(&Job::Update) || self.busy.contains(&Job::UpdateSettings);
        let row = |k: f32| Rect::from_min_size(pos2(c.left(), c.top() - 6.0 + k * 36.0), vec2(c.width(), 36.0));
        if widgets::switch_help(ui, row(0.0), Id::unique("update-check"), "Look for updates", "update-check", status.check, !busy).clicked() {
            self.busy.push(Job::UpdateSettings);
            self.link.update_preference(false, !status.check);
        }
        if widgets::switch_help(ui, row(1.0), Id::unique("update-auto"), "Install them without asking", "update-auto", status.install_automatically, !busy && status.check && status.can_update && !status.needs_password).clicked() {
            self.busy.push(Job::UpdateSettings);
            self.link.update_preference(true, !status.install_automatically);
        }
        let mut y = c.top() - 6.0 + 2.0 * 36.0 + 8.0;
        // The button that matters: look now; once a newer version is known,
        // install it from here too.
        let checking = self.busy.contains(&Job::UpdateCheck);
        let installing = self.busy.contains(&Job::Update);
        let version = self.update_available.clone();
        let (label, look) = match (&version, status.can_update) {
            _ if installing => ("Updating…".to_owned(), Look::Quiet),
            _ if checking => ("Checking…".to_owned(), Look::Quiet),
            (Some(v), true) => (format!("Update to {v}"), Look::Primary),
            (Some(_), false) => ("Download".to_owned(), Look::Primary),
            (None, _) => ("Check for updates".to_owned(), Look::Quiet),
        };
        let b = Rect::from_min_size(pos2(c.left(), y), vec2(button_width(ui, &label), 32.0));
        if button(ui, b, Id::unique("update-now-or-check"), &label, look, !busy && !installing && !checking && !self.sheet_open(), None).clicked() {
            match (&version, status.can_update) {
                (Some(_), true) => self.begin_update(),
                (Some(_), false) => self.link.download_update(),
                (None, _) => self.link.check_updates_now(),
            }
        }
        let said = if checking { "Looking for a new version…".to_owned() }
            else if installing { self.update_output.clone().unwrap_or_default() }
            else if let Some(v) = &version { format!("oneMK {v} is available. You have {}.", env!("CARGO_PKG_VERSION")) }
            // The note below says it when the check is on; once.
            else if status.error.is_some() && !failure_note_shown(&status) { "The last check didn't work.".to_owned() }
            else if self.update_ui.checked { format!("You're up to date. oneMK {}.", env!("CARGO_PKG_VERSION")) }
            else { format!("oneMK {}", env!("CARGO_PKG_VERSION")) };
        let tx = b.right() + 12.0;
        ui.painter().text(pos2(tx, b.center().y), Align2::LEFT_CENTER, fit(ui.painter(), &said, &regular(T_BODY), (c.right() - tx).max(40.0)), regular(T_BODY), INK2);
        y += 44.0;
        if status.rollback_available {
            let label = "Go back to the previous version";
            let b = Rect::from_min_size(pos2(c.left(), y), vec2(button_width(ui, label), 32.0));
            if button(ui, b, Id::unique("update-rollback"), label, Look::Quiet, !busy, None).clicked() {
                self.busy.push(Job::Update);
                self.link.rollback_update();
            }
            y += 44.0;
        }
        // The last check failed (offline, or a release without its signed
        // list): say so rather than look up to date.
        para(ui, pos2(c.left(), y), c.width(), update_note(&status), regular(T_META), INK3);
    }
}

/// Whether the note under the button is the one about a failed check.
fn failure_note_shown(status: &UpdateStatus) -> bool {
    status.check && status.error.is_some() && status.available.is_none()
}

fn update_note(status: &UpdateStatus) -> &str {
    if failure_note_shown(status) { "The last check for updates didn't work. oneMK tries again tomorrow." }
    else if status.can_update && status.needs_password { "Installing asks for your password, then restarts oneMK for a few seconds." }
    else if status.can_update { "At most once a day. Installing restarts oneMK for a few seconds." } else { status.advice.as_str() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(check: bool) -> UpdateStatus {
        UpdateStatus { check, error: Some("offline".into()), ..Default::default() }
    }

    #[test]
    fn a_failed_check_is_said_once_not_twice() {
        // Checking on: the note under the button says it, the line beside
        // the button does not repeat it.
        let status = failed(true);
        assert!(failure_note_shown(&status));
        assert!(update_note(&status).contains("didn't work"));
        // Checking off: the note is about something else, so the line says it.
        let status = failed(false);
        assert!(!failure_note_shown(&status));
        assert!(!update_note(&status).contains("didn't work"));
    }
}
