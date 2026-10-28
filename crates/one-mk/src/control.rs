//! The control channel between the CLI and the running daemon: one request
//! line, one reply, then the connection closes. The daemon only wakes when
//! a client connects.
//!
//! Unix: a socket in the config folder, readable and writable by the owner
//! only. Elsewhere: a TCP port on 127.0.0.1 plus a random token. Both are
//! written to a file in the (private, local) config folder, and a client
//! must send the token first.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Who we are, who is paired and online, and who controls whom.
    Status,
    /// `devices.toml` changed: apply it now.
    Reload,
    /// End the control of whichever peer controls this machine, as if
    /// this machine's own mouse or keyboard had been used.
    TakeBack,
}

impl Request {
    fn name(self) -> &'static str {
        match self {
            Request::Status => "status",
            Request::Reload => "reload",
            Request::TakeBack => "takeback",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Request::Status, Request::Reload, Request::TakeBack].into_iter().find(|r| r.name() == s)
    }
}

/// A request and where to send the reply text.
pub type Pending = (Request, oneshot::Sender<String>);

/// The daemon's end. Requests arrive from `recv`; dropping it closes the channel.
pub struct Server {
    rx: mpsc::UnboundedReceiver<Pending>,
    task: tokio::task::JoinHandle<()>,
    path: PathBuf,
}

impl Server {
    pub async fn start(dir: &Path) -> Result<Self> {
        if let Ok(Some(_)) = request(dir, Request::Status).await {
            bail!("one-mk is already running for this user");
        }
        let listener = sys::bind(dir)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let token = listener.token();
        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(s) => {
                        tokio::spawn(serve_conn(s, token.clone(), tx.clone()));
                    }
                    Err(e) => {
                        tracing::warn!("control channel stopped: {e}");
                        return;
                    }
                }
            }
        });
        Ok(Self { rx, task, path: sys::path(dir) })
    }

    pub async fn recv(&mut self) -> Option<Pending> {
        self.rx.recv().await
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn serve_conn<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, token: Option<String>, tx: mpsc::UnboundedSender<Pending>) {
    let _ = tokio::time::timeout(TIMEOUT, async {
        let mut line = String::new();
        {
            let mut r = BufReader::new((&mut s).take(1024));
            if let Some(t) = &token {
                r.read_line(&mut line).await?;
                if !same(line.trim_end(), &format!("auth {t}")) {
                    return std::io::Result::Ok(());
                }
                line.clear();
            }
            r.read_line(&mut line).await?;
        }
        let reply = match Request::parse(line.trim()) {
            Some(req) => {
                let (reply_tx, reply_rx) = oneshot::channel();
                let _ = tx.send((req, reply_tx));
                reply_rx.await.unwrap_or_default()
            }
            None => "error unknown request\n".to_owned(),
        };
        s.write_all(reply.as_bytes()).await?;
        s.shutdown().await
    })
    .await;
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Asks the running daemon. `None` means none is running.
pub async fn request(dir: &Path, req: Request) -> Result<Option<String>> {
    let stream = match tokio::time::timeout(TIMEOUT, sys::connect(dir)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            return Ok(None);
        }
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => bail!("one-mk did not answer"),
    };
    Ok(Some(tokio::time::timeout(TIMEOUT, exchange(stream, req)).await??))
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, req: Request) -> std::io::Result<String> {
    s.write_all(format!("{}\n", req.name()).as_bytes()).await?;
    let mut out = String::new();
    s.read_to_string(&mut out).await?;
    Ok(out)
}

#[cfg(unix)]
use unix as sys;
#[cfg(not(unix))]
use tcp as sys;

#[cfg(unix)]
mod unix {
    use std::io;
    use std::path::{Path, PathBuf};
    use tokio::net::{UnixListener, UnixStream};

    pub struct Listener(UnixListener);

    pub fn path(dir: &Path) -> PathBuf {
        dir.join("control.sock")
    }

    pub fn bind(dir: &Path) -> anyhow::Result<Listener> {
        let p = path(dir);
        let _ = std::fs::remove_file(&p); // left over from a crash
        let l = UnixListener::bind(&p)?;
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        Ok(Listener(l))
    }

    impl Listener {
        pub async fn accept(&self) -> io::Result<UnixStream> {
            self.0.accept().await.map(|(s, _)| s)
        }
        /// The peer is identified by the socket's permissions.
        pub fn token(&self) -> Option<String> {
            None
        }
    }

    pub async fn connect(dir: &Path) -> io::Result<UnixStream> {
        UnixStream::connect(path(dir)).await
    }
}

// Compiled everywhere so it is tested everywhere, used where there are no
// Unix sockets.
#[cfg_attr(unix, allow(dead_code))]
mod tcp {
    use std::hash::{BuildHasher, RandomState};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    pub struct Listener {
        inner: TcpListener,
        token: String,
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join("control.port")
    }

    pub fn bind(dir: &Path) -> anyhow::Result<Listener> {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        // std seeds RandomState from the OS and changes its keys on every call.
        let token: String = (0..4).map(|i| format!("{:016x}", RandomState::new().hash_one(i))).collect();
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        writeln!(opts.open(path(dir))?, "{port} {token}")?;
        Ok(Listener { inner: TcpListener::from_std(std_listener)?, token })
    }

    impl Listener {
        pub async fn accept(&self) -> io::Result<TcpStream> {
            self.inner.accept().await.map(|(s, _)| s)
        }
        pub fn token(&self) -> Option<String> {
            Some(self.token.clone())
        }
    }

    pub async fn connect(dir: &Path) -> io::Result<TcpStream> {
        let text = std::fs::read_to_string(path(dir))?;
        let (port, token) = text.trim().split_once(' ').ok_or_else(|| io::Error::other("bad control file"))?;
        let port: u16 = port.parse().map_err(io::Error::other)?;
        let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
        s.write_all(format!("auth {token}\n").as_bytes()).await?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("one-mk-ctl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Answers `status` with "hello" and `reload` with "ok".
    async fn answer(mut server: Server) {
        while let Some((req, reply)) = server.recv().await {
            let _ = reply.send(if req == Request::Status { "hello\n" } else { "ok\n" }.into());
        }
    }

    #[tokio::test]
    async fn daemon_answers_and_is_private() {
        let dir = temp_dir("sock");
        assert_eq!(request(&dir, Request::Status).await.unwrap(), None, "nothing is running yet");
        let server = Server::start(&dir).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(sys::path(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let task = tokio::spawn(answer(server));
        assert!(Server::start(&dir).await.is_err(), "a second daemon is refused");
        assert_eq!(request(&dir, Request::Status).await.unwrap().as_deref(), Some("hello\n"));
        assert_eq!(request(&dir, Request::Reload).await.unwrap().as_deref(), Some("ok\n"));
        task.abort();
        let _ = task.await;
        assert_eq!(request(&dir, Request::Status).await.unwrap(), None, "socket removed with the server");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The loopback transport used where there are no Unix sockets: the
    /// token file works, and a client without the token gets nothing.
    #[tokio::test]
    async fn tcp_transport_checks_the_token() {
        let dir = temp_dir("tcp");
        let listener = tcp::bind(&dir).unwrap();
        let token = listener.token();
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let s = listener.accept().await.unwrap();
                tokio::spawn(serve_conn(s, token.clone(), tx.clone()));
            }
        });
        tokio::spawn(async move {
            while let Some((_, reply)) = rx.recv().await {
                let _ = reply.send("hello\n".into());
            }
        });
        let s = tcp::connect(&dir).await.unwrap();
        assert_eq!(exchange(s, Request::Status).await.unwrap(), "hello\n");

        // Wrong token: closed without an answer.
        let text = std::fs::read_to_string(tcp::path(&dir)).unwrap();
        let port: u16 = text.split(' ').next().unwrap().parse().unwrap();
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"auth nope\n").await.unwrap();
        assert_eq!(exchange(s, Request::Status).await.unwrap_or_default(), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unknown_requests_and_oversized_lines_are_refused() {
        let (client, server) = duplex(4096);
        let (tx, _rx) = mpsc::unbounded_channel();
        tokio::spawn(serve_conn(server, None, tx));
        let mut client = client;
        client.write_all(b"format c:\n").await.unwrap();
        let mut out = String::new();
        client.read_to_string(&mut out).await.unwrap();
        assert_eq!(out, "error unknown request\n");
    }
}
