//! Explicitly opted-in, anonymous aggregate analytics. Never accepts user properties.
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// One anonymous session per process, unrelated to the paired-device identity.
#[derive(Clone)]
pub struct Session {
    key: String,
    host: &'static str,
    id: String,
}

impl Session {
    pub fn from_env() -> Option<Self> {
        Self::configured(
            std::env::var("ONE_MK_ANALYTICS_OPT_IN").ok().as_deref(),
            std::env::var("ONE_MK_POSTHOG_KEY").ok().as_deref(),
            std::env::var("ONE_MK_POSTHOG_HOST").ok().as_deref(),
        )
    }

    fn configured(consent: Option<&str>, key: Option<&str>, host: Option<&str>) -> Option<Self> {
        if consent != Some("true") { return None; }
        let key = key.filter(|key| key.starts_with("phc_") && key.len() > 4 && key.len() <= 256
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))?;
        let host = match host? {
            "https://us.i.posthog.com" => "us.i.posthog.com",
            "https://eu.i.posthog.com" => "eu.i.posthog.com",
            _ => return None,
        };
        Some(Self { key: key.into(), host, id: format!("{:032x}", rand::random::<u128>()) })
    }

    fn payload(&self, event: Event, succeeded: Option<bool>) -> Vec<u8> {
        let mut properties = serde_json::json!({
            "distinct_id": self.id,
            "$process_person_profile": false,
            "$geoip_disable": true,
            "app_version": env!("CARGO_PKG_VERSION"),
        });
        if let Some(succeeded) = succeeded { properties["succeeded"] = succeeded.into(); }
        serde_json::to_vec(&serde_json::json!({
            "api_key": self.key, "event": event.name(), "properties": properties,
        })).expect("fixed analytics payload is serializable")
    }

    /// Best effort, no retry or disk queue. DNS/TLS never block the app beyond
    /// this budget. A slow worker is discarded when the process exits.
    pub fn capture(&self, event: Event, succeeded: Option<bool>) {
        let session = self.clone();
        let (done, wait) = mpsc::sync_channel(1);
        if std::thread::Builder::new().name("anonymous-analytics".into()).spawn(move || {
            let _ = session.send(&session.payload(event, succeeded));
            let _ = done.send(());
        }).is_ok() { let _ = wait.recv_timeout(Duration::from_millis(300)); }
    }

    fn send(&self, body: &[u8]) -> anyhow::Result<()> {
        let address = (self.host, 443).to_socket_addrs()?.next()
            .ok_or_else(|| anyhow::anyhow!("analytics host unavailable"))?;
        let socket = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        socket.set_write_timeout(Some(Duration::from_secs(1)))?;
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?.with_root_certificates(roots).with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(self.host)?;
        let mut stream = rustls::StreamOwned::new(rustls::ClientConnection::new(Arc::new(config), name)?, socket);
        write!(stream, "POST /capture/ HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", self.host, body.len())?;
        stream.write_all(body)?;
        stream.flush()?;
        // Wait for a response without storing its body, following redirects,
        // logging failures or allowing analytics errors to affect the command.
        let mut response = [0; 128];
        let _ = stream.read(&mut response)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub enum Event { DaemonStarted, DaemonStopped, PairingFinished, LayoutFinished, ProfileFinished }

impl Event {
    fn name(self) -> &'static str {
        match self {
            Self::DaemonStarted => "onemk_daemon_launch_requested",
            Self::DaemonStopped => "onemk_daemon_run_finished",
            Self::PairingFinished => "onemk_pairing_command_finished",
            Self::LayoutFinished => "onemk_layout_command_finished",
            Self::ProfileFinished => "onemk_profile_command_finished",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_explicit_consent_key_and_cloud_region() {
        let key = Some("phc_test_only");
        let host = Some("https://eu.i.posthog.com");
        for consent in [None, Some("false"), Some("1"), Some("TRUE")] {
            assert!(Session::configured(consent, key, host).is_none());
        }
        for host in [None, Some("http://eu.i.posthog.com"), Some("https://evil.example"), Some("https://eu.i.posthog.com/path")] {
            assert!(Session::configured(Some("true"), key, host).is_none());
        }
        assert!(Session::configured(Some("true"), None, host).is_none());
        assert!(Session::configured(Some("true"), Some("phc_"), host).is_none());
        assert!(Session::configured(Some("true"), Some("phc_bad\r\n"), host).is_none());
        assert!(Session::configured(Some("true"), key, host).is_some());
    }

    #[test]
    fn fixed_schema_and_session_identity_only() {
        let session = Session::configured(Some("true"), Some("phc_test_only"), Some("https://us.i.posthog.com")).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&session.payload(Event::PairingFinished, Some(false))).unwrap();
        assert_eq!(payload["event"], "onemk_pairing_command_finished");
        let properties = payload["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 5);
        assert_eq!(properties["$process_person_profile"], false);
        assert_eq!(properties["$geoip_disable"], true);
        assert_eq!(properties["succeeded"], false);
        assert_eq!(properties["distinct_id"], session.id);
        let next = Session::configured(Some("true"), Some("phc_test_only"), Some("https://us.i.posthog.com")).unwrap();
        assert_ne!(session.id, next.id);
    }
}
