//! Transport-level behaviour over real QUIC on loopback: stream framing
//! limits, version/ALPN/authentication refusals, handshake flood limits,
//! dead-peer detection, brief outages, shutdown notices and the failure
//! paths of the pairing exchange. Nothing here leaves 127.0.0.1.

use one_mk_net::quinn::{RecvStream, SendStream};
use one_mk_net::{
    ControlWriter, Identity, Node, Pairing, Peer, open_stream, read_clip_header, read_file_msg, read_frame,
    read_stream_kind, write_file_msg,
};
use one_mk_proto::{Batch, ClipFormat, ClipHeader, Control, FileMsg, StreamKind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

fn node() -> Node {
    Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap()
}

/// `(dialer, acceptor)` already connected to each other.
async fn connected(a: &Node, b: &Node) -> (Peer, Peer) {
    let (pa, pb) = tokio::join!(a.connect(b.local_addr().unwrap()), async { b.accept().await.unwrap() });
    (pa.unwrap(), pb.unwrap())
}

/// Opens a stream of `kind` from `from` and returns what `to` accepts, so a test can
/// write raw bytes on one end and read them with the real readers on the other.
async fn stream_pair(from: &Peer, to: &Peer, kind: StreamKind) -> (SendStream, RecvStream) {
    let (send, _recv) = from.open_stream(kind).await.unwrap();
    let (got, _send_back, recv) = to.accept_stream().await.unwrap();
    assert_eq!(got, kind);
    (send, recv)
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = (body.len() as u16).to_le_bytes().to_vec();
    out.extend_from_slice(body);
    out
}

// ------------------------------------------------------------ stream framing

#[tokio::test]
async fn control_frames_larger_than_the_limit_are_refused_before_they_are_read() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Control).await;
    // A length prefix that promises more than Control::MAX_LEN: no body follows, and the
    // reader must not wait for (or allocate) it.
    send.write_all(&((Control::MAX_LEN + 1) as u16).to_le_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let err = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut recv, &mut buf))
        .await
        .expect("must not wait for the body")
        .unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    assert!(buf.capacity() < Control::MAX_LEN, "nothing was allocated for it");
}

#[tokio::test]
async fn frames_decode_stream_ends_and_cuts_are_told_apart() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;

    // A frame of exactly the largest size passes the length check (and then fails to decode as garbage).
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Control).await;
    let mut big = vec![0xEEu8; Control::MAX_LEN];
    big[0] = 200;
    send.write_all(&frame(&big)).await.unwrap();
    let mut buf = Vec::new();
    let err = read_frame(&mut recv, &mut buf).await.unwrap_err();
    assert!(err.to_string().contains("unknown message tag 200"), "{err}");

    // A good frame, then a clean end of the stream: Ok(None), not an error.
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Control).await;
    let mut ok = Vec::new();
    Control::Leave.encode_frame(&mut ok);
    send.write_all(&ok).await.unwrap();
    send.finish().unwrap();
    assert_eq!(read_frame(&mut recv, &mut buf).await.unwrap(), Some(Control::Leave));
    assert_eq!(read_frame(&mut recv, &mut buf).await.unwrap(), None);

    // The stream ends inside the two-byte length: an error.
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Control).await;
    send.write_all(&[7]).await.unwrap();
    send.finish().unwrap();
    assert!(read_frame(&mut recv, &mut buf).await.is_err());

    // The stream ends inside the body: an error, not a short message.
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Control).await;
    let whole = frame(&[5, 30, 0, 0, 0, 1]);
    send.write_all(&whole[..whole.len() - 2]).await.unwrap();
    send.finish().unwrap();
    assert!(read_frame(&mut recv, &mut buf).await.is_err());

    // A hostile version is told apart from garbage.
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Control).await;
    send.write_all(&frame(&[1, 9, 9, 0, 0])).await.unwrap();
    let err = read_frame(&mut recv, &mut buf).await.unwrap_err();
    assert!(err.to_string().contains("protocol version"), "{err}");
}

#[tokio::test]
async fn an_unknown_stream_kind_is_refused_and_every_known_one_is_read() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    for kind in [
        StreamKind::Control,
        StreamKind::Pairing,
        StreamKind::Clipboard,
        StreamKind::File,
        StreamKind::Probe,
        StreamKind::WallpaperVideo,
    ] {
        let (send, _) = open_stream(&pa.conn, kind).await.unwrap();
        let (got, _, mut recv) = pb.accept_stream().await.unwrap();
        assert_eq!(got, kind);
        drop((send, &mut recv));
    }
    for bad in [0u8, 7, 99, 255] {
        let (mut send, _recv) = pa.conn.open_bi().await.unwrap();
        send.write_all(&[bad]).await.unwrap();
        let err = pb.accept_stream().await.expect_err("an unknown kind is an error");
        assert!(err.to_string().contains("unknown stream kind"), "{err}");
    }
    // A stream that never says what it carries does not decide anything until it does.
    let (mut send, _recv) = pa.conn.open_bi().await.unwrap();
    let late = tokio::spawn({
        let conn = pb.conn.clone();
        async move {
            let (_s, mut r) = conn.accept_bi().await.unwrap();
            read_stream_kind(&mut r).await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!late.is_finished());
    send.write_all(&[StreamKind::Probe as u8]).await.unwrap();
    assert_eq!(late.await.unwrap().unwrap(), StreamKind::Probe);
}

#[tokio::test]
async fn file_messages_and_clipboard_headers_cross_the_wire_and_obey_their_limits() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::File).await;
    let messages = [
        FileMsg::Offer {
            path: "dir/é.txt".into(),
            size: 1 << 33,
            mtime: 5,
            batch: Batch { id: 9, files: 2, bytes: 3 },
        },
        FileMsg::Accept { offset: 17 },
        FileMsg::End { hash: [3; 32] },
        FileMsg::Saved { name: "dir/é (1).txt".into() },
    ];
    for m in &messages {
        write_file_msg(&mut send, m).await.unwrap();
    }
    for m in &messages {
        assert_eq!(read_file_msg(&mut recv).await.unwrap().as_ref(), Some(m));
    }
    send.finish().unwrap();
    assert_eq!(read_file_msg(&mut recv).await.unwrap(), None);

    // A file frame over FileMsg::MAX_LEN is refused.
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::File).await;
    send.write_all(&((FileMsg::MAX_LEN + 1) as u16).to_le_bytes()).await.unwrap();
    let err = tokio::time::timeout(Duration::from_secs(2), read_file_msg(&mut recv)).await.unwrap().unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");

    // Clipboard headers: good, over 1 KiB, and a stream that ends before one.
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Clipboard).await;
    let header =
        ClipHeader { items: vec![(ClipFormat::Text, 10), (ClipFormat::Png, 9_000_000)], pull: false, concealed: true };
    let mut bytes = Vec::new();
    header.encode_frame(&mut bytes);
    send.write_all(&bytes).await.unwrap();
    assert_eq!(read_clip_header(&mut recv).await.unwrap(), header);
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Clipboard).await;
    send.write_all(&1025u16.to_le_bytes()).await.unwrap();
    assert!(read_clip_header(&mut recv).await.unwrap_err().to_string().contains("too large"));
    let (mut send, mut recv) = stream_pair(&pa, &pb, StreamKind::Clipboard).await;
    send.finish().unwrap();
    assert!(read_clip_header(&mut recv).await.unwrap_err().to_string().contains("ended early"));
}

#[tokio::test]
async fn the_control_writer_keeps_order_under_a_burst_and_reports_a_dead_connection() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let (send, _) = pa.open_stream(StreamKind::Control).await.unwrap();
    let writer = ControlWriter::spawn(send);
    // 20 000 messages queued from several tasks at once, each task's own order preserved.
    let tasks: Vec<_> = (0..4u32)
        .map(|t| {
            let writer = writer.clone();
            tokio::spawn(async move {
                for i in 0..5000u32 {
                    assert!(writer.send(Control::Key { code: t * 10_000 + i, down: i % 2 == 0 }));
                    if i % 500 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
            })
        })
        .collect();
    let (kind, _, mut recv) = pb.accept_stream().await.unwrap();
    assert_eq!(kind, StreamKind::Control);
    let mut next = [0u32; 4];
    let mut buf = Vec::new();
    for _ in 0..20_000 {
        let Some(Control::Key { code, down }) = read_frame(&mut recv, &mut buf).await.unwrap() else { panic!("a key") };
        let (t, i) = (code / 10_000, code % 10_000);
        assert_eq!(i, next[t as usize], "task {t} arrived out of order");
        assert_eq!(down, i % 2 == 0);
        next[t as usize] += 1;
    }
    for t in tasks {
        t.await.unwrap();
    }
    // The peer goes away: sending reports it soon, and never panics or blocks.
    pb.conn.close(0u32.into(), b"gone");
    let start = Instant::now();
    while writer.send(Control::Leave) {
        assert!(start.elapsed() < Duration::from_secs(5), "a dead connection is reported");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ----------------------------------------------------------- connection rules

#[tokio::test]
async fn the_peer_may_not_open_one_way_streams_and_has_sixteen_two_way_ones() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let uni = tokio::time::timeout(Duration::from_millis(400), pa.conn.open_uni()).await;
    assert!(uni.is_err() || uni.unwrap().is_err(), "one-way streams are not allowed");

    let mut open = Vec::new();
    for _ in 0..16 {
        let (mut s, r) = open_stream(&pa.conn, StreamKind::Probe).await.unwrap();
        s.write_all(&[0]).await.ok();
        open.push((s, r));
    }
    let seventeenth = tokio::time::timeout(Duration::from_millis(400), pa.conn.open_bi()).await;
    assert!(seventeenth.is_err(), "the 17th concurrent stream waits");
    // Streams that are finished on both sides and dropped give their slots back (QUIC hands the
    // credit out in small batches, so a few must finish before the next one opens), which is how
    // clipboard and file streams keep coming for as long as the link lasts.
    let mut accepted = Vec::new();
    for _ in 0..16 {
        accepted.push(pb.accept_stream().await.unwrap());
    }
    for _ in 0..8 {
        let (mut s, mut r) = open.remove(0);
        s.finish().unwrap();
        let (_, mut their_send, mut their_recv) = accepted.remove(0);
        their_send.finish().unwrap();
        assert_eq!(their_recv.read_to_end(16).await.unwrap(), [0]);
        assert!(r.read_to_end(16).await.unwrap().is_empty());
        drop((s, r, their_send, their_recv));
    }
    let freed = tokio::time::timeout(Duration::from_secs(2), pa.conn.open_bi()).await;
    assert!(freed.is_ok(), "finished streams give their slots back");
}

#[tokio::test]
async fn pinned_connections_accept_only_the_expected_computer() {
    let (a, b, c) = (node(), node(), node());
    let accepting = b.clone();
    tokio::spawn(async move { while accepting.accept().await.is_some() {} });
    let addr = b.local_addr().unwrap();
    let ok = a.connect_pinned(addr, b.identity().fingerprint).await.unwrap();
    assert_eq!(ok.fingerprint, b.identity().fingerprint);
    assert!(ok.initiator);
    // Someone else answering at that address is refused before anything is sent to it.
    assert!(a.connect_pinned(addr, c.identity().fingerprint).await.is_err());
    // Pinning ourselves is refused even when it connects.
    let loopback = a.clone();
    tokio::spawn(async move { while loopback.accept().await.is_some() {} });
    let err = a.connect_pinned(a.local_addr().unwrap(), a.identity().fingerprint).await.err().expect("refused");
    assert!(err.to_string().contains("this computer"), "{err}");
}

#[tokio::test]
async fn a_closed_node_ends_accept_and_tells_its_peers_at_once() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let started = Instant::now();
    a.shutdown(Duration::from_secs(1)).await;
    let reason = tokio::time::timeout(Duration::from_secs(2), pb.conn.closed())
        .await
        .expect("the peer is told, not left to time out");
    assert!(started.elapsed() < Duration::from_secs(1), "{reason}");
    assert!(matches!(reason, one_mk_net::quinn::ConnectionError::ApplicationClosed(_)), "{reason:?}");
    let _ = pa;
    // The node that was closed yields no more connections.
    assert!(tokio::time::timeout(Duration::from_secs(1), a.accept()).await.expect("accept must end").is_none());
    // And cannot be dialled any more.
    assert!(node().connect_timeout(a.local_addr().unwrap(), Duration::from_millis(500)).await.is_err());
}

/// A relay that can be switched off, standing in for a Wi-Fi drop: everything passes while
/// `up`, nothing does while it is down.
struct Relay {
    up: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
}

async fn relay(to: std::net::SocketAddr) -> Relay {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = socket.local_addr().unwrap();
    let up = Arc::new(AtomicBool::new(true));
    let flag = up.clone();
    tokio::spawn(async move {
        let mut client = None;
        let mut buf = vec![0u8; 65536];
        loop {
            // Windows reports an earlier send to a closed port as an error on the next receive.
            let (n, from) = match socket.recv_from(&mut buf).await {
                Ok(got) => got,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            };
            if !flag.load(Ordering::SeqCst) {
                continue;
            }
            if from == to {
                if let Some(client) = client {
                    let _ = socket.send_to(&buf[..n], client).await;
                }
            } else {
                client = Some(from);
                let _ = socket.send_to(&buf[..n], to).await;
            }
        }
    });
    Relay { up, addr }
}

#[tokio::test]
async fn a_one_second_outage_is_survived_and_a_long_one_closes_the_connection_in_time() {
    let (a, b) = (node(), node());
    let accepting = b.clone();
    let accepted = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
    let wire = relay(b.local_addr().unwrap()).await;
    let pa = a.connect(wire.addr).await.unwrap();
    let pb = accepted.await.unwrap();

    // Both computers keep working across a short hiccup (keepalives every second, idle limit four).
    wire.up.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    wire.up.store(true, Ordering::SeqCst);
    let (mut send, _) = open_stream(&pa.conn, StreamKind::Control).await.unwrap();
    send.write_all(&frame(&[4])).await.unwrap();
    let (kind, _, mut recv) =
        tokio::time::timeout(Duration::from_secs(3), pb.accept_stream()).await.expect("still connected").unwrap();
    assert_eq!(kind, StreamKind::Control);
    let mut buf = Vec::new();
    assert_eq!(read_frame(&mut recv, &mut buf).await.unwrap(), Some(Control::Leave));
    assert!(pa.conn.close_reason().is_none() && pb.conn.close_reason().is_none());

    // A real loss of the other computer is noticed within about the idle timeout, on both ends,
    // so held keys are let go and the pointer comes home.
    wire.up.store(false, Ordering::SeqCst);
    let started = Instant::now();
    let (ra, rb) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(8), pa.conn.closed()),
        tokio::time::timeout(Duration::from_secs(8), pb.conn.closed())
    );
    let (ra, rb) = (ra.expect("the dialler notices"), rb.expect("the acceptor notices"));
    assert!(matches!(ra, one_mk_net::quinn::ConnectionError::TimedOut), "{ra:?}");
    assert!(matches!(rb, one_mk_net::quinn::ConnectionError::TimedOut), "{rb:?}");
    let took = started.elapsed();
    assert!(took > Duration::from_secs(2) && took < Duration::from_secs(7), "{took:?}");
    // Reconnecting is a fresh connection that works.
    wire.up.store(true, Ordering::SeqCst);
    let accepting = b.clone();
    let again = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
    let pa2 = a.connect_timeout(wire.addr, Duration::from_secs(3)).await.unwrap();
    let pb2 = again.await.unwrap();
    assert_eq!((pa2.fingerprint, pb2.fingerprint), (b.identity().fingerprint, a.identity().fingerprint));
}

/// Four handshakes that never finish from one address use up that address's share, so a fifth
/// is refused, but only until the stuck ones time out; other addresses and the node as a
/// whole are unaffected.
#[tokio::test]
async fn stalled_handshakes_from_one_address_are_limited_and_the_limit_lifts() {
    let server = node();
    let server_addr = server.local_addr().unwrap();
    let accepting = server.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(Ok(peer)) = accepting.accept().await {
            let _ = tx.send(peer);
        }
    });
    // A one-way relay (the server's answers are dropped) makes handshakes hang.
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let hang = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((n, from)) => {
                    if from != server_addr {
                        let _ = socket.send_to(&buf[..n], server_addr).await;
                    }
                }
                // Windows reports an earlier send to a closed port (a stalled client that has gone) as an
                // error on the next receive: that is not the end of the relay.
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            }
        }
    });
    let stallers: Vec<_> = (0..6)
        .map(|_| {
            let n = node();
            tokio::spawn(async move {
                let _ = n.connect(hang).await;
            })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(600)).await;
    for staller in &stallers {
        staller.abort();
    }
    // Mid-flood: a legitimate computer on the same address is turned away (or has to wait).
    let good = node();
    let early = good.connect_timeout(server_addr, Duration::from_millis(800)).await;
    assert!(early.is_err(), "the address is out of handshake slots");
    // The stuck handshakes time out after three seconds and the share comes back.
    tokio::time::sleep(Duration::from_millis(3000)).await;
    let late = good.connect_timeout(server_addr, Duration::from_secs(3)).await.expect("the limit lifted");
    assert_eq!(late.fingerprint, server.identity().fingerprint);
    let accepted = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    assert_eq!(accepted.fingerprint, good.identity().fingerprint);
    assert!(
        server.failed_handshakes().iter().any(|(ip, n, _)| ip.is_loopback() && *n >= 4),
        "the failures are counted by address: {:?}",
        server.failed_handshakes()
    );
}

/// A client that presents no certificate, or speaks another protocol name, never becomes a peer.
#[tokio::test]
async fn clients_without_a_certificate_or_with_another_protocol_name_are_refused() {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    #[derive(Debug)]
    struct AcceptAll(Arc<rustls::crypto::CryptoProvider>);
    impl ServerCertVerifier for AcceptAll {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = |alpn: &[u8], id: Option<&Identity>| {
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAll(provider.clone())));
        let mut config = match id {
            Some(id) => builder.with_client_auth_cert(vec![id.cert.clone()], id.key()).unwrap(),
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = vec![alpn.to_vec()];
        one_mk_net::quinn::ClientConfig::new(Arc::new(
            one_mk_net::quinn::crypto::rustls::QuicClientConfig::try_from(config).unwrap(),
        ))
    };

    let server = node();
    let accepting = server.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(Ok(peer)) = accepting.accept().await {
            let _ = tx.send(peer);
        }
    });
    let addr = server.local_addr().unwrap();
    let id = Identity::ephemeral().unwrap();
    let cases: [(&str, Option<&Identity>, &[u8]); 3] = [
        ("no certificate", None, one_mk_proto::ALPN),
        ("another protocol name", Some(&id), b"not-one-mk"),
        ("an older protocol name", Some(&id), b"one-mk/0"),
    ];
    for (what, identity, alpn) in cases {
        let client = one_mk_net::quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(4), async {
            let connection = client.connect_with(config(alpn, identity), addr, "one-mk").unwrap().await?;
            // TLS 1.3: the client may finish first; the server's refusal arrives right after.
            Err::<(), _>(connection.closed().await)
        })
        .await
        .unwrap_or_else(|_| panic!("{what}: the server left the client hanging"));
        assert!(outcome.is_err(), "{what} was let in");
    }
    assert!(rx.try_recv().is_err(), "none of them became a peer");
    // A correct client still gets in afterwards.
    let good = node();
    good.connect(addr).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().is_some());
}

// ------------------------------------------------------------------- pairing

/// A hostile responder: speaks the pairing steps by hand so each can go wrong.
async fn hostile_responder(peer: &Peer, name_len_field: Option<u16>, then: impl AsyncFnOnce(SendStream, RecvStream)) {
    let (kind, mut send, mut recv) = peer.accept_stream().await.unwrap();
    assert_eq!(kind, StreamKind::Pairing);
    // The initiator's name.
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await.unwrap();
    let mut name = vec![0u8; u16::from_be_bytes(len) as usize];
    recv.read_exact(&mut name).await.unwrap();
    // Ours (length field possibly a lie).
    let shown = b"hostile";
    let field = name_len_field.unwrap_or(shown.len() as u16);
    send.write_all(&field.to_be_bytes()).await.unwrap();
    if name_len_field.is_none() {
        send.write_all(shown).await.unwrap();
        // Commitment from the initiator, our nonce, their reveal.
        let mut commitment = [0u8; 32];
        recv.read_exact(&mut commitment).await.unwrap();
        send.write_all(&[9u8; 32]).await.unwrap();
        let mut reveal = [0u8; 32];
        recv.read_exact(&mut reveal).await.unwrap();
    }
    then(send, recv).await;
}

#[tokio::test]
async fn a_peer_that_announces_an_oversized_name_is_refused() {
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let me = a.identity().fingerprint;
    let (start, _) = tokio::join!(
        Pairing::start(&pa, me, "me"),
        hostile_responder(&pb, Some(60_000), async |_send, _recv| {
            tokio::time::sleep(Duration::from_millis(300)).await;
        })
    );
    let err = start.err().expect("an oversized name must fail the pairing");
    assert!(format!("{err:#}").contains("too long"), "{err:#}");
}

#[tokio::test]
async fn a_wrong_pairing_answer_is_never_taken_as_a_yes() {
    // The decision byte and the optional service port come from the other computer: only an
    // exact "1" from both users confirms, and a malformed port fails the whole exchange.
    for (trailer, answer, outcome) in [
        (&[1u8][..], true, Some((true, None))),
        (&[0u8][..], true, Some((false, None))),
        (&[1u8][..], false, Some((false, None))),
        (&[2u8][..], true, Some((false, None))),
        (&[1u8, 0x64, 0x5E][..], true, Some((true, Some(0x5E64)))),
        (&[1u8, 0, 0][..], true, None),
        (&[1u8, 7][..], true, None),
        (&[1u8, 1, 2, 3][..], true, None),
    ] {
        let (a, b) = (node(), node());
        let (pa, pb) = connected(&a, &b).await;
        let me = a.identity().fingerprint;
        let initiator = async {
            let pairing = Pairing::start(&pa, me, "me").await.unwrap();
            pairing.confirm_port(answer, Some(24900)).await
        };
        let (result, ()) = tokio::join!(
            initiator,
            hostile_responder(&pb, None, async |mut send, recv| {
                send.write_all(trailer).await.unwrap();
                send.finish().unwrap();
                // Keep both streams open while the initiator answers.
                tokio::time::sleep(Duration::from_millis(600)).await;
                drop(recv);
            })
        );
        match outcome {
            Some(expected) => assert_eq!(result.unwrap(), expected, "{trailer:?} / {answer}"),
            None => {
                let err = result.err().unwrap_or_else(|| panic!("{trailer:?} was accepted"));
                assert!(format!("{err:#}").contains("service port"), "{err:#}");
            }
        }
    }
}

#[tokio::test]
async fn a_responder_that_reveals_a_mismatching_nonce_is_caught_by_the_commitment() {
    // The responder side verifies the initiator's commitment against its reveal: an initiator
    // that opens with one nonce and reveals another is refused.
    let (a, b) = (node(), node());
    let (pa, pb) = connected(&a, &b).await;
    let me = b.identity().fingerprint;
    let honest = Pairing::start(&pb, me, "responder");
    let cheat = async {
        let (mut send, mut recv) = pa.open_stream(StreamKind::Pairing).await.unwrap();
        let name = b"initiator";
        send.write_all(&(name.len() as u16).to_be_bytes()).await.unwrap();
        send.write_all(name).await.unwrap();
        let mut len = [0u8; 2];
        recv.read_exact(&mut len).await.unwrap();
        let mut theirs = vec![0u8; u16::from_be_bytes(len) as usize];
        recv.read_exact(&mut theirs).await.unwrap();
        send.write_all(&[1u8; 32]).await.unwrap(); // a commitment to something
        let mut nonce = [0u8; 32];
        recv.read_exact(&mut nonce).await.unwrap();
        send.write_all(&[2u8; 32]).await.unwrap(); // a reveal that does not match it
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    let (result, ()) = tokio::join!(honest, cheat);
    let err = result.err().expect("the commitment check fails");
    assert!(format!("{err:#}").contains("commitment mismatch"), "{err:#}");
}

#[tokio::test]
async fn pairing_codes_differ_between_sessions_and_always_have_six_digits() {
    let (a, b) = (node(), node());
    let mut seen = std::collections::HashSet::new();
    for _ in 0..12 {
        let (pa, pb) = connected(&a, &b).await;
        let (sa, sb) = tokio::join!(
            Pairing::start(&pa, a.identity().fingerprint, "a"),
            Pairing::start(&pb, b.identity().fingerprint, "b")
        );
        let (sa, sb) = (sa.unwrap(), sb.unwrap());
        assert_eq!(sa.sas, sb.sas);
        let shown = sa.sas.to_string();
        assert_eq!(shown.len(), 7, "{shown}");
        assert_eq!(shown.as_bytes()[3], b' ');
        assert!(shown.chars().filter(|c| c.is_ascii_digit()).count() == 6);
        seen.insert(sa.sas.0);
        pa.conn.close(0u32.into(), b"done");
    }
    assert!(seen.len() >= 10, "fresh nonces every time: {} distinct codes in 12", seen.len());
}
