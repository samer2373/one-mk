//! Shortcuts written for the computer that receives them.

use super::Target;
use crate::keynames::{Chord, LCTRL, LMETA, NameError};
use crate::os;
use std::{fmt, str::FromStr};

macro_rules! system_actions {
    ($($variant:ident, $name:literal, $label:literal => [$mac:expr, $windows:expr, $linux:expr];)+) => {
        /// An `os:NAME` shortcut, resolved when the keymap is built.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum SystemAction { $($variant),+ }

        impl SystemAction {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            pub const fn label(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }

            fn shortcuts(self) -> [Option<&'static str>; 3] {
                match self { $(Self::$variant => [$mac, $windows, $linux]),+ }
            }
        }
    }
}

system_actions! {
    NextDesktop, "next-desktop", "Next desktop" => [Some("ctrl+right"), Some("ctrl+super+right"), Some("ctrl+alt+right")];
    PreviousDesktop, "previous-desktop", "Previous desktop" => [Some("ctrl+left"), Some("ctrl+super+left"), Some("ctrl+alt+left")];
    Overview, "overview", "Overview: Mission Control / Task View / Activities" => [Some("ctrl+up"), Some("super+tab"), Some("super")];
    AppWindows, "app-windows", "App windows" => [Some("ctrl+down"), None, Some("super+grave")];
    ShowDesktop, "show-desktop", "Show desktop" => [Some("f11"), Some("super+d"), Some("super+d")];
    Launcher, "app-launcher", "App launcher" => [None, Some("super"), Some("super+a")];
    Search, "search", "System search" => [Some("super+space"), Some("super+s"), Some("super")];
    ScreenshotArea, "screenshot-area", "Screenshot area" => [Some("super+shift+4"), Some("super+shift+s"), Some("print")];
    ScreenshotTool, "screenshot-tool", "Screenshot tool" => [Some("super+shift+5"), Some("super+shift+s"), Some("print")];
    LockScreen, "lock-screen", "Lock screen" => [Some("ctrl+super+q"), Some("super+l"), Some("super+l")];
    ForceQuit, "force-quit", "Force Quit / Task Manager" => [Some("super+alt+esc"), Some("ctrl+shift+esc"), None];
    FullScreen, "full-screen", "Toggle full screen" => [Some("ctrl+super+f"), Some("f11"), Some("f11")];
    Emoji, "emoji-picker", "Emoji picker" => [Some("ctrl+super+space"), Some("super+dot"), None];
    SwitchApp, "switch-app", "Switch app" => [Some("super+tab"), Some("alt+tab"), Some("alt+tab")];
    HideApp, "hide-app", "Hide app" => [Some("super+h"), None, None];
    CloseWindow, "close-window", "Close window" => [Some("super+w"), Some("alt+f4"), Some("alt+f4")];
    QuitApp, "quit-app", "Quit app" => [Some("super+q"), None, None];
    Copy, "copy", "Copy" => [Some("super+c"), Some("ctrl+c"), Some("ctrl+c")];
    Cut, "cut", "Cut" => [Some("super+x"), Some("ctrl+x"), Some("ctrl+x")];
    Paste, "paste", "Paste" => [Some("super+v"), Some("ctrl+v"), Some("ctrl+v")];
    Undo, "undo", "Undo" => [Some("super+z"), Some("ctrl+z"), Some("ctrl+z")];
    Redo, "redo", "Redo" => [Some("super+shift+z"), Some("ctrl+y"), Some("ctrl+shift+z")];
    SelectAll, "select-all", "Select all" => [Some("super+a"), Some("ctrl+a"), Some("ctrl+a")];
    Find, "find", "Find" => [Some("super+f"), Some("ctrl+f"), Some("ctrl+f")];
    Save, "save", "Save" => [Some("super+s"), Some("ctrl+s"), Some("ctrl+s")];
    NewTab, "new-tab", "New tab" => [Some("super+t"), Some("ctrl+t"), Some("ctrl+t")];
    CloseTab, "close-tab", "Close tab" => [Some("super+w"), Some("ctrl+w"), Some("ctrl+w")];
    NextTab, "next-tab", "Next tab" => [Some("ctrl+tab"), Some("ctrl+tab"), Some("ctrl+tab")];
    PreviousTab, "previous-tab", "Previous tab" => [Some("ctrl+shift+tab"), Some("ctrl+shift+tab"), Some("ctrl+shift+tab")];
}

impl SystemAction {
    /// What the target receives, before compensating for any sender-side swap.
    pub fn shortcut(self, os: u8) -> Option<Chord> {
        let index = match os {
            os::MACOS => 0,
            os::WINDOWS => 1,
            os::LINUX => 2,
            _ => return None,
        };
        self.shortcuts()[index].map(|text| text.parse().expect("built-in system shortcut"))
    }

    pub(super) fn chord(self, target: Target) -> Option<Chord> {
        let mut chord = self.shortcut(target.os)?;
        if target.swap {
            let mods = chord.mods;
            chord.mods = (mods & !(LCTRL | LMETA)) | ((mods & LCTRL) << 3) | ((mods & LMETA) >> 3);
            chord.key = crate::swap_ctrl_meta(u32::from(chord.key)) as u16;
        }
        Some(chord)
    }

    /// The platforms for which system shortcuts can be defined.
    pub const TARGETS: [u8; 3] = [os::MACOS, os::WINDOWS, os::LINUX];

    /// A known target narrows the scope. An unspecified target covers all platforms.
    pub fn missing_on(self, target: u8) -> impl Iterator<Item = u8> {
        Self::TARGETS.into_iter().filter(move |&platform| {
            (!Self::TARGETS.contains(&target) || platform == target) && self.shortcut(platform).is_none()
        })
    }

    pub fn unavailable(self, os: u8, keeps_clicking: bool) -> Option<String> {
        self.shortcut(os).is_none().then(|| {
            let platform = match os {
                os::MACOS => "macOS",
                os::WINDOWS => "Windows",
                os::LINUX => "Linux",
                _ => "an unknown target OS",
            };
            let result = if keeps_clicking { "keeps the original click" } else { "sends nothing" };
            format!("{self} has no shortcut on {platform} and {result}")
        })
    }
}

impl fmt::Display for SystemAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "os:{}", self.name())
    }
}

impl FromStr for SystemAction {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, NameError> {
        let name = text.trim().split_once(':').filter(|(prefix, _)| prefix.eq_ignore_ascii_case("os"));
        if let Some((_, name)) = name
            && let Some(action) = Self::ALL.iter().find(|action| action.name().eq_ignore_ascii_case(name.trim()))
        {
            return Ok(*action);
        }
        Err(NameError(format!(
            "unknown system action {text:?}; try `os:overview` or see docs/INPUT.md for the action names"
        )))
    }
}
