//! Scrolling for the computer input is sent to: direction per axis,
//! natural scrolling, speed, smooth or notched, and modifier keys that
//! change scrolling (Shift scrolls sideways, Alt faster).
//!
//! Wire conventions (as libei): smooth scrolling in logical pixels,
//! notched scrolling in 1/120 of a notch, positive `dy` scrolls down
//! (the wheel turned towards you), positive `dx` right.

use crate::keynames::sides;
use crate::os;

/// Pixels per wheel notch when converting (the Windows backend's figure).
pub const PX_PER_NOTCH: f64 = 48.0;
/// 1/120 of a notch is the wire's unit for notched scrolling.
pub const UNITS_PER_NOTCH: f64 = 120.0;
pub const MIN_SPEED: f64 = 0.1;
pub const MAX_SPEED: f64 = 10.0;
pub const MAX_MULTIPLIER: f64 = 50.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// As it comes: smooth stays smooth, notches stay notches.
    #[default]
    Auto,
    /// Everything as pixels.
    Smooth,
    /// Everything in whole notches ("scroll by lines").
    Discrete,
}

/// What holding a modifier does to scrolling.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ModScroll {
    #[default]
    Unchanged,
    /// Vertical scrolling becomes horizontal (and horizontal vertical).
    Horizontal,
    /// Only vertical scrolling: horizontal is dropped.
    Vertical,
    /// Faster or slower.
    Multiply(f64),
}

/// Settings for one kind of device (wheel or touchpad).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub reverse_x: bool,
    pub reverse_y: bool,
    pub speed_x: f64,
    pub speed_y: f64,
    pub mode: Mode,
}

impl Default for Params {
    fn default() -> Self {
        Params { reverse_x: false, reverse_y: false, speed_x: 1.0, speed_y: 1.0, mode: Mode::Auto }
    }
}

/// Everything about scrolling for one target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollConfig {
    /// Notched wheels (`ScrollDiscrete`).
    pub wheel: Params,
    /// Touchpads and smooth scrolling (`Scroll`).
    pub smooth: Params,
    /// By modifier group: Ctrl, Shift, Alt, Meta (see `keynames::CTRL`...).
    pub modifiers: [ModScroll; 4],
}

impl ScrollConfig {
    pub fn is_identity(&self) -> bool {
        *self == ScrollConfig::default()
    }
}

/// `natural` in the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Natural {
    On,
    Off,
    /// The target OS's own default: natural on a Mac, traditional elsewhere.
    Auto,
}

/// Whether `natural` means flipping both axes, sending from an OS
/// `local` to one `target`. Assumes this computer delivers scrolling the
/// way its OS does by default (natural on a Mac, traditional elsewhere).
pub fn natural_flip(natural: Option<Natural>, local: u8, target: u8) -> bool {
    let here = local == os::MACOS;
    match natural {
        None => false,
        Some(Natural::On) => !here,
        Some(Natural::Off) => here,
        Some(Natural::Auto) if target == os::UNKNOWN => false,
        Some(Natural::Auto) => here != (target == os::MACOS),
    }
}

/// What to send.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollOut {
    Smooth { dx: f64, dy: f64 },
    Discrete { dx: i32, dy: i32 },
    Stop,
}

/// The result of one scroll event: what to send, and which held modifiers
/// (sided bits) changed it, to be lifted off the target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tuned {
    pub out: Option<ScrollOut>,
    pub used: u8,
}

/// Scroll tuning for one visit; keeps the partial notches.
#[derive(Clone, Debug, Default)]
pub struct ScrollTune {
    cfg: ScrollConfig,
    /// What hasn't been sent yet, in 1/120 notch.
    rem: (f64, f64),
}

impl ScrollTune {
    pub fn new(cfg: ScrollConfig) -> Self {
        ScrollTune { cfg, rem: (0.0, 0.0) }
    }

    pub fn config(&self) -> &ScrollConfig {
        &self.cfg
    }

    pub fn is_identity(&self) -> bool {
        self.cfg.is_identity()
    }

    /// Smooth scrolling of `dx, dy` pixels with the modifiers `held`.
    pub fn smooth(&mut self, dx: f64, dy: f64, held: u8) -> Tuned {
        let p = self.cfg.smooth;
        let (dx, dy, used) = self.shape(dx, dy, &p, held);
        let out = match p.mode {
            Mode::Auto | Mode::Smooth => (dx != 0.0 || dy != 0.0).then_some(ScrollOut::Smooth { dx, dy }),
            Mode::Discrete => self.notches(dx / PX_PER_NOTCH * UNITS_PER_NOTCH, dy / PX_PER_NOTCH * UNITS_PER_NOTCH, true),
        };
        Tuned { out, used }
    }

    /// Notched scrolling of `dx, dy` 1/120 notches with the modifiers `held`.
    pub fn discrete(&mut self, dx: i32, dy: i32, held: u8) -> Tuned {
        let p = self.cfg.wheel;
        let (dx, dy, used) = self.shape(f64::from(dx), f64::from(dy), &p, held);
        let out = match p.mode {
            Mode::Smooth => (dx != 0.0 || dy != 0.0).then(|| ScrollOut::Smooth {
                dx: dx / UNITS_PER_NOTCH * PX_PER_NOTCH,
                dy: dy / UNITS_PER_NOTCH * PX_PER_NOTCH,
            }),
            Mode::Auto => self.notches(dx, dy, false),
            Mode::Discrete => self.notches(dx, dy, true),
        };
        Tuned { out, used }
    }

    /// The fingers left the touchpad. In discrete mode nothing is sent
    /// (and the partial notch is dropped: the gesture is over).
    pub fn stop(&mut self) -> Option<ScrollOut> {
        self.rem = (0.0, 0.0);
        (self.cfg.smooth.mode != Mode::Discrete).then_some(ScrollOut::Stop)
    }

    /// Modifiers, direction and speed.
    fn shape(&self, mut dx: f64, mut dy: f64, p: &Params, held: u8) -> (f64, f64, u8) {
        let mut used = 0;
        for (g, m) in self.cfg.modifiers.iter().enumerate() {
            let bits = held & sides(1 << g);
            if bits == 0 || *m == ModScroll::Unchanged {
                continue;
            }
            used |= bits;
            match *m {
                ModScroll::Unchanged => {}
                ModScroll::Horizontal => (dx, dy) = (dy, dx),
                ModScroll::Vertical => dx = 0.0,
                ModScroll::Multiply(k) => (dx, dy) = (dx * k, dy * k),
            }
        }
        if p.reverse_x {
            dx = -dx;
        }
        if p.reverse_y {
            dy = -dy;
        }
        (dx * p.speed_x, dy * p.speed_y, used)
    }

    /// Adds `dx, dy` (1/120 notch) to what is left over and sends what is
    /// whole: whole notches if `whole`, else whole units. A change of
    /// direction drops what was left over the other way.
    fn notches(&mut self, dx: f64, dy: f64, whole: bool) -> Option<ScrollOut> {
        let step = if whole { UNITS_PER_NOTCH } else { 1.0 };
        let take = |rem: &mut f64, d: f64| -> i32 {
            if d != 0.0 && rem.signum() != d.signum() {
                *rem = 0.0;
            }
            *rem += d;
            let n = (*rem / step).trunc() * step;
            *rem -= n;
            n.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        };
        let (mut rx, mut ry) = self.rem;
        let (ox, oy) = (take(&mut rx, dx), take(&mut ry, dy));
        self.rem = (rx, ry);
        (ox != 0 || oy != 0).then_some(ScrollOut::Discrete { dx: ox, dy: oy })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keynames::{LALT, LCTRL, LSHIFT, RSHIFT};

    fn tune(f: impl FnOnce(&mut ScrollConfig)) -> ScrollTune {
        let mut c = ScrollConfig::default();
        f(&mut c);
        ScrollTune::new(c)
    }

    #[test]
    fn unchanged_by_default() {
        let mut t = ScrollTune::default();
        assert!(t.is_identity());
        assert_eq!(t.smooth(1.5, -2.0, 0).out, Some(ScrollOut::Smooth { dx: 1.5, dy: -2.0 }));
        assert_eq!(t.discrete(0, 120, LSHIFT).out, Some(ScrollOut::Discrete { dx: 0, dy: 120 }));
        assert_eq!(t.discrete(0, 15, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: 15 }), "high-resolution wheels");
        assert_eq!(t.stop(), Some(ScrollOut::Stop));
    }

    #[test]
    fn reverse_per_axis_and_per_device() {
        let mut t = tune(|c| {
            c.wheel.reverse_y = true;
            c.smooth.reverse_x = true;
        });
        assert_eq!(t.discrete(120, 120, 0).out, Some(ScrollOut::Discrete { dx: 120, dy: -120 }));
        assert_eq!(t.smooth(3.0, 4.0, 0).out, Some(ScrollOut::Smooth { dx: -3.0, dy: 4.0 }));
    }

    #[test]
    fn speed_keeps_partial_units() {
        let mut t = tune(|c| c.wheel.speed_y = 1.5);
        assert_eq!(t.discrete(0, 1, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: 1 }));
        assert_eq!(t.discrete(0, 1, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: 2 }), "1.5 + 1.5 = 3");
        assert_eq!(t.discrete(0, 120, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: 180 }));
    }

    #[test]
    fn discrete_mode_sends_whole_notches() {
        let mut t = tune(|c| {
            c.smooth.mode = Mode::Discrete;
            c.wheel.mode = Mode::Discrete;
            c.wheel.speed_y = 0.5;
        });
        // 48 px is a notch.
        assert_eq!(t.smooth(0.0, 30.0, 0).out, None);
        assert_eq!(t.smooth(0.0, 30.0, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: 120 }));
        assert_eq!(t.stop(), None, "no stop in discrete mode, and the partial notch is dropped");
        assert_eq!(t.smooth(0.0, 30.0, 0).out, None);
        t.stop();
        // Half speed on the wheel: every other notch.
        assert_eq!(t.discrete(0, 120, 0).out, None);
        assert_eq!(t.discrete(0, 120, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: 120 }));
        // Turning back drops what was left over.
        assert_eq!(t.discrete(0, 120, 0).out, None);
        assert_eq!(t.discrete(0, -120, 0).out, None);
        assert_eq!(t.discrete(0, -120, 0).out, Some(ScrollOut::Discrete { dx: 0, dy: -120 }));
    }

    #[test]
    fn smooth_mode_turns_notches_into_pixels() {
        let mut t = tune(|c| c.wheel.mode = Mode::Smooth);
        assert_eq!(t.discrete(-60, 120, 0).out, Some(ScrollOut::Smooth { dx: -24.0, dy: 48.0 }));
    }

    #[test]
    fn modifiers_change_scrolling_and_say_so() {
        let mut t = tune(|c| {
            c.modifiers[1] = ModScroll::Horizontal; // Shift
            c.modifiers[2] = ModScroll::Multiply(5.0); // Alt
            c.modifiers[0] = ModScroll::Unchanged; // Ctrl
        });
        assert_eq!(t.discrete(0, 120, RSHIFT), Tuned { out: Some(ScrollOut::Discrete { dx: 120, dy: 0 }), used: RSHIFT });
        assert_eq!(t.discrete(0, 120, LALT), Tuned { out: Some(ScrollOut::Discrete { dx: 0, dy: 600 }), used: LALT });
        assert_eq!(t.discrete(0, 120, LCTRL), Tuned { out: Some(ScrollOut::Discrete { dx: 0, dy: 120 }), used: 0 });
        assert_eq!(
            t.smooth(1.0, 2.0, LSHIFT | LALT),
            Tuned { out: Some(ScrollOut::Smooth { dx: 10.0, dy: 5.0 }), used: LSHIFT | LALT }
        );
        let mut v = tune(|c| c.modifiers[0] = ModScroll::Vertical);
        assert_eq!(v.smooth(3.0, 2.0, LCTRL).out, Some(ScrollOut::Smooth { dx: 0.0, dy: 2.0 }));
        assert_eq!(v.smooth(3.0, 0.0, LCTRL).out, None);
    }

    #[test]
    fn natural_follows_the_assumed_defaults() {
        use crate::os::{LINUX, MACOS, UNKNOWN, WINDOWS};
        assert!(!natural_flip(None, LINUX, MACOS));
        // auto: flip between a Mac and anything else.
        assert!(natural_flip(Some(Natural::Auto), LINUX, MACOS));
        assert!(natural_flip(Some(Natural::Auto), MACOS, WINDOWS));
        assert!(!natural_flip(Some(Natural::Auto), WINDOWS, LINUX));
        assert!(!natural_flip(Some(Natural::Auto), MACOS, MACOS));
        assert!(!natural_flip(Some(Natural::Auto), LINUX, UNKNOWN));
        // on/off: what the target gets, whatever it is.
        assert!(natural_flip(Some(Natural::On), WINDOWS, WINDOWS));
        assert!(!natural_flip(Some(Natural::On), MACOS, WINDOWS));
        assert!(natural_flip(Some(Natural::Off), MACOS, MACOS));
        assert!(!natural_flip(Some(Natural::Off), LINUX, MACOS));
    }

    /// A million events at awkward speeds: what is sent adds up to what
    /// came in, times the speed, to within one notch (what is still
    /// waiting), in every mode.
    #[test]
    fn no_drift_over_a_million_events() {
        for (mode, smooth_in) in [(Mode::Discrete, true), (Mode::Discrete, false), (Mode::Auto, false)] {
            let mut t = tune(|c| {
                c.smooth.mode = mode;
                c.wheel.mode = mode;
                c.smooth.speed_y = 0.37;
                c.wheel.speed_y = 0.37;
            });
            let mut sent = 0i64;
            let n = 1_000_000;
            for _ in 0..n {
                let tuned = if smooth_in { t.smooth(0.0, 7.0, 0) } else { t.discrete(0, 7, 0) };
                match tuned.out {
                    Some(ScrollOut::Discrete { dy, .. }) => sent += i64::from(dy),
                    None => {}
                    other => panic!("{other:?}"),
                }
            }
            let units_in = if smooth_in { 7.0 / PX_PER_NOTCH * UNITS_PER_NOTCH } else { 7.0 };
            let want = units_in * 0.37 * f64::from(n);
            let step = if mode == Mode::Discrete { UNITS_PER_NOTCH } else { 1.0 };
            assert!((sent as f64 - want).abs() < step + 1e-3, "{mode:?} smooth={smooth_in}: sent {sent}, want {want}");
        }
    }
}
