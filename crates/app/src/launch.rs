//! How the daemon was started, as far as the permission guide cares: macOS
//! gives Accessibility and Input Monitoring to the program that started the
//! daemon, so one started from a terminal needs the terminal app switched on
//! in System Settings, not `one-mk`. The daemon cannot say how it was
//! started (its permission lines are the same either way), so the window
//! looks at the process list once when the guide opens.

use std::collections::HashMap;

/// The terminal app a daemon was started from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Terminal,
    ITerm,
    /// Some other app, or a shell with no app above it (ssh, tmux).
    Other,
}

/// What macOS lists a permission under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Launch {
    /// By the installed app, at login (`launchd`), or by this window: the
    /// daemon, `one-mk`, is what the lists show. Also what is assumed when
    /// it can't be told.
    #[default]
    Installed,
    /// From a terminal (or anything but the above): that app is listed.
    Terminal(App),
}

impl Launch {
    /// The name in System Settings' list to switch on, in a sentence.
    pub fn listed(self) -> &'static str {
        match self {
            Launch::Installed => crate::guide::LIST_NAME,
            Launch::Terminal(App::Terminal) => "Terminal",
            Launch::Terminal(App::ITerm) => "iTerm",
            Launch::Terminal(App::Other) => "the terminal app you started oneMK from",
        }
    }

    /// The row of the pretend list the card points at (a real name, or the
    /// generic one when the app isn't known).
    pub fn row(self) -> &'static str {
        match self {
            Launch::Terminal(App::Other) => "Your terminal",
            other => other.listed(),
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Launch::Terminal(_))
    }

    /// Asks the system (`ps`) how the daemon here was started; `Installed`
    /// when that can't be told. Runs a program: not on the UI thread.
    pub fn detect() -> Launch {
        let out = std::process::Command::new("/bin/ps").args(["-axwwo", "pid=,ppid=,command="]).output();
        match out {
            Ok(out) if out.status.success() => from_processes(&String::from_utf8_lossy(&out.stdout)),
            _ => Launch::Installed,
        }
    }
}

/// The launch of the daemon in `table` (`ps -axo pid=,ppid=,command=`),
/// found through its parents: launchd (pid 1) or one of our own programs
/// above it is the installed way; an app bundle above it is where it was
/// started from; a shell with no app above it is a terminal too. With no
/// daemon in the table, or several that disagree, the installed way.
pub fn from_processes(table: &str) -> Launch {
    let mut rows: HashMap<u32, (u32, &str)> = HashMap::new();
    for line in table.lines() {
        // Two numbers, then the command (padded with spaces in between).
        let (pid, rest) = line.trim_start().split_once(char::is_whitespace).unwrap_or(("", ""));
        let (ppid, command) = rest.trim_start().split_once(char::is_whitespace).unwrap_or(("", ""));
        if let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) {
            rows.insert(pid, (ppid, command.trim()));
        }
    }
    let mut launches: Vec<Launch> = rows.values().filter(|(_, command)| is_daemon(command)).map(|(ppid, _)| launch_of(*ppid, &rows)).collect();
    launches.dedup();
    match launches.as_slice() {
        [only] => *only,
        [first, rest @ ..] if rest.iter().all(|l| l == first) => *first,
        _ => Launch::Installed,
    }
}

/// The daemon: `one-mk run ...`, plain `one-mk` (which is `run`), and either
/// with options in front of or without `run` (`one-mk --home /x run`,
/// `one-mk --port 1`); not `one-mk status` and the like. Options that take a
/// value are skipped with it; one this does not know is taken for the
/// subcommand's place, which is then not `run`.
fn is_daemon(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let program = words.next().unwrap_or("");
    if !(program == "one-mk" || program.ends_with("/one-mk")) {
        return false;
    }
    while let Some(word) = words.next() {
        match word {
            "run" => return true,
            "--help" | "-h" | "--version" | "-V" => return false,
            "--home" | "--port" | "--connect" | "--backend" | "--downloads" => {
                words.next();
            }
            _ if word.starts_with('-') => {}
            _ => return false,
        }
    }
    true
}

/// The launch of a program whose parent process is `ppid`.
fn launch_of(mut ppid: u32, rows: &HashMap<u32, (u32, &str)>) -> Launch {
    let mut shell = false;
    for _ in 0..32 {
        if ppid <= 1 {
            // launchd itself: the installed way, unless shells came between (tmux started it).
            return if shell { Launch::Terminal(App::Other) } else { Launch::Installed };
        }
        let Some((up, command)) = rows.get(&ppid) else { break };
        if command.contains("one-mk-app") || command.contains("/oneMK.app/") {
            return Launch::Installed;
        }
        if let Some(bundle) = bundle_of(command) {
            return Launch::Terminal(match bundle {
                "Terminal" => App::Terminal,
                "iTerm" | "iTerm2" => App::ITerm,
                _ => App::Other,
            });
        }
        shell = true;
        ppid = *up;
    }
    if shell { Launch::Terminal(App::Other) } else { Launch::Installed }
}

/// `Terminal` of `/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal`.
fn bundle_of(command: &str) -> Option<&str> {
    let end = command.find(".app/")?;
    Some(command[..end].rsplit('/').next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/me/.local/bin/one-mk";

    fn table(rows: &[(u32, u32, &str)]) -> String {
        rows.iter().map(|(pid, ppid, command)| format!("{pid:>5} {ppid:>5} {command}\n")).collect()
    }

    #[test]
    fn started_by_launchd_is_the_installed_way() {
        let t = table(&[(1, 0, "/sbin/launchd"), (400, 1, &format!("{HOME} run"))]);
        assert_eq!(from_processes(&t), Launch::Installed);
    }

    #[test]
    fn started_from_a_terminal_app_names_it() {
        let t = table(&[
            (1, 0, "/sbin/launchd"),
            (500, 1, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"),
            (501, 500, "login -pf me"),
            (502, 501, "-zsh"),
            (503, 502, &format!("{HOME} run --port 24800")),
        ]);
        assert_eq!(from_processes(&t), Launch::Terminal(App::Terminal));
        let t = table(&[(1, 0, "/sbin/launchd"), (500, 1, "/Applications/iTerm.app/Contents/MacOS/iTerm2"), (502, 500, "-zsh"), (503, 502, &format!("{HOME} run"))]);
        assert_eq!(from_processes(&t), Launch::Terminal(App::ITerm));
        let t = table(&[(1, 0, "/sbin/launchd"), (500, 1, "/Applications/Ghostty.app/Contents/MacOS/ghostty"), (502, 500, "-zsh"), (503, 502, &format!("{HOME} run"))]);
        assert_eq!(from_processes(&t), Launch::Terminal(App::Other));
    }

    #[test]
    fn started_by_this_window_is_the_installed_way() {
        let t = table(&[(1, 0, "/sbin/launchd"), (600, 1, "/Applications/oneMK.app/Contents/MacOS/one-mk-app"), (601, 600, &format!("{HOME} run --home /x"))]);
        assert_eq!(from_processes(&t), Launch::Installed);
        // A development window, not in a bundle.
        let t = table(&[(1, 0, "/sbin/launchd"), (600, 1, "/work/target/debug/one-mk-app"), (601, 600, &format!("{HOME} run"))]);
        assert_eq!(from_processes(&t), Launch::Installed);
    }

    #[test]
    fn a_plain_or_optioned_daemon_started_in_terminal_names_it() {
        for command in ["one-mk", "one-mk --home /tmp/desk run"] {
            let t = table(&[
                (1, 0, "/sbin/launchd"),
                (500, 1, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"),
                (502, 500, "-zsh"),
                (503, 502, &format!("{HOME} {}", command.trim_start_matches("one-mk").trim_start())),
            ]);
            assert_eq!(from_processes(&t), Launch::Terminal(App::Terminal), "{command}");
        }
    }

    #[test]
    fn a_shell_with_no_app_above_it_is_a_terminal_too() {
        // tmux's server has no app above it.
        let t = table(&[(1, 0, "/sbin/launchd"), (700, 1, "tmux: server"), (701, 700, "-zsh"), (702, 701, &format!("{HOME} run"))]);
        assert_eq!(from_processes(&t), Launch::Terminal(App::Other));
    }

    #[test]
    fn nothing_to_go_by_is_the_installed_way() {
        assert_eq!(from_processes(""), Launch::Installed);
        assert_eq!(from_processes("garbage\n  12 abc x\n"), Launch::Installed);
        // `one-mk status` is not the daemon.
        let t = table(&[(1, 0, "/sbin/launchd"), (502, 1, "-zsh"), (503, 502, &format!("{HOME} status"))]);
        assert_eq!(from_processes(&t), Launch::Installed);
        // Two daemons that were started differently: no telling which is ours.
        let t = table(&[
            (1, 0, "/sbin/launchd"),
            (400, 1, &format!("{HOME} run")),
            (500, 1, "/Applications/iTerm.app/Contents/MacOS/iTerm2"),
            (502, 500, "-zsh"),
            (503, 502, &format!("{HOME} run")),
        ]);
        assert_eq!(from_processes(&t), Launch::Installed);
    }

    #[test]
    fn the_daemon_is_told_from_its_neighbours() {
        assert!(is_daemon("one-mk run"));
        assert!(is_daemon("/usr/local/bin/one-mk run --port 1"));
        assert!(!is_daemon("/usr/local/bin/one-mk status"));
        assert!(!is_daemon("/usr/local/bin/one-mk-app"));
        assert!(!is_daemon("vim notes-one-mk run.txt"));
        assert!(!is_daemon("/usr/local/bin/one-mk runner"));
        // Plain `one-mk` is `run`, and options may come before the subcommand.
        assert!(is_daemon("one-mk"));
        assert!(is_daemon("/usr/local/bin/one-mk --home /tmp/desk run"));
        assert!(is_daemon("/usr/local/bin/one-mk --home /tmp/desk"));
        assert!(is_daemon("/usr/local/bin/one-mk -vv --backend null --no-clipboard run --port 1"));
        assert!(is_daemon("/usr/local/bin/one-mk --home=/tmp/desk --no-discovery"));
        assert!(!is_daemon("/usr/local/bin/one-mk --home /tmp/desk status"));
        assert!(!is_daemon("/usr/local/bin/one-mk -v send a run"));
        assert!(!is_daemon("/usr/local/bin/one-mk --version"));
    }
}
