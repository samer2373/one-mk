use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use one_mk_net::{Device, Fingerprint, Identity, Node, Peer, TrustStore};
use one_mk_net::invite::{Invitation, InvitationState, InviteSession};
use std::sync::Arc;
use std::time::Duration;

fn node() -> Node {
    Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap()
}
fn link(a: &Node) -> Invitation {
    Invitation::new(a.identity().fingerprint, vec![a.local_addr().unwrap()], 24900, Duration::from_secs(60)).unwrap()
}
async fn peers(a: &Node, b: &Node) -> (Peer, Peer) {
    let (pb, pa) = tokio::join!(b.connect(a.local_addr().unwrap()), a.accept());
    (pa.unwrap().unwrap(), pb.unwrap())
}
async fn exchange(a: &Node, b: &Node, link: &Invitation, state: &InvitationState, claimed_b: Fingerprint) -> (bool, bool) {
    let (pa, pb) = peers(a, b).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(async {
            match InviteSession::accept(&pa, a.identity().fingerprint, "A", 24900, state).await {
                Ok(s) => s.confirm(true).await.unwrap_or(false),
                Err(_) => { pa.conn.close(1u32.into(), b"rejected"); false }
            }
        }, async {
            match InviteSession::join(&pb, claimed_b, "B", 24901, link).await {
                Ok(s) => s.confirm(true).await.unwrap_or(false),
                Err(_) => { pb.conn.close(1u32.into(), b"rejected"); false }
            }
        })
    }).await.expect("exchange must not stall")
}

#[tokio::test]
async fn invitation_pairs_without_human_input_and_persists_both_devices() {
    let (a, b) = (node(), node());
    let invitation = Invitation::parse(&link(&a).link()).unwrap();
    let state = InvitationState::new(&invitation);
    let (pa, pb) = peers(&a, &b).await;
    let (sa, sb) = tokio::join!(
        InviteSession::accept(&pa, a.identity().fingerprint, "A", 24900, &state),
        InviteSession::join(&pb, b.identity().fingerprint, "B", 24901, &invitation)
    );
    let (sa, sb) = (sa.unwrap(), sb.unwrap());
    assert_eq!((sa.peer_name.as_str(), sb.peer_name.as_str()), ("B", "A"));
    assert_eq!((sa.service_port, sb.service_port), (24901, 24900));
    let (ca, cb) = tokio::join!(sa.confirm(true), sb.confirm(true));
    assert!(ca.unwrap() && cb.unwrap());
    assert!(!state.active());
    for (node, peer, name) in [(&a, &pa, "B"), (&b, &pb, "A")] {
        let dir = std::env::temp_dir().join(format!("one-mk-invite-test-{}-{}", std::process::id(), node.identity().fingerprint));
        std::fs::create_dir_all(&dir).unwrap();
        let mut trust = TrustStore::load(&dir).unwrap();
        trust.upsert(Device { fingerprint: peer.fingerprint.to_string(), name: name.into(), ..Device::default() });
        trust.save().unwrap();
        assert!(TrustStore::load(&dir).unwrap().is_trusted(&peer.fingerprint));
        std::fs::remove_dir_all(dir).unwrap();
    }
    a.close(); b.close();
}

#[tokio::test]
async fn wrong_secret_then_right_secret_and_reuse() {
    let (a, b) = (node(), node());
    let invitation = link(&a);
    let state = InvitationState::new(&invitation);
    let text = invitation.link();
    let (prefix, data) = text.split_once("&d=").unwrap();
    let mut bytes = URL_SAFE_NO_PAD.decode(data).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    let wrong = Invitation::parse(&format!("{prefix}&d={}", URL_SAFE_NO_PAD.encode(bytes))).unwrap();
    assert_eq!(exchange(&a, &b, &wrong, &state, b.identity().fingerprint).await, (false, false));
    assert!(state.active());
    assert_eq!(exchange(&a, &b, &invitation, &state, b.identity().fingerprint).await, (true, true));
    assert_eq!(exchange(&a, &b, &invitation, &state, b.identity().fingerprint).await, (false, false));
    a.close(); b.close();
}

#[tokio::test]
async fn expired_invitation_is_rejected() {
    let (a, b) = (node(), node());
    let mut invitation = link(&a);
    invitation.expires = 0;
    let state = InvitationState::new(&invitation);
    assert_eq!(exchange(&a, &b, &invitation, &state, b.identity().fingerprint).await, (false, false));
    a.close(); b.close();
}

#[tokio::test]
async fn impersonated_server_certificate_is_rejected_before_opening_stream() {
    let (real_a, fake_a, b) = (node(), node(), node());
    let invitation = link(&real_a);
    let (pa, pb) = peers(&fake_a, &b).await;
    let err = InviteSession::join(&pb, b.identity().fingerprint, "B", 24900, &invitation).await.err().unwrap();
    assert!(err.to_string().contains("certificate mismatch"));
    assert!(tokio::time::timeout(Duration::from_millis(50), pa.accept_stream()).await.is_err());
    real_a.close(); fake_a.close(); b.close();
}

#[tokio::test]
async fn proof_bound_to_a_different_client_certificate_is_rejected() {
    let (a, b, other) = (node(), node(), node());
    let invitation = link(&a);
    let state = InvitationState::new(&invitation);
    assert_eq!(exchange(&a, &b, &invitation, &state, other.identity().fingerprint).await, (false, false));
    assert!(state.active());
    a.close(); b.close(); other.close();
}

#[tokio::test]
async fn refusal_burns_invitation_without_confirming_either_side() {
    let (a, b) = (node(), node());
    let invitation = link(&a);
    let state = InvitationState::new(&invitation);
    let (pa, pb) = peers(&a, &b).await;
    let (sa, sb) = tokio::join!(
        InviteSession::accept(&pa, a.identity().fingerprint, "A", 24900, &state),
        InviteSession::join(&pb, b.identity().fingerprint, "B", 24901, &invitation)
    );
    let (ca, cb) = tokio::join!(sa.unwrap().confirm(false), sb.unwrap().confirm(true));
    assert!(!ca.unwrap() && !cb.unwrap());
    assert!(!state.active());
    a.close(); b.close();
}

#[tokio::test]
async fn fifth_wrong_proof_burns_even_for_the_rightful_joiner() {
    let (a, b) = (node(), node());
    let invitation = link(&a);
    let state = InvitationState::new(&invitation);
    let text = invitation.link();
    let (prefix, data) = text.split_once("&d=").unwrap();
    let mut bytes = URL_SAFE_NO_PAD.decode(data).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    let wrong = Invitation::parse(&format!("{prefix}&d={}", URL_SAFE_NO_PAD.encode(bytes))).unwrap();
    for _ in 0..5 { assert_eq!(exchange(&a, &b, &wrong, &state, b.identity().fingerprint).await, (false, false)); }
    assert!(!state.active());
    assert_eq!(exchange(&a, &b, &invitation, &state, b.identity().fingerprint).await, (false, false));
    a.close(); b.close();
}
