//! Pointer speed and acceleration for the computer input is sent to,
//! applied to this computer's motion deltas before
//! [`RemoteCursor`](crate::RemoteCursor) adds them up.
//!
//! Deltas stay `f64` all the way (the cursor's position and the relative
//! sum are `f64` too), so scaling never rounds: there is no remainder to
//! lose and nothing drifts.
//!
//! Acceleration other than the system's needs *unaccelerated* deltas; it
//! can't be divided back out of accelerated ones. [`PointerTune::apply`]
//! is told whether the deltas are raw, and without them uses the system's
//! (see `docs/INPUT-TUNING.md` for which backends can give raw deltas).

pub const MIN_SPEED: f64 = 0.1;
pub const MAX_SPEED: f64 = 10.0;
pub const MAX_GAIN: f64 = 10.0;
pub const MIN_EXP: f64 = 0.1;
pub const MAX_EXP: f64 = 3.0;
pub const MAX_CAP: f64 = 20.0;

/// Acceleration on top of the speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Accel {
    /// What this computer's OS did to the motion (the default).
    System,
    /// None: the raw deltas, times the speed.
    Off,
    /// `gain = min(1 + k * |delta|^exp, cap)`, `|delta|` in counts per report.
    Curve { k: f64, exp: f64, cap: f64 },
}

impl Accel {
    /// A curve with its numbers kept in range.
    pub fn curve(k: f64, exp: f64, cap: f64) -> Accel {
        let fix = |v: f64, lo: f64, hi: f64, default: f64| if v.is_finite() { v.clamp(lo, hi) } else { default };
        Accel::Curve { k: fix(k, 0.0, MAX_GAIN, 0.0), exp: fix(exp, MIN_EXP, MAX_EXP, 1.0), cap: fix(cap, 1.0, MAX_CAP, 1.0) }
    }

    /// Whether this needs unaccelerated deltas.
    pub fn needs_raw(self) -> bool {
        self != Accel::System
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerTune {
    speed: f64,
    accel: Accel,
}

impl Default for PointerTune {
    fn default() -> Self {
        PointerTune { speed: 1.0, accel: Accel::System }
    }
}

/// `v` within `lo..=hi`; not a number counts as `default`.
pub fn clamp_or(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

impl PointerTune {
    /// The speed is kept within [`MIN_SPEED`]..=[`MAX_SPEED`].
    pub fn new(speed: f64, accel: Accel) -> Self {
        PointerTune { speed: clamp_or(speed, MIN_SPEED, MAX_SPEED, 1.0), accel }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// The acceleration in force when the deltas are `raw` or not.
    pub fn accel(&self, raw: bool) -> Accel {
        if raw { self.accel } else { Accel::System }
    }

    /// Changes nothing (the daemon skips it then).
    pub fn is_identity(&self, raw: bool) -> bool {
        self.speed == 1.0 && matches!(self.accel(raw), Accel::System | Accel::Off)
    }

    /// One motion report: `raw` says whether the backend's deltas are
    /// unaccelerated.
    pub fn apply(&self, dx: f64, dy: f64, raw: bool) -> (f64, f64) {
        let gain = match self.accel(raw) {
            Accel::System | Accel::Off => 1.0,
            Accel::Curve { k, exp, cap } => (1.0 + k * dx.hypot(dy).powf(exp)).min(cap),
        };
        let g = gain * self.speed;
        (dx * g, dy * g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_scales_and_is_clamped() {
        assert_eq!(PointerTune::new(2.0, Accel::System).apply(3.0, -1.5, false), (6.0, -3.0));
        assert_eq!(PointerTune::new(100.0, Accel::System).speed(), MAX_SPEED);
        assert_eq!(PointerTune::new(0.0, Accel::System).speed(), MIN_SPEED);
        assert_eq!(PointerTune::new(f64::NAN, Accel::System).speed(), 1.0);
        assert!(PointerTune::default().is_identity(false));
        assert!(!PointerTune::new(1.5, Accel::System).is_identity(false));
    }

    #[test]
    fn acceleration_needs_raw_deltas() {
        let curve = PointerTune::new(1.0, Accel::curve(0.5, 1.0, 4.0));
        // Accelerated input: the system's acceleration stands; nothing added.
        assert_eq!(curve.apply(2.0, 0.0, false), (2.0, 0.0));
        assert_eq!(curve.accel(false), Accel::System);
        // Raw: gain = 1 + 0.5 * 2 = 2.
        assert_eq!(curve.apply(2.0, 0.0, true), (4.0, 0.0));
        // Capped at 4: 1 + 0.5 * 10 = 6.
        assert_eq!(curve.apply(0.0, 10.0, true), (0.0, 40.0));
        let off = PointerTune::new(1.5, Accel::Off);
        assert_eq!(off.apply(2.0, 4.0, true), (3.0, 6.0));
        assert!(Accel::Off.needs_raw() && !Accel::System.needs_raw());
        assert_eq!(Accel::curve(f64::INFINITY, 100.0, 0.0), Accel::Curve { k: 0.0, exp: MAX_EXP, cap: 1.0 });
    }

    /// A million sub-pixel moves at an odd speed add up to exactly what
    /// they should: nothing is rounded away and nothing drifts.
    #[test]
    fn no_drift_over_a_million_moves() {
        let t = PointerTune::new(1.7, Accel::System);
        let (mut x, mut y) = (0.0f64, 0.0f64);
        let n = 1_000_000;
        for i in 0..n {
            let (dx, dy) = t.apply(0.3, if i % 2 == 0 { -0.45 } else { 0.25 }, false);
            x += dx;
            y += dy;
        }
        let want_x = 0.3 * 1.7 * f64::from(n);
        let want_y = (-0.45 + 0.25) / 2.0 * 1.7 * f64::from(n);
        assert!((x - want_x).abs() < 1e-3, "{x} vs {want_x}");
        assert!((y - want_y).abs() < 1e-3, "{y} vs {want_y}");
    }
}
