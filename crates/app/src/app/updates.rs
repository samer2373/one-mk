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

    pub(super) fn update_notice(&mut self, ctx: &egui::Context) {
        if !self.update_ui.acknowledged {
            self.update_ui.acknowledged = true;
            self.link.acknowledge_app_start();
        }
        let Some(mut status) = self.update_ui.status.clone() else { return; };
        if status.check && status.install_automatically && status.can_update && status.error.is_none()
            && let Some(version) = &self.update_available
            && self.update_ui.attempted.as_ref() != Some(version) && !self.busy.contains(&Job::Update)
        {
            self.update_ui.attempted = Some(version.clone());
            self.begin_update();
        }
        let busy = self.busy.contains(&Job::Update);
        if self.update_available.is_none() && !status.move_to_applications && !self.update_ui.failed && self.tab != Tab::Advanced && !busy { return; }
        egui::Window::new("one·mk updates").id(Id::unique("update-notice"))
            .anchor(egui::Align2::RIGHT_TOP, [-20.0, 72.0]).default_width(350.0).resizable(false).collapsible(false)
            .show(ctx, |ui| {
                if let Some(version) = &self.update_available {
                    ui.label(format!("one·mk {version} is ready"));
                }
                if status.move_to_applications {
                    ui.label("Move one·mk to Applications before updating.");
                    if ui.add_enabled(!busy, egui::Button::new("Move to Applications")).clicked() {
                        self.update_ui.failed = false;
                        self.busy.push(Job::Update);
                        self.link.move_to_applications();
                    }
                } else if !status.can_update {
                    ui.label(&status.advice);
                } else if self.update_available.is_some() && !busy
                    && ui.button("Update now").clicked() { self.begin_update(); }
                if busy {
                    ui.spinner();
                    if let Some(percent) = self.update_output.as_deref().and_then(|text| text.strip_prefix("Downloading… "))
                        .and_then(|text| text.strip_suffix('%')).and_then(|text| text.parse::<f32>().ok()) {
                        ui.add(egui::ProgressBar::new(percent / 100.0).show_percentage());
                    }
                }
                if let Some(output) = &self.update_output { ui.label(output.trim()); }
                if let Some(error) = &status.error { ui.label(format!("Couldn't check: {error}")); }
                if (self.update_ui.failed || !status.can_update) && ui.button("Download").clicked() {
                    self.link.download_update();
                }
                if status.rollback_available && ui.add_enabled(!busy, egui::Button::new("Restore previous version")).clicked() {
                    self.busy.push(Job::Update);
                    self.link.rollback_update();
                }
                ui.separator();
                let saving = self.busy.contains(&Job::UpdateSettings);
                ui.add_enabled_ui(!busy && !saving, |ui| {
                    if ui.checkbox(&mut status.check, "Check for updates").changed() {
                        self.busy.push(Job::UpdateSettings);
                        self.link.update_preference(false, status.check);
                    }
                    if ui.checkbox(&mut status.install_automatically, "Update automatically").changed() {
                        self.busy.push(Job::UpdateSettings);
                        self.link.update_preference(true, status.install_automatically);
                    }
                });
                ui.small("Checks happen at most once a day. By default, we ask before installing. Automatic installs happen only while this window is open.");
            });
    }
}
