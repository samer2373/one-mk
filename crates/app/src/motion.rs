//! Timing: easing curves, tweens that run for a set time, a spring for
//! things let go by hand, and the reduced-motion switch. Every animation
//! in the window is bounded: it runs while something changes, then stops
//! asking for frames.

use egui::{Context, Id};
use std::sync::atomic::{AtomicBool, Ordering};

static REDUCED: AtomicBool = AtomicBool::new(false);

/// True when the person asked for less motion: spatial movement snaps and
/// only fades and colour changes remain.
pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

pub fn set_reduced(on: bool) {
    REDUCED.store(on, Ordering::Relaxed);
}

/// `ONE_MK_REDUCE_MOTION=1` (or `0`) wins; otherwise the desktop's setting.
pub fn detect() -> bool {
    if let Some(v) = std::env::var_os("ONE_MK_REDUCE_MOTION") {
        return v != "0" && !v.is_empty();
    }
    let read = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd).args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    if cfg!(target_os = "linux") {
        read("gsettings", &["get", "org.gnome.desktop.interface", "enable-animations"]).is_some_and(|s| s == "false")
    } else if cfg!(target_os = "macos") {
        read("defaults", &["read", "com.apple.universalaccess", "reduceMotion"]).is_some_and(|s| s == "1")
    } else {
        false
    }
}

/// Duration for movement through space: none when motion is reduced.
pub fn space(secs: f32) -> f32 {
    if reduced() { 0.0 } else { secs }
}

/// Duration for a fade: kept, but short, when motion is reduced.
pub fn fade(secs: f32) -> f32 {
    if reduced() { secs.min(0.12) } else { secs }
}

pub type Ease = fn(f32) -> f32;

pub fn linear(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}
pub fn out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}
pub fn out_quart(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4)
}
/// Confident arrivals: fast, then a long settle. Normalised to end at 1.
pub fn out_expo(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (1.0 - 2f32.powf(-10.0 * t)) / (1.0 - 2f32.powi(-10))
}
pub fn in_cubic(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}
pub fn in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}
/// Overshoots by a hair and settles: the switch knob.
pub fn back_out(t: f32) -> f32 {
    let (c1, t) = (1.2, t.clamp(0.0, 1.0) - 1.0);
    1.0 + (c1 + 1.0) * t * t * t + c1 * t * t
}

/// Progress of something that started at `t0` and lasts `secs`, eased.
pub fn progress(now: f64, t0: f64, secs: f32, ease: Ease) -> f32 {
    if secs <= 0.0 {
        return if now >= t0 { 1.0 } else { 0.0 };
    }
    ease(((now - t0) as f32 / secs).clamp(0.0, 1.0))
}

/// A value that goes to each new target over a fixed time. Retargeting
/// mid-way starts from where it is, so interruptions never jump.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    t0: f64,
    secs: f32,
    ease: Ease,
}

impl Tween {
    pub fn new(v: f32) -> Self {
        Self { from: v, to: v, t0: 0.0, secs: 0.0, ease: linear }
    }

    /// Starts at `from` and heads to `to` once `t0` comes.
    pub fn from_to(from: f32, to: f32, t0: f64, secs: f32, ease: Ease) -> Self {
        Self { from, to, t0, secs, ease }
    }

    pub fn value(&self, now: f64) -> f32 {
        self.from + (self.to - self.from) * progress(now, self.t0, self.secs, self.ease)
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn moving(&self, now: f64) -> bool {
        self.from != self.to && now < self.t0 + self.secs as f64
    }

    /// Heads for `to`, unless it already is.
    pub fn go(&mut self, to: f32, now: f64, secs: f32, ease: Ease) {
        if to != self.to {
            *self = Self { from: self.value(now), to, t0: now, secs, ease };
        }
    }

    /// Jumps straight to `v`.
    pub fn set(&mut self, v: f32) {
        *self = Self::new(v);
    }
}

/// A tween kept in egui's memory under `id`; asks for frames while moving.
pub fn tween(ctx: &Context, id: Id, to: f32, secs: f32, ease: Ease) -> f32 {
    let now = ctx.input(|i| i.time);
    let mut t = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || Tween::new(to)));
    t.go(to, now, secs, ease);
    ctx.data_mut(|d| d.insert_temp(id, t));
    if t.moving(now) {
        ctx.request_repaint();
    }
    t.value(now)
}

/// A critically-damped-ish spring (ω 18, ζ 0.72): settles in about
/// 350 ms with one small overshoot. Returns true while still moving.
pub fn spring(x: &mut f32, vel: &mut f32, to: f32, dt: f32, eps: f32) -> bool {
    if reduced() {
        *x = to;
        *vel = 0.0;
        return false;
    }
    const W: f32 = 18.0;
    const Z: f32 = 0.72;
    const H: f32 = 1.0 / 240.0;
    let mut left = dt;
    while left > 0.0 {
        let h = left.min(H);
        *vel += (-2.0 * Z * W * *vel - W * W * (*x - to)) * h;
        *x += *vel * h;
        left -= h;
    }
    if (*x - to).abs() < eps && vel.abs() < eps * 10.0 {
        *x = to;
        *vel = 0.0;
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_at_zero_and_end_at_one() {
        for e in [linear, out_cubic, out_quart, out_expo, in_cubic, in_out_cubic, back_out] {
            assert!(e(0.0).abs() < 1e-6);
            assert!((e(1.0) - 1.0).abs() < 1e-6);
        }
        assert!((0..100).map(|i| back_out(i as f32 / 100.0)).any(|x| x > 1.0), "the knob overshoots");
        assert!((0..100).map(|i| back_out(i as f32 / 100.0)).all(|x| x < 1.06), "but only by a hair");
    }

    #[test]
    fn tweens_retarget_from_where_they_are() {
        let mut t = Tween::new(0.0);
        t.go(1.0, 0.0, 1.0, linear);
        assert!((t.value(0.5) - 0.5).abs() < 1e-6);
        t.go(0.0, 0.5, 1.0, linear);
        assert!((t.value(0.5) - 0.5).abs() < 1e-6, "no jump");
        assert!(t.moving(1.0));
        assert!(!t.moving(1.6));
        assert_eq!(t.value(2.0), 0.0);
    }

    #[test]
    fn springs_settle_with_a_small_overshoot() {
        let (mut x, mut v, mut peak, mut frames) = (0.0f32, 0.0f32, 0.0f32, 0);
        while spring(&mut x, &mut v, 100.0, 1.0 / 60.0, 0.05) {
            peak = peak.max(x);
            frames += 1;
            assert!(frames < 120, "settles within two seconds");
        }
        assert_eq!(x, 100.0);
        assert!(peak > 100.5 && peak < 108.0, "one small overshoot: {peak}");
        assert!(frames < 45, "about 350 ms (plus the tail): {frames} frames");
    }
}
