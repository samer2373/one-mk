//! `one-mk install` / `uninstall`: start one-mk when you log in.
//!
//! Linux: a systemd user service (or, without systemd, an XDG autostart
//! entry). macOS: a LaunchAgent. Windows: a per-user Run registry entry
//! (no administrator rights needed). Both commands can be repeated safely.

use crate::control::{self, Request};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const LABEL: &str = "com.one-mk.daemon";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// What to start: this program, with `--home` if the user chose a folder.
struct Launch {
    exe: PathBuf,
    home: Option<PathBuf>,
}

impl Launch {
    fn args(&self) -> Vec<String> {
        // `--home` goes after `run`: before it, clap refuses the combination.
        let mut a = vec!["run".to_owned()];
        if let Some(h) = &self.home {
            a.extend(["--home".to_owned(), h.display().to_string()]);
        }
        a
    }
}

/// `dir` is where this copy keeps its settings; the autostarted copy gets
/// `--home` only if `custom_home` says the user chose it.
pub async fn install(dir: &Path, custom_home: bool) -> Result<()> {
    let launch = Launch { exe: std::env::current_exe().context("finding this program")?, home: custom_home.then(|| absolute(dir)) };
    if control::request(dir, Request::Status).await.is_ok_and(|r| r.is_some()) {
        println!("Note: one-mk is already running. Stop it (Ctrl+C where it runs) so the automatic copy can take over.");
    }
    if cfg!(target_os = "linux") {
        install_linux(&launch)
    } else if cfg!(target_os = "macos") {
        install_macos(&launch)
    } else {
        install_windows(&launch)
    }
}

pub fn uninstall() -> Result<()> {
    if cfg!(target_os = "linux") {
        uninstall_linux()
    } else if cfg!(target_os = "macos") {
        uninstall_macos()
    } else {
        uninstall_windows()
    }
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).context("no home directory")
}

/// Runs a command; true if it started and succeeded.
fn ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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

fn command_line(l: &Launch) -> String {
    std::iter::once(l.exe.display().to_string()).chain(l.args()).map(|w| quote(&w)).collect::<Vec<_>>().join(" ")
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
        command_line(l)
    )
}

fn desktop_entry(l: &Launch) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=one-mk\nComment=Share one mouse and keyboard\nExec={}\n\
         Terminal=false\nX-GNOME-Autostart-enabled=true\n",
        command_line(l)
    )
}

fn install_linux(l: &Launch) -> Result<()> {
    let home = home_dir()?;
    if systemd_available() {
        let unit = unit_path(&home);
        write(&unit, &systemd_unit(l))?;
        let _ = remove(&desktop_path(&home)); // never start twice
        if !ok("systemctl", &["--user", "daemon-reload"]) || !ok("systemctl", &["--user", "enable", "--now", "one-mk.service"]) {
            bail!("wrote {} but `systemctl --user enable --now one-mk.service` failed", unit.display());
        }
        println!("one-mk will now start whenever you log in, and is starting now.");
        println!("  service file: {}", unit.display());
        println!("  see how it is doing:  one-mk status    (log: journalctl --user -u one-mk)");
        println!("  to undo:              one-mk uninstall");
    } else {
        let entry = desktop_path(&home);
        write(&entry, &desktop_entry(l))?;
        println!("one-mk will start the next time you log in (systemd is not available here).");
        println!("  autostart entry: {}", entry.display());
        println!("  to start it now:  one-mk run");
        println!("  to undo:          one-mk uninstall");
    }
    Ok(())
}

fn uninstall_linux() -> Result<()> {
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
    report_uninstall(did);
    Ok(())
}

// ----------------------------------------------------------------- macOS

fn plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn launch_agent(l: &Launch, log: &Path) -> String {
    let args: String = std::iter::once(l.exe.display().to_string())
        .chain(l.args())
        .map(|a| format!("    <string>{}</string>\n", xml_escape(&a)))
        .collect();
    let log = xml_escape(&log.display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n  <string>{LABEL}</string>\n\
         \x20 <key>ProgramArguments</key>\n  <array>\n{args}  </array>\n\
         \x20 <key>RunAtLoad</key>\n  <true/>\n\
         \x20 <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n\
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

fn install_macos(l: &Launch) -> Result<()> {
    let home = home_dir()?;
    let plist = plist_path(&home);
    let log = home.join("Library/Logs/one-mk.log");
    write(&plist, &launch_agent(l, &log))?;
    let domain = gui_domain();
    ok("launchctl", &["bootout", &format!("{domain}/{LABEL}")]); // replace an older copy
    if !ok("launchctl", &["bootstrap", &domain, &plist.display().to_string()]) {
        bail!("wrote {} but `launchctl bootstrap {domain}` failed", plist.display());
    }
    println!("one-mk will now start whenever you log in, and is starting now.");
    println!("  LaunchAgent: {}", plist.display());
    println!("  log:         {}", log.display());
    println!("  macOS will ask for Accessibility and Input Monitoring permission the first time; allow one-mk.");
    println!("  to undo:     one-mk uninstall");
    Ok(())
}

fn uninstall_macos() -> Result<()> {
    let plist = plist_path(&home_dir()?);
    let mut did = false;
    if plist.exists() {
        ok("launchctl", &["bootout", &format!("{}/{LABEL}", gui_domain())]);
        did = remove(&plist)?;
    }
    report_uninstall(did);
    Ok(())
}

// --------------------------------------------------------------- Windows

fn run_value(l: &Launch) -> String {
    std::iter::once(l.exe.display().to_string())
        .chain(l.args())
        .map(|w| if w.contains(' ') { format!("\"{w}\"") } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

fn install_windows(l: &Launch) -> Result<()> {
    let value = run_value(l);
    if !ok("reg", &["add", RUN_KEY, "/v", "one-mk", "/t", "REG_SZ", "/d", &value, "/f"]) {
        bail!("could not add one-mk to the startup list (`reg add {RUN_KEY}` failed)");
    }
    println!("one-mk will now start whenever you sign in to Windows.");
    println!("  startup entry: {RUN_KEY}\\one-mk");
    println!("  to start it now:  one-mk run");
    println!("  to undo:          one-mk uninstall");
    Ok(())
}

fn uninstall_windows() -> Result<()> {
    let did = ok("reg", &["delete", RUN_KEY, "/v", "one-mk", "/f"]);
    report_uninstall(did);
    Ok(())
}

fn report_uninstall(did: bool) {
    if did {
        println!("one-mk will no longer start when you log in.");
        println!("  to turn it back on:  one-mk install");
    } else {
        println!("one-mk was not set to start at login; nothing to remove.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(home: Option<&str>) -> Launch {
        Launch { exe: "/opt/my apps/one-mk".into(), home: home.map(PathBuf::from) }
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
        let e = desktop_entry(&launch(None));
        assert!(e.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(e.contains("Exec=\"/opt/my apps/one-mk\" \"run\"\n"));
    }

    #[test]
    fn launch_agent_is_valid_and_escaped() {
        let plist = launch_agent(&launch(Some("/Users/a&b")), Path::new("/Users/me/Library/Logs/one-mk.log"));
        assert!(plist.contains("<string>com.one-mk.daemon</string>"));
        assert!(plist.contains("<string>/opt/my apps/one-mk</string>\n    <string>run</string>\n    <string>--home</string>\n    <string>/Users/a&amp;b</string>"), "{plist}");
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<string>/Users/me/Library/Logs/one-mk.log</string>"));
        assert_eq!(plist.matches("<dict>").count(), plist.matches("</dict>").count());
    }

    #[test]
    fn windows_run_value_quotes_spaces() {
        assert_eq!(run_value(&launch(None)), "\"/opt/my apps/one-mk\" run");
    }

    #[test]
    fn files_live_where_the_systems_look() {
        let h = Path::new("/home/u");
        assert_eq!(unit_path(h), Path::new("/home/u/.config/systemd/user/one-mk.service"));
        assert_eq!(desktop_path(h), Path::new("/home/u/.config/autostart/one-mk.desktop"));
        assert_eq!(plist_path(h), Path::new("/home/u/Library/LaunchAgents/com.one-mk.daemon.plist"));
    }
}
