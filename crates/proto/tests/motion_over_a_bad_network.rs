//! The pointer's journey over a network that loses, repeats and reorders datagrams: the sender's
//! `RemoteCursor` and the receiver's `MotionReceiver` driven through a seeded fault model, in
//! absolute and relative (gaming) modes, across visits (epochs), mode switches and the
//! wrap-around of the 32-bit sequence number and of the relative running sum.

use one_mk_proto::{Datagram, Desktop, MotionReceiver, Pos, Received, Rect, RelReceived, RemoteCursor, Side, Step};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Loses, repeats and reorders what is sent through it.
struct Network {
    in_flight: Vec<Datagram>,
    loss: u64,
    duplicate: u64,
    window: usize,
}

impl Network {
    fn new(loss: u64, duplicate: u64, window: usize) -> Self {
        Self { in_flight: Vec::new(), loss, duplicate, window }
    }

    fn send(&mut self, rng: &mut Rng, datagram: Datagram) {
        if rng.chance(self.loss) {
            return;
        }
        self.in_flight.push(datagram);
        if rng.chance(self.duplicate) {
            self.in_flight.push(datagram);
        }
    }

    /// Delivers something, usually one of the oldest few: a datagram can be overtaken by up to `window`.
    fn deliver(&mut self, rng: &mut Rng) -> Option<Datagram> {
        if self.in_flight.is_empty() {
            return None;
        }
        let pick = rng.below(self.in_flight.len().min(self.window) as u64) as usize;
        Some(self.in_flight.remove(pick))
    }

    fn drain(&mut self, rng: &mut Rng) -> Vec<Datagram> {
        std::iter::from_fn(|| self.deliver(rng)).collect()
    }
}

fn desktop() -> Desktop {
    // Mixed monitors: a landscape one and a taller portrait one.
    Desktop::new(vec![Rect::new(0, 0, 2560, 1440), Rect::new(2560, -300, 1080, 1920)])
}

fn random_delta(rng: &mut Rng) -> (f64, f64) {
    // Mostly small moves, sometimes a fling; quarter-pixel precision like a high-resolution mouse.
    let scale = if rng.chance(5) { 400 } else { 12 };
    ((rng.below(2 * scale + 1) as f64 - scale as f64) / 4.0, (rng.below(2 * scale + 1) as f64 - scale as f64) / 4.0)
}

#[test]
fn absolute_motion_ends_exactly_where_the_sender_is_however_the_network_behaves() {
    for (seed, loss, duplicate, window) in
        [(1, 0, 0, 1), (2, 30, 20, 8), (3, 70, 50, 3), (4, 10, 90, 20), (5, 95, 0, 1)]
    {
        let mut rng = Rng(0xA11CE ^ seed);
        let mut sender = RemoteCursor::enter(desktop(), Side::Left, 0.5, 3);
        let mut receiver = MotionReceiver::default();
        receiver.enter(3);
        let mut net = Network::new(loss, duplicate, window);
        let mut at = sender.pos();
        let mut last_applied = 0u32;
        let (mut moved, mut stale, mut repeats) = (0, 0, 0);
        for step in 0..20_000 {
            let (dx, dy) = random_delta(&mut rng);
            match sender.motion(dx, dy) {
                Step::Move(datagram) => net.send(&mut rng, datagram),
                // Pushed against the way home, or pinned at an edge: nothing to send.
                Step::Leave { .. } | Step::Still => {}
            }
            // A keepalive now and then, as the idle pointer sends.
            if step % 50 == 0 {
                net.send(&mut rng, sender.current());
            }
            for _ in 0..rng.below(3) {
                let Some(Datagram::Motion { epoch, seq, pos }) = net.deliver(&mut rng) else { continue };
                match receiver.accept(epoch, seq, pos) {
                    Received::Move(p) => {
                        moved += 1;
                        assert_eq!(p, pos);
                        at = p;
                        // The pointer only ever goes to newer positions: sequence numbers advance.
                        assert!(seq.wrapping_sub(last_applied) as i32 > 0, "applied {seq} after {last_applied}");
                        last_applied = seq;
                    }
                    Received::Repeat => repeats += 1,
                    Received::Stale => stale += 1,
                }
                assert!(desktop().screen_at(at).is_some(), "the receiver's pointer is on a monitor: {at:?}");
            }
        }
        // Whatever is still in flight arrives; then the sender's idle keepalive lands.
        for d in net.drain(&mut rng) {
            if let Datagram::Motion { epoch, seq, pos } = d
                && let Received::Move(p) = receiver.accept(epoch, seq, pos)
            {
                at = p;
            }
        }
        let Datagram::Motion { epoch, seq, pos } = sender.current() else { panic!("absolute") };
        if let Received::Move(p) = receiver.accept(epoch, seq, pos) {
            at = p;
        }
        assert_eq!(at, sender.pos(), "loss {loss}% dup {duplicate}% window {window}: the pointers agree at rest");
        assert!(moved > 0 && (loss > 50 || stale + repeats > 0), "{moved} {stale} {repeats}");
    }
}

#[test]
fn datagrams_from_an_earlier_visit_never_move_the_pointer_of_the_next() {
    let mut rng = Rng(0xE0C4);
    let mut receiver = MotionReceiver::default();
    let mut old = Vec::new();
    for epoch in 0..300u16 {
        let epoch = epoch.wrapping_mul(7919);
        let mut sender = RemoteCursor::enter(desktop(), Side::Left, 0.25, epoch);
        receiver.enter(epoch);
        let mut here = Vec::new();
        for _ in 0..20 {
            if let Step::Move(d) = sender.motion(rng.below(40) as f64 - 5.0, rng.below(40) as f64 - 20.0) {
                here.push(d);
            }
        }
        // Stragglers of earlier visits arrive in the middle of this one.
        for stray in old.iter().rev().take(30) {
            let Datagram::Motion { epoch: e, seq, pos } = *stray else { unreachable!() };
            if e != epoch {
                assert_eq!(receiver.accept(e, seq, pos), Received::Stale, "epoch {e} during epoch {epoch}");
            }
        }
        for d in &here {
            let Datagram::Motion { epoch: e, seq, pos } = *d else { unreachable!() };
            assert!(matches!(receiver.accept(e, seq, pos), Received::Move(_)));
        }
        old.extend(here);
        receiver.leave();
        assert!(!receiver.is_active());
        // Nothing moves a pointer that nobody is controlling, not even the newest datagram.
        let Some(Datagram::Motion { epoch: e, seq, pos }) = old.last().copied() else { continue };
        assert_eq!(receiver.accept(e, seq.wrapping_add(1), pos), Received::Stale);
    }
}

#[test]
fn a_late_click_is_placed_where_it_happened_and_the_pointer_goes_back() {
    let mut receiver = MotionReceiver::default();
    receiver.enter(1);
    assert!(matches!(receiver.accept(1, 5, Pos::new(10.0, 10.0)), Received::Move(_)));
    // The click carries seq 4, but position 5 was already applied: the click is late.
    assert!(!receiver.sync(4));
    assert!(receiver.sync(5), "a click with the latest position is on time");
    // A click that is ahead of the datagrams makes older datagrams stale.
    assert!(receiver.sync(9));
    assert_eq!(receiver.accept(1, 8, Pos::new(0.0, 0.0)), Received::Stale);
    assert_eq!(receiver.accept(1, 9, Pos::new(0.0, 0.0)), Received::Repeat);
    assert!(matches!(receiver.accept(1, 10, Pos::new(1.0, 1.0)), Received::Move(_)));
    // Across the wrap of the 32-bit counter ordering still holds.
    let mut wrapped = MotionReceiver::default();
    wrapped.enter(2);
    // (A jump of up to half the range is newer; so the counter can run on through the half-way
    // point, the top and zero.)
    for seq in [0x7FFF_FFFF, 0x8000_0000, u32::MAX - 1, u32::MAX, 0, 1, 2] {
        assert!(matches!(wrapped.accept(2, seq, Pos::new(1.0, 1.0)), Received::Move(_)), "{seq}");
    }
    assert_eq!(wrapped.accept(2, u32::MAX, Pos::new(1.0, 1.0)), Received::Stale, "from before the wrap");
    assert!(!wrapped.sync(u32::MAX), "a click stamped before the wrap is late");
}

#[test]
fn relative_motion_never_drifts_over_a_bad_network_and_across_the_wrap_of_the_sum() {
    for (seed, loss, duplicate, window, fling) in
        [(1u64, 0, 0, 1, false), (2, 40, 30, 6, false), (3, 80, 10, 2, false), (4, 25, 25, 5, true)]
    {
        let mut rng = Rng(0xBEE5 ^ seed);
        let mut sender = RemoteCursor::enter(desktop(), Side::Left, 0.5, 9);
        sender.set_relative(true);
        let mut receiver = MotionReceiver::default();
        receiver.enter(9);
        let mut net = Network::new(loss, duplicate, window);
        let mut moved = (0i64, 0i64);
        let mut sent = (0f64, 0f64);
        for step in 0..30_000 {
            // Flings of a hundred thousand pixels (far past any screen) make the 1/256 px sum wrap its 32 bits many times.
            let (dx, dy) = if fling && step % 5 == 0 { (100_000.0, 25_000.0) } else { random_delta(&mut rng) };
            if let Step::Move(d) = sender.motion(dx, dy) {
                sent = (sent.0 + dx, sent.1 + dy);
                net.send(&mut rng, d);
            }
            if step % 40 == 0 {
                net.send(&mut rng, sender.current());
            }
            for _ in 0..rng.below(3) {
                if let Some(Datagram::MotionBy { epoch, seq, sum_x, sum_y }) = net.deliver(&mut rng)
                    && let RelReceived::MoveBy { dx, dy } = receiver.accept_relative(epoch, seq, (sum_x, sum_y))
                {
                    moved = (moved.0 + i64::from(dx), moved.1 + i64::from(dy));
                }
            }
        }
        for d in net.drain(&mut rng) {
            if let Datagram::MotionBy { epoch, seq, sum_x, sum_y } = d
                && let RelReceived::MoveBy { dx, dy } = receiver.accept_relative(epoch, seq, (sum_x, sum_y))
            {
                moved = (moved.0 + i64::from(dx), moved.1 + i64::from(dy));
            }
        }
        let Datagram::MotionBy { epoch, seq, sum_x, sum_y } = sender.current() else { panic!("relative") };
        if let RelReceived::MoveBy { dx, dy } = receiver.accept_relative(epoch, seq, (sum_x, sum_y)) {
            moved = (moved.0 + i64::from(dx), moved.1 + i64::from(dy));
        }
        // Everything the sender moved by arrived, to within the half pixel of rounding: no loss,
        // repeat or reordering leaves a residue.
        let target = (sent.0.round() as i64, sent.1.round() as i64);
        assert!(
            (moved.0 - target.0).abs() <= 1 && (moved.1 - target.1).abs() <= 1,
            "loss {loss}% dup {duplicate}%: moved {moved:?} but the sender moved {sent:?}"
        );
    }
}

#[test]
fn switching_between_absolute_and_relative_mid_visit_keeps_both_ends_consistent() {
    let mut rng = Rng(0x5117);
    let mut sender = RemoteCursor::enter(desktop(), Side::Left, 0.5, 4);
    let mut receiver = MotionReceiver::default();
    receiver.enter(4);
    let mut net = Network::new(20, 20, 4);
    let mut at = sender.pos();
    let mut moved = (0i64, 0i64);
    let mut sent_relative = (0f64, 0f64);
    for step in 0..6000 {
        if step % 97 == 0 {
            let on = !sender.is_relative();
            assert!(sender.set_relative(on), "a real change is reported");
            assert!(!sender.set_relative(on), "and the same mode again is not");
            net.send(&mut rng, sender.current());
        }
        let (dx, dy) = random_delta(&mut rng);
        match sender.motion(dx, dy) {
            Step::Move(d) => {
                if sender.is_relative() {
                    sent_relative = (sent_relative.0 + dx, sent_relative.1 + dy);
                }
                net.send(&mut rng, d);
            }
            Step::Leave { .. } | Step::Still => {}
        }
        for _ in 0..rng.below(3) {
            match net.deliver(&mut rng) {
                Some(Datagram::Motion { epoch, seq, pos }) => {
                    if let Received::Move(p) = receiver.accept(epoch, seq, pos) {
                        at = p;
                    }
                }
                Some(Datagram::MotionBy { epoch, seq, sum_x, sum_y }) => {
                    if let RelReceived::MoveBy { dx, dy } = receiver.accept_relative(epoch, seq, (sum_x, sum_y)) {
                        moved = (moved.0 + i64::from(dx), moved.1 + i64::from(dy));
                    }
                }
                _ => {}
            }
        }
    }
    // After the dust settles, in whichever mode it ended, a final datagram makes both agree.
    let _lost = net.drain(&mut rng);
    match sender.current() {
        Datagram::Motion { epoch, seq, pos } => {
            if let Received::Move(p) = receiver.accept(epoch, seq, pos) {
                at = p;
            }
            assert_eq!(at, sender.pos());
        }
        Datagram::MotionBy { epoch, seq, sum_x, sum_y } => {
            // The running sum is over the whole visit, in both modes' relative stretches.
            if let RelReceived::MoveBy { dx, dy } = receiver.accept_relative(epoch, seq, (sum_x, sum_y)) {
                moved = (moved.0 + i64::from(dx), moved.1 + i64::from(dy));
            }
            assert!(sent_relative.0.abs() < 1e7 && moved.0.abs() < 10_000_000, "{sent_relative:?} {moved:?}");
        }
        other => panic!("{other:?}"),
    }
}
