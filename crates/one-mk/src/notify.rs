//! "Files received" desktop notifications.
//!
//! The notification itself sits behind [`Notifier`] so the batching logic
//! (`track.rs`) is testable without a desktop. [`Desktop`] is the real
//! thing:
//!
//! - Linux and other Unix desktops: `org.freedesktop.Notifications` over
//!   the session D-Bus (zbus is already linked for the input portals), with
//!   a "Show in folder" action that opens the file manager on the file
//!   (`org.freedesktop.FileManager1`, falling back to `xdg-open`). If there
//!   is no notification service on the bus, `notify-send` is tried.
//! - macOS: `osascript` (`display notification`). That API has no buttons,
//!   so there is no "Show in folder" there.
//! - Windows: not done yet (see the TODO below); the transfer still
//!   prints its "Received ..." line.
//!
//! Nothing here runs until a transfer finishes: no task, timer or D-Bus
//! connection is kept while idle. A notification that offers an action
//! keeps one connection open until it is used or dismissed, at most
//! [`ACTION_WAIT`].

// Tests never raise real notifications, so the platform code is unused there.
#![cfg_attr(test, allow(dead_code))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long a notification's "Show in folder" stays live.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
const ACTION_WAIT: Duration = Duration::from_secs(5 * 60);

/// One finished transfer: everything that arrived from one send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesReceived {
    pub peer: String,
    /// Names as saved, relative to `folder`, `/`-separated.
    pub names: Vec<String>,
    pub bytes: u64,
    /// The download folder.
    pub folder: PathBuf,
}

/// Raises a notification. Called once per transfer, from inside the tokio
/// runtime; it must return at once (do the slow part on a task).
pub trait Notifier: Send + Sync + 'static {
    fn files_received(&self, n: FilesReceived);
}

/// `--no-notify` for this run.
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Turns notifications off for the rest of this process (`one-mk run --no-notify`).
pub fn disable() {
    DISABLED.store(true, Ordering::Relaxed);
}

pub fn disabled() -> bool {
    DISABLED.load(Ordering::Relaxed)
}

impl FilesReceived {
    /// Top-level entries: a folder counts once, with its number of files.
    fn groups(&self) -> Vec<(String, usize)> {
        let mut groups: Vec<(String, usize)> = Vec::new();
        for name in &self.names {
            let top = name.split('/').next().unwrap_or(name);
            let is_folder = name.contains('/');
            let key = if is_folder { format!("{top}/") } else { top.to_owned() };
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => groups.push((key, 1)),
            }
        }
        groups
    }

    pub fn title(&self) -> String {
        let peer = display(&self.peer);
        match self.names.as_slice() {
            [one] => format!("{} received from {peer}", display(one.rsplit('/').next().unwrap_or(one))),
            many => format!("{} files received from {peer}", many.len()),
        }
    }

    /// Plain text; [`escape_markup`] before handing it to a server that
    /// reads markup.
    pub fn body(&self) -> String {
        const SHOWN: usize = 3;
        let groups = self.groups();
        let mut parts: Vec<String> = groups
            .iter()
            .take(SHOWN)
            .map(|(name, n)| match (name.ends_with('/'), n) {
                (true, 1) => format!("{} (1 file)", display(name)),
                (true, n) => format!("{} ({n} files)", display(name)),
                (false, _) => display(name),
            })
            .collect();
        if groups.len() > SHOWN {
            parts.push(format!("and {} more", groups.len() - SHOWN));
        }
        if self.names.len() == 1 {
            // The title has the name; the body says where it is.
            return format!("{} saved to {}", human(self.bytes), self.folder.display());
        }
        format!("{}. {} in {}", parts.join(", "), human(self.bytes), self.folder.display())
    }

    /// What "Show in folder" opens: the one file or folder that arrived,
    /// or the download folder when it is several things.
    pub fn reveal(&self) -> PathBuf {
        match self.groups().as_slice() {
            [(name, _)] => self.folder.join(name.trim_end_matches('/')),
            _ => self.folder.clone(),
        }
    }
}

/// A peer-chosen name made safe to show: no control or bidi characters,
/// bounded length (the same cleaning device names get).
fn display(name: &str) -> String {
    one_mk_proto::sanitize_name(name)
}

fn human(n: u64) -> String {
    one_mk_ctl::status::human_bytes(n)
}

/// Notification servers read a little markup (`<b>`, `<a href>`) in the body.
/// File names are chosen by the other computer, so none of it may be live.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Says that something went wrong, for commands started from a file
/// manager menu where nobody sees stderr. Waits until the notification
/// is shown, so call it before the process exits; never fails.
pub async fn failure(title: &str, body: &str) {
    if disabled() {
        return;
    }
    sys::failure(title, body).await;
}

/// A plain note for the user (layout learning's question, or what it did).
/// The caller prints the same words, so nothing is lost where there are no
/// desktop notifications (Windows, for now). Never fails.
pub async fn note(title: &str, body: &str) {
    if disabled() || cfg!(test) {
        return;
    }
    sys::note(title, body).await;
}

/// Says that something changed (broadcast started or stopped), on a task of
/// its own: returns at once. Never during tests, and not with `--no-notify`.
pub fn info(title: String, body: String) {
    if disabled() || cfg!(test) {
        return;
    }
    tokio::spawn(async move { sys::note(&title, &body).await });
}

pub async fn sent(count: usize, peer: &str) {
    let body = format!("Sent {count} files to {}", display(peer));
    println!("{body}");
    if !disabled() { sys::failure("one-mk files sent", &body).await; }
}

/// The real notifier for this platform.
pub struct Desktop;

impl Notifier for Desktop {
    fn files_received(&self, n: FilesReceived) {
        tokio::spawn(async move { sys::show(n).await });
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod sys {
    use super::*;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::path::Path;
    use std::process::Stdio;
    use tokio::process::Command;
    use zbus::zvariant::Value;

    pub async fn show(n: FilesReceived) {
        match via_dbus(&n).await {
            Ok(()) => {}
            Err(e) => {
                tracing::debug!("notification over D-Bus failed ({e:#}); trying notify-send");
                if let Err(e) = via_notify_send(&n).await {
                    tracing::debug!("notify-send failed too: {e:#}; no notification");
                }
            }
        }
    }

    /// A plain error notification: D-Bus, else `notify-send`, else stderr.
    pub async fn failure(title: &str, body: &str) {
        let success = title == "one-mk files sent";
        let (icon, category) = if success { ("document-send", "transfer.complete") } else { ("dialog-error", "transfer.error") };
        if !message(title, body, icon, category).await {
            eprintln!("one-mk: {title}: {body}");
        }
    }

    /// A note: D-Bus, else `notify-send`, else nothing (the daemon printed it).
    pub async fn note(title: &str, body: &str) {
        message(title, body, "input-mouse", "device").await;
    }

    /// Shows a notification without actions; false if neither D-Bus nor
    /// `notify-send` could.
    async fn message(title: &str, body: &str, icon: &str, category: &str) -> bool {
        let shown = async {
            let conn = zbus::Connection::session().await?;
            let proxy = zbus::Proxy::new(
                &conn,
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
            )
            .await?;
            let hints: HashMap<&str, Value> = HashMap::from([("category", Value::from(category))]);
            proxy
                .call::<_, _, u32>("Notify", &("one-mk", 0u32, icon, title, escape_markup(body), Vec::<&str>::new(), hints, -1i32))
                .await?;
            anyhow::Ok(())
        };
        let Err(e) = shown.await else { return true };
        tracing::debug!("notification over D-Bus failed ({e:#}); trying notify-send");
        let status = Command::new("notify-send")
            .args(["--app-name=one-mk", &format!("--icon={icon}"), "--"])
            .arg(title)
            .arg(escape_markup(body))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        status.is_ok_and(|s| s.success())
    }

    async fn via_dbus(n: &FilesReceived) -> anyhow::Result<()> {
        via_bus(&zbus::Connection::session().await?, n, ACTION_WAIT).await
    }

    /// Shows the notification on `conn` and, if "Show in folder" is clicked
    /// within `wait`, opens the file manager.
    pub(super) async fn via_bus(conn: &zbus::Connection, n: &FilesReceived, wait: Duration) -> anyhow::Result<()> {
        let proxy = zbus::Proxy::new(
            conn,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await?;
        // Subscribe first: the click can't come before the answer to Notify, but
        // the stream must exist by then.
        let mut signals = proxy.receive_all_signals().await?;
        let hints: HashMap<&str, Value> = HashMap::from([("category", Value::from("transfer.complete"))]);
        let id: u32 = proxy
            .call(
                "Notify",
                &(
                    "one-mk",
                    0u32,
                    "folder-download",
                    n.title(),
                    escape_markup(&n.body()),
                    vec!["show", "Show in folder"],
                    hints,
                    -1i32,
                ),
            )
            .await?;
        let wait_for_click = async {
            while let Some(msg) = signals.next().await {
                let header = msg.header();
                match header.member().map(|m| m.as_str()) {
                    Some("ActionInvoked") => {
                        if let Ok((nid, key)) = msg.body().deserialize::<(u32, String)>()
                            && nid == id
                            && key == "show"
                        {
                            return true;
                        }
                    }
                    // Reason 2: the user dismissed it. (A banner that merely
                    // times out is kept in the shell's list and can still be clicked.)
                    Some("NotificationClosed") => {
                        if let Ok((nid, reason)) = msg.body().deserialize::<(u32, u32)>()
                            && nid == id
                            && reason != 1
                        {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            false
        };
        if tokio::time::timeout(wait, wait_for_click).await == Ok(true) {
            reveal(conn, &n.reveal()).await;
        }
        Ok(())
    }

    /// Opens the file manager on `path` (selecting it), or failing that
    /// whatever `xdg-open` does with the folder.
    async fn reveal(conn: &zbus::Connection, path: &Path) {
        let shown = async {
            let fm = zbus::Proxy::new(
                conn,
                "org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
            )
            .await?;
            fm.call::<_, _, ()>("ShowItems", &(vec![file_uri(path)], "")).await
        };
        if let Err(e) = shown.await {
            tracing::debug!("FileManager1 is not available ({e}); using xdg-open");
            let folder = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
            // An absolute path, so it can't be read as an option.
            let _ = Command::new("xdg-open").arg(folder).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        }
    }

    /// `file:///a/b%20c`
    pub(super) fn file_uri(path: &Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let mut out = String::from("file://");
        for &b in path.as_os_str().as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    /// Without a notification service on the bus (or a different bus setup),
    /// libnotify's tool may still reach one. `--action` needs libnotify 0.8;
    /// older ones fail on the unknown option and we send it without.
    async fn via_notify_send(n: &FilesReceived) -> anyhow::Result<()> {
        let run = |action: bool| {
            let mut c = Command::new("notify-send");
            c.arg("--app-name=one-mk").arg("--icon=folder-download");
            if action {
                c.arg("--action=show=Show in folder").arg("--wait");
            }
            c.arg("--").arg(n.title()).arg(escape_markup(&n.body()));
            c.stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
            c
        };
        let out = tokio::time::timeout(ACTION_WAIT, run(true).output()).await;
        match out {
            Ok(Ok(o)) if o.status.success() => {
                if String::from_utf8_lossy(&o.stdout).trim() == "show" {
                    let path = n.reveal();
                    let folder = if path.is_dir() { path } else { path.parent().map(Path::to_path_buf).unwrap_or(path) };
                    let _ = Command::new("xdg-open").arg(folder).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
                }
                Ok(())
            }
            Ok(Ok(_)) => {
                run(false).status().await?;
                Ok(())
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Ok(()), // nobody clicked; the child is killed on drop
        }
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use super::*;
    use std::process::Stdio;

    pub async fn failure(title: &str, body: &str) {
        notify(body, title).await;
    }

    pub async fn note(title: &str, body: &str) {
        notify(body, title).await;
    }

    /// Title and body go in as arguments, never into the script text.
    pub async fn show(n: FilesReceived) {
        notify(&n.body(), &n.title()).await;
    }

    async fn notify(body: &str, title: &str) {
        let res = tokio::process::Command::new("osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display notification (item 1 of argv) with title (item 2 of argv)",
                "-e",
                "end run",
            ])
            .arg(body)
            .arg(title)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if let Err(e) = res {
            tracing::debug!("osascript failed: {e}; no notification");
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::*;

    // TODO: a Windows toast (with a "Show in folder" button) needs an app
    // identity (AppUserModelID) registered for the program, which one-mk
    // does not have yet.
    pub async fn show(n: FilesReceived) {
        tracing::info!("not notifying about {} (notifications are not implemented on Windows yet)", n.title());
    }

    /// The SendTo script keeps its window open on failure instead.
    pub async fn failure(title: &str, body: &str) {
        eprintln!("one-mk: {title}: {body}");
    }

    /// Printed by the caller already (see the TODO above about toasts).
    pub async fn note(title: &str, _body: &str) {
        tracing::debug!("not notifying {title:?} (notifications are not implemented on Windows yet)");
    }
}

#[cfg(not(any(unix, windows)))]
mod sys {
    use super::*;
    pub async fn show(_: FilesReceived) {}
    pub async fn failure(title: &str, body: &str) {
        eprintln!("one-mk: {title}: {body}");
    }
    pub async fn note(_: &str, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn got(peer: &str, names: &[&str], bytes: u64) -> FilesReceived {
        FilesReceived {
            peer: peer.into(),
            names: names.iter().map(|s| (*s).to_owned()).collect(),
            bytes,
            folder: PathBuf::from("/home/u/Downloads/one-mk"),
        }
    }

    #[test]
    fn one_file_names_it() {
        let n = got("desk", &["report.pdf"], 2_500_000);
        assert_eq!(n.title(), "report.pdf received from desk");
        assert_eq!(n.body(), "2.5 MB saved to /home/u/Downloads/one-mk");
        assert_eq!(n.reveal(), PathBuf::from("/home/u/Downloads/one-mk/report.pdf"));
    }

    #[test]
    fn several_files_give_count_and_names() {
        let n = got("desk", &["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"], 5_000);
        assert_eq!(n.title(), "5 files received from desk");
        assert_eq!(n.body(), "a.txt, b.txt, c.txt, and 2 more. 5.0 KB in /home/u/Downloads/one-mk");
        assert_eq!(n.reveal(), PathBuf::from("/home/u/Downloads/one-mk"), "several things: the folder");
    }

    #[test]
    fn a_folder_counts_once() {
        let n = got("desk", &["photos/a.jpg", "photos/sub/b.jpg", "photos/c.jpg"], 9_000_000);
        assert_eq!(n.title(), "3 files received from desk");
        assert_eq!(n.body(), "photos/ (3 files). 9.0 MB in /home/u/Downloads/one-mk");
        assert_eq!(n.reveal(), PathBuf::from("/home/u/Downloads/one-mk/photos"), "one folder: that folder");
        let mixed = got("desk", &["photos/a.jpg", "notes.txt"], 1);
        assert_eq!(mixed.body(), "photos/ (1 file), notes.txt. 1 B in /home/u/Downloads/one-mk");
        assert_eq!(mixed.reveal(), PathBuf::from("/home/u/Downloads/one-mk"));
    }

    /// The whole D-Bus conversation against a fake notification server and
    /// file manager on a private bus: the notification carries the title,
    /// body and "Show in folder", and a click opens the file manager on the
    /// right file. (Skipped where `dbus-daemon` isn't installed.)
    #[cfg(all(unix, not(target_os = "macos")))]
    #[tokio::test]
    async fn show_in_folder_on_a_private_bus() {
        use std::collections::HashMap;
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::sync::mpsc;
        use zbus::object_server::SignalEmitter;

        type Seen = (String, String, Vec<String>);
        struct Notifs(mpsc::UnboundedSender<Seen>);
        #[zbus::interface(name = "org.freedesktop.Notifications")]
        impl Notifs {
            #[allow(clippy::too_many_arguments)]
            async fn notify(
                &self,
                _app: String,
                _replaces: u32,
                _icon: String,
                summary: String,
                body: String,
                actions: Vec<String>,
                _hints: HashMap<String, zbus::zvariant::OwnedValue>,
                _timeout: i32,
                #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
            ) -> u32 {
                let _ = self.0.send((summary, body, actions));
                let emitter = emitter.into_owned();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let _ = Notifs::action_invoked(&emitter, 7, "show").await;
                });
                7
            }
            #[zbus(signal)]
            async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, key: &str) -> zbus::Result<()>;
        }
        struct Files(mpsc::UnboundedSender<Vec<String>>);
        #[zbus::interface(name = "org.freedesktop.FileManager1")]
        impl Files {
            fn show_items(&self, uris: Vec<String>, _startup_id: String) {
                let _ = self.0.send(uris);
            }
        }

        let Ok(mut bus) = tokio::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let mut addr = String::new();
        BufReader::new(bus.stdout.take().unwrap()).read_line(&mut addr).await.unwrap();
        let addr = addr.trim();

        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        let (files_tx, mut files) = mpsc::unbounded_channel();
        let _server = zbus::connection::Builder::address(addr)
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .name("org.freedesktop.FileManager1")
            .unwrap()
            .serve_at("/org/freedesktop/Notifications", Notifs(seen_tx))
            .unwrap()
            .serve_at("/org/freedesktop/FileManager1", Files(files_tx))
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(addr).unwrap().build().await.unwrap();

        let n = got("desk", &["my report.pdf"], 2_500_000);
        sys::via_bus(&client, &n, Duration::from_secs(5)).await.unwrap();

        let (summary, body, actions) = seen.try_recv().expect("a notification was sent");
        assert_eq!(summary, "my report.pdf received from desk");
        assert_eq!(body, "2.5 MB saved to /home/u/Downloads/one-mk");
        assert_eq!(actions, ["show", "Show in folder"]);
        assert!(seen.try_recv().is_err(), "once");
        // The click made the file manager select the file.
        assert_eq!(files.try_recv().unwrap(), ["file:///home/u/Downloads/one-mk/my%20report.pdf"]);
    }

    #[test]
    fn what_the_other_computer_chose_is_made_safe() {
        let n = got("evil\u{202E}pc", &["<a href=\"x\">click</a>.txt", "bad\u{202E}gnp.exe"], 1);
        assert!(!n.title().contains('\u{202E}') && !n.body().contains('\u{202E}'));
        let shown = escape_markup(&n.body());
        assert!(!shown.contains('<') && !shown.contains('>'), "{shown}");
        assert!(shown.contains("&lt;a href"), "{shown}");
        assert_eq!(escape_markup("a & b <i>"), "a &amp; b &lt;i&gt;");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(sys::file_uri(std::path::Path::new("/home/u/a b/ü#1.txt")), "file:///home/u/a%20b/%C3%BC%231.txt");
    }
}
