//! Whether to draw our own pointer while a peer controls this computer.
//!
//! A computer with no mouse attached (a laptop used with the lid closed, a
//! headless box, a tablet) often draws no pointer at all: Windows starts
//! the cursor's display count at -1 when no mouse is installed, so even
//! injected motion moves an invisible cursor. The person controlling it
//! from another computer then can't see where the pointer is. Where that
//! happens, one-mk draws a small arrow of its own at the injected position
//! (see `docs/NO-MOUSE-POINTER.md`).
//!
//! This module only decides; the drawing is in the input crate.

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// `[pointer] draw_when_no_mouse` in settings.toml.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawMode {
    /// Draw only when the OS doesn't (no mouse attached, cursor hidden).
    #[default]
    Auto,
    /// Draw whenever a peer controls this computer, where possible.
    Always,
    /// Never draw.
    Never,
}

impl DrawMode {
    pub const ALL: [DrawMode; 3] = [DrawMode::Auto, DrawMode::Always, DrawMode::Never];

    pub fn as_str(self) -> &'static str {
        match self {
            DrawMode::Auto => "auto",
            DrawMode::Always => "always",
            DrawMode::Never => "never",
        }
    }
}

impl fmt::Display for DrawMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DrawMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        DrawMode::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| format!("`{s}`: expected auto, always or never"))
    }
}

/// What the OS does with its own pointer right now, as far as a probe can
/// tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OsCursor {
    /// The OS draws it (X11 always does).
    Shown,
    /// The OS says it isn't drawing it because the input isn't coming from
    /// a mouse (Windows 8+: `CURSOR_SUPPRESSED`, after touch or pen input).
    Suppressed,
    /// It isn't drawn and the probe can't tell why: no mouse (Windows'
    /// display count starts at -1 without one), or an application hid it
    /// (a video player, a game).
    Hidden,
    /// No probe on this OS, or it failed.
    #[default]
    Unknown,
}

/// What the decision is made from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// A peer controls this computer.
    pub controlled: bool,
    /// The peer sends raw motion (gaming mode): there is no position to
    /// draw at, and a game draws its own pointer or none.
    pub relative: bool,
    /// A real pointing device (mouse, touchpad) is attached; `None` where
    /// the OS can't tell us.
    pub pointing_device: Option<bool>,
    pub os_cursor: OsCursor,
    /// This desktop lets us draw a pointer at all (not on Wayland).
    pub can_draw: bool,
}

/// Whether the pointer should be drawn, from these facts alone.
pub fn wants(mode: DrawMode, f: &Facts) -> bool {
    if !f.controlled || !f.can_draw || f.relative {
        return false;
    }
    match mode {
        DrawMode::Never => false,
        DrawMode::Always => true,
        DrawMode::Auto => match (f.os_cursor, f.pointing_device) {
            (OsCursor::Shown, _) => false,
            (OsCursor::Suppressed, _) => true,
            // Not drawn, or no way to know, and no mouse: draw.
            (OsCursor::Hidden | OsCursor::Unknown, Some(false)) => true,
            // A mouse is attached (so an application hid the pointer on
            // purpose), or nothing is known: leave it to the OS.
            _ => false,
        },
    }
}

/// How long a change that comes only from the pointer probe must last
/// before the overlay follows it. The probe can flicker (a hide-while-
/// typing setting, an application changing cursors); the overlay must not.
pub const SETTLE: Duration = Duration::from_millis(750);

/// [`wants`] with hysteresis. Changes of the discrete facts (control
/// starting or ending, gaming mode, the setting, a mouse plugged in or
/// out, the renderer going away) apply at once; a change that only the
/// pointer probe sees applies once it has held for [`SETTLE`]. Driven by
/// the caller's observations (each injected move): no timer.
#[derive(Clone, Debug, Default)]
pub struct Decider {
    on: bool,
    last: Option<(DrawMode, bool, bool, bool, Option<bool>)>,
    /// Since when the probe has wanted the other state.
    flip_since: Option<Instant>,
}

impl Decider {
    /// Whether to draw now.
    pub fn update(&mut self, mode: DrawMode, f: &Facts, now: Instant) -> bool {
        let want = wants(mode, f);
        let discrete = (mode, f.controlled, f.relative, f.can_draw, f.pointing_device);
        let changed = self.last.replace(discrete) != Some(discrete);
        if want == self.on {
            self.flip_since = None;
        } else if changed {
            self.on = want;
            self.flip_since = None;
        } else {
            match self.flip_since {
                Some(t) if now.saturating_duration_since(t) >= SETTLE => {
                    self.on = want;
                    self.flip_since = None;
                }
                Some(_) => {}
                None => self.flip_since = Some(now),
            }
        }
        self.on
    }

    /// The last decision.
    pub fn is_on(&self) -> bool {
        self.on
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controlled(device: Option<bool>, os_cursor: OsCursor) -> Facts {
        Facts { controlled: true, relative: false, pointing_device: device, os_cursor, can_draw: true }
    }

    #[test]
    fn modes_parse_and_print() {
        for m in DrawMode::ALL {
            assert_eq!(m.to_string().parse::<DrawMode>(), Ok(m));
        }
        assert_eq!(" Always ".parse::<DrawMode>(), Ok(DrawMode::Always));
        assert!("sometimes".parse::<DrawMode>().is_err());
        assert_eq!(DrawMode::default(), DrawMode::Auto);
    }

    /// The auto rule, case by case.
    #[test]
    fn auto_draws_only_when_the_os_does_not() {
        use OsCursor::*;
        let auto = |d, c| wants(DrawMode::Auto, &controlled(d, c));
        // The OS draws it: never ours, whatever is attached.
        for d in [Some(true), Some(false), None] {
            assert!(!auto(d, Shown), "{d:?}");
            // Windows says it hides it for touch/pen input: always ours.
            assert!(auto(d, Suppressed), "{d:?}");
        }
        // No mouse and no pointer (or no probe): ours.
        assert!(auto(Some(false), Hidden));
        assert!(auto(Some(false), Unknown));
        // A mouse is attached: a hidden pointer was hidden on purpose.
        assert!(!auto(Some(true), Hidden));
        assert!(!auto(Some(true), Unknown));
        // Nothing known: leave it to the OS.
        assert!(!auto(None, Hidden));
        assert!(!auto(None, Unknown));
    }

    #[test]
    fn always_and_never() {
        let shown = controlled(Some(true), OsCursor::Shown);
        assert!(wants(DrawMode::Always, &shown));
        assert!(!wants(DrawMode::Never, &controlled(Some(false), OsCursor::Hidden)));
    }

    /// Nothing is drawn unless a peer controls us with positions, and the
    /// desktop allows drawing, whatever the setting.
    #[test]
    fn preconditions_win_over_the_setting() {
        let base = controlled(Some(false), OsCursor::Hidden);
        for mode in DrawMode::ALL {
            assert!(!wants(mode, &Facts { controlled: false, ..base }), "{mode}");
            assert!(!wants(mode, &Facts { relative: true, ..base }), "{mode}");
            assert!(!wants(mode, &Facts { can_draw: false, ..base }), "{mode}");
        }
    }

    /// Control starting turns it on at once; the probe flickering doesn't
    /// turn it off; a steady change does, after `SETTLE`.
    #[test]
    fn probe_changes_settle() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut d = Decider::default();
        let hidden = controlled(Some(false), OsCursor::Hidden);
        let shown = controlled(Some(false), OsCursor::Shown);
        assert!(!d.update(DrawMode::Auto, &Facts::default(), t0));
        assert!(d.update(DrawMode::Auto, &hidden, ms(0)), "on when control starts");
        // Flicker: shown for a moment, then hidden again.
        assert!(d.update(DrawMode::Auto, &shown, ms(100)));
        assert!(d.update(DrawMode::Auto, &hidden, ms(200)));
        assert!(d.update(DrawMode::Auto, &shown, ms(300)), "the clock restarts");
        assert!(d.update(DrawMode::Auto, &shown, ms(300 + 749)));
        assert!(!d.update(DrawMode::Auto, &shown, ms(300 + 750)), "off once steady");
        // And back on the same way.
        assert!(!d.update(DrawMode::Auto, &hidden, ms(2000)));
        assert!(d.update(DrawMode::Auto, &hidden, ms(2000 + 750)));
    }

    /// Discrete changes apply at once.
    #[test]
    fn discrete_changes_are_immediate() {
        let t = Instant::now();
        let mut d = Decider::default();
        let no_mouse = controlled(Some(false), OsCursor::Hidden);
        assert!(d.update(DrawMode::Auto, &no_mouse, t));
        // A mouse is plugged in (the OS cursor shows soon after).
        assert!(!d.update(DrawMode::Auto, &controlled(Some(true), OsCursor::Hidden), t));
        assert!(d.update(DrawMode::Auto, &no_mouse, t), "unplugged");
        assert!(!d.update(DrawMode::Never, &no_mouse, t), "setting");
        assert!(d.update(DrawMode::Always, &controlled(Some(true), OsCursor::Shown), t));
        assert!(!d.update(DrawMode::Always, &Facts { relative: true, ..no_mouse }, t), "gaming mode");
        assert!(d.update(DrawMode::Always, &no_mouse, t));
        assert!(!d.update(DrawMode::Always, &Facts { controlled: false, ..no_mouse }, t), "control ends");
        assert!(!d.is_on());
    }
}
