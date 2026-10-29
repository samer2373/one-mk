//! Screensaver and sleep sync (`[power]` in `settings.toml`, set with
//! `one-mk power`).
//!
//! - `block_screensaver_on_peers`: while this computer controls a peer, the
//!   peer's screensaver, display blanking and idle sleep are kept off. The
//!   controller sends `Control::Inhibit` once the peer accepted the visit
//!   and withdraws it when the setting goes off; the peer holds the OS
//!   inhibitor only while the visit lasts, so it lets go when control ends
//!   (the pointer comes back, the peer's own input takes over, the lease
//!   runs out) and when the connection drops, whatever the controller does.
//! - `sleep_sync`: when this computer's display goes to sleep (or it
//!   suspends), connected computers with sleep sync on put their displays to
//!   sleep too. Waking is not synced. The decisions are `one_mk_proto::SleepSync`'s.
//!
//! Only peers that announced `caps::POWER` are sent either message.

use anyhow::Result;
use one_mk_proto::caps;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// `[power]` in `settings.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerSettings {
    /// Keep the screensaver and idle sleep off on a computer this one controls.
    pub block_screensaver_on_peers: bool,
    /// Put displays to sleep with this one (off by default).
    pub sleep_sync: bool,
}

impl PowerSettings {
    pub fn describe(&self) -> String {
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "Keep the screensaver and sleep off on computers this one controls: {}.\nSleep sync (displays sleep together): {}.",
            on(self.block_screensaver_on_peers),
            on(self.sleep_sync)
        )
    }
}

/// What the controller should tell the peer it controls: `Some(on)` if the
/// peer must be told something new. `sent` is what it was last told (this
/// visit).
pub fn inhibit_request(setting: bool, accepted: bool, peer_caps: u32, sent: bool) -> Option<bool> {
    let want = setting && accepted && peer_caps & caps::POWER != 0;
    (want != sent).then_some(want)
}

/// Holds the OS inhibitor while wanted. A failed attempt is not repeated
/// until the want goes away (once a visit), so a system that can't do it
/// is asked once and not on every event.
#[derive(Default)]
pub struct Awake {
    guard: Option<Box<dyn Send>>,
    failed: bool,
}

impl Awake {
    #[cfg(test)]
    pub fn is_held(&self) -> bool {
        self.guard.is_some()
    }

    /// Takes the inhibitor if `want` and not yet held, lets go if not `want`.
    pub fn set(&mut self, want: bool, inhibit: impl FnOnce() -> Result<Box<dyn Send>>) {
        if !want {
            self.guard = None;
            self.failed = false;
        } else if self.guard.is_none() && !self.failed {
            match inhibit() {
                Ok(g) => self.guard = Some(g),
                Err(e) => {
                    self.failed = true;
                    tracing::warn!("could not keep the screen awake for the computer controlling this one: {e:#}");
                }
            }
        }
    }
}

/// `one-mk power`: shows the settings, or changes them.
#[derive(clap::Args, Debug, Default)]
pub struct PowerArgs {
    /// Keep the screensaver and sleep off on computers this one controls.
    #[arg(long, value_name = "on|off", value_parser = crate::settings::parse_on_off, action = clap::ArgAction::Set)]
    pub block_screensaver: Option<bool>,
    /// Put the displays of connected computers to sleep when this one's
    /// sleeps (and follow theirs).
    #[arg(long, value_name = "on|off", value_parser = crate::settings::parse_on_off, action = clap::ArgAction::Set)]
    pub sleep_sync: Option<bool>,
}

pub async fn run_command(dir: &Path, args: PowerArgs) -> Result<()> {
    use crate::settings::Settings;
    let changes = args.block_screensaver.is_some() || args.sleep_sync.is_some();
    let power = if changes {
        Settings::update(dir, |s| {
            if let Some(b) = args.block_screensaver {
                s.power.block_screensaver_on_peers = b;
            }
            if let Some(b) = args.sleep_sync {
                s.power.sleep_sync = b;
            }
            Ok(s.power.clone())
        })?
    } else {
        Settings::load(dir)?.power
    };
    println!("{}", power.describe());
    if changes {
        if crate::control::request(dir, crate::control::Request::Reload).await?.is_some() {
            println!("Applied right away.");
        } else {
            println!("one-mk is not running; this takes effect the next time it starts.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_peer_is_told_only_what_changed() {
        let p = caps::POWER;
        // Setting on, visit accepted, peer able: tell it once.
        assert_eq!(inhibit_request(true, true, p, false), Some(true));
        assert_eq!(inhibit_request(true, true, p, true), None);
        // Before the peer accepted, or without the capability: nothing.
        assert_eq!(inhibit_request(true, false, p, false), None);
        assert_eq!(inhibit_request(true, true, caps::LOCK, false), None);
        assert_eq!(inhibit_request(true, true, 0, false), None);
        // Turned off mid visit: withdraw it.
        assert_eq!(inhibit_request(false, true, p, true), Some(false));
        assert_eq!(inhibit_request(false, true, p, false), None);
    }

    #[test]
    fn the_inhibitor_is_held_exactly_while_wanted() {
        let held = Arc::new(AtomicUsize::new(0));
        let tries = Arc::new(AtomicUsize::new(0));
        struct Hold(Arc<AtomicUsize>);
        impl Drop for Hold {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let take = |ok: bool| {
            let (held, tries) = (held.clone(), tries.clone());
            move || -> Result<Box<dyn Send>> {
                tries.fetch_add(1, Ordering::SeqCst);
                anyhow::ensure!(ok, "no");
                held.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Hold(held)))
            }
        };
        let mut a = Awake::default();
        a.set(false, take(true));
        assert_eq!(held.load(Ordering::SeqCst), 0);
        a.set(true, take(true));
        a.set(true, take(true));
        assert_eq!((held.load(Ordering::SeqCst), tries.load(Ordering::SeqCst)), (1, 1), "taken once");
        a.set(false, take(true));
        assert_eq!(held.load(Ordering::SeqCst), 0, "released");
        // A system that can't is asked once per want, not per event.
        tries.store(0, Ordering::SeqCst);
        a.set(true, take(false));
        a.set(true, take(false));
        assert_eq!((tries.load(Ordering::SeqCst), a.is_held()), (1, false));
        a.set(false, take(false));
        a.set(true, take(false));
        assert_eq!(tries.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn settings_default_to_off_and_round_trip() {
        let s: PowerSettings = toml::from_str("").unwrap();
        assert_eq!(s, PowerSettings::default());
        assert!(!s.block_screensaver_on_peers && !s.sleep_sync);
        let s: PowerSettings = toml::from_str("sleep_sync = true").unwrap();
        assert!(s.sleep_sync && !s.block_screensaver_on_peers);
        assert_eq!(toml::from_str::<PowerSettings>(&toml::to_string(&s).unwrap()).unwrap(), s);
    }
}
