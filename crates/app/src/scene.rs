//! The desk: a small 3D renderer drawn with egui shapes. Devices are lit
//! boxes sorted back to front; screens, the pointer and the crossing ribbon
//! are drawn in each screen's own plane so they stay in perspective.
//! Nothing here knows about the daemon: the app sets targets, the scene
//! moves toward them on a clock and reports when everything has settled.

use crate::motion::{self, Tween};
use crate::style::{GROUND, INK, INK_HOVER, INK2, SIGNAL, fit, mono_medium, regular, semibold};
use crate::widgets::mix;
use egui::{Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use one_mk_proto::{Edge, Rect as Monitor, Side};
use std::ops::{Add, Mul, Sub};

/// Distance between neighbouring devices on the desk, in centimetres.
pub const SPACING: f32 = 62.0;
/// Where the camera rests.
const YAW: f32 = -0.2;
const PITCH: f32 = 0.43;

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
/// Frame-rate independent approach: `rate` is the share covered per 60 Hz
/// frame. It snaps once within `eps`, so it stops asking for frames as
/// soon as the rest is invisible.
pub fn approach(cur: f32, target: f32, rate: f32, dt: f32, eps: f32) -> f32 {
    if motion::reduced() {
        return target;
    }
    let k = 1.0 - (1.0 - rate).powf(dt * 60.0);
    let next = lerp(cur, target, k);
    if (next - target).abs() < eps { target } else { next }
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

/// How far into its reveal digit `i` of a pairing code is, 0..1. The
/// sheet and the screen on the desk share it, so the two can be read
/// together in the same rhythm.
pub fn digit_t(i: usize, since: f64, now: f64) -> f32 {
    if motion::reduced() {
        return 1.0;
    }
    motion::progress(now, since + 0.3 + i as f64 * 0.09, 0.3, motion::out_cubic)
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
    /// `raise` (0..1) shrinks the scene and moves it up, to make room for
    /// a sheet below it.
    pub fn new(yaw: f32, pitch: f32, dist: f32, target: V3, rect: Rect, raise: f32) -> Self {
        let cp = pitch.cos();
        let c = target + v(yaw.sin() * cp * dist, pitch.sin() * dist, yaw.cos() * cp * dist);
        let f = norm(target - c);
        let r = norm(cross(f, v(0.0, 1.0, 0.0)));
        let u = cross(r, f);
        let foc = (rect.width() * 0.98).min(rect.height() * 1.68) * (1.0 - 0.22 * raise);
        Self { c, r, u, f, foc, centre: pos2(rect.center().x, rect.top() + rect.height() * (0.61 - 0.20 * raise)) }
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

    /// Scales the picture by `s` about the centre and moves the centre
    /// down by `dy`: the projection is linear in both.
    fn adjust(&mut self, s: f32, dy: f32) {
        self.foc *= s;
        self.centre.y += dy;
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

/// Top left and size of the box round a group of monitors, in pixels.
fn group(screens: &[Monitor]) -> (i32, i32, f32, f32) {
    let (x0, y0, x1, y1) = screens.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(x0, y0, x1, y1), r| (x0.min(r.x), y0.min(r.y), x1.max(r.right()), y1.max(r.bottom())));
    (x0, y0, (x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32)
}

/// Several monitors: each stands on its own, where it is in pixels.
fn is_wall(kind: Kind, screens: &[Monitor]) -> bool {
    kind == Kind::Monitor && screens.len() > 1
}

/// Desk units per pixel of a wall of monitors: a 1920 px monitor as wide
/// as a monitor alone, less if the whole would be wider or taller than this.
fn wall_k(screens: &[Monitor]) -> f32 {
    let (_, _, bw, bh) = group(screens);
    (56.2 / 1920.0f32).min(135.0 / bw).min(50.0 / bh)
}

/// Between the frames of monitors that touch, and from a frame to its
/// glass, in desk units.
const SEAM: f32 = 3.0;
const GLASS: f32 = SEAM / 2.0 + 0.9;
const BASE_H: f32 = 0.8;

/// Half a computer's width on the desk, before scaling. Several monitors
/// stand side by side as wide as their pixels are.
pub fn half_width(kind: Kind, screens: &[Monitor]) -> f32 {
    match kind {
        Kind::Laptop => 15.5,
        Kind::Monitor if screens.len() > 1 => group(screens).2 * wall_k(screens) / 2.0,
        Kind::Monitor => 29.0,
    }
}

/// The monitor a wall shows its name on: the main one (at 0,0), else the largest.
fn main_panel(screens: &[Monitor]) -> usize {
    screens.iter().position(|r| r.x == 0 && r.y == 0).or_else(|| (0..screens.len()).max_by_key(|&i| i64::from(screens[i].w) * i64::from(screens[i].h))).unwrap_or(0)
}

/// Where each monitor sits on a screen, as (u0, w0, u1, w1), `by` desk
/// units in from its pixels: the group keeps its shape and the monitors
/// their sizes, scaled to fit `pl` and centred on it.
fn panels<'a>(screens: &'a [Monitor], pl: &Plane, by: f32) -> impl Iterator<Item = [f32; 4]> + 'a {
    let (x0, y0, [ku, kw, u, w]) = fitted(screens, pl);
    let (pw, ph) = ((pl.u.1 - pl.u.0).abs(), (pl.w.1 - pl.w.0).abs());
    let (gu, gw) = (by / pw, by / ph);
    screens.iter().map(move |r| {
        let (u0, w0) = (u + (r.x - x0) as f32 * ku, w + (r.y - y0) as f32 * kw);
        [u0 + gu, w0 + gw, u0 + r.w as f32 * ku - gu, w0 + r.h as f32 * kw - gw]
    })
}

/// How `panels` fits a group of monitors to `pl`: its top left in pixels,
/// then the fraction of the screen per pixel across and down, and where
/// the group starts on it.
fn fitted(screens: &[Monitor], pl: &Plane) -> (i32, i32, [f32; 4]) {
    let (x0, y0, bw, bh) = group(screens);
    let (pw, ph) = ((pl.u.1 - pl.u.0).abs(), (pl.w.1 - pl.w.0).abs());
    // Desk units per pixel, then as a fraction of the screen across and down.
    let k = (pw / bw).min(ph / bh);
    let (ku, kw) = (k / pw, k / ph);
    (x0, y0, [ku, kw, (1.0 - bw * ku) / 2.0, (1.0 - bh * kw) / 2.0])
}

/// A computer's pixels on the desk, as `panels` lays them out on its
/// screen and carried on past its edges in the same plane.
#[derive(Clone, Copy)]
pub struct PxMap {
    o: V3,
    du: V3,
    dw: V3,
}

impl PxMap {
    fn of(screens: &[Monitor], pl: &Plane) -> Self {
        let (x0, y0, [ku, kw, u, w]) = fitted(screens, pl);
        let a = pl.at(0.0, 0.0);
        let (du, dw) = (pl.at(1.0, 0.0) - a, pl.at(0.0, 1.0) - a);
        Self { o: a + du * (u - x0 as f32 * ku) + dw * (w - y0 as f32 * kw), du: du * ku, dw: dw * kw }
    }
    /// The desk point of pixel (x, y).
    pub fn at(&self, x: f64, y: f64) -> V3 {
        self.o + self.du * x as f32 + self.dw * y as f32
    }
    /// The pixel at desk point `p` (in the plane; the axes are square).
    fn px(&self, p: V3) -> (f64, f64) {
        let d = p - self.o;
        (f64::from(dot(d, self.du) / dot(self.du, self.du)), f64::from(dot(d, self.dw) / dot(self.dw, self.dw)))
    }
    /// Desk units per pixel.
    fn k(&self) -> f32 {
        dot(self.du, self.du).sqrt()
    }
}

/// A peer drawn at an offset: at this computer's scale, so that its
/// pixels line up with ours on the desk and touching screens look it.
pub struct Fit {
    /// This computer's pixels on the desk.
    pub map: PxMap,
    pub scale: f32,
    /// Where the peer's pixel (0,0) is from where it stands.
    origin: V3,
}

impl Fit {
    /// Where the peer stands with its (0,0) at `offset` of ours.
    pub fn pos(&self, offset: (i32, i32)) -> V3 {
        self.map.at(f64::from(offset.0), f64::from(offset.1)) - self.origin
    }
    /// How close to an edge of ours a dropped peer snaps to it, in our pixels.
    pub fn reach(&self) -> f64 {
        f64::from(7.0 / self.map.k())
    }
}

/// Where a peer whose monitors span `peer` (in its pixels) goes when let
/// go with that span's top left at `at` (in ours): beside the nearest of
/// our `edges` (exposed, any side) that it is within `reach` of, or that it
/// covers a screen of ours, so that the screens touch over a good part of
/// it; otherwise where it is (at most its own size away), touching
/// nothing. Returns its side and offset (where its (0,0) lands in our
/// pixels).
pub fn drop_place(mine: &[Monitor], edges: &[Edge], peer: Monitor, at: (f64, f64), reach: f64) -> (Side, (i32, i32)) {
    let (x0, y0, x1, y1) = mine.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(x0, y0, x1, y1), m| (x0.min(m.x), y0.min(m.y), x1.max(m.right()), y1.max(m.bottom())));
    let x = (at.0.round() as i32).clamp(x0 - 2 * peer.w, x1 + peer.w);
    let y = (at.1.round() as i32).clamp(y0 - 2 * peer.h, y1 + peer.h);
    let r = Monitor::new(x, y, peer.w, peer.h);
    let overlaps = |r: &Monitor| mine.iter().any(|m| m.x < r.right() && r.x < m.right() && m.y < r.bottom() && r.y < m.bottom());
    let snapped = edges
        .iter()
        .filter_map(|e| {
            let (len, start) = match e.side {
                Side::Left | Side::Right => (r.h, r.y),
                Side::Top | Side::Bottom => (r.w, r.x),
            };
            // At least a quarter of the shorter one touches.
            let m = ((e.hi - e.lo).min(len) / 4).max(1);
            let along = start.clamp(e.lo + m - len, e.hi - m);
            let c = match e.side {
                Side::Left => Monitor { x: e.at - r.w, y: along, ..r },
                Side::Right => Monitor { x: e.at, y: along, ..r },
                Side::Top => Monitor { x: along, y: e.at - r.h, ..r },
                Side::Bottom => Monitor { x: along, y: e.at, ..r },
            };
            let cost = f64::from(c.x - r.x).hypot(f64::from(c.y - r.y));
            (!overlaps(&c)).then_some((cost, e.side, c))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .filter(|(cost, ..)| *cost <= reach || overlaps(&r));
    let (side, c) = match snapped {
        Some((_, side, c)) => (side, c),
        None => {
            // Far: on whichever side of our desktop it mostly is.
            let b = Monitor::new(x0, y0, x1 - x0, y1 - y0);
            let dx = f64::from(2 * r.x + r.w - 2 * b.x - b.w) / f64::from(b.w + r.w);
            let dy = f64::from(2 * r.y + r.h - 2 * b.y - b.h) / f64::from(b.h + r.h);
            let side = match (dx.abs() >= dy.abs(), dx < 0.0, dy < 0.0) {
                (true, true, _) => Side::Left,
                (true, false, _) => Side::Right,
                (false, _, true) => Side::Top,
                (false, _, false) => Side::Bottom,
            };
            (side, r)
        }
    };
    (side, (c.x - peer.x, c.y - peer.y))
}

pub struct Geo {
    boxes: Vec<Box3>,
    pub screen: Plane,
    deck: Option<Frame3>,
    /// Centre x, z and half sizes of the footprint on the desk.
    foot: [f32; 4],
}

/// One device on the desk. The app sets where it belongs and whether it
/// is shown; the device gets there on its own clock.
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub me: bool,
    pub online: bool,
    /// Where it stands now, and where it belongs.
    pub pos: V3,
    pub target: V3,
    vel: V3,
    pub scale: f32,
    /// Solid fill: 0 while it is only an outline being paired.
    pub alpha: Tween,
    /// How much of the pairing outline is drawn.
    pub wire: Tween,
    /// Height above its place: set down on arrival, picked up to drag.
    pub lift: Tween,
    /// Size on top of `scale`: a little up when picked, down when leaving.
    pub pop: Tween,
    sel: Tween,
    hover: Tween,
    /// A hop to a new place along an arc: where from, and when.
    hop: Option<(V3, f64)>,
    pub wall: [u32; 2],
    /// The second line on its screen: "This computer", "Linux", "Offline"…
    pub subtitle: String,
    /// Its monitors as the daemon reports them; drawn when there is more than one.
    pub screens: Vec<Monitor>,
    /// The line it replaced, and when, for a crossfade.
    sub_was: Option<(String, f64)>,
    /// Removed once it has faded out.
    pub leaving: bool,
    /// Hung in the plane of this computer's screens, at an offset: a
    /// monitor shows no stand there.
    pub mounted: bool,
    // This frame's values, worked out in `step`.
    a: f32,
    w: f32,
    at: V3,
    k: f32,
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
            vel: V3::default(),
            scale: 1.0,
            alpha: Tween::new(1.0),
            wire: Tween::new(0.0),
            lift: Tween::new(0.0),
            pop: Tween::new(1.0),
            sel: Tween::new(0.0),
            hover: Tween::new(0.0),
            hop: None,
            wall,
            subtitle: String::new(),
            screens: Vec::new(),
            sub_was: None,
            leaving: false,
            mounted: false,
            a: 1.0,
            w: 0.0,
            at: V3::default(),
            k: 1.0,
        }
    }

    /// A new computer: it comes down onto its place and fades in.
    pub fn arrive(&mut self, at: V3, now: f64, delay: f64) {
        self.pos = at + v(0.0, 0.0, if motion::reduced() { 0.0 } else { -8.0 });
        self.target = at;
        self.alpha = Tween::from_to(0.0, 1.0, now + delay, motion::fade(0.24), motion::out_cubic);
        self.lift = Tween::from_to(motion::space(6.0), 0.0, now + delay, motion::space(0.42), motion::out_cubic);
        self.a = 0.0;
    }

    /// Fades and shrinks a little, without travelling; then it is removed.
    pub fn leave(&mut self, now: f64) {
        if !self.leaving {
            self.leaving = true;
            self.alpha.go(0.0, now, motion::fade(0.2), motion::in_cubic);
            self.wire.go(0.0, now, motion::fade(0.2), motion::in_cubic);
            self.pop.go(if motion::reduced() { 1.0 } else { 0.96 }, now, 0.2, motion::in_cubic);
        }
    }

    /// Back from leaving (it reappeared in the daemon's list).
    pub fn stay(&mut self, now: f64) {
        if self.leaving {
            self.leaving = false;
            self.alpha.go(1.0, now, motion::fade(0.24), motion::out_cubic);
            self.pop.go(1.0, now, 0.2, motion::out_cubic);
        }
    }

    /// Moves to a new place along a low arc, rather than through its neighbours.
    pub fn hop_to(&mut self, to: V3, now: f64) {
        if to != self.target && !motion::reduced() {
            self.hop = Some((self.pos, now));
        }
        self.target = to;
    }

    pub fn set_subtitle(&mut self, s: &str, now: f64) {
        if self.subtitle != s {
            let was = std::mem::replace(&mut self.subtitle, s.to_owned());
            self.sub_was = (!was.is_empty()).then_some((was, now));
        }
    }

    /// How visible it is right now, 0..1.
    pub fn visible(&self) -> f32 {
        self.a.max(self.w * 0.9)
    }

    pub fn geometry(&self) -> Geo {
        let mut g = Self::geo(self.kind, &self.screens, self.at, self.scale * self.k);
        if self.mounted && self.kind == Kind::Monitor {
            g.boxes.retain(|b| b.screen.is_some()); // no bases or stands
        }
        g
    }

    /// Its parts standing at `at`, `s` times its usual size.
    fn geo(kind: Kind, screens: &[Monitor], at: V3, s: f32) -> Geo {
        let V3 { x, y, z } = at;
        match kind {
            Kind::Monitor if is_wall(kind, screens) => {
                // The screen is the plane the monitors stand in, exactly
                // their pixels across; each monitor is a frame of its own,
                // on a stand of its own unless another one holds it up.
                let (_, _, bw, bh) = group(screens);
                let k = wall_k(screens) * s;
                let (hw, hh) = (bw * k / 2.0, bh * k / 2.0);
                let cy = 13.0 * s + hh;
                let scr = Frame3::axes(v(x, y + cy, z - 14.0));
                let screen = Plane { fr: scr, u: (-hw, hw), w: (hh, -hh) };
                let mut boxes = Vec::with_capacity(3 * screens.len());
                for (r, [u0, w0, u1, w1]) in screens.iter().zip(panels(screens, &screen, SEAM / 2.0)) {
                    let (a, b) = ((lerp(-hw, hw, u0), lerp(-hw, hw, u1)), (lerp(hh, -hh, w1), lerp(hh, -hh, w0)));
                    let held = screens.iter().any(|o| o.y >= r.bottom() && o.x < r.right() && r.x < o.right());
                    if !held {
                        // A stand and base in proportion, as for a monitor alone.
                        let (mx, m) = ((a.0 + a.1) / 2.0, (a.1 - a.0).max(b.1 - b.0) / 58.0);
                        boxes.push(Box3 { fr: Frame3::axes(v(x + mx, y, z - 12.0)), a: (-11.0 * m, 11.0 * m), b: (0.0, BASE_H), c: (-8.0 * m, 8.0 * m), col: 0x3A404A, screen: None });
                        boxes.push(Box3 { fr: Frame3::axes(v(x + mx, y, z - 15.6)), a: (-3.0 * m, 3.0 * m), b: (0.0, cy + (b.0 + b.1) / 2.0), c: (-1.1, 1.1), col: 0x3A404A, screen: None });
                    }
                    boxes.push(Box3 { fr: scr, a, b, c: (-2.2, 0.0), col: 0x30353E, screen: Some(0x0B0D10) });
                }
                Geo { boxes, screen, deck: None, foot: [x, z - 12.0, hw, 8.0 * s] }
            }
            Kind::Monitor => {
                let (hw, hh) = (half_width(Kind::Monitor, screens) * s, 16.5 * s);
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

    /// Advances its own animations; true while any still runs.
    fn step(&mut self, now: f64, dt: f32, dragged: bool) -> bool {
        let mut moving = false;
        if let Some((from, t0)) = self.hop {
            let p = motion::progress(now, t0, 0.45, motion::out_cubic);
            self.pos = from + (self.target - from) * p;
            self.vel = V3::default();
            if p >= 1.0 {
                self.hop = None;
            }
            moving = true;
        } else {
            // A dragged one stays where the drag puts it.
            if !dragged {
                moving |= motion::spring(&mut self.pos.x, &mut self.vel.x, self.target.x, dt, 0.05);
                moving |= motion::spring(&mut self.pos.y, &mut self.vel.y, self.target.y, dt, 0.05);
                moving |= motion::spring(&mut self.pos.z, &mut self.vel.z, self.target.z, dt, 0.05);
            }
        }
        let hop_y = self.hop.map_or(0.0, |(_, t0)| 4.0 * (std::f32::consts::PI * motion::progress(now, t0, 0.45, motion::out_cubic)).sin());
        for t in [&self.alpha, &self.wire, &self.lift, &self.pop, &self.sel, &self.hover] {
            moving |= t.moving(now);
        }
        if let Some((_, t0)) = &self.sub_was {
            if now - t0 > 0.2 {
                self.sub_was = None;
            } else {
                moving = true;
            }
        }
        self.a = self.alpha.value(now);
        self.w = self.wire.value(now);
        self.k = self.pop.value(now);
        self.at = self.pos + v(0.0, self.lift.value(now) + hop_y, 0.0);
        moving
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
    /// The front of a screen box: draw that monitor's screen after it.
    screen: Option<usize>,
    /// The top of a laptop base: draw the keyboard after it.
    deck: bool,
    /// One of the two faces outlined while the device is being paired.
    outline: bool,
}

fn faces_of(cam: &Camera, b: &Box3, owner: usize, first_box: bool, panel: usize, d: &Device, out: &mut Vec<Face>) {
    for (ln, idx, front) in FACES {
        let n = norm(b.fr.ex * ln.x + b.fr.ey * ln.y + b.fr.ez * ln.z);
        let corners = idx.map(|(i, j, k)| {
            b.fr.at(if i { b.a.1 } else { b.a.0 }, if j { b.b.1 } else { b.b.0 }, if k { b.c.1 } else { b.c.0 })
        });
        let ctr = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
        if dot(n, norm(cam.c - ctr)) <= 0.01 {
            continue;
        }
        let proj = corners.map(|p| cam.p(p));
        // A face seen edge-on has no area; drawn anyway, its anti-aliased
        // edge can smear into a long hairline across the room.
        let pts = proj.map(|p| p.0);
        let area = (0..4).map(|i| pts[i].x * pts[(i + 1) % 4].y - pts[(i + 1) % 4].x * pts[i].y).sum::<f32>().abs() / 2.0;
        if area < 1.0 {
            continue;
        }
        // Computers that are off sit a little darker on the desk.
        let lit = (0.32 + 0.78 * dot(n, LIGHT).max(0.0)) * if d.online { 1.0 } else { 0.75 };
        let col = if front { b.screen.unwrap_or(b.col) } else { b.col };
        let top = ln.y > 0.5;
        out.push(Face {
            pts,
            depth: proj.iter().map(|p| p.1).sum::<f32>() / 4.0,
            fill: rgba(hex(col), lit, 1.0),
            owner,
            screen: (front && b.screen.is_some()).then_some(panel),
            deck: d.kind == Kind::Laptop && first_box && top,
            outline: (front && b.screen.is_some()) || (first_box && top),
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

/// A filled quad with one colour per corner. Meshes have hard edges, so
/// a hairline in `edge` goes round it to smooth the steps.
fn quad_colors(painter: &Painter, pts: [Pos2; 4], cols: [Color32; 4], edge: Option<Color32>) {
    let mut m = Mesh::default();
    for (p, c) in pts.iter().zip(cols) {
        m.colored_vertex(*p, c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(m));
    if let Some(e) = edge {
        painter.add(Shape::closed_line(pts.to_vec(), Stroke::new(1.0, e)));
    }
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

/// The first `frac` of a closed outline, as a path.
fn partial(pts: &[Pos2], frac: f32) -> Vec<Pos2> {
    let n = pts.len();
    let lens: Vec<f32> = (0..n).map(|i| pts[i].distance(pts[(i + 1) % n])).collect();
    let mut left = lens.iter().sum::<f32>() * frac.clamp(0.0, 1.0);
    let mut out = vec![pts[0]];
    for i in 0..n {
        if left <= 0.0 {
            break;
        }
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let t = (left / lens[i]).min(1.0);
        out.push(a + (b - a) * t);
        left -= lens[i];
    }
    out
}

/// `pts` pushed out from their centre by `by` points.
fn grown(pts: &[Pos2; 4], by: f32) -> Vec<Pos2> {
    let c = centroid(pts);
    pts.iter().map(|p| *p + (*p - c).normalized() * by).collect()
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
    t0: f64,
    /// How long the head takes to get there.
    secs: f32,
    pub cargo: Cargo,
    landed: bool,
    /// With reduced motion there is no ribbon: the two edges light instead.
    ribbon: bool,
}

/// After the head lands, the tail keeps going this long and is absorbed
/// into the edge, instead of vanishing on one frame.
const TAIL: f32 = 0.18;
/// How long a lit edge takes to go out after a landing.
const FLASH: f32 = 0.6;

impl Flight {
    /// Head position along the arc (0..1) and how far behind the tail is.
    fn head_tail(&self, now: f64) -> (f32, f32) {
        let u = ((now - self.t0) as f32 / self.secs).max(0.0);
        let head = motion::in_out_cubic(u.min(1.0));
        let absorbed = ((u - 1.0) * self.secs / TAIL).clamp(0.0, 1.0);
        (head, (head - 0.42 * (1.0 - absorbed)).max(0.0))
    }

    fn u(&self, now: f64) -> f32 {
        ((now - self.t0) as f32 / self.secs).clamp(0.0, 1.0)
    }
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
    /// When the pointer last landed on a screen (it pops in at the edge).
    cursor_t0: f64,
    pub flight: Option<Flight>,
    /// The edge that lights after a crossing: (from, to, when).
    flash: Option<(String, String, f64)>,
    /// Shown at the top of the arc: the measured one-way time.
    pub latency_label: Option<String>,
    /// A pairing code shown on screens: (device ids, code, when it started).
    pub code: Option<(Vec<String>, String, f64)>,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    /// The device the keyboard is on, ringed.
    pub focused: Option<usize>,
    /// An outlined place beside this computer that invites pairing.
    pub empty_slot: bool,
    pub slot_hover: bool,
    pub slot_focus: bool,
    /// Files are over the window: online computers outlined solid.
    pub drop_targets: bool,
    /// The files tab is open: online computers outlined as places to drop.
    pub drop_preview: Tween,
    /// When the window opened.
    intro_t0: Option<f64>,
    /// Room for a sheet under the desk, 0..1, and where its top is.
    pub raise: Tween,
    pub sheet_top: Option<f32>,
    /// Nothing may go above this (the top bar).
    pub top_clear: f32,
    faces: Vec<Face>,
    /// Half the view's width over its focal length, as last drawn: how
    /// much of the desk fits across at a given distance.
    view_k: f32,
    last_time: f64,
    now: f64,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            devices: vec![],
            yaw: YAW,
            pitch: PITCH,
            yaw_to: YAW,
            pitch_to: PITCH,
            dist: 152.0,
            focus_x: 0.0,
            cursor_on: String::new(),
            // Off the name and the line under it.
            cursor_uw: (0.7, 0.7),
            cursor_uw_to: (0.7, 0.7),
            cursor_t0: f64::NEG_INFINITY,
            flight: None,
            flash: None,
            latency_label: None,
            code: None,
            hover: None,
            selected: None,
            focused: None,
            empty_slot: false,
            slot_hover: false,
            slot_focus: false,
            drop_targets: false,
            drop_preview: Tween::new(0.0),
            intro_t0: None,
            raise: Tween::new(0.0),
            sheet_top: None,
            top_clear: 0.0,
            faces: vec![],
            view_k: 0.6,
            last_time: -1.0,
            now: 0.0,
        }
    }
}

impl Scene {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.devices.iter().position(|d| d.id == id)
    }

    /// True during the opening moments (new devices stagger in).
    pub fn opening(&self, now: f64) -> bool {
        self.intro_t0.is_none_or(|t0| now - t0 < 0.65)
    }

    pub fn reset_camera(&mut self) {
        self.yaw_to = YAW;
        self.pitch_to = PITCH;
    }

    /// Steps every animation. Returns true while anything is still moving.
    pub fn step(&mut self, time: f64, dragging: Option<usize>) -> bool {
        let dt = if self.last_time < 0.0 { 1.0 / 60.0 } else { (time - self.last_time).clamp(0.0, 0.1) as f32 };
        self.last_time = time;
        self.now = time;
        let t0 = *self.intro_t0.get_or_insert(time);
        let mut moving = time - t0 < motion::space(0.6) as f64;
        let mut go = |cur: &mut f32, to: f32, rate: f32, eps: f32| {
            let next = approach(*cur, to, rate, dt, eps);
            moving |= next != to;
            *cur = next;
        };
        go(&mut self.yaw, self.yaw_to, 0.12, 5e-4);
        go(&mut self.pitch, self.pitch_to, 0.12, 5e-4);
        // Frame everything: centres for the usual distance, outer edges so
        // a wide row never runs off the sides of the window.
        let (lo, hi, elo, ehi) = self.devices.iter().filter(|d| !d.leaving).fold((f32::MAX, f32::MIN, f32::MAX, f32::MIN), |(lo, hi, elo, ehi), d| {
            let half = half_width(d.kind, &d.screens) * d.scale;
            (lo.min(d.target.x), hi.max(d.target.x), elo.min(d.target.x - half), ehi.max(d.target.x + half))
        });
        // The empty slot beside this computer counts as part of the scene.
        let (hi, ehi) = if self.empty_slot { (hi.max(self.slot_device().pos.x), ehi.max(self.slot_device().pos.x + 25.0)) } else { (hi, ehi) };
        let (fx, span) = if lo <= hi { ((elo + ehi) / 2.0, hi - lo) } else { (0.0, 0.0) };
        let wide = if lo <= hi { (ehi - elo) / 2.0 * 1.25 / self.view_k.max(0.1) } else { 0.0 };
        go(&mut self.focus_x, fx, 0.1, 0.05);
        go(&mut self.dist, (132.0 + span * 0.5).max(165.0).max(wide), 0.1, 0.05);
        go(&mut self.cursor_uw.0, self.cursor_uw_to.0, 0.08, 1e-3);
        go(&mut self.cursor_uw.1, self.cursor_uw_to.1, 0.08, 1e-3);
        moving |= self.raise.moving(time) || self.drop_preview.moving(time);
        moving |= time - self.cursor_t0 < 0.12;
        for (i, d) in self.devices.iter_mut().enumerate() {
            d.sel.go(f32::from(self.selected == Some(i)), time, motion::fade(0.16), motion::out_expo);
            d.hover.go(f32::from(self.hover == Some(i)), time, motion::fade(0.12), motion::out_cubic);
            moving |= d.step(time, dt, dragging == Some(i));
        }
        self.devices.retain(|d| !(d.leaving && d.alpha.target() == 0.0 && d.a < 0.01 && d.w < 0.01));
        if let Some(f) = &mut self.flight {
            moving = true;
            if !f.landed && time >= f.t0 + f.secs as f64 {
                f.landed = true;
                if f.cargo == Cargo::Pointer {
                    self.cursor_on = f.to.clone();
                    self.cursor_t0 = time;
                }
                self.flash = Some((f.from.clone(), f.to.clone(), time));
            }
            let tail = if f.ribbon { TAIL as f64 } else { 0.0 };
            if time >= f.t0 + f.secs as f64 + tail {
                self.flight = None;
            }
        }
        if let Some((_, _, t)) = &self.flash {
            if time - t < FLASH as f64 {
                moving = true;
            } else {
                self.flash = None;
            }
        }
        if let Some((_, _, t0)) = &self.code {
            moving |= time - t0 < 1.2;
        }
        moving
    }

    /// Where on `a`'s screen the edge toward `b` is, as (u, w).
    fn edge(&self, a: &Device, b: &Device, w: f32) -> (f32, f32) {
        let (dx, dy) = (b.target.x - a.target.x, (b.target.y - b.target.z) - (a.target.y - a.target.z));
        if dx.abs() >= dy.abs() { (if dx < 0.0 { 0.0 } else { 1.0 }, w) } else { (0.5, if dy > 0.0 { 0.0 } else { 1.0 }) }
    }

    /// Sends the pointer (or something else) flying from one screen to
    /// another. Farther screens take a little longer.
    pub fn fly(&mut self, from: &str, to: &str, cargo: Cargo) {
        let pointer = cargo == Cargo::Pointer;
        let (Some(ia), Some(ib)) = (self.index(from), self.index(to)) else {
            if pointer {
                self.cursor_on = to.into();
            }
            return;
        };
        if ia == ib {
            return;
        }
        if pointer {
            let (u, w) = self.edge(&self.devices[ib], &self.devices[ia], self.cursor_uw.1);
            // Land just inside the edge it came through, then drift in a
            // little, but never onto the name in the middle.
            self.cursor_uw = (u, w);
            let (tu, mut tw) = (lerp(u, 0.5, 0.35), lerp(w, 0.5, 0.2));
            if (0.3..=0.7).contains(&tu) && (0.4..=0.6).contains(&tw) {
                tw = if tw < 0.5 { 0.36 } else { 0.64 };
            }
            self.cursor_uw_to = (tu, tw);
        }
        let d = (self.devices[ib].target - self.devices[ia].target).x.abs();
        let secs = match cargo {
            Cargo::Probe => 1.1,
            _ => (0.45 + 0.0025 * d).clamp(0.5, 0.8),
        };
        let ribbon = !motion::reduced();
        self.flight = Some(Flight { from: from.into(), to: to.into(), t0: self.now, secs: if ribbon { secs } else { 0.15 }, cargo, landed: false, ribbon });
    }

    pub fn hit(&self, p: Pos2) -> Option<usize> {
        self.faces.iter().rev().find(|f| self.devices[f.owner].a > 0.5 && inside(p, &f.pts)).map(|f| f.owner)
    }

    /// The screen-space box around a device, as last drawn.
    pub fn bounds(&self, i: usize) -> Option<Rect> {
        self.faces.iter().filter(|f| f.owner == i).flat_map(|f| f.pts).fold(None, |r: Option<Rect>, p| {
            Some(r.map_or(Rect::from_min_max(p, p), |r| r.union(Rect::from_min_max(p, p))))
        })
    }

    /// Screen-space direction of +x on the desk at a device, for dragging.
    pub fn x_axis(&self, rect: Rect, i: usize) -> egui::Vec2 {
        let cam = self.camera(rect);
        let p = self.devices[i].pos;
        cam.pt(p + v(1.0, 0.0, 0.0)) - cam.pt(p)
    }

    /// How a peer with these monitors is drawn at an offset from this
    /// computer; None while either's monitors are unknown.
    pub fn fit(&self, kind: Kind, screens: &[Monitor]) -> Option<Fit> {
        let me = self.devices.first().filter(|m| !m.screens.is_empty() && !screens.is_empty())?;
        let map = PxMap::of(&me.screens, &Device::geo(me.kind, &me.screens, me.target, me.scale).screen);
        let at = |s: f32| PxMap::of(screens, &Device::geo(kind, screens, V3::default(), s).screen);
        // Its pixel size grows in a straight line with its scale (less a
        // bezel that doesn't scale): two sizes give the one that matches.
        let (k1, k2) = (at(1.0).k(), at(2.0).k());
        let scale = (1.0 + (map.k() - k1) / (k2 - k1)).max(0.1);
        Some(Fit { map, scale, origin: at(scale).at(0.0, 0.0) })
    }

    /// The pixel of `map` under `p` on the window, if `p` looks at its plane.
    pub fn wall_px(&self, rect: Rect, map: &PxMap, p: Pos2) -> Option<(f64, f64)> {
        let cam = self.camera(rect);
        let dir = cam.f + cam.r * ((p.x - cam.centre.x) / cam.foc) - cam.u * ((p.y - cam.centre.y) / cam.foc);
        let n = cross(map.du, map.dw);
        let t = dot(map.o - cam.c, n) / dot(dir, n);
        (t.is_finite() && t > 0.0).then(|| map.px(cam.c + dir * t))
    }

    /// Where the middle of device `i`'s screen is on the window.
    pub fn screen_centre(&self, rect: Rect, i: usize) -> Pos2 {
        self.camera(rect).pt(self.devices[i].geometry().screen.at(0.5, 0.5))
    }

    fn camera(&self, rect: Rect) -> Camera {
        let k = self.intro_t0.map_or(0.0, |t0| motion::progress(self.now, t0, motion::space(0.6), motion::out_expo));
        let raise = self.raise.value(self.now);
        let (yaw, dist) = (self.yaw + lerp(-0.12, 0.0, k), self.dist * lerp(1.12, 1.0, k));
        let target = v(self.focus_x, 15.0, -2.0);
        let mut cam = Camera::new(yaw, self.pitch, dist, target, rect, raise);
        // With a sheet open, fit everything between the top bar and the
        // sheet: shrink if it doesn't fit, then centre it in the room left.
        if let Some(sheet_top) = self.sheet_top.filter(|_| raise > 0.0) {
            let (lo, hi) = (rect.top() + self.top_clear, sheet_top - 16.0);
            let ys: Vec<f32> = self
                .devices
                .iter()
                .filter(|d| !d.leaving)
                .flat_map(|d| d.geometry().boxes)
                .flat_map(|b| [(b.a.0, b.b.0, b.c.0), (b.a.1, b.b.1, b.c.1), (b.a.0, b.b.1, b.c.0), (b.a.1, b.b.0, b.c.1)].map(|(a, bb, c)| cam.pt(b.fr.at(a, bb, c)).y))
                .collect();
            let (top, bottom) = ys.iter().fold((f32::MAX, f32::MIN), |(t, b), y| (t.min(*y), b.max(*y)));
            if top < bottom && hi > lo {
                let s = ((hi - lo) / (bottom - top)).min(1.0);
                let (c, mid) = (cam.centre.y, (top + bottom) / 2.0);
                // After scaling about the centre, the middle sits at c + s*(mid - c).
                let dy = (lo + hi) / 2.0 - (c + s * (mid - c));
                cam.adjust(lerp(1.0, s, raise), dy * raise);
            }
        }
        cam
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        let now = self.now;
        let cam = self.camera(rect);
        self.view_k = rect.width() * 0.5 / cam.foc;
        let geo: Vec<Geo> = self.devices.iter().map(Device::geometry).collect();
        self.room(painter, &cam, &geo, rect);

        let mut faces = Vec::new();
        for (i, (d, g)) in self.devices.iter().zip(&geo).enumerate() {
            if d.visible() < 0.01 {
                continue;
            }
            let mut panel = 0;
            for (k, b) in g.boxes.iter().enumerate() {
                faces_of(&cam, b, i, k == 0, panel, d, &mut faces);
                panel += usize::from(b.screen.is_some());
            }
        }
        faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        let preview = self.drop_preview.value(now);
        for f in &faces {
            let d = &self.devices[f.owner];
            let g = &geo[f.owner];
            if d.w > 0.01 {
                // Being paired: opaque faces (so nothing shows through),
                // and only the screen and the base outlined.
                let o = d.a.max(d.w);
                quad(painter, f.pts, fade(mix(GROUND, f.fill, d.a), o));
                if f.outline {
                    let path = partial(&f.pts, d.w);
                    painter.add(Shape::line(path, Stroke::new(1.25, fade(SIGNAL, 0.85 * d.w * (1.0 - d.a)))));
                }
            } else if d.a < 0.99 {
                quad(painter, f.pts, fade(f.fill, d.a));
            } else {
                quad(painter, f.pts, f.fill);
            }
            if let Some(m) = f.screen {
                self.screen(painter, &cam, f.owner, m, &geo);
                self.screen_outline(painter, f, d, preview);
            }
            if f.deck
                && let Some(k) = g.deck
            {
                let s = d.scale * d.k;
                let well = [k.at(-13.0 * s, 0.0, -9.5 * s), k.at(13.0 * s, 0.0, -9.5 * s), k.at(13.0 * s, 0.0, 1.0 * s), k.at(-13.0 * s, 0.0, 1.0 * s)];
                quad(painter, well.map(|p| cam.pt(p)), fade(Color32::from_rgb(20, 22, 26), 0.85 * d.a));
                let pad = [k.at(-5.5 * s, 0.0, 3.0 * s), k.at(5.5 * s, 0.0, 3.0 * s), k.at(5.5 * s, 0.0, 9.0 * s), k.at(-5.5 * s, 0.0, 9.0 * s)];
                quad(painter, pad.map(|p| cam.pt(p)), fade(Color32::WHITE, 0.12 * d.a));
            }
        }
        self.faces = faces;
        self.ribbon(painter, &cam, &geo);
        if self.empty_slot {
            self.slot(painter, &cam);
        }
    }

    /// The line round a screen: where files can go (amber), which one is
    /// selected or hovered (white), where the keyboard is (the focus ring).
    fn screen_outline(&self, painter: &Painter, f: &Face, d: &Device, preview: f32) {
        let i = f.owner;
        let target = d.online && !d.me && !d.leaving && d.w < 0.01;
        if self.drop_targets && target {
            painter.add(Shape::closed_line(grown(&f.pts, 2.5), Stroke::new(1.5, fade(SIGNAL, 0.9 * d.a))));
        } else if preview > 0.01 && target {
            let mut pts = grown(&f.pts, 3.0);
            pts.push(pts[0]);
            let path = partial(&pts[..4], preview);
            painter.extend(Shape::dashed_line(&path, Stroke::new(1.2, fade(SIGNAL, 0.45 * d.a)), 6.0, 5.0));
        }
        let (sel, hover) = (d.sel.value(self.now), d.hover.value(self.now));
        if sel > 0.01 {
            // Closes in on the screen as it is chosen.
            let st = Stroke::new(1.5, Color32::from_white_alpha((150.0 * sel * d.a) as u8));
            painter.add(Shape::closed_line(grown(&f.pts, lerp(6.0, 2.5, sel)), st));
        } else if hover > 0.01 {
            painter.add(Shape::closed_line(grown(&f.pts, 2.5), Stroke::new(1.0, Color32::from_white_alpha((60.0 * hover) as u8))));
        }
        if self.focused == Some(i) {
            painter.add(Shape::closed_line(grown(&f.pts, 6.0), Stroke::new(2.0, SIGNAL)));
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
        let (at, half) = self.devices.first().map_or((V3::default(), 29.0), |m| (m.pos, half_width(m.kind, &m.screens).max(29.0)));
        d.pos = at + v(half + 26.0 + 29.0 * 0.86, 0.0, 0.0);
        d.at = d.pos;
        d.scale = 0.86;
        d
    }

    /// A dashed outline beside this computer: the place a second one goes.
    fn slot(&self, painter: &Painter, cam: &Camera) {
        let g = self.slot_device().geometry();
        let lit = self.slot_hover || self.slot_focus;
        let k = if lit { 1.0 } else { 0.85 };
        let pts: Vec<Pos2> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)].iter().map(|(u, w)| cam.pt(g.screen.at(*u, *w))).collect();
        let st = Stroke::new(1.2, if lit { fade(SIGNAL, 0.9) } else { Color32::from_white_alpha(60) });
        painter.extend(Shape::dashed_line(&pts, st, 6.0, 5.0));
        if self.slot_focus {
            painter.add(Shape::closed_line(grown(&[pts[0], pts[1], pts[2], pts[3]], 4.0), Stroke::new(2.0, SIGNAL)));
        }
        let (h, w) = (pts[0].distance(pts[3]), pts[0].distance(pts[1]));
        let c = cam.pt(g.screen.at(0.5, 0.5));
        let col = if lit { SIGNAL } else { rgba([0xC4, 0xCA, 0xD2], 1.0, k) };
        let title = semibold((h * 0.11).clamp(12.0, 16.0));
        painter.text(c - vec2(0.0, 2.0), egui::Align2::CENTER_BOTTOM, fit(painter, "Add a computer", &title, w * 0.9), title, col);
        let sub = regular((h * 0.07).clamp(11.0, 13.0));
        let mut job = egui::text::LayoutJob::simple("Pair another computer to share this mouse and keyboard".into(), sub, rgba(hex(0x9DA6B1), 1.0, k), w * 0.84);
        job.halign = egui::Align::Center;
        let g = painter.layout_job(job);
        painter.galley(c + vec2(0.0, 4.0), g, INK2);
    }

    fn room(&self, painter: &Painter, cam: &Camera, geo: &[Geo], rect: Rect) {
        // One flat colour behind: the desk's far edge melts into it.
        let sky_low = Color32::from_rgb(0x10, 0x13, 0x18);
        painter.rect_filled(rect, 0.0, sky_low);
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
        // Light from each screen on the desk, and contact shadows that
        // soften as a device is lifted.
        for (d, g) in self.devices.iter().zip(geo) {
            let a = d.a;
            if a < 0.01 {
                continue;
            }
            let wall = is_wall(d.kind, &d.screens);
            let [fx, fz, rx, _] = g.foot;
            if d.online {
                let c = hex(d.wall[0]);
                let pw = if wall { rx.max(30.0) } else { 30.0 };
                pool(painter, cam, v(fx, 0.0, fz + 16.0), pw, 22.0, rgba([c[0].saturating_add(40), c[1].saturating_add(40), c[2].saturating_add(40)], 1.0, 0.16 * a));
            }
            let up = (d.at.y - d.target.y.max(0.0)).clamp(0.0, 6.0) / 6.0;
            let shadow = |[fx, fz, rx, rz]: [f32; 4]| {
                if d.target.y < 1.0 {
                    let (k, feather) = (lerp(1.0, 0.85, up), lerp(5.0, 9.0, up));
                    soft_rect(painter, cam, [fx, fz, rx * k, rz * k], 0.0, feather, Color32::from_black_alpha((lerp(140.0, 70.0, up) * a) as u8));
                } else {
                    soft_rect(painter, cam, [fx, fz, rx * 0.8, rz * 0.8], 0.0, 9.0, Color32::from_black_alpha((60.0 * a) as u8));
                }
            };
            if wall {
                // Under each monitor's base.
                for b in g.boxes.iter().filter(|b| b.b == (0.0, BASE_H)) {
                    shadow([b.fr.o.x, b.fr.o.z, b.a.1 * 12.0 / 11.0, b.c.1]);
                }
            } else {
                shadow(g.foot);
            }
        }
        // The flying pointer lights the desk under it.
        if let Some((f, p)) = self.flight.as_ref().filter(|f| f.ribbon).and_then(|f| self.flight_head(geo, f).map(|p| (f, p))) {
            let s = (std::f32::consts::PI * f.u(self.now)).sin();
            pool(painter, cam, v(p.x, 0.0, p.z), 26.0, 20.0, Color32::from_rgba_unmultiplied(255, 190, 110, (56.0 * s) as u8));
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
        Some(bez(p0, p1, p2, p3, f.head_tail(self.now).0))
    }

    /// The crossing: a ribbon of light along an arc between the two screens.
    fn ribbon(&self, painter: &Painter, cam: &Camera, geo: &[Geo]) {
        let Some(f) = self.flight.as_ref().filter(|f| f.ribbon) else { return };
        let Some([p0, p1, p2, p3]) = self.flight_curve(geo, f) else { return };
        let (head, tail) = f.head_tail(self.now);
        // At the very start and end the ribbon has no length (and no direction).
        if head - tail < 0.004 {
            return;
        }
        const N: usize = 40;
        let pts: Vec<Pos2> = (0..=N).map(|i| cam.pt(bez(p0, p1, p2, p3, lerp(tail, head, i as f32 / N as f32)))).collect();
        // A strip that widens and brightens toward the head, with a one
        // point feather on each side so its edges stay smooth.
        let (tail_col, head_col) = if f.cargo == Cargo::Parcel { ([255, 181, 71], [255, 214, 140]) } else { ([255, 181, 71], [255, 227, 176]) };
        let mut m = Mesh::default();
        for i in 0..=N {
            let k = i as f32 / N as f32;
            let dir = if i == N { pts[i] - pts[i - 1] } else { pts[i + 1] - pts[i] };
            let len = dir.length();
            let n = if len > 1e-4 { vec2(-dir.y, dir.x) / len } else { egui::Vec2::ZERO };
            let w = 0.6 + 2.0 * k;
            let c = |j: usize| (tail_col[j] as f32 + (head_col[j] as f32 - tail_col[j] as f32) * k) as u8;
            let col = Color32::from_rgba_unmultiplied(c(0), c(1), c(2), ((0.15 + 0.85 * k) * 255.0) as u8);
            let clear = Color32::TRANSPARENT;
            m.colored_vertex(pts[i] + n * (w + 1.0), clear);
            m.colored_vertex(pts[i] + n * w, col);
            m.colored_vertex(pts[i] - n * w, col);
            m.colored_vertex(pts[i] - n * (w + 1.0), clear);
            if i > 0 {
                let (a, b) = ((i as u32 - 1) * 4, i as u32 * 4);
                for s in 0..3 {
                    m.add_triangle(a + s, a + s + 1, b + s + 1);
                    m.add_triangle(a + s, b + s + 1, b + s);
                }
            }
        }
        painter.add(Shape::mesh(m));
        if head < 1.0 {
            let hp = cam.pt(bez(p0, p1, p2, p3, head));
            let rim = Stroke::new(1.0, Color32::from_rgb(0xFF, 0xF8, 0xEC));
            if f.cargo == Cargo::Parcel {
                let r = 4.5;
                painter.add(Shape::convex_polygon(vec![hp + vec2(0.0, -r), hp + vec2(r, 0.0), hp + vec2(0.0, r), hp + vec2(-r, 0.0)], SIGNAL, rim));
            } else {
                painter.circle(hp, 3.5, Color32::from_rgb(0xFF, 0xE3, 0xB0), rim);
            }
        }
        if let Some(label) = self.latency_label.as_ref().filter(|_| f.cargo == Cargo::Probe) {
            let u = f.u(self.now);
            let a = (((u - 0.25) * f.secs) / 0.12).clamp(0.0, 1.0).min(((1.0 - u) / 0.2).clamp(0.0, 1.0));
            let apex = cam.pt(bez(p0, p1, p2, p3, 0.5));
            painter.text(apex - vec2(0.0, 14.0), egui::Align2::CENTER_BOTTOM, label, FontId::monospace(12.0), fade(INK, a));
        }
    }

    /// Monitor `m` of a device's screen: wallpaper, its name, the lit
    /// edge, the pointer. A wall of monitors has its name on one of them,
    /// and its edges and pointer on the ones they are on.
    fn screen(&self, painter: &Painter, cam: &Camera, i: usize, m: usize, geo: &[Geo]) {
        let now = self.now;
        let d = &self.devices[i];
        let pl = &geo[i].screen;
        let a = d.visible();
        if a < 0.02 {
            return;
        }
        let wall = is_wall(d.kind, &d.screens);
        // This monitor's glass on the screen; `at` is in it.
        let glass = if wall { panels(&d.screens, pl, GLASS).nth(m).unwrap_or([0.0, 0.0, 1.0, 1.0]) } else { [0.0, 0.0, 1.0, 1.0] };
        let named = !wall || m == main_panel(&d.screens);
        let on_pl = |u: f32, w: f32| cam.pt(pl.at(u, w));
        let at = |u: f32, w: f32| on_pl(lerp(glass[0], glass[2], u), lerp(glass[1], glass[3], w));
        let rect = |u0: f32, w0: f32, u1: f32, w1: f32| [at(u0, w0), at(u1, w0), at(u1, w1), at(u0, w1)];
        let (h, width) = (at(0.5, 0.0).distance(at(0.5, 1.0)), at(0.0, 0.5).distance(at(1.0, 0.5)));
        // Only a computer that is on, and solid, has its wallpaper lit.
        let lit = if d.online { d.a } else { 0.0 };
        if lit > 0.01 {
            let [c0, c1] = d.wall.map(|c| fade(rgba(hex(c), 1.0, 1.0), lit));
            let mid = mix(c0, c1, 0.5);
            let edge = Some(fade(rgba(hex(0x30353E), 0.9, 1.0), lit));
            quad_colors(painter, rect(0.0, 0.0, 1.0, 1.0), [c0, mid, c1, mid], edge);
        }

        if let Some((_, code, t0)) = self.code.as_ref().filter(|(ids, _, _)| ids.contains(&d.id)) {
            if !named {
                return;
            }
            quad(painter, rect(0.12, 0.24, 0.88, 0.76), fade(Color32::from_rgb(14, 17, 21), 0.9 * a));
            // The label only where it fits above the digits.
            let digits_w = if h > 90.0 { 0.62 } else { 0.5 };
            if h > 90.0 {
                painter.text(at(0.5, 0.38), egui::Align2::CENTER_CENTER, "Pairing code", regular((h * 0.07).clamp(11.0, 14.0)), fade(INK2, a));
            }
            let font = mono_medium((h * 0.15).clamp(12.0, 26.0));
            let adv = painter.layout_no_wrap("0".into(), font.clone(), INK).size().x + font.size * 0.1;
            let gap = font.size * 0.4;
            let total = adv * code.len() as f32 + gap;
            let mut x = at(0.5, digits_w).x - total / 2.0;
            for (k, ch) in code.chars().enumerate() {
                if k == 3 {
                    x += gap;
                }
                let e = digit_t(k, *t0, now);
                let y = at(0.5, digits_w).y + motion::space(6.0) * (1.0 - e);
                painter.text(pos2(x + adv / 2.0, y), egui::Align2::CENTER_CENTER, ch, font.clone(), fade(INK, a * e));
                x += adv;
            }
            return;
        }

        // The computer's own name, as the system's display settings show it.
        let ghost = d.w > 0.01 && d.a < 0.5;
        let (name_col, sub_col) = if ghost {
            (fade(INK2, a), fade(INK2, 0.7 * a))
        } else if d.online {
            (fade(INK, a), fade(INK_HOVER, a))
        } else {
            (fade(INK2, a), fade(INK2, 0.85 * a))
        };
        // A monitor of a wall is narrow for the whole wall's name: use its
        // glass, and a point smaller type if need be.
        let name_font = semibold((h * 0.11).clamp(if wall { 11.0 } else { 12.0 }, 22.0));
        let sub_font = regular((h * 0.07).clamp(11.0, 14.0));
        let room = width * if wall { 0.96 } else { 0.86 };
        if named {
            painter.text(at(0.5, 0.47), egui::Align2::CENTER_BOTTOM, fit(painter, &d.name, &name_font, room), name_font, name_col);
            let k = d.sub_was.as_ref().map_or(1.0, |(_, t0)| motion::progress(now, *t0, motion::fade(0.18), motion::out_cubic));
            if let Some((was, _)) = &d.sub_was {
                painter.text(at(0.5, 0.53), egui::Align2::CENTER_TOP, fit(painter, was, &sub_font, room), sub_font.clone(), sub_col.gamma_multiply(1.0 - k));
            }
            painter.text(at(0.5, 0.53), egui::Align2::CENTER_TOP, fit(painter, &d.subtitle, &sub_font, room), sub_font, sub_col.gamma_multiply(k));
        }
        if !d.online || ghost {
            return;
        }

        // The edges toward online neighbours: a faint line at rest, so the
        // edges can be learned; amber only while the pointer is about to
        // cross, crossing, or has just come through. They run along the
        // monitors at that side of the wall, not the gaps between them.
        let ends = if wall { panels(&d.screens, pl, GLASS).fold([1.0f32, 1.0, 0.0, 0.0], |[a, b, c, e], [u0, w0, u1, w1]| [a.min(u0), b.min(w0), c.max(u1), e.max(w1)]) } else { glass };
        for (j, o) in self.devices.iter().enumerate() {
            if j == i || !o.online || o.a < 0.5 || o.w > 0.01 || !(d.me || o.me) {
                continue;
            }
            let (u, w) = self.edge(d, o, 0.5);
            let end = if u == 0.0 {
                glass[0] - ends[0]
            } else if u == 1.0 {
                ends[2] - glass[2]
            } else if w == 0.0 {
                glass[1] - ends[1]
            } else {
                ends[3] - glass[3]
            };
            if end.abs() > 1e-4 {
                continue;
            }
            let near_cursor = if self.cursor_on == d.id && self.flight.is_none() {
                let (cu, cw) = self.cursor_uw;
                (1.0 - if u == 0.5 { (cw - w).abs() } else { (cu - u).abs() } * 5.0).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let involved = self.flight.as_ref().is_some_and(|f| f.cargo != Cargo::Parcel && !f.landed && ((f.from == d.id && f.to == o.id) || (!f.ribbon && f.to == d.id && f.from == o.id)));
            let flash = self.flash.as_ref().filter(|(from, to, _)| to == &d.id && from == &o.id).map_or(0.0, |(_, _, t)| 1.0 - motion::progress(now, *t, FLASH, motion::out_cubic));
            let glow = near_cursor.max(f32::from(involved)).max(flash);
            let strip = if u == 0.5 {
                let (w0, w1) = if w == 0.0 { (0.0, 0.02) } else { (0.98, 1.0) };
                rect(0.08, w0, 0.92, w1)
            } else {
                let (u0, u1) = if u == 0.0 { (0.0, 0.012) } else { (0.988, 1.0) };
                rect(u0, 0.08, u1, 0.92)
            };
            quad(painter, strip, Color32::from_white_alpha((18.0 * d.a) as u8));
            if glow > 0.01 {
                quad(painter, strip, fade(SIGNAL, 0.9 * glow * d.a));
            }
        }

        if self.cursor_on == d.id && self.flight.as_ref().is_none_or(|f| f.cargo != Cargo::Pointer || f.landed) {
            let (mut cu, mut cw) = self.cursor_uw;
            if wall {
                // On the monitor nearest it, kept on its glass.
                let tall = (pl.w.1 - pl.w.0).abs() / (pl.u.1 - pl.u.0).abs();
                let off = |[u0, w0, u1, w1]: [f32; 4]| (u0 - cu).max(cu - u1).max(0.0).hypot((w0 - cw).max(cw - w1).max(0.0) * tall);
                let near = panels(&d.screens, pl, GLASS).enumerate().min_by(|x, y| off(x.1).total_cmp(&off(y.1))).map_or(0, |(n, _)| n);
                if near != m {
                    return;
                }
                (cu, cw) = (cu.clamp(glass[0], glass[2] - 0.02), cw.clamp(glass[1], glass[3] - 0.03));
            }
            // An arrow in screen pixels of a 1600 px wide screen; it pops
            // in at the edge it came through.
            let pop = lerp(0.85, 1.0, motion::progress(now, self.cursor_t0, motion::space(0.12), motion::out_cubic));
            let s = 3.6 / 1600.0 * pop;
            let aspect = if d.kind == Kind::Laptop { 1470.0 / 956.0 } else { 16.0 / 9.0 };
            // A wall of monitors: as big and as square as on a monitor alone.
            let (su, sw) = if wall {
                let (_, _, bw, bh) = group(&d.screens);
                let k = wall_k(&d.screens);
                (s * 56.2 / (bw * k), s * aspect * 31.2 / (bh * k))
            } else {
                (s, s * aspect)
            };
            let arrow = [(0.0, 0.0), (0.0, 17.0), (4.2, 13.2), (7.2, 19.6), (9.8, 18.4), (6.9, 12.2), (12.2, 11.8)];
            let pts: Vec<Pos2> = arrow.iter().map(|(x, y)| on_pl(cu + x * su, cw + y * sw)).collect();
            let white = fade(Color32::WHITE, d.a);
            // Not convex: draw it as two convex parts.
            painter.add(Shape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[5], pts[6]], white, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![pts[2], pts[3], pts[4], pts[5]], white, Stroke::NONE));
            painter.add(Shape::closed_line(pts, Stroke::new(0.8, fade(Color32::from_rgb(0x11, 0x11, 0x11), d.a))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(s: &mut Scene, from: f64) -> f64 {
        let mut t = from;
        for _ in 0..600 {
            t += 1.0 / 60.0;
            if !s.step(t, None) {
                return t;
            }
        }
        panic!("the scene never settled");
    }

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
            x = approach(x, 1.0, 0.12, 1.0 / 60.0, 1e-3);
        }
        assert_eq!(x, 1.0);
    }

    fn two() -> Scene {
        let mut s = Scene::default();
        s.devices.push(Device::new("me", "desk", Kind::Monitor, true, [0x1D2D3C, 0x0D151E]));
        let mut mac = Device::new("mac", "mac", Kind::Laptop, false, [0x35312C, 0x16140F]);
        mac.pos.x = -SPACING;
        mac.target.x = -SPACING;
        s.devices.push(mac);
        s
    }

    #[test]
    fn devices_can_be_picked() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(900.0, 430.0));
        let mut s = two();
        settle(&mut s, 0.0);
        let ctx = egui::Context::default();
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let painter = Painter::new(ctx.clone(), egui::LayerId::background(), rect);
            s.paint(&painter, rect);
        });
        out.textures_delta.clear();
        let cam = s.camera(rect);
        let me = cam.pt(s.devices[0].geometry().screen.at(0.5, 0.5));
        let mac = cam.pt(s.devices[1].geometry().screen.at(0.5, 0.5));
        assert_eq!(s.hit(me), Some(0));
        assert_eq!(s.hit(mac), Some(1));
        assert!(mac.x < me.x);
        assert!(s.bounds(1).is_some_and(|b| b.contains(mac)));
    }

    #[test]
    fn everything_comes_to_rest() {
        let mut s = two();
        let t = settle(&mut s, 0.0);
        s.fly("me", "mac", Cargo::Pointer);
        s.devices[1].leave(t);
        let t = settle(&mut s, t);
        assert!(s.flight.is_none());
        assert_eq!(s.cursor_on, "mac");
        assert_eq!(s.devices.len(), 1, "the one that left is gone");
        assert!(!s.step(t + 1.0, None), "and nothing asks for frames");
    }

    #[test]
    fn arrivals_come_down_onto_their_place() {
        let mut s = two();
        let t = settle(&mut s, 0.0);
        let mut d = Device::new("pc", "pc", Kind::Monitor, false, [0x283430, 0x101614]);
        d.arrive(v(SPACING, 0.0, 0.0), t, 0.0);
        s.devices.push(d);
        s.step(t + 0.1, None);
        assert!(s.devices[2].at.y > 0.5, "starts above the desk");
        settle(&mut s, t + 0.1);
        assert_eq!(s.devices[2].at, v(SPACING, 0.0, 0.0));
    }

    #[test]
    fn every_monitor_lands_on_its_screen_in_its_shape() {
        // A Mac with three monitors, one of them portrait, beside a laptop.
        let mut mac = Device::new("me", "mac", Kind::Monitor, true, [0, 0]);
        mac.screens = vec![Monitor::new(0, 0, 1920, 1080), Monitor::new(-2592, 98, 1512, 982), Monitor::new(-1080, -840, 1080, 1920), Monitor::new(1920, 0, 1920, 1080)];
        let pl = mac.geometry().screen;
        let (pw, ph) = ((pl.u.1 - pl.u.0).abs(), (pl.w.1 - pl.w.0).abs());
        let p: Vec<[f32; 4]> = panels(&mac.screens, &pl, GLASS).collect();
        assert_eq!(p.len(), 4);
        for [u0, w0, u1, w1] in &p {
            assert!(0.0 <= *u0 && u0 < u1 && *u1 <= 1.0 + 1e-6 && 0.0 <= *w0 && w0 < w1 && *w1 <= 1.0 + 1e-6, "{p:?}");
        }
        let size = |[u0, w0, u1, w1]: [f32; 4]| ((u1 - u0) * pw, (w1 - w0) * ph);
        let (w, h) = size(p[2]);
        assert!(h > w, "the portrait monitor stands upright: {w} x {h}");
        let (w, h) = size(p[0]);
        assert!(((w + 2.0 * GLASS) / (h + 2.0 * GLASS) - 16.0 / 9.0).abs() < 0.01, "the main one keeps 16:9, frame aside: {w} x {h}");
        assert!(p[1][0] < p[2][0] && p[2][0] < p[0][0] && p[0][0] < p[3][0], "left to right as on the desk");
        assert!(p[2][1] < p[0][1], "the portrait one reaches higher");
        assert_eq!(main_panel(&mac.screens), 0, "named on the main one");
    }

    #[test]
    fn several_monitors_stand_apart_as_they_are_arranged() {
        let mut mac = Device::new("me", "mac", Kind::Monitor, true, [0, 0]);
        mac.screens = vec![Monitor::new(0, 0, 1920, 1080), Monitor::new(-2592, 98, 1512, 982), Monitor::new(-1080, -840, 1080, 1920), Monitor::new(1920, 0, 1920, 1080)];
        let g = mac.geometry();
        // Each its own frame, base and stand.
        let frames: Vec<&Box3> = g.boxes.iter().filter(|b| b.screen.is_some()).collect();
        assert_eq!((frames.len(), g.boxes.len()), (4, 12));
        assert_eq!(g.boxes.iter().filter(|b| b.b == (0.0, BASE_H)).count(), 4, "a base under each");
        let k = wall_k(&mac.screens);
        let x = |b: &Box3| b.fr.o.x + (b.a.0 + b.a.1) / 2.0;
        let y = |b: &Box3| b.fr.o.y + (b.b.0 + b.b.1) / 2.0;
        for (b, r) in frames.iter().zip(&mac.screens) {
            let (w, h) = (b.a.1 - b.a.0, b.b.1 - b.b.0);
            // Its pixels at the wall's scale, less the seam.
            assert!((w + SEAM - r.w as f32 * k).abs() < 1e-3 && (h + SEAM - r.h as f32 * k).abs() < 1e-3, "{w} x {h} for {r:?}");
        }
        let portrait = frames[2];
        assert!(portrait.b.1 - portrait.b.0 > portrait.a.1 - portrait.a.0, "the portrait one is taller than wide");
        // Centres as far apart as the monitors' pixels, up as well as across.
        for (i, j) in [(0, 1), (0, 2), (0, 3), (1, 2)] {
            let (a, b) = (frames[i], frames[j]);
            let (ra, rb) = (&mac.screens[i], &mac.screens[j]);
            let dx = (rb.x + rb.w / 2 - ra.x - ra.w / 2) as f32 * k;
            let dy = -((rb.y + rb.h / 2) - (ra.y + ra.h / 2)) as f32 * k;
            assert!((x(b) - x(a) - dx).abs() < 0.01 && (y(b) - y(a) - dy).abs() < 0.01, "{i} {j}");
        }
        // Neighbours in a row don't touch: the seam shows.
        let mut row: Vec<&&Box3> = frames.iter().collect();
        row.sort_by(|a, b| a.a.0.total_cmp(&b.a.0));
        for w in row.windows(2) {
            assert!(w[1].a.0 - w[0].a.1 >= SEAM - 1e-3, "a gap between neighbours");
        }
        // And the desk keeps their room: nothing sticks out past half_width.
        let half = half_width(Kind::Monitor, &mac.screens);
        assert!(g.boxes.iter().all(|b| b.fr.o.x + b.a.0 >= -half - 1e-3 && b.fr.o.x + b.a.1 <= half + 1e-3));
        // Hung on another computer's wall: frames only.
        mac.mounted = true;
        assert_eq!(mac.geometry().boxes.len(), 4);
    }

    #[test]
    fn a_desk_with_many_monitors_draws_picks_and_rests() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(900.0, 430.0));
        let mut s = two();
        s.devices[0].screens = vec![Monitor::new(0, 0, 1920, 1080), Monitor::new(-2592, 98, 1512, 982), Monitor::new(-1080, -840, 1080, 1920), Monitor::new(1920, 0, 1920, 1080)];
        s.devices[1].screens = vec![Monitor::new(0, 0, 1920, 1080)];
        let half = half_width(Kind::Monitor, &s.devices[0].screens);
        assert!(half > 29.0, "a row of monitors stands wider than one: {half}");
        // Where the app puts its neighbour: clear of the wider wall.
        s.devices[1].pos.x = -(half + 26.0 + 15.5);
        s.devices[1].target.x = s.devices[1].pos.x;
        let ctx = egui::Context::default();
        let mut t = 0.0;
        // Twice: the first draw tells the camera how wide the view is.
        for _ in 0..2 {
            t = settle(&mut s, t);
            let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
                let painter = Painter::new(ctx.clone(), egui::LayerId::background(), rect);
                s.paint(&painter, rect);
            });
            out.textures_delta.clear();
        }
        let cam = s.camera(rect);
        for i in 0..2 {
            assert_eq!(s.hit(cam.pt(s.devices[i].geometry().screen.at(0.5, 0.5))), Some(i));
        }
        assert!(!s.step(t + 1.0, None), "monitors ask for no frames of their own");
    }

    #[test]
    fn pairing_codes_reveal_in_one_rhythm() {
        assert_eq!(digit_t(0, 0.0, 0.3), 0.0);
        assert!(digit_t(0, 0.0, 0.45) > 0.0);
        assert!(digit_t(0, 0.0, 0.5) > digit_t(1, 0.0, 0.5), "left to right");
        assert_eq!(digit_t(5, 0.0, 1.1), 1.0, "all shown by 1.1 s");
    }
    /// The user's desk: three monitors in a row, the right one shorter.
    fn user_desk() -> one_mk_proto::Desktop {
        one_mk_proto::Desktop::new(vec![Monitor::new(-1920, 0, 1920, 1080), Monitor::new(0, 0, 1920, 1080), Monitor::new(1920, 0, 1512, 982)])
    }

    #[test]
    fn a_dropped_computer_snaps_to_the_monitor_it_is_near() {
        use one_mk_proto::touching;
        let mine = user_desk();
        let edges: Vec<Edge> = Side::ALL.into_iter().flat_map(|s| mine.exposed_edges(s)).collect();
        let peer = one_mk_proto::Desktop::new(vec![Monitor::new(0, 0, 1920, 1080)]);
        let place = |at| drop_place(&mine.screens, &edges, peer.screens[0], at, 300.0);
        let touches = |(side, offset)| !touching(&mine, &peer, side, offset).is_empty();
        // A little above the middle screen: onto its top edge.
        assert_eq!(place((100.0, -1100.0)), (Side::Top, (100, -1080)));
        assert!(touches(place((100.0, -1100.0))));
        // A little right of the right screen, lower down: beside it, not
        // beside the taller middle one whose edge it hides.
        assert_eq!(place((3500.0, 100.0)), (Side::Right, (3432, 100)));
        assert!(touches(place((3500.0, 100.0))));
        // So low it would barely touch: pulled up to a quarter of the edge.
        assert_eq!(place((3440.0, 900.0)), (Side::Right, (3432, 982 - 982 / 4)));
        // Dropped onto the screens: out to the nearest free edge.
        assert_eq!(place((200.0, 200.0)), (Side::Bottom, (200, 1080)));
        assert!(touches(place((200.0, 200.0))));
        // Far away: exactly there, and nothing touches.
        assert_eq!(place((5200.0, 300.0)), (Side::Right, (5200, 300)));
        assert!(!touches(place((5200.0, 300.0))));
        assert_eq!(place((-1000.0, -2000.0)).0, Side::Top);
        // No farther than its own size.
        assert_eq!(place((99999.0, -99999.0)), (Side::Top, (3432 + 1920, -2160)));
        // A peer whose monitors don't start at (0,0): the offset is for its (0,0).
        let wide = Monitor::new(-1920, 0, 3840, 1080);
        assert_eq!(drop_place(&mine.screens, &edges, wide, (3500.0, 0.0), 300.0), (Side::Right, (3432 + 1920, 0)));
    }

    #[test]
    fn pixels_map_onto_the_desk_and_back() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(900.0, 430.0));
        let mut s = two();
        s.devices[0].screens = user_desk().screens;
        s.devices[1].screens = vec![Monitor::new(0, 0, 1920, 1080)];
        settle(&mut s, 0.0);
        let fit = s.fit(Kind::Monitor, &s.devices[1].screens).unwrap();
        // Our pixels land on the monitors as drawn: the middle one's top
        // left is half a seam in from pixel (0,0), the right one's right
        // edge half a seam in from 3432, where a peer snaps beside it.
        let g = s.devices[0].geometry();
        let frames: Vec<&Box3> = g.boxes.iter().filter(|b| b.screen.is_some()).collect();
        let corner = |b: &Box3, right: bool| b.fr.at(if right { b.a.1 } else { b.a.0 }, b.b.1, 0.0);
        let (a, b) = (fit.map.at(0.0, 0.0), corner(frames[1], false));
        assert!(((a - b).x + SEAM / 2.0).abs() < 1e-3 && ((a - b).y - SEAM / 2.0).abs() < 1e-3, "{a:?} {b:?}");
        let (a, b) = (fit.map.at(3432.0, 0.0), corner(frames[2], true));
        assert!(((a - b).x - SEAM / 2.0).abs() < 1e-3 && ((a - b).y - SEAM / 2.0).abs() < 1e-3, "{a:?} {b:?}");
        // The window and the pixels agree, both ways.
        let cam = s.camera(rect);
        let (x, y) = s.wall_px(rect, &fit.map, cam.pt(fit.map.at(3432.0, 500.0))).unwrap();
        assert!((x - 3432.0).abs() < 2.0 && (y - 500.0).abs() < 2.0, "{x} {y}");
        // A peer at our scale: 1920 of its pixels as wide as 1920 of ours.
        let pos = fit.pos((3432, 0));
        let pg = Device::geo(Kind::Monitor, &s.devices[1].screens, pos, fit.scale);
        let pm = PxMap::of(&s.devices[1].screens, &pg.screen);
        let (c, d) = (pm.at(0.0, 0.0), fit.map.at(3432.0, 0.0));
        assert!((c - d).x.abs() < 0.01 && (c - d).y.abs() < 0.01, "its (0,0) on ours");
        assert!((pm.k() - fit.map.k()).abs() < 1e-3 * fit.map.k(), "same scale");
    }
}
