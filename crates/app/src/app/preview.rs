//! The flat previews of screens that the Arrange plan and the Wallpaper
//! canvas share: the recessed ground they stand on (a quiet dot grid), a
//! plan tile (a soft gradient, a hairline outline, light along its top, a
//! shadow that grows as it lifts) and a monitor (a graphite bezel, its
//! glass, a sheen). Colours come from `style::preview_look`. Nothing here
//! moves by itself: what animates is passed in.

use super::*;
use crate::style::{PreviewLook, preview_look};
use egui::epaint::{Mesh, Shadow, Vertex, WHITE_UV};

/// The outline of a rounded rectangle, clockwise from the top left corner's
/// start, `per` points to a corner.
fn rounded_path(rect: Rect, r: f32, per: usize) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let corners = [
        (pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
        (pos2(rect.right() - r, rect.top() + r), 1.5 * std::f32::consts::PI),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 0.5 * std::f32::consts::PI),
    ];
    let mut out = Vec::with_capacity(4 * (per + 1));
    for (c, from) in corners {
        for k in 0..=per {
            let a = from + std::f32::consts::FRAC_PI_2 * k as f32 / per as f32;
            out.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    out
}

/// A rounded rectangle filled top to bottom from `top` to `bottom` (a fan
/// from its centre: the blend is exact, as it only depends on height).
/// A mesh has no feathered edge: [`smooth_gradient`] gives it one.
pub(super) fn gradient(rect: Rect, r: f32, top: Color32, bottom: Color32) -> egui::Shape {
    let at = |y: f32| mix(top, bottom, ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0));
    let mut mesh = Mesh::default();
    mesh.vertices.push(Vertex { pos: rect.center(), uv: WHITE_UV, color: at(rect.center().y) });
    let path = rounded_path(rect, r, 6);
    for p in &path {
        mesh.vertices.push(Vertex { pos: *p, uv: WHITE_UV, color: at(p.y) });
    }
    let n = path.len() as u32;
    for k in 0..n {
        mesh.indices.extend([0, 1 + k, 1 + (k + 1) % n]);
    }
    egui::Shape::mesh(mesh)
}

/// [`gradient`] with a clean edge: an anti-aliased rounded rectangle in the
/// middle colour first, the blend half a point inside it.
pub(super) fn smooth_gradient(p: &Painter, rect: Rect, r: u8, top: Color32, bottom: Color32) {
    p.rect_filled(rect, CornerRadius::same(r), mix(top, bottom, 0.5));
    p.add(gradient(rect.shrink(0.6), (f32::from(r) - 0.6).max(0.0), top, bottom));
}

/// A soft shadow under `rect`, `lift` (0 resting, 1 carried) deep.
pub(super) fn shadow(p: &Painter, rect: Rect, r: u8, lift: f32, look: &PreviewLook) {
    let lift = lift.clamp(0.0, 1.0);
    let near = Shadow { offset: [0, 1], blur: 3, spread: 0, color: look.shadow.gamma_multiply(0.9) };
    let far = Shadow { offset: [0, (3.0 + 7.0 * lift) as i8], blur: (8.0 + 16.0 * lift) as u8, spread: 0, color: look.shadow.gamma_multiply(0.8 + 0.7 * lift) };
    p.add(near.as_shape(rect, CornerRadius::same(r)));
    p.add(far.as_shape(rect, CornerRadius::same(r)));
}

/// The ground the screens stand on: a recessed well with a rim and a dot
/// grid every `step` points, `r` round.
pub(super) fn stage(p: &Painter, rect: Rect, r: u8, step: f32) {
    let look = preview_look();
    p.rect(rect, CornerRadius::same(r), look.stage, Stroke::new(1.0, look.stage_edge), StrokeKind::Inside);
    let inner = rect.shrink(step * 0.75);
    let (cols, rows) = ((inner.width() / step).floor() as i32, (inner.height() / step).floor() as i32);
    let x0 = rect.center().x - cols as f32 * step / 2.0;
    let y0 = rect.center().y - rows as f32 * step / 2.0;
    for j in 0..=rows {
        for i in 0..=cols {
            let c = pos2((x0 + i as f32 * step).round() + 0.5, (y0 + j as f32 * step).round() + 0.5);
            p.circle_filled(c, 0.9, look.stage_dot);
        }
    }
}

/// How a plan tile is shown.
#[derive(Clone, Copy, Default)]
pub(super) struct TileState {
    pub chosen: bool,
    /// 0..1, tweened by the caller.
    pub hover: f32,
    /// 0..1: being carried (it lifts).
    pub lift: f32,
    pub online: bool,
}

/// A screen on the Arrange plan: a soft-shadowed tile with a gradient, a
/// hairline outline (the accent when chosen) and light along its top.
pub(super) fn tile(p: &Painter, rect: Rect, r: u8, s: TileState) {
    let look = preview_look();
    let a = if s.online { 1.0 } else { 0.55 };
    shadow(p, rect, r, s.lift.max(s.hover * 0.35), &look);
    let (top, bottom) = if s.chosen { (look.chosen_top, look.chosen_bottom) } else { (look.tile_top, look.tile_bottom) };
    let (top, bottom) = if s.online { (top, bottom) } else { (mix(top, look.stage, 0.5), mix(bottom, look.stage, 0.5)) };
    smooth_gradient(p, rect, r, top, bottom);
    // Light along the top edge, inside the outline.
    let hi = Rect::from_min_max(rect.min + vec2(f32::from(r) * 0.6, 1.0), pos2(rect.right() - f32::from(r) * 0.6, rect.top() + 2.0));
    p.line_segment([hi.left_top(), hi.right_top()], Stroke::new(1.0, look.tile_hi.gamma_multiply(a)));
    if s.chosen {
        p.rect_stroke(rect.expand(2.5), CornerRadius::same(r + 2), Stroke::new(3.0, pal().ring), StrokeKind::Middle);
        p.rect_stroke(rect, CornerRadius::same(r), Stroke::new(1.5, pal().accent), StrokeKind::Inside);
    } else {
        let edge = mix(look.tile_edge, look.tile_edge_lit, s.hover.max(s.lift));
        p.rect_stroke(rect, CornerRadius::same(r), Stroke::new(1.0, edge.gamma_multiply(a)), StrokeKind::Inside);
    }
}

/// A monitor's bezel and shadow round `screen`; returns its glass (the
/// part a picture shows in) and the glass's corner.
pub(super) fn monitor(p: &Painter, screen: Rect, lift: f32) -> (Rect, u8) {
    let look = preview_look();
    let side = screen.width().min(screen.height());
    let bezel = (side * 0.03).clamp(2.0, 4.0).round();
    let r = (side * 0.075).clamp(5.0, 10.0).round() as u8;
    shadow(p, screen, r, lift, &look);
    p.rect_stroke(screen, CornerRadius::same(r), Stroke::new(1.0, look.bezel_edge), StrokeKind::Outside);
    smooth_gradient(p, screen, r, look.bezel_top, look.bezel_bottom);
    let hi_y = screen.top() + 0.5;
    p.line_segment([pos2(screen.left() + f32::from(r), hi_y), pos2(screen.right() - f32::from(r), hi_y)], Stroke::new(1.0, look.bezel_hi));
    let glass_r = r.saturating_sub(bezel as u8).max(2);
    (screen.shrink(bezel), glass_r)
}

/// The glass of a screen with no picture: a cool gradient, quietly lit.
pub(super) fn blank_glass(p: &Painter, glass: Rect, r: u8) {
    let look = preview_look();
    smooth_gradient(p, glass, r, look.blank_top, look.blank_bottom);
}

/// The finish over a screen's glass: a soft sheen on its upper part and a
/// fine dark edge where the glass meets the bezel.
pub(super) fn glass_finish(p: &Painter, glass: Rect, r: u8) {
    let look = preview_look();
    let upper = Rect::from_min_max(glass.min, pos2(glass.right(), glass.top() + glass.height() * 0.45));
    p.add(gradient(upper, f32::from(r), look.sheen, Color32::TRANSPARENT));
    p.rect_stroke(glass, CornerRadius::same(r), Stroke::new(1.0, look.glass_edge), StrokeKind::Inside);
}

/// A computer's name on a screen, from `at` (its bottom left): on a
/// picture, white on a dark capsule; on blank glass, quiet ink.
pub(super) fn name_tag(p: &Painter, at: Pos2, text: &str, on_picture: bool) -> Rect {
    let look = preview_look();
    let font = medium(T_META);
    if on_picture {
        let galley = p.layout_no_wrap(text.to_owned(), font, look.label_ink);
        let pill = Rect::from_min_size(pos2(at.x, at.y - galley.size().y - 6.0), galley.size() + vec2(14.0, 6.0));
        p.rect_filled(pill, CornerRadius::same((pill.height() / 2.0).round() as u8), look.label_back);
        p.galley(pill.min + vec2(7.0, 3.0), galley, look.label_ink);
        pill
    } else {
        p.text(at + vec2(2.0, -4.0), Align2::LEFT_BOTTOM, text, font, look.blank_ink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rounded_path_stays_inside_its_rectangle() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 40.0));
        let path = rounded_path(r, 8.0, 6);
        assert_eq!(path.len(), 28);
        assert!(path.iter().all(|p| r.expand(0.01).contains(*p)));
        // A corner bigger than half the height is held to it.
        assert!(rounded_path(r, 80.0, 4).iter().all(|p| r.expand(0.01).contains(*p)));
    }
}
