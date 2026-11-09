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
        let status = self.running().cloned();
        let wallpaper = status.as_ref().and_then(|status| status.wallpaper.clone()).unwrap_or_default();
        // A drop is a choice like Choose…: only while choosing is possible.
        let dropped = ui.input(|input| input.raw.dropped_files.first().map(|file| file.path().to_path_buf()));
        if let Some(path) = dropped && accepts_drop(status.is_some(), wallpaper.available) { self.wallpaper_command(Command::Set { path }); }
        let plan = wallpaper.plan.as_ref();
        let canvas = plan.map(|plan| plan.canvas.clone()).unwrap_or_else(|| status.as_ref().map(one_mk_ctl::wallpaper::canvas).unwrap_or_default());
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        let poster_path = std::env::var_os("ONE_MK_APP_WALLPAPER").map(Into::into).or_else(|| wallpaper.poster.clone());
        #[cfg(not(any(debug_assertions, feature = "design-evidence")))]
        let poster_path = wallpaper.poster.clone();
        if self.wallpaper_ui.poster_path != poster_path {
            self.wallpaper_ui.poster = poster_path.as_ref().and_then(|path| {
                let image = image::ImageReader::open(path).ok()?.decode().ok()?.into_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                Some(ui.ctx().load_texture("wallpaper-poster", egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()), egui::TextureOptions::LINEAR))
            });
            self.wallpaper_ui.poster_path = poster_path;
        }
        let enabled = accepts_drop(status.is_some(), wallpaper.available);
        let selected = plan.is_some();
        let fit_now = plan.map_or(Fit::Fill, |plan| plan.fit);
        let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
        let computers: Vec<_> = status.as_ref().map(|status| std::iter::once((status.id.clone(), status.name.clone(), true))
            .chain(status.peers.iter().map(|peer| (peer.id.clone(), peer.name.clone(), peer.online))).collect()).unwrap_or_default();
        let out = egui::ScrollArea::vertical().id_salt("wallpaper-scroll").show_viewport(ui, |ui, _| {
            let width = area.width();
            let origin = ui.cursor().min;
            // The stage: every screen, as it stands, each showing its part
            // of the one picture; what plays and how to change it under it.
            // As tall as the screens need at this width, within reason.
            let need = canvas.bounds().map_or(180.0, |b| ((width - 48.0) as f64 * b.height / b.width) as f32);
            let screens_h = stage_screens_h(need, area.height(), wallpaper.gnome.is_some());
            let stage = Rect::from_min_size(origin, vec2(width, screens_h + 20.0 + 26.0 + 64.0));
            widgets::group(ui, stage, R_GROUP);
            if hovering && enabled {
                ui.painter().rect_stroke(stage, CornerRadius::same(R_GROUP), Stroke::new(1.5, pal().accent), StrokeKind::Inside);
            }
            let screens = Rect::from_min_max(stage.min + vec2(24.0, 26.0), stage.max - vec2(24.0, 84.0));
            draw_canvas(ui, screens, &canvas, plan.map(|plan| &plan.media), fit_now, self.wallpaper_ui.poster.as_ref(), &computers);
            a11y_text(ui, Id::unique("wallpaper-stage"), stage, Role::Image, &match plan { Some(plan) => plan.playing_summary(&wallpaper.computers), None => "No wallpaper playing".into() }, None);
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
                    p.rect(chip, CornerRadius::same(16), pal().surface, Stroke::new(1.0, pal().line2), StrokeKind::Inside);
                    p.text(chip.center(), Align2::CENTER_CENTER, shown, regular(T_META), pal().ink2);
                }
            } else {
                let hint = if status.is_none() { "Start oneMK to choose a wallpaper" } else if !wallpaper.available { "Wallpaper isn't available on this computer" } else { "Choose a video or picture" };
                ui.painter().text(pos2(x + 2.0, foot_y + 16.0), Align2::LEFT_CENTER, hint, regular(T_BODY), pal().ink2);
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
            // The choices give way before the label does: in a narrow column
            // the label keeps the room it needs (its name, the "i" and a margin).
            let fit_w = fit_track_w(fit_row.width(), text_width(ui, "Picture sizing", regular(T_BODY)) + 26.0 + 8.0);
            label_help(ui, short(fit_row), Id::unique("wallpaper-fit-label"), "Picture sizing", "wallpaper-fit", enabled && selected, fit_w + 12.0);
            let track = Rect::from_min_size(pos2(fit_row.right() - fit_w, fit_row.center().y - 14.0), vec2(fit_w, 28.0));
            if let Some(chosen) = (Pills { id: Id::unique("wallpaper-fit"), track, cells: Pills::even(track, 3, 2.0), labels: &["Fill", "Fit", "Stretch"], names: vec!["Fill the screens".into(), "Fit inside the screens".into(), "Stretch across the screens".into()], chosen: Some(match fit_now { Fit::Fill => 0, Fit::Fit => 1, Fit::Stretch => 2 }), enabled: vec![enabled && selected; 3], role: Role::RadioGroup, font: T_META }).show(ui, true) {
                self.wallpaper_command(Command::Fit { fit: [Fit::Fill, Fit::Fit, Fit::Stretch][chosen] });
            }
            let battery = plan.is_none_or(|plan| plan.pause_on_battery);
            if switch_help(ui, short(rows[1]), Id::unique("wallpaper-battery"), "Pause on battery", "wallpaper-battery", battery, enabled && selected && plan.is_none_or(|plan| !plan.picture)).clicked() {
                self.wallpaper_command(Command::Battery { enabled: !battery });
            }

            let n = computers.len().max(1);
            let at = if two { pos2(fills.right() + M * 1.4, fills.top()) } else { pos2(origin.x, fills.bottom() + 22.0) };
            let play_on = Rect::from_min_size(at, vec2(col_w, GROUP_TITLE + 8.0 + n as f32 * ROW));
            let rows = list(ui, play_on, "Play on", n);
            if computers.is_empty() {
                ui.painter().text(rows[0].left_center(), Align2::LEFT_CENTER, "Your computers appear here", regular(T_BODY), pal().ink2);
            }
            for ((id, name, online), row) in computers.iter().zip(rows) {
                let p = ui.painter();
                let dot = pos2(row.left() + 4.0, row.center().y);
                if *online { widgets::status_mark(p, dot, 7.0, widgets::Mark::Ok); } else { widgets::status_mark(p, dot, 7.0, widgets::Mark::Off); }
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
                        ui.painter().text(pos2(row.left() + 18.0 + label_w + 10.0, row.center().y), Align2::LEFT_CENTER, shown, regular(T_META), if failed.is_some() { pal().warn } else { pal().ink3 });
                    }
                }
                let switch_row = Rect::from_min_max(pos2(row.left() + 18.0, row.top()), row.max);
                let label = fit(ui.painter(), name, &regular(T_BODY), label_w);
                let response = switch_help(ui, short(switch_row), Id::unique(("wallpaper-computer", id)), &label, "wallpaper-computer", on, enabled && selected);
                let response = match &failed { Some(reason) => response.on_hover_text(reason), None => response };
                if response.clicked() {
                    self.wallpaper_command(Command::Computer { id: id.clone(), enabled: !on });
                }
            }

            let mut bottom = fills.bottom().max(play_on.bottom());
            // A row only says "can't play it here" (the sentence hides in a
            // hover): other computers' reasons are written out in full, with
            // the computer's name, so what to do about it can be read.
            for (id, name, online) in &computers {
                if let Some(one_mk_wallpaper::Playback::Error(reason)) = wallpaper.computers.get(id)
                    && *online && plan.is_none_or(|plan| plan.computers.contains(id)) && reason != &wallpaper.message {
                    bottom += 14.0;
                    bottom += para(ui, pos2(origin.x + 14.0, bottom), width - 28.0, &format!("{name}: {reason}"), regular(T_BODY), pal().warn);
                }
            }
            if let Some(setup) = &wallpaper.gnome {
                bottom += 22.0;
                ui.painter().text(pos2(origin.x + 14.0, bottom), Align2::LEFT_TOP, "GNOME video setup", regular(T_BODY), pal().ink);
                bottom += 28.0;
                bottom += para(ui, pos2(origin.x + 14.0, bottom), width - 28.0, &setup.message, regular(T_BODY), pal().ink2);
                let action = if setup.can_install { Some((one_mk_wallpaper::ExtensionAction::Install, "Install GNOME extension")) }
                    else if setup.can_enable { Some((one_mk_wallpaper::ExtensionAction::Enable, "Switch on GNOME extension")) } else { None };
                if let Some((action, label)) = action {
                    bottom += 12.0;
                    let bounds = Rect::from_min_size(pos2(origin.x + 14.0, bottom), vec2(230.0_f32.min(width - 28.0), 32.0));
                    if button(ui, bounds, Id::unique("gnome-extension-setup"), label, Look::Glass, status.is_some(), None).clicked() {
                        self.wallpaper_command(Command::GnomeExtension { operation: action });
                    }
                    bottom = bounds.bottom();
                }
            }
            let message = if status.is_none() || wallpaper.message.is_empty() || plan.is_some_and(|plan| [plan.playing_message(), plan.playing_summary(&wallpaper.computers)].contains(&wallpaper.message)) { String::new() } else { wallpaper.message.clone() };
            if !message.is_empty() {
                bottom += 14.0;
                bottom += para(ui, pos2(origin.x + 14.0, bottom), width - 28.0, &message, regular(T_BODY), pal().ink2);
            }
            // What isn't proven on this computer yet is said, never hidden.
            if let Some(note) = wallpaper.experimental.as_ref().filter(|_| status.is_some()) {
                bottom += 14.0;
                bottom += para(ui, pos2(origin.x + 14.0, bottom), width - 28.0, &format!("Experimental: {note}"), regular(T_META), pal().ink2);
            }
            ui.allocate_space(vec2(width, bottom - origin.y + 8.0));
        });
        // More below the fold: a fade says so.
        let hidden = out.content_size.y - out.state.clamped_offset().y - out.inner_rect.height();
        if hidden > 4.0 {
            more_below(ui, out.inner_rect);
        }
    }
}

/// The height the screens' preview gets: as tall as the screens need at this
/// width, within reason; in a short window with the GNOME setup block under
/// it, no taller than lets the setup show without scrolling far.
fn stage_screens_h(need: f32, page_h: f32, gnome: bool) -> f32 {
    let wanted = need.clamp(150.0, 330.0);
    if !gnome {
        return wanted;
    }
    // The stage's own chrome (110), the two lists (22 + 130), the setup (about 150), a margin.
    let room = page_h - (110.0 + 22.0 + 130.0 + 150.0 + 8.0);
    wanted.min(room.max(110.0))
}

/// How wide the Fill, Fit, Stretch choices are in a row `row_w` wide whose
/// label needs `label_w`: all of 204 where it fits, narrower down to 150.
fn fit_track_w(row_w: f32, label_w: f32) -> f32 {
    (row_w - label_w - 12.0).clamp(150.0, 204.0)
}

/// A soft fade over the foot of a scrolling page that goes on below, with a
/// chevron, so what is cut off reads as "scroll" and not as "missing".
fn more_below(ui: &Ui, inner: Rect) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let fade = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 40.0), inner.right_bottom());
    let mut mesh = Mesh::default();
    for (pos, color) in [(fade.left_top(), Color32::TRANSPARENT), (fade.right_top(), Color32::TRANSPARENT), (fade.right_bottom(), pal().bg), (fade.left_bottom(), pal().bg)] {
        mesh.vertices.push(Vertex { pos, uv: WHITE_UV, color });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    let painter = ui.painter().with_clip_rect(inner);
    painter.add(egui::Shape::mesh(mesh));
    let c = pos2(inner.center().x, inner.bottom() - 12.0);
    painter.add(egui::Shape::line(vec![c + vec2(-5.0, -2.5), c + vec2(0.0, 2.5), c + vec2(5.0, -2.5)], Stroke::new(1.5, pal().ink3)));
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn the_picture_sizing_label_keeps_its_room() {
        // The 820 pt window's column: the label (about 90 + 26) is not squeezed to "Picture s…".
        let label = 116.0;
        let narrow = 278.0;
        let w = fit_track_w(narrow, label);
        assert!(narrow - w - 12.0 >= label, "label got {} of {label}", narrow - w - 12.0);
        assert!(w >= 150.0);
        assert_eq!(fit_track_w(520.0, label), 204.0, "a wide column keeps the full choices");
    }

    #[test]
    fn a_short_window_trims_the_preview_only_when_the_setup_block_is_there() {
        assert_eq!(stage_screens_h(183.0, 480.0, false), 183.0);
        assert!(stage_screens_h(183.0, 480.0, true) < 183.0);
        assert_eq!(stage_screens_h(183.0, 480.0, true), 110.0, "never below 110");
        assert_eq!(stage_screens_h(183.0, 900.0, true), 183.0, "a tall window keeps it");
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
            p.hline(row.x_range(), row.bottom(), Stroke::new(1.0, pal().line));
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
        ui.painter().text(area.center(), Align2::CENTER_CENTER, "Your screens appear here", regular(T_BODY), pal().ink2);
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
        p.add(egui::epaint::Shadow { offset: [0, 14], blur: 30, spread: 0, color: pal().shadow(130) }.as_shape(screen, CornerRadius::same(6)));
        p.rect_filled(screen, CornerRadius::same(6), BEZEL);
        if let (Some(texture), Some(image_rect)) = (poster, image_rect) {
            // Each screen shows the part of the picture that falls on it.
            let picture = image_rect;
            let inside = screen.intersect(picture);
            if inside.is_positive() {
                let uv = Rect::from_min_max(
                    pos2((inside.left() - picture.left()) / picture.width(), (inside.top() - picture.top()) / picture.height()),
                    pos2((inside.right() - picture.left()) / picture.width(), (inside.bottom() - picture.top()) / picture.height()),
                );
                p.add(egui::Shape::Rect(egui::epaint::RectShape::filled(inside, CornerRadius::same(5), UNTINTED).with_texture(texture.id(), uv)));
            }
        }
        // Glass on the screen: a soft light from the top left.
        p.add(widgets::rounded_gradient(screen, 6.0, white(22), Color32::TRANSPARENT));
        if !monitor.online {
            p.rect_filled(screen, CornerRadius::same(6), black(150));
        }
        p.rect_stroke(screen, CornerRadius::same(6), Stroke::new(2.0, BEZEL), StrokeKind::Outside);
        if named.insert(monitor.computer.clone()) {
            let name = computers.iter().find(|c| c.0 == monitor.computer).map_or(monitor.name.as_str(), |c| c.1.as_str());
            let name = fit_text(ui, name, screen.width() - 14.0);
            let at = screen.left_bottom() + vec2(8.0, -7.0);
            p.text(at + vec2(0.0, 1.0), Align2::LEFT_BOTTOM, &name, regular(T_META), black(160));
            p.text(at, Align2::LEFT_BOTTOM, name, regular(T_META), if monitor.online { white(255) } else { SCREEN_INK2 });
        }
    }
}

fn fit_text(ui: &Ui, text: &str, width: f32) -> String { fit(ui.painter(), text, &regular(T_META), width) }

/// Whether a video or picture can be chosen now (Choose…, or dropped on the
/// page): oneMK runs and wallpaper is available on this computer.
fn accepts_drop(running: bool, available: bool) -> bool { running && available }

/// These rows carry subtitles of their own, so they take the "i" without a caption.
fn short(row: Rect) -> Rect {
    Rect::from_center_size(row.center(), vec2(row.width(), 40.0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_drop_is_ignored_while_choosing_is_disabled() {
        assert!(super::accepts_drop(true, true));
        assert!(!super::accepts_drop(false, true));
        assert!(!super::accepts_drop(true, false));
    }
}
