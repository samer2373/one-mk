use super::*;
use one_mk_wallpaper::{Canvas, Command, Fit, Media};

#[derive(Default)]
pub(super) struct WallpaperUi {
    picker: Option<Receiver<Option<PathBuf>>>,
    poster_path: Option<PathBuf>,
    poster: Option<egui::TextureHandle>,
}

impl App {
    fn wallpaper_command(&mut self, command: Command) {
        self.link.wallpaper(command);
    }

    fn choose_video(&mut self, ctx: &egui::Context) {
        if self.wallpaper_ui.picker.is_some() { return; }
        let (sender, receiver) = std::sync::mpsc::channel();
        self.wallpaper_ui.picker = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new().set_title("Choose a video")
                .add_filter("Video", &["mp4", "mov", "m4v", "webm", "mkv"]).pick_file();
            let _ = sender.send(file);
            ctx.request_repaint();
        });
    }

    pub(super) fn wallpaper_page(&mut self, ui: &mut Ui, area: Rect) {
        if let Some(result) = self.wallpaper_ui.picker.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
            self.wallpaper_ui.picker = None;
            if let Some(path) = result { self.wallpaper_command(Command::Set { path }); }
        }
        let dropped = ui.input(|input| input.raw.dropped_files.first().map(|file| file.path().to_path_buf()));
        if let Some(path) = dropped { self.wallpaper_command(Command::Set { path }); }
        let status = self.running().cloned();
        let wallpaper = status.as_ref().and_then(|status| status.wallpaper.clone()).unwrap_or_default();
        let plan = wallpaper.plan.as_ref();
        let canvas = plan.map(|plan| plan.canvas.clone()).unwrap_or_else(|| status.as_ref().map(one_mk_ctl::wallpaper::canvas).unwrap_or_default());
        let mut poster_path = wallpaper.poster.clone();
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("ONE_MK_APP_WALLPAPER") { poster_path = Some(path.into()); }
        if self.wallpaper_ui.poster_path != poster_path {
            self.wallpaper_ui.poster = poster_path.as_ref().and_then(|path| {
                let image = image::ImageReader::open(path).ok()?.decode().ok()?.into_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                Some(ui.ctx().load_texture("wallpaper-poster", egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()), egui::TextureOptions::LINEAR))
            });
            self.wallpaper_ui.poster_path = poster_path;
        }
        let enabled = status.is_some() && wallpaper.available;
        let selected = plan.is_some();
        egui::ScrollArea::vertical().id_salt("wallpaper-scroll").show_viewport(ui, |ui, _| {
            let width = area.width();
            let origin = ui.cursor().min;
            let preview = Rect::from_min_size(origin, vec2(width, (width * 0.37).clamp(230.0, 360.0)));
            let inside = pages::card(ui, preview, "One picture, across your computers");
            draw_canvas(ui, inside, &canvas, plan.map(|plan| &plan.media), plan.map_or(Fit::Fill, |plan| plan.fit), self.wallpaper_ui.poster.as_ref());
            let controls = Rect::from_min_size(pos2(origin.x, preview.bottom() + 14.0), vec2(width, 166.0));
            let inside = pages::card(ui, controls, plan.map_or("Video", |plan| plan.title.as_str()));
            let choose = Rect::from_min_size(inside.min, vec2(170.0, 34.0));
            if button(ui, choose, Id::unique("choose-video"), "Choose a video…", Look::Primary, enabled && self.wallpaper_ui.picker.is_none(), None).clicked() {
                self.choose_video(ui.ctx());
            }
            if inside.width() > 400.0 { ui.painter().text(choose.right_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, "or drop one here", regular(T_META), INK2); }
            let track = Rect::from_min_size(inside.min + vec2(0.0, 49.0), vec2(240.0_f32.min(inside.width()), 32.0));
            if let Some(chosen) = (Pills { id: Id::unique("wallpaper-fit"), track, cells: Pills::even(track, 3, 2.0), labels: &["Fill", "Fit", "Stretch"], names: vec!["Fill canvas".into(), "Fit inside canvas".into(), "Stretch across canvas".into()], chosen: Some(match plan.map_or(Fit::Fill, |plan| plan.fit) { Fit::Fill => 0, Fit::Fit => 1, Fit::Stretch => 2 }), enabled: vec![enabled && selected; 3], role: Role::RadioGroup, font: T_META }).show(ui, true) {
                self.wallpaper_command(Command::Fit { fit: [Fit::Fill, Fit::Fit, Fit::Stretch][chosen] });
            }
            let computers: Vec<_> = status.as_ref().map(|status| std::iter::once((status.id.clone(), status.name.clone(), true))
                .chain(status.peers.iter().map(|peer| (peer.id.clone(), peer.name.clone(), peer.online))).collect()).unwrap_or_default();
            let play_on = Rect::from_min_size(pos2(origin.x, controls.bottom() + 14.0), vec2(width, 62.0 + computers.len() as f32 * 36.0));
            let inside = pages::card(ui, play_on, "Play on");
            for (index, (id, name, online)) in computers.iter().enumerate() {
                let row = Rect::from_min_size(inside.min + vec2(0.0, index as f32 * 36.0), vec2(inside.width(), 34.0));
                let on = plan.is_none_or(|plan| plan.computers.contains(id));
                let label = if *online { name.clone() } else { format!("{name} · offline") };
                if switch(ui, row, Id::unique(("wallpaper-computer", id)), &label, on, enabled && selected).clicked() {
                    self.wallpaper_command(Command::Computer { id: id.clone(), enabled: !on });
                }
            }
            let power = Rect::from_min_size(pos2(origin.x, play_on.bottom() + 14.0), vec2(width, 165.0));
            let inside = pages::card(ui, power, "Playback");
            let battery = plan.is_none_or(|plan| plan.pause_on_battery);
            let row = Rect::from_min_size(inside.min, vec2(inside.width(), 34.0));
            if switch(ui, row, Id::unique("wallpaper-battery"), "Pause on battery", battery, enabled && selected).clicked() {
                self.wallpaper_command(Command::Battery { enabled: !battery });
            }
            let paused = plan.is_some_and(|plan| plan.paused);
            let play = Rect::from_min_size(inside.min + vec2(0.0, 48.0), vec2(100.0, 34.0));
            if button(ui, play, Id::unique("wallpaper-play"), if paused { "Play" } else { "Pause" }, Look::Primary, enabled && selected, None).clicked() { self.wallpaper_command(Command::Pause { paused: !paused }); }
            let off = play.translate(vec2(112.0, 0.0));
            if button(ui, off, Id::unique("wallpaper-off"), "Turn off", Look::Quiet, status.is_some() && selected, None).clicked() { self.wallpaper_command(Command::Off); }
            let mut message = if status.is_none() { "Start oneMK to choose a wallpaper.".into() } else if wallpaper.message.is_empty() { "Choose a video to get started.".into() } else { wallpaper.message.clone() };
            for (_, name, _) in computers.iter().filter(|(_, _, online)| !online) {
                if !message.ends_with('.') { message.push('.'); }
                message += &format!(" {name} is offline — its part is kept for it.");
            }
            let message_rect = Rect::from_min_size(pos2(origin.x + 14.0, power.bottom() + 12.0), vec2(width - 28.0, 100.0));
            para(ui, message_rect.min, message_rect.width(), &message, regular(T_BODY), INK2);
            ui.allocate_space(vec2(width, message_rect.bottom() - origin.y));
        });
    }
}

fn draw_canvas(ui: &Ui, area: Rect, canvas: &Canvas, media: Option<&Media>, fit: Fit, poster: Option<&egui::TextureHandle>) {
    let Some(bounds) = canvas.bounds() else {
        ui.painter().text(area.center(), Align2::CENTER_CENTER, "Your monitors appear here", regular(T_BODY), INK2);
        return;
    };
    let scale = ((area.width() - 30.0) as f64 / bounds.width).min((area.height() - 44.0) as f64 / bounds.height);
    let top = area.center() - vec2((bounds.width * scale) as f32, (bounds.height * scale) as f32) / 2.0;
    let map = |rect: one_mk_wallpaper::Rect| Rect::from_min_size(top + vec2(((rect.x - bounds.x) * scale) as f32, ((rect.y - bounds.y) * scale) as f32), vec2((rect.width * scale) as f32, (rect.height * scale) as f32));
    let fallback = poster.map(|texture| Media { width: texture.size()[0] as u32, height: texture.size()[1] as u32, duration_ms: 1 }).unwrap_or_default();
    let image_rect = canvas.image_rect(media.unwrap_or(&fallback), fit).map(map);
    for monitor in &canvas.monitors {
        let screen = map(monitor.rect);
        ui.painter().rect_filled(screen, 3, Color32::from_rgb(14, 22, 32));
        let painter = ui.painter().with_clip_rect(screen);
        if let (Some(texture), Some(image_rect)) = (poster, image_rect) { painter.image(texture.id(), image_rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE); }
        ui.painter().rect_stroke(screen, 3, Stroke::new(1.5, if monitor.online { INK2 } else { INK2.gamma_multiply(0.4) }), StrokeKind::Inside);
        let name = fit_text(ui, &monitor.name, screen.width());
        ui.painter().text(screen.center_bottom() + vec2(0.0, 11.0), Align2::CENTER_TOP, name, regular(T_META), INK2);
    }
}

fn fit_text(ui: &Ui, text: &str, width: f32) -> String { fit(ui.painter(), text, &regular(T_META), width) }
