//! The client side of the control channel (see `one-mk`'s `control.rs` for
//! the daemon side): one request line, one reply, then the connection
//! closes. This is a plain blocking client so a tray needs no async runtime.
//!
//! Unix: a socket in the config folder. Elsewhere: a localhost TCP port
//! plus a random token, both in a file in the config folder; the client
//! sends the token first.

use anyhow::{Result, bail};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::status::TransferStatus;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Who we are, who is paired and online, and who controls whom.
    Status,
    /// `devices.toml` changed: apply it now.
    Reload,
    /// End the control of whichever peer controls this machine, as if
    /// this machine's own mouse or keyboard had been used.
    TakeBack,
    /// `settings.toml` (clipboard and notification settings) changed: apply it now.
    Settings,
    /// Send this computer's clipboard to one paired computer now (the
    /// only one online if none is named).
    ClipSend(Option<String>),
    /// Fetch one paired computer's clipboard into this one now.
    ClipGet(Option<String>),
    /// A running `one-mk send` reports how far it is, so `status` and the
    /// window can show it. `finished` ends the report.
    Transfer { t: TransferStatus, finished: bool },
}

impl Request {
    /// The command word, the first thing on the request line.
    pub fn name(&self) -> &'static str {
        match self {
            Request::Status => "status",
            Request::Reload => "reload",
            Request::TakeBack => "takeback",
            Request::Settings => "settings",
            Request::ClipSend(_) => "clip-send",
            Request::ClipGet(_) => "clip-get",
            Request::Transfer { .. } => "transfer",
        }
    }

    /// The whole request line, without the newline.
    pub fn line(&self) -> String {
        let device = |d: &Option<String>| d.as_deref().map(|d| format!(" {}", d.replace(['\n', '\r'], " "))).unwrap_or_default();
        match self {
            Request::ClipSend(d) | Request::ClipGet(d) => format!("{}{}", self.name(), device(d)),
            Request::Transfer { t, finished } => format!(
                "transfer {} {} {} {} {} {} {}",
                t.id,
                t.files,
                t.done,
                t.total,
                t.rate,
                u8::from(*finished),
                t.peer.replace(['\n', '\r'], " ")
            ),
            _ => self.name().to_owned(),
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let (word, rest) = line.split_once(' ').map_or((line, None), |(w, r)| (w, Some(r.trim()).filter(|r| !r.is_empty())));
        let device = || rest.map(str::to_owned);
        Some(match (word, rest) {
            ("status", None) => Request::Status,
            ("reload", None) => Request::Reload,
            ("takeback", None) => Request::TakeBack,
            ("settings", None) => Request::Settings,
            ("clip-send", _) => Request::ClipSend(device()),
            ("clip-get", _) => Request::ClipGet(device()),
            ("transfer", Some(rest)) => {
                let f: Vec<&str> = rest.splitn(7, ' ').collect();
                let [id, files, done, total, rate, finished, peer] = f[..] else { return None };
                Request::Transfer {
                    t: TransferStatus {
                        id: id.parse().ok()?,
                        sending: true,
                        peer: peer.to_owned(),
                        files: files.parse().ok()?,
                        done: done.parse().ok()?,
                        total: total.parse().ok()?,
                        rate: rate.parse().ok()?,
                    },
                    finished: finished == "1",
                }
            }
            _ => return None,
        })
    }

    /// How long to wait for the daemon's answer: a clipboard can take a
    /// while to cross a slow link.
    pub fn reply_timeout(&self) -> Duration {
        match self {
            Request::ClipSend(_) | Request::ClipGet(_) => Duration::from_secs(45),
            _ => Duration::from_secs(5),
        }
    }
}

/// The daemon's socket (Unix).
pub fn unix_socket_path(dir: &Path) -> PathBuf {
    dir.join("control.sock")
}

/// The daemon's port-and-token file (everywhere without Unix sockets).
pub fn tcp_port_path(dir: &Path) -> PathBuf {
    dir.join("control.port")
}

/// Splits the contents of the port file into the port and the token.
pub fn parse_port_file(text: &str) -> io::Result<(u16, String)> {
    let (port, token) = text.trim().split_once(' ').ok_or_else(|| io::Error::other("bad control file"))?;
    Ok((port.parse().map_err(io::Error::other)?, token.to_owned()))
}

fn is_not_running(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
}

/// Asks the running daemon. `None` means none is running.
pub fn request(dir: &Path, req: Request, timeout: Duration) -> Result<Option<String>> {
    #[cfg(unix)]
    let connected = connect_unix(dir, timeout);
    #[cfg(not(unix))]
    let connected = connect_tcp(dir, timeout);
    match connected {
        Ok(stream) => Ok(Some(exchange(stream, req, timeout)?)),
        Err(e) if is_not_running(&e) => Ok(None),
        Err(e) => bail!("could not reach one-mk: {e}"),
    }
}

/// True if a daemon answers `status`.
pub fn is_running(dir: &Path) -> bool {
    request(dir, Request::Status, Duration::from_secs(2)).is_ok_and(|r| r.is_some())
}

trait Conn: Read + Write {
    fn timeouts(&self, t: Duration) -> io::Result<()>;
}

#[cfg(unix)]
impl Conn for std::os::unix::net::UnixStream {
    fn timeouts(&self, t: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(t))?;
        self.set_write_timeout(Some(t))
    }
}

impl Conn for std::net::TcpStream {
    fn timeouts(&self, t: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(t))?;
        self.set_write_timeout(Some(t))
    }
}

fn exchange(mut s: impl Conn, req: Request, timeout: Duration) -> io::Result<String> {
    s.timeouts(timeout)?;
    s.write_all(format!("{}\n", req.line()).as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

#[cfg(unix)]
fn connect_unix(dir: &Path, _timeout: Duration) -> io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(unix_socket_path(dir))
}

// Compiled everywhere so it is tested everywhere, used where there are no
// Unix sockets.
#[cfg_attr(unix, allow(dead_code))]
fn connect_tcp(dir: &Path, timeout: Duration) -> io::Result<std::net::TcpStream> {
    let (port, token) = parse_port_file(&std::fs::read_to_string(tcp_port_path(dir))?)?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout)?;
    s.set_write_timeout(Some(timeout))?;
    s.write_all(format!("auth {token}\n").as_bytes())?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-ctl-client-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn requests_have_names() {
        for r in [Request::Status, Request::Reload, Request::TakeBack, Request::Settings] {
            assert_eq!(Request::parse(r.name()), Some(r));
        }
        assert_eq!(Request::parse("format c:"), None);
        assert_eq!(Request::parse("status extra"), None, "words that take no argument refuse one");
    }

    #[test]
    fn requests_with_arguments_survive_the_wire() {
        let t = TransferStatus { id: 9, sending: true, peer: "mac mini".into(), files: 3, done: 10, total: 20, rate: 5 };
        for r in [
            Request::ClipSend(None),
            Request::ClipSend(Some("mac mini".into())),
            Request::ClipGet(None),
            Request::ClipGet(Some("desk".into())),
            Request::Transfer { t: t.clone(), finished: false },
            Request::Transfer { t, finished: true },
        ] {
            assert_eq!(Request::parse(&r.line()), Some(r.clone()), "{}", r.line());
        }
        // A newline in a name can't smuggle a second request.
        assert_eq!(Request::ClipSend(Some("a\nstatus".into())).line(), "clip-send a status");
        assert_eq!(Request::parse("clip-send  "), Some(Request::ClipSend(None)));
        assert_eq!(Request::parse("transfer 1 2 3"), None);
        assert_eq!(Request::parse("transfer x 1 2 3 4 0 peer"), None);
        assert!(Request::ClipGet(None).reply_timeout() > Request::Status.reply_timeout());
    }

    #[test]
    fn nothing_running_is_none() {
        let dir = temp_dir("none");
        assert_eq!(request(&dir, Request::Status, Duration::from_secs(1)).unwrap(), None);
        assert!(!is_running(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tcp_client_sends_token_then_request() {
        let dir = temp_dir("tcp");
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        std::fs::write(tcp_port_path(&dir), format!("{} sekrit\n", l.local_addr().unwrap().port())).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let (mut auth, mut req) = (String::new(), String::new());
            r.read_line(&mut auth).unwrap();
            r.read_line(&mut req).unwrap();
            s.write_all(format!("{}|{}", auth.trim(), req.trim()).as_bytes()).unwrap();
        });
        let s = connect_tcp(&dir, Duration::from_secs(2)).unwrap();
        assert_eq!(exchange(s, Request::TakeBack, Duration::from_secs(2)).unwrap(), "auth sekrit|takeback");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn unix_client_talks_to_a_socket() {
        let dir = temp_dir("unix");
        let l = std::os::unix::net::UnixListener::bind(unix_socket_path(&dir)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = String::new();
            BufReader::new(s.try_clone().unwrap()).read_line(&mut req).unwrap();
            s.write_all(format!("got {}", req.trim()).as_bytes()).unwrap();
        });
        assert_eq!(request(&dir, Request::Status, Duration::from_secs(2)).unwrap().as_deref(), Some("got status"));
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
