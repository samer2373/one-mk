//! Line icons, drawn rather than loaded: one 16-unit grid, one stroke,
//! the colour of the text beside them. Each is a few strokes, so they
//! stay sharp at any scale and cost nothing to keep.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Desk,
    Arrange,
    Clipboard,
    Wallpaper,
    Keys,
    Health,
    Settings,
    Chevron,
    Lock,
    Broadcast,
    Move,
    Send,
    Get,
    Plus,
    Power,
    Update,
    Clock,
}

/// Draws `icon` centred in `rect`, scaled from its 16-unit grid.
pub fn draw(p: &Painter, icon: Icon, rect: Rect, col: Color32) {
    let k = rect.width().min(rect.height()) / 16.0;
    let o = rect.center() - egui::vec2(8.0, 8.0) * k;
    let at = |x: f32, y: f32| -> Pos2 { pos2(o.x + x * k, o.y + y * k) };
    let st = Stroke::new((1.5 * k).max(1.2), col);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|(x, y)| at(*x, *y)).collect(), st));
    };
    let rrect = |x: f32, y: f32, w: f32, h: f32, r: f32| {
        p.rect_stroke(Rect::from_min_max(at(x, y), at(x + w, y + h)), r * k, st, egui::StrokeKind::Middle);
    };
    let dot = |x: f32, y: f32| {
        p.circle_filled(at(x, y), 0.9 * k, col);
    };
    let ring = |x: f32, y: f32, r: f32| {
        p.circle_stroke(at(x, y), r * k, st);
    };
    match icon {
        Icon::Wallpaper => {
            rrect(1.5, 2.5, 13.0, 11.0, 1.5);
            line(&[(2.0, 11.5), (6.0, 7.5), (9.0, 10.5), (11.0, 8.5), (14.0, 11.5)]);
            ring(10.75, 5.75, 1.0);
        }
        Icon::Desk => {
            rrect(1.75, 2.5, 12.5, 8.5, 1.5);
            line(&[(8.0, 11.0), (8.0, 13.5)]);
            line(&[(5.0, 13.5), (11.0, 13.5)]);
        }
        Icon::Arrange => {
            rrect(1.75, 3.0, 5.5, 5.0, 1.0);
            rrect(8.75, 5.0, 5.5, 6.0, 1.0);
            line(&[(4.5, 10.5), (4.5, 12.5)]);
            line(&[(11.5, 12.5), (11.5, 13.5)]);
        }
        Icon::Clipboard => {
            rrect(3.0, 2.5, 10.0, 12.0, 1.5);
            line(&[(6.0, 2.5), (6.0, 3.75), (10.0, 3.75), (10.0, 2.5)]);
            line(&[(5.75, 7.5), (10.25, 7.5)]);
            line(&[(5.75, 10.0), (8.75, 10.0)]);
        }
        Icon::Keys => {
            rrect(1.5, 4.0, 13.0, 8.0, 1.5);
            for x in [4.0, 6.5, 9.0, 11.5] {
                dot(x, 6.5);
            }
            line(&[(5.0, 9.5), (11.0, 9.5)]);
        }
        Icon::Health => line(&[(1.5, 8.5), (4.5, 8.5), (6.25, 4.5), (9.75, 12.5), (11.5, 8.5), (14.5, 8.5)]),
        Icon::Settings => {
            line(&[(2.0, 4.5), (9.0, 4.5)]);
            line(&[(12.0, 4.5), (14.0, 4.5)]);
            line(&[(2.0, 11.5), (5.0, 11.5)]);
            line(&[(8.0, 11.5), (14.0, 11.5)]);
            ring(10.5, 4.5, 1.5);
            ring(6.5, 11.5, 1.5);
        }
        Icon::Chevron => line(&[(6.0, 3.5), (10.5, 8.0), (6.0, 12.5)]),
        Icon::Lock => {
            rrect(3.0, 7.0, 10.0, 7.0, 1.5);
            line(&[(5.5, 7.0), (5.5, 5.0), (6.2, 3.3), (8.0, 2.5), (9.8, 3.3), (10.5, 5.0), (10.5, 7.0)]);
        }
        Icon::Broadcast => {
            rrect(1.5, 5.0, 13.0, 7.0, 1.5);
            for x in [4.0, 6.5, 9.0, 11.5] {
                dot(x, 8.5);
            }
            line(&[(8.0, 1.5), (8.0, 3.2)]);
            line(&[(4.8, 2.4), (5.8, 3.6)]);
            line(&[(11.2, 2.4), (10.2, 3.6)]);
        }
        Icon::Move => {
            line(&[(8.0, 1.5), (8.0, 14.5)]);
            line(&[(1.5, 8.0), (14.5, 8.0)]);
            line(&[(6.0, 3.5), (8.0, 1.5), (10.0, 3.5)]);
            line(&[(6.0, 12.5), (8.0, 14.5), (10.0, 12.5)]);
            line(&[(3.5, 6.0), (1.5, 8.0), (3.5, 10.0)]);
            line(&[(12.5, 6.0), (14.5, 8.0), (12.5, 10.0)]);
        }
        Icon::Send => {
            rrect(2.0, 3.0, 8.0, 10.0, 1.5);
            line(&[(7.0, 8.0), (14.0, 8.0)]);
            line(&[(11.5, 5.5), (14.0, 8.0), (11.5, 10.5)]);
        }
        Icon::Get => {
            rrect(6.0, 3.0, 8.0, 10.0, 1.5);
            line(&[(9.0, 8.0), (2.0, 8.0)]);
            line(&[(4.5, 5.5), (2.0, 8.0), (4.5, 10.5)]);
        }
        Icon::Power => {
            let pts: Vec<(f32, f32)> = (0..=24).map(|i| {
                let a = -1.05 + i as f32 / 24.0 * (std::f32::consts::TAU - 2.1);
                (8.0 - 5.5 * (a - std::f32::consts::FRAC_PI_2).cos(), 8.6 - 5.5 * (a - std::f32::consts::FRAC_PI_2).sin())
            }).collect();
            line(&pts);
            line(&[(8.0, 1.8), (8.0, 7.5)]);
        }
        Icon::Update => {
            line(&[(8.0, 2.0), (8.0, 10.5)]);
            line(&[(4.5, 7.0), (8.0, 10.5), (11.5, 7.0)]);
            line(&[(2.5, 13.5), (13.5, 13.5)]);
        }
        Icon::Clock => {
            ring(8.0, 8.0, 6.25);
            line(&[(8.0, 4.5), (8.0, 8.0), (10.5, 9.5)]);
        }
        Icon::Plus => {
            line(&[(8.0, 3.0), (8.0, 13.0)]);
            line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
    }
}

/// The pointer from the logo (its tip in Volt): where the mouse is now.
pub fn pointer(p: &Painter, tip: Pos2, h: f32, col: Color32) {
    // The oneMK mark (docs/brand/logo/onemk-symbol-*.svg) without its
    // fillets: a body cut by a screen's edge, its tip already across and
    // lit. In units of the 256 master, the tip's point at (44, 47) and
    // the mark 127 high; drawn `h` high from `tip`. The body is the light
    // ink, the tip `col` (Volt where it says where the pointer is).
    let k = h / 127.0;
    let at = |x: f32, y: f32| pos2(tip.x + (x - 44.0) * k, tip.y + (y - 47.0) * k);
    let body = if col == crate::style::pal().accent { crate::style::pal().ink } else { col };
    // The body is not convex (its notch): two convex pieces.
    p.add(Shape::convex_polygon(vec![at(110.0, 61.0), at(180.0, 80.0), at(180.0, 103.0), at(136.0, 115.0), at(68.0, 103.0)], body, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(68.0, 103.0), at(136.0, 115.0), at(131.0, 166.0), at(108.0, 172.0)], body, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(53.0, 46.0), at(100.0, 58.0), at(66.0, 95.0), at(43.0, 59.0)], col, Stroke::NONE));
}

/// A magnifier: search, centred on `c`.
pub fn search(p: &Painter, c: Pos2, col: Color32) {
    let lens = c + egui::vec2(-1.5, -1.5);
    p.circle_stroke(lens, 5.0, Stroke::new(1.6, col));
    p.line_segment([lens + egui::vec2(3.6, 3.6), lens + egui::vec2(7.0, 7.0)], Stroke::new(1.6, col));
}

/// The oneMK symbol, flat (handoff section 2): a keycap with the pointer on
/// its top, `size` points square from `min`, in the theme's logo colours.
/// Built on the brand's 64 grid: the key 54 r15 at (5, 5), its top 40 r11
/// at (12, 8), the pointer as the brand path.
pub fn symbol(p: &Painter, min: Pos2, size: f32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let pal = crate::style::pal();
    let k = size / 64.0;
    let at = |x: f32, y: f32| pos2(min.x + x * k, min.y + y * k);
    let rounded = |x: f32, y: f32, w: f32, r: f32| (Rect::from_min_max(at(x, y), at(x + w, y + w)), egui::CornerRadius::same((r * k).round() as u8));
    let (key, kr) = rounded(5.0, 5.0, 54.0, 15.0);
    p.rect_filled(key, kr, pal.logo_cap);
    let (top, tr) = rounded(12.0, 8.0, 40.0, 11.0);
    p.rect_filled(top, tr, pal.logo_top);
    // The pointer (M24 17 L24 39 L29.2 34 L32.6 42 L36.4 40.4 L33 32.6
    // L40 32.6 Z) is not convex: five triangles, then its outline at a
    // hair's width so its edges are smoothed as the window's other shapes.
    let pts = [at(24.0, 17.0), at(24.0, 39.0), at(29.2, 34.0), at(32.6, 42.0), at(36.4, 40.4), at(33.0, 32.6), at(40.0, 32.6)];
    let mut mesh = Mesh::default();
    for q in pts {
        mesh.vertices.push(Vertex { pos: q, uv: WHITE_UV, color: pal.logo_pointer });
    }
    for [a, b, c] in [[0, 1, 2], [0, 2, 5], [0, 5, 6], [2, 3, 4], [2, 4, 5]] {
        mesh.add_triangle(a, b, c);
    }
    p.add(mesh);
    p.add(Shape::closed_line(pts.to_vec(), Stroke::new(0.5, pal.logo_pointer)));
}
