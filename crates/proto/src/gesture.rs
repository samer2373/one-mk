//! Held-button gestures. No timer or I/O; cancellation never replays clicks.

use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Up, Down, Left, Right }

impl FromStr for Direction {
    type Err = &'static str;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "up" => Ok(Self::Up), "down" => Ok(Self::Down),
            "left" => Ok(Self::Left), "right" => Ok(Self::Right),
            _ => Err("choose a gesture direction: up, down, left or right"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule<T> {
    pub button: u32,
    pub direction: Direction,
    pub threshold_px: f64,
    pub output: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonResult { Pass, Swallow, Replay(u32) }

#[derive(Clone, Debug)]
struct Press { button: u32, displacement: (f64, f64), fired: bool, moved: bool }

#[derive(Clone, Debug)]
pub struct Gesture<T> {
    rules: Vec<Rule<T>>,
    presses: Vec<Press>,
}

impl<T> Gesture<T> {
    pub fn new(rules: Vec<Rule<T>>) -> Self {
        Self { rules: rules.into_iter().filter(|rule| crate::keynames::is_button(rule.button) && !matches!(rule.button, 0x110 | 0x111)
            && rule.threshold_px.is_finite() && (1.0..=10_000.0).contains(&rule.threshold_px)).collect(), presses: Vec::new() }
    }

    pub fn cancel(&mut self) { self.presses.clear(); }

    pub fn rules(&self) -> &[Rule<T>] { &self.rules }

    pub fn active(&self) -> bool { !self.presses.is_empty() }

    pub fn button(&mut self, button: u32, down: bool) -> ButtonResult {
        if let Some(index) = self.presses.iter().position(|press| press.button == button) {
            if down { return ButtonResult::Swallow; }
            let press = self.presses.remove(index);
            return if press.fired || press.moved { ButtonResult::Swallow } else { ButtonResult::Replay(button) };
        }
        if !self.rules.iter().any(|rule| rule.button == button) { return ButtonResult::Pass; }
        if down { self.presses.push(Press { button, displacement: (0.0, 0.0), fired: false, moved: false }); }
        ButtonResult::Swallow
    }

    /// Returns whether motion was consumed, firing at most once per press.
    pub fn motion(&mut self, dx: f64, dy: f64, fire: &mut impl FnMut(&T)) -> bool {
        for press in &mut self.presses {
            if press.fired || !dx.is_finite() || !dy.is_finite() { continue; }
            press.displacement.0 = (press.displacement.0 + dx).clamp(-100_000.0, 100_000.0);
            press.displacement.1 = (press.displacement.1 + dy).clamp(-100_000.0, 100_000.0);
            let (horizontal, vertical) = press.displacement;
            let slop = self.rules.iter().filter(|rule| rule.button == press.button)
                .map(|rule| (rule.threshold_px / 2.0).min(16.0)).fold(16.0, f64::min);
            press.moved |= horizontal.abs().max(vertical.abs()) >= slop;
            let direction = if horizontal.abs() > vertical.abs() {
                if horizontal < 0.0 { Direction::Left } else { Direction::Right }
            } else if vertical < 0.0 { Direction::Up } else { Direction::Down };
            if let Some(rule) = self.rules.iter().find(|rule| rule.button == press.button && rule.direction == direction
                && horizontal.abs().max(vertical.abs()) >= rule.threshold_px) {
                press.fired = true;
                fire(&rule.output);
            }
        }
        !self.presses.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primary_buttons_are_never_reserved() {
        for button in [0x110, 0x111] {
            let mut gesture = Gesture::new(vec![Rule { button, direction: Direction::Left, threshold_px: 60.0, output: () }]);
            assert_eq!(gesture.button(button, true), ButtonResult::Pass);
        }
    }

    #[test]
    fn missed_swipes_never_replay_even_after_returning_to_origin() {
        for (horizontal, vertical) in [(-55.0, 0.0), (0.0, -500.0), (-20.0, 0.0)] {
            let mut gesture = state();
            gesture.button(0x113, true);
            gesture.motion(horizontal, vertical, &mut |_| panic!("unexpected gesture"));
            gesture.motion(-horizontal, -vertical, &mut |_| panic!("unexpected gesture"));
            assert_eq!(gesture.button(0x113, false), ButtonResult::Swallow);
        }
    }
    fn state() -> Gesture<()> {
        Gesture::new(vec![Rule { button: 0x113, direction: Direction::Left, threshold_px: 60.0, output: () }])
    }
    #[test]
    fn click_or_action_never_both_and_cancellation_never_replays() {
        let mut state = state();
        let mut fired = 0;
        assert_eq!(state.button(0x113, true), ButtonResult::Swallow);
        assert!(state.motion(-5.0, 0.0, &mut |_| fired += 1));
        assert_eq!(state.button(0x113, false), ButtonResult::Replay(0x113));
        state.button(0x113, true);
        state.motion(-60.0, 0.0, &mut |_| fired += 1);
        state.motion(-600.0, 0.0, &mut |_| fired += 1);
        assert_eq!(state.button(0x113, false), ButtonResult::Swallow);
        assert_eq!(fired, 1);
        state.button(0x113, true);
        state.cancel();
        assert_eq!(state.button(0x113, false), ButtonResult::Swallow);
        assert!(!state.motion(-600.0, 0.0, &mut |_| fired += 1));
        assert_eq!(fired, 1);
    }
    #[test]
    fn every_direction_threshold_and_simultaneous_buttons() {
        for (direction, horizontal, vertical) in [(Direction::Left, -60.0, 0.0), (Direction::Right, 60.0, 0.0),
            (Direction::Up, 0.0, -60.0), (Direction::Down, 0.0, 60.0)] {
            let mut state = Gesture::new(vec![Rule { button: 0x113, direction, threshold_px: 60.0, output: () },
                Rule { button: 0x114, direction, threshold_px: 60.0, output: () }]);
            state.button(0x113, true);
            state.button(0x114, true);
            let mut fired = 0;
            state.motion(horizontal * 0.99, vertical * 0.99, &mut |_| fired += 1);
            assert_eq!(fired, 0);
            state.motion(horizontal * 0.02, vertical * 0.02, &mut |_| fired += 1);
            assert_eq!(fired, 2);
            state.motion(horizontal, vertical, &mut |_| fired += 1);
            assert_eq!(fired, 2);
            assert_eq!(state.button(0x113, false), ButtonResult::Swallow);
            assert_eq!(state.button(0x114, false), ButtonResult::Swallow);
            assert!(!state.motion(horizontal, vertical, &mut |_| panic!("released gesture fired")));
        }
    }

    #[test]
    fn random_streams_balance_replay_and_fire_once() {
        let mut seed = 123u64;
        for _ in 0..100 {
            let mut state = state();
            let mut pressed = false;
            let mut fired = 0;
            for _ in 0..1000 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                match seed % 5 {
                    0 => { state.button(0x113, true); pressed = true; fired = 0; }
                    1 => {
                        let result = state.button(0x113, false);
                        if result == ButtonResult::Replay(0x113) { assert!(pressed && fired == 0); }
                        pressed = false;
                    }
                    2 => { state.cancel(); pressed = false; fired = 0; }
                    _ => {
                        state.motion(-30.0, 1.0, &mut |_| { assert!(pressed); fired += 1; });
                        assert!(fired <= 1);
                    }
                }
            }
        }
    }
}
