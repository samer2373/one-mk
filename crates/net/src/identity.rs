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
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join("identity.cert.der");
        let key_path = dir.join("identity.key.der");
        let (cert, key) = match (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            (Ok(c), Ok(k)) => (c, k),
            _ => {
                let (c, k) = generate()?;
                std::fs::create_dir_all(dir)?;
                write_private(&key_path, &k)?;
                std::fs::write(&cert_path, &c)?;
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
            .open(path)?;
        f.write_all(data)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
        Ok(())
    }
}
