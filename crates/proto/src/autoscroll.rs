//! Visit-local autoscroll. The caller schedules ticks only while active.

use std::time::{Duration, Instant};

pub const TICK: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub button: u32,
    pub speed: f64,
    pub dead_zone: f64,
    pub vertical_only: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { button: 0x112, speed: 1.0, dead_zone: 8.0, vertical_only: false }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), &'static str> {
        if matches!(self.button, 0x110 | 0x111) {
            return Err("primary left/right buttons must remain available for clicking; choose middle or button4 instead");
        }
        if !crate::keynames::is_button(self.button) {
            return Err("choose a mouse button (for example middle or button4)");
        }
        if !self.speed.is_finite() || !(0.1..=10.0).contains(&self.speed) {
            return Err("autoscroll speed must be between 0.1 and 10");
        }
        if !self.dead_zone.is_finite() || !(0.0..=1000.0).contains(&self.dead_zone) {
            return Err("autoscroll dead zone must be between 0 and 1000 backend motion units");
        }
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn primary_buttons_cannot_capture_clicks() {
    for button in [0x110, 0x111] {
        assert!(Config { button, ..Config::default() }.validate().is_err());
    }
}

#[derive(Clone, Debug, Default)]
pub struct Autoscroll {
    config: Option<Config>,
    displacement: (f64, f64),
    next: Option<Instant>,
    pressed: bool,
}

impl Autoscroll {
    pub fn new(config: Option<Config>) -> Self {
        Self { config: config.filter(|cfg| cfg.validate().is_ok()), ..Self::default() }
    }

    pub fn active(&self) -> bool { self.next.is_some() }

    pub fn config(&self) -> Option<Config> { self.config }

    pub fn deadline(&self) -> Option<Instant> { self.next }

    pub fn stop(&mut self) {
        self.next = None;
        self.displacement = (0.0, 0.0);
    }

    /// True means this transition belongs to the swallowed toggle click.
    /// Other button presses stop scrolling but are still forwarded.
    pub fn key(&mut self, code: u32, down: bool, now: Instant) -> bool {
        let Some(config) = self.config else { return false };
        if code == config.button {
            if down && !self.pressed {
                if self.active() {
                    self.stop();
                } else {
                    self.displacement = (0.0, 0.0);
                    self.next = Some(now + TICK);
                }
            }
            self.pressed = down;
            return true;
        }
        if down && (code == 1 || crate::keynames::is_button(code)) { self.stop(); }
        false
    }

    pub fn motion(&mut self, dx: f64, dy: f64) -> bool {
        if !self.active() { return false; }
        if dx.is_finite() && dy.is_finite() {
            self.displacement.0 = (self.displacement.0 + dx).clamp(-10_000.0, 10_000.0);
            self.displacement.1 = (self.displacement.1 + dy).clamp(-10_000.0, 10_000.0);
        }
        true
    }

    /// Pixels per tick, not per wakeup delay: a stalled loop never catches
    /// up with a burst of scrolling. ScrollTune owns fractional notches.
    pub fn tick(&mut self, now: Instant) -> Option<(f64, f64)> {
        let config = self.config?;
        if now < self.next? { return None; }
        self.next = Some(now + TICK);
        let rate = |distance: f64| {
            let outside = (distance.abs() - config.dead_zone).max(0.0);
            (outside / 20.0).powf(1.5).min(100.0) * config.speed * distance.signum()
        };
        let dx = if config.vertical_only { 0.0 } else { rate(self.displacement.0) };
        let dy = rate(self.displacement.1);
        (dx != 0.0 || dy != 0.0).then_some((dx, dy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_dead_zone_axes_and_stop() {
        let now = Instant::now();
        let mut state = Autoscroll::new(Some(Config::default()));
        assert_eq!(state.deadline(), None);
        assert!(state.key(0x112, true, now));
        assert!(state.key(0x112, false, now));
        assert!(state.motion(8.0, -8.0));
        assert_eq!(state.tick(now + TICK), None);
        state.motion(40.0, -40.0);
        let (dx, dy) = state.tick(now + TICK * 2).unwrap();
        assert!(dx > 0.0 && dy < 0.0);
        assert!(!state.key(1, true, now));
        assert_eq!(state.deadline(), None);
        assert_eq!(state.tick(now + TICK * 3), None);
        assert!(!state.motion(100.0, 100.0));
    }

    #[test]
    fn hostile_motion_and_vertical_only_are_bounded() {
        let now = Instant::now();
        let mut state = Autoscroll::new(Some(Config { vertical_only: true, ..Config::default() }));
        state.key(0x112, true, now);
        state.motion(f64::NAN, f64::INFINITY);
        assert_eq!(state.tick(now + TICK), None);
        state.motion(f64::MAX, f64::MAX);
        assert_eq!(state.tick(now + TICK * 2), Some((0.0, 100.0)));
        assert!(Config { speed: f64::NAN, ..Config::default() }.validate().is_err());
    }

    #[test]
    fn random_streams_never_tick_after_stop() {
        let now = Instant::now();
        let mut seed = 97u64;
        for _ in 0..100 {
            let mut state = Autoscroll::new(Some(Config::default()));
            for step in 0..1000 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let time = now + TICK * step;
                match seed % 5 {
                    0 => { state.key(0x112, seed & 8 != 0, time); }
                    1 => { state.motion((seed as i32 % 100) as f64, 50.0); }
                    2 => { state.stop(); assert_eq!(state.tick(time), None); }
                    _ => { state.tick(time); }
                }
                if !state.active() { assert_eq!(state.deadline(), None); }
            }
        }
    }
}
