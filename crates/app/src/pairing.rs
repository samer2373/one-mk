//! Pairing from the window: runs `one-mk pair --machine` and turns its
//! event lines into [`Step`]s; the person's answer goes back on its stdin.
//! The code shown is whatever `one-mk` computed; this file only relays it.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Invitation(String),
    /// Waiting for the other computer to start pairing too.
    Searching { me: String, port: Option<u16> },
    /// Nothing found for a while: show what to check.
    Hint,
    /// A computer answered; the code exchange is running.
    Found,
    /// Compare this code with the other screen.
    Code { name: String, code: String },
    /// We said yes; waiting for the other side.
    Waiting { name: String },
    Paired { id: String, name: String },
    /// Over without pairing: why.
    Ended(String),
    /// The attempt failed but pairing goes on waiting.
    Retry(String),
}

pub struct Pairing {
    /// `None` once `finish` has handed it to its reaper.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    pub rx: Receiver<Step>,
}

/// Parses one event line from `one-mk pair --machine`.
pub fn parse(line: &str) -> Option<Step> {
    let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    Some(match f[..] {
        ["invitation", link] => Step::Invitation(link.into()),
        ["me", name, _id, port] => Step::Searching { me: name.into(), port: port.parse().ok() },
        ["me", name, ..] => Step::Searching { me: name.into(), port: None },
        ["searching"] => return None,
        ["hint"] => Step::Hint,
        ["peer", ..] => Step::Found,
        ["code", name, code] => Step::Code { name: name.into(), code: code.chars().filter(char::is_ascii_digit).collect() },
        ["waiting"] => Step::Waiting { name: String::new() },
        ["paired", id, name, _side] => Step::Paired { id: short(id), name: name.into() },
        // `cancelled` follows our own "no": the codes didn't match here.
        ["cancelled"] => Step::Ended(
            "The codes didn't match, so nothing was saved. If you didn't expect that, make sure the other computer is yours, then try again.".into(),
        ),
        ["declined"] => Step::Ended("The other computer didn't confirm the code, so nothing was saved. Try again and confirm on both screens.".into()),
        ["retry", why] => Step::Retry(plain(why)),
        _ => return None,
    })
}

/// The command line's reasons, said for the window.
fn plain(why: &str) -> String {
    if why.starts_with("no answer within") { "nobody confirmed the code in 2 minutes".into() } else { crate::view::window_issue(why) }
}

/// What the window says when `one-mk pair` stops without saying why.
pub const STOPPED: &str = "oneMK stopped unexpectedly while pairing. Try again.";

/// The short id (`abcd-ef01`) of a full fingerprint, as status reports it.
fn short(fp: &str) -> String {
    // `get`, not a slice: a line that isn't hex must not cut a character in two.
    match (fp.get(..4), fp.get(4..8)) {
        (Some(first), Some(second)) if !fp.contains('-') => format!("{first}-{second}"),
        _ => fp.into(),
    }
}

impl Pairing {
    pub fn start(daemon: &std::path::Path, home_args: &[String], ctx: egui::Context) -> std::io::Result<Self> {
        Self::with_method(daemon, home_args, ctx, Method::Search, one_mk_proto::DEFAULT_PORT)
    }

    pub fn with_method(daemon: &std::path::Path, home_args: &[String], ctx: egui::Context, method: Method, port: u16) -> std::io::Result<Self> {
        let args = method.args(port)?;
        let mut cmd = Command::new(daemon);
        cmd.args(args).args(home_args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = one_mk_ctl::proc::hide_window(&mut cmd).spawn()?;
        if let Method::Join(link) = method && let Some(stdin) = &mut child.stdin {
            writeln!(stdin, "{}", link.trim())?; stdin.flush()?;
        }
        let (tx, rx) = channel();
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let errors = std::thread::spawn(move || {
            let mut err = String::new();
            // Log lines go here too; the final error is the one-line `one-mk: ...`.
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.starts_with("one-mk: ") {
                    err = line;
                }
            }
            err.trim_start_matches("one-mk: ").to_owned()
        });
        std::thread::spawn(move || {
            let mut last_name = String::new();
            let mut over = false;
            let mut gone = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Some(mut step) = parse(&line) else { continue };
                match &mut step {
                    Step::Code { name, .. } => last_name = name.clone(),
                    Step::Waiting { name } => *name = last_name.clone(),
                    _ => {}
                }
                over |= matches!(step, Step::Paired { .. } | Step::Ended(_));
                // Nobody listening any more (the sheet is gone): keep reading
                // all the same, so a process that goes on talking never
                // finds its output closed and fails on it.
                if !gone && tx.send(step).is_err() {
                    gone = true;
                }
                ctx.request_repaint();
            }
            // It exited. If it never said how pairing ended, say so, with
            // its own error if it left one: never leave the sheet waiting.
            let err = errors.join().unwrap_or_default();
            if !over && !gone {
                let _ = tx.send(Step::Ended(if err.is_empty() { STOPPED.into() } else { plain(&err) }));
                ctx.request_repaint();
            }
        });
        Ok(Self { stdin: child.stdin.take(), child: Some(child), rx })
    }

    /// Pairing is over and it worked: `one-mk` still has things to do after
    /// its last event (it updates the file manager's entries for the new
    /// computer), so it is left to end by itself, beside the window, and
    /// only a process that hangs is stopped, after a while. Dropping a
    /// `Pairing` instead kills it at once.
    pub fn finish(mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while child.try_wait().ok().flatten().is_none() && std::time::Instant::now() < end {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }

    pub fn answer(&mut self, yes: bool) {
        if let Some(s) = &mut self.stdin {
            let _ = s.write_all(if yes { b"yes\n" } else { b"no\n" });
            let _ = s.flush();
        }
    }
}

pub enum Method {
    Search,
    Invite,
    Join(String),
    Connect(String),
}

impl Method {
    fn args(&self, port: u16) -> std::io::Result<Vec<String>> {
        Ok(match self {
            Self::Search => vec!["pair".into(), "--machine".into()],
            Self::Invite => vec!["invite".into(), "--machine".into(), "--service-port".into(), port.to_string()],
            Self::Join(link) => {
                if link.trim().is_empty() || link.trim().contains(['\r', '\n']) { return Err(std::io::Error::other("Paste one complete invitation link.")); }
                vec!["join".into(), "--machine".into(), "--service-port".into(), port.to_string()]
            }
            Self::Connect(address) => {
                let target = match connect_target(address)? {
                    Target::Ready(target) => target,
                    // The window looks a name up beside itself (`Lookup`) and
                    // connects to the answer; this waits, for any other caller.
                    Target::Name(name) => Lookup::start(name, None).wait(LOOKUP_WAIT)?,
                };
                vec!["pair".into(), "--machine".into(), "--connect".into(), target.to_string()]
            }
        })
    }
}

/// How long a name may take to look up.
pub const LOOKUP_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Lookups still running: one that times out keeps its thread until the
/// system's resolver gives up, so only a few are allowed at once.
static LOOKUPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const LOOKUPS_AT_MOST: usize = 4;

/// Where Connect goes: an address as it is, or a name that needs a lookup.
#[derive(Debug, PartialEq)]
pub enum Target {
    Ready(std::net::SocketAddr),
    Name(String),
}

/// What Connect's text means, with the default port where it names none.
pub fn connect_target(address: &str) -> std::io::Result<Target> {
    let address = address.trim();
    if address.is_empty() { return Err(std::io::Error::other("Enter the other computer's address.")); }
    let address = if let Ok(ip) = address.parse::<std::net::IpAddr>() { std::net::SocketAddr::new(ip, one_mk_proto::DEFAULT_PORT).to_string() }
        else if address.contains(':') { address.to_owned() } else { format!("{address}:{}", one_mk_proto::DEFAULT_PORT) };
    Ok(match address.parse::<std::net::SocketAddr>() {
        Ok(target) => Target::Ready(target),
        Err(_) => Target::Name(address),
    })
}

/// A name being looked up on a thread of its own: a broken network can keep
/// it waiting far longer than the window may.
pub struct Lookup {
    rx: Receiver<std::io::Result<std::net::SocketAddr>>,
}

impl Lookup {
    /// Starts looking `address` (`name:port`) up; `ctx` is woken when it ends.
    pub fn start(address: String, ctx: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        if LOOKUPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= LOOKUPS_AT_MOST {
            LOOKUPS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            let _ = tx.send(Err(std::io::Error::other("Other name lookups are still running. Try the IP address.")));
            return Self { rx };
        }
        std::thread::spawn(move || {
            use std::net::ToSocketAddrs;
            let found = address.to_socket_addrs().and_then(|mut found| found.next().ok_or_else(|| std::io::Error::other("That computer's address could not be found.")));
            LOOKUPS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            let _ = tx.send(found);
            if let Some(ctx) = ctx { ctx.request_repaint(); }
        });
        Self { rx }
    }

    /// `None` while it is still looking.
    pub fn poll(&self) -> Option<std::io::Result<std::net::SocketAddr>> {
        self.rx.try_recv().ok()
    }

    fn wait(self, limit: std::time::Duration) -> std::io::Result<std::net::SocketAddr> {
        self.rx.recv_timeout(limit).unwrap_or_else(|_| Err(too_slow()))
    }
}

/// What a lookup that outlasts its limit says.
pub fn too_slow() -> std::io::Error {
    std::io::Error::other("That computer's name took too long to look up. Try its IP address.")
}

impl Drop for Pairing {
    /// Closing the sheet stops pairing.
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_pairing_commands_keep_links_off_the_command_line() {
        assert_eq!(Method::Invite.args(32001).unwrap(), ["invite", "--machine", "--service-port", "32001"]);
        assert_eq!(Method::Join("onemk://private-link".into()).args(32002).unwrap(), ["join", "--machine", "--service-port", "32002"]);
        for link in ["", "   ", "onemk://one\nonemk://two"] { assert!(Method::Join(link.into()).args(1).is_err()); }
        assert!(matches!(parse("invitation\tonemk://private-link"), Some(Step::Invitation(link)) if link == "onemk://private-link"));
    }

    #[test]
    fn direct_addresses_accept_ips_and_hosts_with_optional_ports() {
        for (address, expected) in [("127.0.0.1:32123", "127.0.0.1:32123".to_owned()),
            ("127.0.0.1", format!("127.0.0.1:{}", one_mk_proto::DEFAULT_PORT)), ("[::1]:32123", "[::1]:32123".into())] {
            assert_eq!(Method::Connect(address.into()).args(1).unwrap(), ["pair", "--machine", "--connect", expected.as_str()]);
        }
        assert!(Method::Connect("localhost:32123".into()).args(1).is_ok());
        for address in ["", "127.0.0.1:99999", "127.0.0.1:bad"] { assert!(Method::Connect(address.into()).args(1).is_err()); }
    }

    #[test]
    fn only_a_name_needs_a_lookup() {
        let port = one_mk_proto::DEFAULT_PORT;
        assert_eq!(connect_target("10.0.0.5").unwrap(), Target::Ready(format!("10.0.0.5:{port}").parse().unwrap()));
        assert_eq!(connect_target(" [::1]:32123 ").unwrap(), Target::Ready("[::1]:32123".parse().unwrap()));
        assert_eq!(connect_target("studio.local").unwrap(), Target::Name(format!("studio.local:{port}")));
        assert_eq!(connect_target("studio:32123").unwrap(), Target::Name("studio:32123".into()));
        assert!(connect_target("  ").is_err());
    }

    #[test]
    fn a_lookup_answers_without_the_caller_waiting_for_it() {
        let lookup = Lookup::start("127.0.0.1:32123".into(), None);
        let started = std::time::Instant::now();
        let found = loop {
            if let Some(found) = lookup.poll() { break found; }
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "no answer");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(found.unwrap(), "127.0.0.1:32123".parse().unwrap());
    }

    #[test]
    fn a_short_id_is_cut_on_characters_whatever_the_line_holds() {
        assert_eq!(short("0123456789abcdef"), "0123-4567");
        assert_eq!(short("abcd-ef01"), "abcd-ef01", "already short");
        assert_eq!(short("abc"), "abc");
        // Byte 4 is inside a character here.
        assert_eq!(short("abcéfghijk"), "abcéfghijk");
        assert!(matches!(parse("paired\tééééééé\tOffice PC\tright"), Some(Step::Paired { .. })));
    }

    /// A stand-in for `one-mk`: says "paired", then takes a moment before it
    /// writes a marker (what the real one does with the file manager's entries).
    #[cfg(unix)]
    fn slow_finisher(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("one-mk-fake");
        let marker = dir.join("integrated");
        std::fs::write(&script, format!("#!/bin/sh\nprintf 'paired\\t0123456789abcdef\\tStudio PC\\tright\\n'\nsleep 0.3\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, marker)
    }

    #[cfg(unix)]
    fn start_fake(script: &std::path::Path) -> Pairing {
        // A script just written can be "busy" for a moment while another test forks.
        for _ in 0..50 {
            match Pairing::with_method(script, &[], egui::Context::default(), Method::Search, 1) {
                Ok(pairing) => return pairing,
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(e) => panic!("{e}"),
            }
        }
        panic!("the fake one-mk stayed busy");
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_pairing_lets_one_mk_end_by_itself() {
        let dir = std::env::temp_dir().join(format!("omk-pairing-finish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (script, marker) = slow_finisher(&dir);
        let pairing = start_fake(&script);
        assert!(matches!(pairing.rx.recv_timeout(std::time::Duration::from_secs(10)), Ok(Step::Paired { .. })));
        pairing.finish();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "the process was ended before it finished its work");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn closing_the_sheet_still_stops_pairing_at_once() {
        let dir = std::env::temp_dir().join(format!("omk-pairing-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (script, marker) = slow_finisher(&dir);
        let pairing = start_fake(&script);
        assert!(matches!(pairing.rx.recv_timeout(std::time::Duration::from_secs(10)), Ok(Step::Paired { .. })));
        drop(pairing);
        std::thread::sleep(std::time::Duration::from_millis(700));
        assert!(!marker.exists(), "a dropped pairing is killed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_events() {
        assert_eq!(parse("me\tdesk\tabcd-ef01\t4000\n"), Some(Step::Searching { me: "desk".into(), port: Some(4000) }));
        assert_eq!(parse("searching"), None);
        assert_eq!(parse("code\tStudio PC\t482 913"), Some(Step::Code { name: "Studio PC".into(), code: "482913".into() }));
        assert_eq!(
            parse("paired\t0123456789abcdef\tStudio PC\tright"),
            Some(Step::Paired { id: "0123-4567".into(), name: "Studio PC".into() })
        );
        assert!(matches!(parse("declined"), Some(Step::Ended(_))));
        assert_eq!(parse("retry\ttimed out"), Some(Step::Retry("timed out".into())));
        assert_eq!(parse("retry\tno answer within 120 s"), Some(Step::Retry("nobody confirmed the code in 2 minutes".into())));
        assert!(matches!(parse("cancelled"), Some(Step::Ended(why)) if why.starts_with("The codes didn't match")));
        assert_eq!(parse("nonsense\tline"), None);
    }
}
