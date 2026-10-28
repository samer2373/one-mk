//! The desk: a small 3D renderer drawn with egui shapes. Devices are lit
//! boxes sorted back to front; screens, the pointer and the crossing ribbon
//! are drawn in each screen's own plane so they stay in perspective.
//! Nothing here knows about the daemon: the app sets targets, the scene
//! eases toward them.

use egui::{Color32, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2};
use std::ops::{Add, Mul, Sub};

pub const SIGNAL: Color32 = Color32::from_rgb(0xFF, 0xB5, 0x47);
const INK2: [u8; 3] = [0x9D, 0xA6, 0xB1];
/// Distance between neighbouring devices on the desk, in centimetres.
pub const SPACING: f32 = 62.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, b: V3) -> V3 {
        v(self.x + b.x, self.y + b.y, self.z + b.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, b: V3) -> V3 {
        v(self.x - b.x, self.y - b.y, self.z - b.z)
    }
}
impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 {
        v(self.x * s, self.y * s, self.z * s)
    }
}
fn dot(a: V3, b: V3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: V3, b: V3) -> V3 {
    v(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}
fn norm(a: V3) -> V3 {
    a * (1.0 / dot(a, a).sqrt())
}
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
pub fn ease(t: f32) -> f32 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4)
}
/// Frame-rate independent approach: `rate` is the share covered per 60 Hz frame.
pub fn approach(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let k = 1.0 - (1.0 - rate).powf(dt * 60.0);
    let next = lerp(cur, target, k);
    if (next - target).abs() < 1e-3 { target } else { next }
}
fn rgba(c: [u8; 3], k: f32, a: f32) -> Color32 {
    let f = |x: u8| (x as f32 * k).clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(f(c[0]), f(c[1]), f(c[2]), (a.clamp(0.0, 1.0) * 255.0) as u8)
}
fn hex(n: u32) -> [u8; 3] {
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}
fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

// ---------- camera ----------

pub struct Camera {
    c: V3,
    r: V3,
    u: V3,
    f: V3,
    foc: f32,
    centre: Pos2,
}

impl Camera {
    /// `raise` (0..1) moves the scene up, to make room for a sheet below it.
    pub fn new(yaw: f32, pitch: f32, dist: f32, target: V3, rect: Rect, raise: f32) -> Self {
        let cp = pitch.cos();
        let c = target + v(yaw.sin() * cp * dist, pitch.sin() * dist, yaw.cos() * cp * dist);
        let f = norm(target - c);
        let r = norm(cross(f, v(0.0, 1.0, 0.0)));
        let u = cross(r, f);
        let foc = (rect.width() * 0.98).min(rect.height() * 1.68);
        Self { c, r, u, f, foc, centre: pos2(rect.center().x, rect.top() + rect.height() * (0.61 - 0.16 * raise)) }
    }

    /// Screen position and depth.
    pub fn p(&self, p: V3) -> (Pos2, f32) {
        let d = p - self.c;
        let z = dot(d, self.f).max(1.0);
        (pos2(self.centre.x + self.foc * dot(d, self.r) / z, self.centre.y - self.foc * dot(d, self.u) / z), z)
    }

    pub fn pt(&self, p: V3) -> Pos2 {
        self.p(p).0
    }
}

// ---------- devices ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Monitor,
    Laptop,
}

#[derive(Clone, Copy)]
struct Frame3 {
    o: V3,
    ex: V3,
    ey: V3,
    ez: V3,
}

impl Frame3 {
    fn axes(o: V3) -> Self {
        Self { o, ex: v(1.0, 0.0, 0.0), ey: v(0.0, 1.0, 0.0), ez: v(0.0, 0.0, 1.0) }
    }
    fn at(&self, a: f32, b: f32, c: f32) -> V3 {
        self.o + self.ex * a + self.ey * b + self.ez * c
    }
}

#[derive(Clone, Copy)]
struct Box3 {
    fr: Frame3,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    col: u32,
    /// The front face is a screen, in this colour when off.
    screen: Option<u32>,
}

/// A screen's surface: `at(0, 0)` is its top left, `at(1, 1)` its bottom right.
#[derive(Clone, Copy)]
pub struct Plane {
    fr: Frame3,
    u: (f32, f32),
    w: (f32, f32),
}

impl Plane {
    pub fn at(&self, u: f32, w: f32) -> V3 {
        self.fr.at(lerp(self.u.0, self.u.1, u), lerp(self.w.0, self.w.1, w), 0.02)
    }
}

pub struct Geo {
    boxes: Vec<Box3>,
    pub screen: Plane,
    deck: Option<Frame3>,
    /// Centre x, z and half sizes of the footprint on the desk.
    foot: [f32; 4],
}

/// One device on the desk, easing toward its targets.
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub me: bool,
    pub online: bool,
    pub pos: V3,
    pub target: V3,
    pub scale: f32,
    /// Solid fill (0 while being paired, as a wireframe).
    pub alpha: f32,
    pub alpha_to: f32,
    /// How much of the wireframe outline is drawn.
    pub wire: f32,
    pub wire_to: f32,
    pub wall: [u32; 2],
    /// The second line on its screen: "This computer", "Linux", "Offline"…
    pub subtitle: String,
    /// Removed once it has faded out.
    pub leaving: bool,
}

const LIGHT: V3 = v(-0.39, 0.74, 0.48);

impl Device {
    pub fn new(id: &str, name: &str, kind: Kind, me: bool, wall: [u32; 2]) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            me,
            online: true,
            pos: V3::default(),
            target: V3::default(),
            scale: 1.0,
            alpha: 1.0,
            alpha_to: 1.0,
            wire: 1.0,
            wire_to: 1.0,
            wall,
            subtitle: String::new(),
            leaving: false,
        }
    }

    pub fn visible(&self) -> f32 {
        self.alpha.max(self.wire * 0.9)
    }

    pub fn geometry(&self) -> Geo {
        let V3 { x, y, z } = self.pos;
        let s = self.scale;
        match self.kind {
            Kind::Monitor => {
                let (hw, hh) = (29.0 * s, 16.5 * s);
                let cy = 13.0 * s + hh;
                let scr = Frame3::axes(v(x, y + cy, z - 14.0));
                Geo {
                    boxes: vec![
                        Box3 { fr: Frame3::axes(v(x, y, z - 12.0)), a: (-11.0 * s, 11.0 * s), b: (0.0, 0.8), c: (-8.0 * s, 8.0 * s), col: 0x3A404A, screen: None },
                        Box3 { fr: Frame3::axes(v(x, y, z - 15.6)), a: (-3.0 * s, 3.0 * s), b: (0.0, cy), c: (-1.1, 1.1), col: 0x3A404A, screen: None },
                        Box3 { fr: scr, a: (-hw, hw), b: (-hh, hh), c: (-2.2, 0.0), col: 0x30353E, screen: Some(0x0B0D10) },
                    ],
                    screen: Plane { fr: scr, u: (-hw + 0.9, hw - 0.9), w: (hh - 0.9, -hh + 0.9) },
                    deck: None,
                    foot: [x, z - 12.0, 12.0 * s, 8.0 * s],
                }
            }
            Kind::Laptop => {
                let (ca, sa) = (0.26f32.cos(), 0.26f32.sin());
                let lid = Frame3 { o: v(x, y + 1.4 * s, z - 6.5 * s), ex: v(1.0, 0.0, 0.0), ey: v(0.0, ca, -sa), ez: v(0.0, sa, ca) };
                Geo {
                    boxes: vec![
                        Box3 { fr: Frame3::axes(v(x, y, z + 4.0 * s)), a: (-15.5 * s, 15.5 * s), b: (0.0, 1.4 * s), c: (-10.5 * s, 10.5 * s), col: 0xAEB3BB, screen: None },
                        Box3 { fr: lid, a: (-15.5 * s, 15.5 * s), b: (0.0, 20.4 * s), c: (-0.6, 0.0), col: 0xAEB3BB, screen: Some(0x0B0D10) },
                    ],
                    screen: Plane { fr: lid, u: (-14.4 * s, 14.4 * s), w: (19.5 * s, 1.3 * s) },
                    deck: Some(Frame3::axes(v(x, y + 1.41 * s, z + 4.0 * s))),
                    foot: [x, z + 2.0 * s, 17.0 * s, 12.0 * s],
                }
            }
        }
    }
}

/// A face of a unit box: outward normal, corners as (a, b, c) high/low
/// picks, and whether it is the front.
type FaceDef = (V3, [(bool, bool, bool); 4], bool);
const FACES: [FaceDef; 6] = [
    (v(0.0, 0.0, 1.0), [(false, false, true), (true, false, true), (true, true, true), (false, true, true)], true),
    (v(0.0, 0.0, -1.0), [(true, false, false), (false, false, false), (false, true, false), (true, true, false)], false),
    (v(1.0, 0.0, 0.0), [(true, false, true), (true, false, false), (true, true, false), (true, true, true)], false),
    (v(-1.0, 0.0, 0.0), [(false, false, false), (false, false, true), (false, true, true), (false, true, false)], false),
    (v(0.0, 1.0, 0.0), [(false, true, true), (true, true, true), (true, true, false), (false, true, false)], false),
    (v(0.0, -1.0, 0.0), [(false, false, false), (true, false, false), (true, false, true), (false, false, true)], false),
];

pub struct Face {
    pub pts: [Pos2; 4],
    depth: f32,
    fill: Color32,
    pub owner: usize,
    /// The front of a screen box: draw the screen after it.
    screen: bool,
    /// The top of a laptop base: draw the keyboard after it.
    deck: bool,
}

fn faces_of(cam: &Camera, b: &Box3, owner: usize, first_box: bool, laptop: bool, out: &mut Vec<Face>) {
    for (ln, idx, front) in FACES {
        let n = norm(b.fr.ex * ln.x + b.fr.ey * ln.y + b.fr.ez * ln.z);
        let corners = idx.map(|(i, j, k)| {
            b.fr.at(if i { b.a.1 } else { b.a.0 }, if j { b.b.1 } else { b.b.0 }, if k { b.c.1 } else { b.c.0 })
        });
        let ctr = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
        if dot(n, cam.c - ctr) <= 0.0 {
            continue;
        }
        let proj = corners.map(|p| cam.p(p));
        let lit = 0.32 + 0.78 * dot(n, LIGHT).max(0.0);
        let col = if front { b.screen.unwrap_or(b.col) } else { b.col };
        out.push(Face {
            pts: proj.map(|p| p.0),
            depth: proj.iter().map(|p| p.1).sum::<f32>() / 4.0,
            fill: rgba(hex(col), lit, 1.0),
            owner,
            screen: front && b.screen.is_some(),
            deck: laptop && first_box && ln.y > 0.5,
        });
    }
}

fn centroid(q: &[Pos2]) -> Pos2 {
    let n = q.len() as f32;
    Pos2::new(q.iter().map(|p| p.x).sum::<f32>() / n, q.iter().map(|p| p.y).sum::<f32>() / n)
}

fn inside(p: Pos2, q: &[Pos2]) -> bool {
    let mut c = false;
    let mut j = q.len() - 1;
    for i in 0..q.len() {
        if (q[i].y > p.y) != (q[j].y > p.y) && p.x < (q[j].x - q[i].x) * (p.y - q[i].y) / (q[j].y - q[i].y) + q[i].x {
            c = !c;
        }
        j = i;
    }
    c
}

// ---------- drawing helpers ----------

fn quad(painter: &Painter, pts: [Pos2; 4], fill: Color32) {
    painter.add(Shape::convex_polygon(pts.to_vec(), fill, Stroke::NONE));
}

/// A filled quad with one colour per corner.
fn quad_colors(painter: &Painter, pts: [Pos2; 4], cols: [Color32; 4]) {
    let mut m = Mesh::default();
    for (p, c) in pts.iter().zip(cols) {
        m.colored_vertex(*p, c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(m));
}

/// A soft disc on the desk (y = `y`): `col` at the centre fading to nothing.
fn pool(painter: &Painter, cam: &Camera, c: V3, rx: f32, rz: f32, col: Color32) {
    let mut m = Mesh::default();
    m.colored_vertex(cam.pt(c), col);
    const N: u32 = 28;
    for i in 0..N {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        m.colored_vertex(cam.pt(c + v(a.cos() * rx, 0.0, a.sin() * rz)), Color32::TRANSPARENT);
    }
    for i in 0..N {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % N);
    }
    painter.add(Shape::mesh(m));
}

/// A rectangle on the desk with a soft edge: `col` inside, fading over `feather`.
fn soft_rect(painter: &Painter, cam: &Camera, [x, z, rx, rz]: [f32; 4], y: f32, feather: f32, col: Color32) {
    let ring = |grow: f32| {
        [v(x - rx - grow, y, z - rz - grow), v(x + rx + grow, y, z - rz - grow), v(x + rx + grow, y, z + rz + grow), v(x - rx - grow, y, z + rz + grow)]
    };
    let mut m = Mesh::default();
    for p in ring(-feather * 0.3) {
        m.colored_vertex(cam.pt(p), col);
    }
    for p in ring(feather) {
        m.colored_vertex(cam.pt(p), Color32::TRANSPARENT);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    for i in 0..4 {
        let j = (i + 1) % 4;
        m.add_triangle(i, j, 4 + j);
        m.add_triangle(i, 4 + j, 4 + i);
    }
    painter.add(Shape::mesh(m));
}

/// Draws the first `frac` of a closed outline, for the draw-in effect.
fn partial_outline(painter: &Painter, pts: &[Pos2], frac: f32, stroke: Stroke) {
    let n = pts.len();
    let lens: Vec<f32> = (0..n).map(|i| pts[i].distance(pts[(i + 1) % n])).collect();
    let mut left = lens.iter().sum::<f32>() * frac.clamp(0.0, 1.0);
    for i in 0..n {
        if left <= 0.0 {
            break;
        }
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let t = (left / lens[i]).min(1.0);
        painter.line_segment([a, a + (b - a) * t], stroke);
        left -= lens[i];
    }
}

fn bez(p0: V3, p1: V3, p2: V3, p3: V3, t: f32) -> V3 {
    let s = 1.0 - t;
    p0 * (s * s * s) + p1 * (3.0 * s * s * t) + p2 * (3.0 * s * t * t) + p3 * (t * t * t)
}

// ---------- the scene ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cargo {
    /// The mouse and keyboard moved to the other screen.
    Pointer,
    /// Files being sent.
    Parcel,
    /// A latency measurement, labelled with its result.
    Probe,
}

/// Something travelling from one screen to another.
#[derive(Clone)]
pub struct Flight {
    pub from: String,
    pub to: String,
    /// 0..1 through the flight.
    pub t: f32,
    pub cargo: Cargo,
}

pub struct Scene {
    pub devices: Vec<Device>,
    pub yaw: f32,
    pub pitch: f32,
    pub yaw_to: f32,
    pub pitch_to: f32,
    pub dist: f32,
    pub focus_x: f32,
    /// Where the pointer is: a device id, and its spot on that screen.
    pub cursor_on: String,
    pub cursor_uw: (f32, f32),
    pub cursor_uw_to: (f32, f32),
    pub flight: Option<Flight>,
    /// Lights a screen edge for a moment after a crossing.
    pub flash: f32,
    /// Shown at the top of the arc: the measured one-way time.
    pub latency_label: Option<String>,
    /// A pairing code shown on screens: (device ids, code, when it started).
    pub code: Option<(Vec<String>, String, f64)>,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    /// An outlined place beside this computer that invites pairing.
    pub empty_slot: bool,
    pub slot_hover: bool,
    /// Devices outlined as places to drop files.
    pub drop_targets: bool,
    /// Opening animation, 0..1.
    pub intro: f32,
    /// Room for a sheet under the desk, 0..1.
    pub raise: f32,
    pub raise_to: f32,
    faces: Vec<Face>,
    last_time: f64,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            devices: vec![],
            yaw: -0.55,
            pitch: 0.62,
            yaw_to: -0.2,
            pitch_to: 0.43,
            dist: 152.0,
            focus_x: 0.0,
            cursor_on: String::new(),
            cursor_uw: (0.55, 0.5),
            cursor_uw_to: (0.55, 0.5),
            flight: None,
            flash: 0.0,
            latency_label: None,
            code: None,
            hover: None,
            selected: None,
            empty_slot: false,
            slot_hover: false,
            drop_targets: false,
            intro: 0.0,
            raise: 0.0,
            raise_to: 0.0,
            faces: vec![],
            last_time: -1.0,
        }
    }
}

impl Cargo {
    /// How long the flight takes: a measurement lingers so its label reads.
    fn secs(self) -> f32 {
        if self == Cargo::Probe { 1.1 } else { 0.7 }
    }
}

impl Scene {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.devices.iter().position(|d| d.id == id)
    }

    /// Steps every animation. Returns true while anything is still moving.
    pub fn step(&mut self, time: f64, dragging: Option<usize>) -> bool {
        let dt = if self.last_time < 0.0 { 1.0 / 60.0 } else { (time - self.last_time).clamp(0.0, 0.1) as f32 };
        self.last_time = time;
        let mut moving = false;
        let mut go = |cur: &mut f32, to: f32, rate: f32| {
            let next = approach(*cur, to, rate, dt);
            moving |= next != to;
            *cur = next;
        };
        go(&mut self.intro, 1.0, 0.035);
        go(&mut self.raise, self.raise_to, 0.1);
        go(&mut self.yaw, self.yaw_to, 0.12);
        go(&mut self.pitch, self.pitch_to, 0.12);
        let (lo, hi) = self
            .devices
            .iter()
            .filter(|d| !d.leaving)
            .fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d.target.x), hi.max(d.target.x)));
        // The empty slot beside this computer counts as part of the scene.
        let hi = if self.empty_slot { hi.max(self.slot_device().pos.x) } else { hi };
        let (fx, span) = if lo <= hi { ((lo + hi) / 2.0, hi - lo) } else { (0.0, 0.0) };
        go(&mut self.focus_x, fx, 0.06);
        go(&mut self.dist, 132.0 + span * 0.5, 0.06);
        go(&mut self.cursor_uw.0, self.cursor_uw_to.0, 0.08);
        go(&mut self.cursor_uw.1, self.cursor_uw_to.1, 0.08);
        go(&mut self.flash, 0.0, 0.05);
        for (i, d) in self.devices.iter_mut().enumerate() {
            if dragging != Some(i) {
                go(&mut d.pos.x, d.target.x, 0.14);
            }
            go(&mut d.pos.y, d.target.y, 0.14);
            go(&mut d.pos.z, d.target.z, 0.14);
            go(&mut d.alpha, d.alpha_to, 0.06);
            go(&mut d.wire, d.wire_to, 0.05);
        }
        self.devices.retain(|d| !(d.leaving && d.alpha < 0.01 && d.wire < 0.01));
        if let Some(f) = &mut self.flight {
            f.t += dt / f.cargo.secs();
            moving = true;
            if f.t >= 1.0 {
                if f.cargo == Cargo::Pointer {
                    self.cursor_on = f.to.clone();
                }
                self.flash = 1.0;
                self.flight = None;
            }
        }
        if let Some((_, _, t0)) = &self.code {
            moving |= time - t0 < 2.0;
        }
        moving
    }

    /// Where on `a`'s screen the edge toward `b` is, as (u, w).
    fn edge(&self, a: &Device, b: &Device, w: f32) -> (f32, f32) {
        let (dx, dy) = (b.target.x - a.target.x, (b.target.y - b.target.z) - (a.target.y - a.target.z));
        if dx.abs() >= dy.abs() { (if dx < 0.0 { 0.0 } else { 1.0 }, w) } else { (0.5, if dy > 0.0 { 0.0 } else { 1.0 }) }
    }

    /// Sends the pointer (or something else) flying from one screen to another.
    pub fn fly(&mut self, from: &str, to: &str, cargo: Cargo) {
        let pointer = cargo == Cargo::Pointer;
        if from == to || self.index(from).is_none() || self.index(to).is_none() {
            if pointer {
                self.cursor_on = to.into();
            }
            return;
        }
        if pointer {
            let (a, b) = (&self.devices[self.index(to).unwrap()], &self.devices[self.index(from).unwrap()]);
            let (u, w) = self.edge(a, b, self.cursor_uw.1);
            // Land just inside the edge it came through, then drift in a little.
            self.cursor_uw = (u, w);
            self.cursor_uw_to = (lerp(u, 0.5, 0.35), lerp(w, 0.5, 0.2));
        }
        self.flight = Some(Flight { from: from.into(), to: to.into(), t: 0.0, cargo });
    }

    pub fn hit(&self, p: Pos2) -> Option<usize> {
        self.faces.iter().rev().find(|f| self.devices[f.owner].alpha > 0.5 && inside(p, &f.pts)).map(|f| f.owner)
    }

    /// Screen-space direction of +x on the desk at a device, for dragging.
    pub fn x_axis(&self, rect: Rect, i: usize) -> egui::Vec2 {
        let cam = self.camera(rect);
        let p = self.devices[i].pos;
        cam.pt(p + v(1.0, 0.0, 0.0)) - cam.pt(p)
    }

    fn camera(&self, rect: Rect) -> Camera {
        let k = ease_out(self.intro);
        Camera::new(self.yaw, self.pitch, self.dist * lerp(1.35, 1.0, k), v(self.focus_x, 15.0, -2.0), rect, self.raise)
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect, time: f64) {
        let cam = self.camera(rect);
        let geo: Vec<Geo> = self.devices.iter().map(Device::geometry).collect();
        self.room(painter, &cam, &geo, rect);

        let mut faces = Vec::new();
        for (i, (d, g)) in self.devices.iter().zip(&geo).enumerate() {
            if d.visible() < 0.01 {
                continue;
            }
            for (k, b) in g.boxes.iter().enumerate() {
                faces_of(&cam, b, i, k == 0, d.kind == Kind::Laptop, &mut faces);
            }
        }
        faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        for f in &faces {
            let d = &self.devices[f.owner];
            let g = &geo[f.owner];
            if d.alpha < 0.99 {
                quad(painter, f.pts, fade(f.fill, d.alpha));
                let wire = d.wire * (1.0 - d.alpha);
                partial_outline(painter, &f.pts, d.wire, Stroke::new(1.0, fade(SIGNAL, 0.85 * wire)));
            } else {
                quad(painter, f.pts, f.fill);
            }
            if f.screen {
                self.screen(painter, &cam, f.owner, &geo, time);
                // Amber means "goes there": drop targets. Selection stays quiet.
                let outline = if self.drop_targets && d.online && !d.me {
                    Some(Stroke::new(1.5, fade(SIGNAL, 0.9 * d.alpha)))
                } else if self.selected == Some(f.owner) {
                    Some(Stroke::new(1.5, Color32::from_white_alpha((150.0 * d.alpha) as u8)))
                } else if self.hover == Some(f.owner) {
                    Some(Stroke::new(1.0, Color32::from_white_alpha(60)))
                } else {
                    None
                };
                if let Some(st) = outline {
                    painter.add(Shape::closed_line(f.pts.iter().map(|p| *p + (*p - centroid(&f.pts)).normalized() * 2.5).collect(), st));
                }
            }
            if f.deck
                && let Some(k) = g.deck
            {
                let s = d.scale;
                let well = [k.at(-13.0 * s, 0.0, -9.5 * s), k.at(13.0 * s, 0.0, -9.5 * s), k.at(13.0 * s, 0.0, 1.0 * s), k.at(-13.0 * s, 0.0, 1.0 * s)];
                quad(painter, well.map(|p| cam.pt(p)), fade(Color32::from_rgb(20, 22, 26), 0.85 * d.alpha));
                let pad = [k.at(-5.5 * s, 0.0, 3.0 * s), k.at(5.5 * s, 0.0, 3.0 * s), k.at(5.5 * s, 0.0, 9.0 * s), k.at(-5.5 * s, 0.0, 9.0 * s)];
                quad(painter, pad.map(|p| cam.pt(p)), fade(Color32::WHITE, 0.12 * d.alpha));
            }
        }
        self.faces = faces;
        self.ribbon(painter, &cam, &geo);
        if self.empty_slot {
            self.slot(painter, &cam);
        }
    }

    /// Where the empty slot's screen is, for clicks.
    pub fn slot_rect(&self, rect: Rect) -> Option<Rect> {
        if !self.empty_slot {
            return None;
        }
        let cam = self.camera(rect);
        let g = self.slot_device().geometry();
        Some(Rect::from_two_pos(cam.pt(g.screen.at(0.0, 0.0)), cam.pt(g.screen.at(1.0, 1.0))))
    }

    fn slot_device(&self) -> Device {
        let mut d = Device::new("", "", Kind::Monitor, false, [0, 0]);
        d.pos = self.devices.first().map_or(V3::default(), |m| m.pos) + v(29.0 + 26.0 + 29.0 * 0.86, 0.0, 0.0);
        d.scale = 0.86;
        d
    }

    /// A dashed outline beside this computer: the place a second one goes.
    fn slot(&self, painter: &Painter, cam: &Camera) {
        let g = self.slot_device().geometry();
        let k = if self.slot_hover { 1.0 } else { 0.55 };
        let pts: Vec<Pos2> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)].iter().map(|(u, w)| cam.pt(g.screen.at(*u, *w))).collect();
        let st = Stroke::new(1.2, if self.slot_hover { fade(SIGNAL, 0.9) } else { Color32::from_white_alpha(60) });
        painter.extend(Shape::dashed_line(&pts, st, 6.0, 5.0));
        let h = pts[0].distance(pts[3]);
        let c = cam.pt(g.screen.at(0.5, 0.5));
        let col = if self.slot_hover { SIGNAL } else { rgba([0xC4, 0xCA, 0xD2], 1.0, k) };
        painter.text(c - egui::vec2(0.0, 2.0), egui::Align2::CENTER_BOTTOM, "Add a computer", FontId::new((h * 0.1).clamp(10.0, 16.0), FontFamily::Name("semibold".into())), col);
        painter.text(c + egui::vec2(0.0, 4.0), egui::Align2::CENTER_TOP, "Pair it to share this mouse and keyboard", FontId::new((h * 0.065).clamp(8.0, 12.0), FontFamily::Proportional), rgba(INK2, 1.0, k));
    }

    fn room(&self, painter: &Painter, cam: &Camera, geo: &[Geo], rect: Rect) {
        let (sky_top, sky_low) = (Color32::from_rgb(0x0B, 0x0E, 0x12), Color32::from_rgb(0x10, 0x13, 0x18));
        quad_colors(painter, [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()], [sky_top, sky_top, sky_low, sky_low]);
        // The desk is lit around the computers and falls off into the room:
        // no edges, nothing to suggest a box the computers sit in.
        let (cx, cz) = (self.focus_x, -4.0);
        let (rx, rz) = (150.0 + self.dist * 0.4, 70.0);
        let light = |x: f32, z: f32| {
            let d = (((x - cx) / rx).powi(2) + ((z - cz) / rz).powi(2)).sqrt();
            (1.0 - d).clamp(0.0, 1.0).powf(1.4)
        };
        let lit = Color32::from_rgb(0x24, 0x29, 0x31);
        let shade = |k: f32| {
            let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
            Color32::from_rgb(f(sky_low.r(), lit.r()), f(sky_low.g(), lit.g()), f(sky_low.b(), lit.b()))
        };
        const NX: usize = 18;
        const NZ: usize = 10;
        let xs: Vec<f32> = (0..=NX).map(|i| cx - rx + 2.0 * rx * i as f32 / NX as f32).collect();
        let zs: Vec<f32> = (0..=NZ).map(|j| cz - rz + 2.0 * rz * j as f32 / NZ as f32).collect();
        let mut m = Mesh::default();
        for &z in &zs {
            for &x in &xs {
                m.colored_vertex(cam.pt(v(x, 0.0, z)), shade(light(x, z)));
            }
        }
        let w = (NX + 1) as u32;
        for j in 0..NZ as u32 {
            for i in 0..NX as u32 {
                let k = j * w + i;
                m.add_triangle(k, k + 1, k + w + 1);
                m.add_triangle(k, k + w + 1, k + w);
            }
        }
        painter.add(Shape::mesh(m));
        // A 10 cm grid, the room's measuring tape, fading with the light.
        let seg = |a: V3, b: V3| {
            let k = light((a.x + b.x) / 2.0, (a.z + b.z) / 2.0);
            if k > 0.02 {
                painter.line_segment([cam.pt(a), cam.pt(b)], Stroke::new(1.0, Color32::from_white_alpha((14.0 * k) as u8)));
            }
        };
        let (x0, x1) = (((cx - rx) / 10.0).ceil() as i32, ((cx + rx) / 10.0) as i32);
        let (z0, z1) = (((cz - rz) / 10.0).ceil() as i32, ((cz + rz) / 10.0) as i32);
        for i in x0..=x1 {
            for j in z0..z1 {
                let (x, z) = (i as f32 * 10.0, j as f32 * 10.0);
                seg(v(x, 0.0, z), v(x, 0.0, z + 10.0));
            }
        }
        for j in z0..=z1 {
            for i in x0..x1 {
                let (x, z) = (i as f32 * 10.0, j as f32 * 10.0);
                seg(v(x, 0.0, z), v(x + 10.0, 0.0, z));
            }
        }
        // Light from each screen on the desk, and contact shadows.
        for (d, g) in self.devices.iter().zip(geo) {
            let a = d.alpha;
            if a < 0.01 {
                continue;
            }
            let [fx, fz, rx, rz] = g.foot;
            if d.online {
                let c = hex(d.wall[0]);
                pool(painter, cam, v(fx, 0.0, fz + 16.0), 30.0, 22.0, rgba([c[0].saturating_add(40), c[1].saturating_add(40), c[2].saturating_add(40)], 1.0, 0.16 * a));
            }
            if d.pos.y < 1.0 {
                soft_rect(painter, cam, [fx, fz, rx, rz], 0.0, 5.0, Color32::from_black_alpha((140.0 * a) as u8));
            } else {
                soft_rect(painter, cam, [fx, fz, rx * 0.8, rz * 0.8], 0.0, 9.0, Color32::from_black_alpha((60.0 * a) as u8));
            }
        }
        // The flying pointer lights the desk under it.
        if let Some((f, p)) = self.flight.as_ref().and_then(|f| self.flight_head(geo, f).map(|p| (f, p))) {
            let s = (std::f32::consts::PI * f.t).sin();
            pool(painter, cam, v(p.x, 0.0, p.z), 26.0 * s.max(0.05), 20.0 * s.max(0.05), Color32::from_rgba_unmultiplied(255, 190, 110, 56));
        }
    }

    fn flight_curve(&self, geo: &[Geo], f: &Flight) -> Option<[V3; 4]> {
        let (ia, ib) = (self.index(&f.from)?, self.index(&f.to)?);
        let (a, b) = (&self.devices[ia], &self.devices[ib]);
        let w = if f.cargo == Cargo::Pointer { self.cursor_uw.1 } else { 0.55 };
        let (ua, wa) = self.edge(a, b, w);
        let (ub, wb) = self.edge(b, a, w);
        let p0 = geo[ia].screen.at(ua, wa);
        let p3 = geo[ib].screen.at(ub, wb);
        let mid = (p0 + p3) * 0.5;
        let lift = v(0.0, 16.0 + (p3.x - p0.x).abs() * 0.45, 18.0);
        Some([p0, p0 + (mid - p0) * 0.5 + lift, p3 + (mid - p3) * 0.5 + lift, p3])
    }

    fn flight_head(&self, geo: &[Geo], f: &Flight) -> Option<V3> {
        let [p0, p1, p2, p3] = self.flight_curve(geo, f)?;
        Some(bez(p0, p1, p2, p3, ease(f.t.clamp(0.0, 1.0))))
    }

    /// The crossing: a ribbon of light along an arc between the two screens.
    fn ribbon(&self, painter: &Painter, cam: &Camera, geo: &[Geo]) {
        let Some(f) = &self.flight else { return };
        let Some([p0, p1, p2, p3]) = self.flight_curve(geo, f) else { return };
        let head = ease(f.t.clamp(0.0, 1.0));
        let tail = (head - 0.42).max(0.0);
        // At the very start the ribbon has no length (and no direction).
        if head - tail < 0.004 {
            return;
        }
        const N: usize = 40;
        let pts: Vec<Pos2> = (0..=N).map(|i| cam.pt(bez(p0, p1, p2, p3, lerp(tail, head, i as f32 / N as f32)))).collect();
        // A strip that widens and brightens toward the head.
        let mut m = Mesh::default();
        for i in 0..=N {
            let k = i as f32 / N as f32;
            let dir = if i == N { pts[i] - pts[i - 1] } else { pts[i + 1] - pts[i] };
            let len = dir.length();
            let n = if len > 1e-4 { egui::vec2(-dir.y, dir.x) / len } else { egui::Vec2::ZERO } * (0.4 + 2.8 * k);
            let col = if f.cargo == Cargo::Parcel {
                Color32::from_rgba_unmultiplied(255, (181.0 + 40.0 * k) as u8, (71.0 + 120.0 * k) as u8, ((0.15 + 0.85 * k) * 255.0) as u8)
            } else {
                Color32::from_rgba_unmultiplied(255, (200.0 + 40.0 * k) as u8, (140.0 + 100.0 * k) as u8, ((0.15 + 0.85 * k) * 255.0) as u8)
            };
            m.colored_vertex(pts[i] + n, col);
            m.colored_vertex(pts[i] - n, col);
            if i > 0 {
                let b = (i as u32 - 1) * 2;
                m.add_triangle(b, b + 1, b + 3);
                m.add_triangle(b, b + 3, b + 2);
            }
        }
        painter.add(Shape::mesh(m));
        let hp = cam.pt(bez(p0, p1, p2, p3, head));
        if f.cargo == Cargo::Parcel {
            let r = 4.5;
            painter.add(Shape::convex_polygon(
                vec![hp + egui::vec2(0.0, -r), hp + egui::vec2(r, 0.0), hp + egui::vec2(0.0, r), hp + egui::vec2(-r, 0.0)],
                SIGNAL,
                Stroke::new(1.0, Color32::from_rgb(0xFF, 0xF8, 0xEC)),
            ));
        } else {
            painter.circle_filled(hp, 4.2, Color32::from_rgb(0xFF, 0xF8, 0xEC));
        }
        if let Some(label) = self.latency_label.as_ref().filter(|_| f.cargo == Cargo::Probe) {
            let a = (std::f32::consts::PI * f.t).sin();
            let apex = cam.pt(bez(p0, p1, p2, p3, 0.5));
            painter.text(
                apex - egui::vec2(0.0, 14.0),
                egui::Align2::CENTER_BOTTOM,
                label,
                FontId::new(12.0, FontFamily::Monospace),
                rgba([0xED, 0xEF, 0xF2], 1.0, a),
            );
        }
    }

    /// A device's screen: wallpaper, windows, the lit edge, the pointer.
    fn screen(&self, painter: &Painter, cam: &Camera, i: usize, geo: &[Geo], time: f64) {
        let d = &self.devices[i];
        let pl = &geo[i].screen;
        let a = d.alpha.max(d.wire * 0.9);
        if a < 0.02 {
            return;
        }
        if !d.online {
            let at = |u: f32, w: f32| cam.pt(pl.at(u, w));
            let h = at(0.5, 0.0).distance(at(0.5, 1.0));
            painter.text(at(0.5, 0.47), egui::Align2::CENTER_BOTTOM, &d.name, FontId::new((h * 0.105).clamp(9.0, 22.0), FontFamily::Name("semibold".into())), rgba(INK2, 1.0, 0.7 * a));
            painter.text(at(0.5, 0.53), egui::Align2::CENTER_TOP, &d.subtitle, FontId::new((h * 0.07).clamp(8.0, 14.0), FontFamily::Proportional), rgba(INK2, 1.0, 0.5 * a));
            return;
        }
        let at = |u: f32, w: f32| cam.pt(pl.at(u, w));
        let rect = |u0: f32, w0: f32, u1: f32, w1: f32| [at(u0, w0), at(u1, w0), at(u1, w1), at(u0, w1)];
        let [c0, c1] = d.wall.map(|c| fade(rgba(hex(c), 1.0, 1.0), a));
        let mid = fade(Color32::from_rgb(
            ((hex(d.wall[0])[0] as u16 + hex(d.wall[1])[0] as u16) / 2) as u8,
            ((hex(d.wall[0])[1] as u16 + hex(d.wall[1])[1] as u16) / 2) as u8,
            ((hex(d.wall[0])[2] as u16 + hex(d.wall[1])[2] as u16) / 2) as u8,
        ), a);
        quad_colors(painter, rect(0.0, 0.0, 1.0, 1.0), [c0, mid, c1, mid]);

        let showing_code = self.code.as_ref().filter(|(ids, _, _)| ids.contains(&d.id));
        if let Some((_, code, t0)) = showing_code {
            quad(painter, rect(0.2, 0.26, 0.8, 0.74), fade(Color32::from_rgb(14, 17, 21), 0.86 * a));
            let shown = (((time - t0 - 0.5) / 0.11).max(0.0) as usize).min(code.len());
            let digits: String = code.chars().take(shown).chain(std::iter::repeat_n('·', code.len() - shown)).collect();
            let text = if digits.len() == 6 { format!("{} {}", &digits[..3], &digits[3..]) } else { digits };
            let h = at(0.5, 0.0).distance(at(0.5, 1.0));
            painter.text(at(0.5, 0.42), egui::Align2::CENTER_CENTER, "Pairing code", FontId::new((h * 0.07).max(6.0), FontFamily::Proportional), rgba(INK2, 1.0, a));
            painter.text(at(0.5, 0.58), egui::Align2::CENTER_CENTER, text, FontId::new((h * 0.16).max(8.0), FontFamily::Monospace), rgba([0xED, 0xEF, 0xF2], 1.0, a));
            return;
        }

        // A menu bar for scale, then the computer's own name, as the
        // system's display arrangement shows it.
        quad(painter, rect(0.0, 0.0, 1.0, 0.045), fade(Color32::BLACK, 0.22 * a));
        let h = at(0.5, 0.0).distance(at(0.5, 1.0));
        let ink = rgba([0xED, 0xEF, 0xF2], 1.0, a);
        painter.text(at(0.5, 0.47), egui::Align2::CENTER_BOTTOM, &d.name, FontId::new((h * 0.105).clamp(9.0, 22.0), FontFamily::Name("semibold".into())), ink);
        painter.text(at(0.5, 0.53), egui::Align2::CENTER_TOP, &d.subtitle, FontId::new((h * 0.07).clamp(8.0, 14.0), FontFamily::Proportional), rgba([0xC4, 0xCA, 0xD2], 1.0, 0.85 * a));

        // The edges toward online neighbours, lit while the pointer is near.
        for (j, o) in self.devices.iter().enumerate() {
            if j == i || !o.online || o.alpha < 0.5 || !(d.me || o.me) {
                continue;
            }
            let (u, w) = self.edge(d, o, 0.5);
            let near_cursor = if self.cursor_on == d.id && self.flight.is_none() {
                let (cu, cw) = self.cursor_uw;
                (1.0 - if u == 0.5 { (cw - w).abs() } else { (cu - u).abs() } * 5.0).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let involved = self.flight.as_ref().is_some_and(|f| f.cargo != Cargo::Parcel && (f.from == d.id || f.to == d.id));
            let glow = near_cursor.max(if involved { 1.0 } else { self.flash * f32::from(self.cursor_on == d.id || self.cursor_on == o.id) });
            let col = fade(SIGNAL, (0.18 + 0.82 * glow) * a);
            let strip = if u == 0.5 {
                let (w0, w1) = if w == 0.0 { (0.0, 0.02) } else { (0.98, 1.0) };
                rect(0.08, w0, 0.92, w1)
            } else {
                let (u0, u1) = if u == 0.0 { (0.0, 0.012) } else { (0.988, 1.0) };
                rect(u0, 0.08, u1, 0.92)
            };
            quad(painter, strip, col);
        }

        if self.cursor_on == d.id && self.flight.as_ref().is_none_or(|f| f.cargo != Cargo::Pointer) {
            let (cu, cw) = self.cursor_uw;
            // An arrow in screen pixels of a 1600 px wide screen.
            let s = 3.6 / 1600.0;
            let arrow = [(0.0, 0.0), (0.0, 17.0), (4.2, 13.2), (7.2, 19.6), (9.8, 18.4), (6.9, 12.2), (12.2, 11.8)];
            let aspect = if d.kind == Kind::Laptop { 1470.0 / 956.0 } else { 16.0 / 9.0 };
            let pts: Vec<Pos2> = arrow.iter().map(|(x, y)| at(cu + x * s, cw + y * s * aspect)).collect();
            // Not convex: draw it as two convex parts.
            painter.add(Shape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[5], pts[6]], Color32::WHITE, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![pts[2], pts[3], pts[4], pts[5]], Color32::WHITE, Stroke::NONE));
            painter.add(Shape::closed_line(pts, Stroke::new(0.8, Color32::from_rgb(0x11, 0x11, 0x11))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_puts_the_target_near_the_centre() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 400.0));
        let cam = Camera::new(0.0, 0.4, 150.0, v(0.0, 15.0, 0.0), rect, 0.0);
        let p = cam.pt(v(0.0, 15.0, 0.0));
        assert!((p.x - 400.0).abs() < 0.01, "{p:?}");
        assert!(cam.pt(v(10.0, 15.0, 0.0)).x > p.x, "+x is to the right");
        assert!(cam.pt(v(0.0, 25.0, 0.0)).y < p.y, "+y is up");
    }

    #[test]
    fn approach_settles() {
        let mut x = 0.0;
        for _ in 0..600 {
            x = approach(x, 1.0, 0.12, 1.0 / 60.0);
        }
        assert_eq!(x, 1.0);
    }

    #[test]
    fn devices_can_be_picked() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(900.0, 430.0));
        let mut s = Scene { intro: 1.0, ..Default::default() };
        s.devices.push(Device::new("me", "desk", Kind::Monitor, true, [0x1D2D3C, 0x0D151E]));
        let mut mac = Device::new("mac", "mac", Kind::Laptop, false, [0x3D3127, 0x1A1511]);
        mac.pos.x = -SPACING;
        mac.target.x = -SPACING;
        s.devices.push(mac);
        for _ in 0..200 {
            s.step(0.0, None);
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let painter = Painter::new(ctx.clone(), egui::LayerId::background(), rect);
            s.paint(&painter, rect, 0.0);
        });
        out.textures_delta.clear();
        let cam = s.camera(rect);
        let me = cam.pt(s.devices[0].geometry().screen.at(0.5, 0.5));
        let mac = cam.pt(s.devices[1].geometry().screen.at(0.5, 0.5));
        assert_eq!(s.hit(me), Some(0));
        assert_eq!(s.hit(mac), Some(1));
        assert!(mac.x < me.x);
    }
}
