//! TLS for QUIC. Devices use self-signed certificates and authenticate each
//! other by fingerprint (pinned at pairing time), not through a CA. The
//! ordinary verifiers accept any certificate but check its handshake signature.
//! Invitation clients instead pin the server inside the TLS verifier, before
//! disclosing their client certificate. Other callers check the trust store.

use crate::identity::Identity;
use anyhow::Result;
use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DistinguishedName, SignatureScheme};
use std::sync::Arc;

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn server_config(id: &Identity) -> Result<rustls::ServerConfig> {
    let p = provider();
    let mut cfg = rustls::ServerConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(AnyCert(p.signature_verification_algorithms)))
        .with_single_cert(vec![id.cert.clone()], id.key())?;
    cfg.alpn_protocols = vec![one_mk_proto::ALPN.to_vec()];
    Ok(cfg)
}

pub fn client_config(id: &Identity) -> Result<rustls::ClientConfig> {
    client_config_inner(id, None)
}

pub fn pinned_client_config(id: &Identity, expected: crate::Fingerprint) -> Result<rustls::ClientConfig> {
    client_config_inner(id, Some(expected))
}

fn client_config_inner(id: &Identity, expected: Option<crate::Fingerprint>) -> Result<rustls::ClientConfig> {
    let p = provider();
    let mut cfg = rustls::ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCert { algorithms: AnyCert(p.signature_verification_algorithms), expected }))
        .with_client_auth_cert(vec![id.cert.clone()], id.key())?;
    cfg.alpn_protocols = vec![one_mk_proto::ALPN.to_vec()];
    Ok(cfg)
}

#[derive(Debug)]
struct AnyCert(WebPkiSupportedAlgorithms);

#[derive(Debug)]
struct PinnedCert {
    algorithms: AnyCert,
    expected: Option<crate::Fingerprint>,
}

impl ServerCertVerifier for PinnedCert {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], server_name: &ServerName<'_>, ocsp_response: &[u8], now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        if self.expected.is_some_and(|expected| expected != crate::Fingerprint::of(end_entity)) {
            return Err(rustls::Error::General("invitation server certificate mismatch".into()));
        }
        self.algorithms.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        ServerCertVerifier::verify_tls12_signature(&self.algorithms, message, cert, dss)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.algorithms.verify13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.0.supported_schemes()
    }
}

impl AnyCert {
    fn verify13(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }
}

impl ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verify13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

impl ClientCertVerifier for AnyCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verify13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_checked_before_the_server_receives_a_client_certificate() {
        let server_id = Identity::ephemeral().unwrap();
        let client_id = Identity::ephemeral().unwrap();
        for expected in [server_id.fingerprint, client_id.fingerprint] {
            let mut client = rustls::ClientConnection::new(Arc::new(pinned_client_config(&client_id, expected).unwrap()), ServerName::try_from("one-mk").unwrap()).unwrap();
            let mut server = rustls::ServerConnection::new(Arc::new(server_config(&server_id).unwrap())).unwrap();
            let mut rejected = false;
            for _ in 0..8 {
                let mut outgoing = Vec::new();
                client.write_tls(&mut outgoing).unwrap();
                server.read_tls(&mut std::io::Cursor::new(outgoing)).unwrap();
                if server.process_new_packets().is_err() { break; }
                let mut incoming = Vec::new();
                server.write_tls(&mut incoming).unwrap();
                client.read_tls(&mut std::io::Cursor::new(incoming)).unwrap();
                if client.process_new_packets().is_err() { rejected = true; }
            }
            if expected == server_id.fingerprint {
                assert!(!rejected);
                assert!(server.peer_certificates().is_some());
            } else {
                assert!(rejected);
                assert!(server.peer_certificates().is_none());
            }
        }
    }
}
