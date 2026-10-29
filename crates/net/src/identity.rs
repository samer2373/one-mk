use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// SHA-256 of a device's certificate. This is the device's permanent id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of(cert: &[u8]) -> Self {
        Self(Sha256::digest(cert).into())
    }

    /// Short form for humans, e.g. `3f9a-1c07`.
    pub fn short(&self) -> String {
        let h = self.to_string();
        format!("{}-{}", &h[..4], &h[4..8])
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl FromStr for Fingerprint {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        anyhow::ensure!(s.len() == 64, "fingerprint must be 64 hex characters");
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).context("invalid fingerprint")?;
        }
        Ok(Self(out))
    }
}

/// This device's long-lived certificate and key.
pub struct Identity {
    pub name: String,
    pub cert: CertificateDer<'static>,
    key: Vec<u8>,
    pub fingerprint: Fingerprint,
}

impl Identity {
    /// Loads the identity from `dir`, creating it on first run.
    ///
    /// The private key must only be readable by this user: every load
    /// tightens the key file to 0600 and `dir` to 0700 (on Unix) if they
    /// are wider.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join("identity.cert.der");
        let key_path = dir.join("identity.key.der");
        secure_dir(dir)?;
        let (cert, key) = match (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            (Ok(c), Ok(k)) => {
                restrict_file(&key_path)?;
                (c, k)
            }
            _ => {
                let (c, k) = generate()?;
                write_private(&key_path, &k)?;
                std::fs::write(&cert_path, &c)
                    .with_context(|| format!("writing {}", cert_path.display()))?;
                (c, k)
            }
        };
        Ok(Self::from_parts(cert, key))
    }

    /// A throwaway identity (benchmarks and tests).
    pub fn ephemeral() -> Result<Self> {
        let (c, k) = generate()?;
        Ok(Self::from_parts(c, k))
    }

    fn from_parts(cert: Vec<u8>, key: Vec<u8>) -> Self {
        Self {
            name: device_name(),
            fingerprint: Fingerprint::of(&cert),
            cert: CertificateDer::from(cert),
            key,
        }
    }

    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()))
    }
}

fn generate() -> Result<(Vec<u8>, Vec<u8>)> {
    let ck = rcgen::generate_simple_self_signed(vec!["one-mk".to_owned()])?;
    Ok((ck.cert.der().to_vec(), ck.signing_key.serialize_der()))
}

fn device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "one-mk device".to_owned())
}

/// Creates `dir` if needed and makes it private to this user (0700 on
/// Unix).
// TODO(windows): set an ACL granting only the current user access. Today
// the folder relies on the default (per-user) permissions of
// %LOCALAPPDATA%.
pub fn secure_dir(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let builder = &mut std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(builder, 0o700);
        match builder.create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
        }
    }
    #[cfg(unix)]
    tighten(dir, 0o700)?;
    Ok(())
}

/// Makes an existing file private to this user (0600 on Unix).
fn restrict_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    tighten(path, 0o600)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Removes any permission bits outside `max`.
#[cfg(unix)]
fn tighten(path: &Path, max: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.permissions().mode();
    if mode & 0o777 & !max != 0 {
        tracing::warn!("{} was accessible to others (mode {:o}); fixing", path.display(), mode & 0o777);
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777 & max))
            .with_context(|| format!("restricting permissions of {}", path.display()))?;
    }
    Ok(())
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        f.write_all(data)?;
        // `mode` only applies to new files; an existing one keeps its bits.
        restrict_file(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn load_tightens_key_and_dir_permissions() {
        let dir = std::env::temp_dir().join(format!("one-mk-perm-{}-{:x}", std::process::id(), rand::random::<u64>()));
        let created = Identity::load_or_create(&dir).unwrap();
        let key = dir.join("identity.key.der");
        assert_eq!(mode(&key), 0o600);
        assert_eq!(mode(&dir), 0o700);

        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let loaded = Identity::load_or_create(&dir).unwrap();
        assert_eq!(loaded.fingerprint, created.fingerprint);
        assert_eq!(mode(&key), 0o600);
        assert_eq!(mode(&dir), 0o700);

        // Narrower permissions are left alone.
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o400)).unwrap();
        Identity::load_or_create(&dir).unwrap();
        assert_eq!(mode(&key), 0o400);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
