//! What a desktop can tell oneMK about the focused window, so the controls
//! that depend on it can say so where they are instead of silently doing
//! nothing. GNOME and KDE on Wayland have no way to ask which window has the
//! focus (on purpose), and macOS fullscreen windows are not probed yet; see
//! `crates/input/src/system/{linux,macos}.rs` for what the service does.

use one_mk_proto::os;

/// This computer, as far as the focus probe is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub os: u8,
    /// A Wayland session (Linux only).
    pub wayland: bool,
}

impl Session {
    /// This session, looked at once.
    pub fn current() -> Self {
        static SESSION: std::sync::OnceLock<Session> = std::sync::OnceLock::new();
        *SESSION.get_or_init(Self::detect)
    }

    /// Looks at the running session. The window runs in it, so the
    /// environment it sees is the desktop's.
    pub fn detect() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self::from_env(os::local(), var("XDG_SESSION_TYPE").as_deref(), var("WAYLAND_DISPLAY").is_some())
    }

    fn from_env(os: u8, session_type: Option<&str>, wayland_display: bool) -> Self {
        let wayland = match session_type {
            Some(kind) => kind.eq_ignore_ascii_case("wayland"),
            None => wayland_display,
        };
        Self { os, wayland: os == os::LINUX && wayland }
    }

    /// Why "Stay in fullscreen windows" can miss a fullscreen window on this
    /// computer; `None` where it works as described.
    pub fn fullscreen_note(self) -> Option<&'static str> {
        match self.os {
            _ if self.wayland => Some("On Wayland only programs that run through XWayland (most games) are seen as fullscreen. A native Wayland window is not, so the pointer can still cross."),
            os::MACOS => Some("Fullscreen windows are not detected on macOS yet, so this does not stop the pointer from crossing."),
            _ => None,
        }
    }
}

/// Whether a computer running `os` may be unable to say which app has the
/// focus. Linux on GNOME or KDE with Wayland can't (the window can't see
/// the session of another computer, so any Linux one is flagged).
pub fn may_not_report_app(os: u8) -> bool {
    os == os::LINUX
}

/// The note under "Only in app": what the daemon says right now about rules
/// that are not in force (`tuning`, from the status; exact, but only known
/// once a computer was controlled), else a caution when one of the
/// computers the rule is for runs Linux. `peer_os` is the OS of each.
pub fn app_rule_note(tuning: &[String], peer_os: &[u8]) -> Option<String> {
    let live: Vec<&str> = tuning.iter().map(String::as_str).filter(|line| line.starts_with("Rule for app") || line.starts_with("Rules for apps")).collect();
    if let Some(first) = live.first() {
        let more = if live.len() > 1 { format!(" ({} more computers say the same.)", live.len() - 1) } else { String::new() };
        return Some(format!("{first}{more}"));
    }
    peer_os.iter().any(|o| may_not_report_app(*o)).then(|| {
        "Linux computers on GNOME or KDE (Wayland) can't report which app has the focus, so a rule for an app does not apply to their Wayland windows.".to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_comes_from_the_environment() {
        assert!(Session::from_env(os::LINUX, Some("wayland"), false).wayland);
        assert!(Session::from_env(os::LINUX, Some("Wayland"), false).wayland);
        assert!(!Session::from_env(os::LINUX, Some("x11"), true).wayland, "an X11 session wins over a stray display variable");
        assert!(Session::from_env(os::LINUX, None, true).wayland);
        assert!(!Session::from_env(os::LINUX, None, false).wayland);
        assert!(!Session::from_env(os::WINDOWS, Some("wayland"), true).wayland, "Wayland is a Linux thing");
    }

    #[test]
    fn the_fullscreen_guard_says_what_this_desktop_cannot_do() {
        let note = |os, wayland| Session { os, wayland }.fullscreen_note();
        assert!(note(os::LINUX, true).is_some_and(|n| n.contains("XWayland")));
        assert!(note(os::MACOS, false).is_some_and(|n| n.contains("macOS")));
        assert_eq!(note(os::LINUX, false), None, "X11 sees its fullscreen windows");
        assert_eq!(note(os::WINDOWS, false), None);
    }

    #[test]
    fn a_rule_for_an_app_warns_only_where_it_may_not_fire() {
        assert_eq!(app_rule_note(&[], &[os::WINDOWS, os::MACOS]), None);
        assert_eq!(app_rule_note(&[], &[]), None);
        let caution = app_rule_note(&[], &[os::MACOS, os::LINUX]).unwrap();
        assert!(caution.contains("Wayland"), "{caution}");
    }

    #[test]
    fn what_the_daemon_says_replaces_the_caution() {
        let said = vec![
            "Rule for app firefox not active: Studio does not report its focused app (it can't tell which app has the focus right now).".to_owned(),
            "Rules for one keyboard or mouse are not in force: x.".to_owned(),
        ];
        let note = app_rule_note(&said, &[os::LINUX]).unwrap();
        assert!(note.starts_with("Rule for app firefox not active"), "{note}");
        assert!(!note.contains("one keyboard"), "{note}");
        // Other lines in the status are not about apps: no note, no caution on Windows.
        assert_eq!(app_rule_note(&said[1..], &[os::WINDOWS]), None);
    }
}
