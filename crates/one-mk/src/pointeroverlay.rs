//! The pointer one-mk draws on a computer with no mouse (plan 3A):
//! `[pointer] draw_when_no_mouse` in settings.toml, `one-mk
//! pointer-overlay [auto|always|never]`, and what `one-mk doctor` says
//! about it. The drawing itself is `one_mk_input::cursoroverlay`; the
//! findings behind it are in `docs/NO-MOUSE-POINTER.md`.

use crate::settings::Settings;
use anyhow::Result;
use one_mk_input::cursoroverlay::{self, DrawMode, Support};
use one_mk_input::{Inject, InjectTx};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::Path;
use tokio::sync::mpsc;

/// `[pointer]` in settings.toml.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointerSettings {
    /// `auto` (draw when the OS shows no pointer for lack of a mouse),
    /// `always` or `never`.
    #[serde(serialize_with = "write_mode", deserialize_with = "read_mode")]
    pub draw_when_no_mouse: DrawMode,
}

fn write_mode<S: Serializer>(m: &DrawMode, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(m.as_str())
}

fn read_mode<'de, D: Deserializer<'de>>(d: D) -> Result<DrawMode, D::Error> {
    String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
}

/// The daemon's sender of injected input for the backend called `backend`.
pub fn inject_tx(tx: mpsc::UnboundedSender<Inject>, backend: &str, settings: &Settings) -> InjectTx {
    InjectTx::new(tx, cursoroverlay::platform(backend), settings.pointer.draw_when_no_mouse)
}

/// What drawing the pointer means here: with the running daemon's backend
/// if known, else the session's.
pub fn support_here(running_backend: Option<&str>) -> Support {
    let guess = if cfg!(target_os = "linux") {
        let set = |v| std::env::var_os(v).is_some_and(|d| !d.is_empty());
        if set("WAYLAND_DISPLAY") {
            "portal"
        } else if set("DISPLAY") {
            "x11"
        } else {
            ""
        }
    } else {
        "native"
    };
    cursoroverlay::support(running_backend.filter(|b| !b.is_empty()).unwrap_or(guess))
}

/// In words, for `one-mk pointer-overlay` and `one-mk doctor`.
pub fn describe(support: Support, mode: DrawMode) -> String {
    match (support, mode) {
        (Support::NoInput, _) => "the null input backend draws nothing".into(),
        (_, DrawMode::Never) => "off: the pointer is never drawn by one-mk".into(),
        (Support::Available, DrawMode::Auto) => {
            "available: drawn while another computer controls this one and it shows no pointer of its own (no mouse attached)"
                .into()
        }
        (Support::Available, DrawMode::Always) => "available: drawn whenever another computer controls this one".into(),
        (Support::NotNeeded, DrawMode::Auto) => "not needed: X11 always draws the pointer, mouse or not".into(),
        (Support::NotNeeded, DrawMode::Always) => {
            "not needed (X11 always draws the pointer), but drawn anyway while another computer controls this one, as set"
                .into()
        }
        (Support::NotAvailable, _) => {
            "not available on this desktop: on Wayland only the desktop can draw a pointer (it normally shows the one one-mk moves)"
                .into()
        }
    }
}

/// `one-mk pointer-overlay [auto|always|never]`.
pub async fn command(dir: &Path, mode: Option<&str>) -> Result<()> {
    let Some(mode) = mode else {
        let mode = Settings::load(dir)?.pointer.draw_when_no_mouse;
        let running = crate::control::request(dir, crate::control::Request::Status)
            .await
            .ok()
            .flatten()
            .and_then(|r| one_mk_ctl::status::Status::decode(&r));
        let backend = running.as_ref().map(|s| s.backend.as_str());
        println!("Drawing the pointer when this computer has no mouse: {mode}.");
        println!("Here: {}.", describe(support_here(backend), mode));
        if running.is_some_and(|s| s.pointer_overlay) {
            println!("It is drawn right now.");
        }
        return Ok(());
    };
    let mode: DrawMode = mode.parse().map_err(anyhow::Error::msg)?;
    Settings::update(dir, |s| {
        s.pointer.draw_when_no_mouse = mode;
        Ok(())
    })?;
    println!(
        "{}",
        match mode {
            DrawMode::Auto => "one-mk draws the pointer while another computer controls this one and it shows none (no mouse attached).",
            DrawMode::Always => "one-mk draws the pointer whenever another computer controls this one.",
            DrawMode::Never => "one-mk never draws the pointer.",
        }
    );
    println!("Here: {}.", describe(support_here(None), mode));
    crate::settings::apply(dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_round_trip() {
        let s: Settings = toml::from_str("").unwrap();
        assert_eq!(s.pointer.draw_when_no_mouse, DrawMode::Auto);
        let s: Settings = toml::from_str("[pointer]\ndraw_when_no_mouse = \"always\"\n").unwrap();
        assert_eq!(s.pointer.draw_when_no_mouse, DrawMode::Always);
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("[pointer]\ndraw_when_no_mouse = \"always\""), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap().pointer, s.pointer);
        let bad = toml::from_str::<Settings>("[pointer]\ndraw_when_no_mouse = \"sometimes\"\n").unwrap_err();
        assert!(bad.to_string().contains("expected auto, always or never"), "{bad}");
    }

    #[test]
    fn descriptions() {
        assert!(describe(Support::NotAvailable, DrawMode::Always).starts_with("not available on this desktop"));
        assert!(describe(Support::NotNeeded, DrawMode::Auto).starts_with("not needed"));
        assert!(describe(Support::Available, DrawMode::Auto).starts_with("available"));
        assert!(describe(Support::Available, DrawMode::Never).starts_with("off"));
        assert_eq!(support_here(Some("wlroots")), Support::NotAvailable);
        assert_eq!(support_here(Some("x11")), Support::NotNeeded);
    }
}
