//! Start one-mk when you log in.
//!
//! Linux: a systemd user service (or, without systemd, an XDG autostart
//! entry). macOS: a LaunchAgent. Windows: a per-user Run registry entry
//! (no administrator rights needed). All of it can be repeated safely.
//!
//! If the tray program is installed next to `one-mk`, it is started at
//! login too. On Windows it replaces the plain `one-mk run` entry, because
//! a console program shows a console window at login while the tray starts
//! the daemon hidden (`--start-daemon`).

use crate::proc::run_quiet as ok;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const LABEL: &str = "com.one-mk.daemon";
const TRAY_LABEL: &str = "com.one-mk.tray";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// What to start: the `one-mk` program (and the tray, if there is one), with
/// `--home` if the user chose a folder.
pub struct Launch {
    pub exe: PathBuf,
    pub tray: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// Also start it right now (unless it is already running).
    pub start_now: bool,
}

impl Launch {
    /// `one-mk run`, with `--home` after `run`: before it, clap refuses it.
    fn args(&self) -> Vec<String> {
        let mut a = vec!["run".to_owned()];
        a.extend(self.home_args());
        a
    }

    fn home_args(&self) -> Vec<String> {
        self.home.iter().flat_map(|h| ["--home".to_owned(), h.display().to_string()]).collect()
    }

    fn tray_args(&self, start_daemon: bool) -> Vec<String> {
        let mut a: Vec<String> = start_daemon.then(|| "--start-daemon".to_owned()).into_iter().collect();
        a.extend(self.home_args());
        a
    }
}

/// The program called `name` that sits next to `exe`, if there is one.
pub fn sibling(exe: &Path, name: &str) -> Option<PathBuf> {
    let p = exe.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    p.is_file().then_some(p)
}

/// Makes `launch` start at login and says what was done.
pub fn install(launch: &Launch) -> Result<String> {
    if cfg!(target_os = "linux") {
        install_linux(launch)
    } else if cfg!(target_os = "macos") {
        install_macos(launch)
    } else {
        install_windows(launch)
    }
}

/// Stops starting one-mk (and the tray) at login, and says what was done.
pub fn uninstall() -> Result<String> {
    let did = if cfg!(target_os = "linux") {
        uninstall_linux()?
    } else if cfg!(target_os = "macos") {
        uninstall_macos()?
    } else {
        uninstall_windows()
    };
    Ok(if did {
        "one-mk will no longer start when you log in.\n  to turn it back on:  one-mk install".to_owned()
    } else {
        "one-mk was not set to start at login; nothing to remove.".to_owned()
    })
}

/// True if one-mk is set to start at login.
pub fn is_installed() -> bool {
    if cfg!(target_os = "linux") {
        home_dir().is_ok_and(|h| unit_path(&h).exists() || desktop_path(&h).exists())
    } else if cfg!(target_os = "macos") {
        home_dir().is_ok_and(|h| plist_path(&h, LABEL).exists())
    } else {
        ok("reg", &["query", RUN_KEY, "/v", "one-mk"])
    }
}

pub fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).context("no home directory")
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

fn remove(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

// ----------------------------------------------------------------- Linux

fn unit_path(home: &Path) -> PathBuf {
    home.join(".config/systemd/user/one-mk.service")
}

fn desktop_path(home: &Path) -> PathBuf {
    home.join(".config/autostart/one-mk.desktop")
}

fn tray_desktop_path(home: &Path) -> PathBuf {
    home.join(".config/autostart/one-mk-tray.desktop")
}

/// The (hidden) application entry that gives one-mk an app id, for the
/// GlobalShortcuts portal.
fn app_entry_path(home: &Path) -> PathBuf {
    home.join(".local/share/applications").join(format!("{}.desktop", one_mk_proto::LINUX_APP_ID))
}

fn systemd_available() -> bool {
    ok("systemctl", &["--user", "show-environment"])
}

/// Quotes one word for a systemd `ExecStart=` or desktop `Exec=` line.
fn quote(word: &str) -> String {
    let mut out = String::from("\"");
    for c in word.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        if c == '%' {
            out.push('%');
        }
        out.push(c);
    }
    out + "\""
}

fn command_line(exe: &Path, args: &[String]) -> String {
    std::iter::once(exe.display().to_string()).chain(args.iter().cloned()).map(|w| quote(&w)).collect::<Vec<_>>().join(" ")
}

fn systemd_unit(l: &Launch) -> String {
    format!(
        "[Unit]\n\
         Description=one-mk: share one mouse and keyboard\n\
         After=graphical-session.target\n\
         PartOf=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        command_line(&l.exe, &l.args())
    )
}

fn desktop_entry(name: &str, exe: &Path, args: &[String]) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Share one mouse and keyboard\nExec={}\n\
         Terminal=false\nX-GNOME-Autostart-enabled=true\n",
        command_line(exe, args)
    )
}

fn install_linux(l: &Launch) -> Result<String> {
    let home = home_dir()?;
    let mut msg = String::new();
    // Before the service starts: the daemon registers its app id at startup.
    let app = app_entry_path(&home);
    write(&app, &(desktop_entry("one-mk", &l.exe, &l.args()).replace("X-GNOME-Autostart-enabled=true\n", "") + "NoDisplay=true\n"))?;
    if systemd_available() {
        let unit = unit_path(&home);
        write(&unit, &systemd_unit(l))?;
        let _ = remove(&desktop_path(&home)); // never start twice
        let enable: &[&str] = if l.start_now { &["--user", "enable", "--now", "one-mk.service"] } else { &["--user", "enable", "one-mk.service"] };
        if !ok("systemctl", &["--user", "daemon-reload"]) || !ok("systemctl", enable) {
            bail!("wrote {} but `systemctl --user enable one-mk.service` failed", unit.display());
        }
        msg += &format!(
            "one-mk will now start whenever you log in{}.\n  service file: {}\n  see how it is doing:  one-mk status    (log: journalctl --user -u one-mk)\n",
            if l.start_now { ", and is starting now" } else { " (it is already running now)" },
            unit.display()
        );
    } else {
        let entry = desktop_path(&home);
        write(&entry, &desktop_entry("one-mk", &l.exe, &l.args()))?;
        msg += &format!("one-mk will start the next time you log in (systemd is not available here).\n  autostart entry: {}\n  to start it now:  one-mk run\n", entry.display());
    }
    if let Some(tray) = &l.tray {
        let entry = tray_desktop_path(&home);
        write(&entry, &desktop_entry("one-mk tray", tray, &l.tray_args(false)))?;
        msg += &format!("  the tray icon will start at login too: {}\n", entry.display());
    }
    msg += &format!("  desktop entry (for keyboard shortcuts): {}\n", app.display());
    msg += "  to undo:              one-mk uninstall";
    Ok(msg)
}

fn uninstall_linux() -> Result<bool> {
    let home = home_dir()?;
    let unit = unit_path(&home);
    let mut did = false;
    if unit.exists() {
        if systemd_available() {
            ok("systemctl", &["--user", "disable", "--now", "one-mk.service"]);
        }
        did |= remove(&unit)?;
        if systemd_available() {
            ok("systemctl", &["--user", "daemon-reload"]);
        }
    }
    did |= remove(&desktop_path(&home))?;
    did |= remove(&tray_desktop_path(&home))?;
    did |= remove(&app_entry_path(&home))?;
    Ok(did)
}

// ----------------------------------------------------------------- macOS

fn plist_path(home: &Path, label: &str) -> PathBuf {
    home.join("Library/LaunchAgents").join(format!("{label}.plist"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn launch_agent(label: &str, exe: &Path, args: &[String], log: &Path, keep_alive: bool) -> String {
    let args: String = std::iter::once(exe.display().to_string())
        .chain(args.iter().cloned())
        .map(|a| format!("    <string>{}</string>\n", xml_escape(&a)))
        .collect();
    let log = xml_escape(&log.display().to_string());
    let keep_alive = if keep_alive { "  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n" } else { "" };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n  <string>{label}</string>\n\
         \x20 <key>ProgramArguments</key>\n  <array>\n{args}  </array>\n\
         \x20 <key>RunAtLoad</key>\n  <true/>\n\
         {keep_alive}\
         \x20 <key>ProcessType</key>\n  <string>Interactive</string>\n\
         \x20 <key>StandardOutPath</key>\n  <string>{log}</string>\n\
         \x20 <key>StandardErrorPath</key>\n  <string>{log}</string>\n\
         </dict>\n\
         </plist>\n"
    )
}

fn gui_domain() -> String {
    let uid = Command::new("id").arg("-u").output().ok().and_then(|o| String::from_utf8(o.stdout).ok());
    format!("gui/{}", uid.as_deref().map_or("501", str::trim))
}

/// Writes a LaunchAgent and, if `load_now`, (re)loads it.
fn mac_agent(home: &Path, label: &str, exe: &Path, args: &[String], log: &Path, keep_alive: bool, load_now: bool) -> Result<PathBuf> {
    let plist = plist_path(home, label);
    write(&plist, &launch_agent(label, exe, args, log, keep_alive))?;
    if load_now {
        let domain = gui_domain();
        ok("launchctl", &["bootout", &format!("{domain}/{label}")]); // replace an older copy
        if !ok("launchctl", &["bootstrap", &domain, &plist.display().to_string()]) {
            bail!("wrote {} but `launchctl bootstrap {domain}` failed", plist.display());
        }
    }
    Ok(plist)
}

fn install_macos(l: &Launch) -> Result<String> {
    let home = home_dir()?;
    let log = home.join("Library/Logs/one-mk.log");
    let plist = mac_agent(&home, LABEL, &l.exe, &l.args(), &log, true, l.start_now)?;
    let mut msg = format!(
        "one-mk will now start whenever you log in{}.\n  LaunchAgent: {}\n  log:         {}\n  macOS will ask for Accessibility and Input Monitoring permission the first time; allow one-mk.\n",
        if l.start_now { ", and is starting now" } else { " (it is already running now)" },
        plist.display(),
        log.display()
    );
    if let Some(tray) = &l.tray {
        let tray_log = home.join("Library/Logs/one-mk-tray.log");
        let p = mac_agent(&home, TRAY_LABEL, tray, &l.tray_args(false), &tray_log, false, false)?;
        msg += &format!("  the menu bar icon will start at login too: {}\n", p.display());
    }
    msg += "  to undo:     one-mk uninstall";
    Ok(msg)
}

fn uninstall_macos() -> Result<bool> {
    let home = home_dir()?;
    let mut did = false;
    for label in [LABEL, TRAY_LABEL] {
        let plist = plist_path(&home, label);
        if plist.exists() {
            ok("launchctl", &["bootout", &format!("{}/{label}", gui_domain())]);
            did |= remove(&plist)?;
        }
    }
    Ok(did)
}

// --------------------------------------------------------------- Windows

fn quote_win(w: &str) -> String {
    if w.contains(' ') { format!("\"{w}\"") } else { w.to_owned() }
}

/// The login entry: the tray (which starts the daemon hidden) if there is
/// one, else `one-mk run`.
fn run_value(l: &Launch) -> String {
    let (exe, args) = match &l.tray {
        Some(t) => (t, l.tray_args(true)),
        None => (&l.exe, l.args()),
    };
    std::iter::once(exe.display().to_string()).chain(args).map(|w| quote_win(&w)).collect::<Vec<_>>().join(" ")
}

fn install_windows(l: &Launch) -> Result<String> {
    let value = run_value(l);
    if !ok("reg", &["add", RUN_KEY, "/v", "one-mk", "/t", "REG_SZ", "/d", &value, "/f"]) {
        bail!("could not add one-mk to the startup list (`reg add {RUN_KEY}` failed)");
    }
    Ok(format!(
        "one-mk will now start whenever you sign in to Windows{}.\n  startup entry: {RUN_KEY}\\one-mk\n  to start it now:  {}\n  to undo:          one-mk uninstall",
        if l.tray.is_some() { ", as a tray icon without a console window" } else { "" },
        if l.tray.is_some() { "one-mk-tray --start-daemon" } else { "one-mk run" },
    ))
}

fn uninstall_windows() -> bool {
    ok("reg", &["delete", RUN_KEY, "/v", "one-mk", "/f"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(home: Option<&str>) -> Launch {
        Launch { exe: "/opt/my apps/one-mk".into(), tray: None, home: home.map(PathBuf::from), start_now: true }
    }

    #[test]
    fn systemd_unit_quotes_the_path() {
        let unit = systemd_unit(&launch(None));
        assert!(unit.contains("ExecStart=\"/opt/my apps/one-mk\" \"run\"\n"), "{unit}");
        assert!(unit.contains("WantedBy=graphical-session.target"));
        assert!(unit.contains("Restart=on-failure"));
        let with_home = systemd_unit(&launch(Some("/tmp/a b")));
        assert!(with_home.contains("ExecStart=\"/opt/my apps/one-mk\" \"run\" \"--home\" \"/tmp/a b\"\n"), "{with_home}");
    }

    #[test]
    fn quoting_escapes_specials() {
        assert_eq!(quote(r#"a"b$c%d\e"#), r#""a\"b\$c%%d\\e""#);
    }

    #[test]
    fn desktop_entry_launches_run() {
        let l = launch(None);
        let e = desktop_entry("one-mk", &l.exe, &l.args());
        assert!(e.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(e.contains("Exec=\"/opt/my apps/one-mk\" \"run\"\n"));
    }

    #[test]
    fn tray_entry_passes_home_but_not_start_daemon() {
        let l = Launch { tray: Some("/opt/my apps/one-mk-tray".into()), ..launch(Some("/tmp/h")) };
        let e = desktop_entry("one-mk tray", l.tray.as_ref().unwrap(), &l.tray_args(false));
        assert!(e.contains("Exec=\"/opt/my apps/one-mk-tray\" \"--home\" \"/tmp/h\"\n"), "{e}");
    }

    #[test]
    fn launch_agent_is_valid_and_escaped() {
        let l = launch(Some("/Users/a&b"));
        let plist = launch_agent(LABEL, &l.exe, &l.args(), Path::new("/Users/me/Library/Logs/one-mk.log"), true);
        assert!(plist.contains("<string>com.one-mk.daemon</string>"));
        assert!(plist.contains("<string>/opt/my apps/one-mk</string>\n    <string>run</string>\n    <string>--home</string>\n    <string>/Users/a&amp;b</string>"), "{plist}");
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<string>/Users/me/Library/Logs/one-mk.log</string>"));
        assert_eq!(plist.matches("<dict>").count(), plist.matches("</dict>").count());
        let tray = launch_agent(TRAY_LABEL, Path::new("/t"), &[], Path::new("/l"), false);
        assert!(!tray.contains("KeepAlive"));
    }

    #[test]
    fn windows_login_entry_prefers_the_tray() {
        assert_eq!(run_value(&launch(None)), "\"/opt/my apps/one-mk\" run");
        let l = Launch { tray: Some("C:/Program Files/one-mk/one-mk-tray.exe".into()), ..launch(Some("D:/x"))};
        assert_eq!(run_value(&l), "\"C:/Program Files/one-mk/one-mk-tray.exe\" --start-daemon --home D:/x");
    }

    #[test]
    fn files_live_where_the_systems_look() {
        let h = Path::new("/home/u");
        assert_eq!(unit_path(h), Path::new("/home/u/.config/systemd/user/one-mk.service"));
        assert_eq!(desktop_path(h), Path::new("/home/u/.config/autostart/one-mk.desktop"));
        assert_eq!(tray_desktop_path(h), Path::new("/home/u/.config/autostart/one-mk-tray.desktop"));
        assert_eq!(app_entry_path(h), Path::new("/home/u/.local/share/applications/io.github.onemk.OneMk.desktop"));
        assert_eq!(plist_path(h, LABEL), Path::new("/home/u/Library/LaunchAgents/com.one-mk.daemon.plist"));
    }
}
