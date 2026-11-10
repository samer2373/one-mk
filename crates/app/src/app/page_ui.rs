//! The layout shared by the pages made of cards (Arrange, Wallpaper,
//! Clipboard & files), as the handoff draws them (5.2 to 5.4, section 6):
//! a title (22, 600) and a sentence (13, `ink2`), then cards in two columns
//! (one when the window is compact), each under its group label (13, 600,
//! `ink2`), on a `surface` card of radius 14 with 52-point rows split by
//! hairlines. A row has an optional status mark, a label, a caption, a value
//! and a switch or a button.
//!
//! The page is laid out top down from `origin` and painted as it goes: a
//! card's background is reserved first and filled in once the rows have said
//! how tall it is. The caller allocates `Page::height` at the end, so the
//! page scrolls.

use super::*;
use crate::widgets::{Mark, State};
use egui::epaint::Shape;
use egui::layers::ShapeIdx;
use egui::{FontId, Response};

/// What the page keeps clear on each side, on top of the window's own 16.
pub(super) const INSET: f32 = 16.0;
/// Between columns, and between a card and the group label under it.
pub(super) const GAP: f32 = 20.0;
/// A card's inner padding, left and right.
pub(super) const CARD_PAD: f32 = 18.0;
/// A row on a page.
pub(super) const ROW: f32 = widgets::ROW_PAGE;
/// The room a group label takes above its card.
const LABEL_H: f32 = 30.0;
/// How far a mark pushes a row's words in.
pub(super) const MARK_INDENT: f32 = 21.0;
/// Pages go to one column below this window width (the handoff's compact).
fn compact(ui: &Ui) -> bool {
    ui.ctx().content_rect().width() < shell::COMPACT_BELOW
}

/// The widest a page of cards gets: a wider window centres it (calm
/// margins, rows a hand's reach long) rather than stretching every row.
pub(super) const MAX_W: f32 = 1160.0;
/// The page title's size: a clear step above the group labels and rows.
/// (At least the type scale's page title, should that grow.)
pub(super) const TITLE: f32 = T_PAGE.max(26.0);
/// The page's sentence under its title.
pub(super) const SUBTITLE: f32 = 14.0;

/// Where a page's content sits across `width` from `origin_x`: its left
/// edge and width, inset on both sides and centred once it reaches `max_w`.
pub(super) fn frame_x(origin_x: f32, width: f32, max_w: f32) -> (f32, f32) {
    let room = (width - 2.0 * INSET).max(200.0);
    let w = room.min(max_w);
    ((origin_x + INSET + (room - w) / 2.0).round(), w)
}

/// A page's head (handoff 5.2): the title (26, 600) and its sentence (14,
/// `ink2`) from `origin`, where the cards start. Returns where what comes
/// under it starts.
pub(super) fn head(ui: &Ui, origin: Pos2, width: f32, title: &str, sub: &str) -> f32 {
    let (x, w) = frame_x(origin.x, width, MAX_W);
    let p = ui.painter();
    // A short window gives the head less room (the handoff's 22 title).
    let tight = ui.ctx().content_rect().height() < 700.0;
    let title_font = semibold(if tight { T_PAGE } else { TITLE });
    p.text(pos2(x, origin.y + if tight { 32.0 } else { 40.0 }), Align2::LEFT_CENTER, fit(p, title, &title_font, w), title_font, pal().ink);
    let sub_top = origin.y + if tight { 49.0 } else { 60.0 };
    let sub_h = if sub.is_empty() { 0.0 } else { para(ui, pos2(x, sub_top), w.min(760.0), sub, regular(SUBTITLE), pal().ink2).max(20.0) };
    a11y_text(ui, Id::unique(("page-title", title)), Rect::from_min_size(pos2(x, origin.y + 22.0), vec2(w, 36.0)), Role::Heading, title, None);
    if !sub.is_empty() {
        a11y_text(ui, Id::unique(("page-sub", title)), Rect::from_min_size(pos2(x, sub_top), vec2(w, sub_h)), Role::Label, sub, None);
    }
    sub_top + sub_h + if tight { 16.0 } else { 22.0 }
}

/// A page being laid out.
pub(super) struct Page {
    pub x: f32,
    pub w: f32,
    pub col_w: f32,
    /// The right column's width (the left's, unless [`Page::split`]).
    right_w: f32,
    pub two: bool,
    origin_y: f32,
    /// Where the next group label of each column goes.
    y: [f32; 2],
}

impl Page {
    /// Starts a page at `origin` (the top left of the scrolling content),
    /// `width` wide, and draws its title and sentence.
    pub fn begin(ui: &Ui, origin: Pos2, width: f32, title: &str, sub: &str) -> Page {
        let (x, w) = frame_x(origin.x, width, MAX_W);
        let two = !compact(ui);
        let top = head(ui, origin, width, title, sub);
        let col_w = if two { (w - GAP) / 2.0 } else { w };
        Page { x, w, col_w, right_w: col_w, two, origin_y: origin.y, y: [top, top] }
    }

    /// The two columns in the proportion `left` : 1 (Health's 1.2 : 1)
    /// instead of halves; one column stays the page's width.
    pub fn split(&mut self, left: f32) {
        if self.two {
            let unit = (self.w - GAP) / (left + 1.0);
            (self.col_w, self.right_w) = (unit * left, unit);
        }
    }

    /// How wide a card in column `col` is.
    pub fn width_of(&self, col: usize) -> f32 {
        if col.min(usize::from(self.two)) == 1 { self.right_w } else { self.col_w }
    }

    pub fn col_x(&self, col: usize) -> f32 {
        self.x + col.min(usize::from(self.two)) as f32 * (self.col_w + GAP)
    }

    /// The top of the next thing at full width.
    pub fn below(&self) -> f32 {
        self.y[0].max(self.y[1])
    }

    /// Moves both columns down to `y` (after something drawn across both).
    pub fn skip_to(&mut self, y: f32) {
        self.y = [self.y[0].max(y), self.y[1].max(y)];
    }

    /// A card in column `col` (0 when the page has one column), under `label`.
    pub fn card(&mut self, ui: &Ui, col: usize, label: &str) -> Card {
        let col = col.min(usize::from(self.two));
        let (x, y) = (self.col_x(col), self.y[col]);
        Card::start(ui, x, y, if col == 1 { self.right_w } else { self.col_w }, label, Some(col))
    }

    /// A card across the whole page, under every card above it.
    pub fn wide(&mut self, ui: &Ui, label: &str) -> Card {
        Card::start(ui, self.x, self.below(), self.w, label, None)
    }

    /// Closes a card: draws its surface and moves its column down.
    pub fn finish(&mut self, ui: &Ui, card: Card) -> Rect {
        let rect = Rect::from_min_max(pos2(card.x, card.top), pos2(card.x + card.w, card.y.max(card.top + 4.0)));
        ui.painter().set(card.idx, Shape::Vec(widgets::group_shapes(rect, R_GROUP)));
        match card.col {
            Some(col) => self.y[col] = rect.bottom() + GAP,
            None => self.y = [rect.bottom() + GAP; 2],
        }
        rect
    }

    /// Plain text under the cards of a column (a note or an explanation),
    /// at the card's own inset; moves the column down.
    pub fn note(&mut self, ui: &Ui, col: Option<usize>, text: &str, font: FontId, colour: Color32) -> f32 {
        let (x, y, w) = match col {
            Some(c) => (self.col_x(c), self.y[c.min(usize::from(self.two))], self.col_w),
            None => (self.x, self.below(), self.w),
        };
        let h = para(ui, pos2(x + 4.0, y - 6.0), w - 8.0, text, font, colour);
        let bottom = y - 6.0 + h + 12.0;
        match col {
            Some(c) => self.y[c.min(usize::from(self.two))] = bottom,
            None => self.y = [bottom; 2],
        }
        h
    }

    /// The scrolling content's height, from where the page began.
    pub fn height(&self) -> f32 {
        self.below() - GAP + 28.0 - self.origin_y
    }
}

/// A card being filled with rows.
pub(super) struct Card {
    idx: ShapeIdx,
    col: Option<usize>,
    pub x: f32,
    pub w: f32,
    /// The card's top (under its label).
    top: f32,
    /// Where the next row starts.
    y: f32,
    rows: usize,
}

impl Card {
    fn start(ui: &Ui, x: f32, y: f32, w: f32, label: &str, col: Option<usize>) -> Card {
        let p = ui.painter();
        if !label.is_empty() {
            p.text(pos2(x, y + 10.0), Align2::LEFT_CENTER, fit(p, label, &semibold(T_BODY), w), semibold(T_BODY), pal().ink2);
            a11y_text(ui, Id::unique(("card-label", label)), Rect::from_min_size(pos2(x, y), vec2(w, LABEL_H)), Role::Heading, label, None);
        }
        let top = y + if label.is_empty() { 0.0 } else { LABEL_H };
        Card { idx: p.add(Shape::Noop), col, x, w, top, y: top, rows: 0 }
    }

    /// The label's right end, to put a quiet action or a status beside it.
    pub fn label_right(&self) -> Pos2 {
        pos2(self.x + self.w, self.top - LABEL_H + 10.0)
    }

    /// The next row, `h` tall, inside the card's padding; a hairline above it
    /// unless it is the first.
    pub fn row(&mut self, ui: &Ui, h: f32) -> Rect {
        if self.rows > 0 {
            widgets::hairline(ui.painter(), egui::Rangef::new(self.x + CARD_PAD, self.x + self.w - CARD_PAD), self.y - 0.5);
        }
        self.rows += 1;
        let r = Rect::from_min_size(pos2(self.x + CARD_PAD, self.y), vec2(self.w - 2.0 * CARD_PAD, h));
        self.y += h;
        r
    }

    /// More room in the same row: continues without a hairline.
    pub fn more(&mut self, h: f32) -> Rect {
        let r = Rect::from_min_size(pos2(self.x + CARD_PAD, self.y), vec2(self.w - 2.0 * CARD_PAD, h));
        self.y += h;
        r
    }

}

/// How tall a card's row is when it stands in for a list that is empty.
pub(super) const EMPTY_ROW: f32 = 64.0;

/// A row that stands in for an empty list: a quiet round tile with a
/// dashed screen in it, and the sentence that says what fills the list.
pub(super) fn empty_row(ui: &Ui, row: Rect, text: &str) {
    let p = ui.painter();
    let c = pos2(row.left() + 16.0, row.center().y);
    p.circle_filled(c, 16.0, pal().surface2);
    let glyph = Rect::from_center_size(c, vec2(14.0, 10.0));
    let corners = vec![glyph.left_top(), glyph.right_top(), glyph.right_bottom(), glyph.left_bottom(), glyph.left_top()];
    p.extend(Shape::dashed_line(&corners, Stroke::new(1.25, pal().ink3), 2.0, 1.5));
    p.line_segment([pos2(c.x - 3.0, glyph.bottom() + 2.5), pos2(c.x + 3.0, glyph.bottom() + 2.5)], Stroke::new(1.25, pal().ink3));
    let x = row.left() + 44.0;
    p.text(pos2(x, row.center().y), Align2::LEFT_CENTER, fit(p, text, &regular(T_BODY), row.right() - x), regular(T_BODY), pal().ink2);
}

/// A page's scrolling area (`ONE_MK_APP_SCROLL=points` scrolls it in a
/// debug build, for screenshots of what is below the fold).
pub(super) fn scroll(id: &str) -> egui::ScrollArea {
    let area = egui::ScrollArea::vertical().id_salt(id.to_owned()).auto_shrink([false, false]);
    #[cfg(any(debug_assertions, feature = "design-evidence"))]
    let area = std::env::var("ONE_MK_APP_SCROLL").ok().and_then(|value| value.parse::<f32>().ok()).map_or(area.clone(), |offset| area.vertical_scroll_offset(offset));
    area
}

/// What a caption under a label looks like.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Cap {
    Plain,
    /// Numbers: JetBrains Mono.
    Mono,
    /// Something needs the person: mono in `warn`.
    WarnMono,
}

/// A row's words: its label, a caption under it, an optional mark.
pub(super) struct Words<'a> {
    pub label: &'a str,
    pub caption: Option<&'a str>,
    pub cap: Cap,
    pub mark: Option<Mark>,
    pub strong: bool,
    /// The label sits high even without a one-line caption (the caller
    /// paints a longer one under it).
    pub raised: bool,
    /// The label is quieter (a control that can't be used now).
    pub dim: bool,
}

impl<'a> Words<'a> {
    pub fn new(label: &'a str) -> Words<'a> {
        Words { label, caption: None, cap: Cap::Plain, mark: None, strong: false, raised: false, dim: false }
    }
    pub fn caption(mut self, caption: &'a str) -> Self {
        self.caption = Some(caption);
        self
    }
    pub fn caption_opt(mut self, caption: Option<&'a str>) -> Self {
        self.caption = caption.filter(|c| !c.is_empty());
        self
    }
    pub fn cap(mut self, cap: Cap) -> Self {
        self.cap = cap;
        self
    }
    pub fn mark(mut self, mark: Mark) -> Self {
        self.mark = Some(mark);
        self
    }
    pub fn raised(mut self) -> Self {
        self.raised = true;
        self
    }
    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }
}

/// Where the words of a row start, and how much room they have, when the
/// right `reserved` points belong to a control.
pub(super) fn words_box(row: Rect, mark: bool, reserved: f32) -> (f32, f32) {
    let indent = if mark { MARK_INDENT } else { 0.0 };
    (row.left() + indent, (row.width() - indent - reserved).max(20.0))
}

/// Paints a row's words (a one-line caption, as in the handoff) and its
/// mark; names the row for screen readers when `name` is set. Returns how
/// wide the label came out.
pub(super) fn paint_words(ui: &Ui, row: Rect, words: &Words, reserved: f32) -> f32 {
    let p = ui.painter();
    let (x, room) = words_box(row, words.mark.is_some(), reserved);
    if let Some(mark) = words.mark {
        widgets::status_mark(p, pos2(row.left() + 5.0, row.center().y), 8.0, mark);
    }
    let font = if words.strong { semibold(T_BODY) } else { regular(T_BODY) };
    let shown = fit(p, words.label, &font, room);
    let label_y = if words.caption.is_some() || words.raised { row.top() + 16.0 } else { row.center().y };
    let rect = p.text(pos2(x, label_y), Align2::LEFT_CENTER, &shown, font, if words.dim { pal().ink2 } else { pal().ink });
    if shown != words.label {
        ui.interact(rect, Id::unique(("row-name", words.label)), Sense::hover()).on_hover_text(words.label);
    }
    if let Some(caption) = words.caption {
        let (font, colour) = match words.cap {
            Cap::Plain => (regular(T_META), pal().ink2),
            Cap::Mono => (mono(T_META), pal().ink2),
            Cap::WarnMono => (mono(T_META), pal().warn),
        };
        p.text(pos2(x, row.top() + 36.0), Align2::LEFT_CENTER, fit(p, caption, &font, room), font, colour);
    }
    rect.width()
}

/// A value at the right of a row (13, `ink2`; mono for numbers; the link
/// colour for the one that is "in use"). Returns its width.
pub(super) fn value(ui: &Ui, row: Rect, at_right: f32, text: &str, mono_text: bool, link: bool) -> f32 {
    let p = ui.painter();
    let font = if link { semibold(T_BODY) } else if mono_text { mono(T_BODY) } else { regular(T_BODY) };
    let colour = if link { pal().accent_text } else if mono_text { pal().ink } else { pal().ink2 };
    let rect = p.text(pos2(row.right() - at_right, row.center().y), Align2::RIGHT_CENTER, text, font, colour);
    rect.width()
}

/// A button at the right end of a row, 28 tall (the row's own size).
#[allow(clippy::too_many_arguments)]
pub(super) fn row_button(ui: &mut Ui, row: Rect, at_right: f32, id: Id, label: &str, look: Look, enabled: bool, name: Option<&str>) -> Response {
    let w = button_width(ui, label);
    let r = Rect::from_min_size(pos2(row.right() - at_right - w, row.center().y - 14.0), vec2(w, 28.0));
    button(ui, r, id, label, look, enabled, name)
}

/// A switch row: the words at the left (the caption from `help.rs`, and its
/// "i", when `key` is filed there and the row has no caption of its own),
/// the switch at the right.
pub(super) fn toggle_row(ui: &mut Ui, row: Rect, id: Id, words: &Words, key: Option<&str>, on: bool, enabled: bool) -> Response {
    if words.caption.is_none()
        && words.mark.is_none()
        && let Some(key) = key.filter(|key| crate::help::get(key).is_some())
    {
        return switch_help(ui, row, id, words.label, key, on, enabled);
    }
    let r = ui.interact(widgets::hit_rect(row), id, if enabled { widgets::sense(ui, Sense::click()) } else { Sense::hover() });
    let ctx = ui.ctx().clone();
    let press = tween(&ctx, id.with("press"), f32::from(enabled && r.is_pointer_button_down_on()), motion::fade(0.06), motion::out_cubic);
    let knob = tween(&ctx, id.with("knob"), f32::from(on), motion::space(0.26), motion::back_out);
    let fill = tween(&ctx, id.with("track"), f32::from(on), motion::fade(0.2), motion::linear);
    let hover = tween(&ctx, id.with("hover"), f32::from(enabled && r.hovered()), 0.12, motion::out_cubic);
    paint_words(ui, row, words, widgets::SWITCH.x + 12.0);
    let track = Rect::from_min_size(pos2(row.right() - widgets::SWITCH.x, row.center().y - widgets::SWITCH.y / 2.0), widgets::SWITCH);
    widgets::paint_switch(ui.painter(), track, widgets::Switch { on, knob, fill, undimmed: false }, State { hover, press, enabled, focus: r.has_focus() && widgets::keyboard_focus(&ctx) });
    if r.hovered() && enabled {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let name = words.label.to_owned();
    let can = enabled && r.enabled();
    r.widget_info(move || egui::WidgetInfo::selected(Role::Switch, can, on, &name));
    r
}

/// The height of `text` wrapped to `width` in the body face (for rows that
/// carry a sentence).
pub(super) fn text_h(ui: &Ui, text: &str, width: f32, font: FontId) -> f32 {
    ui.painter().layout(text.to_owned(), font, pal().ink2, width.max(1.0)).size().y
}

/// A banner's surface (handoff 4.3): `surface` on a hairline, radius 12;
/// `warn_soft` for one that needs the person.
pub(super) fn banner_frame(ui: &Ui, rect: Rect, attention: bool) {
    let p = ui.painter();
    let fill = if attention { pal().warn_soft } else { pal().surface };
    p.rect(rect, CornerRadius::same(12), fill, Stroke::new(1.0, pal().line), StrokeKind::Inside);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(width: f32, window: f32) -> (Page, Vec<Rect>) {
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut out = (None, vec![]);
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(window, 800.0))), ..Default::default() }, |ui| {
            let mut page = Page::begin(ui, pos2(16.0, 60.0), width, "Title", "A sentence.");
            let mut left = page.card(ui, 0, "Left");
            let mut rects = vec![left.row(ui, ROW), left.row(ui, ROW)];
            rects.push(page.finish(ui, left));
            let mut right = page.card(ui, 1, "Right");
            rects.push(right.row(ui, ROW));
            rects.push(page.finish(ui, right));
            out = (Some(page), rects);
        });
        output.textures_delta.clear();
        (out.0.unwrap(), out.1)
    }

    #[test]
    fn two_columns_split_the_page_and_one_column_stacks() {
        crate::style::set_palette(crate::style::Palette::light(), None);
        let (page, rects) = lay(1200.0 - 32.0, 1200.0);
        assert!(page.two);
        // Cards: 32 from each window edge, a 20 gap between columns.
        assert_eq!(rects[2].left(), 32.0);
        assert_eq!(rects[4].right(), 1168.0);
        assert!((rects[4].left() - rects[2].right() - GAP).abs() < 0.01);
        // Two rows of 52 and no padding above or below them.
        assert_eq!(rects[2].height(), 2.0 * ROW);
        assert_eq!(rects[4].height(), ROW);
        // Both columns start together, under the same sentence.
        assert_eq!(rects[2].top(), rects[4].top());
        let (page, rects) = lay(820.0 - 32.0, 820.0);
        assert!(!page.two);
        assert_eq!(rects[2].width(), 820.0 - 64.0);
        assert!(rects[4].top() > rects[2].bottom(), "stacked");
    }

    #[test]
    fn a_wide_window_centres_the_page_at_its_widest() {
        crate::style::set_palette(crate::style::Palette::light(), None);
        let (page, rects) = lay(1920.0 - 32.0, 1920.0);
        assert_eq!(page.w, MAX_W);
        let (left, right) = (rects[2].left(), 1920.0 - rects[4].right());
        assert!((left - right).abs() <= 1.0, "centred: {left} and {right}");
        assert_eq!(frame_x(16.0, 900.0, MAX_W), (32.0, 868.0), "narrower than the widest: inset only");
    }

    #[test]
    fn rows_are_split_by_hairlines_and_the_page_height_covers_the_cards() {
        crate::style::set_palette(crate::style::Palette::dark(), None);
        let (page, rects) = lay(1000.0, 1000.0);
        assert!(rects[0].bottom() - rects[0].top() == ROW);
        assert_eq!(rects[1].top(), rects[0].bottom());
        assert!(page.height() >= rects[2].bottom() - 60.0, "the scroll area reaches the last card");
    }
}
