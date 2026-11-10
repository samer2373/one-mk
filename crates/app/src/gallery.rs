//! Development aid, debug builds only: `ONE_MK_APP_GALLERY=1` draws every
//! shared control (widgets.rs) in every state, the light theme on the left
//! and the dark one on the right, over the whole window, so a screenshot
//! shows them all against the handoff's components (section 6).

use crate::icons::{self, Icon};
use crate::style::{self, Palette, mono, pal, regular, semibold, T_BODY, T_META};
use crate::widgets::{self, Look, Mark, State, Switch};
use egui::{Align2, CornerRadius, Painter, Rect, Ui, pos2, vec2};

pub fn wanted() -> bool {
    std::env::var_os("ONE_MK_APP_GALLERY").is_some()
}

/// Paints the gallery over everything, then puts the window's own palette back.
pub fn show(ui: &Ui) {
    let full = ui.ctx().content_rect();
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("widget-gallery")));
    let was = pal();
    let half = full.width() / 2.0;
    for (k, palette) in [Palette::light(), Palette::dark()].into_iter().enumerate() {
        style::set_palette(palette, None);
        let col = Rect::from_min_size(pos2(full.left() + k as f32 * half, full.top()), vec2(half, full.height()));
        p.rect_filled(col, 0.0, pal().bg);
        column(&p, col.shrink2(vec2(24.0, 18.0)), if k == 0 { "Light" } else { "Dark" });
    }
    style::set_palette(was, None);
}

const REST: State = State::REST;
const HOVER: State = State { hover: 1.0, ..REST };
const PRESSED: State = State { hover: 1.0, press: 1.0, ..REST };
const OFF: State = State { enabled: false, ..REST };
const FOCUS: State = State { focus: true, ..REST };
const STATES: [(&str, State); 5] = [("Rest", REST), ("Hover", HOVER), ("Pressed", PRESSED), ("Disabled", OFF), ("Focus", FOCUS)];

fn label(p: &Painter, at: egui::Pos2, text: &str) {
    p.text(at, Align2::LEFT_CENTER, text, semibold(T_BODY), pal().ink2);
}

fn column(p: &Painter, r: Rect, title: &str) {
    let mut y = r.top() + 10.0;
    p.text(pos2(r.left(), y), Align2::LEFT_CENTER, title, semibold(16.0), pal().ink);
    y += 26.0;
    let x0 = r.left();
    let cell = ((r.width() - 70.0) / 5.0).min(118.0);

    // Buttons: four looks across five states, 36 tall.
    for (k, (name, _)) in STATES.iter().enumerate() {
        p.text(pos2(x0 + 70.0 + k as f32 * cell + 4.0, y), Align2::LEFT_CENTER, *name, regular(T_META), pal().ink3);
    }
    y += 16.0;
    for (look, word) in [(Look::Primary, "Primary"), (Look::Quiet, "Secondary"), (Look::Text, "Text"), (Look::Danger, "Danger"), (Look::Destructive, "Destructive")] {
        p.text(pos2(x0, y + 18.0), Align2::LEFT_CENTER, word, regular(T_META), pal().ink2);
        for (k, (_, s)) in STATES.iter().enumerate() {
            let b = Rect::from_min_size(pos2(x0 + 70.0 + k as f32 * cell, y), vec2(cell - 10.0, 36.0));
            widgets::paint_button(p, b, "Pair", look, *s);
        }
        y += 44.0;
    }
    // Sizes: 38, 36, 34, 30.
    p.text(pos2(x0, y + 19.0), Align2::LEFT_CENTER, "Heights", regular(T_META), pal().ink2);
    let mut x = x0 + 70.0;
    for h in [38.0, 36.0, 34.0, 30.0] {
        let b = Rect::from_min_size(pos2(x, y + (38.0 - h) / 2.0), vec2(cell - 10.0, h));
        widgets::paint_button(p, b, &format!("{h:.0}"), if h >= 36.0 { Look::Primary } else { Look::Quiet }, REST);
        x += cell;
    }
    y += 50.0;

    // Segmented: the chosen capsule, a hovered cell, disabled.
    label(p, pos2(x0, y), "Segmented");
    y += 16.0;
    let seg = |p: &Painter, track: Rect, chosen: usize, hovered: Option<usize>, enabled: bool| {
        p.rect_filled(track, CornerRadius::same(10), pal().surface2);
        let cells = widgets::Pills::even(track, 4, 3.0);
        for (k, (c, word)) in cells.iter().zip(["Left", "Right", "Above", "Below"]).enumerate() {
            if k == chosen {
                p.add(egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: pal().shadow(60) }.as_shape(*c, CornerRadius::same(7)));
                p.rect_filled(*c, CornerRadius::same(7), pal().seg_on);
            } else if hovered == Some(k) {
                p.rect_filled(*c, CornerRadius::same(7), pal().surface2_hover);
            }
            let ink = if !enabled { pal().ink3 } else if k == chosen { pal().ink } else if hovered == Some(k) { pal().ink_hover } else { pal().ink2 };
            p.text(c.center(), Align2::CENTER_CENTER, word, if k == chosen { semibold(T_BODY) } else { regular(T_BODY) }, ink);
        }
    };
    let w = (r.width() - 12.0) / 2.0;
    seg(p, Rect::from_min_size(pos2(x0, y), vec2(w, 34.0)), 0, Some(2), true);
    seg(p, Rect::from_min_size(pos2(x0 + w + 12.0, y), vec2(w, 34.0)), 1, None, false);
    y += 48.0;

    // Switches: off and on, hovered, pressed, disabled, focused.
    label(p, pos2(x0, y), "Switches");
    y += 18.0;
    let mut x = x0;
    for (sw, s) in [
        (Switch::at(false), REST), (Switch::at(true), REST), (Switch::at(false), HOVER), (Switch::at(true), HOVER),
        (Switch::at(true), PRESSED), (Switch::at(false), OFF), (Switch::at(true), OFF), (Switch::at(true), FOCUS),
    ] {
        widgets::paint_switch(p, Rect::from_min_size(pos2(x, y), widgets::SWITCH), sw, s);
        x += 52.0;
    }
    y += 38.0;

    // A card of rows: page rows (52) with hairlines, not after the last.
    label(p, pos2(x0, y), "Card and rows");
    y += 16.0;
    let card = Rect::from_min_size(pos2(x0, y), vec2(r.width(), 3.0 * widgets::ROW_PAGE));
    for s in widgets::group_shapes(card, style::R_GROUP) {
        p.add(s);
    }
    let rows = [("Start at login", None, None, Some(true)), ("Keep the pointer here", Some("Edges lead nowhere, for games"), None, Some(false)), ("Held means after", None, Some("200 ms"), None)];
    for (k, (name, caption, value, switch)) in rows.into_iter().enumerate() {
        let row = Rect::from_min_size(pos2(card.left() + 18.0, card.top() + k as f32 * widgets::ROW_PAGE), vec2(card.width() - 36.0, widgets::ROW_PAGE));
        widgets::paint_row(p, if switch.is_some() { row.with_max_x(row.right() - 50.0) } else { row }, name, caption, value);
        if let Some(on) = switch {
            widgets::paint_switch(p, Rect::from_min_size(pos2(row.right() - 36.0, row.center().y - 11.0), widgets::SWITCH), Switch::at(on), REST);
        }
        if k < 2 {
            widgets::hairline(p, row.x_range(), row.bottom());
        }
    }
    y = card.bottom() + 18.0;

    // Chips, mono chips, status marks.
    label(p, pos2(x0, y), "Chips · keys · status");
    y += 16.0;
    let mut x = x0;
    for (word, on, s) in [("Caps Lock as Esc/Ctrl", true, REST), ("Swap Alt and Super", false, REST), ("Hovered", false, HOVER), ("Off", false, OFF), ("Focus", true, FOCUS)] {
        let cw = p.layout_no_wrap(word.to_owned(), regular(T_BODY), pal().ink).size().x + 28.0;
        if x + cw > r.right() {
            break;
        }
        widgets::paint_chip(p, Rect::from_min_size(pos2(x, y), vec2(cw, 30.0)), word, on, s);
        x += cw + 8.0;
    }
    y += 42.0;
    let mut x = x0;
    for key in ["Ctrl K", "Esc", "Ctrl+Alt+P", "200 ms"] {
        x = widgets::kbd(p, pos2(x, y + 10.0), Align2::LEFT_CENTER, key).right() + 10.0;
    }
    for (mark, word) in [(Mark::Ok, "online"), (Mark::Warn, "needs you"), (Mark::Off, "off"), (Mark::Error, "error")] {
        widgets::status_mark(p, pos2(x + 8.0, y + 10.0), 8.0, mark);
        let g = p.layout_no_wrap(word.to_owned(), regular(T_META), pal().ink2);
        let gw = g.size().x;
        p.galley(pos2(x + 18.0, y + 10.0 - g.size().y / 2.0), g, pal().ink2);
        x += 30.0 + gw;
    }
    y += 34.0;

    // Progress: going, stalled, size unknown.
    label(p, pos2(x0, y), "Progress");
    y += 16.0;
    let pw = (r.width() - 24.0) / 3.0;
    widgets::paint_progress(p, Rect::from_min_size(pos2(x0, y), vec2(pw, 5.0)), Some(0.45), false);
    widgets::paint_progress(p, Rect::from_min_size(pos2(x0 + pw + 12.0, y), vec2(pw, 5.0)), Some(0.7), true);
    widgets::paint_progress(p, Rect::from_min_size(pos2(x0 + 2.0 * (pw + 12.0), y), vec2(pw, 5.0)), None, false);
    p.text(pos2(x0, y + 16.0), Align2::LEFT_CENTER, "45% · 12.3 of 27.1 MB · 1 s left", mono(T_META), pal().ink2);
    y += 34.0;

    // Fields: rest (placeholder), hovered, focused, disabled, filled.
    label(p, pos2(x0, y), "Fields");
    y += 16.0;
    let fw = (r.width() - 12.0) / 2.0;
    for (k, (hint, text, hover, focused, enabled)) in [
        ("Type here · nothing is sent", "", false, false, true), ("Hovered", "", true, false, true),
        ("", "Focused", false, true, true), ("Disabled", "", false, false, false),
    ].into_iter().enumerate() {
        let f = Rect::from_min_size(pos2(x0 + (k % 2) as f32 * (fw + 12.0), y + (k / 2) as f32 * 46.0), vec2(fw, 38.0));
        let fr = widgets::paint_field(p, f, hover, focused, enabled);
        if focused {
            widgets::paint_focus(p, f, fr.nw);
        }
        let (t, col) = if text.is_empty() { (hint, pal().ink3) } else { (text, pal().ink) };
        p.text(f.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, t, regular(T_BODY), if enabled { col } else { pal().ink3 });
    }
    y += 2.0 * 46.0 + 6.0;

    // Icons: ink2 at rest, ink when active.
    label(p, pos2(x0, y), "Icons");
    y += 18.0;
    let mut x = x0;
    for (k, icon) in [Icon::Desk, Icon::Arrange, Icon::Wallpaper, Icon::Clipboard, Icon::Keys, Icon::Health, Icon::Settings, Icon::Lock, Icon::Send].into_iter().enumerate() {
        let active = k == 0;
        if active {
            p.rect_filled(Rect::from_min_size(pos2(x - 6.0, y - 6.0), vec2(28.0, 28.0)), CornerRadius::same(8), pal().surface2);
        }
        icons::draw(p, icon, Rect::from_min_size(pos2(x, y), vec2(16.0, 16.0)), if active { pal().ink } else { pal().ink2 });
        x += 34.0;
    }
}
