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

    /// The peer's monitors changed while we control it.
    pub fn set_desktop(&mut self, desktop: Desktop) {
        self.pos = desktop.clamp(self.pos);
        self.desktop = desktop;
    }

    pub fn motion(&mut self, dx: f64, dy: f64) -> Step {
        let target = Pos::new(self.pos.x + dx, self.pos.y + dy);
        if self.desktop.screen_at(target).is_none() && self.crosses_home(target) {
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

    fn crosses_home(&self, target: Pos) -> bool {
        let r = self.desktop.screen_at(self.pos).copied().unwrap_or_else(|| self.desktop.bounds());
        match self.home {
            Side::Left => target.x < f64::from(r.x),
            Side::Right => target.x >= f64::from(r.right()),
            Side::Top => target.y < f64::from(r.y),
            Side::Bottom => target.y >= f64::from(r.bottom()),
        }
    }
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

    /// Returns the position to apply, or `None` if the datagram is stale.
    pub fn accept(&mut self, epoch: u16, seq: u32, pos: Pos) -> Option<Pos> {
        if self.epoch != Some(epoch) || !newer(seq, self.last_seq) {
            return None;
        }
        self.last_seq = seq;
        Some(pos)
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
        assert_eq!(r.accept(1, 1, p), None); // before Enter
        r.enter(1);
        assert_eq!(r.accept(1, 2, p), Some(p));
        assert_eq!(r.accept(1, 1, p), None); // reordered
        assert_eq!(r.accept(0, 3, p), None); // previous visit
        r.sync(10);
        assert_eq!(r.accept(1, 9, p), None);
        assert_eq!(r.accept(1, 11, p), Some(p));
    }

    #[test]
    fn seq_wraps() {
        let mut r = MotionReceiver::default();
        r.enter(0);
        for s in [0x4000_0000, 0x8000_0000, 0xC000_0000, u32::MAX - 1] {
            r.sync(s);
        }
        assert!(r.accept(0, u32::MAX, Pos::default()).is_some());
        assert!(r.accept(0, 0, Pos::default()).is_some());
        assert!(r.accept(0, u32::MAX, Pos::default()).is_none());
    }
}
