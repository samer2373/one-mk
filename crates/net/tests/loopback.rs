use one_mk_net::{ControlWriter, Identity, Node, Pairing, read_frame};
use one_mk_proto::{Control, Datagram, Pos, StreamKind};
use std::sync::Arc;

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
