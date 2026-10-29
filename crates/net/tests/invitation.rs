use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use one_mk_net::{Fingerprint, Identity, Node, Peer};
use one_mk_net::invite::{Invitation, InvitationState, InviteSession};
use std::sync::Arc;
use std::time::Duration;

fn node() -> Node {
    Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap()
}
fn link(a: &Node) -> Invitation {
    Invitation::new(a.identity().fingerprint, vec!["192.168.1.2:1234".parse().unwrap()], 24900, Duration::from_secs(60)).unwrap()
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
                Ok(mut s) => s.confirm(true).await.unwrap_or(false),
                Err(_) => { pa.conn.close(1u32.into(), b"rejected"); false }
            }
        }, async {
            match InviteSession::join(&pb, claimed_b, "B", 24901, link).await {
                Ok(mut s) => s.confirm(true).await.unwrap_or(false),
                Err(_) => { pb.conn.close(1u32.into(), b"rejected"); false }
            }
        })
    }).await.expect("exchange must not stall")
}

#[tokio::test]
async fn invitation_pairs_without_human_input() {
    let (a, b) = (node(), node());
    let invitation = Invitation::parse(&link(&a).link()).unwrap();
    let state = InvitationState::new(&invitation);
    let (pa, pb) = peers(&a, &b).await;
    let (sa, sb) = tokio::join!(
        InviteSession::accept(&pa, a.identity().fingerprint, "A", 24900, &state),
        InviteSession::join(&pb, b.identity().fingerprint, "B", 24901, &invitation)
    );
    let (mut sa, mut sb) = (sa.unwrap(), sb.unwrap());
    assert_eq!(sa.code, sb.code);
    assert_eq!((sa.peer_name.as_str(), sb.peer_name.as_str()), ("B", "A"));
    assert_eq!((sa.service_port, sb.service_port), (24901, 24900));
    let (ca, cb) = tokio::join!(sa.confirm(true), sb.confirm(true));
    assert!(ca.unwrap() && cb.unwrap());
    assert!(!state.active());
    let (committed_a, committed_b) = tokio::join!(sa.committed(), sb.committed());
    committed_a.unwrap(); committed_b.unwrap();
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
    let (mut sa, mut sb) = (sa.unwrap(), sb.unwrap());
    let (ca, cb) = tokio::join!(sa.confirm(false), sb.confirm(true));
    assert!(!ca.unwrap() && !cb.unwrap());
    assert!(!state.active());
    a.close(); b.close();
}

#[tokio::test]
async fn wrong_secret_cannot_name_or_burn_the_invitation() {
    let (a, b) = (node(), node());
    let invitation = link(&a);
    let state = InvitationState::new(&invitation);
    let text = invitation.link();
    let (prefix, data) = text.split_once("&d=").unwrap();
    let mut bytes = URL_SAFE_NO_PAD.decode(data).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    let wrong = Invitation::parse(&format!("{prefix}&d={}", URL_SAFE_NO_PAD.encode(bytes))).unwrap();
    for _ in 0..5 { assert_eq!(exchange(&a, &b, &wrong, &state, b.identity().fingerprint).await, (false, false)); }
    assert!(state.active());
    assert_eq!(exchange(&a, &b, &invitation, &state, b.identity().fingerprint).await, (true, true));
    a.close(); b.close();
}

#[tokio::test]
async fn copied_link_sessions_agree_on_codes_and_honor_refusal() {
    let (host, real, impostor) = (node(), node(), node());
    let invitation = link(&host);
    let mut codes = Vec::new();
    for joiner in [&impostor, &real] {
        let state = InvitationState::new(&invitation);
        let (incoming, outgoing) = peers(&host, joiner).await;
        let (host_session, join_session) = tokio::join!(
            InviteSession::accept(&incoming, host.identity().fingerprint, "Host", 24900, &state),
            InviteSession::join(&outgoing, joiner.identity().fingerprint, "B", 24901, &invitation)
        );
        let (mut host_session, mut join_session) = (host_session.unwrap(), join_session.unwrap());
        assert_eq!(host_session.code, join_session.code);
        codes.push(host_session.code.clone());
        let (host_decision, join_decision) = tokio::join!(host_session.confirm(false), join_session.confirm(true));
        assert!(!host_decision.unwrap() && !join_decision.unwrap());
        incoming.conn.close(0u32.into(), b"test complete");
    }
    assert_eq!(codes.len(), 2);
    host.close(); real.close(); impostor.close();
}

#[tokio::test]
async fn pinned_handshake_rejects_impostor_before_client_authentication() {
    let (host, joiner) = (node(), node());
    let expected = Identity::ephemeral().unwrap().fingerprint;
    let (result, accepted) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(2), joiner.connect_pinned(host.local_addr().unwrap(), expected)),
        tokio::time::timeout(Duration::from_millis(300), host.accept())
    );
    assert!(result.unwrap().is_err());
    assert!(accepted.is_err(), "server must not receive an authenticated client certificate");
    host.close(); joiner.close();
}

#[tokio::test]
async fn lost_commit_ack_reports_uncertainty_instead_of_success() {
    let (host, joiner) = (node(), node());
    let invitation = link(&host);
    let state = InvitationState::new(&invitation);
    let (incoming, outgoing) = peers(&host, &joiner).await;
    let (host_session, join_session) = tokio::join!(
        InviteSession::accept(&incoming, host.identity().fingerprint, "A", 24900, &state),
        InviteSession::join(&outgoing, joiner.identity().fingerprint, "B", 24901, &invitation)
    );
    let (mut host_session, mut join_session) = (host_session.unwrap(), join_session.unwrap());
    let (host_yes, join_yes) = tokio::join!(host_session.confirm(true), join_session.confirm(true));
    assert!(host_yes.unwrap() && join_yes.unwrap());
    drop(join_session);
    outgoing.conn.close(0u32.into(), b"simulated crash before saving");
    assert!(tokio::time::timeout(Duration::from_secs(2), host_session.committed()).await.unwrap().is_err());
    host.close(); joiner.close();
}
