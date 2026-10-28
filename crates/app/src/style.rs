//! Fonts and colours: Geist for words, Geist Mono for numbers, dark neutral
//! surfaces and one warm signal colour.

use crate::app::{GROUND, INK, INK2, LINE, PANEL, RAISED};
use crate::scene::SIGNAL;
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};
use std::sync::Arc;

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("geist", &include_bytes!("../fonts/Geist-Regular.ttf")[..]),
        ("geist-semibold", &include_bytes!("../fonts/Geist-SemiBold.ttf")[..]),
        ("geist-mono", &include_bytes!("../fonts/GeistMono-Regular.ttf")[..]),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // Geist first; egui's own fonts stay behind it for symbols and other scripts.
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "geist".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "geist-mono".into());
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.insert(FontFamily::Name("semibold".into()), [vec!["geist-semibold".into()], fallback].concat());
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = GROUND;
        v.window_fill = RAISED;
        v.extreme_bg_color = PANEL;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.selection.bg_fill = SIGNAL;
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x1E, 0x15, 0x05));
        v.override_text_color = Some(INK);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, INK2);
        v.widgets.hovered.bg_fill = Color32::from_rgb(0x26, 0x2C, 0x36);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x26, 0x2C, 0x36);
        v.widgets.active.bg_fill = SIGNAL;
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
}
