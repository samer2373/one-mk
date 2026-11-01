//! The keyboard and the mouse as lit objects on the desk, drawn with egui
//! meshes: rounded keycaps with dished tops on a low aluminium case, and a
//! mouse model packed by `scripts/mesh-pack.py`. They are lit like the
//! desk: the screens' cool light from behind, a warm lamp in front.
//!
//! Building one costs a few milliseconds, so each is built once into a
//! mesh and kept until its size or what it shows changes; hover, the keys
//! held and the chosen key are drawn over the kept mesh. Everything is in
//! millimetres; y is up and the viewer looks toward -z.

use egui::{Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Rgba, Shape, Stroke, pos2};
use std::sync::Arc;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn p3(x: f32, y: f32, z: f32) -> P3 {
    P3 { x, y, z }
}

impl Add for P3 {
    type Output = P3;
    fn add(self, b: P3) -> P3 {
        p3(self.x + b.x, self.y + b.y, self.z + b.z)
    }
}
impl Sub for P3 {
    type Output = P3;
    fn sub(self, b: P3) -> P3 {
        p3(self.x - b.x, self.y - b.y, self.z - b.z)
    }
}
impl Mul<f32> for P3 {
    type Output = P3;
    fn mul(self, s: f32) -> P3 {
        p3(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for P3 {
    type Output = P3;
    fn neg(self) -> P3 {
        p3(-self.x, -self.y, -self.z)
    }
}
impl P3 {
    fn dot(self, b: P3) -> f32 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }
    fn cross(self, b: P3) -> P3 {
        p3(self.y * b.z - self.z * b.y, self.z * b.x - self.x * b.z, self.x * b.y - self.y * b.x)
    }
    fn norm(self) -> P3 {
        let l = self.dot(self).sqrt();
        if l > 1e-6 { self * (1.0 / l) } else { p3(0.0, 1.0, 0.0) }
    }
    /// Turned round the vertical axis by `a` radians.
    fn yaw(self, a: f32) -> P3 {
        let (s, c) = a.sin_cos();
        p3(self.x * c + self.z * s, self.y, -self.x * s + self.z * c)
    }
}

fn lerp_col(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}
fn lerp3(a: P3, b: P3, t: f32) -> P3 {
    a + (b - a) * t
}

/// A pinhole camera that looks at `target` from `dist` away.
#[derive(Clone, Copy, Debug)]
pub struct Cam {
    eye: P3,
    right: P3,
    up: P3,
    fwd: P3,
    foc: f32,
    centre: Pos2,
}

impl Cam {
    /// `yaw` turns round the vertical axis, `pitch` looks down.
    pub fn new(target: P3, yaw: f32, pitch: f32, dist: f32, foc: f32, centre: Pos2) -> Self {
        let back = p3(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos());
        let eye = target + back * dist;
        let fwd = (-back).norm();
        let right = fwd.cross(p3(0.0, 1.0, 0.0)).norm();
        let up = right.cross(fwd);
        Self { eye, right, up, fwd, foc, centre }
    }

    pub fn project(&self, p: P3) -> (Pos2, f32) {
        let d = p - self.eye;
        let z = d.dot(self.fwd).max(1e-3);
        (pos2(self.centre.x + d.dot(self.right) / z * self.foc, self.centre.y - d.dot(self.up) / z * self.foc), z)
    }

    pub fn pt(&self, p: P3) -> Pos2 {
        self.project(p).0
    }

    /// The same view, scaled and moved so `pts` fill `rect` less `pad`.
    pub fn fitted(self, pts: &[P3], rect: Rect, pad: f32) -> Self {
        let ps: Vec<Pos2> = pts.iter().map(|p| self.pt(*p)).collect();
        let (mut lo, mut hi) = (pos2(f32::MAX, f32::MAX), pos2(f32::MIN, f32::MIN));
        for p in &ps {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let room = rect.shrink(pad);
        let k = (room.width() / (hi.x - lo.x)).min(room.height() / (hi.y - lo.y));
        let mid = pos2((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
        let mut cam = self;
        cam.foc *= k;
        cam.centre = room.center() - (mid - self.centre) * k;
        cam
    }
}

/// Light: the screens behind the desk (cool, from above and behind), a
/// warm lamp in front and to the right, and a little bounce from the desk.
struct Lighting {
    eye: P3,
}

const KEY_DIR: P3 = p3(-0.22, 0.86, -0.46);
const KEY_COL: [f32; 3] = [0.78, 0.86, 1.0];
const LAMP_DIR: P3 = p3(0.62, 0.52, 0.58);
const LAMP_COL: [f32; 3] = [1.0, 0.74, 0.46];

/// A surface: its colour, how glossy it is and how strong the highlight.
#[derive(Clone, Copy, Debug)]
pub struct Finish {
    pub albedo: Color32,
    pub gloss: f32,
    pub spec: f32,
}

impl Lighting {
    fn shade(&self, f: Finish, p: P3, n: P3) -> Color32 {
        self.shade_with(f.albedo, f, p, n)
    }

    fn shade_with(&self, albedo: Color32, f: Finish, p: P3, n: P3) -> Color32 {
        let a = Rgba::from(albedo);
        let v = (self.eye - p).norm();
        let mut out = [0.0f32; 3];
        let alb = [a.r(), a.g(), a.b()];
        let amb = 0.09 + 0.05 * n.y.max(0.0);
        for (i, o) in out.iter_mut().enumerate() {
            *o = alb[i] * amb;
        }
        for (dir, col, k) in [(KEY_DIR.norm(), KEY_COL, 1.05), (LAMP_DIR.norm(), LAMP_COL, 0.42)] {
            let lam = n.dot(dir).max(0.0);
            let h = (dir + v).norm();
            let sp = n.dot(h).max(0.0).powf(f.gloss) * f.spec * if lam > 0.0 { 1.0 } else { 0.0 };
            for i in 0..3 {
                out[i] += (alb[i] * lam + sp) * col[i] * k;
            }
        }
        // A cool rim where the surface turns away: the screens behind.
        let rim = (1.0 - n.dot(v).max(0.0)).powi(4) * 0.12 * n.y.max(0.0).sqrt();
        for i in 0..3 {
            out[i] += rim * KEY_COL[i];
        }
        Color32::from(Rgba::from_rgb(out[0], out[1], out[2]))
    }
}

/// Triangles of one object, shaded, sorted back to front when drawn.
#[derive(Default)]
struct Tris {
    v: Vec<([Pos2; 3], [Color32; 3], f32)>,
}

impl Tris {
    /// Adds a lit triangle unless it faces away from the camera.
    fn add(&mut self, cam: &Cam, lit: &Lighting, f: Finish, p: [P3; 3], n: [P3; 3]) {
        let c = (p[0] + p[1] + p[2]) * (1.0 / 3.0);
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let face = if face.dot(n[0] + n[1] + n[2]) < 0.0 { -face } else { face };
        if face.dot(cam.eye - c) <= 0.0 {
            return;
        }
        let (a, za) = cam.project(p[0]);
        let (b, zb) = cam.project(p[1]);
        let (d, zd) = cam.project(p[2]);
        let cols = [lit.shade(f, p[0], n[0]), lit.shade(f, p[1], n[1]), lit.shade(f, p[2], n[2])];
        self.v.push(([a, b, d], cols, (za + zb + zd) / 3.0));
    }

    /// A triangle whose colour changes across it (a baked texture).
    fn add_coloured(&mut self, cam: &Cam, lit: &Lighting, f: Finish, p: [P3; 3], n: [P3; 3], a: [Color32; 3]) {
        let c = (p[0] + p[1] + p[2]) * (1.0 / 3.0);
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let face = if face.dot(n[0] + n[1] + n[2]) < 0.0 { -face } else { face };
        if face.dot(cam.eye - c) <= 0.0 {
            return;
        }
        let (pa, za) = cam.project(p[0]);
        let (pb, zb) = cam.project(p[1]);
        let (pd, zd) = cam.project(p[2]);
        let cols = [lit.shade_with(a[0], f, p[0], n[0]), lit.shade_with(a[1], f, p[1], n[1]), lit.shade_with(a[2], f, p[2], n[2])];
        self.v.push(([pa, pb, pd], cols, (za + zb + zd) / 3.0));
    }

    fn quad(&mut self, cam: &Cam, lit: &Lighting, f: Finish, p: [P3; 4], n: [P3; 4]) {
        self.add(cam, lit, f, [p[0], p[1], p[2]], [n[0], n[1], n[2]]);
        self.add(cam, lit, f, [p[0], p[2], p[3]], [n[0], n[2], n[3]]);
    }

    /// Back to front, as one mesh.
    fn into_mesh(mut self, m: &mut Mesh) {
        self.v.sort_by(|a, b| b.2.total_cmp(&a.2));
        for (pts, cols, _) in &self.v {
            let i = m.vertices.len() as u32;
            for k in 0..3 {
                m.colored_vertex(pts[k], cols[k]);
            }
            m.add_triangle(i, i + 1, i + 2);
        }
    }
}

/// A rounded rectangle at height `y`, counter-clockwise seen from above.
fn rring(cx: f32, cz: f32, hw: f32, hd: f32, r: f32, y: f32) -> Vec<P3> {
    let r = r.min(hw).min(hd);
    const SEG: usize = 5;
    let corners = [(hw - r, -(hd - r), -FRAC_PI_2), (hw - r, hd - r, 0.0), (-(hw - r), hd - r, FRAC_PI_2), (-(hw - r), -(hd - r), PI)];
    let mut out = Vec::with_capacity(4 * (SEG + 1));
    for (ox, oz, a0) in corners {
        for s in 0..=SEG {
            let a = a0 + FRAC_PI_2 * s as f32 / SEG as f32;
            out.push(p3(cx + ox + r * a.cos(), y, cz + oz + r * a.sin()));
        }
    }
    out
}

/// The walls between two rings, with normals smoothed round the corners.
fn walls(t: &mut Tris, cam: &Cam, lit: &Lighting, f: Finish, lo: &[P3], hi: &[P3], centre: P3) {
    let n = lo.len();
    let face = |i: usize| {
        let j = (i + 1) % n;
        let nn = (lo[j] - lo[i]).cross(hi[i] - lo[i]).norm();
        let mid = (lo[i] + lo[j]) * 0.5;
        let out = p3(mid.x - centre.x, 0.0, mid.z - centre.z);
        if nn.dot(out) < 0.0 { -nn } else { nn }
    };
    let faces: Vec<P3> = (0..n).map(face).collect();
    let vn: Vec<P3> = (0..n).map(|i| (faces[i] + faces[(i + n - 1) % n]).norm()).collect();
    for i in 0..n {
        let j = (i + 1) % n;
        t.quad(cam, lit, f, [lo[i], lo[j], hi[j], hi[i]], [vn[i], vn[j], vn[j], vn[i]]);
    }
}

// ---------------------------------------------------------------- keyboard

/// One key unit: 19.05 mm, as on real keyboards.
pub const U: f32 = 19.05;

/// A key to draw: where it is in key units (y down the board), its legend,
/// what it does now when that differs, and whether it is a modifier
/// (drawn in the warmer grey).
#[derive(Clone, Debug, PartialEq)]
pub struct Cap {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
    pub to: Option<String>,
    pub modifier: bool,
}

const CASE: Finish = Finish { albedo: Color32::from_rgb(0x3A, 0x3E, 0x45), gloss: 40.0, spec: 0.22 };
const CAP: Finish = Finish { albedo: Color32::from_rgb(0x34, 0x37, 0x3D), gloss: 18.0, spec: 0.08 };
const CAP_MOD: Finish = Finish { albedo: Color32::from_rgb(0x55, 0x52, 0x4D), gloss: 18.0, spec: 0.08 };
const DESK_LIT: Color32 = Color32::from_rgb(0x3B, 0x30, 0x27);
pub const SIGNAL: Color32 = Color32::from_rgb(0xFF, 0xB5, 0x47);

/// A legend to print on a key: where, how big, which colour, and whether
/// it is the key's new job (printed on its front, in amber).
struct Legend {
    at: Pos2,
    size: f32,
    text: String,
    to: bool,
    modifier: bool,
}

/// A built keyboard: the lit mesh, each key's top on screen (for clicks
/// and highlights) and its legends.
pub struct Keyboard {
    built_for: (Rect, Vec<Cap>),
    mesh: Arc<Mesh>,
    pub tops: Vec<Vec<Pos2>>,
    legends: Vec<Legend>,
}

impl Keyboard {
    /// The keyboard for `caps` filling `rect`: the last one when nothing
    /// changed, else built anew.
    pub fn get(slot: &mut Option<Keyboard>, rect: Rect, caps: &[Cap]) {
        if slot.as_ref().is_some_and(|k| k.built_for.0 == rect && k.built_for.1 == caps) {
            return;
        }
        *slot = Some(Self::build(rect, caps));
    }

    fn build(rect: Rect, caps: &[Cap]) -> Keyboard {
        let cols = caps.iter().map(|c| c.x + c.w).fold(1.0f32, f32::max);
        let rows = caps.iter().map(|c| c.y + c.h).fold(1.0f32, f32::max);
        let (w, d) = (cols * U, rows * U);
        let margin = 9.0;
        let (x0, z0) = (-w / 2.0, -d / 2.0);
        let base = Cam::new(p3(0.0, 0.0, 0.0), -0.06, 0.92, 760.0, 1.0, rect.center());
        let bounds = [p3(x0 - margin, 0.0, z0 - margin), p3(-x0 + margin, 0.0, z0 - margin), p3(-x0 + margin, -16.0, -z0 + margin + 4.0), p3(x0 - margin, -16.0, -z0 + margin + 4.0), p3(0.0, 12.0, z0)];
        let cam = base.fitted(&bounds, rect, 10.0);
        let lit = Lighting { eye: cam.eye };
        let mut m = Mesh::default();
        desk(&mut m, &cam, p3(0.0, -16.0, 0.0), w * 0.9, d * 1.6);
        fade_to_edges(&mut m, 0, rect, 48.0);
        let (hw, hd) = (w / 2.0 + margin, d / 2.0 + margin);
        let shadow = rring(0.0, 6.0, hw + 6.0, hd + 6.0, 14.0, -15.9);
        soft_blob(&mut m, &cam, &shadow, p3(0.0, -15.9, 6.0), 16.0, 150);
        // The case: a low slab with rounded corners and a bevelled top edge.
        let mut t = Tris::default();
        let bottom = rring(0.0, 0.0, hw - 1.5, hd - 1.5, 9.0, -16.0);
        let side_top = rring(0.0, 0.0, hw, hd, 10.0, -2.0);
        let top = rring(0.0, 0.0, hw - 2.0, hd - 2.0, 8.0, 0.0);
        walls(&mut t, &cam, &lit, CASE, &bottom, &side_top, p3(0.0, 0.0, 0.0));
        walls(&mut t, &cam, &lit, CASE, &side_top, &top, p3(0.0, 0.0, 0.0));
        fan(&mut t, &cam, &lit, Finish { albedo: Color32::from_rgb(0x1C, 0x1E, 0x22), ..CASE }, &top, p3(0.0, 0.0, 0.0), p3(0.0, 1.0, 0.0));
        t.into_mesh(&mut m);
        // Keys, back to front, each with its legend on top.
        let mut order: Vec<usize> = (0..caps.len()).collect();
        order.sort_by(|a, b| caps[*a].y.total_cmp(&caps[*b].y));
        let mut tops = vec![Vec::new(); caps.len()];
        let mut legends = Vec::with_capacity(caps.len() * 2);
        let mut keys = Tris::default();
        for i in order {
            let k = &caps[i];
            let cx = x0 + (k.x + k.w / 2.0) * U;
            let cz = z0 + (k.y + k.h / 2.0) * U;
            let finish = if k.modifier { CAP_MOD } else { CAP };
            let row = (k.y.max(0.0) as usize).min(4) as f32;
            let top = cap(&mut keys, &cam, &lit, finish, cx, cz, k.w * U, k.h * U, 1.0, 7.6 - row * 0.25);
            tops[i] = top.iter().map(|p| cam.pt(*p)).collect();
            let y_top = top[0].y;
            let (tl, tr) = (cam.pt(top[top.len() * 3 / 4]), cam.pt(top[0]));
            let px = (tr.x - tl.x).abs().max(1.0);
            let size = (px / k.w.max(1.0) * 0.25).clamp(8.0, 14.0);
            let at = cam.pt(p3(cx - k.w * U / 2.0 + 4.2, y_top, cz - k.h * U / 2.0 + 4.0));
            legends.push(Legend { at, size, text: k.label.clone(), to: false, modifier: k.modifier });
            if let Some(to) = &k.to {
                // On the cap's front, as side legends are on real keycaps.
                let at = cam.pt(p3(cx, 1.0 + (y_top - 1.0) * 0.42, cz + k.h * U / 2.0 - 1.1));
                legends.push(Legend { at, size: (size * 1.05).max(10.5), text: to.clone(), to: true, modifier: k.modifier });
            }
        }
        keys.into_mesh(&mut m);
        Keyboard { built_for: (rect, caps.to_vec()), mesh: Arc::new(m), tops, legends }
    }

    /// Draws the kept mesh, then what changes from frame to frame: the
    /// keys held (darker, as if pressed), the hovered key, the chosen one
    /// in amber, and the legends.
    pub fn paint(&self, p: &Painter, held: &[usize], hovered: Option<usize>, chosen: Option<usize>, font: &dyn Fn(f32, bool) -> FontId) {
        p.add(Shape::Mesh(self.mesh.clone()));
        let fill = |i: usize, col: Color32| {
            if let Some(t) = self.tops.get(i).filter(|t| t.len() > 2) {
                p.add(Shape::convex_polygon(t.clone(), col, Stroke::NONE));
            }
        };
        for &i in held {
            fill(i, Color32::from_black_alpha(90));
        }
        if let Some(i) = hovered.filter(|i| Some(*i) != chosen) {
            fill(i, Color32::from_white_alpha(18));
        }
        for l in &self.legends {
            let col = if l.to { SIGNAL } else if l.modifier { Color32::from_rgb(0xD9, 0xD3, 0xC8) } else { Color32::from_rgb(0xC6, 0xCB, 0xD3) };
            p.text(l.at, if l.to { Align2::CENTER_CENTER } else { Align2::LEFT_TOP }, &l.text, font(l.size, l.to), col);
        }
        if let Some(t) = chosen.and_then(|i| self.tops.get(i)).filter(|t| t.len() > 2) {
            let mut ring = t.clone();
            ring.push(ring[0]);
            p.add(Shape::line(ring, Stroke::new(2.0, SIGNAL)));
        }
    }

    /// The key under `pos`, front keys first.
    pub fn hit(&self, pos: Pos2) -> Option<usize> {
        (0..self.tops.len()).rev().find(|&i| inside(pos, &self.tops[i]))
    }

    /// A key's outline's bounding box (for focus rings and screen readers).
    pub fn bounds(&self, i: usize) -> Rect {
        self.tops.get(i).map_or(Rect::NOTHING, |t| Rect::from_points(t))
    }
}

pub fn inside(p: Pos2, poly: &[Pos2]) -> bool {
    let mut odd = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            odd = !odd;
        }
    }
    odd
}

/// A sculpted keycap: a rounded skirt, a bevel and a dished top. Returns
/// the ring of the top face.
#[allow(clippy::too_many_arguments)]
fn cap(t: &mut Tris, cam: &Cam, lit: &Lighting, f: Finish, cx: f32, cz: f32, w: f32, d: f32, y0: f32, h: f32) -> Vec<P3> {
    let (hw, hd) = (w / 2.0 - 0.55, d / 2.0 - 0.55);
    let base = rring(cx, cz, hw, hd, 1.8, y0);
    let shoulder = rring(cx, cz - 0.5, hw - 2.0, hd - 2.4, 2.4, y0 + h - 0.9);
    let top = rring(cx, cz - 0.6, hw - 2.6, hd - 3.0, 2.2, y0 + h);
    let c = p3(cx, y0, cz);
    walls(t, cam, lit, f, &base, &shoulder, c);
    walls(t, cam, lit, f, &shoulder, &top, c);
    // The dish: a shallow cylinder across the key, lowest in the middle.
    let dish = 0.55;
    let (thw, thd) = (hw - 2.6, hd - 3.0);
    let centre = p3(cx, y0 + h - dish, cz - 0.6);
    let inner: Vec<P3> = top.iter().map(|p| {
        let q = lerp3(centre, *p, 0.55);
        p3(q.x, y0 + h - dish * (1.0 - 0.55 * 0.55), q.z)
    }).collect();
    let normal = |p: P3| {
        let k = 2.0 * dish / (thd * thd).max(1.0);
        let kx = 2.0 * dish * 0.3 / (thw * thw).max(1.0);
        p3(-(p.x - centre.x) * kx, 1.0, -(p.z - centre.z) * k).norm()
    };
    let n = top.len();
    for i in 0..n {
        let j = (i + 1) % n;
        t.quad(cam, lit, f, [inner[i], inner[j], top[j], top[i]], [normal(inner[i]), normal(inner[j]), normal(top[j]), normal(top[i])]);
        t.add(cam, lit, f, [centre, inner[i], inner[j]], [p3(0.0, 1.0, 0.0), normal(inner[i]), normal(inner[j])]);
    }
    top
}

fn fan(t: &mut Tris, cam: &Cam, lit: &Lighting, f: Finish, ring: &[P3], c: P3, n: P3) {
    let centre = p3(c.x, ring[0].y, c.z);
    for i in 0..ring.len() {
        let j = (i + 1) % ring.len();
        t.add(cam, lit, f, [centre, ring[i], ring[j]], [n, n, n]);
    }
}

/// The desk under an object: walnut where the light falls, the room beyond.
fn desk(m: &mut Mesh, cam: &Cam, c: P3, rx: f32, rz: f32) {
    const NX: usize = 24;
    const NZ: usize = 16;
    let base = m.vertices.len() as u32;
    // Lit wood fading to nothing, so it sits on whatever is behind.
    let shade = |k: f32| DESK_LIT.gamma_multiply(k);
    for j in 0..=NZ {
        for i in 0..=NX {
            let (u, w) = (i as f32 / NX as f32 * 2.0 - 1.0, j as f32 / NZ as f32 * 2.0 - 1.0);
            let p = p3(c.x + u * rx * 2.2, c.y, c.z + w * rz * 1.6);
            let dist = ((u * 1.1).powi(2) + (w * 1.05).powi(2)).sqrt();
            m.colored_vertex(cam.pt(p), shade((1.0 - dist).clamp(0.0, 1.0).powf(1.3)));
        }
    }
    let w = (NX + 1) as u32;
    for j in 0..NZ as u32 {
        for i in 0..NX as u32 {
            let k = j * w + i;
            m.add_triangle(base + k, base + k + 1, base + k + w + 1);
            m.add_triangle(base + k, base + k + w + 1, base + k + w);
        }
    }
}

/// The desk is drawn clipped to `rect`: fade it out over `feather` points
/// before each edge, so it ends softly instead of in a hard box.
fn fade_to_edges(m: &mut Mesh, from: usize, rect: Rect, feather: f32) {
    for v in &mut m.vertices[from..] {
        let d = (v.pos.x - rect.left()).min(rect.right() - v.pos.x).min(v.pos.y - rect.top()).min(rect.bottom() - v.pos.y);
        let k = (d / feather).clamp(0.0, 1.0);
        v.color = v.color.gamma_multiply(k * k * (3.0 - 2.0 * k));
    }
}

/// A soft shadow following an outline on the desk.
fn soft_blob(m: &mut Mesh, cam: &Cam, ring: &[P3], c: P3, feather: f32, alpha: u8) {
    let b = m.vertices.len() as u32;
    let col = Color32::from_black_alpha(alpha);
    m.colored_vertex(cam.pt(c), col);
    let n = ring.len() as u32;
    for p in ring {
        m.colored_vertex(cam.pt(*p), col);
    }
    for p in ring {
        let d = p3(p.x - c.x, 0.0, p.z - c.z).norm();
        m.colored_vertex(cam.pt(*p + d * feather + p3(-feather * 0.2, 0.0, feather * 0.3)), Color32::TRANSPARENT);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(b, b + 1 + i, b + 1 + j);
        m.add_triangle(b + 1 + i, b + 1 + j, b + 1 + n + j);
        m.add_triangle(b + 1 + i, b + 1 + n + j, b + 1 + n + i);
    }
}

// ------------------------------------------------------------------- model

/// A model packed by `scripts/mesh-pack.py`: parts with their own
/// vertices, normals and baked colours, in millimetres, standing on y = 0.
pub struct Model {
    pub parts: Vec<ModelPart>,
}

pub struct ModelPart {
    pub name: String,
    gloss: f32,
    spec: f32,
    pos: Vec<P3>,
    nrm: Vec<P3>,
    col: Vec<Color32>,
    idx: Vec<u32>,
}

impl Model {
    pub fn parse(b: &[u8]) -> Option<Model> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        if take(4)? != b"OMKM" {
            return None;
        }
        let u32_ = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let f32_ = |s: &[u8]| f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        if u32_(take(4)?) != 1 {
            return None;
        }
        let count = u32_(take(4)?) as usize;
        let mut parts = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let len = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
            let name = String::from_utf8_lossy(take(len)?).into_owned();
            let (gloss, spec) = (f32_(take(4)?), f32_(take(4)?));
            let (nv, ni) = (u32_(take(4)?) as usize, u32_(take(4)?) as usize);
            if nv > 1 << 20 || ni > 1 << 22 {
                return None;
            }
            let mut v3 = |n: usize| -> Option<Vec<P3>> {
                let s = take(n * 12)?;
                Some(s.as_chunks::<12>().0.iter().map(|c| p3(f32_(&c[0..4]), f32_(&c[4..8]), f32_(&c[8..12]))).collect())
            };
            let pos = v3(nv)?;
            let nrm = v3(nv)?;
            let col = take(nv * 3)?.as_chunks::<3>().0.iter().map(|c| Color32::from_rgb(c[0], c[1], c[2])).collect();
            let idx: Vec<u32> = take(ni * 4)?.as_chunks::<4>().0.iter().map(|c| u32_(c)).collect();
            if idx.iter().any(|i| *i as usize >= nv) {
                return None;
            }
            parts.push(ModelPart { name, gloss, spec, pos, nrm, col, idx });
        }
        Some(Model { parts })
    }

    fn points(&self) -> impl Iterator<Item = P3> + '_ {
        self.parts.iter().flat_map(|p| p.pos.iter().copied())
    }
}

/// A built model view: the lit mesh, and each part's outline and the
/// point its label line goes to.
pub struct ModelView {
    built_for: (Rect, Rect, Option<String>, [u32; 3]),
    mesh: Arc<Mesh>,
    pub parts: Vec<ModelHit>,
}

/// Where a part of a model is on screen.
pub struct ModelHit {
    pub name: String,
    pub anchor: Pos2,
    /// Its triangles that face the camera, for clicks.
    pub tris: Vec<[Pos2; 3]>,
}

impl ModelHit {
    pub fn contains(&self, p: Pos2) -> bool {
        self.tris.iter().any(|t| inside(p, t))
    }
}

impl ModelView {
    /// `m` on the desk, filling `rect` and framed in `room`, turned by
    /// `yaw` and seen from `cam_yaw`/`pitch`; the part named `chosen` is
    /// tinted amber. The last one when nothing changed.
    #[allow(clippy::too_many_arguments)]
    pub fn get(slot: &mut Option<ModelView>, m: &Model, rect: Rect, room: Rect, yaw: f32, cam_yaw: f32, pitch: f32, chosen: Option<&str>) {
        let key = (rect, room, chosen.map(str::to_owned), [yaw.to_bits(), cam_yaw.to_bits(), pitch.to_bits()]);
        if slot.as_ref().is_some_and(|v| v.built_for == key) {
            return;
        }
        let pts: Vec<P3> = m.points().map(|p| p.yaw(yaw)).collect();
        let (mut lo, mut hi) = (p3(f32::MAX, f32::MAX, f32::MAX), p3(f32::MIN, f32::MIN, f32::MIN));
        for p in &pts {
            lo = p3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = p3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        let mid = (lo + hi) * 0.5;
        let base = Cam::new(mid, cam_yaw, pitch, 420.0, 1.0, rect.center());
        let cam = base.fitted(&pts, room, room.height() * 0.08);
        let lit = Lighting { eye: cam.eye };
        let mut mesh = Mesh::default();
        desk(&mut mesh, &cam, p3(mid.x, 0.0, mid.z), (hi.x - lo.x) * 1.6, (hi.z - lo.z) * 1.0);
        fade_to_edges(&mut mesh, 0, room, 48.0);
        let foot: Vec<P3> = (0..40)
            .map(|i| {
                let a = i as f32 / 40.0 * TAU;
                p3(mid.x + a.cos() * (hi.x - lo.x) * 0.46, 0.0, mid.z + a.sin() * (hi.z - lo.z) * 0.46)
            })
            .collect();
        soft_blob(&mut mesh, &cam, &foot, p3(mid.x, 0.0, mid.z), 14.0, 170);
        let mut t = Tris::default();
        let mut parts = Vec::with_capacity(m.parts.len());
        for part in &m.parts {
            let f = Finish { albedo: Color32::GRAY, gloss: part.gloss.max(2.0), spec: part.spec };
            let tint = |c: Color32| if chosen == Some(part.name.as_str()) { lerp_col(c, SIGNAL, 0.55) } else { c };
            let mut tris = Vec::new();
            let mut centre = p3(0.0, 0.0, 0.0);
            for tri in part.idx.as_chunks::<3>().0 {
                let i = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                let v = i.map(|k| part.pos[k].yaw(yaw));
                let n = i.map(|k| part.nrm[k].yaw(yaw));
                t.add_coloured(&cam, &lit, f, v, n, i.map(|k| tint(part.col[k])));
                let c = (v[0] + v[1] + v[2]) * (1.0 / 3.0);
                if (n[0] + n[1] + n[2]).dot(cam.eye - c) > 0.0 {
                    tris.push(v.map(|p| cam.pt(p)));
                }
            }
            for p in &part.pos {
                centre = centre + p.yaw(yaw);
            }
            let centre = centre * (1.0 / part.pos.len().max(1) as f32);
            parts.push(ModelHit { name: part.name.clone(), anchor: cam.pt(centre), tris });
        }
        t.into_mesh(&mut mesh);
        *slot = Some(ModelView { built_for: key, mesh: Arc::new(mesh), parts });
    }

    /// The kept mesh, and the hovered part lit a little.
    pub fn shapes(&self, hovered: Option<&str>) -> Shape {
        let mut v = vec![Shape::Mesh(self.mesh.clone())];
        if let Some(h) = hovered.and_then(|h| self.parts.iter().find(|x| x.name == h)) {
            let mut m = Mesh::default();
            for t in &h.tris {
                let i = m.vertices.len() as u32;
                for q in t {
                    m.colored_vertex(*q, Color32::from_white_alpha(22));
                }
                m.add_triangle(i, i + 1, i + 2);
            }
            v.push(Shape::mesh(m));
        }
        Shape::Vec(v)
    }

    /// The part under `pos` among `named`.
    pub fn hit(&self, pos: Pos2, named: &[&str]) -> Option<String> {
        self.parts.iter().filter(|h| named.contains(&h.name.as_str())).find(|h| h.contains(pos)).map(|h| h.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE: &[u8] = include_bytes!("../assets/gear/mouse.omkm");

    #[test]
    fn the_built_in_mouse_has_its_five_buttons() {
        let m = Model::parse(MOUSE).expect("the built-in mouse parses");
        for n in ["left", "right", "wheel", "back", "forward"] {
            assert!(m.parts.iter().any(|p| p.name == n), "{n} missing");
        }
    }

    #[test]
    fn broken_model_files_are_refused_not_trusted() {
        assert!(Model::parse(b"").is_none());
        assert!(Model::parse(b"OMKM").is_none());
        assert!(Model::parse(b"NOPE\x01\0\0\0\0\0\0\0").is_none());
        // Cut short anywhere: refused, never a panic.
        for cut in [13, 100, MOUSE.len() / 2, MOUSE.len() - 1] {
            assert!(Model::parse(&MOUSE[..cut]).is_none(), "cut at {cut}");
        }
        // An index past the vertices: refused.
        let mut bad = b"OMKM".to_vec();
        bad.extend(1u32.to_le_bytes());
        bad.extend(1u32.to_le_bytes());
        bad.extend(1u16.to_le_bytes());
        bad.push(b'x');
        bad.extend(1.0f32.to_le_bytes());
        bad.extend(0.1f32.to_le_bytes());
        bad.extend(1u32.to_le_bytes());
        bad.extend(3u32.to_le_bytes());
        bad.extend([0u8; 24 + 3]);
        for i in [0u32, 0, 7] {
            bad.extend(i.to_le_bytes());
        }
        assert!(Model::parse(&bad).is_none());
    }

    #[test]
    fn a_key_is_hit_where_it_is_drawn() {
        let caps: Vec<Cap> = (0..4)
            .map(|k| Cap { x: k as f32 * 1.5, y: f32::from(k as u8 % 2), w: 1.0, h: 1.0, label: k.to_string(), to: None, modifier: false })
            .collect();
        let mut board = None;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 300.0));
        Keyboard::get(&mut board, rect, &caps);
        let b = board.as_ref().expect("built");
        for i in 0..caps.len() {
            let t = &b.tops[i];
            let c = t.iter().fold(pos2(0.0, 0.0), |a, p| a + p.to_vec2()) / t.len() as f32;
            assert!(rect.contains(c), "key {i} is drawn inside the area");
            assert_eq!(b.hit(c), Some(i));
        }
        assert_eq!(b.hit(pos2(1.0, 1.0)), None);
        // The same keys in the same place: kept, not built again.
        let mesh = Arc::as_ptr(&b.mesh);
        Keyboard::get(&mut board, rect, &caps);
        assert_eq!(Arc::as_ptr(&board.as_ref().expect("kept").mesh), mesh);
    }
}
