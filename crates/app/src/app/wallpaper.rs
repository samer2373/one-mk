use super::*;
use one_mk_wallpaper::{Canvas, Command, Fit, Media};

#[derive(Default)]
pub(super) struct WallpaperUi {
    picker: Option<Receiver<Option<PathBuf>>>,
    poster_path: Option<PathBuf>,
    poster: Option<egui::TextureHandle>,
}

impl App {
    pub(super) fn wallpaper_command(&mut self, command: Command) {
        self.link.wallpaper(command);
    }

    pub(super) fn choose_video(&mut self, ctx: &egui::Context) {
        if self.wallpaper_ui.picker.is_some() { return; }
        let (sender, receiver) = std::sync::mpsc::channel();
        self.wallpaper_ui.picker = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new().set_title("Choose a video or picture")
                .add_filter("Video or picture", &["mp4", "mov", "m4v", "webm", "mkv", "png", "jpg", "jpeg", "heic", "webp"]).pick_file();
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
        #[cfg(debug_assertions)]
        let poster_path = std::env::var_os("ONE_MK_APP_WALLPAPER").map(Into::into).or_else(|| wallpaper.poster.clone());
        #[cfg(not(debug_assertions))]
        let poster_path = wallpaper.poster.clone();
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
        let fit_now = plan.map_or(Fit::Fill, |plan| plan.fit);
        let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
        let computers: Vec<_> = status.as_ref().map(|status| std::iter::once((status.id.clone(), status.name.clone(), true))
            .chain(status.peers.iter().map(|peer| (peer.id.clone(), peer.name.clone(), peer.online))).collect()).unwrap_or_default();
        egui::ScrollArea::vertical().id_salt("wallpaper-scroll").show_viewport(ui, |ui, _| {
            let width = area.width();
            let origin = ui.cursor().min;
            // The stage: every screen, as it stands, each showing its part
            // of the one picture; what plays and how to change it under it.
            // As tall as the screens need at this width, within reason.
            let need = canvas.bounds().map_or(180.0, |b| ((width - 48.0) as f64 * b.height / b.width) as f32);
            let stage = Rect::from_min_size(origin, vec2(width, need.clamp(150.0, 330.0) + 20.0 + 26.0 + 64.0));
            widgets::group(ui, stage, R_GROUP);
            if hovering && enabled {
                ui.painter().rect_stroke(stage, CornerRadius::same(R_GROUP), Stroke::new(1.5, ACCENT), StrokeKind::Inside);
            }
            let screens = Rect::from_min_max(stage.min + vec2(24.0, 26.0), stage.max - vec2(24.0, 84.0));
            draw_canvas(ui, screens, &canvas, plan.map(|plan| &plan.media), fit_now, self.wallpaper_ui.poster.as_ref(), &computers);
            a11y_text(ui, Id::unique("wallpaper-stage"), stage, Role::Image, &match plan { Some(plan) => plan.playing_message(), None => "No wallpaper playing".into() }, None);
            let foot_y = stage.bottom() - 18.0 - 32.0;
            let mut x = stage.left() + 18.0;
            if let Some(plan) = plan {
                let paused = plan.paused;
                let play = Rect::from_min_size(pos2(x, foot_y), vec2(96.0, 32.0));
                if !plan.picture {
                    if button(ui, play, Id::unique("wallpaper-play"), if paused { "Play" } else { "Pause" }, Look::Glass, enabled, None).clicked() {
                        self.wallpaper_command(Command::Pause { paused: !paused });
                    }
                    x = play.right() + 8.0;
                }
                let what = if plan.picture { plan.title.clone() } else { format!("{} · {} loop", plan.title, clock(plan.media.duration_ms)) };
                let room = (stage.right() - 18.0 - 180.0 - 110.0 - x).max(0.0);
                let shown = fit_text(ui, &what, room - 28.0);
                if room > 60.0 {
                    let chip = Rect::from_min_size(pos2(x, foot_y), vec2(text_width(ui, &shown, regular(T_META)) + 28.0, 32.0));
                    let p = ui.painter();
                    p.rect(chip, CornerRadius::same(16), Color32::from_black_alpha(90), Stroke::new(1.0, Color32::from_white_alpha(16)), StrokeKind::Inside);
                    p.text(chip.center(), Align2::CENTER_CENTER, shown, regular(T_META), INK2);
                }
            } else {
                let hint = if status.is_none() { "Start oneMK to choose a wallpaper" } else if !wallpaper.available { "Wallpaper isn't available on this computer" } else { "Choose a video or picture" };
                ui.painter().text(pos2(x + 2.0, foot_y + 16.0), Align2::LEFT_CENTER, hint, regular(T_BODY), INK2);
            }
            let choose = Rect::from_min_size(pos2(stage.right() - 18.0 - 170.0, foot_y), vec2(170.0, 32.0));
            if button(ui, choose, Id::unique("choose-video"), "Choose…", Look::Primary, enabled && self.wallpaper_ui.picker.is_none(), None).clicked() {
                self.choose_video(ui.ctx());
            }
            if selected {
                let off = Rect::from_min_size(pos2(choose.left() - 8.0 - 100.0, foot_y), vec2(100.0, 32.0));
                if button(ui, off, Id::unique("wallpaper-off"), "Turn off", Look::Quiet, status.is_some(), None).clicked() {
                    self.wallpaper_command(Command::Off);
                }
            }

            // Below: how it fills the screens, and where it plays. Side by
            // side when there's room.
            let two = width >= 640.0;
            let col_w = if two { (width - M * 1.4) / 2.0 } else { width };
            let fills_h = GROUP_TITLE + 8.0 + 2.0 * ROW;
            let fills = Rect::from_min_size(pos2(origin.x, stage.bottom() + 22.0), vec2(col_w, fills_h));
            let rows = list(ui, fills, "How it fills the screens", 2);
            let fit_row = rows[0];
            ui.painter().text(fit_row.left_center(), Align2::LEFT_CENTER, "Fit", regular(T_BODY), if enabled && selected { INK } else { INK2 });
            let track = Rect::from_min_size(pos2(fit_row.right() - 204.0, fit_row.center().y - 14.0), vec2(204.0, 28.0));
            if let Some(chosen) = (Pills { id: Id::unique("wallpaper-fit"), track, cells: Pills::even(track, 3, 2.0), labels: &["Fill", "Fit", "Stretch"], names: vec!["Fill the screens".into(), "Fit inside the screens".into(), "Stretch across the screens".into()], chosen: Some(match fit_now { Fit::Fill => 0, Fit::Fit => 1, Fit::Stretch => 2 }), enabled: vec![enabled && selected; 3], role: Role::RadioGroup, font: T_META }).show(ui, true) {
                self.wallpaper_command(Command::Fit { fit: [Fit::Fill, Fit::Fit, Fit::Stretch][chosen] });
            }
            let battery = plan.is_none_or(|plan| plan.pause_on_battery);
            if switch(ui, rows[1], Id::unique("wallpaper-battery"), "Pause on battery", battery, enabled && selected && plan.is_none_or(|plan| !plan.picture)).clicked() {
                self.wallpaper_command(Command::Battery { enabled: !battery });
            }

            let n = computers.len().max(1);
            let at = if two { pos2(fills.right() + M * 1.4, fills.top()) } else { pos2(origin.x, fills.bottom() + 22.0) };
            let play_on = Rect::from_min_size(at, vec2(col_w, GROUP_TITLE + 8.0 + n as f32 * ROW));
            let rows = list(ui, play_on, "Play on", n);
            if computers.is_empty() {
                ui.painter().text(rows[0].left_center(), Align2::LEFT_CENTER, "Your computers appear here", regular(T_BODY), INK2);
            }
            for ((id, name, online), row) in computers.iter().zip(rows) {
                let p = ui.painter();
                let dot = pos2(row.left() + 4.0, row.center().y);
                if *online { p.circle_filled(dot, 3.5, OK); } else { p.circle_stroke(dot, 3.0, Stroke::new(1.5, INK3)); }
                let on = plan.is_none_or(|plan| plan.computers.contains(id));
                let screens = canvas.monitors.iter().filter(|monitor| &monitor.computer == id).count();
                // Why a computer can't play it is often a sentence: the row
                // says so briefly, the whole reason shows on hover.
                let failed = match wallpaper.computers.get(id) { Some(one_mk_wallpaper::Playback::Error(reason)) if *online && on => Some(reason.clone()), _ => None };
                let note = if failed.is_some() { "can't play it here".to_owned() }
                    else if *online && on && let Some(playback) = wallpaper.computers.get(id) { playback.message().to_owned() } else { match (*online, screens) {
                    (false, _) => "offline · its part is kept".to_owned(),
                    (true, 0) => String::new(),
                    (true, 1) => "1 screen".to_owned(),
                    (true, n) => format!("{n} screens"),
                } };
                let label_w = text_width(ui, name, regular(T_BODY)).min(row.width() * 0.5);
                if !note.is_empty() {
                    let room = row.width() - 18.0 - label_w - 10.0 - 50.0;
                    if room > 40.0 {
                        let shown = fit_text(ui, &note, room);
                        ui.painter().text(pos2(row.left() + 18.0 + label_w + 10.0, row.center().y), Align2::LEFT_CENTER, shown, regular(T_META), if failed.is_some() { WARN } else { INK3 });
                    }
                }
                let switch_row = Rect::from_min_max(pos2(row.left() + 18.0, row.top()), row.max);
                let label = fit(ui.painter(), name, &regular(T_BODY), label_w);
                let response = switch(ui, switch_row, Id::unique(("wallpaper-computer", id)), &label, on, enabled && selected);
                let response = match &failed { Some(reason) => response.on_hover_text(reason), None => response };
                if response.clicked() {
                    self.wallpaper_command(Command::Computer { id: id.clone(), enabled: !on });
                }
            }

            let mut bottom = fills.bottom().max(play_on.bottom());
            let message = if status.is_none() || wallpaper.message.is_empty() || plan.is_some_and(|plan| plan.playing_message() == wallpaper.message) { String::new() } else { wallpaper.message.clone() };
            if !message.is_empty() {
                bottom += 14.0;
                bottom += para(ui, pos2(origin.x + 14.0, bottom), width - 28.0, &message, regular(T_BODY), INK2);
            }
            ui.allocate_space(vec2(width, bottom - origin.y + 8.0));
        });
    }
}

/// A list row's height.
const ROW: f32 = 46.0;
/// The room a group's title takes above it (as `pages::card`).
const GROUP_TITLE: f32 = 30.0;

/// A titled group holding `n` rows split by hairlines; the rows' rects.
fn list(ui: &Ui, r: Rect, title: &str, n: usize) -> Vec<Rect> {
    pages::card(ui, r, title);
    let p = ui.painter();
    let (left, right, top) = (r.left() + 16.0, r.right() - 16.0, r.top() + GROUP_TITLE + 4.0);
    (0..n.max(1)).map(|k| {
        let row = Rect::from_min_max(pos2(left, top + k as f32 * ROW), pos2(right, top + (k + 1) as f32 * ROW));
        if k + 1 < n {
            p.hline(row.x_range(), row.bottom(), Stroke::new(1.0, HAIRLINE));
        }
        row
    }).collect()
}

/// "0:42", "1:05:00": how long one loop is.
fn clock(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// The screens, where they stand, each showing its part of the picture,
/// named by computer under the first of each.
fn draw_canvas(ui: &Ui, area: Rect, canvas: &Canvas, media: Option<&Media>, fit: Fit, poster: Option<&egui::TextureHandle>, computers: &[(String, String, bool)]) {
    let Some(bounds) = canvas.bounds() else {
        ui.painter().text(area.center(), Align2::CENTER_CENTER, "Your screens appear here", regular(T_BODY), INK2);
        return;
    };
    let gap = 10.0;
    let scale = (area.width() as f64 / bounds.width).min(area.height() as f64 / bounds.height);
    let top = area.center() - vec2((bounds.width * scale) as f32, (bounds.height * scale) as f32) / 2.0;
    let map = |rect: one_mk_wallpaper::Rect| Rect::from_min_size(top + vec2(((rect.x - bounds.x) * scale) as f32, ((rect.y - bounds.y) * scale) as f32), vec2((rect.width * scale) as f32, (rect.height * scale) as f32));
    let fallback = poster.map(|texture| Media { width: texture.size()[0] as u32, height: texture.size()[1] as u32, duration_ms: 1, frame_rate: 0 }).unwrap_or_default();
    let image_rect = canvas.image_rect(media.unwrap_or(&fallback), fit).map(map);
    let p = ui.painter();
    let mut named = std::collections::BTreeSet::new();
    for monitor in &canvas.monitors {
        // A bezel's worth apart, so they read as separate screens.
        let screen = map(monitor.rect).shrink(gap / 2.0);
        if screen.width() < 4.0 || screen.height() < 4.0 {
            continue;
        }
        p.add(egui::epaint::Shadow { offset: [0, 14], blur: 30, spread: 0, color: Color32::from_black_alpha(130) }.as_shape(screen, CornerRadius::same(6)));
        p.add(widgets::rounded_gradient(screen, 6.0, Color32::from_rgb(0x1A, 0x1E, 0x26), Color32::from_rgb(0x10, 0x13, 0x18)));
        if let (Some(texture), Some(image_rect)) = (poster, image_rect) {
            // Each screen shows the part of the picture that falls on it.
            let picture = image_rect;
            let inside = screen.intersect(picture);
            if inside.is_positive() {
                let uv = Rect::from_min_max(
                    pos2((inside.left() - picture.left()) / picture.width(), (inside.top() - picture.top()) / picture.height()),
                    pos2((inside.right() - picture.left()) / picture.width(), (inside.bottom() - picture.top()) / picture.height()),
                );
                p.add(egui::Shape::Rect(egui::epaint::RectShape::filled(inside, CornerRadius::same(5), Color32::WHITE).with_texture(texture.id(), uv)));
            }
        }
        // Glass on the screen: a soft light from the top left.
        p.add(widgets::rounded_gradient(screen, 6.0, Color32::from_white_alpha(22), Color32::TRANSPARENT));
        if !monitor.online {
            p.rect_filled(screen, CornerRadius::same(6), Color32::from_black_alpha(150));
        }
        p.rect_stroke(screen, CornerRadius::same(6), Stroke::new(2.0, Color32::from_rgb(0x1B, 0x1F, 0x26)), StrokeKind::Outside);
        if named.insert(monitor.computer.clone()) {
            let name = computers.iter().find(|c| c.0 == monitor.computer).map_or(monitor.name.as_str(), |c| c.1.as_str());
            let name = fit_text(ui, name, screen.width() - 14.0);
            let at = screen.left_bottom() + vec2(8.0, -7.0);
            p.text(at + vec2(0.0, 1.0), Align2::LEFT_BOTTOM, &name, regular(T_META), Color32::from_black_alpha(160));
            p.text(at, Align2::LEFT_BOTTOM, name, regular(T_META), if monitor.online { Color32::WHITE } else { INK2 });
        }
    }
}

fn fit_text(ui: &Ui, text: &str, width: f32) -> String { fit(ui.painter(), text, &regular(T_META), width) }
