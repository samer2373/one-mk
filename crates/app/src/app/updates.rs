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
        // The top bar says an update waits (a pill, shell.rs); what it is
        // and the choices are in Settings (updates_card).
    }
}

/// Where an update stands: the top bar's pill, and the box in Settings.
pub(super) enum UpdateBox {
    /// A newer version waits (the card says how it installs here).
    Ready,
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
        self.update_available.as_ref().map(|_| UpdateBox::Ready)
    }
}

impl App {
    /// Whether it is known how updates work here (else Settings says it is
    /// checking, and shows nothing else about them).
    pub(super) fn update_known(&self) -> bool {
        self.update_ui.status.is_some()
    }

    /// The room under Settings' "Check for updates" row: a version kept to
    /// go back to, the update's own progress or failure, and the note.
    pub(super) fn update_more_h(&self, ui: &Ui, w: f32) -> f32 {
        let Some(status) = &self.update_ui.status else { return 0.0 };
        let note_h = ui.painter().layout(update_note(status).to_owned(), regular(T_META), pal().ink3, w.max(1.0)).size().y;
        let rollback = if status.rollback_available { 44.0 } else { 0.0 };
        4.0 + rollback + self.update_box_h(ui, w) + note_h.max(26.0) + 10.0
    }

    /// One row of Settings' Updates group: `update-check`, `update-auto`,
    /// `update-now` (the button and what it found) or `update-more`.
    pub(super) fn update_row(&mut self, ui: &mut Ui, key: &str, row: Rect) {
        let Some(status) = self.update_ui.status.clone() else {
            // Looking; or it could not be read, which the person is told (the
            // answer never comes, so "looking" would be said for ever).
            let said = self.update_output.as_deref().map(str::trim).filter(|said| !said.is_empty() && !self.busy.contains(&Job::Update));
            let (text, ink) = match said {
                Some(said) => (said, pal().ink),
                None => ("Checking how updates work here…", pal().ink2),
            };
            ui.painter().text(row.left_center(), Align2::LEFT_CENTER, fit(ui.painter(), text, &regular(T_BODY), row.width()), regular(T_BODY), ink);
            return;
        };
        let busy = self.busy.contains(&Job::Update) || self.busy.contains(&Job::UpdateSettings);
        match key {
            "update-check" => {
                if widgets::switch_help(ui, row, Id::unique("update-check"), "Look for updates", "update-check", status.check, !busy).clicked() {
                    self.busy.push(Job::UpdateSettings);
                    self.link.update_preference(false, !status.check);
                }
            }
            "update-auto" => {
                if widgets::switch_help(ui, row, Id::unique("update-auto"), "Install them without asking", "update-auto", status.install_automatically, !busy && status.check && status.can_update && !status.needs_password).clicked() {
                    self.busy.push(Job::UpdateSettings);
                    self.link.update_preference(true, !status.install_automatically);
                }
            }
            "update-now" => self.update_now_row(ui, row, &status, busy),
            _ => {
                let c = Rect::from_min_max(row.min + vec2(0.0, 4.0), row.max);
                let mut y = c.top();
                if status.rollback_available {
                    let label = "Go back to the previous version";
                    let b = Rect::from_min_size(pos2(c.left(), y), vec2(button_width(ui, label), 32.0));
                    if button(ui, b, Id::unique("update-rollback"), label, Look::Quiet, !busy, None).clicked() {
                        self.busy.push(Job::Update);
                        self.link.rollback_update();
                    }
                    y += 44.0;
                }
                let y = self.update_box_rows(ui, c, y);
                // The last check failed (offline, or a release without its
                // signed list): say so rather than look up to date.
                para(ui, pos2(c.left(), y), c.width(), update_note(&status), regular(T_META), pal().ink3);
            }
        }
    }

    /// The button that matters: look now; once a newer version is known,
    /// install it from here too; and what it found, beside it.
    fn update_now_row(&mut self, ui: &mut Ui, row: Rect, status: &UpdateStatus, busy: bool) {
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
        let b = Rect::from_min_size(pos2(row.left(), row.center().y - 16.0), vec2(button_width(ui, &label), 32.0));
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
            else if status.error.is_some() && !failure_note_shown(status) { "The last check didn't work.".to_owned() }
            else if self.update_ui.checked { format!("You're up to date. oneMK {}.", env!("CARGO_PKG_VERSION")) }
            else { format!("oneMK {}", env!("CARGO_PKG_VERSION")) };
        let tx = b.right() + 12.0;
        ui.painter().text(pos2(tx, b.center().y), Align2::LEFT_CENTER, fit(ui.painter(), &said, &regular(T_BODY), (row.right() - tx).max(40.0)), regular(T_BODY), pal().ink2);
    }

    /// What the old sidebar's update box said that the card's own lines don't
    /// (its note already says how a ready update goes): how far it is,
    /// moving the app first, or what went wrong. Its height `w` wide.
    fn update_box_h(&self, ui: &Ui, w: f32) -> f32 {
        let para_h = |text: &str, rows: usize| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), regular(T_META), pal().ink2, w.max(1.0));
            job.wrap.max_rows = rows;
            ui.painter().layout_job(job).size().y
        };
        match self.update_box() {
            Some(UpdateBox::MoveFirst) => 24.0 + 44.0,
            Some(UpdateBox::Working { .. }) => 16.0,
            Some(UpdateBox::Failed { said }) => 24.0 + para_h(if said.is_empty() { FAILED_FALLBACK } else { &said }, 3) + 10.0 + 44.0,
            _ => 0.0,
        }
    }

    /// The update box's lines in the card, from `y`; returns where it ends.
    fn update_box_rows(&mut self, ui: &mut Ui, c: Rect, mut y: f32) -> f32 {
        let line = |ui: &Ui, y: f32, text: &str, font: egui::FontId, col: Color32| {
            ui.painter().text(pos2(c.left(), y + 9.0), Align2::LEFT_CENTER, fit(ui.painter(), text, &font, c.width()), font, col);
        };
        let locked = self.sheet_open();
        match self.update_box() {
            Some(UpdateBox::MoveFirst) => {
                line(ui, y, "Move to Applications", semibold(T_BODY), pal().ink);
                ui.painter().text(pos2(c.left() + text_width(ui, "Move to Applications", semibold(T_BODY)) + 10.0, y + 9.0), Align2::LEFT_CENTER, "It's running from the disk image", regular(T_META), pal().ink2);
                let b = Rect::from_min_size(pos2(c.left(), y + 24.0), vec2(button_width(ui, "Move to Applications"), 32.0));
                if button(ui, b, Id::unique("update-move"), "Move to Applications", Look::Quiet, !locked, None).clicked() {
                    self.update_ui.failed = false;
                    self.busy.push(Job::Update);
                    self.link.move_to_applications();
                }
                y += 24.0 + 44.0;
            }
            Some(UpdateBox::Working { done, said }) => {
                // A bar that fills; without a percentage, a short bead that
                // travels along it.
                let track = Rect::from_min_size(pos2(c.left(), y + 4.0), vec2(c.width(), 5.0));
                let p = ui.painter();
                p.rect_filled(track, CornerRadius::same(3), pal().surface2);
                let fill = match done {
                    Some(k) => Rect::from_min_size(track.min, vec2(track.width() * k.clamp(0.04, 1.0), track.height())),
                    None => {
                        let t = if motion::reduced() { 0.5 } else { (self.now * 0.8).fract() as f32 };
                        let bw = track.width() * 0.28;
                        Rect::from_min_size(pos2(track.left() + (track.width() - bw) * t, track.top()), vec2(bw, track.height()))
                    }
                };
                p.rect_filled(fill, CornerRadius::same(3), pal().accent);
                if done.is_none() && !motion::reduced() {
                    ui.ctx().request_repaint_after_secs(1.0 / 30.0);
                }
                widgets::a11y_text(ui, Id::unique("update-progress"), track, Role::ProgressIndicator, &said, Some(Live::Polite));
                y += 16.0;
            }
            Some(UpdateBox::Failed { said }) => {
                line(ui, y, "The update didn't finish", semibold(T_BODY), pal().ink);
                let shown = if said.is_empty() { FAILED_FALLBACK } else { said.as_str() };
                let mut job = egui::text::LayoutJob::simple(shown.to_owned(), regular(T_META), pal().ink2, c.width().max(1.0));
                job.wrap.max_rows = 3;
                let g = ui.painter().layout_job(job);
                let gh = g.size().y;
                ui.painter().galley(pos2(c.left(), y + 24.0), g, pal().ink2);
                let b = Rect::from_min_size(pos2(c.left(), y + 24.0 + gh + 10.0), vec2(button_width(ui, "Download"), 32.0));
                if button(ui, b, Id::unique("update-download"), "Download", Look::Quiet, !locked, Some("Open the download page")).clicked() {
                    self.link.download_update();
                }
                y += 24.0 + gh + 10.0 + 44.0;
            }
            _ => {}
        }
        y
    }
}

/// What a failed update says when it said nothing itself.
const FAILED_FALLBACK: &str = "Get the new version from the download page.";

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
