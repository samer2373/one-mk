use crate::identity::{Fingerprint, Identity};
use crate::tls;
use anyhow::{Context, Result, anyhow, bail};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::pki_types::CertificateDer;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};

/// At most this many TLS handshakes run at once; more are refused.
const MAX_HANDSHAKES: usize = 16;
/// At most this many handshakes from one IP address at once.
const MAX_HANDSHAKES_PER_IP: usize = 4;
/// A handshake that hasn't finished by now is dropped. On a LAN it takes
/// a few milliseconds.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// A QUIC endpoint that both accepts and dials connections; every device is
/// an equal peer, there is no "server" or "client" role.
#[derive(Clone)]
pub struct Node {
    endpoint: quinn::Endpoint,
    client: quinn::ClientConfig,
    identity: Arc<Identity>,
    accepted: Arc<Accepted>,
}

/// Authenticated incoming connections, filled by a background task that
/// runs handshakes concurrently (started by the first `accept()`).
struct Accepted {
    start: Mutex<Option<mpsc::Sender<Peer>>>,
    rx: tokio::sync::Mutex<mpsc::Receiver<Peer>>,
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

        let socket = UdpSocket::bind(addr).with_context(|| format!("binding UDP {addr}"))?;
        let runtime = quinn::default_runtime().context("no async runtime")?;
        let endpoint =
            quinn::Endpoint::new(quinn::EndpointConfig::default(), Some(server), socket, runtime)?;
        let (tx, rx) = mpsc::channel(MAX_HANDSHAKES);
        let accepted = Arc::new(Accepted { start: Mutex::new(Some(tx)), rx: tokio::sync::Mutex::new(rx) });
        Ok(Self { endpoint, client, identity, accepted })
    }

    /// Binds `port` on all interfaces: a dual-stack IPv6 socket (`[::]`) so
    /// one port serves both IPv4 and IPv6 peers, or IPv4 only (`0.0.0.0`)
    /// where IPv6 is unavailable. Port 0 picks a free port.
    pub fn bind_any(identity: Arc<Identity>, port: u16) -> Result<Self> {
        let v6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let v4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
        match Self::bind(identity.clone(), v6) {
            Ok(node) => Ok(node),
            Err(e6) => Self::bind(identity, v4).map_err(|e4| {
                anyhow!("could not open UDP port {port} (IPv6 {v6}: {}; IPv4 {v4}: {})", root(&e6), root(&e4))
            }),
        }
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
        let peer = Self::peer(conn, true)?;
        if peer.fingerprint == self.identity.fingerprint {
            // E.g. a stale or virtual-interface address that loops back here.
            peer.conn.close(0u32.into(), b"self");
            bail!("{addr} is this computer itself");
        }
        Ok(peer)
    }

    /// Like `connect`, but gives up after `timeout`.
    pub async fn connect_timeout(&self, addr: SocketAddr, timeout: Duration) -> Result<Peer> {
        match tokio::time::timeout(timeout, self.connect(addr)).await {
            Ok(res) => res,
            Err(_) => bail!("no answer from {addr} within {} s", timeout.as_secs_f32()),
        }
    }

    /// Waits for the next authenticated incoming connection. `None` once
    /// the node is closed.
    ///
    /// Handshakes run concurrently in the background (at most
    /// `MAX_HANDSHAKES` at once, `MAX_HANDSHAKES_PER_IP` per source address,
    /// each limited to `HANDSHAKE_TIMEOUT`), so a peer that stalls mid-
    /// handshake cannot hold up anyone else. Failed handshakes are logged
    /// and skipped, so this never returns `Some(Err(_))` today; the `Result`
    /// is kept for API stability.
    pub async fn accept(&self) -> Option<Result<Peer>> {
        let tx = self.accepted.start.lock().expect("not poisoned").take();
        if let Some(tx) = tx {
            tokio::spawn(accept_loop(self.endpoint.clone(), tx));
        }
        self.accepted.rx.lock().await.recv().await.map(Ok)
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

/// The innermost cause, which is the useful part for a one-line message.
fn root(e: &anyhow::Error) -> String {
    e.root_cause().to_string()
}

async fn accept_loop(endpoint: quinn::Endpoint, tx: mpsc::Sender<Peer>) {
    let slots = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    let per_ip: Arc<Mutex<HashMap<IpAddr, usize>>> = Arc::default();
    while let Some(incoming) = endpoint.accept().await {
        let ip = incoming.remote_address().ip().to_canonical();
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            tracing::debug!(%ip, "too many handshakes in progress; refusing");
            incoming.refuse();
            continue;
        };
        let Some(ip_slot) = IpSlot::take(&per_ip, ip) else {
            tracing::debug!(%ip, "too many handshakes from this address; refusing");
            incoming.refuse();
            continue;
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            let res = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
                let conn = incoming.await?;
                Node::peer(conn, false)
            })
            .await;
            drop(ip_slot);
            match res {
                Ok(Ok(peer)) => {
                    // Keep the slot until `accept()` takes the peer, so a
                    // caller that stops accepting also stops new handshakes.
                    let _ = tx.send(peer).await;
                }
                Ok(Err(e)) => tracing::debug!(%ip, "incoming handshake failed: {e:#}"),
                Err(_) => tracing::debug!(%ip, "incoming handshake timed out"),
            }
            drop(permit);
        });
    }
}

/// One in-flight handshake from `ip`; released on drop.
struct IpSlot {
    map: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl IpSlot {
    fn take(map: &Arc<Mutex<HashMap<IpAddr, usize>>>, ip: IpAddr) -> Option<Self> {
        let mut m = map.lock().expect("not poisoned");
        let n = m.entry(ip).or_default();
        if *n >= MAX_HANDSHAKES_PER_IP {
            return None;
        }
        *n += 1;
        Some(Self { map: map.clone(), ip })
    }
}

impl Drop for IpSlot {
    fn drop(&mut self) {
        let mut m = self.map.lock().expect("not poisoned");
        if let Some(n) = m.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                m.remove(&self.ip);
            }
        }
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
    // Control, clipboard and a few parallel file streams; one-mk never
    // uses unidirectional streams, so the peer may not open any.
    t.max_concurrent_bidi_streams(16u32.into());
    t.max_concurrent_uni_streams(0u32.into());
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_ip_limit_is_released_on_drop() {
        let map = Arc::default();
        let ip: IpAddr = [10, 0, 0, 1].into();
        let slots: Vec<_> = (0..MAX_HANDSHAKES_PER_IP).map(|_| IpSlot::take(&map, ip).unwrap()).collect();
        assert!(IpSlot::take(&map, ip).is_none());
        assert!(IpSlot::take(&map, [10, 0, 0, 2].into()).is_some());
        drop(slots);
        assert!(IpSlot::take(&map, ip).is_some());
        assert!(map.lock().unwrap().is_empty());
    }
}
