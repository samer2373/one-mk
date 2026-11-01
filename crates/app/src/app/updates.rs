use super::*;
use one_mk_ctl::update_logic::UpdateStatus;

#[derive(Default)]
pub(super) struct UpdateUi {
    pub status: Option<UpdateStatus>,
    pub failed: bool,
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
        if status.check && status.install_automatically && status.can_update && status.error.is_none()
            && let Some(version) = &self.update_available
            && self.update_ui.attempted.as_ref() != Some(version) && !self.busy.contains(&Job::Update)
        {
            self.update_ui.attempted = Some(version.clone());
            self.begin_update();
        }
        let _ = ctx;
        // What to show lives in the sidebar's foot (shell.rs, update_box),
        // and the choices in Advanced.
    }
}

/// What the update box in the sidebar shows.
pub(super) enum UpdateBox {
    /// A newer version, and whether this install can replace itself.
    Ready { version: String, can: bool, advice: String },
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
        Some(UpdateBox::Ready { version, can: status.is_none_or(|s| s.can_update), advice: status.map(|s| s.advice.clone()).unwrap_or_default() })
    }
}

impl App {
    pub(super) fn updates_card_h(&self) -> f32 {
        let rollback = self.update_ui.status.as_ref().is_some_and(|s| s.rollback_available);
        48.0 + 2.0 * 36.0 + if rollback { 44.0 } else { 0.0 } + 26.0 + 14.0
    }

    /// Advanced > Updates: whether to look for new versions (at most once
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
        if widgets::switch(ui, row(0.0), Id::unique("update-check"), "Look for updates", status.check, !busy).clicked() {
            self.busy.push(Job::UpdateSettings);
            self.link.update_preference(false, !status.check);
        }
        if widgets::switch(ui, row(1.0), Id::unique("update-auto"), "Install them without asking", status.install_automatically, !busy && status.check && status.can_update).clicked() {
            self.busy.push(Job::UpdateSettings);
            self.link.update_preference(true, !status.install_automatically);
        }
        let mut y = c.top() - 6.0 + 2.0 * 36.0 + 8.0;
        if status.rollback_available {
            let label = "Go back to the previous version";
            let b = Rect::from_min_size(pos2(c.left(), y), vec2(button_width(ui, label), 32.0));
            if button(ui, b, Id::unique("update-rollback"), label, Look::Quiet, !busy, None).clicked() {
                self.busy.push(Job::Update);
                self.link.rollback_update();
            }
            y += 44.0;
        }
        let note = if status.can_update { "At most once a day. Installing restarts oneMK for a few seconds." } else { status.advice.as_str() };
        ui.painter().text(pos2(c.left(), y + 6.0), Align2::LEFT_CENTER, fit(ui.painter(), note, &regular(T_META), c.width()), regular(T_META), INK3);
    }
}
