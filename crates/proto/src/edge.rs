//! Whether the pointer may cross an edge: dead corners, a dwell time, a
//! modifier to hold and the fullscreen guard. Also which computer
//! `one-mk switch next` picks.
//!
//! The input backends capture the pointer as soon as it hits an armed edge
//! and report `Activated`; the daemon then asks [`Approach::start`] whether
//! to go on to the peer, hand the pointer back, or hold it at the edge for
//! a moment (a dwell, or waiting for the modifier). While it is held, the
//! captured mouse moves a virtual pointer along the armed piece of edge it
//! hit, and moving away from that edge (into the desktop, or along it past
//! where it is armed) hands it back where it would have been. All of this
//! is measured against that piece, which may lie inside the desktop's
//! bounding box (a peer placed beside a shorter monitor), never against
//! the bounding box itself.
//!
//! Input captured while the pointer is held was meant for this computer,
//! so it is typed here when the pointer is handed back. But only what can
//! never stay held: the key-up of a key held now still comes from the real
//! keyboard, past the injector, so a key-down replayed through the
//! injector would never be released there (on Windows and macOS the
//! injector would repeat it forever; on Linux the virtual keyboard would
//! keep it down). A key is therefore typed as a tap (down and up), a click
//! as a whole click (the same holds for buttons), and the
//! modifiers pressed while held go down just around the click, scroll or
//! key they modify and up right after. See [`Next::Release`].

use crate::{Desktop, Edge, Pos, Side};
use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// How far (in px) the pointer may move back from the edge, while held
/// there, before it is handed back. Small enough to feel immediate, large
/// enough that hand jitter while pushing doesn't count.
pub const AWAY: f64 = 4.0;

/// A modifier key the user must hold for the pointer to cross.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modifier {
    #[default]
    None,
    Shift,
    Ctrl,
    Alt,
}

impl Modifier {
    pub const ALL: [Modifier; 4] = [Modifier::None, Modifier::Shift, Modifier::Ctrl, Modifier::Alt];

    pub fn as_str(self) -> &'static str {
        match self {
            Modifier::None => "none",
            Modifier::Shift => "shift",
            Modifier::Ctrl => "ctrl",
            Modifier::Alt => "alt",
        }
    }

    /// evdev codes of its left and right keys.
    fn keys(self) -> &'static [u32] {
        match self {
            Modifier::None => &[],
            Modifier::Shift => &[KEY_LEFTSHIFT, KEY_RIGHTSHIFT],
            Modifier::Ctrl => &[KEY_LEFTCTRL, KEY_RIGHTCTRL],
            Modifier::Alt => &[KEY_LEFTALT, KEY_RIGHTALT],
        }
    }
}

impl fmt::Display for Modifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Modifier {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_ascii_lowercase();
        let s = match s.as_str() {
            "control" => "ctrl",
            "option" => "alt",
            other => other,
        };
        Modifier::ALL
            .into_iter()
            .find(|m| m.as_str() == s)
            .ok_or_else(|| format!("unknown modifier {s:?} (expected none, shift, ctrl or alt)"))
    }
}

const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const KEY_LEFTALT: u32 = 56;
const KEY_RIGHTCTRL: u32 = 97;
const KEY_RIGHTALT: u32 = 100;
const KEY_LEFTMETA: u32 = 125;
const KEY_RIGHTMETA: u32 = 126;

/// The modifier keys held on this computer, left and right separately (so
/// releasing one Shift while holding the other still counts as Shift).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(u8);

impl Mods {
    /// evdev codes of every modifier key.
    pub const KEYS: [u32; 8] = [
        KEY_LEFTSHIFT,
        KEY_RIGHTSHIFT,
        KEY_LEFTCTRL,
        KEY_RIGHTCTRL,
        KEY_LEFTALT,
        KEY_RIGHTALT,
        KEY_LEFTMETA,
        KEY_RIGHTMETA,
    ];

    /// From evdev codes of held keys (others are ignored).
    pub fn from_keys(codes: impl IntoIterator<Item = u32>) -> Self {
        let mut m = Mods::default();
        for c in codes {
            m.update(c, true);
        }
        m
    }

    /// From an XKB modifier mask (libei reports these): Shift is bit 0,
    /// Control bit 2, Mod1 (Alt) bit 3, Mod4 (Super) bit 6 in every
    /// standard keymap. Sides are unknown, so the left keys stand for both.
    pub fn from_xkb(mask: u32) -> Self {
        let mut codes = Vec::new();
        for (bit, code) in [(0, KEY_LEFTSHIFT), (2, KEY_LEFTCTRL), (3, KEY_LEFTALT), (6, KEY_LEFTMETA)] {
            if mask & (1 << bit) != 0 {
                codes.push(code);
            }
        }
        Self::from_keys(codes)
    }

    /// Records a key; returns false if it is not a modifier.
    pub fn update(&mut self, code: u32, down: bool) -> bool {
        let Some(i) = Self::KEYS.iter().position(|&k| k == code) else {
            return false;
        };
        if down {
            self.0 |= 1 << i;
        } else {
            self.0 &= !(1 << i);
        }
        true
    }

    pub fn is_modifier(code: u32) -> bool {
        Self::KEYS.contains(&code)
    }

    pub fn has(self, m: Modifier) -> bool {
        m == Modifier::None || m.keys().iter().any(|&k| self.held(k))
    }

    fn held(self, code: u32) -> bool {
        Self::KEYS.iter().position(|&k| k == code).is_some_and(|i| self.0 & (1 << i) != 0)
    }

    /// The held keys, as evdev codes.
    pub fn keys(self) -> impl Iterator<Item = u32> {
        Self::KEYS.into_iter().filter(move |&k| self.held(k))
    }
}

/// The rules for crossing one edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeGuard {
    /// Crossings within this many pixels of a screen's corner are ignored
    /// (0: off).
    pub corner: u32,
    /// The pointer must stay at the edge this long before it crosses (0: off).
    pub dwell_ms: u32,
    /// A modifier to hold while crossing.
    pub modifier: Modifier,
    /// Don't cross while the focused window here is fullscreen.
    pub fullscreen: bool,
}

/// Why a crossing was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `one-mk lock-pointer` is on.
    Locked,
    Corner,
    Fullscreen,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Refusal::Locked => "the pointer is locked to this computer",
            Refusal::Corner => "too close to a corner",
            Refusal::Fullscreen => "a fullscreen window has the focus",
        }
    }
}

/// What the focused window looks like, as far as this OS can tell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Focus {
    /// It covers its whole monitor.
    pub fullscreen: bool,
    /// Names it goes by: the program's file name without extension, its
    /// window class, its bundle id. Matched without regard to case.
    pub apps: Vec<String>,
}

/// Whether the fullscreen guard stops a crossing. `focus` is `None` when
/// this OS can't tell (GNOME on Wayland, for one): then the guard never
/// blocks, so an unsupported system behaves as if it were off. Programs in
/// `allow` may always be left; those in `block` never, fullscreen or not.
pub fn fullscreen_blocks(enabled: bool, focus: Option<&Focus>, allow: &[String], block: &[String]) -> bool {
    let Some(f) = focus.filter(|_| enabled) else {
        return false;
    };
    let listed = |list: &[String]| f.apps.iter().any(|a| list.iter().any(|l| app_matches(a, l)));
    if listed(allow) {
        return false;
    }
    listed(block) || f.fullscreen
}

/// `entry` names `app`: equal ignoring case, also when `app` is a path or
/// ends in `.exe` or `.app`.
pub fn app_matches(app: &str, entry: &str) -> bool {
    let base = app.rsplit(['/', '\\']).next().unwrap_or(app);
    let lower = base.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").or_else(|| lower.strip_suffix(".app")).unwrap_or(&lower);
    let entry = entry.trim().to_ascii_lowercase();
    !entry.is_empty() && (stem == entry || lower == entry || app.eq_ignore_ascii_case(&entry))
}

/// Whether `pos`, at `edge`, is within `corner` px of a corner of the
/// desktop's outline along that edge: an end of the exposed part of the
/// edge of the monitor `pos` is on. That is where the outline turns, or
/// where two monitors meet in a straight line (each has its own corner
/// there: a maximized window's close button). A monitor's corner covered
/// by a neighbour is inside the desktop and doesn't count.
pub fn in_dead_corner(desktop: &Desktop, edge: &Edge, pos: Pos, corner: u32) -> bool {
    if corner == 0 {
        return false;
    }
    let along = along(edge.side, desktop.clamp(pos));
    let piece = desktop
        .exposed_edges(edge.side)
        .into_iter()
        .find(|e| e.at == edge.at && f64::from(e.lo) <= along && along < f64::from(e.hi))
        .unwrap_or(*edge);
    let to_start = along - f64::from(piece.lo);
    let to_end = f64::from(piece.hi) - 1.0 - along;
    to_start.min(to_end) < f64::from(corner)
}

/// `p`'s coordinate along a `side` edge.
fn along(side: Side, p: Pos) -> f64 {
    match side {
        Side::Left | Side::Right => p.y,
        Side::Top | Side::Bottom => p.x,
    }
}

/// The row or column of pixels just inside `edge`, where the pointer
/// pushing against it is.
fn line(edge: &Edge) -> f64 {
    match edge.side {
        Side::Left | Side::Top => f64::from(edge.at),
        Side::Right | Side::Bottom => f64::from(edge.at) - 1.0,
    }
}

/// How far `p` is from `edge`, inwards (negative beyond it).
fn depth(edge: &Edge, p: Pos) -> f64 {
    match edge.side {
        Side::Left => p.x - line(edge),
        Side::Right => line(edge) - p.x,
        Side::Top => p.y - line(edge),
        Side::Bottom => line(edge) - p.y,
    }
}

/// `p` moved onto the line of `edge`, keeping its place along it.
fn onto(edge: &Edge, p: Pos) -> Pos {
    match edge.side {
        Side::Left | Side::Right => Pos::new(line(edge), p.y),
        Side::Top | Side::Bottom => Pos::new(p.x, line(edge)),
    }
}

/// Whether `p` is on `edge` (as [`onto`] puts it), within its ends.
fn on(edge: &Edge, p: Pos) -> bool {
    let a = along(edge.side, p);
    depth(edge, p).abs() < 1.0 && f64::from(edge.lo) <= a && a < f64::from(edge.hi)
}

/// The pointer is held at an edge, waiting to cross.
#[derive(Clone, Debug, PartialEq)]
pub struct Approach {
    /// The armed piece of edge the pointer is held at.
    edge: Edge,
    /// Where the pointer would be: on the edge, moved along it by the mouse.
    pos: Pos,
    guard: EdgeGuard,
    /// The end of the dwell, until it has passed.
    deadline: Option<Instant>,
    mods: Mods,
    /// Modifier keys pressed while held (captured, so this computer never
    /// saw them): they modify what is typed here if the pointer is handed
    /// back.
    swallowed: Mods,
}

/// What [`Approach::start`] decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Start {
    /// Go to the peer, entering at this point of our edge.
    Cross(Pos),
    /// Hand the pointer back where it hit the edge.
    Refuse(Refusal),
    /// Hold it at the edge.
    Wait(Approach),
}

/// What to do next while the pointer is held.
#[derive(Clone, Debug, PartialEq)]
pub enum Next {
    Wait,
    /// Go to the peer, entering at this point of our edge.
    Cross(Pos),
    /// Hand the pointer back at `pos`, then on this computer: press the
    /// `around` modifiers (pressed while held and still down), type `keys`
    /// (code, down) and replay the captured click (as a whole click: down
    /// and up) or scroll that ended the wait, if any, then release `around`
    /// again (in reverse order).
    /// Nothing is left down: the user's own key-ups go past the injector.
    Release {
        pos: Pos,
        around: Vec<u32>,
        keys: Vec<(u32, bool)>,
    },
}

impl Approach {
    /// The pointer hit the armed `edge` (a piece of our desktop's
    /// outline, see [`Desktop::exposed_edges`]) at `pos`. `mods` are the
    /// modifiers held here (as far as the backend can tell); `fullscreen`
    /// is the outcome of [`fullscreen_blocks`].
    pub fn start(
        guard: EdgeGuard,
        desktop: &Desktop,
        edge: Edge,
        pos: Pos,
        mods: Mods,
        fullscreen: bool,
        now: Instant,
    ) -> Start {
        if fullscreen {
            return Start::Refuse(Refusal::Fullscreen);
        }
        // On the edge, within its ends (the backend reports where the
        // pointer was stopped, which may be a pixel or two off).
        let hi = f64::from(edge.hi) - 1.0 / 256.0;
        let pos = match edge.side {
            Side::Left | Side::Right => Pos::new(pos.x, pos.y.clamp(f64::from(edge.lo), hi)),
            Side::Top | Side::Bottom => Pos::new(pos.x.clamp(f64::from(edge.lo), hi), pos.y),
        };
        let pos = desktop.clamp(onto(&edge, pos));
        if in_dead_corner(desktop, &edge, pos, guard.corner) {
            // Refused rather than held: a corner is usually reached on the
            // way somewhere else (a hot corner, a close button).
            return Start::Refuse(Refusal::Corner);
        }
        let deadline = (guard.dwell_ms > 0).then(|| now + Duration::from_millis(u64::from(guard.dwell_ms)));
        let a = Approach { edge, pos, guard, deadline, mods, swallowed: Mods::default() };
        match a.ready(desktop, now) {
            true => Start::Cross(pos),
            false => Start::Wait(a),
        }
    }

    pub fn side(&self) -> Side {
        self.edge.side
    }

    pub fn edge(&self) -> Edge {
        self.edge
    }

    pub fn pos(&self) -> Pos {
        self.pos
    }

    /// When to call [`Approach::timer`]; `None` when only input can end
    /// the wait (no timer runs then).
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn ready(&self, desktop: &Desktop, now: Instant) -> bool {
        self.deadline.is_none_or(|d| now >= d)
            && self.mods.has(self.guard.modifier)
            && !in_dead_corner(desktop, &self.edge, self.pos, self.guard.corner)
    }

    fn check(&mut self, desktop: &Desktop, now: Instant) -> Next {
        if self.deadline.is_some_and(|d| now >= d) {
            self.deadline = None; // passed: no more timers
        }
        if self.ready(desktop, now) { Next::Cross(self.pos) } else { Next::Wait }
    }

    /// Hands the pointer back. `around`: there is input to replay, which
    /// the held modifiers modify (alone, pressing and releasing them would
    /// only do harm: a lone Alt opens the menu bar on Windows).
    fn release(&mut self, pos: Pos, around: bool, keys: Vec<(u32, bool)>) -> Next {
        let around = match around {
            true => self.swallowed.keys().filter(|&k| self.mods.held(k)).collect(),
            false => Vec::new(),
        };
        Next::Release { pos, around, keys }
    }

    pub fn timer(&mut self, desktop: &Desktop, now: Instant) -> Next {
        self.check(desktop, now)
    }

    /// Captured mouse motion while held. Outwards (beyond the edge, even
    /// where another of our monitors is) only pushes against it.
    pub fn motion(&mut self, desktop: &Desktop, dx: f64, dy: f64, now: Instant) -> Next {
        let target = Pos::new(self.pos.x + dx, self.pos.y + dy);
        if depth(&self.edge, target) > AWAY {
            return self.release(desktop.clamp(target), false, Vec::new());
        }
        // Slide along the edge, staying on it, as far as the desktop goes.
        // Past an end of the armed piece nothing faces the edge (or the
        // desktop goes on round a corner): the pointer is handed back there.
        let slid = desktop.clamp(onto(&self.edge, target));
        if !on(&self.edge, slid) {
            return self.release(slid, false, Vec::new());
        }
        self.pos = slid;
        self.check(desktop, now)
    }

    /// The backend reports which modifiers are held.
    pub fn modifiers(&mut self, desktop: &Desktop, mods: Mods, now: Instant) -> Next {
        self.mods = mods;
        self.check(desktop, now)
    }

    /// A captured key while held. The required modifier is taken as the
    /// go-ahead; any other key means the user is doing something else
    /// here, so the pointer is handed back and the key is typed on this
    /// computer, so it isn't lost: a key-down as a tap (its key-up will
    /// come from the keyboard, past the injector), a key-up as it is (this
    /// computer saw the key go down before the capture). A modifier going
    /// down is dropped: pressing and releasing it at once means nothing,
    /// or worse.
    pub fn key(&mut self, desktop: &Desktop, code: u32, down: bool, now: Instant) -> Next {
        let wanted = self.guard.modifier.keys().contains(&code);
        if wanted {
            self.mods.update(code, down);
            self.swallowed.update(code, down);
            return self.check(desktop, now);
        }
        let pos = self.pos;
        match (down, Mods::is_modifier(code)) {
            (false, _) => self.release(pos, false, vec![(code, false)]),
            (true, true) => self.release(pos, false, Vec::new()),
            (true, false) => self.release(pos, true, vec![(code, true), (code, false)]),
        }
    }

    /// A captured click or scroll while held: hand the pointer back (the
    /// caller replays it on this computer, inside `around`).
    pub fn other_input(&mut self) -> Next {
        let pos = self.pos;
        self.release(pos, true, Vec::new())
    }
}

/// `one-mk switch next`: the computer after `current` (`None`: this one)
/// in `online`, then this one again, round and round.
pub fn cycle_next<T: PartialEq + Copy>(current: Option<T>, online: &[T]) -> Option<T> {
    match current.and_then(|c| online.iter().position(|o| *o == c)) {
        None => online.first().copied(),
        Some(i) => online.get(i + 1).copied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    fn desk() -> Desktop {
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080)])
    }

    /// 1920x1080 with a 1280x1024 monitor to its right, tops aligned.
    fn dual() -> Desktop {
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 1024)])
    }

    /// The whole `side` edge of [`desk`].
    fn whole(side: Side) -> Edge {
        Edge::of(side, &Rect::new(0, 0, 1920, 1080))
    }

    fn at(ms: u64, t0: Instant) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn wait(s: Start) -> Approach {
        match s {
            Start::Wait(a) => a,
            other => panic!("expected to wait, got {other:?}"),
        }
    }

    #[test]
    fn no_guards_cross_at_once() {
        let t0 = Instant::now();
        let p = Pos::new(1919.0, 500.0);
        assert_eq!(
            Approach::start(EdgeGuard::default(), &desk(), whole(Side::Right), p, Mods::default(), false, t0),
            Start::Cross(p)
        );
    }

    #[test]
    fn dead_corners() {
        let d = dual();
        let right = Edge::of(Side::Right, &Rect::new(1920, 0, 1280, 1024));
        let top = Edge { side: Side::Top, at: 0, lo: 0, hi: 3200 };
        let corner = |e: &Edge, x, y| in_dead_corner(&d, e, Pos::new(x, y), 10);
        // Right edge of the right monitor: its own top and bottom corners.
        assert!(corner(&right, 3199.0, 0.0));
        assert!(corner(&right, 3199.0, 9.0));
        assert!(!corner(&right, 3199.0, 10.0));
        assert!(!corner(&right, 3199.0, 1013.0));
        assert!(corner(&right, 3199.0, 1014.0));
        assert!(corner(&right, 3199.0, 1023.0));
        // Top edge: the inner corners where the two monitors meet count.
        assert!(corner(&top, 1915.0, 0.0));
        assert!(corner(&top, 1925.0, 0.0));
        assert!(!corner(&top, 1900.0, 0.0));
        // 0 turns it off.
        assert!(!in_dead_corner(&d, &right, Pos::new(3199.0, 0.0), 0));

        let g = EdgeGuard { corner: 10, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let start = |y| Approach::start(g, &d, right, Pos::new(3199.0, y), Mods::default(), false, t0);
        assert_eq!(start(3.0), Start::Refuse(Refusal::Corner));
        assert_eq!(start(500.0), Start::Cross(Pos::new(3199.0, 500.0)));
    }

    #[test]
    fn dwell_crosses_after_the_time_unless_moved_away() {
        let d = desk();
        let g = EdgeGuard { dwell_ms: 300, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let mut a = wait(Approach::start(g, &d, whole(Side::Right), Pos::new(1919.0, 500.0), Mods::default(), false, t0));
        assert_eq!(a.deadline(), Some(at(300, t0)));
        // Pushing and sliding along the edge keeps it waiting.
        assert_eq!(a.motion(&d, 30.0, 20.0, at(100, t0)), Next::Wait);
        assert_eq!(a.pos(), Pos::new(1919.0, 520.0));
        assert_eq!(a.motion(&d, -2.0, 0.0, at(150, t0)), Next::Wait, "jitter is not moving away");
        assert_eq!(a.timer(&d, at(299, t0)), Next::Wait);
        assert_eq!(a.timer(&d, at(300, t0)), Next::Cross(Pos::new(1919.0, 520.0)));
        assert_eq!(a.deadline(), None, "no timer once the dwell has passed");

        // Moving away first hands the pointer back where it would be.
        let mut a = wait(Approach::start(g, &d, whole(Side::Right), Pos::new(1919.0, 500.0), Mods::default(), false, t0));
        let Next::Release { pos, around, keys } = a.motion(&d, -30.0, 5.0, at(50, t0)) else { panic!() };
        assert_eq!(pos, Pos::new(1889.0, 505.0));
        assert!(around.is_empty() && keys.is_empty());
    }

    #[test]
    fn dwell_on_every_side() {
        let d = desk();
        let g = EdgeGuard { dwell_ms: 50, ..EdgeGuard::default() };
        let t0 = Instant::now();
        for (side, p, away) in [
            (Side::Left, Pos::new(0.0, 300.0), (10.0, 0.0)),
            (Side::Right, Pos::new(1919.0, 300.0), (-10.0, 0.0)),
            (Side::Top, Pos::new(800.0, 0.0), (0.0, 10.0)),
            (Side::Bottom, Pos::new(800.0, 1079.0), (0.0, -10.0)),
        ] {
            let mut a = wait(Approach::start(g, &d, whole(side), p, Mods::default(), false, t0));
            // Pushing outwards hard stays on the edge.
            assert_eq!(a.motion(&d, -away.0 * 50.0, -away.1 * 50.0, at(10, t0)), Next::Wait, "{side}");
            assert_eq!(a.pos(), p, "{side}");
            assert!(matches!(a.clone().motion(&d, away.0, away.1, at(20, t0)), Next::Release { .. }), "{side}");
            assert_eq!(a.timer(&d, at(50, t0)), Next::Cross(p), "{side}");
        }
    }

    #[test]
    fn sliding_into_a_corner_waits_until_out_of_it() {
        let d = desk();
        let g = EdgeGuard { dwell_ms: 100, corner: 20, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let mut a = wait(Approach::start(g, &d, whole(Side::Right), Pos::new(1919.0, 40.0), Mods::default(), false, t0));
        assert_eq!(a.motion(&d, 0.0, -30.0, at(50, t0)), Next::Wait);
        assert_eq!(a.timer(&d, at(100, t0)), Next::Wait, "in the corner when the dwell ends");
        assert_eq!(a.deadline(), None);
        assert_eq!(a.motion(&d, 0.0, 15.0, at(150, t0)), Next::Cross(Pos::new(1919.0, 25.0)));
    }

    #[test]
    fn modifier_to_cross() {
        let d = desk();
        let g = EdgeGuard { modifier: Modifier::Shift, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let p = Pos::new(0.0, 500.0);
        // Already held (Windows and macOS report it at activation).
        let shift = Mods::from_keys([54]);
        assert_eq!(Approach::start(g, &d, whole(Side::Left), p, shift, false, t0), Start::Cross(p));
        // Ctrl is not Shift: held at the edge, with no timer.
        let mut a = wait(Approach::start(g, &d, whole(Side::Left), p, Mods::from_keys([29]), false, t0));
        assert_eq!(a.deadline(), None);
        // Pressing Shift while pushing goes across.
        assert_eq!(a.key(&d, 42, true, at(500, t0)), Next::Cross(p));

        // The backend's modifier report works too.
        let mut a = wait(Approach::start(g, &d, whole(Side::Left), p, Mods::default(), false, t0));
        assert_eq!(a.modifiers(&d, Mods::from_xkb(1), t0), Next::Cross(p));

        // Another key hands the pointer back and is typed on this computer.
        let g = EdgeGuard { modifier: Modifier::Alt, ..EdgeGuard::default() };
        let mut a = wait(Approach::start(g, &d, whole(Side::Left), p, Mods::default(), false, t0));
        assert_eq!(a.key(&d, 30, true, t0), Next::Release { pos: p, around: vec![], keys: vec![(30, true), (30, false)] });
        let g = EdgeGuard { modifier: Modifier::Alt, dwell_ms: 100, ..EdgeGuard::default() };
        let mut a = wait(Approach::start(g, &d, whole(Side::Left), p, Mods::default(), false, t0));
        assert_eq!(a.key(&d, 100, true, t0), Next::Wait, "Alt held, the dwell still runs");
        assert_eq!(a.clone().other_input(), Next::Release { pos: p, around: vec![100], keys: vec![] }, "Alt-click");
        assert_eq!(
            a.key(&d, 30, true, t0),
            Next::Release { pos: p, around: vec![100], keys: vec![(30, true), (30, false)] },
            "Alt+A"
        );
        let mut a = wait(Approach::start(g, &d, whole(Side::Left), p, Mods::default(), false, t0));
        a.key(&d, 56, true, t0);
        a.key(&d, 56, false, t0);
        assert_eq!(
            a.motion(&d, 20.0, 0.0, t0),
            Next::Release { pos: Pos::new(20.0, 500.0), around: vec![], keys: vec![] },
            "released again"
        );
    }

    /// Replaying what was captured while held never leaves a key down on
    /// this computer: every key pressed here is released in the same
    /// replay, since the user's own key-up won't come through the injector.
    #[test]
    fn handing_back_leaves_nothing_held() {
        let d = desk();
        let t0 = Instant::now();
        let p = Pos::new(1919.0, 500.0);
        // Every key the user may press at the edge, under every guard.
        let keys = [30u32, 1, 28, 42, 54, 29, 97, 56, 100, 125, 126];
        let guards = [
            EdgeGuard { dwell_ms: 300, ..EdgeGuard::default() },
            EdgeGuard { modifier: Modifier::Shift, dwell_ms: 300, ..EdgeGuard::default() },
            EdgeGuard { modifier: Modifier::Alt, dwell_ms: 300, ..EdgeGuard::default() },
        ];
        for g in guards {
            for &first in &keys {
                for &then in &keys {
                    for end in 0..3 {
                        let mut a = wait(Approach::start(g, &d, whole(Side::Right), p, Mods::default(), false, t0));
                        let mut next = a.key(&d, first, true, at(10, t0));
                        if next == Next::Wait {
                            next = match end {
                                0 => a.key(&d, then, true, at(20, t0)),
                                1 => a.other_input(),
                                _ => a.motion(&d, -50.0, 0.0, at(20, t0)),
                            };
                        }
                        if next == Next::Wait {
                            continue; // both were the guard's modifier
                        }
                        let Next::Release { around, keys, .. } = next else { panic!("{next:?}") };
                        // What the injector would hold, in order.
                        let mut held = Vec::new();
                        let events = around.iter().map(|&c| (c, true)).chain(keys).chain(around.iter().rev().map(|&c| (c, false)));
                        for (code, down) in events {
                            match down {
                                true => held.push(code),
                                false => held.retain(|&k| k != code),
                            }
                        }
                        assert!(held.is_empty(), "{g:?} {first} {then} {end}: {held:?} left down");
                    }
                }
            }
        }
        // A modifier alone is not tapped (a lone Alt opens the menu bar).
        let g = EdgeGuard { dwell_ms: 300, ..EdgeGuard::default() };
        let mut a = wait(Approach::start(g, &d, whole(Side::Right), p, Mods::default(), false, t0));
        assert_eq!(a.key(&d, 56, true, t0), Next::Release { pos: p, around: vec![], keys: vec![] });
        // A key-up is passed on: this computer saw the key go down.
        let mut a = wait(Approach::start(g, &d, whole(Side::Right), p, Mods::default(), false, t0));
        assert_eq!(a.key(&d, 30, false, t0), Next::Release { pos: p, around: vec![], keys: vec![(30, false)] });
    }

    #[test]
    fn dwell_and_modifier_both_needed() {
        let d = desk();
        let g = EdgeGuard { modifier: Modifier::Ctrl, dwell_ms: 200, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let p = Pos::new(800.0, 0.0);
        let mut a = wait(Approach::start(g, &d, whole(Side::Top), p, Mods::from_keys([97]), false, t0));
        assert_eq!(a.timer(&d, at(100, t0)), Next::Wait);
        assert_eq!(a.timer(&d, at(200, t0)), Next::Cross(p));
        let mut a = wait(Approach::start(g, &d, whole(Side::Top), p, Mods::default(), false, t0));
        assert_eq!(a.timer(&d, at(200, t0)), Next::Wait);
        assert_eq!(a.deadline(), None);
        assert_eq!(a.key(&d, 29, true, at(900, t0)), Next::Cross(p));
    }

    #[test]
    fn modifier_names() {
        assert_eq!("Shift".parse::<Modifier>(), Ok(Modifier::Shift));
        assert_eq!("control".parse::<Modifier>(), Ok(Modifier::Ctrl));
        assert_eq!("option".parse::<Modifier>(), Ok(Modifier::Alt));
        assert!("hyper".parse::<Modifier>().is_err());
        for m in Modifier::ALL {
            assert_eq!(m.to_string().parse::<Modifier>(), Ok(m));
        }
        let mut m = Mods::from_keys([42, 54]);
        m.update(42, false);
        assert!(m.has(Modifier::Shift), "the right Shift is still down");
        assert!(!m.update(30, true));
        assert_eq!(Mods::from_xkb(0b100_1101).keys().collect::<Vec<_>>(), vec![42, 29, 56, 125]);
    }

    #[test]
    fn fullscreen_guard() {
        let game = Focus { fullscreen: true, apps: vec!["C:\\Games\\Doom.exe".into()] };
        let video = Focus { fullscreen: true, apps: vec!["vlc".into(), "VLC media player".into()] };
        let editor = Focus { fullscreen: false, apps: vec!["code".into()] };
        let none: [String; 0] = [];
        let allow = ["VLC".to_owned()];
        let block = ["code".to_owned()];
        assert!(fullscreen_blocks(true, Some(&game), &none, &none));
        assert!(!fullscreen_blocks(false, Some(&game), &none, &none), "off");
        assert!(!fullscreen_blocks(true, None, &none, &none), "can't tell: never blocks");
        assert!(!fullscreen_blocks(true, Some(&video), &allow, &none), "allowed");
        assert!(fullscreen_blocks(true, Some(&video), &none, &none));
        assert!(!fullscreen_blocks(true, Some(&editor), &none, &none));
        assert!(fullscreen_blocks(true, Some(&editor), &none, &block), "blocked even windowed");
        assert!(app_matches("C:\\Games\\Doom.exe", "doom"));
        assert!(app_matches("/Applications/Safari.app", "safari"));
        assert!(app_matches("com.apple.Safari", "COM.APPLE.SAFARI"));
        assert!(!app_matches("doom", ""));
        assert!(!app_matches("doomed", "doom"));

        let t0 = Instant::now();
        let g = EdgeGuard { fullscreen: true, ..EdgeGuard::default() };
        let r = Approach::start(g, &desk(), whole(Side::Left), Pos::new(0.0, 5.0), Mods::default(), true, t0);
        assert_eq!(r, Start::Refuse(Refusal::Fullscreen));
    }

    /// 1920x1080 with a 1280x720 monitor to its right, tops aligned: the
    /// left monitor's right edge is exposed below the shorter one, inside
    /// the bounding box.
    fn short_neighbour() -> (Desktop, Edge) {
        let d = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1280, 720)]);
        let e = Edge { side: Side::Right, at: 1920, lo: 720, hi: 1080 };
        assert_eq!(d.exposed_edges(Side::Right)[0], e);
        (d, e)
    }

    #[test]
    fn dwell_at_an_inner_edge() {
        let (d, e) = short_neighbour();
        let g = EdgeGuard { dwell_ms: 500, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let p = Pos::new(1919.0, 900.0);
        let mut a = wait(Approach::start(g, &d, e, p, Mods::default(), false, t0));
        // Outwards is pushing against the edge, not retreating.
        assert_eq!(a.motion(&d, 1.0, 0.0, at(10, t0)), Next::Wait);
        assert_eq!(a.motion(&d, 300.0, 0.0, at(20, t0)), Next::Wait);
        assert_eq!(a.pos(), p);
        assert_eq!(a.motion(&d, -2.0, 0.0, at(30, t0)), Next::Wait, "jitter");
        // Sliding along it.
        assert_eq!(a.motion(&d, 40.0, 100.0, at(40, t0)), Next::Wait);
        assert_eq!(a.pos(), Pos::new(1919.0, 1000.0));
        // Down into the desktop's bottom corner it stays on the edge.
        assert_eq!(a.motion(&d, 0.0, 500.0, at(50, t0)), Next::Wait);
        assert!(a.pos().x == 1919.0 && a.pos().y > 1079.0 && a.pos().y < 1080.0, "{:?}", a.pos());
        assert!(matches!(a.timer(&d, at(500, t0)), Next::Cross(_)));

        // Inwards, past a few px, hands it back.
        let mut a = wait(Approach::start(g, &d, e, p, Mods::default(), false, t0));
        assert_eq!(a.motion(&d, -3.0, 0.0, at(10, t0)), Next::Wait);
        assert_eq!(a.motion(&d, -10.0, 2.0, at(20, t0)), Next::Release { pos: Pos::new(1909.0, 902.0), around: vec![], keys: vec![] });

        // Up along the edge past its end (nothing faces it there, the right
        // monitor begins): handed back there, never onto the other monitor.
        let mut a = wait(Approach::start(g, &d, e, p, Mods::default(), false, t0));
        assert_eq!(a.motion(&d, 0.0, -170.0, at(10, t0)), Next::Wait);
        assert_eq!(a.motion(&d, 50.0, -15.0, at(20, t0)), Next::Release { pos: Pos::new(1919.0, 715.0), around: vec![], keys: vec![] });
    }

    #[test]
    fn modifier_at_an_inner_edge() {
        let (d, e) = short_neighbour();
        let g = EdgeGuard { modifier: Modifier::Shift, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let p = Pos::new(1919.0, 900.0);
        let mut a = wait(Approach::start(g, &d, e, p, Mods::default(), false, t0));
        assert_eq!(a.motion(&d, 25.0, 0.0, t0), Next::Wait, "pushing outwards");
        assert_eq!(a.motion(&d, 5.0, 30.0, t0), Next::Wait, "sliding");
        assert_eq!(a.key(&d, 42, true, t0), Next::Cross(Pos::new(1919.0, 930.0)));
        let mut a = wait(Approach::start(g, &d, e, p, Mods::default(), false, t0));
        assert!(matches!(a.motion(&d, -20.0, 0.0, t0), Next::Release { .. }));
    }

    #[test]
    fn dead_corners_at_an_inner_edge() {
        let (d, e) = short_neighbour();
        let g = EdgeGuard { corner: 10, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let start = |y| Approach::start(g, &d, e, Pos::new(1919.0, y), Mods::default(), false, t0);
        // Where the edge begins below the right monitor, and the desktop's
        // own bottom corner; not the left monitor's covered corner at y=0.
        assert_eq!(start(725.0), Start::Refuse(Refusal::Corner));
        assert_eq!(start(1075.0), Start::Refuse(Refusal::Corner));
        assert_eq!(start(735.0), Start::Cross(Pos::new(1919.0, 735.0)));
        assert_eq!(start(1065.0), Start::Cross(Pos::new(1919.0, 1065.0)));
        // Reported a pixel off the armed piece: put on it.
        assert_eq!(start(1078.0), Start::Refuse(Refusal::Corner));

        // Sliding into the corner where the edge begins waits there.
        let g = EdgeGuard { corner: 10, dwell_ms: 100, ..EdgeGuard::default() };
        let mut a = wait(Approach::start(g, &d, e, Pos::new(1919.0, 760.0), Mods::default(), false, t0));
        assert_eq!(a.motion(&d, 0.0, -35.0, at(10, t0)), Next::Wait);
        assert_eq!(a.timer(&d, at(100, t0)), Next::Wait, "in the corner");
        assert_eq!(a.motion(&d, 0.0, 10.0, at(110, t0)), Next::Cross(Pos::new(1919.0, 735.0)));
    }

    /// The edge of a monitor whose neighbour is taller: the top of a
    /// 1920x1080 monitor beside a portrait one reaching 420 px higher.
    #[test]
    fn edge_beside_a_taller_monitor() {
        let d = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, -420, 1080, 1920)]);
        let e = Edge { side: Side::Top, at: 0, lo: 0, hi: 1920 };
        assert!(d.exposed_edges(Side::Top).contains(&e));
        let g = EdgeGuard { dwell_ms: 200, corner: 10, ..EdgeGuard::default() };
        let t0 = Instant::now();
        let p = Pos::new(900.0, 0.0);
        let mut a = wait(Approach::start(g, &d, e, p, Mods::default(), false, t0));
        assert_eq!(a.motion(&d, 0.0, -50.0, at(10, t0)), Next::Wait, "outwards");
        assert_eq!(a.motion(&d, 30.0, 2.0, at(20, t0)), Next::Wait, "sliding");
        assert_eq!(a.pos(), Pos::new(930.0, 0.0));
        assert!(matches!(a.clone().motion(&d, 0.0, 10.0, at(30, t0)), Next::Release { .. }), "inwards");
        assert_eq!(a.timer(&d, at(200, t0)), Next::Cross(Pos::new(930.0, 0.0)));
        // The corner where the taller monitor begins is a corner.
        assert!(in_dead_corner(&d, &e, Pos::new(1915.0, 0.0), 10));
        assert!(!in_dead_corner(&d, &e, Pos::new(1900.0, 0.0), 10));
        // Sliding along it into the taller monitor's side: handed back.
        assert!(matches!(a.clone().motion(&d, 1200.0, 0.0, at(30, t0)), Next::Release { .. }));
    }

    /// An L: a monitor on top of another, a third right of the lower one.
    /// The top monitor's right edge and the third's top edge are inside
    /// the bounding box and meet at the inner corner (1920, 1080).
    #[test]
    fn l_shaped_desktop() {
        let d = Desktop::new(vec![
            Rect::new(0, 0, 1920, 1080),
            Rect::new(0, 1080, 1920, 1080),
            Rect::new(1920, 1080, 1920, 1080),
        ]);
        let right = Edge { side: Side::Right, at: 1920, lo: 0, hi: 1080 };
        let top = Edge { side: Side::Top, at: 1080, lo: 1920, hi: 3840 };
        assert!(d.exposed_edges(Side::Right).contains(&right));
        assert!(d.exposed_edges(Side::Top).contains(&top));
        let g = EdgeGuard { dwell_ms: 100, corner: 10, ..EdgeGuard::default() };
        let t0 = Instant::now();

        let mut a = wait(Approach::start(g, &d, top, Pos::new(2500.0, 1080.0), Mods::default(), false, t0));
        assert_eq!(a.motion(&d, 0.0, -3.0, at(10, t0)), Next::Wait, "outwards");
        assert_eq!(a.motion(&d, -400.0, -1.0, at(20, t0)), Next::Wait, "along");
        assert_eq!(a.pos(), Pos::new(2100.0, 1080.0));
        assert!(matches!(a.clone().motion(&d, 0.0, 10.0, at(30, t0)), Next::Release { .. }));
        assert_eq!(a.timer(&d, at(100, t0)), Next::Cross(Pos::new(2100.0, 1080.0)));
        let start = |e, x, y| Approach::start(g, &d, e, Pos::new(x, y), Mods::default(), false, t0);
        assert_eq!(start(top, 1925.0, 1080.0), Start::Refuse(Refusal::Corner), "the inner corner");
        assert_eq!(start(right, 1919.0, 1075.0), Start::Refuse(Refusal::Corner), "the inner corner");

        let mut a = wait(start(right, 1919.0, 500.0));
        assert_eq!(a.motion(&d, 2.0, 0.0, at(10, t0)), Next::Wait, "outwards");
        // Down past the corner, onto the lower monitor: handed back there.
        assert_eq!(a.motion(&d, 0.0, 700.0, at(20, t0)), Next::Release { pos: Pos::new(1919.0, 1200.0), around: vec![], keys: vec![] });
    }

    #[test]
    fn switch_next_cycles_through_here() {
        let online = [3, 5, 9];
        assert_eq!(cycle_next(None, &online), Some(3));
        assert_eq!(cycle_next(Some(3), &online), Some(5));
        assert_eq!(cycle_next(Some(9), &online), None, "back here");
        assert_eq!(cycle_next(Some(4), &online), Some(3), "no longer online");
        assert_eq!(cycle_next::<u8>(None, &[]), None);
    }
}
