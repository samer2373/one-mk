//! Explicit, fail-closed signed updates. Startup work finishes once, never polls.

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use one_mk_ctl::update_logic::{self as policy, InstallKind, Manifest, State};
use ring::{digest, signature};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const LATEST: &str = "https://api.github.com/repos/samer2373/one-mk/releases/latest";
const STATE: &str = "update-state.json";
const NOTES: &str = "https://github.com/samer2373/one-mk/releases/latest";
const FLOOR: Option<&str> = option_env!("GITHUB_RUN_NUMBER");
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[path = "update_bundle.rs"]
mod bundle;

#[derive(Args, Debug)]
pub struct Options {
    #[arg(long)]
    pub status_json: bool,
    #[arg(long)]
    pub startup_check: bool,
    #[arg(long, value_parser = ["on", "off"])]
    pub install_automatically: Option<String>,
    #[arg(long)]
    pub move_to_applications: bool,
    #[arg(long)]
    pub started_version: Option<String>,
    /// Check for a signed release without downloading or replacing the executable.
    #[arg(long, conflicts_with_all = ["rollback", "check_on_start"])]
    pub check: bool,
    /// Restore one-mk.old for a portable installation; restart manually afterward.
    #[arg(long, conflicts_with_all = ["url", "check_on_start"])]
    pub rollback: bool,
    /// Enable or disable the daily startup check (never applies automatically).
    #[arg(long, value_parser = ["on", "off"], conflicts_with = "url")]
    pub check_on_start: Option<String>,
    /// GitHub-compatible releases/latest endpoint (HTTPS; signatures still required).
    #[arg(long)]
    pub url: Option<String>,
    /// Only for the update itself, run with the account's password: install
    /// the package staged in this folder. It trusts nothing but the folder:
    /// it checks the signed update list, the version and the package itself.
    #[arg(long, hide = true, value_name = "STAGING_FOLDER")]
    pub install_deb: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings { pub check: bool, pub install_automatically: bool, pub url: Option<String> }

impl Default for Settings {
    fn default() -> Self { Self { check: true, install_automatically: false, url: None } }
}

fn now() -> Result<u64> { Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()) }

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }

fn verify(key: &[u8], message: &[u8], signed: &[u8]) -> Result<()> {
    policy::signature_inputs(key, signed, message)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, key).verify(message, signed)
        .map_err(|_| anyhow::anyhow!("release signature is not trusted; install manually or ask the publisher to repair signing"))
}

fn verify_embedded(message: &[u8], signed: &[u8]) -> Result<()> {
    if verify(&crate::update_key::PUBLIC_KEY, message, signed).is_ok() { return Ok(()); }
    for key in crate::update_key::ADDITIONAL_PUBLIC_KEYS {
        if verify(key, message, signed).is_ok() { return Ok(()); }
    }
    bail!("release signature is not trusted; install manually or ask the publisher to repair signing")
}

struct Temporary { path: PathBuf, file: File }

impl Temporary {
    fn new(parent: &Path) -> Result<Self> {
        Self::named(parent, "payload")
    }

    fn named(parent: &Path, purpose: &str) -> Result<Self> {
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        for _ in 0..100 {
            let path = parent.join(format!(".one-mk-update-{purpose}-{}-{}{suffix}", std::process::id(), TEMP_ID.fetch_add(1, Ordering::Relaxed)));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            // Private from the first byte (the saved state, and the update's payload).
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&path) {
                Ok(file) => return Ok(Self { path, file }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("cannot create an update file beside one-mk; check directory permissions and free disk space"),
            }
        }
        bail!("cannot reserve a temporary update file; remove stale .one-mk-update-* files and retry")
    }
}

impl Drop for Temporary {
    fn drop(&mut self) { let _ = fs::remove_file(&self.path); }
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn save_state(home: &Path, state: &State) -> Result<()> {
    let mut temporary = Temporary::named(home, "state")?;
    temporary.file.write_all(&serde_json::to_vec(state)?)?;
    temporary.file.sync_all()?;
    atomic_rename(&temporary.path, &home.join(STATE))?;
    sync_directory(home)
}

fn atomic_rename(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(not(windows))]
    fs::rename(source, destination)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" { fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32; }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
        ensure!(unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } != 0,
            "cannot atomically save update state; check directory permissions and retry");
    }
    Ok(())
}

fn load_state(home: &Path) -> Result<State> {
    match File::open(home.join(STATE)) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 16 * 1024, "update state is too large; restore it from a trusted backup");
            let state: State = serde_json::from_slice(&bytes).context("update state is damaged; restore it from a trusted backup before updating")?;
            if state.sequence > 0 {
                policy::hex_bytes::<32>(&state.digest)?;
                policy::version(state.highest_version.as_deref().context("missing update version history")?)?;
            }
            Ok(state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(error) => Err(error).context("cannot read update state; check config directory permissions"),
    }
}

fn lock(home: &Path) -> Result<File> {
    one_mk_ctl::private::create_dir_all(home)?;
    let file = one_mk_ctl::private::lock_file(&home.join("update.lock"))?;
    file.try_lock().context("another update check or installation is in progress; wait for it to finish and retry")?;
    clean_temporary(home, &[".one-mk-update-state-"])?;
    Ok(file)
}

fn clean_temporary(parent: &Path, prefixes: &[&str]) -> Result<()> {
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if prefixes.iter().any(|prefix| entry.file_name().to_string_lossy().starts_with(prefix)) && entry.file_type()?.is_file() {
            fs::remove_file(entry.path()).context("cannot clean an interrupted update; check permissions and retry")?;
        }
    }
    Ok(())
}

/// The end of an install or a rollback. The window reads "Restarting…" and
/// restarts oneMK itself; in a terminal nothing restarts, so say what to do.
fn restarting(done: &str, by_hand: &str) {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() { println!("{done}. {by_hand}"); } else { println!("{done}. Restarting…"); }
}

/// `denied`: what to tell someone whose account can't write the folder.
fn target_lock(target: &Path, denied: &str) -> Result<File> {
    let parent = target.parent().context("update target has no directory")?;
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(parent.join(".one-mk-update.lock"))
        .with_context(|| denied.to_owned())?;
    // Another update holds it for as long as it runs. A child process that
    // another thread is starting can also hold a copy for a moment (until it
    // execs and closes it), so wait a little before calling it busy.
    let mut tries = 0;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::WouldBlock) => bail!("Another update is running. Wait for it to finish, then try again"),
            Err(std::fs::TryLockError::Error(error)) => return Err(error).with_context(|| denied.to_owned()),
        }
    }
    clean_temporary(parent, &[".one-mk-update-payload-", ".one-mk-update-probe-"])?;
    Ok(file)
}

fn validate_url(url: &Url) -> Result<()> {
    ensure!(url.username().is_empty() && url.password().is_none() && url.fragment().is_none(), "update URL must not contain credentials or a fragment");
    ensure!(url.scheme() == "https" || (cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1")),
        "update URL must use HTTPS (HTTP is allowed only for 127.0.0.1 in unit tests)");
    ensure!(url.host_str().is_some(), "update URL has no hostname");
    Ok(())
}

fn allowed(url: &Url, origin: &Url) -> bool {
    (url.host_str() == origin.host_str() && url.port_or_known_default() == origin.port_or_known_default()) ||
        (url.scheme() == "https" && url.port_or_known_default() == Some(443) && matches!(url.host_str(),
            Some("github.com" | "objects.githubusercontent.com" | "release-assets.githubusercontent.com")))
}

trait Connection: Read + Write {}
impl<Stream: Read + Write> Connection for Stream {}

struct ResponseReader { connection: Box<dyn Connection>, deadline: Instant }
impl Read for ResponseReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "update response timed out; retry on a reliable connection"));
        }
        self.connection.read(buffer)
    }
}

fn connect(url: &Url) -> Result<Box<dyn Connection>> {
    let host = url.host_str().context("update URL has no hostname")?;
    let port = url.port_or_known_default().context("update URL has no port")?;
    let addresses = (host, port).to_socket_addrs().context("cannot find the update server; check your internet connection")?;
    let mut connection = None;
    for address in addresses.take(4) {
        if let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) { connection = Some(socket); break; }
    }
    let socket = connection.context("cannot reach the update server; check your internet connection and retry")?;
    socket.set_read_timeout(Some(Duration::from_secs(15)))?;
    socket.set_write_timeout(Some(Duration::from_secs(15)))?;
    if url.scheme() == "http" { return Ok(Box::new(socket)); }
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?.with_root_certificates(roots).with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())?;
    Ok(Box::new(rustls::StreamOwned::new(rustls::ClientConnection::new(Arc::new(config), name)?, socket)))
}

fn line(reader: &mut impl BufRead, budget: &mut usize) -> Result<String> {
    let mut bytes = Vec::new();
    reader.take((*budget + 1) as u64).read_until(b'\n', &mut bytes).context("cannot read the update response; check your connection and retry")?;
    ensure!(!bytes.is_empty() && bytes.len() <= *budget && bytes.ends_with(b"\r\n"), "update server sent invalid or oversized HTTP headers");
    *budget -= bytes.len();
    Ok(std::str::from_utf8(&bytes).context("update server sent invalid HTTP text; check the configured URL")?.trim_end_matches("\r\n").to_owned())
}

fn request(url: &Url, origin: &Url, limit: u64, output: &mut impl Write) -> Result<u64> {
    let mut current = url.clone();
    // 30 s, plus time for the size at a (slow) 32 KiB/s, at most an hour.
    let deadline = Instant::now() + Duration::from_secs((30 + limit / (32 * 1024)).min(3600));
    for redirects in 0..=3 {
        validate_url(&current)?;
        ensure!(allowed(&current, origin), "update download redirected to an unapproved host; no files were replaced");
        let mut connection = connect(&current)?;
        let path = match current.query() { Some(query) => format!("{}?{query}", current.path()), None => current.path().to_owned() };
        let host = current.host_str().context("missing hostname")?;
        write!(connection, "GET {path} HTTP/1.1\r\nHost: {host}:{}\r\nUser-Agent: one-mk/{}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n", current.port_or_known_default().context("missing port")?, env!("CARGO_PKG_VERSION"))?;
        connection.flush()?;
        let mut reader = BufReader::new(ResponseReader { connection, deadline });
        let mut budget = 32 * 1024;
        let status = line(&mut reader, &mut budget)?;
        let code = status.split_whitespace().nth(1).context("invalid HTTP response; check the configured update URL")?.parse::<u16>().context("update server sent an invalid HTTP status; retry later")?;
        ensure!(status.starts_with("HTTP/1.1 ") || status.starts_with("HTTP/1.0 "), "unsupported update server response");
        let mut length = None;
        let mut location = None;
        let mut chunked = false;
        let mut encoding = false;
        loop {
            let header = line(&mut reader, &mut budget)?;
            if header.is_empty() { break; }
            let (name, value) = header.split_once(':').context("invalid update response header")?;
            match name.to_ascii_lowercase().as_str() {
                "content-length" => { ensure!(length.is_none(), "duplicate download length"); length = Some(value.trim().parse::<u64>().context("update server sent an invalid download length; retry later")?); }
                "location" => { ensure!(location.is_none(), "duplicate redirect location"); location = Some(value.trim().to_owned()); }
                "transfer-encoding" => { ensure!(!chunked && value.trim().eq_ignore_ascii_case("chunked"), "unsupported transfer encoding"); chunked = true; }
                "content-encoding" => { ensure!(!encoding && value.trim() == "identity", "compressed update response refused"); encoding = true; }
                _ => {}
            }
        }
        if matches!(code, 301 | 302 | 303 | 307 | 308) {
            ensure!(redirects < 3, "too many update redirects; no files were replaced");
            let next = current.join(location.as_deref().context("update redirect has no destination")?)?;
            ensure!(current.scheme() != "https" || next.scheme() == "https", "insecure update redirect refused");
            current = next;
            continue;
        }
        ensure!(code == 200, "update server returned HTTP {code}; check for a published signed release and retry later");
        ensure!(!(chunked && length.is_some()), "ambiguous HTTP body framing refused");
        ensure!(length.is_none_or(|size| size <= limit), "download exceeds its allowed size; no files were replaced");
        let mut total = 0;
        let mut buffer = [0u8; 16 * 1024];
        loop {
            ensure!(Instant::now() < deadline, "update download timed out; retry on a reliable connection");
            let remaining = if chunked {
                let mut chunk_budget = 1024;
                let size = line(&mut reader, &mut chunk_budget)?;
                let size = u64::from_str_radix(size.split(';').next().context("invalid chunk length")?, 16).context("update server sent an invalid download chunk; retry later")?;
                ensure!(size <= limit.saturating_sub(total), "download exceeds its allowed size; no files were replaced");
                if size == 0 {
                    let mut trailer_budget = 8192;
                    while !line(&mut reader, &mut trailer_budget)?.is_empty() {}
                    break;
                }
                size
            } else { length.map_or(limit.saturating_sub(total) + 1, |size| size.saturating_sub(total)) };
            if remaining == 0 { break; }
            let mut copied = 0;
            while copied < remaining {
                ensure!(Instant::now() < deadline, "update download timed out; retry on a reliable connection");
                let capacity = (remaining - copied).min(buffer.len() as u64) as usize;
                let count = reader.read(&mut buffer[..capacity]).context("update download stopped; check your connection and retry; old executable was not replaced")?;
                if count == 0 {
                    ensure!(!chunked && length.is_none(), "update download was interrupted; old executable was not replaced");
                    return Ok(total);
                }
                total += count as u64;
                copied += count as u64;
                ensure!(total <= limit, "download exceeds its allowed size; no files were replaced");
                output.write_all(&buffer[..count]).context("cannot save the update; check free disk space and directory permissions")?;
            }
            if chunked {
                let mut ending = [0; 2];
                reader.read_exact(&mut ending)?;
                ensure!(ending == *b"\r\n", "invalid chunk ending");
            }
        }
        return Ok(total);
    }
    bail!("too many update redirects")
}

fn fetch(url: &Url, origin: &Url, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    request(url, origin, limit, &mut bytes)?;
    Ok(bytes)
}

#[derive(Deserialize)]
struct Release { tag_name: String, assets: Vec<ReleaseAsset> }
#[derive(Deserialize)]
struct ReleaseAsset { name: String, browser_download_url: String }

fn asset_url(release: &Release, name: &str, origin: &Url) -> Result<Url> {
    let mut assets = release.assets.iter().filter(|asset| asset.name == name);
    let asset = assets.next().with_context(|| format!("release has no {name}; ask the publisher to sign this release"))?;
    ensure!(assets.next().is_none(), "release has duplicate asset names");
    let url = Url::parse(&asset.browser_download_url).context("release asset has an invalid URL; ask the publisher to repair the release")?;
    validate_url(&url)?;
    ensure!(allowed(&url, origin), "release asset points to an unapproved host");
    Ok(url)
}

/// The exact signed bytes behind a checked manifest, kept so a later step
/// (the root step of a package install) can verify them again by itself.
/// Only Linux packages read them.
#[cfg_attr(not(unix), allow(dead_code))]
struct Signed { manifest: Vec<u8>, signature: Vec<u8> }

type Checked = (Manifest, Release, Url, Signed);

fn checked(home: &Path, endpoint: &str, key: &[u8], current: &str, state: &mut State) -> Result<Checked> {
    let origin = Url::parse(endpoint).context("invalid update URL; set an HTTPS releases/latest URL")?;
    validate_url(&origin)?;
    let release: Release = serde_json::from_slice(&fetch(&origin, &origin, 1024 * 1024)?).context("update server did not return a release; check the configured URL")?;
    ensure!(release.assets.len() <= 256, "too many release assets");
    let bytes = fetch(&asset_url(&release, "update-manifest.json", &origin)?, &origin, policy::MANIFEST_LIMIT as u64)?;
    let signed = fetch(&asset_url(&release, "update-manifest.json.sig", &origin)?, &origin, 64)?;
    if key == crate::update_key::PUBLIC_KEY && crate::update_key::PRODUCTION_KEY {
        verify_embedded(&bytes, &signed)?;
    } else {
        verify(key, &bytes, &signed)?;
    }
    let manifest = policy::parse(&bytes)?;
    ensure!(release.tag_name == format!("v{}", manifest.version), "release tag and signed version do not match");
    let digest = hex(digest::digest(&digest::SHA256, &bytes).as_ref());
    let available = match policy::accept(&manifest, current, state, &digest, FLOOR.and_then(|value| value.parse().ok()).unwrap_or(0)) {
        Ok(available) => available,
        Err(error) => {
            // A rejected release (revoked, too old, a downgrade) is no longer
            // advertised.
            if state.available.take().is_some() { save_state(home, state)?; }
            return Err(error);
        }
    };
    state.sequence = manifest.sequence;
    state.digest = digest;
    state.highest_version = Some(manifest.version.clone());
    state.available = available.then(|| manifest.version.clone());
    state.error = None;
    save_state(home, state)?;
    Ok((manifest, release, origin, Signed { manifest: bytes, signature: signed }))
}

fn target() -> Result<(PathBuf, InstallKind)> {
    let executable = std::env::current_exe()?;
    // `APPIMAGE` is inherited by every child of an AppImage (a terminal opened
    // inside another AppImage, say): it only counts when this very process runs
    // from that image's mount.
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from)
        .filter(|_| std::env::var_os("APPDIR").is_some_and(|dir| executable.starts_with(dir)));
    let executable = appimage.clone().unwrap_or(executable);
    let home = directories::BaseDirs::new().map(|base| base.home_dir().to_string_lossy().into_owned()).unwrap_or_default();
    let mut kind = policy::install_kind(&executable.to_string_lossy(), &home, appimage.is_some(), std::env::var_os("FLATPAK_ID").is_some());
    if kind == InstallKind::Deb && !from_package(&executable) { kind = InstallKind::Managed; }
    Ok((executable, kind))
}

/// Whether the one-mk package installed this file: its file list, read
/// directly (`dpkg-query --search` reads every package's list, on each
/// status the window asks for).
fn from_package(executable: &Path) -> bool {
    let wanted = executable.to_string_lossy();
    let Ok(entries) = fs::read_dir("/var/lib/dpkg/info") else { return false };
    entries.flatten().filter(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name == "one-mk.list" || (name.starts_with("one-mk:") && name.ends_with(".list"))
    }).any(|entry| fs::read_to_string(entry.path()).is_ok_and(|list| package_lists(&list, &wanted)))
}

fn package_lists(list: &str, path: &str) -> bool { list.lines().any(|line| line == path) }

fn refuse_root() -> Result<()> {
    #[cfg(unix)]
    ensure!(unsafe { libc::geteuid() } != 0, "updates never run as root; run as your normal user or use your package manager");
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" { fn IsUserAnAdmin() -> i32; }
        ensure!(unsafe { IsUserAnAdmin() } == 0, "updates never run elevated; open a normal terminal or use the installer");
    }
    Ok(())
}

struct VersionProbe { child: std::process::Child }

impl Drop for VersionProbe {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Ok(process_id) = i32::try_from(self.child.id()) {
            unsafe { libc::kill(-process_id, libc::SIGKILL); }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn version_check(path: &Path, expected: Option<&str>) -> Result<()> {
    // Never beside the executable of a Mac app: a file added inside the app
    // while it starts makes macOS kill it as "damaged", so the probe goes next to the app.
    let mut probe_dir = path.parent().context("executable has no directory")?;
    if let Some(app) = path.ancestors().find(|dir| dir.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))) {
        probe_dir = app.parent().context("the app has no parent folder")?;
    }
    let mut output_file = Temporary::named(probe_dir, "probe")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut command = Command::new(path);
    command.arg("--version").stdin(Stdio::null()).stdout(Stdio::from(output_file.file.try_clone()?)).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // Retried only where a just-written file can be busy (unix ETXTBSY).
    #[cfg_attr(not(unix), allow(clippy::never_loop))]
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            #[cfg(unix)]
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error).context("new executable could not start; install the complete release manually"),
        }
    };
    let mut probe = VersionProbe { child };
    let status = loop {
        if let Some(status) = probe.child.try_wait()? { break status; }
        if Instant::now() >= deadline || output_file.file.metadata()?.len() > 256 {
            bail!("new executable did not respond to --version; old executable was not replaced");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    use std::io::{Seek, SeekFrom};
    output_file.file.seek(SeekFrom::Start(0))?;
    let mut output = String::new();
    (&mut output_file.file).take(257).read_to_string(&mut output)?;
    ensure!(output.len() <= 256, "new executable returned too much version output; old executable was not replaced");
    ensure!(status.success(), "new executable rejected --version; old executable was not replaced");
    let found = output.trim().strip_prefix("one-mk ").context("new executable returned an invalid version; old executable was not replaced")?;
    policy::version(found)?;
    ensure!(expected.is_none_or(|expected| expected == found), "new executable version does not match its signed manifest; old executable was not replaced");
    Ok(())
}

fn regular(path: &Path) -> Result<()> {
    ensure!(fs::symlink_metadata(path)?.file_type().is_file(), "update target must be a regular file, not a symlink; use the original installation method");
    Ok(())
}

fn replace(candidate: &Path, target: &Path) -> Result<()> {
    regular(target)?;
    let parent = target.parent().context("update target has no directory")?;
    let old = parent.join("one-mk.old");
    #[cfg(unix)]
    {
        let mut backup = Temporary::new(parent)?;
        let mut source = File::open(target)?;
        std::io::copy(&mut source, &mut backup.file)?;
        fs::set_permissions(&backup.path, fs::metadata(target)?.permissions())?;
        backup.file.sync_all()?;
        fs::rename(&backup.path, &old)?;
        sync_directory(parent)?;
        fs::rename(candidate, target)?;
    }
    #[cfg(windows)]
    {
        // After an update the running daemon is mapped from `one-mk.old`, which
        // Windows will not let us delete: move it aside under a unique name
        // (removed later, best effort) instead.
        if old.exists() {
            regular(&old)?;
            let stale = parent.join(format!("one-mk.old.{}", now()?));
            fs::rename(&old, &stale).context("cannot move the previous executable aside; stop one-mk and retry")?;
        }
        for entry in fs::read_dir(parent)?.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("one-mk.old.") { let _ = fs::remove_file(entry.path()); }
        }
        fs::rename(target, &old)?;
        if let Err(error) = fs::rename(candidate, target) {
            fs::rename(&old, target).context("cannot restore old executable; rename one-mk.old back to one-mk.exe manually")?;
            return Err(error).context("cannot install the new executable; old executable restored");
        }
    }
    sync_directory(parent)
}

struct Hashed<'a> { file: &'a mut File, hash: digest::Context, total: u64, size: u64, percent: u64 }
fn progress(percent: u64) { println!("Downloading… {percent}%"); }
impl Write for Hashed<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.file.write(bytes)?;
        self.hash.update(&bytes[..count]);
        self.total += count as u64;
        let percent = self.total.saturating_mul(100) / self.size.max(1);
        if percent >= self.percent + 5 {
            self.percent = percent;
            progress(percent);
        }
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> { self.file.flush() }
}

fn download(asset: &policy::Asset, release: &Release, origin: &Url, parent: &Path) -> Result<Temporary> {
    println!("Downloading…");
    let mut temporary = Temporary::new(parent)?;
    let mut output = Hashed { file: &mut temporary.file, hash: digest::Context::new(&digest::SHA256), total: 0, size: asset.size, percent: 0 };
    let size = request(&asset_url(release, &asset.name, origin)?, origin, asset.size, &mut output)?;
    ensure!(size == asset.size, "download length does not match signed manifest; old executable was not replaced");
    ensure!(constant_time_eq::constant_time_eq(output.hash.finish().as_ref(), &policy::hex_bytes::<32>(&asset.sha256)?),
        "download checksum does not match signed manifest; old executable was not replaced");
    temporary.file.sync_all()?;
    Ok(temporary)
}

fn apply(manifest: &Manifest, release: &Release, origin: &Url, target: &Path, kind: InstallKind) -> Result<()> {
    refuse_root()?;
    let _lock = target_lock(target, "This app's folder can't be changed by your account. Choose Download instead")?;
    regular(target)?;
    let asset = policy::select_install(manifest, std::env::consts::OS, std::env::consts::ARCH, kind)?;
    let parent = target.parent().context("update target has no directory")?;
    let mut temporary = download(asset, release, origin, parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary.path, fs::Permissions::from_mode(0o700))?;
    }
    temporary.file.sync_all()?;
    let candidate = temporary.path.clone();
    temporary.file = File::open(&candidate)?;
    println!("Checking the update…");
    version_check(&candidate, Some(&manifest.version))?;
    println!("Installing…");
    replace(&candidate, target)?;
    Ok(())
}

/// A Linux package: the update is downloaded and checked against the signed
/// list as the person, then the system asks for the password once (pkexec,
/// with the polkit action the package ships) and a root step installs it.
/// The root step trusts nothing this account says: it is given only the
/// staging folder and verifies the signed list, the version and the package
/// itself before anything is installed.
#[cfg(unix)]
fn apply_deb(signed: &Signed, manifest: &Manifest, release: &Release, origin: &Url, home: &Path, program: &Path) -> Result<()> {
    refuse_root()?;
    let asset = policy::select_install(manifest, std::env::consts::OS, std::env::consts::ARCH, InstallKind::Deb)?;
    let staging = home.join(DEB_STAGING);
    let _ = fs::remove_dir_all(&staging);
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&staging).context("cannot prepare the update; check free disk space")?;
    }
    let result = (|| {
        let temporary = download(asset, release, origin, &staging)?;
        println!("Checking the update…");
        check_deb_fields(&deb_fields(&temporary.path)?, &manifest.version)?;
        stage_signed(&staging, signed)?;
        // The name the signed list gives it: the root step looks it up there.
        fs::rename(&temporary.path, staging.join(&asset.name)).context("cannot prepare the update; check free disk space")?;
        println!("Installing… Your computer asks for your password.");
        let status = root_command(program, &staging).stdin(Stdio::null()).status().context("this computer has no password prompt (pkexec); install the downloaded package with your package manager")?;
        match status.code() {
            Some(0) => Ok(()),
            Some(126) => bail!("The password was not given. Nothing was changed"),
            Some(127) => bail!("This account isn't allowed to install software. Nothing was changed"),
            _ => bail!("The package could not be installed (the reason is above). Check that no other install is running, then try again"),
        }
    })();
    let _ = fs::remove_dir_all(&staging);
    result?;
    restarting(&format!("oneMK {} installed", manifest.version), "Quit oneMK and open it again to use it");
    Ok(())
}

/// The one command run as root: the program and the staging folder, nothing
/// the root step would have to believe (no checksum, no version).
#[cfg(unix)]
fn root_command(program: &Path, staging: &Path) -> Command {
    let mut command = Command::new("pkexec");
    command.arg(program).args(["update", "--install-deb"]).arg(staging);
    command
}

#[cfg(unix)]
const DEB_STAGING: &str = "update-staging";
#[cfg(unix)]
const STAGED_MANIFEST: &str = "update-manifest.json";
#[cfg(unix)]
const STAGED_SIGNATURE: &str = "update-manifest.json.sig";
/// Where the root step keeps its own replay record and its private copy of
/// the package: a folder only root can use, never the person's settings.
#[cfg(unix)]
const ROOT_STATE: &str = "/var/lib/one-mk";

/// The signed list exactly as it was downloaded, next to the package.
#[cfg(unix)]
fn stage_signed(staging: &Path, signed: &Signed) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    for (name, bytes) in [(STAGED_MANIFEST, &signed.manifest), (STAGED_SIGNATURE, &signed.signature)] {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(staging.join(name))
            .context("cannot prepare the update; check free disk space")?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn deb_fields(path: &Path) -> Result<String> {
    let output = Command::new("/usr/bin/dpkg-deb").arg("--field").arg(path).args(["Package", "Version"]).output()
        .context("cannot read the downloaded package; is dpkg installed?")?;
    ensure!(output.status.success(), "the downloaded package is not a valid .deb");
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The package must be oneMK, at the version the signed list names.
#[cfg(unix)]
fn check_deb_fields(fields: &str, version: &str) -> Result<()> {
    let field = |name: &str| fields.lines().find_map(|line| line.strip_prefix(name)?.strip_prefix(':')).map(str::trim);
    ensure!(field("Package") == Some("one-mk"), "the downloaded package is not oneMK; nothing was installed");
    ensure!(field("Version") == Some(version), "the downloaded package is not version {version}; nothing was installed");
    Ok(())
}

/// Opens the staging folder without following a link and checks that it
/// belongs to the account that asked and that no one else can write to it.
/// Everything in it is then opened relative to this handle, so the folder
/// can't be swapped for another one in between.
#[cfg(unix)]
fn open_staging(path: &Path, owner: u32) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    ensure!(path.is_absolute() && path.file_name().is_some_and(|name| name == DEB_STAGING), "not an update folder; nothing was installed");
    let folder = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)
        .context("cannot open the update folder; nothing was installed")?;
    let meta = folder.metadata()?;
    ensure!(meta.is_dir() && meta.uid() == owner && meta.mode() & 0o022 == 0, "the update folder is not private to the account that asked; nothing was installed");
    Ok(folder)
}

/// Opens one file of the staging folder: a regular file (never a link, a
/// pipe or a device) owned by the account that asked, at most `limit` bytes.
#[cfg(unix)]
fn open_staged(folder: &File, name: &str, owner: u32, limit: u64) -> Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    ensure!(!name.is_empty() && !name.contains('/') && !name.starts_with('.'), "not an update file; nothing was installed");
    let text = std::ffi::CString::new(name)?;
    // SAFETY: a valid folder handle and a NUL-terminated name; the new
    // descriptor is owned by the File made from it.
    let fd = unsafe { libc::openat(folder.as_raw_fd(), text.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 { return Err(std::io::Error::last_os_error()).with_context(|| format!("cannot open {name} in the update folder; nothing was installed")); }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata()?;
    ensure!(meta.is_file() && meta.uid() == owner && meta.len() <= limit, "{name} in the update folder is not a file the update made; nothing was installed");
    Ok(file)
}

#[cfg(unix)]
fn read_staged(folder: &File, name: &str, owner: u32, limit: u64) -> Result<Vec<u8>> {
    let file = open_staged(folder, name, owner, limit)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "{name} in the update folder is too large; nothing was installed");
    Ok(bytes)
}

/// Checks a signature (message, signature) against the trusted key(s).
#[cfg(unix)]
type Verify = dyn Fn(&[u8], &[u8]) -> Result<()>;

/// What the root step relies on, apart from the staging folder. Tests pass
/// their own key, folders and a fake installer; the real step passes the
/// compiled-in key, this program's version and `dpkg`.
#[cfg(unix)]
struct RootStep<'a> {
    /// The account that asked (pkexec's `PKEXEC_UID`): it must own the staged files.
    owner: u32,
    /// The installed version: only a newer one is installed.
    installed: &'a str,
    /// The release sequence this build was made with: an older signed list is refused.
    floor: u64,
    /// A folder only root can use: the replay record and the private copy.
    state: &'a Path,
    verify: &'a Verify,
    fields: &'a dyn Fn(&Path) -> Result<String>,
    install: &'a dyn Fn(&Path) -> Result<()>,
}

/// Runs as root: verifies the signed list in the staging folder against the
/// compiled-in key, refuses anything not newer than what is installed (and
/// any list older than one this step already accepted), copies the package
/// the list names through one open handle into a root-only file while
/// hashing it, checks size, SHA-256, package name and version on that copy,
/// and installs that same copy. A file swapped in the staging folder after
/// it was opened changes nothing.
#[cfg(unix)]
fn install_staged(staging: &Path, step: &RootStep) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let folder = open_staging(staging, step.owner)?;
    let bytes = read_staged(&folder, STAGED_MANIFEST, step.owner, policy::MANIFEST_LIMIT as u64)?;
    let signed = read_staged(&folder, STAGED_SIGNATURE, step.owner, 64)?;
    (step.verify)(&bytes, &signed).context("the update list is not signed by the publisher; nothing was installed")?;
    let manifest = policy::parse(&bytes)?;
    let digest = hex(digest::digest(&digest::SHA256, &bytes).as_ref());
    fs::DirBuilder::new().recursive(true).mode(0o700).create(step.state).context("cannot prepare the install")?;
    fs::set_permissions(step.state, fs::Permissions::from_mode(0o700))?;
    let _lock = lock(step.state)?;
    clean_temporary(step.state, &[".one-mk-update-payload-"])?;
    let mut state = load_state(step.state)?;
    // Signed, but not newer than what is installed, older than a list this
    // step accepted before, revoked or too new for this installation: refused.
    ensure!(policy::accept(&manifest, step.installed, &state, &digest, step.floor)?, "this is not a newer version; nothing was installed");
    let asset = policy::select_install(&manifest, "linux", std::env::consts::ARCH, InstallKind::Deb)?;
    let mut source = open_staged(&folder, &asset.name, step.owner, asset.size)?;
    drop(folder);
    let mut copy = Temporary::new(step.state)?;
    fs::set_permissions(&copy.path, fs::Permissions::from_mode(0o600))?;
    let mut hash = digest::Context::new(&digest::SHA256);
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 { break; }
        total += count as u64;
        ensure!(total <= asset.size, "the package is larger than the signed release says; nothing was installed");
        hash.update(&buffer[..count]);
        copy.file.write_all(&buffer[..count])?;
    }
    drop(source);
    ensure!(total == asset.size, "the package size does not match the signed release; nothing was installed");
    ensure!(constant_time_eq::constant_time_eq(hash.finish().as_ref(), &policy::hex_bytes::<32>(&asset.sha256)?),
        "the package does not match the signed release; nothing was installed");
    copy.file.sync_all()?;
    check_deb_fields(&(step.fields)(&copy.path)?, &manifest.version)?;
    (step.install)(&copy.path)?;
    state.sequence = manifest.sequence;
    state.digest = digest;
    state.highest_version = Some(manifest.version.clone());
    state.available = None;
    state.error = None;
    state.last_attempt = now()?;
    save_state(step.state, &state)
}

/// Runs as root (through pkexec): installs the package staged by the
/// person's own update, after checking everything again (`install_staged`).
#[cfg(unix)]
fn install_deb_as_root(staging: &Path) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "this step is run by the update, with your password");
    let owner: u32 = std::env::var("PKEXEC_UID").ok().and_then(|uid| uid.parse().ok()).context("this step is run by the update, with your password")?;
    ensure!(owner != 0, "this step is run by the update from your own account");
    ensure!(crate::update_key::PRODUCTION_KEY, "release verification is not configured in this build; install the package with your package manager");
    let install = |copy: &Path| -> Result<()> {
        let status = Command::new("/usr/bin/dpkg").arg("-i").arg(copy).env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").env("DEBIAN_FRONTEND", "noninteractive")
            .stdin(Stdio::null()).status().context("dpkg could not be started")?;
        ensure!(status.success(), "dpkg could not install the update");
        Ok(())
    };
    install_staged(staging, &RootStep {
        owner, installed: env!("CARGO_PKG_VERSION"), floor: FLOOR.and_then(|value| value.parse().ok()).unwrap_or(0),
        state: Path::new(ROOT_STATE), verify: &verify_embedded, fields: &deb_fields, install: &install,
    })
}

fn rollback(target: &Path) -> Result<()> {
    refuse_root()?;
    let _lock = target_lock(target, "This app's folder can't be changed by your account")?;
    let old = target.parent().context("update target has no directory")?.join("one-mk.old");
    regular(&old).context("no safe previous executable; reinstall the complete release manually")?;
    version_check(&old, None)?;
    let mut temporary = Temporary::new(target.parent().context("update target has no directory")?)?;
    std::io::copy(&mut File::open(&old)?, &mut temporary.file)?;
    fs::set_permissions(&temporary.path, fs::metadata(&old)?.permissions())?;
    temporary.file.sync_all()?;
    if target.try_exists()? {
        replace(&temporary.path, target)
    } else {
        atomic_rename(&temporary.path, target)?;
        sync_directory(target.parent().context("update target has no directory")?)
    }
}

pub fn run(home: &Path, options: Options) -> Result<()> {
    run_inner(home, options).map_err(|error| {
        tracing::debug!(error = ?error, "explicit update failed");
        let message = if error.chain().count() == 1 && error.is::<std::io::Error>() {
            format!("cannot access update files or the update server ({error}); check directory permissions, free disk space and your connection, then retry")
        } else {
            error.to_string()
        };
        anyhow::anyhow!(message)
    })
}

fn window_status(home: &Path) -> Result<policy::UpdateStatus> {
    let settings = crate::settings::Settings::load(home)?.update;
    let state = load_state(home)?;
    let (executable, kind) = target()?;
    let movable = kind == InstallKind::MacApp && policy::translocated(&executable.to_string_lossy());
    #[cfg(target_os = "macos")]
    let rollback_available = kind == InstallKind::MacApp && bundle::rollback_available(&executable);
    #[cfg(not(target_os = "macos"))]
    let rollback_available = false;
    // A portable copy updates only this program: with the window and tray
    // beside it they would be left at the old version, so download instead.
    let leaves_behind = kind == InstallKind::Portable && executable.parent().is_some_and(|dir| {
        ["one-mk-app", "one-mk-app.exe", "one-mk-tray", "one-mk-tray.exe", "one-mk-wallpaper", "one-mk-wallpaper.exe"].iter().any(|name| dir.join(name).exists())
    });
    Ok(policy::UpdateStatus {
        check: settings.check, install_automatically: settings.install_automatically,
        available: policy::cached_version(home, env!("CARGO_PKG_VERSION")), error: state.error,
        can_update: kind.installs_in_app() && !movable && !leaves_behind, needs_password: kind.needs_password(),
        move_to_applications: movable, rollback_available,
        advice: if movable {
            "Move oneMK to Applications before updating".into()
        } else if leaves_behind {
            "Download the complete release to update this copy".into()
        } else {
            kind.advice().into()
        },
        skipped: policy::skipped_version(home),
    })
}

fn check_release(home: &Path, endpoint: &str, state: &mut State) -> Result<Checked> {
    state.last_attempt = now()?;
    save_state(home, state)?;
    match checked(home, endpoint, &crate::update_key::PUBLIC_KEY, env!("CARGO_PKG_VERSION"), state) {
        Ok(release) => Ok(release),
        Err(error) => {
            state.error = Some(error.to_string());
            save_state(home, state)?;
            Err(error)
        }
    }
}

fn run_inner(home: &Path, options: Options) -> Result<()> {
    // The one step that runs as root, started by `apply_deb` through pkexec.
    if let Some(staging) = &options.install_deb {
        #[cfg(unix)]
        return install_deb_as_root(staging);
        #[cfg(not(unix))]
        bail!("package installs exist only on Linux; nothing was installed ({})", staging.display());
    }
    refuse_root()?;
    if options.status_json {
        println!("{}", serde_json::to_string(&window_status(home)?)?);
        return Ok(());
    }
    if let Some(value) = options.install_automatically {
        crate::settings::Settings::update(home, |settings| { settings.update.install_automatically = value == "on"; Ok(()) })?;
        return Ok(());
    }
    if let Some(value) = options.check_on_start {
        crate::settings::Settings::update(home, |settings| { settings.update.check = value == "on"; Ok(()) })?;
        println!("Update checks are {}.", if value == "on" { "on" } else { "off" });
        return Ok(());
    }
    let (target, kind) = target()?;
    if let Some(started_version) = options.started_version {
        #[cfg(target_os = "macos")]
        if kind == InstallKind::MacApp { bundle::acknowledge(&target, &started_version)?; }
        let _ = started_version;
        return Ok(());
    }
    let _lock = lock(home)?;
    if options.move_to_applications {
        #[cfg(target_os = "macos")]
        { return bundle::move_to_applications(&target); }
        #[cfg(not(target_os = "macos"))]
        bail!("Moving to Applications is available only on a Mac");
    }
    if options.rollback {
        ensure!(kind.automatic(), "this installation is managed: {}", kind.advice());
        // Remember the version gone back from, so it isn't installed again
        // by itself the next time the window opens.
        let skip = || {
            let _ = one_mk_ctl::private::write(&home.join(policy::SKIPPED), env!("CARGO_PKG_VERSION"));
        };
        #[cfg(target_os = "macos")]
        if kind == InstallKind::MacApp {
            bundle::rollback_bundle(&target, home)?;
            skip();
            return Ok(());
        }
        rollback(&target)?;
        skip();
        restarting("Previous oneMK restored", "Quit oneMK and open it again to use it");
        return Ok(());
    }
    ensure!(crate::update_key::PRODUCTION_KEY, "release verification is not configured: publisher must replace the test key; install the complete release manually");
    let settings = crate::settings::Settings::load(home)?.update;
    let endpoint = options.url.or(settings.url).unwrap_or_else(|| LATEST.into());
    let mut state = load_state(home)?;
    if options.startup_check && (!settings.check || !policy::startup_due(state.last_attempt, now()?)) { return Ok(()); }
    let (manifest, release, origin, signed) = check_release(home, &endpoint, &mut state)?;
    if state.available.is_none() { println!("one-mk {} is up to date.", env!("CARGO_PKG_VERSION")); return Ok(()); }
    println!("one-mk {} is available (you have {}).", manifest.version, env!("CARGO_PKG_VERSION"));
    if options.check || options.startup_check { return Ok(()); }
    if kind == InstallKind::Deb {
        #[cfg(unix)]
        return apply_deb(&signed, &manifest, &release, &origin, home, &target);
        #[cfg(not(unix))]
        { let _ = signed; bail!("{}", kind.advice()); }
    }
    if !kind.automatic() {
        bail!("{}. Choose Download to open the release page: {NOTES}", kind.advice());
    }
    #[cfg(target_os = "macos")]
    if kind == InstallKind::MacApp { return bundle::apply_bundle(&manifest, &release, &origin, &target, home); }
    apply(&manifest, &release, &origin, &target, kind)?;
    restarting(&format!("oneMK {} installed", manifest.version), "Quit oneMK and open it again to use it");
    Ok(())
}

pub async fn stop(home: &Path) -> Result<()> {
    if crate::control::request(home, crate::control::Request::Stop).await?.is_none() {
        println!("one-mk is not running for this settings folder.");
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if crate::control::request(home, crate::control::Request::Status).await?.is_none() {
            println!("one-mk stopped.");
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "one-mk is still stopping; wait a moment, then check one-mk status before restarting");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub fn startup(home: PathBuf) {
    if !crate::update_key::PRODUCTION_KEY || refuse_root().is_err() { return; }
    let settings = match crate::settings::Settings::load(&home) {
        Ok(settings) if settings.update.check => settings.update,
        _ => return,
    };
    let spawned = std::thread::Builder::new().name("one-mk-update".into()).spawn(move || {
        let result = (|| -> Result<()> {
            let _lock = lock(&home)?;
            let mut state = load_state(&home)?;
            let time = now()?;
            if !policy::startup_due(state.last_attempt, time) { return Ok(()); }
            check_release(&home, settings.url.as_deref().unwrap_or(LATEST), &mut state)?;
            Ok(())
        })();
        if let Err(error) = result { tracing::debug!(%error, "startup update check failed"); }
    });
    if let Err(error) = spawned { tracing::debug!(%error, "cannot start update check"); }
}

pub fn notice(home: &Path) -> Option<String> {
    let state = load_state(home).ok()?;
    let available = state.available?;
    (policy::compare(&available, env!("CARGO_PKG_VERSION")).ok()? == std::cmp::Ordering::Greater)
        .then(|| format!("one-mk {available} is available; run one-mk update"))
}

pub fn doctor(home: &Path) -> (bool, String) {
    match load_state(home) {
        Ok(state) if state.error.is_some() => (false, format!("Couldn't check: {}", state.error.unwrap_or_default())),
        Ok(state) if state.sequence > 0 => {
            if policy::cached_version(home, env!("CARGO_PKG_VERSION")).is_some() {
                (true, "Update available — Update now".into())
            } else { (false, format!("Up to date ({})", env!("CARGO_PKG_VERSION"))) }
        }
        Ok(_) => (false, "Couldn't check: no completed update check yet".into()),
        Err(_) => (false, "Couldn't check: saved update information couldn't be read".into()),
    }
}

#[cfg(all(test, unix))]
#[path = "update_tests.rs"]
mod tests;
