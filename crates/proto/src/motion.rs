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
#[derive(Clone, Debug)]
pub struct RemoteCursor {
    desktop: Desktop,
    home: Side,
    pos: Pos,
    epoch: u16,
    seq: u32,
}

impl RemoteCursor {
    /// Enters `desktop` through its `home` edge (the side facing us) at
    /// fraction `t` along that edge.
    pub fn enter(desktop: Desktop, home: Side, t: f64, epoch: u16) -> Self {
        let pos = desktop.point_on_edge(home, t, 0.0);
        Self { desktop, home, pos, epoch, seq: 0 }
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

    /// The latest position as a datagram; resending it is always safe.
    pub fn current(&self) -> Datagram {
        Datagram::Motion { epoch: self.epoch, seq: self.seq, pos: self.pos }
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

/// The receiver's filter for motion datagrams: drops packets from a previous
/// visit (epoch) and packets that arrive out of order.
#[derive(Clone, Debug, Default)]
pub struct MotionReceiver {
    epoch: Option<u16>,
    last_seq: u32,
}

impl MotionReceiver {
    pub fn enter(&mut self, epoch: u16) {
        self.epoch = Some(epoch);
        self.last_seq = 0;
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

    /// A reliable event (click) carried the sender's position at `seq`;
    /// older datagrams still in flight must not move the pointer back.
    pub fn sync(&mut self, seq: u32) {
        if newer(seq, self.last_seq) {
            self.last_seq = seq;
        }
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
        r.sync(10);
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
