//! Per-application rules: which program has the focus on the computer
//! input goes to, and whether a rule's `app` names it.
//!
//! The controlled computer says what its focused window is called (see
//! [`Control::Focus`](crate::Control::Focus)), as many names as its OS
//! gives, and a rule's [`AppPattern`] matches if it names any of them:
//!
//! | | names reported |
//! |---|---|
//! | Windows | the program file (`chrome.exe`) and the window class |
//! | macOS | the bundle id (`com.apple.Terminal`) and the app's name (`Terminal`) |
//! | Linux, X11 and XWayland windows | the window's `WM_CLASS`, both parts (`code`, `Code`) |
//! | Linux, Wayland-native windows | nothing: a background program can't tell |
//!
//! Matching ignores case, and `.exe`/`.app` may be left off (`chrome`
//! names `chrome.exe`). `*` stands for any run of characters
//! (`com.jetbrains.*`); a leading `!` turns the pattern around (every app
//! but that one). When the app isn't known nothing matches, negated or not:
//! a per-app rule only ever applies while the app is known to be right.
//!
//! Names come from another computer, so they are cleaned before use
//! ([`clean_names`]); patterns come from the settings file and are refused
//! when they could not be meant ([`AppPattern::from_str`]).

use crate::os;
use std::fmt;
use std::str::FromStr;

/// The longest app name or pattern, in characters.
pub const MAX_APP_LEN: usize = 128;
/// The most names one focused window is reported by.
pub const MAX_APP_NAMES: usize = 4;

/// Characters that never belong in an app name: control characters and the
/// invisible ones that could disguise one (zero-width, bidi overrides).
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
        )
}

/// One name a peer reported, made safe to use and show: invisible and
/// control characters removed, trimmed, at most [`MAX_APP_LEN`] characters.
/// `None` when nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let kept: String = name.chars().filter(|c| !invisible(*c)).take(MAX_APP_LEN * 2).collect();
    let trimmed: String = kept.trim().chars().take(MAX_APP_LEN).collect();
    let trimmed = trimmed.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The names of the focused window, cleaned: no empty ones, no
/// duplicates (ignoring case), at most [`MAX_APP_NAMES`]. Empty means the
/// app is unknown.
pub fn clean_names<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if out.len() == MAX_APP_NAMES {
            break;
        }
        if let Some(n) = clean_name(n.as_ref())
            && !out.iter().any(|o| o.to_lowercase() == n.to_lowercase())
        {
            out.push(n);
        }
    }
    out
}

/// A rule's `app`: a name or glob, maybe negated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPattern {
    negate: bool,
    /// Lowercase, runs of `*` collapsed into one.
    glob: Vec<char>,
    /// As written (trimmed), for messages.
    text: String,
}

impl FromStr for AppPattern {
    type Err = String;

    /// Refuses what could not be meant: nothing, control or invisible
    /// characters, more than [`MAX_APP_LEN`] characters, a folder (only
    /// names are reported), and `!*` (no app at all).
    fn from_str(s: &str) -> Result<Self, String> {
        let text = s.trim();
        if s.chars().any(invisible) {
            return Err("app: control or invisible characters aren't allowed".into());
        }
        if text.chars().count() > MAX_APP_LEN {
            return Err(format!("app: at most {MAX_APP_LEN} characters"));
        }
        let (negate, rest) = match text.strip_prefix('!') {
            Some(r) => (true, r.trim_start()),
            None => (false, text),
        };
        if rest.is_empty() {
            return Err("app: give a program name, like chrome.exe or com.apple.Terminal".into());
        }
        if rest.contains(['/', '\\']) {
            return Err(format!("app {rest:?}: give the program's name without its folder (chrome.exe, not a path)"));
        }
        let mut glob: Vec<char> = Vec::new();
        for c in rest.chars().flat_map(char::to_lowercase) {
            if !(c == '*' && glob.last() == Some(&'*')) {
                glob.push(c);
            }
        }
        if negate && glob == ['*'] {
            return Err("app \"!*\" matches no app at all".into());
        }
        Ok(AppPattern { negate, glob, text: text.to_owned() })
    }
}

impl fmt::Display for AppPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl AppPattern {
    /// Whether the pattern says which app (`!` patterns say which not).
    pub fn is_negated(&self) -> bool {
        self.negate
    }

    /// Whether a rule with this pattern applies while the focused window
    /// goes by `names` (empty: unknown, and then never).
    pub fn matches<S: AsRef<str>>(&self, names: &[S]) -> bool {
        if names.is_empty() {
            return false;
        }
        let any = names.iter().any(|n| self.names(n.as_ref()));
        any != self.negate
    }

    /// Whether the glob names `name`, also with `.exe`/`.app` left off.
    fn names(&self, name: &str) -> bool {
        let lower: Vec<char> = name.chars().flat_map(char::to_lowercase).take(MAX_APP_LEN * 2).collect();
        if glob_match(&self.glob, &lower) {
            return true;
        }
        let ext = |e: &str| lower.len() > e.len() && lower[lower.len() - e.len()..].iter().copied().eq(e.chars());
        let stem = if ext(".exe") || ext(".app") { &lower[..lower.len() - 4] } else { return false };
        glob_match(&self.glob, stem)
    }

    /// Why this pattern can never name a focused app on a computer running
    /// `target` (one of [`os`]), if it can't: each OS reports names of its
    /// own kind (see the module docs).
    pub fn never_on(&self, target: u8) -> Option<String> {
        let literal: String = self.glob.iter().filter(|c| **c != '*').collect();
        let exe = self.glob.ends_with(&['.', 'e', 'x', 'e']);
        let dots = literal.matches('.').count();
        let looks_like_bundle = dots >= 2 && !exe && !literal.contains(' ');
        match target {
            os::MACOS if exe && !self.negate => Some(format!(
                "app {:?} never matches on a Mac: macOS names apps by bundle id (com.apple.Terminal) or name (Terminal), not .exe files",
                self.text
            )),
            os::WINDOWS if looks_like_bundle && !self.negate => Some(format!(
                "app {:?} never matches on Windows: it looks like a macOS bundle id; Windows names programs by file (chrome.exe)",
                self.text
            )),
            _ => None,
        }
    }
}

/// Whether `glob` (lowercase, `*` for any run) matches all of `s`: the
/// usual two-pointer walk that returns to the last `*`, so time is at most
/// `glob.len() * s.len()` steps, with no recursion.
fn glob_match(glob: &[char], s: &[char]) -> bool {
    let (mut g, mut i) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while i < s.len() {
        if g < glob.len() && glob[g] == '*' {
            star = Some((g, i));
            g += 1;
        } else if g < glob.len() && glob[g] == s[i] {
            g += 1;
            i += 1;
        } else if let Some((sg, si)) = star {
            g = sg + 1;
            i = si + 1;
            star = Some((sg, si + 1));
        } else {
            return false;
        }
    }
    glob[g..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> AppPattern {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn names_ignore_case_and_extensions() {
        let windows = ["chrome.exe", "Chrome_WidgetWin_1"];
        for yes in ["chrome.exe", "Chrome.EXE", "chrome", "CHROME", "chrome_widgetwin_1", "chr*", "*.exe", "*"] {
            assert!(p(yes).matches(&windows), "{yes}");
        }
        for no in ["chrom", "chrome.ex", "firefox", "chrome.exe.exe", "*.app", "edge*"] {
            assert!(!p(no).matches(&windows), "{no}");
        }
        let mac = ["com.apple.Terminal", "Terminal"];
        assert!(p("com.apple.terminal").matches(&mac));
        assert!(p("com.apple.*").matches(&mac));
        assert!(p("terminal").matches(&mac));
        assert!(!p("com.apple").matches(&mac));
        assert!(p("Visual Studio Code").matches(&["com.microsoft.VSCode", "Visual Studio Code"]));
        assert!(p("Safari").matches(&["Safari.app"]));
        let x11 = ["code", "Code"];
        assert!(p("code").matches(&x11));
        assert!(!p("cod").matches(&x11));
        assert!(p("c*e").matches(&x11));
        assert!(p("*o*d*").matches(&x11));
        assert!(!p("*x*").matches(&x11));
    }

    #[test]
    fn globs() {
        let m = |g: &str, s: &str| glob_match(&g.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>());
        assert!(m("", ""));
        assert!(!m("", "a"));
        assert!(m("*", ""));
        assert!(m("a*b*c", "aXXbYYc"));
        assert!(m("a*b*c", "abc"));
        assert!(!m("a*b*c", "acb"));
        assert!(m("*abc", "ababc"));
        assert!(m("a*a*a", "aaaaa"));
        assert!(!m("a*a*b", "aaaaa"));
        assert!(m("ü*", "über"));
    }

    #[test]
    fn negation_and_unknown() {
        let not_code = p("!code");
        assert!(not_code.is_negated());
        assert!(not_code.matches(&["firefox"]));
        assert!(!not_code.matches(&["code", "Code"]));
        assert!(!not_code.matches(&["Code"]));
        // Unknown: nothing applies, negated or not.
        let none: [&str; 0] = [];
        assert!(!not_code.matches(&none));
        assert!(!p("*").matches(&none));
        assert!(!p("code").matches(&none));
        assert!(p("! code").matches(&["vim"]), "a space after ! is fine");
        assert_eq!(p("  !code ").to_string(), "!code");
    }

    #[test]
    fn patterns_that_cant_be_meant_are_refused() {
        for (bad, says) in [
            ("", "give a program name"),
            ("   ", "give a program name"),
            ("!", "give a program name"),
            ("!*", "no app at all"),
            ("!**", "no app at all"),
            ("C:\\Program Files\\chrome.exe", "without its folder"),
            ("/usr/bin/code", "without its folder"),
            ("code\n", "control"),
            ("co\u{0}de", "control"),
            ("co\u{1b}[2Kde", "control"),
            ("evil\u{202E}exe.txt", "invisible"),
            ("a\u{200B}b", "invisible"),
        ] {
            let e = bad.parse::<AppPattern>().unwrap_err();
            assert!(e.contains(says), "{bad:?}: {e}");
        }
        let long = "x".repeat(MAX_APP_LEN + 1);
        assert!(long.parse::<AppPattern>().unwrap_err().contains("at most"));
        assert!("x".repeat(MAX_APP_LEN).parse::<AppPattern>().is_ok());
        // Many stars are one star: no blow-up.
        assert_eq!(p("a***b").glob, ['a', '*', 'b']);
    }

    /// Long and hostile names from a peer: cleaned, never a panic, and
    /// matching stays quick (the glob walk is bounded).
    #[test]
    fn hostile_names_are_cleaned() {
        assert_eq!(clean_name("  code \n"), Some("code".into()));
        assert_eq!(clean_name("co\u{1b}[2Kde"), Some("co[2Kde".into()));
        assert_eq!(clean_name("evil\u{202E}exe.txt"), Some("evilexe.txt".into()));
        assert_eq!(clean_name("\u{200B}\u{FEFF}\t"), None);
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name(&"é".repeat(10_000)).unwrap().chars().count(), MAX_APP_LEN);
        assert_eq!(clean_name("日本語のアプリ"), Some("日本語のアプリ".into()));
        let names = clean_names(&["a", "A", "", "b", "\u{7}", "c", "d", "e"]);
        assert_eq!(names, ["a", "b", "c", "d"]);
        // A worst case for the glob walk: many stars, a long near-miss.
        let star = p(&"a*".repeat(MAX_APP_LEN / 2 - 1));
        let long = "a".repeat(MAX_APP_LEN * 4) + "b";
        let start = std::time::Instant::now();
        assert!(!p("a*a*a*a*a*a*a*c").matches(&[long.as_str()]));
        assert!(star.matches(&[long.as_str()]));
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        // Case-folding that changes length (İ lowercases to two chars).
        assert!(p("i̇stanbul*").matches(&["İSTANBUL.exe"]));
    }

    #[test]
    fn names_that_cant_occur_on_an_os() {
        assert!(p("chrome.exe").never_on(os::MACOS).unwrap().contains("never matches on a Mac"));
        assert!(p("*.exe").never_on(os::MACOS).is_some());
        assert_eq!(p("chrome.exe").never_on(os::WINDOWS), None);
        // Wine windows on X11 have WM_CLASS like `notepad.exe`.
        assert_eq!(p("notepad.exe").never_on(os::LINUX), None);
        assert!(p("com.apple.Terminal").never_on(os::WINDOWS).unwrap().contains("bundle id"));
        assert!(p("com.jetbrains.*").never_on(os::WINDOWS).is_some());
        assert_eq!(p("com.apple.Terminal").never_on(os::MACOS), None);
        assert_eq!(p("org.gnome.Nautilus").never_on(os::LINUX), None);
        assert_eq!(p("chrome").never_on(os::MACOS), None);
        assert_eq!(p("!chrome.exe").never_on(os::MACOS), None, "always true there, not never");
        assert_eq!(p("code").never_on(os::UNKNOWN), None);
    }
}
