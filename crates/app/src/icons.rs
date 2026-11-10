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

/// A magnifier: search, centred on `c`.
pub fn search(p: &Painter, c: Pos2, col: Color32) {
    let lens = c + egui::vec2(-1.5, -1.5);
    p.circle_stroke(lens, 5.0, Stroke::new(1.6, col));
    p.line_segment([lens + egui::vec2(3.6, 3.6), lens + egui::vec2(7.0, 7.0)], Stroke::new(1.6, col));
}

/// The flat symbol on the brand's 64 grid, as the brand's flat symbol SVGs
/// draw it (a test reads the files): the key and its top as (x, y, side,
/// corner radius), and the pointer's path.
const SYMBOL_KEY: [f32; 4] = [5.0, 5.0, 54.0, 15.0];
const SYMBOL_TOP: [f32; 4] = [12.0, 8.0, 40.0, 11.0];
pub(crate) const SYMBOL_POINTER: [(f32, f32); 7] = [(24.0, 17.0), (24.0, 39.0), (29.2, 34.0), (32.6, 42.0), (36.4, 40.4), (33.0, 32.6), (40.0, 32.6)];

/// The oneMK symbol, flat (handoff section 2): a keycap with the pointer on
/// its top, `size` points square from `min`, in the theme's logo colours.
pub fn symbol(p: &Painter, min: Pos2, size: f32) {
    symbol_in(p, min, size, &crate::style::pal());
}

/// The flat symbol in the logo colours of `pal` (the dark theme's: the
/// light key with its cobalt pointer, as on the app icon's tile).
pub fn symbol_in(p: &Painter, min: Pos2, size: f32, pal: &crate::style::Palette) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let k = size / 64.0;
    let at = |x: f32, y: f32| pos2(min.x + x * k, min.y + y * k);
    let rounded = |[x, y, w, r]: [f32; 4]| (Rect::from_min_max(at(x, y), at(x + w, y + w)), egui::CornerRadius::same((r * k).round() as u8));
    let (key, kr) = rounded(SYMBOL_KEY);
    p.rect_filled(key, kr, pal.logo_cap);
    let (top, tr) = rounded(SYMBOL_TOP);
    p.rect_filled(top, tr, pal.logo_top);
    // The pointer is not convex: five triangles, then its outline at a
    // hair's width so its edges are smoothed as the window's other shapes.
    let pts = SYMBOL_POINTER.map(|(x, y)| at(x, y));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The window's symbol is the brand's flat symbol: the same grid and
    /// path as the SVGs (copied to `crates/app/assets/brand/` by
    /// scripts/make-icons.py), in the same colours for each theme.
    #[test]
    fn symbol_is_the_brand_files() {
        let attr = |tag: &str, name: &str| -> String {
            let at = tag.find(&format!(" {name}=\"")).unwrap_or_else(|| panic!("{name} in {tag}")) + name.len() + 3;
            tag[at..].split('"').next().expect("value").to_owned()
        };
        let num = |tag: &str, name: &str| -> f32 { attr(tag, name).parse().expect("number") };
        let hex = |c: Color32| format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
        for (svg, p) in [
            (include_str!("../assets/brand/onemk-symbol-flat-on-light.svg"), crate::style::Palette::light()),
            (include_str!("../assets/brand/onemk-symbol-flat-on-dark.svg"), crate::style::Palette::dark()),
        ] {
            let rects: Vec<&str> = svg.split("<rect").skip(1).map(|t| t.split('>').next().expect("tag")).collect();
            assert_eq!(rects.len(), 2, "{svg}");
            for (tag, want, colour) in [(rects[0], SYMBOL_KEY, p.logo_cap), (rects[1], SYMBOL_TOP, p.logo_top)] {
                assert_eq!(num(tag, "width"), num(tag, "height"));
                assert_eq!([num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "rx")], want, "{tag}");
                assert_eq!(attr(tag, "fill"), hex(colour), "{tag}");
            }
            let path = svg.split("<path").nth(1).expect("path");
            let points: Vec<(f32, f32)> = attr(path, "d")
                .trim_end_matches('Z')
                .split(['M', 'L'])
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let (x, y) = s.trim().split_once(' ').expect("x y");
                    (x.parse().expect("x"), y.parse().expect("y"))
                })
                .collect();
            assert_eq!(points, SYMBOL_POINTER.to_vec());
            assert_eq!(attr(path, "fill"), hex(p.logo_pointer));
        }
    }
}
