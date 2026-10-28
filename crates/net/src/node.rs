use crate::identity::{Fingerprint, Identity};
use crate::tls;
use anyhow::{Context, Result, anyhow};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::pki_types::CertificateDer;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

/// A QUIC endpoint that both accepts and dials connections; every device is
/// an equal peer, there is no "server" or "client" role.
#[derive(Clone)]
pub struct Node {
    endpoint: quinn::Endpoint,
    client: quinn::ClientConfig,
    identity: Arc<Identity>,
}

/// An established, authenticated connection (not necessarily trusted yet).
pub struct Peer {
    pub conn: quinn::Connection,
    pub fingerprint: Fingerprint,
    /// Whether we dialed this connection.
    pub initiator: bool,
}

impl Node {
    pub fn bind(identity: Arc<Identity>, addr: SocketAddr) -> Result<Self> {
        let transport = Arc::new(transport());

        let mut server = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(
            tls::server_config(&identity)?,
        )?));
        server.transport_config(transport.clone());

        let mut client = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(
            tls::client_config(&identity)?,
        )?));
        client.transport_config(transport);

        let socket = bind_socket(addr)?;
        let runtime = quinn::default_runtime().context("no async runtime")?;
        let endpoint =
            quinn::Endpoint::new(quinn::EndpointConfig::default(), Some(server), socket, runtime)?;
        Ok(Self { endpoint, client, identity })
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    pub async fn connect(&self, addr: SocketAddr) -> Result<Peer> {
        let conn = self
            .endpoint
            .connect_with(self.client.clone(), addr, "one-mk")?
            .await
            .with_context(|| format!("connecting to {addr}"))?;
        Self::peer(conn, true)
    }

    /// Waits for the next incoming connection. `None` once the node is closed.
    pub async fn accept(&self) -> Option<Result<Peer>> {
        let incoming = self.endpoint.accept().await?;
        Some(async move { Self::peer(incoming.await?, false) }.await)
    }

    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"bye");
    }

    fn peer(conn: quinn::Connection, initiator: bool) -> Result<Peer> {
        let certs = conn
            .peer_identity()
            .and_then(|id| id.downcast::<Vec<CertificateDer<'static>>>().ok())
            .ok_or_else(|| anyhow!("peer presented no certificate"))?;
        let cert = certs.first().ok_or_else(|| anyhow!("peer presented no certificate"))?;
        Ok(Peer { fingerprint: Fingerprint::of(cert), conn, initiator })
    }
}

fn transport() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    // Notice a dead peer quickly so held keys are released and the pointer
    // comes home, but survive brief Wi-Fi hiccups.
    t.max_idle_timeout(Some(Duration::from_secs(4).try_into().expect("valid timeout")));
    t.keep_alive_interval(Some(Duration::from_secs(1)));
    // LAN round trips are sub-millisecond; don't start from the 333 ms default.
    t.initial_rtt(Duration::from_millis(2));
    t.datagram_receive_buffer_size(Some(256 * 1024));
    t.datagram_send_buffer_size(256 * 1024);
    t
}

/// Binds a UDP socket, preferring a dual-stack IPv6 socket so one port
/// serves both IPv4 and IPv6 peers.
fn bind_socket(addr: SocketAddr) -> Result<UdpSocket> {
    let socket = UdpSocket::bind(addr).with_context(|| format!("binding UDP {addr}"))?;
    Ok(socket)
}
