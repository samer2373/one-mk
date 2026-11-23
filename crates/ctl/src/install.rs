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

/// Refuses a path with a line break or NUL in it: it would end up in a
/// service file or login entry, where the rest of the path would be read as
/// a new setting (`Exec=`, `ExecStart=`).
pub fn check_plain(what: &str, path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if text.contains(['\r', '\n']) {
        bail!("{what} has a line break in it ({text:?}); one-mk cannot write it into a startup or menu entry safely. Move it somewhere with a plain path.");
    }
    if text.contains('\0') {
        bail!("{what} has a NUL character in it ({text:?}); one-mk cannot write it into a startup or menu entry.");
    }
    Ok(())
}

impl Launch {
    /// Every path that is going into a generated file must be plain.
    fn check(&self) -> Result<()> {
        check_plain("the program's path", &self.exe)?;
        if let Some(t) = &self.tray {
            check_plain("the tray program's path", t)?;
        }
        if let Some(h) = &self.home {
            check_plain("the settings folder's path", h)?;
        }
        Ok(())
    }

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

/// The path to start this program by later (from the login service, the
/// file manager menu). Inside an AppImage the running copy lives in a
/// temporary mount that is gone afterwards, so it is the AppImage file
/// itself, which hands its arguments to `one-mk`.
pub fn program_path() -> Result<PathBuf> {
    match std::env::var_os("APPIMAGE").map(PathBuf::from) {
        Some(p) if p.is_file() => Ok(p),
        _ => std::env::current_exe().context("finding this program"),
    }
}

/// The program called `name` that sits next to `exe`, if there is one.
pub fn sibling(exe: &Path, name: &str) -> Option<PathBuf> {
    let p = exe.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    p.is_file().then_some(p)
}

/// Makes `launch` start at login and says what was done.
pub fn install(launch: &Launch) -> Result<String> {
    launch.check()?;
    if cfg!(target_os = "linux") {
        install_linux(launch)
    } else if cfg!(target_os = "macos") {
        install_macos(launch)
    } else {
        install_windows(launch)
    }
}

/// Stops starting one-mk (and the tray) at login, stops the copy the service manager is running,
/// and says what was done.
pub fn uninstall() -> Result<String> {
    remove_login(true)
}

/// Stops starting one-mk (and the tray) at login and nothing else: a copy that is running (and
/// sharing) now keeps running until it is stopped. What the window's "Start at login" switch uses.
pub fn disable_login() -> Result<String> {
    remove_login(false)
}

fn remove_login(stop: bool) -> Result<String> {
    let did = if cfg!(target_os = "linux") {
        uninstall_linux(stop)?
    } else if cfg!(target_os = "macos") {
        uninstall_macos(stop)?
    } else {
        return uninstall_windows();
    };
    Ok(if did {
        format!(
            "one-mk will no longer start when you log in{}.\n  to turn it back on:  one-mk install",
            if stop { "" } else { " (the copy running now is left running)" }
        )
    } else {
        "one-mk was not set to start at login; nothing to remove.".to_owned()
    })
}

/// True if one-mk is set to start at login, and the system has not switched that off.
pub fn is_installed() -> bool {
    if cfg!(windows) {
        ok("reg", &["query", RUN_KEY, "/v", "one-mk"])
    } else {
        login_entry().is_some()
    }
}

/// True if the login entry starts oneMK with this settings folder (`None`:
/// the usual one). A window opened on another folder (`--home`) doesn't
/// show the usual folder's entry as its own, nor replace it unasked.
pub fn is_installed_for(home: Option<&Path>) -> bool {
    login_entry().is_some_and(|entry| entry_starts(&entry, home))
}

/// The text of this user's login entry, if the system would run it: a service systemd has
/// disabled, an autostart entry the desktop has switched off, or a LaunchAgent macOS has
/// disabled (all of which leave the file in place) are not login entries.
fn login_entry() -> Option<String> {
    login_entry_text(true)
}

/// The login entry's text; with `running_only` false also an entry the system has switched off
/// (it is still there to be removed, and still names a settings folder).
fn login_entry_text(running_only: bool) -> Option<String> {
    if cfg!(target_os = "linux") {
        let config = crate::xdg::config_home().ok()?;
        let mut text = String::new();
        if let Ok(unit) = std::fs::read_to_string(unit_path(&config)) && (!running_only || !systemd_for(&config) || ok("systemctl", &["--user", "is-enabled", "one-mk.service"])) {
            text += &unit;
        }
        if let Ok(entry) = std::fs::read_to_string(desktop_path(&config)) && (!running_only || !autostart_disabled(&entry)) {
            text += &entry;
        }
        (!text.is_empty()).then_some(text)
    } else if cfg!(target_os = "macos") {
        let home = home_dir().ok()?;
        let text = std::fs::read_to_string(plist_path(&home, LABEL)).ok()?;
        let disabled = running_only && launchd_owns(&home, account_home().as_deref()) && Command::new("launchctl").args(["print-disabled", &gui_domain()]).output().ok()
            .is_some_and(|out| launchd_disabled(&String::from_utf8_lossy(&out.stdout), LABEL));
        (!disabled).then_some(text)
    } else {
        windows_startup_value().ok().flatten()
    }
}

/// The settings folder the login entry starts oneMK with (`None`: the usual one, or no entry).
/// What `one-mk uninstall` reads before it removes the entry, so the wallpaper of that folder
/// is put back too. On Windows an entry that belongs to another copy of oneMK (it is left in
/// place) is not this copy's to clean up for, so it gives `None`.
pub fn registered_home() -> Option<PathBuf> {
    let text = login_entry_text(false)?;
    if cfg!(windows) {
        let exe = std::env::current_exe().ok()?;
        if !windows_run_owned(&text, &exe.parent()?.to_string_lossy()) {
            return None;
        }
    }
    entry_home(&text)
}

/// The `--home` folder in a login entry written by [`install`]: a systemd unit or autostart
/// entry (`ExecStart=` / `Exec=` lines), a LaunchAgent (the argument list), or a Windows `Run`
/// value (one command line). Each is read back the way its system reads it, so a folder with
/// spaces, quotes, `%` or `$` in its name comes out as it went in.
fn entry_home(entry: &str) -> Option<PathBuf> {
    if entry.contains("<plist") {
        let (_, after) = entry.split_once("<string>--home</string>")?;
        let after = after.trim_start().strip_prefix("<string>")?;
        return Some(PathBuf::from(xml_unescape(after.split_once("</string>")?.0)));
    }
    let word_after_home = |words: Vec<String>| {
        let at = words.iter().position(|w| w == "--home")?;
        words.get(at + 1).filter(|w| !w.is_empty()).map(PathBuf::from)
    };
    let lines = entry.lines().map(str::trim);
    let linux: Vec<Vec<String>> = lines.clone().filter_map(|line| {
        line.strip_prefix("ExecStart=").map(|rest| exec_words(rest, false))
            .or_else(|| line.strip_prefix("Exec=").map(|rest| exec_words(rest, true)))
    }).collect();
    if !linux.is_empty() {
        return linux.into_iter().find_map(word_after_home);
    }
    word_after_home(win_words(entry.trim()))
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// The words of a systemd `ExecStart=` value (`desktop` false) or a desktop entry's `Exec=` value
/// (true), undoing [`quote_systemd`] and [`quote_desktop`].
fn exec_words(line: &str, desktop: bool) -> Vec<String> {
    // A desktop entry's value is a string first (`\\` is a backslash), then Exec-quoted.
    let text: String = if desktop {
        let mut out = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c != '\\' { out.push(c); continue; }
            match chars.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    } else {
        line.to_owned()
    };
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => { quoted = !quoted; started = true; }
            '\\' if quoted => {
                if let Some(next) = chars.next() { word.push(next); }
                started = true;
            }
            // `%%` (and systemd's `$$`) are one literal character.
            '%' | '$' if chars.peek() == Some(&c) && (c == '%' || !desktop) => { chars.next(); word.push(c); started = true; }
            c if c.is_whitespace() && !quoted => {
                if started { words.push(std::mem::take(&mut word)); started = false; }
            }
            c => { word.push(c); started = true; }
        }
    }
    if started { words.push(word); }
    words
}

/// The words of a Windows command line, by the rules of `CommandLineToArgvW` (the inverse of [`quote_win`]).
fn win_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut backslashes = 1;
                while chars.peek() == Some(&'\\') { chars.next(); backslashes += 1; }
                started = true;
                if chars.peek() == Some(&'"') {
                    word.push_str(&"\\".repeat(backslashes / 2));
                    if backslashes % 2 == 1 { chars.next(); word.push('"'); }
                } else {
                    word.push_str(&"\\".repeat(backslashes));
                }
            }
            '"' => { quoted = !quoted; started = true; }
            c if c.is_whitespace() && !quoted => {
                if started { words.push(std::mem::take(&mut word)); started = false; }
            }
            c => { word.push(c); started = true; }
        }
    }
    if started { words.push(word); }
    words
}

/// Whether an XDG autostart entry says it is off (`Hidden=true`, or the `X-GNOME-Autostart-enabled=false`
/// that GNOME's startup settings write into it).
fn autostart_disabled(entry: &str) -> bool {
    entry.lines().any(|line| {
        let Some((key, value)) = line.split_once('=') else { return false };
        let value = value.trim();
        match key.trim() {
            "Hidden" => value.eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => value.eq_ignore_ascii_case("false"),
            _ => false,
        }
    })
}

/// Whether `launchctl print-disabled` lists `label` as disabled (`"label" => disabled`; older systems say `true`).
fn launchd_disabled(listing: &str, label: &str) -> bool {
    listing.lines().any(|line| {
        let Some((name, state)) = line.split_once("=>") else { return false };
        name.trim().trim_matches('"') == label && matches!(state.trim().trim_end_matches(','), "disabled" | "true")
    })
}

fn entry_starts(entry: &str, home: Option<&Path>) -> bool {
    let Some(home) = home else { return !entry.contains("--home") };
    let home = home.display().to_string();
    let windows = format!("--home {}", quote_win(&home));
    entry.contains(&format!("<string>--home</string>\n    <string>{}</string>", xml_escape(&home)))
        || entry.contains(&format!("\"--home\" {}", quote_systemd(&home)))
        || entry.contains(&format!("\"--home\" {}", quote_desktop(&home)))
        || entry.trim_end().ends_with(&windows) || entry.contains(&format!("{windows} "))
}

pub fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).context("no home directory")
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        crate::private::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    crate::private::write_readable(path, text).with_context(|| format!("writing {}", path.display()))
}

fn remove(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

// ----------------------------------------------------------------- Linux

// All of these take the XDG base folders (`$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`:
// `crate::xdg`), never `$HOME/.config` by hand, so the start-at-login entries
// and the file manager's entries always land in the same tree.
fn unit_path(config: &Path) -> PathBuf {
    config.join("systemd/user/one-mk.service")
}

fn desktop_path(config: &Path) -> PathBuf {
    config.join("autostart/one-mk.desktop")
}

fn tray_desktop_path(config: &Path) -> PathBuf {
    config.join("autostart/one-mk-tray.desktop")
}

/// The (hidden) application entry that gives one-mk an app id, for the
/// GlobalShortcuts portal.
fn app_entry_path(data: &Path) -> PathBuf {
    data.join("applications").join(format!("{}.desktop", one_mk_proto::LINUX_APP_ID))
}

/// Whether [`install`] starts the service right away as well (a systemd
/// user service or a LaunchAgent does; an autostart entry or a Windows
/// startup entry only starts it at the next login).
pub fn starts_service_now() -> bool {
    if cfg!(target_os = "macos") {
        true
    } else if cfg!(target_os = "linux") {
        crate::xdg::config_home().is_ok_and(|config| systemd_for(&config))
    } else {
        false
    }
}

fn systemd_available() -> bool {
    ok("systemctl", &["--user", "show-environment"])
}

/// Whether the start-at-login entry for this config folder is a systemd
/// user service. The user's systemd is one per account and reads units only
/// from its own search path, whatever `$XDG_CONFIG_HOME` says here: for a
/// unit written anywhere else (a test run with a throwaway config folder),
/// `systemctl --user enable --now one-mk.service` would enable and start the
/// account's real one-mk instead, and uninstall would stop it. Such a folder
/// gets an autostart entry, which only that folder's session ever reads.
fn systemd_for(config: &Path) -> bool {
    systemd_available() && systemd_search_path().is_some_and(|dirs| unit_dir_searched(&dirs, &unit_path(config)))
}

/// systemd's own unit search path (`UnitPath`, space separated).
fn systemd_search_path() -> Option<String> {
    let out = Command::new("systemctl").args(["--user", "show", "--property=UnitPath"]).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    out.status.success().then(|| text.trim().trim_start_matches("UnitPath=").to_owned())
}

/// Whether `unit`'s folder is one of `search_path`'s folders.
fn unit_dir_searched(search_path: &str, unit: &Path) -> bool {
    let Some(dir) = unit.parent() else { return false };
    let same = |a: &Path, b: &Path| a == b || std::fs::canonicalize(a).is_ok_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b));
    search_path.split_whitespace().any(|d| same(Path::new(d), dir))
}

/// Quotes one word for a systemd `ExecStart=` line: inside double quotes a backslash and a quote are
/// escaped with a backslash, and systemd's own substitutions are doubled (`$$` for a dollar sign, `%%`
/// for a percent sign); a backslash before a dollar sign would be an unknown escape, not a literal one.
fn quote_systemd(word: &str) -> String {
    let mut out = String::from("\"");
    for c in word.chars() {
        match c {
            '"' | '\\' => { out.push('\\'); out.push(c); }
            '$' | '%' => { out.push(c); out.push(c); }
            _ => out.push(c),
        }
    }
    out + "\""
}

/// Quotes one word for a desktop entry's `Exec=` line, which is read twice: the file's string escapes
/// first (a literal backslash is `\\`), then the Exec quoting, which backslash-escapes a double quote,
/// backtick, dollar sign and backslash inside double quotes. So each of those is written with its Exec
/// backslash itself doubled (a literal backslash needs four), and `%` is doubled for the field codes.
fn quote_desktop(word: &str) -> String {
    let mut out = String::from("\"");
    for c in word.chars() {
        match c {
            '"' | '`' | '$' => { out.push_str("\\\\"); out.push(c); }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out + "\""
}

fn command_line(exe: &Path, args: &[String], quote: fn(&str) -> String) -> String {
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
        command_line(&l.exe, &l.args(), quote_systemd)
    )
}

fn desktop_entry(name: &str, exe: &Path, args: &[String]) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Share one mouse and keyboard\nExec={}\n\
         Terminal=false\nX-GNOME-Autostart-enabled=true\n",
        command_line(exe, args, quote_desktop)
    )
}

/// The files a Linux package ships, as (path relative to the package root,
/// text), for the program installed at `bindir` (`/usr/bin`). Built from
/// the same text `one-mk install` writes into the home folder, so the two
/// cannot drift apart. Packagers can write them with the
/// `packaging-files` example. The release's `.deb` (`scripts/bundle-deb.sh`)
/// ships the hidden app-id entry from here, which the GlobalShortcuts portal
/// needs installed; the service and the tray autostart are left to `one-mk
/// install`, per user.
pub fn packaged_files(bindir: &Path) -> Vec<(String, String)> {
    let launch = Launch { exe: bindir.join("one-mk"), tray: Some(bindir.join("one-mk-tray")), home: None, start_now: false };
    let daemon_entry = desktop_entry("one-mk", &launch.exe, &launch.args()).replace("X-GNOME-Autostart-enabled=true\n", "") + "NoDisplay=true\n";
    vec![
        ("usr/lib/systemd/user/one-mk.service".to_owned(), systemd_unit(&launch)),
        (format!("usr/share/applications/{}.desktop", one_mk_proto::LINUX_APP_ID), daemon_entry),
        // Not started by default: the tray only shows the state of a service the user turned on.
        ("etc/xdg/autostart/one-mk-tray.desktop".to_owned(), desktop_entry("one-mk tray", launch.tray.as_ref().unwrap(), &launch.tray_args(false))),
    ]
}

fn install_linux(l: &Launch) -> Result<String> {
    install_linux_in(&crate::xdg::config_home()?, &crate::xdg::data_home()?, l)
}

fn install_linux_in(config: &Path, data: &Path, l: &Launch) -> Result<String> {
    let mut msg = String::new();
    // Before the service starts: the daemon registers its app id at startup.
    let app = app_entry_path(data);
    write(&app, &(desktop_entry("one-mk", &l.exe, &l.args()).replace("X-GNOME-Autostart-enabled=true\n", "") + "NoDisplay=true\n"))?;
    if systemd_for(config) {
        let unit = unit_path(config);
        write(&unit, &systemd_unit(l))?;
        let _ = remove(&desktop_path(config)); // never start twice
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
        let entry = desktop_path(config);
        write(&entry, &desktop_entry("one-mk", &l.exe, &l.args()))?;
        msg += &format!("one-mk will start the next time you log in (systemd is not available here).\n  autostart entry: {}\n  to start it now:  one-mk run\n", entry.display());
    }
    let entry = tray_desktop_path(config);
    if let Some(tray) = &l.tray {
        write(&entry, &desktop_entry("one-mk tray", tray, &l.tray_args(false)))?;
        msg += &format!("  the tray icon will start at login too: {}\n", entry.display());
    } else if remove(&entry)? {
        // A copy without a tray must not leave an earlier copy's tray starting from its old place.
        msg += "  the tray icon entry of an earlier install was removed (this copy has no tray)\n";
    }
    msg += &format!("  desktop entry (for keyboard shortcuts): {}\n", app.display());
    msg += "  to undo:              one-mk uninstall";
    Ok(msg)
}

fn uninstall_linux(stop: bool) -> Result<bool> {
    uninstall_linux_in(&crate::xdg::config_home()?, &crate::xdg::data_home()?, stop)
}

/// What stops the service from starting at login: `--now` also stops the running one.
fn systemd_disable_args(stop: bool) -> &'static [&'static str] {
    if stop { &["--user", "disable", "--now", "one-mk.service"] } else { &["--user", "disable", "one-mk.service"] }
}

fn uninstall_linux_in(config: &Path, data: &Path, stop: bool) -> Result<bool> {
    let unit = unit_path(config);
    let mut did = false;
    if unit.exists() {
        // Only the unit systemd reads is the one it would disable.
        let systemd = systemd_for(config);
        if systemd {
            ok("systemctl", systemd_disable_args(stop));
        }
        did |= remove(&unit)?;
        if systemd {
            ok("systemctl", &["--user", "daemon-reload"]);
        }
    }
    did |= remove(&desktop_path(config))?;
    did |= remove(&tray_desktop_path(config))?;
    // The desktop entry the keyboard shortcuts are registered under belongs to the running copy.
    if stop {
        did |= remove(&app_entry_path(data))?;
    }
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
    // Login Items shows the entry as the app (its name and icon) when it knows which one it belongs to;
    // for a bare program inside the bundle it would say "one-mk" with the program's own icon.
    let associated = if exe.to_string_lossy().contains(".app/Contents/MacOS/") { "  <key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>com.one-mk.app</string>\n  </array>\n" } else { "" };
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
         {associated}\
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

/// The account's own home folder, from the user database: `$HOME` may point
/// elsewhere (a test run, a throwaway folder) while launchd does not.
fn account_home() -> Option<PathBuf> {
    let user = Command::new("id").arg("-un").output().ok()?;
    let user = String::from_utf8(user.stdout).ok()?;
    let read = Command::new("dscl").args([".", "-read", &format!("/Users/{}", user.trim()), "NFSHomeDirectory"]).output().ok()?;
    dscl_home(&String::from_utf8_lossy(&read.stdout))
}

/// `NFSHomeDirectory: /Users/me` (a path with spaces comes on the next line).
fn dscl_home(said: &str) -> Option<PathBuf> {
    let home = said.split_once("NFSHomeDirectory:")?.1.trim();
    (!home.is_empty()).then(|| PathBuf::from(home))
}

/// launchd holds one job per label for the whole login session, whatever
/// `$HOME` says. Loading or unloading from any other home folder would stop
/// or replace the installed oneMK, so only the account's own one may.
fn launchd_owns(home: &Path, account: Option<&Path>) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    account.is_some_and(|account| real(home) == real(account))
}

/// Writes a LaunchAgent and, if `load_now` and `home` is the account's own,
/// (re)loads it; whether it did.
fn mac_agent(home: &Path, label: &str, exe: &Path, args: &[String], log: &Path, keep_alive: bool, load_now: bool) -> Result<(PathBuf, bool)> {
    let plist = plist_path(home, label);
    write(&plist, &launch_agent(label, exe, args, log, keep_alive))?;
    let load_now = load_now && launchd_owns(home, account_home().as_deref());
    if load_now {
        let domain = gui_domain();
        ok("launchctl", &["bootout", &format!("{domain}/{label}")]); // replace an older copy
        if !ok("launchctl", &["bootstrap", &domain, &plist.display().to_string()]) {
            bail!("wrote {} but `launchctl bootstrap {domain}` failed", plist.display());
        }
    }
    Ok((plist, load_now))
}

fn install_macos(l: &Launch) -> Result<String> {
    let home = home_dir()?;
    let log = home.join("Library/Logs/one-mk.log");
    let (plist, started) = mac_agent(&home, LABEL, &l.exe, &l.args(), &log, true, l.start_now)?;
    let mut msg = format!(
        "one-mk will now start whenever you log in{}.\n  LaunchAgent: {}\n  log:         {}\n  macOS will ask for Accessibility and Input Monitoring permission the first time; allow one-mk.\n",
        if started { ", and is starting now" } else if l.start_now { " (not started now: $HOME is not your home folder)" } else { " (it is already running now)" },
        plist.display(),
        log.display()
    );
    if let Some(tray) = &l.tray {
        let tray_log = home.join("Library/Logs/one-mk-tray.log");
        let (p, _) = mac_agent(&home, TRAY_LABEL, tray, &l.tray_args(false), &tray_log, false, false)?;
        msg += &format!("  the menu bar icon will start at login too: {}\n", p.display());
    } else {
        // A copy without a tray must not leave an earlier copy's menu bar item starting from its old place.
        let old = plist_path(&home, TRAY_LABEL);
        if old.exists() {
            if launchd_owns(&home, account_home().as_deref()) {
                ok("launchctl", &["bootout", &format!("{}/{TRAY_LABEL}", gui_domain())]);
            }
            if remove(&old)? { msg += "  the menu bar icon entry of an earlier install was removed (this copy has no tray)\n"; }
        }
    }
    msg += "  to undo:     one-mk uninstall";
    Ok(msg)
}

/// `stop` also boots the running jobs out of launchd; without it their LaunchAgent files are
/// removed (so nothing starts at the next login) and the running copy is left alone.
fn uninstall_macos(stop: bool) -> Result<bool> {
    let home = home_dir()?;
    let owns = launchd_owns(&home, account_home().as_deref());
    let mut did = false;
    for label in [LABEL, TRAY_LABEL] {
        let plist = plist_path(&home, label);
        if plist.exists() {
            if owns && stop {
                ok("launchctl", &["bootout", &format!("{}/{label}", gui_domain())]);
            }
            did |= remove(&plist)?;
        }
    }
    Ok(did)
}

// --------------------------------------------------------------- Windows

/// One argument as Windows' command line parsing reads it back: quoted if it has a space, with the
/// backslashes before a closing (or embedded) quote doubled, so a folder spelled with a trailing
/// backslash does not swallow the quote and the arguments after it.
fn quote_win(w: &str) -> String {
    if !w.contains([' ', '\t']) { return w.to_owned(); }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in w.chars() {
        match c {
            '\\' => { backslashes += 1; out.push(c); }
            '"' => { out.push_str(&"\\".repeat(backslashes + 1)); out.push(c); backslashes = 0; }
            _ => { backslashes = 0; out.push(c); }
        }
    }
    out.push_str(&"\\".repeat(backslashes));
    out + "\""
}

/// The longest command Windows runs from a `Run` key (documented; a longer one is stored, then not started).
const RUN_VALUE_MAX: usize = 260;

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
    let length = value.encode_utf16().count();
    if length > RUN_VALUE_MAX {
        bail!("the startup command would be {length} characters long, and Windows starts nothing longer than {RUN_VALUE_MAX} characters from the startup list. Move oneMK or the settings folder to a shorter path, then run install again.");
    }
    if !ok("reg", &["add", RUN_KEY, "/v", "one-mk", "/t", "REG_SZ", "/d", &value, "/f"]) {
        bail!("could not add one-mk to the startup list (`reg add {RUN_KEY}` failed)");
    }
    Ok(format!(
        "one-mk will now start whenever you sign in to Windows{}.\n  startup entry: {RUN_KEY}\\one-mk\n  to start it now:  {}\n  to undo:          one-mk uninstall",
        if l.tray.is_some() { ", as a tray icon without a console window" } else { "" },
        if l.tray.is_some() { "one-mk-tray --start-daemon" } else { "one-mk run" },
    ))
}

fn uninstall_windows() -> Result<String> {
    let Some(value) = windows_startup_value()? else {
        return Ok("one-mk was not set to start at login; nothing to remove.".into());
    };
    let exe = std::env::current_exe()?;
    let folder = exe.parent().context("the current executable has no parent folder")?;
    if !windows_run_owned(&value, &folder.to_string_lossy()) {
        return Ok("the startup entry belongs to another copy of one-mk; left in place".into());
    }
    if !ok("reg", &["delete", RUN_KEY, "/v", "one-mk", "/f"]) {
        bail!("could not remove the one-mk startup entry (`reg delete` failed)");
    }
    Ok("one-mk will no longer start when you log in.\n  to turn it back on:  one-mk install".into())
}

#[cfg(not(windows))]
fn windows_startup_value() -> Result<Option<String>> { Ok(None) }

#[cfg(windows)]
fn windows_startup_value() -> Result<Option<String>> {
    use std::ffi::c_void;
    const RRF_RT_REG_SZ: u32 = 2;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(key: *mut c_void, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
    }
    let key = (0x80000001u32 as i32 as isize) as *mut c_void;
    let subkey: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\Run".encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = "one-mk".encode_utf16().chain(Some(0)).collect();
    let mut value = vec![0u16; 32768];
    let mut size = (value.len() * 2) as u32;
    let status = unsafe { RegGetValueW(key, subkey.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), value.as_mut_ptr().cast(), &mut size) };
    if status == ERROR_FILE_NOT_FOUND { return Ok(None); }
    if status != 0 { bail!("could not read the one-mk startup entry (Windows registry error {status}); left in place"); }
    value.truncate(size as usize / 2);
    if value.last() == Some(&0) { value.pop(); }
    Ok(Some(String::from_utf16(&value).context("startup entry is not valid UTF-16; left in place")?))
}

fn windows_run_owned(value: &str, folder: &str) -> bool {
    let value = value.trim();
    let program = if let Some(quoted) = value.strip_prefix('"') {
        let Some(end) = quoted.find('"') else { return false };
        &quoted[..end]
    } else {
        value.split_whitespace().next().unwrap_or("")
    };
    let normalize = |path: &str| path.strip_prefix(r"\\?\").unwrap_or(path).replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let program = normalize(program);
    let Some((parent, _)) = program.rsplit_once('\\') else { return false };
    parent == normalize(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_is_used_only_for_a_unit_folder_it_reads() {
        let real = "/home/u/.config/systemd/user.control /run/user/1000/systemd/transient /home/u/.config/systemd/user /etc/systemd/user /usr/lib/systemd/user";
        assert!(unit_dir_searched(real, &unit_path(Path::new("/home/u/.config"))));
        // A throwaway config folder: systemd would act on the real unit instead.
        assert!(!unit_dir_searched(real, &unit_path(Path::new("/tmp/omk-test/x/c"))));
        assert!(!unit_dir_searched("", &unit_path(Path::new("/home/u/.config"))));
        // The same folder through a link (a symlinked home) still counts.
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("c");
        std::fs::create_dir_all(config.join("systemd/user")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&config, dir.path().join("link")).unwrap();
            let search = format!("/etc/systemd/user {}", config.join("systemd/user").display());
            assert!(unit_dir_searched(&search, &unit_path(&dir.path().join("link"))));
        }
    }

    fn launch(home: Option<&str>) -> Launch {
        Launch { exe: "/opt/my apps/one-mk".into(), tray: None, home: home.map(PathBuf::from), start_now: true }
    }

    /// A run with `$HOME` elsewhere must never load or unload the installed
    /// oneMK's launchd job (one per label for the whole login session).
    #[test]
    fn only_the_account_home_drives_launchd() {
        assert_eq!(dscl_home("NFSHomeDirectory: /Users/me\n"), Some(PathBuf::from("/Users/me")));
        assert_eq!(dscl_home("NFSHomeDirectory:\n /Users/Jane Doe\n"), Some(PathBuf::from("/Users/Jane Doe")));
        assert_eq!(dscl_home("No such key: NFSHomeDirectory\n"), None);
        let account = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(launchd_owns(account.path(), Some(account.path())));
        assert!(!launchd_owns(other.path(), Some(account.path())));
        assert!(!launchd_owns(account.path(), None), "unknown account home: hands off");
        #[cfg(unix)]
        {
            let link = other.path().join("same");
            std::os::unix::fs::symlink(account.path(), &link).unwrap();
            assert!(launchd_owns(&link, Some(account.path())), "the same folder by another path");
        }
    }

    #[test]
    fn windows_startup_ownership_is_folder_scoped() {
        assert!(windows_run_owned(r#""C:\Users\Jürgen\one-mk\one-mk.exe" run"#, r"C:\Users\Jürgen\one-mk"));
        let folder = r"C:\Program Files\one-mk\";
        for value in [r#""c:\PROGRAM FILES\ONE-MK\one-mk.exe" run"#, r#""C:\Program Files\one-mk\one-mk-tray.exe" --start-daemon"#] {
            assert!(windows_run_owned(value, folder), "{value}");
        }
        assert!(windows_run_owned(r#""C:/Program Files/one-mk/one-mk.exe" run"#, folder));
        assert!(windows_run_owned(r#""\\?\C:\Program Files\one-mk\one-mk.exe" run"#, folder));
        assert!(windows_run_owned(r"C:\one-mk\one-mk.exe run", r"c:\ONE-MK\\"));
        for value in [r#""C:\Portable\one-mk.exe" run"#, r#""C:\Program Files\one-mk-other\one-mk.exe" run"#, r#""C:\Program Files\one-mk\sub\one-mk.exe" run"#, r#""C:\Program Files\one-mk\one-mk.exe run"#, ""] {
            assert!(!windows_run_owned(value, folder), "{value}");
        }
    }

    #[test]
    fn systemd_unit_quotes_the_path() {
        let unit = systemd_unit(&launch(None));
        assert!(unit.contains("ExecStart=\"/opt/my apps/one-mk\" \"run\"\n"), "{unit}");
        assert!(unit.contains("WantedBy=graphical-session.target"));
        assert!(unit.contains("Restart=on-failure"));
        let with_home = systemd_unit(&launch(Some("/tmp/a b")));
        assert!(with_home.contains("ExecStart=\"/opt/my apps/one-mk\" \"run\" \"--home\" \"/tmp/a b\"\n"), "{with_home}");
        assert!(entry_starts(&with_home, Some(Path::new("/tmp/a b"))) && !entry_starts(&with_home, None));
    }

    // Linux package contents: Windows would join the paths with backslashes.
    #[cfg(unix)]
    #[test]
    fn packaged_files_match_what_install_writes() {
        let files = packaged_files(Path::new("/usr/bin"));
        let unit = &files.iter().find(|(p, _)| p.ends_with("one-mk.service")).unwrap().1;
        assert_eq!(*unit, systemd_unit(&Launch { exe: "/usr/bin/one-mk".into(), tray: None, home: None, start_now: false }));
        assert!(unit.contains("ExecStart=\"/usr/bin/one-mk\" \"run\"\n"));
        let tray = &files.iter().find(|(p, _)| p.starts_with("etc/xdg/autostart/")).unwrap().1;
        assert!(tray.contains("Exec=\"/usr/bin/one-mk-tray\"\n"), "{tray}");
        let hidden = &files.iter().find(|(p, _)| p.ends_with("io.github.onemk.OneMk.desktop")).unwrap().1;
        assert!(hidden.contains("NoDisplay=true") && !hidden.contains("Autostart"), "{hidden}");
    }

    #[test]
    fn hostile_paths_never_become_directives() {
        let evil = "/opt/x\nExecStop=touch /tmp/injected\n/one-mk";
        for l in [
            Launch { exe: evil.into(), ..launch(None) },
            Launch { tray: Some(evil.into()), ..launch(None) },
            Launch { home: Some(evil.into()), ..launch(None) },
            Launch { exe: "/opt/a\rb".into(), ..launch(None) },
            Launch { exe: "/opt/a\0b".into(), ..launch(None) },
        ] {
            assert!(l.check().is_err());
            // `install` refuses before anything is written anywhere.
            assert!(install(&l).is_err());
        }
        assert!(launch(Some("/tmp/a b")).check().is_ok());
    }

    #[test]
    fn quoting_escapes_specials() {
        assert_eq!(quote_systemd(r#"a"b$c%d\e"#), r#""a\"b$$c%%d\\e""#);
        assert_eq!(quote_desktop(r#"a"b$c%d\e`f"#), r#""a\\"b\\$c%%d\\\\e\\`f""#);
    }

    /// systemd's reading of one `ExecStart=` word: C escapes inside quotes, `$$` and `%%` for one character.
    fn systemd_words(line: &str) -> Vec<String> {
        let (mut words, mut word, mut quoted, mut chars) = (vec![], String::new(), false, line.chars().peekable());
        while let Some(c) = chars.next() {
            match c {
                '"' => { quoted = !quoted; if !quoted { words.push(std::mem::take(&mut word)); } }
                '\\' if quoted => word.push(chars.next().expect("a complete escape")),
                '$' | '%' if quoted => { assert_eq!(chars.next(), Some(c), "a lone {c} is a substitution"); word.push(c); }
                ' ' if !quoted => {}
                _ => word.push(c),
            }
        }
        words
    }

    /// The desktop entry reading: string escapes over the whole value first, then Exec unquoting.
    fn exec_words(value: &str) -> Vec<String> {
        let mut text = String::new();
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            if c != '\\' { text.push(c); continue; }
            text.push(match chars.next().expect("a complete escape") { 's' => ' ', 'n' => '\n', 't' => '\t', 'r' => '\r', '\\' => '\\', other => panic!("{other} is not a string escape") });
        }
        let (mut words, mut word, mut quoted, mut chars) = (vec![], String::new(), false, text.chars());
        while let Some(c) = chars.next() {
            match c {
                '"' => { quoted = !quoted; if !quoted { words.push(std::mem::take(&mut word)); } }
                '\\' if quoted => word.push(chars.next().expect("a complete escape")),
                '%' if quoted => { assert_eq!(chars.next(), Some('%'), "a field code"); word.push('%'); }
                ' ' if !quoted => {}
                _ => word.push(c),
            }
        }
        words
    }

    #[test]
    fn login_commands_read_back_as_the_same_arguments_for_systemd_and_the_desktop() {
        let l = Launch { exe: r#"/opt/a$b/50%/back\slash "q" `t`/one-mk"#.into(), tray: None, home: Some(r"/tmp/h\o$me %u".into()), start_now: false };
        let want: Vec<String> = std::iter::once(l.exe.display().to_string()).chain(l.args()).collect();
        let unit = systemd_unit(&l);
        let exec = unit.lines().find_map(|line| line.strip_prefix("ExecStart=")).unwrap();
        assert!(exec.contains("a$$b"), "{exec}");
        assert_eq!(systemd_words(exec), want);
        let entry = desktop_entry("one-mk", &l.exe, &l.args());
        let exec = entry.lines().find_map(|line| line.strip_prefix("Exec=")).unwrap();
        assert_eq!(exec_words(exec), want);
        // The window recognises its own entry in either form.
        assert!(entry_starts(&unit, l.home.as_deref()) && entry_starts(&entry, l.home.as_deref()));
    }

    #[test]
    fn autostart_and_launchd_switches_are_read_from_their_files() {
        assert!(!autostart_disabled("[Desktop Entry]\nExec=x\nX-GNOME-Autostart-enabled=true\n"));
        assert!(autostart_disabled("[Desktop Entry]\nExec=x\nX-GNOME-Autostart-enabled=false\n"));
        assert!(autostart_disabled("[Desktop Entry]\nHidden=true\n"));
        assert!(!autostart_disabled("[Desktop Entry]\nExec=\"Hidden=true\"\n"));
        let listing = "disabled services = {\n\t\"com.apple.x\" => enabled\n\t\"com.one-mk.daemon\" => disabled\n\t\"com.one-mk.tray\" => enabled\n}\n";
        assert!(launchd_disabled(listing, "com.one-mk.daemon"));
        assert!(!launchd_disabled(listing, "com.one-mk.tray"));
        assert!(!launchd_disabled(listing, "com.one-mk"));
        assert!(launchd_disabled("\t\"com.one-mk.daemon\" => true,\n", "com.one-mk.daemon"));
    }

    #[test]
    fn windows_arguments_survive_a_trailing_backslash() {
        assert_eq!(quote_win(r"C:\Program Files\x\"), r#""C:\Program Files\x\\""#);
        assert_eq!(quote_win(r"C:\x\"), r"C:\x\");
        assert_eq!(quote_win(r"C:\a b"), r#""C:\a b""#);
        let l = Launch { home: Some(r"D:\my home\".into()), ..launch(None) };
        let value = run_value(&l);
        assert!(value.ends_with(r#"--home "D:\my home\\""#), "{value}");
        assert!(entry_starts(&value, l.home.as_deref()));
        // The login command is limited to 260 characters.
        let deep = Launch { exe: format!(r"C:\{}\one-mk.exe", "d".repeat(300)).into(), ..launch(None) };
        assert!(install_windows(&deep).unwrap_err().to_string().contains("260 characters"));
        assert!(run_value(&deep).len() > RUN_VALUE_MAX);
    }

    #[test]
    fn reinstalling_without_a_tray_removes_the_old_tray_login_entry() {
        let dir = tempfile::tempdir().unwrap();
        let (config, data) = (dir.path().join("config"), dir.path().join("data"));
        // A throwaway config folder is not in systemd's search path: autostart entries only.
        let with = Launch { tray: Some("/opt/x/one-mk-tray".into()), start_now: false, ..launch(None) };
        install_linux_in(&config, &data, &with).unwrap();
        let tray = tray_desktop_path(&config);
        assert!(tray.exists());
        install_linux_in(&config, &data, &Launch { tray: None, ..with }).unwrap();
        assert!(!tray.exists());
        assert!(desktop_path(&config).exists(), "the daemon's own entry stays");
    }

    /// Turning off "Start at login" must not end the sharing that is going on: only the full
    /// uninstall stops the running service, and only that one takes the shortcuts' entry away.
    #[test]
    fn disabling_login_does_not_stop_the_running_copy() {
        assert!(systemd_disable_args(true).contains(&"--now"));
        assert!(!systemd_disable_args(false).contains(&"--now"));
        for stop in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (config, data) = (dir.path().join("config"), dir.path().join("data"));
            let with = Launch { tray: Some("/opt/x/one-mk-tray".into()), start_now: false, ..launch(None) };
            install_linux_in(&config, &data, &with).unwrap();
            assert!(uninstall_linux_in(&config, &data, stop).unwrap());
            assert!(!desktop_path(&config).exists() && !tray_desktop_path(&config).exists(), "still starts at login");
            assert_eq!(app_entry_path(&data).exists(), !stop, "stop={stop}");
            assert!(!uninstall_linux_in(&config, &data, stop).unwrap(), "nothing left to remove");
        }
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
        assert!(entry_starts(&plist, Some(Path::new("/Users/a&b"))));
        assert!(!entry_starts(&plist, Some(Path::new("/Users/a"))) && !entry_starts(&plist, None));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<string>/Users/me/Library/Logs/one-mk.log</string>"));
        assert_eq!(plist.matches("<dict>").count(), plist.matches("</dict>").count());
        let tray = launch_agent(TRAY_LABEL, Path::new("/t"), &[], Path::new("/l"), false);
        assert!(!tray.contains("KeepAlive") && !tray.contains("AssociatedBundleIdentifiers"), "a bare program belongs to no app");
        // Inside the app bundle: Login Items names and draws it as the app.
        let in_app = launch_agent(LABEL, Path::new("/Applications/one-mk.app/Contents/MacOS/one-mk"), &[], Path::new("/l"), true);
        assert!(in_app.contains("<key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>com.one-mk.app</string>\n  </array>"), "{in_app}");
        assert_eq!(in_app.matches("<array>").count(), in_app.matches("</array>").count());
    }

    #[test]
    fn windows_login_entry_prefers_the_tray() {
        assert_eq!(run_value(&launch(None)), "\"/opt/my apps/one-mk\" run");
        let l = Launch { tray: Some("C:/Program Files/one-mk/one-mk-tray.exe".into()), ..launch(Some("D:/x"))};
        assert_eq!(run_value(&l), "\"C:/Program Files/one-mk/one-mk-tray.exe\" --start-daemon --home D:/x");
        assert!(entry_starts(&run_value(&l), Some(Path::new("D:/x"))));
        assert!(!entry_starts(&run_value(&l), Some(Path::new("D:/"))) && !entry_starts(&run_value(&l), None));
    }

    #[test]
    fn files_live_where_the_systems_look() {
        // Under the XDG folders, wherever they are: not under a hard-coded
        // `$HOME/.config` while the file manager's entries follow `$XDG_DATA_HOME`.
        let (config, data) = (Path::new("/cfg"), Path::new("/dat/share"));
        assert_eq!(unit_path(config), Path::new("/cfg/systemd/user/one-mk.service"));
        assert_eq!(desktop_path(config), Path::new("/cfg/autostart/one-mk.desktop"));
        assert_eq!(tray_desktop_path(config), Path::new("/cfg/autostart/one-mk-tray.desktop"));
        assert_eq!(app_entry_path(data), Path::new("/dat/share/applications/io.github.onemk.OneMk.desktop"));
        assert_eq!(plist_path(Path::new("/home/u"), LABEL), Path::new("/home/u/Library/LaunchAgents/com.one-mk.daemon.plist"));
    }

    /// The settings folder is read back out of every kind of login entry exactly as it was
    /// written, whatever the folder's name holds: spaces, quotes, `%`, `$`, backslashes, `&`.
    #[test]
    fn the_registered_home_is_read_back_from_every_kind_of_entry() {
        let homes = ["/tmp/plain", "/tmp/with space/x", "/tmp/q\"uote", "/tmp/50%/$HOME/`x`", "/tmp/back\\slash", "/tmp/a&b<c>", "C:\\Users\\me\\One Mk\\", "/tmp/日本"];
        for home in homes {
            let l = Launch { exe: "/opt/my apps/one-mk".into(), tray: Some("/opt/my apps/one-mk-tray".into()), home: Some(home.into()), start_now: false };
            let plain = Launch { exe: l.exe.clone(), tray: None, home: l.home.clone(), start_now: false };
            let expect = Some(PathBuf::from(home));
            assert_eq!(entry_home(&systemd_unit(&l)), expect, "systemd: {home}");
            assert_eq!(entry_home(&desktop_entry("one-mk", &l.exe, &l.args())), expect, "desktop: {home}");
            assert_eq!(entry_home(&desktop_entry("one-mk tray", l.tray.as_ref().unwrap(), &l.tray_args(false))), expect, "tray desktop: {home}");
            let plist = launch_agent(LABEL, &l.exe, &l.args(), Path::new("/l"), true);
            assert_eq!(entry_home(&plist), expect, "plist: {home}");
            // A quote cannot be in a Windows path (and `quote_win` leaves one alone without a space).
            if !home.contains('"') {
                assert_eq!(entry_home(&run_value(&l)), expect, "windows tray: {home}");
                assert_eq!(entry_home(&run_value(&plain)), expect, "windows run: {home}");
            }
            // Both files of one install, as `login_entry` joins them.
            assert_eq!(entry_home(&(systemd_unit(&plain) + &desktop_entry("one-mk", &plain.exe, &plain.args()))), expect, "both: {home}");
        }
        // The usual folder: no `--home` anywhere.
        let usual = launch(None);
        for text in [systemd_unit(&usual), desktop_entry("one-mk", &usual.exe, &usual.args()), launch_agent(LABEL, &usual.exe, &usual.args(), Path::new("/l"), true), run_value(&usual)] {
            assert_eq!(entry_home(&text), None, "{text}");
        }
        // A program path that merely contains the word, or `--home` last with nothing after it.
        assert_eq!(entry_home("ExecStart=\"/opt/--home/one-mk\" run\n"), None);
        assert_eq!(entry_home("\"C:\\a\\one-mk.exe\" run --home"), None);
    }

    /// The entries the desktop must find are written readable but never
    /// group-writable, in folders made private, whatever the umask.
    #[cfg(unix)]
    #[test]
    fn login_entries_are_written_with_fixed_modes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/autostart/one-mk.desktop");
        write(&path, "[Desktop Entry]\n").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o133, 0, "readable by the owner, never group-writable");
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
