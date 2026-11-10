use super::page_ui::{self, GAP, MARK_INDENT, Page, ROW, Words, text_h, toggle_row};
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
        let out = page_ui::scroll("wallpaper-scroll").show_viewport(ui, |ui, _| {
            let origin = ui.cursor().min;
            let mut page = Page::begin(ui, origin, area.width(), "Wallpaper", "One picture or video across every screen, like one wide monitor.");

            // The preview: every screen, as it stands, each showing its part
            // of the one picture; what plays and how to change it under it.
            // As tall as the screens need at this width, within reason.
            let need = canvas.bounds().map_or(180.0, |b| ((page.w - 2.0 * (page_ui::CARD_PAD + WELL_PAD.x)) as f64 * b.height / b.width) as f32);
            let screens_h = stage_screens_h(need, area.height(), wallpaper.gnome.is_some());
            let mut card = page.wide(ui, "");
            // The screens stand in a recessed well, inset as the rows are.
            let stage = card.more(page_ui::CARD_PAD + screens_h + 2.0 * WELL_PAD.y + 4.0);
            let well = Rect::from_min_max(pos2(card.x + page_ui::CARD_PAD, stage.top() + page_ui::CARD_PAD), pos2(card.x + card.w - page_ui::CARD_PAD, stage.bottom() - 4.0));
            preview::stage(ui.painter(), well, 12, 16.0);
            let screens = well.shrink2(WELL_PAD);
            draw_canvas(ui, screens, &canvas, plan.map(|plan| &plan.media), fit_now, self.wallpaper_ui.poster.as_ref(), &computers);
            a11y_text(ui, Id::unique("wallpaper-stage"), stage, Role::Image, &match plan { Some(plan) => plan.playing_summary(&wallpaper.computers), None => "No wallpaper playing".into() }, None);
            let foot = card.row(ui, 70.0);
            let mut right = 0.0;
            let mut button_at = |ui: &mut Ui, id: &str, label: &str, look: Look, on: bool| {
                let w = button_width(ui, label);
                let r = Rect::from_min_size(pos2(foot.right() - right - w, foot.center().y - 17.0), vec2(w, 34.0));
                right += w + 8.0;
                button(ui, r, Id::unique(id), label, look, on, None)
            };
            let choose = button_at(ui, "choose-video", "Choose…", Look::Primary, enabled && self.wallpaper_ui.picker.is_none());
            let mut pause = None;
            let mut turn_off = None;
            if let Some(plan) = plan {
                if !plan.picture {
                    pause = Some(button_at(ui, "wallpaper-play", if plan.paused { "Play" } else { "Pause" }, Look::Quiet, enabled));
                }
                turn_off = Some(button_at(ui, "wallpaper-off", "Turn off", Look::Quiet, status.is_some()));
            }
            if choose.clicked() {
                self.choose_video(ui.ctx());
            }
            if let (Some(plan), Some(pause)) = (plan, &pause) && pause.clicked() {
                self.wallpaper_command(Command::Pause { paused: !plan.paused });
            }
            if turn_off.is_some_and(|response| response.clicked()) {
                self.wallpaper_command(Command::Off);
            }
            let room = (foot.width() - right - 8.0).max(0.0);
            let p = ui.painter();
            if let Some(plan) = plan {
                let name = fit_text(ui, &plan.title, room * 0.6);
                let nw = p.text(pos2(foot.left(), foot.center().y), Align2::LEFT_CENTER, &name, medium(T_BODY), pal().ink).width();
                let what = if plan.paused { "paused".to_owned() } else { plan.playing_summary(&wallpaper.computers).to_lowercase() };
                let detail = if plan.picture { what } else { format!("{} loop · {what}", clock(plan.media.duration_ms)) };
                let detail = fit(p, &detail, &mono(T_META), room - nw - 12.0);
                p.text(pos2(foot.left() + nw + 12.0, foot.center().y), Align2::LEFT_CENTER, detail, mono(T_META), pal().ink2);
            } else {
                let hint = if status.is_none() { "Start oneMK to choose a wallpaper" } else if !wallpaper.available { "Wallpaper isn't available on this computer" } else { "Choose a video or picture" };
                // Can't be chosen here: said with the "off" mark, not only in words.
                let blocked = status.is_none() || !wallpaper.available;
                let x = foot.left() + if blocked { 18.0 } else { 0.0 };
                if blocked {
                    widgets::status_mark(p, pos2(foot.left() + 5.0, foot.center().y), 8.0, widgets::Mark::Off);
                }
                p.text(pos2(x, foot.center().y), Align2::LEFT_CENTER, fit(p, hint, &regular(T_BODY), room - (x - foot.left())), regular(T_BODY), if blocked { pal().ink2 } else { pal().ink });
            }
            let preview = page.finish(ui, card);
            if hovering && enabled {
                ui.painter().rect_stroke(preview, CornerRadius::same(R_GROUP), Stroke::new(1.5, pal().accent), StrokeKind::Inside);
            }

            // GNOME's video wallpaper needs its extension: one step, said as a
            // banner that needs the person, with the button that does it.
            if let Some(setup) = &wallpaper.gnome {
                let action = if setup.can_install { Some((one_mk_wallpaper::ExtensionAction::Install, "Install GNOME extension")) }
                    else if setup.can_enable { Some((one_mk_wallpaper::ExtensionAction::Enable, "Switch on GNOME extension")) } else { None };
                let button_w = action.map_or(0.0, |(_, label)| button_width(ui, label));
                let text_w = (page.w - 32.0 - if action.is_some() { button_w + 16.0 } else { 0.0 }).max(120.0);
                let message_h = text_h(ui, &setup.message, text_w, regular(T_BODY));
                let h = (16.0 + 20.0 + 4.0 + message_h + 16.0).max(if action.is_some() { 66.0 } else { 0.0 });
                let rect = Rect::from_min_size(pos2(page.x, page.below() - 4.0), vec2(page.w, h));
                page_ui::banner_frame(ui, rect, true);
                let p = ui.painter();
                p.text(rect.min + vec2(16.0, 26.0), Align2::LEFT_CENTER, "GNOME video setup", semibold(T_BODY), pal().ink);
                para(ui, rect.min + vec2(16.0, 40.0), text_w, &setup.message, regular(T_BODY), pal().ink2);
                a11y_text(ui, Id::unique("gnome-setup"), rect, Role::Status, &format!("GNOME video setup. {}", setup.message), None);
                if let Some((action, label)) = action {
                    let b = Rect::from_min_size(pos2(rect.right() - 16.0 - button_w, rect.center().y - 17.0), vec2(button_w, 34.0));
                    if button(ui, b, Id::unique("gnome-extension-setup"), label, Look::Quiet, status.is_some(), None).clicked() {
                        self.wallpaper_command(Command::GnomeExtension { operation: action });
                    }
                }
                page.skip_to(rect.bottom() + 16.0);
            }

            // How it fills the screens, and where it plays.
            let mut card = page.card(ui, 0, "How it fills");
            let row = card.row(ui, ROW);
            // The choices give way before the label does: in a narrow column
            // the label keeps the room it needs (its name, the "i" and a margin).
            let fit_w = fit_track_w(row.width(), text_width(ui, "Picture sizing", regular(T_BODY)) + 26.0 + 8.0);
            label_help(ui, row, Id::unique("wallpaper-fit-label"), "Picture sizing", "wallpaper-fit", enabled && selected, fit_w + 12.0);
            let track = Rect::from_min_size(pos2(row.right() - fit_w, row.center().y - 15.0), vec2(fit_w, 30.0));
            if let Some(chosen) = (Pills { id: Id::unique("wallpaper-fit"), track, cells: Pills::even(track, 3, 2.0), labels: &["Fill", "Fit", "Stretch"], names: vec!["Fill the screens".into(), "Fit inside the screens".into(), "Stretch across the screens".into()], chosen: Some(match fit_now { Fit::Fill => 0, Fit::Fit => 1, Fit::Stretch => 2 }), enabled: vec![enabled && selected; 3], role: Role::RadioGroup, font: T_META }).show(ui, true) {
                self.wallpaper_command(Command::Fit { fit: [Fit::Fill, Fit::Fit, Fit::Stretch][chosen] });
            }
            let battery = plan.is_none_or(|plan| plan.pause_on_battery);
            let row = card.row(ui, ROW);
            let can = enabled && selected && plan.is_none_or(|plan| !plan.picture);
            if toggle_row(ui, row, Id::unique("wallpaper-battery"), &Words::new("Pause on battery").dim(!can), Some("wallpaper-battery"), battery, can).clicked() {
                self.wallpaper_command(Command::Battery { enabled: !battery });
            }
            page.finish(ui, card);

            let mut card = page.card(ui, 1, "Play on");
            if computers.is_empty() {
                let row = card.row(ui, page_ui::EMPTY_ROW);
                page_ui::empty_row(ui, row, "Your computers appear here");
            }
            for (id, name, online) in &computers {
                let on = plan.is_none_or(|plan| plan.computers.contains(id));
                let screens = canvas.monitors.iter().filter(|monitor| &monitor.computer == id).count();
                // Why a computer can't play it is often a sentence: the row
                // says so briefly, the whole reason shows on hover.
                let failed = match wallpaper.computers.get(id) { Some(one_mk_wallpaper::Playback::Error(reason)) if *online && on => Some(reason.clone()), _ => None };
                let note = play_note(*online, on, screens, wallpaper.computers.get(id), failed.is_some());
                let row = card.row(ui, ROW);
                let mut words = Words::new(name).caption_opt(Some(note.as_str()));
                if failed.is_some() {
                    words = words.mark(widgets::Mark::Warn);
                }
                let response = toggle_row(ui, row, Id::unique(("wallpaper-computer", id)), &words, None, on, enabled && selected);
                let response = match &failed { Some(reason) => response.on_hover_text(reason), None => response };
                if response.clicked() {
                    self.wallpaper_command(Command::Computer { id: id.clone(), enabled: !on });
                }
            }
            page.finish(ui, card);

            // A row only says "can't play it here" (the sentence hides in a
            // hover): other computers' reasons are written out in full, with
            // the computer's name, so what to do about it can be read.
            for (id, name, online) in &computers {
                if let Some(one_mk_wallpaper::Playback::Error(reason)) = wallpaper.computers.get(id)
                    && *online && plan.is_none_or(|plan| plan.computers.contains(id)) && reason != &wallpaper.message {
                    let y = page.below() - GAP + 8.0;
                    let h = para(ui, pos2(page.x + MARK_INDENT - 2.0, y), page.w - MARK_INDENT, &format!("{name}: {reason}"), regular(T_BODY), pal().ink);
                    widgets::status_mark(ui.painter(), pos2(page.x + 5.0, y + 9.0), 8.0, widgets::Mark::Warn);
                    page.skip_to(y + h + 8.0 + GAP);
                }
            }
            let message = if status.is_none() || wallpaper.message.is_empty() || plan.is_some_and(|plan| [plan.playing_message(), plan.playing_summary(&wallpaper.computers)].contains(&wallpaper.message)) { String::new() } else { wallpaper.message.clone() };
            if !message.is_empty() {
                page.note(ui, None, &message, regular(T_BODY), pal().ink2);
            }
            // What isn't proven on this computer yet is said, never hidden.
            if let Some(note) = wallpaper.experimental.as_ref().filter(|_| status.is_some()) {
                page.note(ui, None, &format!("Experimental: {note}"), regular(T_META), pal().ink2);
            }
            ui.allocate_space(vec2(area.width(), page.height()));
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

/// What a computer's row in "Play on" says under its name.
fn play_note(online: bool, on: bool, screens: usize, playback: Option<&one_mk_wallpaper::Playback>, failed: bool) -> String {
    if failed {
        return "can't play it here".to_owned();
    }
    if online && on && let Some(playback) = playback {
        return playback.message().to_owned();
    }
    match (online, screens) {
        (false, _) => "Offline · its part is kept".to_owned(),
        (true, 0) => String::new(),
        (true, 1) => "1 screen".to_owned(),
        (true, n) => format!("{n} screens"),
    }
}

/// "0:42", "1:05:00": how long one loop is.
fn clock(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Between the well's rim and the screens in it.
const WELL_PAD: egui::Vec2 = egui::Vec2 { x: 24.0, y: 22.0 };

/// The screens, where they stand, each showing its part of the picture,
/// named by computer under the first of each.
fn draw_canvas(ui: &Ui, area: Rect, canvas: &Canvas, media: Option<&Media>, fit: Fit, poster: Option<&egui::TextureHandle>, computers: &[(String, String, bool)]) {
    let Some(bounds) = canvas.bounds() else {
        // No screens known yet: a ghost of one, dashed, over the sentence.
        let p = ui.painter();
        let ghost = Rect::from_center_size(area.center() - vec2(0.0, 16.0), vec2(150.0, 86.0));
        let r = 8.0;
        let mut path = vec![];
        for (c, from) in [(ghost.left_top() + vec2(r, r), 180.0_f32), (ghost.right_top() + vec2(-r, r), 270.0), (ghost.right_bottom() + vec2(-r, -r), 0.0), (ghost.left_bottom() + vec2(r, -r), 90.0)] {
            for k in 0..=4 {
                let a = (from + 22.5 * k as f32).to_radians();
                path.push(c + vec2(a.cos(), a.sin()) * r);
            }
        }
        path.push(path[0]);
        p.extend(egui::Shape::dashed_line(&path, Stroke::new(1.5, pal().line2), 6.0, 4.0));
        p.text(pos2(area.center().x, ghost.bottom() + 22.0), Align2::CENTER_CENTER, "Your screens appear here", regular(T_BODY), pal().ink2);
        return;
    };
    let gap = 8.0;
    let scale = (area.width() as f64 / bounds.width).min(area.height() as f64 / bounds.height);
    let top = area.center() - vec2((bounds.width * scale) as f32, (bounds.height * scale) as f32) / 2.0;
    let map = |rect: one_mk_wallpaper::Rect| Rect::from_min_size(top + vec2(((rect.x - bounds.x) * scale) as f32, ((rect.y - bounds.y) * scale) as f32), vec2((rect.width * scale) as f32, (rect.height * scale) as f32));
    let fallback = poster.map(|texture| Media { width: texture.size()[0] as u32, height: texture.size()[1] as u32, duration_ms: 1, frame_rate: 0 }).unwrap_or_default();
    let image_rect = canvas.image_rect(media.unwrap_or(&fallback), fit).map(map);
    let p = ui.painter();
    let mut named = std::collections::BTreeSet::new();
    for monitor in &canvas.monitors {
        // Apart by a hair, so they read as separate screens.
        let screen = map(monitor.rect).shrink(gap / 2.0);
        if screen.width() < 8.0 || screen.height() < 8.0 {
            continue;
        }
        // A graphite bezel round the glass, the same in both themes.
        let (glass, r) = preview::monitor(p, screen, 0.0);
        let radius = CornerRadius::same(r);
        let mut pictured = false;
        if let (Some(texture), Some(image_rect)) = (poster, image_rect) {
            // Each screen shows the part of the picture that falls on it.
            let picture = image_rect;
            let inside = glass.intersect(picture);
            if inside.is_positive() {
                let uv = Rect::from_min_max(
                    pos2((inside.left() - picture.left()) / picture.width(), (inside.top() - picture.top()) / picture.height()),
                    pos2((inside.right() - picture.left()) / picture.width(), (inside.bottom() - picture.top()) / picture.height()),
                );
                if inside != glass {
                    // Fit leaves bands the picture doesn't cover: blank glass there.
                    preview::blank_glass(p, glass, r);
                }
                p.add(egui::Shape::Rect(egui::epaint::RectShape::filled(inside, radius, UNTINTED).with_texture(texture.id(), uv)));
                pictured = true;
            }
        }
        if !pictured {
            preview::blank_glass(p, glass, r);
        }
        if !monitor.online {
            p.rect_filled(glass, radius, black(140));
        }
        preview::glass_finish(p, glass, r);
        if named.insert(monitor.computer.clone()) && glass.width() > 40.0 && glass.height() > 24.0 {
            let name = computers.iter().find(|c| c.0 == monitor.computer).map_or(monitor.name.as_str(), |c| c.1.as_str());
            let name = if monitor.online { name.to_owned() } else { format!("{name} · Offline") };
            let name = crate::style::fit(p, &name, &medium(T_META), glass.width() - 28.0);
            preview::name_tag(p, glass.left_bottom() + vec2(7.0, -7.0), &name, pictured || !monitor.online);
        }
    }
}

fn fit_text(ui: &Ui, text: &str, width: f32) -> String { fit(ui.painter(), text, &regular(T_META), width) }

/// Whether a video or picture can be chosen now (Choose…, or dropped on the
/// page): oneMK runs and wallpaper is available on this computer.
fn accepts_drop(running: bool, available: bool) -> bool { running && available }

#[cfg(test)]
mod tests {
    #[test]
    fn play_on_rows_say_what_each_computer_does() {
        use one_mk_wallpaper::Playback;
        assert_eq!(super::play_note(false, true, 1, None, false), "Offline · its part is kept");
        assert_eq!(super::play_note(true, true, 2, None, false), "2 screens");
        assert_eq!(super::play_note(true, false, 1, Some(&Playback::Ready), false), "1 screen", "switched off here: its screens, not 'playing'");
        assert_eq!(super::play_note(true, true, 1, Some(&Playback::Ready), false), "playing");
        assert_eq!(super::play_note(true, true, 1, Some(&Playback::Error("no decoder".into())), true), "can't play it here");
        assert_eq!(super::play_note(true, true, 0, None, false), "");
    }

    #[test]
    fn a_loop_is_told_in_minutes_and_hours() {
        assert_eq!(super::clock(10_000), "0:10");
        assert_eq!(super::clock(61_001), "1:02");
        assert_eq!(super::clock(3_600_000), "1:00:00");
    }

    #[test]
    fn a_drop_is_ignored_while_choosing_is_disabled() {
        assert!(super::accepts_drop(true, true));
        assert!(!super::accepts_drop(false, true));
        assert!(!super::accepts_drop(true, false));
    }
}
