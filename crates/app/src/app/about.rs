//! Settings → General → About: oneMK itself (its keycap, wordmark, version
//! and licence), who makes it (Bigfoot Technology: its tile, name and line
//! and a button, all one link to its site) and the credits for what the
//! window draws and isn't ours. The palette's "About oneMK" opens it.

use super::*;
use one_mk_ctl::brand;

/// The 3D mouse's attribution, word for word (CC BY 4.0 asks for it).
pub(super) const MOUSE_CREDIT: &str = "The 3D mouse is “Logitech G502 x lightspeed” by Okopchi, CC BY 4.0, changed for oneMK.";
const MOUSE_URL: &str = "https://sketchfab.com/3d-models/logitech-g502-x-lightspeed-cda7107cc1444b4788d747f0361f7d40";
/// The fonts the window carries (their licences ship with it).
pub(super) const FONTS_CREDIT: &str = "Fonts: Inter, JetBrains Mono and Unbounded, under the SIL Open Font License 1.1.";
/// What a screen reader says for the maker's lockup.
pub(super) const MAKER_NAME: &str = "Bigfoot Technology, opens bigfootjo.com";
const VISIT: &str = "Visit bigfootjo.com";

/// The maker's tile, in points (its art comes at 1x, 2x and 3x of this).
const TILE: f32 = 44.0;
/// The tile's corner, as the mark draws it (23 of 80).
const TILE_R: f32 = TILE * 23.0 / 80.0;
const KEYCAP: f32 = 56.0;
const PAD: f32 = 20.0;
/// The maker's lockup: tall enough for the tile and room round it.
const LOCKUP_H: f32 = 64.0;
/// Below this width the button goes under the name.
const STACK_BELOW: f32 = 460.0;

/// "Version 1.0.0": the build's own version.
pub(super) fn version_words() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

/// The licence, as a person reads it: "MIT or Apache 2.0" for the SPDX
/// expression "MIT OR Apache-2.0".
pub(super) fn licence_words(spdx: &str) -> String {
    spdx.split(" OR ").map(|l| l.trim().replace("Apache-2.0", "Apache 2.0")).collect::<Vec<_>>().join(" or ")
}

/// "Free and open source · MIT or Apache 2.0".
pub(super) fn free_words() -> String {
    format!("Free and open source · {}", licence_words(env!("CARGO_PKG_LICENSE")))
}

/// The maker's white art for a tile `px` pixels square: the nearest one
/// drawn at least that big (1x, 2x or 3x), kept once loaded.
fn mark_texture(ctx: &egui::Context, px: f32) -> egui::TextureHandle {
    const MARKS: [(u32, &[u8]); 3] = [
        (44, include_bytes!("../../assets/bigfoot/mark-44.rgba")),
        (88, include_bytes!("../../assets/bigfoot/mark-88.rgba")),
        (132, include_bytes!("../../assets/bigfoot/mark-132.rgba")),
    ];
    let (size, bytes) = MARKS.iter().copied().find(|(s, _)| *s as f32 >= px - 0.5).unwrap_or(MARKS[2]);
    let id = Id::unique(("bigfoot-mark", size));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let ([w, h], rgba) = embedded(bytes);
    let t = ctx.load_texture("bigfoot-mark", egui::ColorImage::from_rgba_unmultiplied([w, h], rgba), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// The maker's tile at `min`: its own orange (in both themes) with the
/// white art over it, a soft shadow under it and a hairline ring round it,
/// so it sits on the card rather than on top of it.
fn paint_tile(ui: &Ui, min: Pos2, press: f32) {
    let p = ui.painter();
    let rect = Rect::from_min_size(min, vec2(TILE, TILE));
    let r = CornerRadius::same(TILE_R.round() as u8);
    let k = if pal().theme == crate::style::Theme::Light { 0.16 } else { 0.5 };
    p.add(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: pal().shadow_ink.gamma_multiply(k * (1.0 - 0.5 * press)) }.as_shape(rect, r));
    p.rect_filled(rect, r, crate::style::BIGFOOT_ORANGE);
    let tex = mark_texture(ui.ctx(), TILE * ui.ctx().pixels_per_point());
    p.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), crate::style::UNTINTED);
    p.rect_stroke(rect, r, Stroke::new(1.0, pal().line2), StrokeKind::Outside);
}

/// How the About card is laid out `w` wide: the heights of its three parts.
fn parts(ui: &Ui, w: f32) -> [f32; 3] {
    let head = PAD + KEYCAP + PAD;
    let maker = PAD + 18.0 + 10.0 + LOCKUP_H + if w < STACK_BELOW { 44.0 } else { 0.0 } + PAD - 6.0;
    let para = |text: &str| ui.painter().layout(text.to_owned(), regular(T_META), pal().ink2, w.max(1.0)).size().y;
    let credits = PAD + 18.0 + 10.0 + para(MOUSE_CREDIT) + 6.0 + para(FONTS_CREDIT) + PAD;
    [head, maker, credits]
}

impl App {
    /// The About card's height `w` wide.
    pub(super) fn about_h(&self, ui: &Ui, w: f32) -> f32 {
        parts(ui, w).iter().sum()
    }

    /// The About card, in `r` (the card's rows' room).
    pub(super) fn about_row(&mut self, ui: &mut Ui, r: Rect) {
        let [head_h, maker_h, _] = parts(ui, r.width());
        let p = ui.painter().clone();

        // oneMK: the keycap, the wordmark, the version and the licence.
        let top = r.top() + PAD;
        crate::icons::symbol(&p, pos2(r.left(), top), KEYCAP);
        let x = r.left() + KEYCAP + 18.0;
        let mut job = egui::text::LayoutJob::default();
        job.append("oneMK", 0.0, egui::TextFormat { font_id: wordmark(24.0), color: pal().ink, extra_letter_spacing: -0.6, ..Default::default() });
        let mark = p.layout_job(job);
        p.galley(pos2(x, top - 1.0), mark, pal().ink);
        p.text(pos2(x, top + 38.0), Align2::LEFT_CENTER, version_words(), mono(T_META), pal().ink2);
        let free = free_words();
        p.text(pos2(x, top + 55.0), Align2::LEFT_CENTER, fit(&p, &free, &regular(T_META), r.right() - x), regular(T_META), pal().ink2);
        a11y_text(ui, Id::unique("about-onemk"), Rect::from_min_size(r.min, vec2(r.width(), head_h)), Role::Label, &format!("oneMK. {}. {free}.", version_words()), None);

        // Made by: tile, name, line and button, one link to the maker's site.
        let y = r.top() + head_h;
        widgets::hairline(&p, r.x_range(), y);
        p.text(pos2(r.left(), y + PAD + 9.0), Align2::LEFT_CENTER, "Made by", semibold(T_META), pal().ink2);
        let stacked = r.width() < STACK_BELOW;
        let lockup = Rect::from_min_size(pos2(r.left(), y + PAD + 28.0), vec2(r.width(), LOCKUP_H + if stacked { 44.0 } else { 0.0 }));
        let hit = lockup.expand2(vec2(10.0, 0.0));
        let id = Id::unique("about-maker");
        let resp = ui.interact(hit, id, widgets::sense(ui, Sense::click()));
        let hover = tween(ui.ctx(), id.with("hover"), f32::from(resp.hovered()), 0.12, motion::out_cubic);
        let press = tween(ui.ctx(), id.with("press"), f32::from(resp.is_pointer_button_down_on()), motion::fade(0.06), motion::out_cubic);
        // The whole lockup answers the pointer: a quiet wash, deeper pressed.
        if hover > 0.0 || press > 0.0 {
            p.rect_filled(hit, CornerRadius::same(R_INNER), pal().hover.gamma_multiply((hover + press).min(1.5)));
        }
        let tile_at = pos2(lockup.left(), lockup.top() + (LOCKUP_H - TILE) / 2.0);
        paint_tile(ui, tile_at, press);
        let tx = tile_at.x + TILE + 16.0;
        let bw = button_width(ui, VISIT);
        let room = if stacked { lockup.right() - tx } else { lockup.right() - tx - bw - 16.0 };
        let mid = tile_at.y + TILE / 2.0;
        p.text(pos2(tx, mid - 10.0), Align2::LEFT_CENTER, fit(&p, brand::MAKER, &semibold(T_BODY + 1.0), room), semibold(T_BODY + 1.0), pal().ink);
        p.text(pos2(tx, mid + 10.0), Align2::LEFT_CENTER, fit(&p, brand::MAKER_LINE, &regular(T_BODY), room), regular(T_BODY), pal().ink2);
        let button = if stacked {
            Rect::from_min_size(pos2(tx, lockup.top() + LOCKUP_H + 4.0), vec2(bw, 32.0))
        } else {
            Rect::from_min_size(pos2(lockup.right() - bw, mid - 16.0), vec2(bw, 32.0))
        };
        widgets::paint_button(&p, button, VISIT, Look::Quiet, widgets::State { hover, press, enabled: true, focus: false });
        widgets::focus_ring(ui, &resp, hit, R_INNER);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        widgets::a11y(&resp, Role::Link, MAKER_NAME, None);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        widgets::targets::note("about maker", hit);
        // Enter or Space on it (the keyboard's click) opens it too.
        if resp.on_hover_text(brand::MAKER_URL).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(brand::MAKER_URL));
        }

        // Credits: what the window draws and isn't ours.
        let y = r.top() + head_h + maker_h;
        widgets::hairline(&p, r.x_range(), y);
        p.text(pos2(r.left(), y + PAD + 9.0), Align2::LEFT_CENTER, "Credits", semibold(T_META), pal().ink2);
        let cy = y + PAD + 28.0;
        let galley = p.layout(MOUSE_CREDIT.to_owned(), regular(T_META), pal().ink2, r.width());
        let credit = Rect::from_min_size(pos2(r.left(), cy), galley.size());
        let link = ui.interact(widgets::hit_rect(credit), Id::unique("credit-mouse"), widgets::sense(ui, Sense::click()));
        let ink = if link.hovered() { pal().ink } else { pal().ink2 };
        p.galley(credit.min, p.layout(MOUSE_CREDIT.to_owned(), regular(T_META), ink, r.width()), ink);
        if link.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            p.hline(egui::Rangef::new(credit.left(), credit.right()), credit.bottom() + 1.0, Stroke::new(1.0, pal().line2));
        }
        widgets::focus_ring(ui, &link, credit.expand(3.0), 6);
        widgets::a11y(&link, Role::Link, "Credit: open the 3D mouse model's page", None);
        if link.on_hover_text("Open the model's page").clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(MOUSE_URL));
        }
        let fy = credit.bottom() + 6.0;
        let fonts = para(ui, pos2(r.left(), fy), r.width(), FONTS_CREDIT, regular(T_META), pal().ink2);
        a11y_text(ui, Id::unique("credit-fonts"), Rect::from_min_size(pos2(r.left(), fy), vec2(r.width(), fonts)), Role::Label, FONTS_CREDIT, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_says_this_builds_version_and_licence() {
        assert_eq!(version_words(), format!("Version {}", env!("CARGO_PKG_VERSION")));
        assert!(version_words().starts_with("Version ") && version_words().split('.').count() >= 3);
        assert_eq!(licence_words("MIT OR Apache-2.0"), "MIT or Apache 2.0");
        assert_eq!(licence_words("MIT"), "MIT");
        assert_eq!(free_words(), "Free and open source · MIT or Apache 2.0");
    }

    #[test]
    fn the_maker_links_to_its_site() {
        assert_eq!(brand::MAKER_URL, "https://bigfootjo.com/");
        assert_eq!(MAKER_NAME, "Bigfoot Technology, opens bigfootjo.com");
        assert_eq!(VISIT, format!("Visit {}", brand::MAKER_SITE));
    }

    #[test]
    fn the_credits_stay_word_for_word() {
        assert_eq!(MOUSE_CREDIT, "The 3D mouse is “Logitech G502 x lightspeed” by Okopchi, CC BY 4.0, changed for oneMK.");
        assert!(MOUSE_URL.starts_with("https://sketchfab.com/3d-models/logitech-g502-x-lightspeed-"));
        // The fonts named are the ones the window carries, with their licence.
        for font in ["Inter", "JetBrains Mono", "Unbounded"] {
            assert!(FONTS_CREDIT.contains(font));
        }
        let ofl = include_str!("../../fonts/OFL-Inter.txt");
        assert!(ofl.contains("SIL Open Font License, Version 1.1") && FONTS_CREDIT.contains("SIL Open Font License 1.1"));
    }

    #[test]
    fn the_makers_art_comes_at_one_two_and_three_times() {
        for (px, bytes) in [(44, &include_bytes!("../../assets/bigfoot/mark-44.rgba")[..]), (88, include_bytes!("../../assets/bigfoot/mark-88.rgba")), (132, include_bytes!("../../assets/bigfoot/mark-132.rgba"))] {
            let ([w, h], rgba) = embedded(bytes);
            assert_eq!((w, h), (px, px));
            assert_eq!(rgba.len(), px * px * 4);
            assert_eq!(px as f32, TILE * (px / 44) as f32);
        }
    }
}
