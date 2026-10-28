use crate::{Datagram, Desktop, Pos, Side};

/// Result of applying a local pointer delta while controlling a peer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// The pointer moved; send this datagram.
    Move(Datagram),
    /// The pointer was pushed past the edge that leads back home, at
    /// fraction `t` along that edge.
    Leave { t: f64 },
    /// Pinned against an edge; nothing to send.
    Still,
}

/// The sender's model of the pointer on a remote desktop.
///
/// Motion is sent as absolute positions, so every datagram is
/// self-contained: a lost packet is simply superseded by the next one,
/// nothing is ever retransmitted, and the pointer cannot drift.
/// It also means mouse acceleration is applied exactly once (by the machine
/// the mouse is plugged into) and DPI scaling never distorts speed.
///
/// In relative mode (gaming) the deltas themselves are sent, as a running
/// sum ([`Datagram::MotionBy`]), and the pointer never leaves through an
/// edge: games capture the pointer and want raw movement, not a position
/// pinned at the screen border. `pos` is then only an estimate (the peer's
/// pointer clamped to its screens), used if absolute mode resumes.
#[derive(Clone, Debug)]
pub struct RemoteCursor {
    desktop: Desktop,
    home: Side,
    pos: Pos,
    epoch: u16,
    seq: u32,
    relative: bool,
    /// Every delta sent in relative mode during this visit, in pixels.
    sum: (f64, f64),
}

impl RemoteCursor {
    /// Enters `desktop` through its `home` edge (the side facing us) at
    /// fraction `t` along that edge.
    pub fn enter(desktop: Desktop, home: Side, t: f64, epoch: u16) -> Self {
        let pos = desktop.point_on_edge(home, t, 0.0);
        Self::enter_at(desktop, home, pos, epoch)
    }

    /// Enters `desktop` at `pos` without crossing an edge (`one-mk switch`);
    /// `home` is still the edge that leads back.
    pub fn enter_at(desktop: Desktop, home: Side, pos: Pos, epoch: u16) -> Self {
        let pos = desktop.clamp(pos);
        Self { desktop, home, pos, epoch, seq: 0, relative: false, sum: (0.0, 0.0) }
    }

    pub fn is_relative(&self) -> bool {
        self.relative
    }

    /// Switches between absolute and relative motion. Returns true if that
    /// changed anything; the peer must then be told
    /// ([`Control::PointerMode`](crate::Control::PointerMode)) before the
    /// next datagram. A new `seq` makes the next datagram (of the new kind)
    /// supersede every one of the old kind still in flight.
    pub fn set_relative(&mut self, on: bool) -> bool {
        if self.relative == on {
            return false;
        }
        self.relative = on;
        self.seq = self.seq.wrapping_add(1);
        true
    }

    pub fn pos(&self) -> Pos {
        self.pos
    }
    pub fn epoch(&self) -> u16 {
        self.epoch
    }
    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// Where the pointer is along the home edge, in [0, 1]; used to bring
    /// it back to the matching spot on the sender's edge.
    pub fn home_fraction(&self) -> f64 {
        self.desktop.edge_fraction(self.home, self.pos)
    }

    /// The latest position (or relative sum) as a datagram; resending it
    /// is always safe.
    pub fn current(&self) -> Datagram {
        if self.relative {
            let (sum_x, sum_y) = (wrapping_fixed(self.sum.0), wrapping_fixed(self.sum.1));
            Datagram::MotionBy { epoch: self.epoch, seq: self.seq, sum_x, sum_y }
        } else {
            Datagram::Motion { epoch: self.epoch, seq: self.seq, pos: self.pos }
        }
    }

    /// The peer's monitors changed while we control it. Returns the new
    /// position, under a new `seq` so the receiver applies it; send it.
    pub fn set_desktop(&mut self, desktop: Desktop) -> Datagram {
        self.pos = desktop.clamp(self.pos);
        self.desktop = desktop;
        self.seq = self.seq.wrapping_add(1);
        self.current()
    }

    pub fn motion(&mut self, dx: f64, dy: f64) -> Step {
        let target = Pos::new(self.pos.x + dx, self.pos.y + dy);
        if self.relative {
            if dx == 0.0 && dy == 0.0 {
                return Step::Still;
            }
            self.sum = (self.sum.0 + dx, self.sum.1 + dy);
            self.pos = self.desktop.clamp(target);
            self.seq = self.seq.wrapping_add(1);
            return Step::Move(self.current());
        }
        if self.crosses_home(target) {
            return Step::Leave { t: self.desktop.edge_fraction(self.home, target) };
        }
        let next = self.desktop.clamp(target);
        if next == self.pos {
            return Step::Still;
        }
        self.pos = next;
        self.seq = self.seq.wrapping_add(1);
        Step::Move(self.current())
    }

    /// Only pushing past the outer edge of the whole desktop leads home; an
    /// inner edge between two monitors (or a gap beside a smaller one)
    /// just clamps.
    fn crosses_home(&self, target: Pos) -> bool {
        let r = self.desktop.bounds();
        match self.home {
            Side::Left => target.x < f64::from(r.x),
            Side::Right => target.x >= f64::from(r.right()),
            Side::Top => target.y < f64::from(r.y),
            Side::Bottom => target.y >= f64::from(r.bottom()),
        }
    }
}

/// A running sum in 1/256 px, reduced to `i32` (wrapping). Differences of
/// two such values are exact as long as they are less than 2^31 / 256 px
/// apart, which consecutive datagrams always are.
fn wrapping_fixed(v: f64) -> i32 {
    ((v * 256.0).round() as i64) as i32
}

/// What to do with a received motion datagram.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Received {
    /// A newer position: move the pointer there.
    Move(Pos),
    /// The current position again (an idle resend): nothing to move, but
    /// it shows the sender is alive.
    Repeat,
    /// From a previous visit or overtaken by a newer one; ignore it.
    Stale,
}

/// What to do with a received relative-motion datagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelReceived {
    /// Move the pointer by this many whole pixels (possibly zero: the sum
    /// moved by less than half a pixel).
    MoveBy { dx: i32, dy: i32 },
    Repeat,
    Stale,
}

/// The receiver's filter for motion datagrams: drops packets from a previous
/// visit (epoch) and packets that arrive out of order.
#[derive(Clone, Debug, Default)]
pub struct MotionReceiver {
    epoch: Option<u16>,
    last_seq: u32,
    rel: RelativeSum,
}

/// Turns the sender's running sums into whole-pixel moves. What has been
/// moved is always the rounded sum, so rounding never accumulates into
/// drift, and lost datagrams are made up by the next one.
#[derive(Clone, Debug, Default)]
struct RelativeSum {
    /// The last wire sum applied (wrapping 1/256 px).
    last: (i32, i32),
    /// The same, unwrapped.
    total: (i64, i64),
    /// Whole pixels moved so far.
    moved: (i64, i64),
}

impl RelativeSum {
    fn apply(&mut self, sum: (i32, i32)) -> (i32, i32) {
        let step = |last: &mut i32, total: &mut i64, moved: &mut i64, now: i32| {
            *total += i64::from(now.wrapping_sub(*last));
            *last = now;
            let target = (*total + 128).div_euclid(256);
            let d = target - *moved;
            *moved = target;
            i32::try_from(d).unwrap_or(if d < 0 { i32::MIN } else { i32::MAX })
        };
        let dx = step(&mut self.last.0, &mut self.total.0, &mut self.moved.0, sum.0);
        let dy = step(&mut self.last.1, &mut self.total.1, &mut self.moved.1, sum.1);
        (dx, dy)
    }
}

impl MotionReceiver {
    pub fn enter(&mut self, epoch: u16) {
        self.epoch = Some(epoch);
        self.last_seq = 0;
        self.rel = RelativeSum::default();
    }

    pub fn leave(&mut self) {
        self.epoch = None;
    }

    pub fn is_active(&self) -> bool {
        self.epoch.is_some()
    }

    pub fn accept(&mut self, epoch: u16, seq: u32, pos: Pos) -> Received {
        if self.epoch != Some(epoch) {
            Received::Stale
        } else if newer(seq, self.last_seq) {
            self.last_seq = seq;
            Received::Move(pos)
        } else if seq == self.last_seq {
            Received::Repeat
        } else {
            Received::Stale
        }
    }

    /// A relative-motion datagram: the sender's running sums at `seq`.
    pub fn accept_relative(&mut self, epoch: u16, seq: u32, sum: (i32, i32)) -> RelReceived {
        if self.epoch != Some(epoch) {
            RelReceived::Stale
        } else if newer(seq, self.last_seq) {
            self.last_seq = seq;
            let (dx, dy) = self.rel.apply(sum);
            RelReceived::MoveBy { dx, dy }
        } else if seq == self.last_seq {
            RelReceived::Repeat
        } else {
            RelReceived::Stale
        }
    }

    /// A reliable event (click) carried the sender's position at `seq`;
    /// older datagrams still in flight must not move the pointer back.
    ///
    /// Returns false if the click is late: a newer position (a higher
    /// `seq`) was already applied. The click then belongs at its own
    /// position, after which the pointer must go back to the newer one.
    pub fn sync(&mut self, seq: u32) -> bool {
        if newer(seq, self.last_seq) {
            self.last_seq = seq;
        }
        !newer(self.last_seq, seq)
    }
}

fn newer(seq: u32, last: u32) -> bool {
    (seq.wrapping_sub(last) as i32) > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    fn desk() -> Desktop {
        Desktop::new(vec![Rect::new(0, 0, 1920, 1080)])
    }

    #[test]
    fn enter_move_and_leave() {
        // Remote is to our right, so its left edge leads home.
        let mut c = RemoteCursor::enter(desk(), Side::Left, 0.5, 7);
        assert_eq!(c.pos(), Pos::new(0.0, 540.0));
        let Step::Move(Datagram::Motion { epoch, seq, pos }) = c.motion(10.0, -5.0) else {
            panic!()
        };
        assert_eq!((epoch, seq, pos), (7, 1, Pos::new(10.0, 535.0)));
        // Pushing against the far edge just pins the pointer.
        c.motion(5000.0, 0.0);
        assert_eq!(c.motion(5.0, 0.0), Step::Still);
        // Going all the way back leaves through the home edge.
        assert_eq!(c.motion(-5000.0, 0.0), Step::Leave { t: 535.0 / 1080.0 });
    }

    #[test]
    fn receiver_drops_stale_and_reordered() {
        let mut r = MotionReceiver::default();
        let p = Pos::new(1.0, 1.0);
        assert_eq!(r.accept(1, 1, p), Received::Stale); // before Enter
        r.enter(1);
        assert_eq!(r.accept(1, 0, p), Received::Repeat); // resent Enter position
        assert_eq!(r.accept(1, 2, p), Received::Move(p));
        assert_eq!(r.accept(1, 2, p), Received::Repeat); // idle resend
        assert_eq!(r.accept(1, 1, p), Received::Stale); // reordered
        assert_eq!(r.accept(0, 3, p), Received::Stale); // previous visit
        assert!(r.sync(10));
        assert!(r.sync(10), "same seq is not late");
        assert!(!r.sync(4), "older than the latest position");
        assert_eq!(r.accept(1, 9, p), Received::Stale);
        assert_eq!(r.accept(1, 10, p), Received::Repeat);
        assert_eq!(r.accept(1, 11, p), Received::Move(p));
    }

    #[test]
    fn seq_wraps() {
        let mut r = MotionReceiver::default();
        r.enter(0);
        for s in [0x4000_0000, 0x8000_0000, 0xC000_0000, u32::MAX - 1] {
            r.sync(s);
        }
        let moved = |r: Received| matches!(r, Received::Move(_));
        assert!(moved(r.accept(0, u32::MAX, Pos::default())));
        assert!(moved(r.accept(0, 0, Pos::default())));
        assert_eq!(r.accept(0, u32::MAX, Pos::default()), Received::Stale);
    }

    #[test]
    fn leaves_only_through_the_outer_home_edge() {
        // A 1080p monitor and a taller 1440p one to its right; home is left.
        let d = Desktop::new(vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 2560, 1440)]);
        let mut c = RemoteCursor::enter(d, Side::Left, 0.5, 1);
        let y = 1440.0 - 1.0 / 256.0;
        assert!(matches!(c.motion(1980.0, 800.0), Step::Move(_)));
        assert_eq!(c.pos(), Pos::new(1980.0, y));
        // Moving left from the bottom of the tall monitor, below the short
        // one, clamps against the inner edge instead of leaving.
        assert!(matches!(c.motion(-100.0, 0.0), Step::Move(_)), "left from the middle of the desktop");
        assert_eq!(c.pos(), Pos::new(1920.0, y));
        assert_eq!(c.motion(-10.0, 0.0), Step::Still);
        // Only past the desktop's outer left edge does it leave.
        assert!(matches!(c.motion(-1921.0, 0.0), Step::Leave { .. }));
    }

    /// Sends `deltas` through a cursor in relative mode, delivering only the
    /// datagrams `deliver` picks, and returns the whole pixels the receiver
    /// moved in all.
    fn relay(deltas: &[(f64, f64)], mut deliver: impl FnMut(usize) -> bool) -> (i64, i64) {
        let mut c = RemoteCursor::enter(desk(), Side::Left, 0.5, 2);
        let mut r = MotionReceiver::default();
        r.enter(2);
        assert!(c.set_relative(true));
        let mut moved = (0i64, 0i64);
        let mut take = |r: &mut MotionReceiver, d: Datagram| {
            let Datagram::MotionBy { epoch, seq, sum_x, sum_y } = d else { panic!("{d:?}") };
            if let RelReceived::MoveBy { dx, dy } = r.accept_relative(epoch, seq, (sum_x, sum_y)) {
                moved = (moved.0 + i64::from(dx), moved.1 + i64::from(dy));
            }
        };
        for (i, &(dx, dy)) in deltas.iter().enumerate() {
            match c.motion(dx, dy) {
                Step::Move(d) if deliver(i) => take(&mut r, d),
                Step::Move(_) | Step::Still => {}
                Step::Leave { .. } => panic!("relative motion never leaves"),
            }
        }
        // The idle resend of the last sum always gets through eventually.
        take(&mut r, c.current());
        moved
    }

    #[test]
    fn relative_motion_rounds_without_drift() {
        // 1000 moves of 0.3 px: each rounds to 0, the sum is 300 px.
        assert_eq!(relay(&[(0.3, -0.3); 1000], |_| true), (300, -300));
        // Back and forth by fractions ends exactly where it started.
        let wobble: Vec<_> = (0..999).map(|i| if i % 3 == 2 { (-1.4, 0.7) } else { (0.7, -0.35) }).collect();
        assert_eq!(relay(&wobble, |_| true), (0, 0));
        // Large moves pass straight through (and are not clamped at the
        // peer's screen edge: a game turning keeps turning).
        assert_eq!(relay(&[(5000.0, 0.0); 20], |_| true), (100_000, 0));
    }

    #[test]
    fn lost_relative_datagrams_are_made_up() {
        let deltas: Vec<_> = (0..500).map(|i| (f64::from(i % 7) - 2.6, 1.25)).collect();
        let exact = |v: f64| v.round() as i64;
        let (sx, sy) = deltas.iter().fold((0.0, 0.0), |a, d| (a.0 + d.0, a.1 + d.1));
        assert_eq!(relay(&deltas, |_| true), (exact(sx), exact(sy)));
        // Two thirds lost: the same total.
        assert_eq!(relay(&deltas, |i| i % 3 == 0), (exact(sx), exact(sy)));
    }

    #[test]
    fn relative_sum_wraps_without_a_jump() {
        // A sum that passes i32::MAX (in 1/256 px) after hours of turning.
        let mut s = RelativeSum::default();
        let start = i32::MAX - 1000;
        s.apply((start, 0));
        let (dx, _) = s.apply((start.wrapping_add(2560), 0)); // +10 px, wrapping
        assert_eq!(dx, 10);
        let (dx, _) = s.apply((start.wrapping_add(2560 - 512), 0)); // -2 px
        assert_eq!(dx, -2);
    }

    #[test]
    fn relative_repeats_and_stale_datagrams_move_nothing() {
        let mut r = MotionReceiver::default();
        r.enter(1);
        assert_eq!(r.accept_relative(0, 1, (256, 0)), RelReceived::Stale, "previous visit");
        assert_eq!(r.accept_relative(1, 1, (256, 0)), RelReceived::MoveBy { dx: 1, dy: 0 });
        assert_eq!(r.accept_relative(1, 1, (256, 0)), RelReceived::Repeat);
        assert_eq!(r.accept_relative(1, 3, (1024, 0)), RelReceived::MoveBy { dx: 3, dy: 0 });
        assert_eq!(r.accept_relative(1, 2, (512, 0)), RelReceived::Stale, "reordered");
        // An absolute datagram from before the switch is stale too.
        assert_eq!(r.accept(1, 2, Pos::new(5.0, 5.0)), Received::Stale);
    }

    #[test]
    fn switching_modes_mid_visit() {
        let mut c = RemoteCursor::enter(desk(), Side::Left, 0.5, 1);
        c.motion(100.0, 0.0);
        assert!(c.set_relative(true));
        assert!(!c.set_relative(true));
        assert!(matches!(c.current(), Datagram::MotionBy { sum_x: 0, sum_y: 0, .. }));
        // Pushing far past the home edge doesn't leave; the estimate clamps.
        let Step::Move(Datagram::MotionBy { sum_x, .. }) = c.motion(-500.0, 0.0) else { panic!() };
        assert_eq!(sum_x, -500 * 256);
        assert_eq!(c.pos(), Pos::new(0.0, 540.0));
        assert_eq!(c.motion(0.0, 0.0), Step::Still);
        // Back to absolute: positions resume from the estimate, newer seq.
        let seq = c.seq();
        assert!(c.set_relative(false));
        assert!(matches!(c.current(), Datagram::Motion { pos, seq: s, .. } if pos == Pos::new(0.0, 540.0) && s > seq));
        assert!(matches!(c.motion(-1.0, 0.0), Step::Leave { .. }));
    }

    #[test]
    fn enter_at_a_point() {
        let c = RemoteCursor::enter_at(desk(), Side::Left, Pos::new(960.0, 540.0), 3);
        assert_eq!(c.current(), Datagram::Motion { epoch: 3, seq: 0, pos: Pos::new(960.0, 540.0) });
        let c = RemoteCursor::enter_at(desk(), Side::Left, Pos::new(5000.0, 540.0), 3);
        assert!(desk().screen_at(c.pos()).is_some());
    }

    #[test]
    fn desktop_change_sends_a_new_position() {
        let mut c = RemoteCursor::enter(desk(), Side::Left, 0.5, 4);
        let mut r = MotionReceiver::default();
        r.enter(4);
        let Step::Move(Datagram::Motion { seq, pos, .. }) = c.motion(1000.0, 0.0) else { panic!() };
        assert_eq!(r.accept(4, seq, pos), Received::Move(pos));
        // The monitor shrinks under the pointer: the clamped position must
        // be accepted, not dropped as a repeat of the old seq.
        let Datagram::Motion { epoch, seq, pos } = c.set_desktop(Desktop::new(vec![Rect::new(0, 0, 800, 600)])) else {
            panic!()
        };
        assert_eq!(pos, Pos::new(800.0 - 1.0 / 256.0, 540.0));
        assert_eq!(r.accept(epoch, seq, pos), Received::Move(pos));
    }
}
