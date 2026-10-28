use one_mk_net::{ControlWriter, Identity, Node, Pairing, read_frame};
use one_mk_proto::{Control, Datagram, Pos, StreamKind};
use std::sync::Arc;
use std::time::Duration;

fn node() -> Node {
    Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap()
}

#[tokio::test]
async fn pair_then_exchange_messages() {
    let (a, b) = (node(), node());
    let b_addr = b.local_addr().unwrap();

    let (pa, pb) = tokio::join!(a.connect(b_addr), async { b.accept().await.unwrap() });
    let (pa, pb) = (pa.unwrap(), pb.unwrap());
    assert_eq!(pa.fingerprint, b.identity().fingerprint);
    assert_eq!(pb.fingerprint, a.identity().fingerprint);

    // Both sides derive the same code.
    let (sa, sb) = tokio::join!(
        Pairing::start(&pa, a.identity().fingerprint, "a"),
        Pairing::start(&pb, b.identity().fingerprint, "b")
    );
    let (sa, sb) = (sa.unwrap(), sb.unwrap());
    assert_eq!(sa.sas, sb.sas);
    assert_eq!((sa.peer_name.as_str(), sb.peer_name.as_str()), ("b", "a"));
    let (ca, cb) = tokio::join!(sa.confirm(true), sb.confirm(true));
    assert!(ca.unwrap() && cb.unwrap());

    // Control stream.
    let (send, _) = pa.open_stream(StreamKind::Control).await.unwrap();
    let w = ControlWriter::spawn(send);
    w.send(Control::Key { code: 30, down: true });
    w.send(Control::Leave);
    let (kind, _, mut recv) = pb.accept_stream().await.unwrap();
    assert_eq!(kind, StreamKind::Control);
    let mut buf = Vec::new();
    assert_eq!(read_frame(&mut recv, &mut buf).await.unwrap(), Some(Control::Key { code: 30, down: true }));
    assert_eq!(read_frame(&mut recv, &mut buf).await.unwrap(), Some(Control::Leave));

    // Datagrams.
    let d = Datagram::Motion { epoch: 1, seq: 1, pos: Pos::new(10.0, 20.0) };
    let (bytes, n) = d.encode();
    pa.conn.send_datagram(bytes[..n].to_vec().into()).unwrap();
    let got = pb.conn.read_datagram().await.unwrap();
    assert_eq!(Datagram::decode(&got).unwrap(), d);
}

#[tokio::test]
async fn rejected_pairing_is_not_confirmed() {
    let (a, b) = (node(), node());
    let b_addr = b.local_addr().unwrap();
    let (pa, pb) = tokio::join!(a.connect(b_addr), async { b.accept().await.unwrap() });
    let (pa, pb) = (pa.unwrap(), pb.unwrap());
    let (sa, sb) = tokio::join!(
        Pairing::start(&pa, a.identity().fingerprint, "a"),
        Pairing::start(&pb, b.identity().fingerprint, "b")
    );
    let (ca, cb) = tokio::join!(sa.unwrap().confirm(true), sb.unwrap().confirm(false));
    assert!(!ca.unwrap());
    assert!(!cb.unwrap());
}

/// A client whose handshake never completes (its packets reach the node,
/// the node's replies are dropped) must not hold up other connections.
#[tokio::test]
async fn stalled_handshake_does_not_block_accept() {
    let server = node();
    let server_addr = server.local_addr().unwrap();

    // One-way relay: client -> server only.
    let relay = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = relay.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        while let Ok((n, from)) = relay.recv_from(&mut buf).await {
            if from != server_addr {
                let _ = relay.send_to(&buf[..n], server_addr).await;
            }
        }
    });
    let staller = node();
    tokio::spawn(async move {
        let _ = staller.connect(relay_addr).await;
    });

    let accepting = {
        let server = server.clone();
        tokio::spawn(async move { server.accept().await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    let good = node();
    let (dialed, accepted) = tokio::time::timeout(Duration::from_secs(2), async {
        let dialed = good.connect(server_addr).await;
        (dialed, accepting.await.unwrap())
    })
    .await
    .expect("a stalled handshake blocked accept");
    dialed.unwrap();
    assert_eq!(accepted.unwrap().unwrap().fingerprint, good.identity().fingerprint);
}

#[tokio::test]
async fn connect_timeout_gives_up() {
    let a = node();
    // Nothing answers here.
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let err = a.connect_timeout(silent.local_addr().unwrap(), Duration::from_millis(200)).await.err().expect("must time out");
    assert!(err.to_string().contains("no answer"), "{err}");
}

#[tokio::test]
async fn bind_any_picks_a_port() {
    let n = Node::bind_any(Arc::new(Identity::ephemeral().unwrap()), 0).unwrap();
    assert_ne!(n.local_addr().unwrap().port(), 0);
}

#[tokio::test]
async fn connecting_to_self_is_refused() {
    let a = node();
    let acceptor = a.clone();
    tokio::spawn(async move { while acceptor.accept().await.is_some() {} });
    let err = a.connect(a.local_addr().unwrap()).await.err().expect("must fail");
    assert!(err.to_string().contains("itself"), "{err}");
}

/// Peer names can't carry terminal escapes into the pairing prompt, and
/// both sides still agree on the code (names are bound into it).
#[tokio::test]
async fn pairing_names_are_sanitized() {
    let (a, b) = (node(), node());
    let b_addr = b.local_addr().unwrap();
    let (pa, pb) = tokio::join!(a.connect(b_addr), async { b.accept().await.unwrap() });
    let (pa, pb) = (pa.unwrap(), pb.unwrap());
    let evil = "laptop\x1b[3A\r\x1b[2K      123 456\u{202E}";
    let (sa, sb) = tokio::join!(
        Pairing::start(&pa, a.identity().fingerprint, evil),
        Pairing::start(&pb, b.identity().fingerprint, "b")
    );
    let (sa, sb) = (sa.unwrap(), sb.unwrap());
    assert_eq!(sa.sas, sb.sas);
    assert!(!sb.peer_name.chars().any(char::is_control), "{:?}", sb.peer_name);
    assert_eq!(sb.peer_name, one_mk_proto::sanitize_name(evil));
}
