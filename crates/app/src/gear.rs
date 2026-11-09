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

    fn contained(self, points: &[P3], room: Rect) -> Self {
        let bounds = Rect::from_points(&points.iter().map(|point| self.pt(*point)).collect::<Vec<_>>());
        let scale = (room.width() / bounds.width()).min(room.height() / bounds.height()).min(1.0);
        let bounds = Rect::from_min_max(self.centre + (bounds.min - self.centre) * scale, self.centre + (bounds.max - self.centre) * scale);
        let offset = egui::vec2(
            (room.left() - bounds.left()).max(0.0) - (bounds.right() - room.right()).max(0.0),
            (room.top() - bounds.top()).max(0.0) - (bounds.bottom() - room.bottom()).max(0.0),
        );
        Self { foc: self.foc * scale, centre: self.centre + offset, ..self }
    }

    fn framed_horizontally(self, points: &[P3], room: Rect) -> Self {
        let bounds = Rect::from_points(&points.iter().map(|point| self.pt(*point)).collect::<Vec<_>>());
        let offset = if bounds.width() >= room.width() {
            0.0_f32.clamp(room.right() - bounds.right(), room.left() - bounds.left())
        } else {
            room.center().x - bounds.center().x
        };
        Self { centre: self.centre + egui::vec2(offset, 0.0), ..self }
    }

    /// This view fitted to the points `near(g)`: a box round what is
    /// focused, grown by `g` in steps of `grow` until the view magnifies by
    /// no more than `max` of this one.
    #[allow(clippy::too_many_arguments)]
    fn zoom_on(self, base: Cam, near: impl Fn(f32) -> Vec<P3>, rect: Rect, pad: f32, grow: f32, max: f32) -> Cam {
        let mut g = 0.0;
        loop {
            let to = base.fitted(&near(g), rect, pad);
            if to.foc <= self.foc * max || g > grow * 40.0 {
                return to;
            }
            g += grow;
        }
    }
}

/// How far a model or the board is turned: radians, added to the resting view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
}

impl View {
    /// The keyboard as it rests: tipped toward you a little, not turned.
    pub const KEYBOARD_REST: View = View { yaw: 0.0, pitch: 0.4 };

    /// The keyboard turned `yaw` and `pitch` from where it rests.
    pub fn keyboard(yaw: f32, pitch: f32) -> View {
        View { yaw, pitch: Self::KEYBOARD_REST.pitch + pitch }
    }
}

/// The camera's magnification and centre as they are shown, easing toward
/// where they should be. Choosing another button while zoomed on one moves
/// from where the view is now, even part-way through the last move, and
/// speeds up and slows down smoothly (a critically damped spring).
#[derive(Clone, Copy, Debug, Default)]
pub struct Glide {
    /// Natural log of the magnification (so a zoom is even all the way),
    /// then the centre's x and y, each with its speed.
    at: [f32; 3],
    speed: [f32; 3],
    started: bool,
}

impl Glide {
    /// Stiffness: settles in about 0.6 s.
    const W: f32 = 11.0;

    /// The stage the camera looks through moved on the screen without
    /// changing size (the page scrolled a little): the camera moves with
    /// it, so a glide in progress goes on instead of starting over.
    fn shift(&mut self, by: egui::Vec2) {
        if self.started {
            self.at[1] += by.x;
            self.at[2] += by.y;
        }
    }

    /// Moves on by `dt` seconds toward `to`: where the camera is now, and
    /// whether it is still moving. `jump` (or the first time, or reduced
    /// motion) goes straight there.
    fn step(&mut self, to: Cam, dt: f32, jump: bool) -> (Cam, bool) {
        let goal = [to.foc.ln(), to.centre.x, to.centre.y];
        if jump || !self.started || crate::motion::reduced() {
            (self.at, self.speed, self.started) = (goal, [0.0; 3], true);
            return (to, false);
        }
        const H: f32 = 1.0 / 240.0;
        let mut left = dt.clamp(0.0, 0.1);
        while left > 0.0 {
            let h = left.min(H);
            for ((x, v), to) in self.at.iter_mut().zip(self.speed.iter_mut()).zip(goal) {
                *v += (-2.0 * Self::W * *v - Self::W * Self::W * (*x - to)) * h;
                *x += *v * h;
            }
            left -= h;
        }
        // Close enough, and slow enough, to stop: a thousandth of a zoom, a
        // tenth of a pixel.
        let still = (self.at[0] - goal[0]).abs() < 0.001 && (self.at[1] - goal[1]).abs() < 0.1 && (self.at[2] - goal[2]).abs() < 0.1 && self.speed.iter().all(|v| v.abs() < 0.05);
        if still {
            (self.at, self.speed) = (goal, [0.0; 3]);
        }
        (Cam { foc: self.at[0].exp(), centre: pos2(self.at[1], self.at[2]), ..to }, !still)
    }
}

/// A turn pulled by a drag and eased toward where it was pulled, within
/// limits, so letting go leaves it there.
#[derive(Clone, Copy, Debug, Default)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    yaw_v: f32,
    pitch_v: f32,
    yaw_to: f32,
    pitch_to: f32,
    /// How far the drag has pulled, before the limits soften it.
    yaw_pull: f32,
    pitch_pull: f32,
}

/// How far past a limit a drag can pull: dragging back starts to turn
/// again after at most this much of the limit.
const PULL_SLACK: f32 = 1.6;

/// How far a drag turns the keyboard (yaw, pitch in radians), eased into
/// rather than hit. The full swing the Keys view has always had.
pub const KEYBOARD_TURN: (f32, f32) = (0.7, 0.3);
/// The mouse turns further: its side buttons are only seen from the side.
pub const MOUSE_TURN: (f32, f32) = (0.9, 0.3);

/// `pull` turned into a turn that follows it one to one near the resting
/// view and eases into `limit` instead of stopping hard against it.
pub fn soft_limit(pull: f32, limit: f32) -> f32 {
    if limit <= 0.0 { 0.0 } else { limit * (pull / limit).tanh() }
}

impl Orbit {
    /// A drag of `d` points; the turn eases to a stop short of `yaw` and
    /// `pitch` radians from the resting view.
    pub fn pull(&mut self, d: egui::Vec2, yaw: f32, pitch: f32) {
        self.yaw_pull = (self.yaw_pull - d.x * 0.008).clamp(-yaw * PULL_SLACK, yaw * PULL_SLACK);
        self.pitch_pull = (self.pitch_pull + d.y * 0.006).clamp(-pitch * PULL_SLACK, pitch * PULL_SLACK);
        self.yaw_to = soft_limit(self.yaw_pull, yaw);
        self.pitch_to = soft_limit(self.pitch_pull, pitch);
    }

    /// Back to the resting view.
    pub fn reset(&mut self) {
        (self.yaw_to, self.pitch_to, self.yaw_pull, self.pitch_pull) = (0.0, 0.0, 0.0, 0.0);
    }

    /// Moves on by `dt` seconds; true while it is still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        let a = crate::motion::spring(&mut self.yaw, &mut self.yaw_v, self.yaw_to, dt, 0.0005);
        let b = crate::motion::spring(&mut self.pitch, &mut self.pitch_v, self.pitch_to, dt, 0.0005);
        a || b
    }
}

/// Light: the screens behind the desk (cool, from above and behind), a
/// warm lamp in front and to the right, and a little bounce from the desk.
struct Lighting {
    eye: P3,
    rim: f32,
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
        let rim = (1.0 - n.dot(v).max(0.0)).powi(4) * self.rim * n.y.max(0.0).sqrt();
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
    /// Drawn this much nearer than they are, so a part that sits on the
    /// shell wins where the two touch.
    bias: f32,
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
        self.v.push(([pa, pb, pd], cols, (za + zb + zd) / 3.0 - self.bias));
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
/// The brand's Volt: a changed key's new legend, the chosen key or button.
pub const SIGNAL: Color32 = crate::style::SIGNAL;

/// A legend to print on a key: where, how big, which colour, and whether
/// it is the key's new job (printed on its front, in Volt).
struct Legend {
    at: Pos2,
    size: f32,
    text: String,
    to: bool,
    modifier: bool,
    /// The widest it may be drawn: the cap's own width.
    max_w: f32,
}

/// A legend's size for a key `height` points deep on screen: within the
/// readable range, on a half-point grid.
fn legend_size(height: f32) -> f32 {
    (height.clamp(9.0, 15.0) * 2.0).round() / 2.0
}

fn legend_font(painter: &Painter, text: &str, size: f32, minimum: f32, max_w: f32, font: &dyn Fn(f32) -> FontId) -> FontId {
    let fits = |candidate: FontId| painter.layout_no_wrap(text.to_owned(), candidate, Color32::WHITE).size().x <= max_w;
    let mut current = size.max(minimum);
    loop {
        let primary = font(current);
        if fits(primary.clone()) {
            return primary;
        }
        if current <= minimum {
            return primary;
        }
        current = (current - 0.5).max(minimum);
    }
}

fn legend_variants(text: &str) -> &[&str] {
    match text {
        "Bksp" => &["Backspace", "Bksp"],
        "Sup" => &["Super", "Sup"],
        "Mnu" => &["Menu", "Mnu"],
        "Ent" => &["Enter", "Ent"],
        _ => &[],
    }
}

fn adaptive_legend(painter: &Painter, text: &str, size: f32, max_w: f32, font: &dyn Fn(f32) -> FontId) -> Option<(String, FontId)> {
    let variants = legend_variants(text);
    let choices = if variants.is_empty() { std::slice::from_ref(&text) } else { variants };
    for (index, &choice) in choices.iter().enumerate() {
        let minimum = if index + 1 < choices.len() { (size * 0.85).max(9.0) } else { 9.0 };
        let chosen = legend_font(painter, choice, size, minimum, max_w, font);
        if painter.layout_no_wrap(choice.to_owned(), chosen.clone(), Color32::WHITE).size().x <= max_w {
            return Some((choice.to_owned(), chosen));
        }
    }
    None
}

/// The smallest a legend gets while the keyboard is turned: a turned board
/// is fitted smaller, and a legend a little smaller still reads better than
/// none (the resting view keeps them all at [`adaptive_legend`]'s 9).
const TURNED_LEGEND_FLOOR: f32 = 8.0;

/// What a legend shows on the 3D board: as [`adaptive_legend`] would, or,
/// when the board is `turned` and that has no room, its short form a
/// little smaller rather than nothing.
fn board_legend(painter: &Painter, text: &str, size: f32, max_w: f32, turned: bool) -> Option<(String, FontId)> {
    adaptive_legend(painter, text, size, max_w, &crate::style::mono).or_else(|| {
        let short = legend_variants(text).last().copied().unwrap_or(text);
        let font = legend_font(painter, short, size, TURNED_LEGEND_FLOOR, max_w, &crate::style::mono);
        (turned && painter.layout_no_wrap(short.to_owned(), font.clone(), Color32::WHITE).size().x <= max_w).then(|| (short.to_owned(), font))
    })
}

/// A built keyboard: the lit mesh, each key's top on screen (for clicks
/// and highlights) and its legends.
pub struct Keyboard {
    flat: bool,
    returning: bool,
    built_for: (Rect, Vec<Cap>, View, Option<usize>),
    /// Where the camera is headed, and whether it was still moving when this
    /// was built.
    target: Cam,
    moving: bool,
    mesh: Arc<Mesh>,
    pub tops: Vec<Vec<Pos2>>,
    legends: Vec<Legend>,
    #[cfg(test)]
    case_bounds: Rect,
}

impl Keyboard {
    #[cfg(test)]
    pub fn working(slot: &mut Option<Keyboard>, rect: Rect, caps: &[Cap]) {
        if slot.as_ref().is_some_and(|board| board.flat && board.built_for.0 == rect && board.built_for.1 == caps) { return; }
        let columns = caps.iter().map(|cap| cap.x + cap.w).fold(1.0f32, f32::max);
        let rows = caps.iter().map(|cap| cap.y + cap.h).fold(1.0f32, f32::max);
        let unit = ((rect.width() - 16.0) / columns).min((rect.height() - 16.0) / rows);
        let origin = rect.center() - egui::vec2(columns * unit, rows * unit) / 2.0;
        let mut tops = Vec::with_capacity(caps.len());
        let mut legends = Vec::new();
        for cap in caps {
            let key = Rect::from_min_size(origin + egui::vec2(cap.x * unit, cap.y * unit), egui::vec2(cap.w * unit, cap.h * unit)).shrink(2.0);
            tops.push(vec![key.left_top(), key.right_top(), key.right_bottom(), key.left_bottom()]);
            let lines: Vec<&str> = vec![&cap.label];
            let size = if lines.len() > 1 { (unit * 0.29).clamp(11.0, 12.0) } else { (unit * 0.36).clamp(11.0, 15.0) };
            for (index, text) in lines.iter().enumerate() {
                legends.push(Legend { at: key.center() + egui::vec2(0.0, (index as f32 - (lines.len() - 1) as f32 / 2.0) * (size + 1.0)), size, text: (*text).to_owned(), to: cap.to.is_some(), modifier: cap.modifier, max_w: key.width() - 6.0 });
            }
        }
        let target = Self::aim(rect, caps, View::default(), None);
        *slot = Some(Keyboard { #[cfg(test)] case_bounds: rect, flat: true, returning: false, built_for: (rect, caps.to_vec(), View::default(), None), target, moving: false, mesh: Arc::new(Mesh::default()), tops, legends });
    }

    /// The keyboard for `caps` filling `rect`, turned by `view`, and zoomed
    /// on the key `focus` (or the whole board when `None`): the last one when
    /// nothing changed, else built anew. The camera eases there by `glide`;
    /// true while it is still moving, so the caller keeps drawing.
    pub fn get(slot: &mut Option<Keyboard>, rect: Rect, caps: &[Cap], view: View, focus: Option<usize>, glide: &mut Glide, dt: f32) -> bool {
        let same = slot.as_ref().filter(|k| !k.flat && k.built_for.0 == rect && k.built_for.1 == caps && k.built_for.2 == view && k.built_for.3 == focus);
        let mut step = None;
        if let Some(k) = same {
            let s = glide.step(k.target, dt, false);
            if !s.1 && !k.moving {
                return false;
            }
            step = Some((k.target, s));
        }
        let (target, (cam, moving)) = step.unwrap_or_else(|| {
            let target = Self::aim(rect, caps, view, focus);
            // Only a stage of another size starts the camera over. One that
            // merely moved (the page scrolled a few points as a key was
            // chosen) takes the camera along, so the zoom still glides.
            let before = slot.as_ref().map(|k| k.built_for.0);
            let resized = before.is_none_or(|old| old.size() != rect.size());
            if let Some(old) = before.filter(|old| !resized && old.min != rect.min) {
                glide.shift(rect.min - old.min);
            }
            (target, glide.step(target, dt, resized))
        });
        // Orientation follows the orbit while scale/position ease toward it.
        // Keep the case inside the overview even during that interpolation.
        // A detail-to-overview glide deliberately starts enlarged. Let it
        // finish before enforcing overview bounds, preserving the zoom out.
        let returning = focus.is_none() && moving && slot.as_ref().is_some_and(|board| board.built_for.3.is_some() || board.returning);
        let cam = if focus.is_none() && !returning { cam.contained(&Self::case_corners(caps), rect.shrink(3.0)) } else { cam };
        let mut board = Self::build(rect, caps, view, focus, cam, target, moving);
        board.returning = returning;
        *slot = Some(board);
        moving
    }

    fn case_corners(caps: &[Cap]) -> Vec<P3> {
        let half_w = caps.iter().map(|cap| cap.x + cap.w).fold(1.0f32, f32::max) * U / 2.0 + 9.0;
        let half_d = caps.iter().map(|cap| cap.y + cap.h).fold(1.0f32, f32::max) * U / 2.0 + 9.0;
        [-half_w, half_w].into_iter().flat_map(|x| {
            [-16.0, 8.0].into_iter().flat_map(move |y| [-half_d, half_d].into_iter().map(move |z| p3(x, y, z)))
        }).collect()
    }

    /// The camera for the whole board, or on the key `focus`.
    fn aim(rect: Rect, caps: &[Cap], view: View, focus: Option<usize>) -> Cam {
        let cols = caps.iter().map(|c| c.x + c.w).fold(1.0f32, f32::max);
        let rows = caps.iter().map(|c| c.y + c.h).fold(1.0f32, f32::max);
        let (w, d) = (cols * U, rows * U);
        let (x0, z0) = (-w / 2.0, -d / 2.0);
        let base = Cam::new(p3(0.0, 0.0, 0.0), -0.02 + view.yaw, 0.92 + view.pitch, 2000.0, 1.0, rect.center());
        let bounds = Self::case_corners(caps);
        let whole = base.fitted(&bounds, rect, 3.0);
        match focus.and_then(|i| caps.get(i)) {
            Some(k) => {
                let (cx, cz) = (x0 + (k.x + k.w / 2.0) * U, z0 + (k.y + k.h / 2.0) * U);
                let near = |g: f32| -> Vec<P3> {
                    let (hx, hz) = (k.w * U / 2.0 + 2.2 * U + g, k.h * U / 2.0 + 1.6 * U + g);
                    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|(sx, sz)| p3(cx + sx * hx, 7.0, cz + sz * hz)).collect()
                };
                whole.zoom_on(base, near, rect, 24.0, 0.4 * U, 1.8).framed_horizontally(&bounds, rect.expand(16.0))
            }
            None => whole,
        }
    }

    fn build(rect: Rect, caps: &[Cap], view: View, focus: Option<usize>, cam: Cam, target: Cam, moving: bool) -> Keyboard {
        let cols = caps.iter().map(|c| c.x + c.w).fold(1.0f32, f32::max);
        let rows = caps.iter().map(|c| c.y + c.h).fold(1.0f32, f32::max);
        let (w, d) = (cols * U, rows * U);
        let margin = 9.0;
        let (x0, z0) = (-w / 2.0, -d / 2.0);
        let lit = Lighting { eye: cam.eye, rim: 0.12 };
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
            let half_depth = k.h * U / 2.0 - 3.05;
            let front = cam.pt(p3(cx, y_top, cz - 0.6 - half_depth));
            let back = cam.pt(p3(cx, y_top, cz - 0.6 + half_depth));
            // In half points: every size is a font the painter rasterizes
            // once, so a turn reuses a handful instead of making new ones
            // each frame (which stalled the first turn for most of a second).
            let size = legend_size((back.y - front.y).abs() * 0.72);
            let left = cam.pt(p3(cx - k.w * U / 2.0 + 2.2, y_top, cz - 0.6));
            let right = cam.pt(p3(cx + k.w * U / 2.0 - 2.2, y_top, cz - 0.6));
            let at = cam.pt(p3(cx, y_top, cz - 0.6));
            let max_w = (2.0 * (at.x - left.x).min(right.x - at.x)).max(1.0);
            legends.push(Legend { at, size, text: k.label.clone(), to: k.to.is_some(), modifier: k.modifier, max_w });
        }
        keys.into_mesh(&mut m);
        Keyboard { #[cfg(test)] case_bounds: Rect::from_points(&Self::case_corners(caps).iter().map(|point| cam.pt(*point)).collect::<Vec<_>>()), flat: false, returning: false, built_for: (rect, caps.to_vec(), view, focus), target, moving, mesh: Arc::new(m), tops, legends }
    }

    /// Draws the kept mesh, then what changes from frame to frame: the
    /// keys held (darker, as if pressed), the hovered key, the chosen one
    /// with a light outline, and the legends.
    pub fn paint(&self, p: &Painter, held: &[usize], hovered: Option<usize>, chosen: Option<usize>, font: &dyn Fn(f32, bool) -> FontId) {
        p.add(Shape::Mesh(self.mesh.clone()));
        if self.flat {
            for key in &self.tops {
                p.rect(Rect::from_points(key), egui::CornerRadius::same(4), crate::style::BUTTON, Stroke::new(1.0, crate::style::KEY_EDGE), egui::StrokeKind::Inside);
            }
        }
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
        for l in self.legends.iter().filter(|legend| !legend.text.is_empty()) {
            let col = if l.modifier { Color32::from_rgb(0xD9, 0xD3, 0xC8) } else { Color32::from_rgb(0xC6, 0xCB, 0xD3) };
            let base_font = |size| font(size, l.to);
            let (text, chosen_font) = if self.flat {
                let chosen = legend_font(p, &l.text, l.size, 11.0, l.max_w, &base_font);
                (crate::style::fit(p, &l.text, &chosen, l.max_w), chosen)
            } else if let Some(legend) = board_legend(p, &l.text, l.size, l.max_w, self.built_for.2 != View::KEYBOARD_REST) {
                legend
            } else {
                continue;
            };
            if self.flat {
                p.text(l.at, Align2::CENTER_CENTER, text, chosen_font, col);
            } else {
                let galley = p.layout_no_wrap(text, chosen_font, col);
                p.galley(l.at - galley.mesh_bounds.center().to_vec2(), galley, col);
            }
        }
        for (index, _) in self.built_for.1.iter().enumerate().filter(|(_, cap)| cap.to.is_some()) {
            if let Some(top) = self.tops.get(index).filter(|top| top.len() > 2) {
                let mut outline = top.clone();
                outline.push(outline[0]);
                p.add(Shape::line(outline, Stroke::new(1.0, SIGNAL.gamma_multiply(0.55))));
            }
            let bounds = self.bounds(index);
            p.line_segment([bounds.center_bottom() + egui::vec2(-4.0, -3.0), bounds.center_bottom() + egui::vec2(4.0, -3.0)], Stroke::new(2.0, SIGNAL));
        }
        if let Some(t) = chosen.and_then(|i| self.tops.get(i)).filter(|t| t.len() > 2) {
            let mut ring = t.clone();
            ring.push(ring[0]);
            p.add(Shape::line(ring, Stroke::new(2.0, crate::style::INK)));
        }
    }

    /// The key under `pos`, front keys first.
    pub fn hit(&self, pos: Pos2) -> Option<usize> {
        (0..self.tops.len()).rev().find(|&i| inside(pos, &self.tops[i])).or_else(|| {
            (0..self.tops.len()).filter(|&index| self.target_bounds(index).contains(pos))
                .min_by(|&first, &second| self.bounds(first).center().distance_sq(pos).total_cmp(&self.bounds(second).center().distance_sq(pos)))
        })
    }

    /// A key's outline's bounding box (for focus rings and screen readers).
    pub fn bounds(&self, i: usize) -> Rect {
        self.tops.get(i).map_or(Rect::NOTHING, |t| Rect::from_points(t))
    }

    pub fn target_bounds(&self, index: usize) -> Rect {
        let bounds = self.bounds(index);
        Rect::from_center_size(bounds.center(), bounds.size().max(egui::vec2(24.0, 24.0)))
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
    let shoulder = rring(cx, cz - 0.5, hw - 1.0, hd - 2.4, 2.4, y0 + h - 0.9);
    let top = rring(cx, cz - 0.6, hw - 1.1, hd - 2.5, 2.2, y0 + h);
    let c = p3(cx, y0, cz);
    walls(t, cam, lit, f, &base, &shoulder, c);
    walls(t, cam, lit, f, &shoulder, &top, c);
    // The dish: a shallow cylinder across the key, lowest in the middle.
    let dish = 0.55;
    let (thw, thd) = (hw - 1.1, hd - 2.5);
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

/// The longest side a model's triangle keeps, in millimetres. Triangles are
/// drawn back to front by their middle, which only holds for small ones: a
/// long triangle of the shell reaches under a button and covers part of it.
const MAX_EDGE: f32 = 4.0;

impl ModelPart {
    /// The same surface with every triangle cut (at the middle of its
    /// longest side, normals and colours blended) until its sides are short.
    fn refined(self, max: f32) -> ModelPart {
        type V = (P3, P3, Color32);
        let len = |a: &V, b: &V| {
            let d = a.0 - b.0;
            d.dot(d).sqrt()
        };
        let mid = |a: &V, b: &V| -> V { (lerp3(a.0, b.0, 0.5), lerp3(a.1, b.1, 0.5).norm(), lerp_col(a.2, b.2, 0.5)) };
        let (mut pos, mut nrm, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut todo: Vec<([V; 3], u8)> = Vec::new();
        for tri in self.idx.as_chunks::<3>().0 {
            todo.push((tri.map(|k| (self.pos[k as usize], self.nrm[k as usize], self.col[k as usize])), 0));
            while let Some((t, depth)) = todo.pop() {
                let sides = [len(&t[0], &t[1]), len(&t[1], &t[2]), len(&t[2], &t[0])];
                let long = (0..3).max_by(|a, b| sides[*a].total_cmp(&sides[*b])).unwrap_or(0);
                if sides[long] <= max || depth >= 16 {
                    for v in t {
                        idx.push(pos.len() as u32);
                        pos.push(v.0);
                        nrm.push(v.1);
                        col.push(v.2);
                    }
                    continue;
                }
                // Cut side `long` (between corners long and long + 1) in two.
                let (a, b, c) = (t[long], t[(long + 1) % 3], t[(long + 2) % 3]);
                let m = mid(&a, &b);
                todo.push(([a, m, c], depth + 1));
                todo.push(([m, b, c], depth + 1));
            }
        }
        ModelPart { pos, nrm, col, idx, ..self }
    }
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
            parts.push(ModelPart { name, gloss, spec, pos, nrm, col, idx }.refined(MAX_EDGE));
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
    built_for: (Rect, Rect, Option<String>, [u32; 3], Option<String>),
    /// Where the camera is headed, and whether it was still moving when this
    /// was built.
    target: Cam,
    moving: bool,
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
    /// tinted Volt, and the camera goes in on the part `focus` (or stays out
    /// on the whole mouse when `None`), easing there by `glide`. The last
    /// one when nothing changed; true while the camera is still moving.
    #[allow(clippy::too_many_arguments)]
    pub fn get(slot: &mut Option<ModelView>, m: &Model, rect: Rect, room: Rect, yaw: f32, cam_yaw: f32, pitch: f32, chosen: Option<&str>, focus: Option<&str>, glide: &mut Glide, dt: f32) -> bool {
        let key = (rect, room, chosen.map(str::to_owned), [yaw.to_bits(), cam_yaw.to_bits(), pitch.to_bits()], focus.map(str::to_owned));
        let same = slot.as_ref().filter(|v| v.built_for == key);
        let mut step = None;
        if let Some(v) = same {
            let s = glide.step(v.target, dt, false);
            if !s.1 && !v.moving {
                return false;
            }
            step = Some((v.target, s));
        }
        let resized = slot.as_ref().is_none_or(|v| v.built_for.0 != rect || v.built_for.1 != room);
        let pts: Vec<P3> = m.points().map(|p| p.yaw(yaw)).collect();
        let (mut lo, mut hi) = (p3(f32::MAX, f32::MAX, f32::MAX), p3(f32::MIN, f32::MIN, f32::MIN));
        for p in &pts {
            lo = p3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = p3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        let mid = (lo + hi) * 0.5;
        let (target, (cam, moving)) = step.unwrap_or_else(|| {
            let base = Cam::new(mid, cam_yaw, pitch, 420.0, 1.0, rect.center());
            let whole = base.fitted(&pts, room, room.height() * 0.16);
            // Zoomed: the part and a little of what is round it fill the room.
            let target = match focus.and_then(|f| m.parts.iter().find(|p| p.name == f)) {
                Some(part) => {
                    let q: Vec<P3> = part.pos.iter().map(|p| p.yaw(yaw)).collect();
                    let (mut a, mut b) = (p3(f32::MAX, f32::MAX, f32::MAX), p3(f32::MIN, f32::MIN, f32::MIN));
                    for p in &q {
                        a = p3(a.x.min(p.x), a.y.min(p.y), a.z.min(p.z));
                        b = p3(b.x.max(p.x), b.y.max(p.y), b.z.max(p.z));
                    }
                    let corners = |g: f32| -> Vec<P3> {
                        let g = 12.0 + g;
                        (0..8).map(|i| p3(if i & 1 == 0 { a.x - g } else { b.x + g }, if i & 2 == 0 { a.y } else { b.y + g }, if i & 4 == 0 { a.z - g } else { b.z + g })).collect()
                    };
                    // Not so close that the mouse spills over its labels.
                    whole.zoom_on(base, corners, room, room.height() * 0.08, 6.0, 1.5).contained(&pts, room.shrink(8.0))
                }
                None => whole,
            };
            (target, glide.step(target, dt, resized))
        });
        let lit = Lighting { eye: cam.eye, rim: 0.26 };
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
            // The buttons sit on the shell (named Material.…).
            t.bias = if part.name.starts_with("Material") { 0.0 } else { 0.6 };
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
        *slot = Some(ModelView { built_for: key, target, moving, mesh: Arc::new(mesh), parts });
        moving
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
    fn adaptive_legends_prefer_full_names_then_readable_short_names() {
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.painter();
            let width = |text: &str| painter.layout_no_wrap(text.to_owned(), crate::style::regular(9.0), Color32::WHITE).size().x;
            let wide = width("Backspace") + 1.0;
            let short = width("Bksp") + 0.1;
            let full = adaptive_legend(painter, "Bksp", 9.0, wide, &crate::style::regular).unwrap();
            assert_eq!(full.0, "Backspace");
            let compact = adaptive_legend(painter, "Bksp", 9.0, short, &crate::style::regular).unwrap();
            assert_eq!(compact.0, "Bksp");
            assert_eq!(compact.1.size, 9.0);
            assert!(adaptive_legend(painter, "Bksp", 9.0, 1.0, &crate::style::regular).is_none());
        });
        output.textures_delta.clear();
    }

    #[test]
    fn long_legends_do_not_shrink_far_below_their_short_form() {
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.painter();
            for (short, full) in [("Bksp", "Backspace"), ("Sup", "Super"), ("Ent", "Enter")] {
                let room = painter.layout_no_wrap(full.to_owned(), crate::style::mono(9.0), Color32::WHITE).size().x + 0.1;
                let chosen = adaptive_legend(painter, short, 14.0, room, &crate::style::mono).unwrap();
                assert_eq!(chosen.0, short);
                assert!(chosen.1.size >= 11.5, "{short}: {:?}", chosen.1);
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn horizontal_framing_preserves_zoom_and_avoids_empty_edges() {
        let room = Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 200.0));
        let points = [p3(-300.0, 0.0, 0.0), p3(300.0, 0.0, 0.0)];
        for offset in [-250.0, 250.0] {
            let camera = Cam::new(p3(0.0, 0.0, 0.0), 0.0, 0.9, 760.0, 760.0, room.center() + egui::vec2(offset, 0.0));
            let framed = camera.framed_horizontally(&points, room);
            let bounds = Rect::from_points(&points.iter().map(|point| framed.pt(*point)).collect::<Vec<_>>());
            assert!(bounds.left() <= room.left() + 0.01);
            assert!(bounds.right() >= room.right() - 0.01);
            assert_eq!(framed.foc, camera.foc);
            assert_eq!(framed.centre.y, camera.centre.y);
        }
    }

    #[test]
    fn legend_font_keeps_its_face_and_readability_floor() {
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.painter();
            let width = |font: FontId| painter.layout_no_wrap("Num".to_owned(), font, Color32::WHITE).size().x;
            let available = width(crate::style::mono(10.0));
            let chosen = legend_font(painter, "Num", 15.0, 9.0, available, &crate::style::mono);
            assert_eq!(chosen, crate::style::mono(10.0));
            assert!(width(chosen) <= available);
            assert_eq!(legend_font(painter, "Num", 15.0, 9.0, 1.0, &crate::style::mono), crate::style::mono(9.0));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn single_glyph_legends_use_the_projected_cap_height() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for width in [560.0, 660.0, 900.0] {
                let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 220.0));
                let mut board = None;
                Keyboard::get(&mut board, rect, &caps, View { yaw: 0.0, pitch: 0.4 }, None, &mut Glide::default(), 0.0);
                let board = board.expect("3D board");
                for legend in board.legends.iter().filter(|legend| legend.text.chars().count() == 1 && !legend.text.trim().is_empty()) {
                    let (text, font) = adaptive_legend(ui.painter(), &legend.text, legend.size, legend.max_w, &crate::style::mono).expect("readable glyph");
                    assert!(font.size >= 10.0, "{} is too small at {width}: {}", text, font.size);
                    let galley = ui.painter().layout_no_wrap(text.clone(), font, Color32::WHITE);
                    let bounds = Rect::from_center_size(legend.at, galley.size());
                    let top = board.tops.iter().find(|top| inside(legend.at, top)).expect("legend's cap");
                    for corner in [bounds.left_top(), bounds.right_top(), bounds.left_bottom(), bounds.right_bottom()] {
                        assert!(inside(corner, top), "{text} exceeds its cap at {width}: {bounds:?} within {:?}", Rect::from_points(top));
                    }
                }
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn page_navigation_legends_fit_the_medium_3d_board() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(660.0, 220.0));
        let mut board = None;
        Keyboard::get(&mut board, rect, &caps, View { yaw: 0.0, pitch: 0.4 }, None, &mut Glide::default(), 0.0);
        let board = board.expect("3D board");
        let ctx = egui::Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for text in ["PgUp", "PgDn"] {
                let legend = board.legends.iter().find(|legend| legend.text == text).expect("page key legend");
                let font = legend_font(ui.painter(), text, legend.size, 9.0, legend.max_w, &crate::style::mono);
                let width = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x;
                assert!(width <= legend.max_w, "{text} must fit without an ellipsis");
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn whole_board_legends_remain_readable_without_mixed_fonts() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        let context = egui::Context::default();
        crate::style::install(&context);
        let mut failures = Vec::new();
        let mut output = context.run_ui(Default::default(), |ui| {
            for width in [560.0, 660.0, 724.0, 900.0] {
                let (columns, rows) = crate::tune::keyboard_size(true);
                let inner = width - 36.0;
                let height = (rows * (inner / columns).min(42.0) + 32.0).max(24.0 + inner * 0.36);
                let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(width - 16.0, height - 32.0));
                let mut board = None;
                Keyboard::get(&mut board, rect, &caps, View { yaw: 0.0, pitch: 0.4 }, None, &mut Glide::default(), 0.0);
                let board = board.unwrap();
                for legend in board.legends.iter().filter(|legend| !legend.text.is_empty()) {
                    if let Some((text, font)) = adaptive_legend(ui.painter(), &legend.text, legend.size, legend.max_w, &crate::style::mono) {
                        assert!(font.size >= 9.0);
                        assert_eq!(font.family, egui::FontFamily::Monospace);
                        assert!(!text.contains('…'));
                        let galley = ui.painter().layout_no_wrap(text.clone(), font, Color32::WHITE);
                        let bounds = galley.mesh_bounds.translate(legend.at.to_vec2() - galley.mesh_bounds.center().to_vec2());
                        let top = board.tops.iter().find(|top| inside(legend.at, top)).expect("legend's cap");
                        for corner in [bounds.left_top(), bounds.right_top(), bounds.left_bottom(), bounds.right_bottom()] {
                            assert!(inside(corner, top), "{text} exceeds its cap at {width}: {bounds:?}");
                        }
                    } else {
                        let needed = ui.painter().layout_no_wrap(legend.text.clone(), crate::style::mono(9.0), Color32::WHITE).size().x;
                        failures.push(format!("{} at {width}: {:.1}px available, {needed:.1}px needed", legend.text, legend.max_w));
                    }
                }
            }
        });
        output.textures_delta.clear();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn a_turn_eases_into_its_limit_and_comes_straight_back() {
        // One to one near the resting view, never past the limit.
        assert!((soft_limit(0.02, 0.22) - 0.02).abs() < 0.001);
        assert!(soft_limit(0.22 * PULL_SLACK, 0.22) < 0.22 && soft_limit(-0.22 * PULL_SLACK, 0.22) > -0.22 && soft_limit(10.0, 0.22) <= 0.22);
        let mut previous = -1.0;
        for step in -40..=40 {
            let turn = soft_limit(step as f32 * 0.02, 0.22);
            assert!(turn > previous, "always further the further it is pulled");
            previous = turn;
        }
        // A long drag (240 points) stops short of the limit, softly: each
        // further point turns it less than the one before.
        let mut orbit = Orbit::default();
        let mut turns = Vec::new();
        for _ in 0..24 {
            orbit.pull(egui::vec2(-10.0, 0.0), KEYBOARD_TURN.0, KEYBOARD_TURN.1);
            turns.push(orbit.yaw_to);
        }
        assert!(turns.last().is_some_and(|t| *t < KEYBOARD_TURN.0));
        assert!(turns.windows(3).all(|w| w[2] - w[1] <= w[1] - w[0] + 1e-6), "{turns:?}");
        // Dragging back turns it back within a short stretch, however far it was pulled.
        let far = orbit.yaw_to;
        for _ in 0..3 {
            orbit.pull(egui::vec2(10.0, 0.0), KEYBOARD_TURN.0, KEYBOARD_TURN.1);
        }
        assert!(orbit.yaw_to < far - 0.01, "{far} -> {}", orbit.yaw_to);
        orbit.reset();
        orbit.pull(egui::vec2(-10.0, 0.0), KEYBOARD_TURN.0, KEYBOARD_TURN.1);
        assert!((orbit.yaw_to - 0.08).abs() < 0.01, "a reset forgets the pull: {}", orbit.yaw_to);
    }

    #[test]
    fn the_keyboard_turned_a_natural_amount_keeps_every_legend() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        let context = egui::Context::default();
        crate::style::install(&context);
        // A natural turn, a third of the full swing. The very end of the swing is for
        // looking at the keyboard as a solid thing: small legends may drop out there.
        let (yaw, pitch) = (KEYBOARD_TURN.0 * 0.3, KEYBOARD_TURN.1 * 0.3);
        let mut lost = Vec::new();
        let mut output = context.run_ui(Default::default(), |ui| {
            // The keyboard card in the default window (1100 points) and a wide one.
            for width in [780.0, 900.0] {
                let (columns, rows) = crate::tune::keyboard_size(true);
                let inner = width - 36.0;
                let height = (rows * (inner / columns).min(42.0) + 32.0).max(24.0 + inner * 0.36);
                let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(width - 16.0, height - 32.0));
                for (yaw, pitch) in [(yaw, pitch), (yaw, 0.0), (yaw, -pitch), (-yaw, pitch), (-yaw, 0.0), (-yaw, -pitch)] {
                    let mut board = None;
                    Keyboard::get(&mut board, rect, &caps, View::keyboard(yaw, pitch), None, &mut Glide::default(), 0.0);
                    let board = board.unwrap();
                    for legend in board.legends.iter().filter(|legend| !legend.text.is_empty()) {
                        match board_legend(ui.painter(), &legend.text, legend.size, legend.max_w, true) {
                            Some((_, font)) => assert!(font.size >= TURNED_LEGEND_FLOOR),
                            None => lost.push(format!("{} at {width}, turned {yaw:.2}/{pitch:.2}", legend.text)),
                        }
                    }
                }
            }
        });
        output.textures_delta.clear();
        assert!(lost.is_empty(), "{}", lost.join("\n"));
    }

    #[test]
    fn legends_come_in_a_few_sizes() {
        // Each size is a font to rasterize: a turn must not make new ones every frame.
        assert_eq!(legend_size(4.0), 9.0);
        assert_eq!(legend_size(30.0), 15.0);
        assert_eq!(legend_size(11.26), 11.5);
        assert_eq!(legend_size(11.24), 11.0);
    }

    #[test]
    fn the_built_in_mouse_has_its_five_buttons() {
        let m = Model::parse(MOUSE).expect("the built-in mouse parses");
        for n in ["left", "right", "wheel", "back", "forward"] {
            assert!(m.parts.iter().any(|p| p.name == n), "{n} missing");
        }
    }

    /// A part's bounding box: low and high corners.
    fn bounds(m: &Model, name: &str) -> (P3, P3) {
        let part = m.parts.iter().find(|p| p.name == name).expect("the part is there");
        let (mut lo, mut hi) = (p3(f32::MAX, f32::MAX, f32::MAX), p3(f32::MIN, f32::MIN, f32::MIN));
        for p in &part.pos {
            lo = p3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = p3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        (lo, hi)
    }

    /// Long triangles are drawn in the wrong order against the ones they
    /// overlap (a bare shell covered half a button), so none are left, and
    /// cutting them keeps the mouse a size that draws quickly.
    #[test]
    fn no_triangle_of_the_mouse_is_long() {
        let m = Model::parse(MOUSE).expect("parses");
        let mut count = 0;
        for part in &m.parts {
            for t in part.idx.as_chunks::<3>().0 {
                count += 1;
                for k in 0..3 {
                    let d = part.pos[t[k] as usize] - part.pos[t[(k + 1) % 3] as usize];
                    assert!(d.dot(d).sqrt() <= MAX_EDGE + 1e-3, "{} has a side of {} mm", part.name, d.dot(d).sqrt());
                }
            }
        }
        assert!(count < 40_000, "{count} triangles");
    }

    #[test]
    fn the_mouse_buttons_are_the_real_buttons() {
        let m = Model::parse(MOUSE).expect("parses");
        let (ll, lh) = bounds(&m, "left");
        let (rl, rh) = bounds(&m, "right");
        // Each button is its own piece, on its own side, with a gap between.
        assert!(lh.x <= 0.5 && rl.x >= 10.0, "left x {}..{}, right x {}..{}", ll.x, lh.x, rl.x, rh.x);
        // Whole buttons, not slivers: long, and as tall as the shell.
        assert!(lh.z - ll.z > 50.0 && rh.z - rl.z > 50.0);
        assert!(lh.y > 30.0 && rh.y > 30.0);
        // The wheel sits between them, and the thumb buttons on the left side.
        let (wl, wh) = bounds(&m, "wheel");
        assert!(wl.x > lh.x - 2.0 && wh.x < rl.x + 2.0);
        for n in ["forward", "back"] {
            assert!(bounds(&m, n).1.x < -10.0, "{n} is on the left side");
        }
    }

    /// A real board's keys, and the area they are drawn in.
    fn real_board() -> (Vec<Cap>, Rect) {
        let caps = (0..75).map(|k| Cap { x: (k % 15) as f32, y: (k / 15) as f32, w: 1.0, h: 1.0, label: k.to_string(), to: None, modifier: false }).collect();
        (caps, Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(900.0, 360.0)))
    }

    /// One 60 Hz frame of a board looking at `focus`: where key `look` is on screen.
    fn frame(board: &mut Option<Keyboard>, glide: &mut Glide, focus: Option<usize>, look: usize) -> (Rect, bool) {
        let (caps, rect) = real_board();
        let moving = Keyboard::get(board, rect, &caps, View::default(), focus, glide, 1.0 / 60.0);
        (board.as_ref().expect("built").bounds(look), moving)
    }

    #[test]
    fn zooming_on_a_key_brings_it_up_and_zooming_out_brings_it_back() {
        let (mut b, mut g) = (None, Glide::default());
        let whole = frame(&mut b, &mut g, None, 37).0;
        let mut seen = vec![];
        loop {
            let (r, moving) = frame(&mut b, &mut g, Some(37), 37);
            seen.push(r.width());
            if !moving {
                break;
            }
            assert!(seen.len() < 200, "it never settles");
        }
        let close = *seen.last().unwrap();
        assert!(close > whole.width() * 1.4, "{close} vs {}", whole.width());
        assert!(seen.windows(2).all(|w| w[1] >= w[0] - 0.01), "it only grows on the way in");
        assert!(seen.len() > 12 && seen[seen.len() / 2] > whole.width() && seen[seen.len() / 2] < close, "it takes a while, passing through the middle");
        let (_, rect) = real_board();
        let r = frame(&mut b, &mut g, Some(37), 37).0;
        assert!(r.center().distance(rect.center()) < whole.center().distance(rect.center()) + 1.0, "the key comes toward the middle");
        // Let go: the same view as before, reached by easing, not at once.
        let (first, _) = frame(&mut b, &mut g, None, 37);
        assert!(first.width() > whole.width() * 1.5, "the first frame out is still close");
        while frame(&mut b, &mut g, None, 37).1 {}
        assert!((frame(&mut b, &mut g, None, 37).0.width() - whole.width()).abs() < 0.01);
    }

    #[test]
    fn selected_edge_keys_keep_padding_in_a_short_editor_stage() {
        let (caps, _) = real_board();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(860.0, 166.0));
        for selected in [0, 14, 30, 60, 74] {
            let (mut board, mut glide) = (None, Glide::default());
            Keyboard::get(&mut board, rect, &caps, View::default(), Some(selected), &mut glide, 1.0 / 60.0);
            let bounds = board.as_ref().unwrap().bounds(selected);
            assert!(rect.shrink(20.0).contains_rect(bounds), "key {selected}: {bounds:?}");
            assert!((bounds.center().y - rect.center().y).abs() < 32.0, "key {selected} is not vertically centered");
            assert!(bounds.width() < rect.width() / 4.0, "board context must remain visible");
        }
    }

    #[test]
    fn choosing_the_next_key_glides_there_from_where_the_view_is() {
        let (mut b, mut g) = (None, Glide::default());
        while frame(&mut b, &mut g, Some(37), 40).1 {}
        let from = frame(&mut b, &mut g, Some(37), 40).0;
        // Tap the one beside it, then another part-way through: each frame
        // moves the view by a little, never a jump.
        let mut last = from.center();
        let mut biggest = 0.0f32;
        let mut frames = 0;
        for n in 0.. {
            let focus = if n < 12 { 38 } else { 39 };
            let (r, moving) = frame(&mut b, &mut g, Some(focus), 40);
            biggest = biggest.max(r.center().distance(last));
            last = r.center();
            frames += 1;
            if !moving && n >= 12 {
                break;
            }
            assert!(n < 300, "it never settles");
        }
        let to = frame(&mut b, &mut g, Some(39), 40).0;
        assert!(from.center().distance(to.center()) > 100.0, "the keys are far enough apart to see a jump");
        assert!(biggest < 45.0, "moved {biggest} px in one frame");
        assert!(frames > 24, "it took {frames} frames");
    }

    /// Choosing a key can scroll the page a few points, which moves the stage
    /// on the screen. That must not turn the zoom into a snap.
    #[test]
    fn a_stage_that_moves_a_little_does_not_turn_the_glide_into_a_snap() {
        let (caps, rect) = real_board();
        let (mut b, mut g) = (None, Glide::default());
        Keyboard::get(&mut b, rect, &caps, View::default(), None, &mut g, 1.0 / 60.0);
        let moved = rect.translate(egui::vec2(0.0, -10.0));
        let mut positions = vec![];
        for _ in 0..12 {
            Keyboard::get(&mut b, moved, &caps, View::default(), Some(37), &mut g, 1.0 / 60.0);
            positions.push(b.as_ref().expect("built").bounds(37).center());
        }
        let steps = positions.windows(2).map(|w| w[0].distance(w[1])).collect::<Vec<_>>();
        let biggest = steps.iter().cloned().fold(0.0, f32::max);
        let total: f32 = steps.iter().sum();
        assert!(biggest < total * 0.5, "one frame did most of the move (a snap): {steps:?}");
        assert!(steps.iter().filter(|s| **s > 0.5).count() >= 8, "it glides over many frames: {steps:?}");
        // A stage of another size does start over.
        let bigger = Rect::from_min_size(rect.min, rect.size() + egui::vec2(40.0, 20.0));
        Keyboard::get(&mut b, bigger, &caps, View::default(), Some(37), &mut g, 1.0 / 60.0);
        assert!(!g.speed.iter().any(|v| *v != 0.0), "a resize goes straight there");
    }

    #[test]
    fn choosing_the_next_mouse_button_glides_there_too() {
        let m = Model::parse(MOUSE).expect("parses");
        let room = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(500.0, 300.0));
        let (mut view, mut g) = (None, Glide::default());
        let mut at = |focus: Option<&str>| {
            let moving = ModelView::get(&mut view, &m, room, room, 0.0, -2.2, 0.62, focus, focus, &mut g, 1.0 / 60.0);
            (view.as_ref().expect("built").parts.iter().find(|p| p.name == "wheel").expect("wheel").anchor, moving)
        };
        while at(Some("left")).1 {}
        let from = at(Some("left")).0;
        let (mut last, mut biggest) = (from, 0.0f32);
        for n in 0..300 {
            let (p, moving) = at(Some("forward"));
            biggest = biggest.max(p.distance(last));
            last = p;
            if !moving {
                break;
            }
            assert!(n < 299, "it never settles");
        }
        assert!(from.distance(last) > 20.0, "the buttons are far enough apart to see a jump, {}", from.distance(last));
        assert!(biggest < from.distance(last) * 0.3, "moved {biggest} px in one frame of {}", from.distance(last));
    }

    #[test]
    fn mouse_button_zoom_eases_in_and_returns_to_the_whole_model() {
        let model = Model::parse(MOUSE).expect("built-in mouse");
        let room = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(350.0, 174.0));
        let (mut view, mut glide) = (None, Glide::default());
        let mut frame = |focus| {
            let moving = ModelView::get(&mut view, &model, room, room, 0.0, -2.2, 0.62, focus, focus, &mut glide, 1.0 / 60.0);
            (glide.at[0].exp(), moving)
        };
        let whole = frame(None).0;
        let mut zooms = vec![whole];
        for index in 0..300 {
            let (zoom, moving) = frame(Some("back"));
            zooms.push(zoom);
            if !moving { break; }
            assert!(index < 299, "mouse zoom never settles");
        }
        let close = *zooms.last().unwrap();
        assert!(close > whole * 1.15, "selected button must be visibly larger");
        assert!(zooms.len() > 12);
        assert!(zooms.windows(2).all(|pair| pair[1] >= pair[0] - 0.01));
        let first_out = frame(None).0;
        assert!(first_out > whole * 1.1 && first_out < close);
        let mut last = first_out;
        for index in 0..300 {
            let (zoom, moving) = frame(None);
            assert!(zoom <= last + 0.01);
            last = zoom;
            if !moving { break; }
            assert!(index < 299, "mouse zoom-out never settles");
        }
        assert!((last - whole).abs() < 0.01);
    }

    #[test]
    fn focused_mouse_stays_clear_of_callouts_at_every_orbit_extreme() {
        let model = Model::parse(MOUSE).expect("built-in mouse");
        for width in [136.0, 280.0, 350.0, 640.0] {
            let room = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(width, 174.0));
            for yaw in [-0.9, 0.0, 0.9] {
                let points: Vec<P3> = model.points().map(|point| point.yaw(yaw)).collect();
                for pitch in [0.32, 0.62, 0.92] {
                    for part in ["left", "middle", "right", "back", "forward"] {
                        let (mut view, mut glide) = (None, Glide::default());
                        ModelView::get(&mut view, &model, room, room, yaw, -2.2, pitch, Some(part), Some(part), &mut glide, 1.0 / 60.0);
                        let camera = view.as_ref().unwrap().target;
                        assert!(points.iter().all(|point| room.shrink(7.9).contains(camera.pt(*point))), "{width} {yaw} {pitch} {part}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_drag_turns_within_its_limits_and_settles() {
        let mut o = Orbit::default();
        o.pull(egui::vec2(-5000.0, 5000.0), 0.9, 0.3);
        for _ in 0..600 {
            o.step(1.0 / 60.0);
        }
        // Eased short of the limits, not stopped hard against them.
        let (yaw, pitch) = (soft_limit(0.9 * PULL_SLACK, 0.9), soft_limit(0.3 * PULL_SLACK, 0.3));
        assert!(yaw < 0.9 && yaw > 0.8 && pitch < 0.3 && pitch > 0.27);
        assert!((o.yaw - yaw).abs() < 1e-3 && (o.pitch - pitch).abs() < 1e-3, "{} {}", o.yaw, o.pitch);
        o.reset();
        for _ in 0..600 {
            o.step(1.0 / 60.0);
        }
        assert!(o.yaw.abs() < 1e-3 && o.pitch.abs() < 1e-3);
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
        Keyboard::get(&mut board, rect, &caps, View::default(), None, &mut Glide::default(), 0.0);
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
        Keyboard::get(&mut board, rect, &caps, View::default(), None, &mut Glide::default(), 0.0);
        assert_eq!(Arc::as_ptr(&board.as_ref().expect("kept").mesh), mesh);
    }

    #[test]
    fn the_complete_board_fits_compact_and_wide_stages() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: cap.code.to_string(), to: None, modifier: false,
        }).collect();
        for (width, height) in [(640.0, 110.0), (880.0, 180.0), (1280.0, 180.0)] {
            let rect = Rect::from_min_size(pos2(8.0, 50.0), egui::vec2(width, height));
            let mut board = None;
            Keyboard::get(&mut board, rect, &caps, View::default(), None, &mut Glide::default(), 0.0);
            let board = board.expect("whole board");
            for (index, cap) in caps.iter().enumerate() {
                assert!(rect.shrink(2.0).contains_rect(board.bounds(index)), "key {} outside {rect:?}", cap.label);
            }
        }
    }

    #[test]
    fn taller_narrow_3d_stage_preserves_key_hits_and_increases_size() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        for width in [560.0, 660.0] {
            let build = |height| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(width, height));
                let mut board = None;
                Keyboard::get(&mut board, rect, &caps, View { yaw: 0.0, pitch: 0.4 }, None, &mut Glide::default(), 0.0);
                (rect, board.expect("3D board"))
            };
            let (_, shorter) = build(150.0);
            let (rect, taller) = build(220.0);
            assert!(!taller.flat);
            assert!(taller.bounds(0).width() > shorter.bounds(0).width());
            for index in 0..caps.len() {
                let bounds = taller.bounds(index);
                assert!(rect.shrink(2.0).contains_rect(bounds), "key {index} outside viewport");
                assert_eq!(taller.hit(bounds.center()), Some(index), "key {index} hit target");
            }
            for legend in &taller.legends {
                assert!(taller.tops.iter().any(|top| inside(legend.at, top)), "legend {} is outside its key", legend.text);
                assert!(legend.max_w > 0.0);
            }
        }
    }

    #[test]
    fn whole_board_keys_fit_at_orbit_extremes() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        for width in [560.0, 660.0, 900.0] {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(width, 220.0));
            for yaw in [-0.7, 0.0, 0.7] {
                for pitch in [0.1, 0.4, 0.7] {
                    let mut board = None;
                    Keyboard::get(&mut board, rect, &caps, View { yaw, pitch }, None, &mut Glide::default(), 0.0);
                    let board = board.expect("3D board");
                    for (index, cap) in caps.iter().enumerate() {
                        assert!(rect.shrink(2.0).contains_rect(board.bounds(index)), "key {} outside {rect:?} at {yaw}, {pitch}", cap.label);
                    }
                }
            }
        }
    }

    #[test]
    fn keyboard_case_stays_inside_stage_during_orbit_glides() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: crate::tune::cap_label(cap.code), to: None, modifier: false,
        }).collect();
        for width in [544.0, 660.0, 754.0, 900.0] {
            let rect = Rect::from_min_size(pos2(8.0, 8.0), egui::vec2(width, 248.0));
            let mut board = None;
            let mut glide = Glide::default();
            for view in [View { yaw: 0.0, pitch: 0.4 }, View { yaw: -0.7, pitch: 0.7 }, View { yaw: 0.7, pitch: 0.1 }, View { yaw: 0.0, pitch: 0.4 }] {
                for frame in 0..120 {
                    Keyboard::get(&mut board, rect, &caps, view, None, &mut glide, 1.0 / 60.0);
                    let bounds = board.as_ref().unwrap().case_bounds;
                    assert!(rect.shrink(2.9).contains_rect(bounds), "case clipped at width {width}, view {view:?}, frame {frame}: {bounds:?}");
                }
            }
        }
    }

    #[test]
    fn working_board_fits_and_keeps_one_legend_per_key() {
        let caps: Vec<Cap> = crate::tune::keyboard(true, false).into_iter().map(|cap| Cap {
            x: cap.x, y: cap.y, w: cap.w, h: cap.h, label: cap.code.to_string(), to: (cap.code == 58).then(|| "Esc/Ctrl".into()), modifier: false,
        }).collect();
        for (width, height) in [(860.0, 250.0), (1064.0, 340.0)] {
            let rect = Rect::from_min_size(pos2(8.0, 50.0), egui::vec2(width, height));
            let mut board = None;
            Keyboard::working(&mut board, rect, &caps);
            let board = board.unwrap();
            for index in 0..caps.len() {
                let bounds = board.bounds(index);
                assert!(rect.shrink(2.0).contains_rect(bounds));
                assert_eq!(board.hit(bounds.center()), Some(index));
                assert!(bounds.width() >= 24.0 && bounds.height() >= 24.0, "undersized key: {bounds:?}");
            }
            let remapped: Vec<_> = board.legends.iter().filter(|legend| legend.to).collect();
            assert_eq!(remapped.len(), 1);
            assert_eq!(remapped[0].text, "58");
            assert_eq!(board.legends.len(), caps.len());
            assert!(board.legends.iter().all(|legend| legend.size >= 11.0));
        }
    }

    #[test]
    fn slash_legends_are_not_tap_hold_separators() {
        let caps = vec![
            Cap { x: 0.0, y: 0.0, w: 1.0, h: 1.0, label: crate::tune::cap_label(53), to: None, modifier: false },
            Cap { x: 1.0, y: 0.0, w: 1.0, h: 1.0, label: crate::tune::cap_label(98), to: Some("/".into()), modifier: false },
        ];
        let mut board = None;
        Keyboard::working(&mut board, Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 60.0)), &caps);
        let legends = &board.unwrap().legends;
        assert_eq!(legends.len(), 2);
        assert!(legends.iter().all(|legend| legend.text == "/"));
    }

    #[test]
    fn small_key_targets_extend_beyond_the_painted_polygon() {
        let caps = vec![Cap { x: 0.0, y: 0.0, w: 1.0, h: 1.0, label: "A".into(), to: None, modifier: false }];
        let mut board = None;
        Keyboard::working(&mut board, Rect::from_min_size(Pos2::ZERO, egui::vec2(36.0, 36.0)), &caps);
        let board = board.unwrap();
        let target = board.target_bounds(0);
        assert!(target.width() >= 24.0 && target.height() >= 24.0);
        let edge = target.left_center() + egui::vec2(0.5, 0.0);
        assert!(!inside(edge, &board.tops[0]));
        assert_eq!(board.hit(edge), Some(0));
    }
}
