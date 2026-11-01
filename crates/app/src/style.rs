//! Fonts, colours and the type scale: Geist for words, Geist Mono for
//! numbers, dark neutral surfaces and one warm signal colour. Every colour
//! and size the window uses is named here.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Stroke};
use std::sync::Arc;

pub const GROUND: Color32 = Color32::from_rgb(0x0F, 0x12, 0x16);
/// The room the desk stands in: the window's ground behind everything.
pub const ROOM: Color32 = Color32::from_rgb(0x10, 0x13, 0x18);
/// The room on a see-through window (macOS): a dark tint over the blurred
/// desktop, 88% so grey text keeps 4.5:1 even over a white wallpaper.
pub const ROOM_TINT: Color32 = Color32::from_rgba_premultiplied(14, 17, 21, 224);
/// Glass, frosted: over its blurred backdrop, a little white (more at the
/// top, as if lit from above) over a little dark, so a panel reads lighter
/// than the room yet keeps text at 4.5:1 over the brightest backdrop.
/// `GLASS_TOP` at the top, `GLASS` at the bottom (premultiplied).
pub const GLASS_TOP: Color32 = Color32::from_rgba_premultiplied(24, 25, 27, 113);
pub const GLASS: Color32 = Color32::from_rgba_premultiplied(13, 14, 16, 107);
/// The same on a see-through window, where the room under the panel is
/// already a dark tint: less dark, the same frost.
pub const GLASS_CLEAR_TOP: Color32 = Color32::from_rgba_premultiplied(22, 23, 24, 71);
pub const GLASS_CLEAR: Color32 = Color32::from_rgba_premultiplied(11, 11, 12, 62);
/// Without blur (old OpenGL, or reduce transparency): solid.
pub const GLASS_SOLID_TOP: Color32 = Color32::from_rgb(0x1C, 0x20, 0x27);
pub const GLASS_SOLID: Color32 = Color32::from_rgb(0x16, 0x1A, 0x20);
/// A control's glass (a button, the chosen segment): clearer than a
/// panel's, a little lighter.
pub const CONTROL_TOP: Color32 = Color32::from_rgba_premultiplied(40, 41, 44, 70);
pub const CONTROL: Color32 = Color32::from_rgba_premultiplied(22, 23, 25, 52);
pub const CONTROL_SOLID_TOP: Color32 = Color32::from_rgb(0x2A, 0x2E, 0x35);
pub const CONTROL_SOLID: Color32 = Color32::from_rgb(0x22, 0x26, 0x2C);
/// The edge of a glass shape where the light doesn't catch it.
pub const GLASS_HAIRLINE: Color32 = Color32::from_rgba_premultiplied(12, 12, 12, 12);
/// A glass panel's rim, and its lit top edge (drawn when there is no blur;
/// with blur the glass lights its own rim).
pub const GLASS_RIM: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);
pub const GLASS_EDGE: Color32 = Color32::from_rgba_premultiplied(56, 56, 56, 56);
/// What floats over everything (a sheet, a toast, a menu): solid, so it
/// reads the same whatever is under it and while it moves.
pub const POPOVER: Color32 = Color32::from_rgb(0x1A, 0x1E, 0x25);
/// A control or a chosen row on glass: a little lighter than the glass.
pub const ON_GLASS: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
pub const ON_GLASS_LIT: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
/// Corners: panels, things inside them, small controls.
pub const R_PANEL: u8 = 20;
pub const R_INNER: u8 = 12;
/// A group of rows on glass (settings).
pub const R_GROUP: u8 = 16;
/// The gap round and between panels.
pub const M: f32 = 12.0;
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x17, 0x1C);
/// Wells and fields on glass: a touch lighter than what is under them.
pub const RAISED: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);
/// Hairlines: the ink at 7%, so they sit on glass and on solid alike.
pub const LINE: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
pub const INK: Color32 = Color32::from_rgb(0xED, 0xEF, 0xF2);
pub const INK2: Color32 = Color32::from_rgb(0x9D, 0xA6, 0xB1);
/// Labels and quiet numbers on glass: 4.5:1 over any backdrop.
pub const INK3: Color32 = Color32::from_rgb(0x8F, 0x98, 0xA4);
/// Between INK2 and INK: hovered labels.
pub const INK_HOVER: Color32 = Color32::from_rgb(0xC4, 0xCA, 0xD2);
/// The one warm colour: something crossing, a drop target, the main action.
pub const SIGNAL: Color32 = Color32::from_rgb(0xFF, 0xB5, 0x47);
/// The accent for chosen and on states (a switch that is on). The signal
/// for now; the brand's own colour when the new mark lands.
pub const ACCENT: Color32 = SIGNAL;
pub const ACCENT_LIT: Color32 = SIGNAL_LIT;
pub const SIGNAL_LIT: Color32 = Color32::from_rgb(0xFF, 0xC2, 0x66);
pub const SIGNAL_INK: Color32 = Color32::from_rgb(0x1E, 0x15, 0x05);
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x5E);
/// Connected: a dot, never words alone.
pub const OK: Color32 = Color32::from_rgb(0x5B, 0xC9, 0x8F);
/// Quiet buttons, and the pill under the chosen tab or segment.
pub const BUTTON: Color32 = Color32::from_rgba_premultiplied(22, 22, 22, 22);
pub const BUTTON_LIT: Color32 = Color32::from_rgba_premultiplied(34, 34, 34, 34);
pub const PILL: Color32 = Color32::from_rgba_premultiplied(32, 32, 32, 32);
/// A switch that is off: a visible track with a 3:1 edge.
pub const TRACK_OFF: Color32 = Color32::from_rgb(0x4A, 0x52, 0x5E);
pub const TRACK_EDGE: Color32 = Color32::from_rgb(0x70, 0x7A, 0x86);
/// The outline of a drawn key or mouse button: 3:1 on the panel and on
/// the key's own fill.
pub const KEY_EDGE: Color32 = Color32::from_rgb(0x69, 0x73, 0x80);
/// Things that are off or offline, still 3:1 on the panel.
pub const OFF: Color32 = Color32::from_rgb(0x70, 0x7A, 0x86);

/// Panel titles and the sheet title.
pub const T_TITLE: f32 = 15.0;
/// Body, row labels, tabs, buttons.
pub const T_BODY: f32 = 13.0;
/// Chips, keys, system names, values.
pub const T_META: f32 = 12.0;
/// Paragraph line height, as a multiple of the size.
pub const LEADING: f32 = 1.45;

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
/// Heavier mono, for the pairing code.
pub fn mono_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("mono-medium".into()))
}

/// `text`, shortened with an ellipsis to fit `max_w` points.
pub fn fit(painter: &Painter, text: &str, font: &FontId, max_w: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_owned(), font.clone(), INK).size().x;
    if width(text) <= max_w {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |n: usize| format!("{}…", chars[..n].iter().collect::<String>().trim_end());
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(&cut(mid)) <= max_w { lo = mid } else { hi = mid - 1 }
    }
    // Never end on a combining mark that lost its letter.
    while lo > 0 && is_combining(chars[lo]) {
        lo -= 1;
    }
    cut(lo)
}

fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F | 0x200D | 0xFE0F)
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts(&[]));
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = GROUND;
        v.window_fill = POPOVER;
        v.extreme_bg_color = PANEL;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.selection.bg_fill = SIGNAL;
        v.selection.stroke = Stroke::new(1.0, SIGNAL_INK);
        v.override_text_color = Some(INK);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, INK2);
        v.widgets.inactive.bg_fill = PILL;
        v.widgets.hovered.bg_fill = PILL;
        v.widgets.hovered.weak_bg_fill = PILL;
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, INK_HOVER);
        v.widgets.active.bg_fill = TRACK_EDGE;
        v.widgets.active.fg_stroke = Stroke::new(1.0, INK);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        // Thin scroll bars that float over the list, in the panel's greys.
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        style.spacing.scroll.bar_width = 6.0;
        // A list with more below shows its handle even at rest.
        style.spacing.scroll.dormant_handle_opacity = 0.6;
    });
}

fn fonts(extra: &[(String, Vec<u8>)]) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("geist", &include_bytes!("../fonts/Geist-Regular.ttf")[..]),
        ("geist-semibold", &include_bytes!("../fonts/Geist-SemiBold.ttf")[..]),
        ("geist-mono", &include_bytes!("../fonts/GeistMono-Regular.ttf")[..]),
        ("geist-mono-medium", &include_bytes!("../fonts/GeistMono-Medium.ttf")[..]),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // Geist first; egui's own fonts stay behind it for symbols, then the
    // system's fonts for scripts Geist doesn't cover.
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "geist".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "geist-mono".into());
    for (name, bytes) in extra {
        fonts.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes.clone())));
        fonts.families.entry(FontFamily::Proportional).or_default().push(name.clone());
    }
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.insert(FontFamily::Name("semibold".into()), [vec!["geist-semibold".into()], fallback.clone()].concat());
    fonts.families.insert(FontFamily::Name("mono-medium".into()), [vec!["geist-mono-medium".into()], fallback].concat());
    fonts
}

/// Names can be in any script. When one has letters no loaded font
/// covers, find a system font that does and add it behind Geist. Fonts
/// are only read when such a name shows up, so most people never pay.
#[derive(Default)]
pub struct Fallbacks {
    loaded: Vec<(String, Vec<u8>)>,
    tried: std::collections::HashSet<char>,
}

impl Fallbacks {
    pub fn cover<'a>(&mut self, ctx: &egui::Context, names: impl Iterator<Item = &'a str>) {
        let missing: Vec<char> = names
            .flat_map(str::chars)
            .filter(|c| !c.is_control() && !self.tried.contains(c))
            .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&regular(T_BODY), *c)))
            .collect();
        let mut added = false;
        for c in missing {
            if !self.tried.insert(c) {
                continue;
            }
            if let Some(path) = system_font_for(c)
                && !self.loaded.iter().any(|(n, _)| n == &path)
                && let Ok(bytes) = std::fs::read(&path)
            {
                self.loaded.push((path, bytes));
                added = true;
            }
        }
        if added {
            ctx.set_fonts(fonts(&self.loaded));
        }
    }
}

fn system_font_for(c: char) -> Option<String> {
    if cfg!(target_os = "linux") {
        let out = std::process::Command::new("fc-match").args(["-f", "%{file}", &format!(":charset={:x}", c as u32)]).output().ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        // Collections (.ttc) hold several faces; egui reads the first.
        return (out.status.success() && !path.is_empty()).then_some(path);
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Supplemental/Arial Unicode.ttf", "/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/AppleSDGothicNeo.ttc"]
    } else {
        &["C:\\Windows\\Fonts\\seguisym.ttf", "C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\Nirmala.ttc"]
    };
    candidates.iter().find(|p| std::path::Path::new(p).exists()).map(|p| (*p).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_shortened_to_fit() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let font = regular(T_BODY);
            assert_eq!(fit(&p, "desk", &font, 200.0), "desk");
            let long = "A very long computer name that will not fit anywhere at all";
            let short = fit(&p, long, &font, 120.0);
            assert!(short.ends_with('…') && short.len() < long.len(), "{short}");
            assert!(p.layout_no_wrap(short, font.clone(), INK).size().x <= 120.0);
            assert_eq!(fit(&p, long, &font, 1.0), "…");
        });
        out.textures_delta.clear();
    }
}
