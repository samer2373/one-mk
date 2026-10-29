//! Out-of-band, single-use invitations. See docs/PAIRING-INVITE.md.
use crate::{Fingerprint, Peer};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use one_mk_proto::{StreamKind, sanitize_name};
use ring::hmac;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, Zeroizing};

const PREFIX: &str = "onemk://pair?v=1&d=";
// An old numeric-pairing peer reads 65535 as a name length and rejects it.
const MAGIC: &[u8; 7] = b"\xff\xffOMKI\x01";
const DOMAIN: &[u8] = b"one-mk invitation v1";
pub const MAX_LINK: usize = 1600;
pub const MAX_LIFETIME: Duration = Duration::from_secs(3600);
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_GUESSES: u8 = 5;

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Deliberately neither Debug nor Display: logging a link discloses a credential.
pub struct Invitation {
    pub fingerprint: Fingerprint,
    pub addrs: Vec<SocketAddr>,
    pub mdns: String,
    pub service_port: u16,
    pub expires: u64,
    secret: Zeroizing<[u8; 32]>,
}

impl Invitation {
    pub fn new(fingerprint: Fingerprint, addrs: Vec<SocketAddr>, service_port: u16, ttl: Duration) -> Result<Self> {
        ensure!(ttl.as_secs() >= 1 && ttl <= MAX_LIFETIME, "expiry must be 1 second to 1 hour");
        let link = Self {
            fingerprint, addrs, service_port,
            mdns: format!("one-mk-{}.local.", fingerprint.short()),
            expires: unix_now() + ttl.as_secs(),
            secret: Zeroizing::new(rand::random()),
        };
        link.validate()?;
        Ok(link)
    }

    fn validate(&self) -> Result<()> {
        ensure!((1..=8).contains(&self.addrs.len()), "invitation needs 1 to 8 addresses");
        ensure!(self.service_port != 0, "invalid service port");
        ensure!(self.mdns == format!("one-mk-{}.local.", self.fingerprint.short()), "invalid mDNS name");
        let port = self.addrs[0].port();
        ensure!(port != 0 && self.addrs.iter().all(|a| a.port() == port && usable_address(a.ip())), "invalid invitation address");
        Ok(())
    }

    fn public_bytes(&self) -> Vec<u8> {
        let mut out = vec![1];
        out.extend_from_slice(&self.fingerprint.0);
        out.extend_from_slice(&self.expires.to_be_bytes());
        out.extend_from_slice(&self.service_port.to_be_bytes());
        put(&mut out, self.mdns.as_bytes());
        out.push(self.addrs.len() as u8);
        for addr in &self.addrs { put(&mut out, addr.to_string().as_bytes()); }
        out
    }

    /// Call only when intentionally transferring the bearer credential.
    pub fn link(&self) -> Zeroizing<String> {
        let mut bytes = Zeroizing::new(self.public_bytes());
        bytes.extend_from_slice(self.secret.as_ref());
        let encoded = Zeroizing::new(URL_SAFE_NO_PAD.encode(&*bytes));
        Zeroizing::new(format!("{PREFIX}{}", encoded.as_str()))
    }

    pub fn parse(link: &str) -> Result<Self> {
        ensure!(link.len() <= MAX_LINK && link.is_ascii(), "invalid invitation length or encoding");
        let encoded = link.strip_prefix(PREFIX).context("unsupported invitation URL or version")?;
        let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded).context("invalid invitation encoding")?);
        let mut r = Reader(&bytes);
        ensure!(r.take(1)? == [1], "unsupported invitation version");
        let fingerprint = Fingerprint(r.array()?);
        let expires = u64::from_be_bytes(r.array()?);
        let service_port = u16::from_be_bytes(r.array()?);
        let mdns = r.text(64)?;
        let count = r.take(1)?[0] as usize;
        ensure!((1..=8).contains(&count), "invalid address count");
        let mut addrs = Vec::with_capacity(count);
        for _ in 0..count { addrs.push(r.text(64)?.parse().context("invalid invitation address")?); }
        let secret = Zeroizing::new(r.array()?);
        ensure!(r.0.is_empty(), "trailing invitation data");
        let out = Self { fingerprint, addrs, mdns, service_port, expires, secret };
        out.validate()?;
        Ok(out)
    }

    pub fn is_expired(&self) -> bool { unix_now() >= self.expires }
    fn id(&self) -> [u8; 32] {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.secret.as_ref());
        hmac::sign(&key, b"one-mk invite id v1").as_ref().try_into().expect("SHA256 tag")
    }
}

pub fn usable_address(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast()
        && !matches!(ip, IpAddr::V4(addr) if addr.is_broadcast())
        && !matches!(ip, IpAddr::V6(addr) if addr.is_unicast_link_local())
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure!(self.0.len() >= n, "truncated invitation");
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> { Ok(self.take(N)?.try_into()?) }
    fn text(&mut self, max: usize) -> Result<String> {
        let n = u16::from_be_bytes(self.array()?) as usize;
        ensure!(n <= max, "invitation field too long");
        Ok(std::str::from_utf8(self.take(n)?)?.to_owned())
    }
}

/// One mutex makes check-and-burn atomic across racing connections.
/// Uses a monotonic deadline as well as wall time: turning the clock back
/// cannot extend an invitation. No secret is persisted.
pub struct InvitationState {
    inner: Mutex<State>,
    pub deadline: Instant,
    expires: u64,
    id: [u8; 32],
}
struct State {
    secret: Option<Zeroizing<[u8; 32]>>,
    guesses: u8,
}
impl InvitationState {
    pub fn new(invitation: &Invitation) -> Self {
        let remaining = Duration::from_secs(invitation.expires.saturating_sub(unix_now())).min(MAX_LIFETIME);
        Self {
            inner: Mutex::new(State { secret: Some(Zeroizing::new(*invitation.secret)), guesses: 0 }),
            deadline: Instant::now() + remaining,
            expires: invitation.expires,
            id: invitation.id(),
        }
    }
    pub fn burn(&self) { self.inner.lock().expect("invitation mutex").secret = None; }
    pub fn expired(&self) -> bool { self.expired_at(Instant::now(), unix_now()) }
    fn expired_at(&self, monotonic: Instant, wall: u64) -> bool { monotonic >= self.deadline || wall >= self.expires }
    pub fn active(&self) -> bool {
        let mut s = self.inner.lock().expect("invitation mutex");
        if self.expired() { s.secret = None; }
        s.secret.is_some()
    }
    fn consume(&self, id: &[u8; 32], transcript: &[u8], proof: &[u8; 32]) -> Result<([u8; 32], String)> {
        ensure!(id == &self.id, "unknown invitation");
        let mut s = self.inner.lock().expect("invitation mutex");
        if self.expired() { s.secret = None; }
        let secret = s.secret.as_ref().context("invitation unavailable")?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_ref());
        // ring::hmac::verify uses constant-time tag comparison. Never compare
        // the secret or an expected tag with ==, nor send either secret.
        if hmac::verify(&key, &role(b"join", transcript), proof).is_err() {
            s.guesses += 1;
            if s.guesses >= MAX_GUESSES { s.secret = None; }
            anyhow::bail!("invitation proof rejected");
        }
        let answer = tag(&key, b"accept", transcript);
        let code = sas(&key, transcript);
        s.secret = None; // burn BEFORE replying, prompting, or writing trust
        Ok((answer, code))
    }
}

pub struct InviteSession {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    pub peer_name: String,
    pub service_port: u16,
    pub code: String,
}
impl InviteSession {
    /// Called only by A on its existing pairing Node. The caller holds an
    /// admission permit and bounds the entire exchange by STEP_TIMEOUT.
    pub async fn accept(peer: &Peer, me: Fingerprint, name: &str, service_port: u16, state: &InvitationState) -> Result<Self> {
        ensure!(!peer.initiator, "expected incoming connection");
        let (kind, mut send, mut recv) = peer.accept_stream().await?;
        ensure!(kind == StreamKind::Pairing, "expected pairing stream");
        let mut magic = [0; 7];
        recv.read_exact(&mut magic).await?;
        ensure!(&magic == MAGIC, "unsupported invitation protocol");
        let mut id = [0; 32];
        recv.read_exact(&mut id).await?;
        if !state.active() {
            send.write_all(&[0]).await?;
            send.finish()?;
            anyhow::bail!("invitation already used or expired");
        }
        if id != state.id {
            send.write_all(&[0]).await?;
            send.finish()?;
            anyhow::bail!("unknown invitation");
        }
        send.write_all(&[1]).await?;
        let (peer_raw, peer_port, nb) = read_hello(&mut recv).await?;
        let my_name = sanitize_name(name);
        let na = rand::random();
        send.write_all(&hello(my_name.as_bytes(), service_port, &na)).await?;
        let t = transcript(peer, &id, me, peer.fingerprint, my_name.as_bytes(), &peer_raw, service_port, peer_port, &na, &nb)?;
        let mut proof = [0; 32];
        recv.read_exact(&mut proof).await?;
        let (answer, code) = state.consume(&id, &t, &proof)?;
        send.write_all(&answer).await?;
        Ok(Self { send, recv, peer_name: sanitize_name(&String::from_utf8(peer_raw)?), service_port: peer_port, code })
    }

    /// Pin the TLS certificate BEFORE sending any invitation protocol data.
    pub async fn join(peer: &Peer, me: Fingerprint, name: &str, service_port: u16, invitation: &Invitation) -> Result<Self> {
        ensure!(peer.initiator && peer.fingerprint == invitation.fingerprint, "invitation server certificate mismatch");
        ensure!(!invitation.is_expired(), "invitation expired");
        let (mut send, mut recv) = peer.open_stream(StreamKind::Pairing).await?;
        let id = invitation.id();
        let my_name = sanitize_name(name);
        let nb = rand::random();
        let mut start = MAGIC.to_vec();
        start.extend_from_slice(&id);
        start.extend_from_slice(&hello(my_name.as_bytes(), service_port, &nb));
        send.write_all(&start).await?;
        let mut available = [0];
        recv.read_exact(&mut available).await?;
        ensure!(available == [1], "invitation already used or expired");
        let (peer_raw, peer_port, na) = read_hello(&mut recv).await?;
        ensure!(peer_port == invitation.service_port, "invitation service port mismatch");
        let t = transcript(peer, &id, peer.fingerprint, me, &peer_raw, my_name.as_bytes(), peer_port, service_port, &na, &nb)?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, invitation.secret.as_ref());
        send.write_all(&tag(&key, b"join", &t)).await?;
        let mut answer = [0; 32];
        recv.read_exact(&mut answer).await?;
        hmac::verify(&key, &role(b"accept", &t), &answer).map_err(|_| anyhow::anyhow!("invitation response rejected"))?;
        let code = sas(&key, &t);
        Ok(Self { send, recv, peer_name: sanitize_name(&String::from_utf8(peer_raw)?), service_port: peer_port, code })
    }

    /// Call only after local approval. The caller bounds this by the
    /// invitation deadline and its human-answer timeout.
    pub async fn confirm(&mut self, accept: bool) -> Result<bool> {
        self.send.write_all(&[u8::from(accept)]).await?;
        let mut answer = [0];
        self.recv.read_exact(&mut answer).await?;
        Ok(accept && answer == [1])
    }

    pub async fn committed(mut self) -> Result<()> {
        self.send.write_all(&[2]).await?;
        self.send.finish()?;
        let mut answer = [0];
        self.recv.read_exact(&mut answer).await?;
        ensure!(answer == [2], "peer did not acknowledge saving trust");
        let _ = tokio::time::timeout(Duration::from_secs(3), self.send.stopped()).await;
        Ok(())
    }
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn hello(name: &[u8], port: u16, nonce: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::new();
    put(&mut out, name);
    out.extend_from_slice(&port.to_be_bytes());
    out.extend_from_slice(nonce);
    out
}
async fn read_hello(recv: &mut quinn::RecvStream) -> Result<(Vec<u8>, u16, [u8; 32])> {
    let mut len = [0; 2];
    recv.read_exact(&mut len).await?;
    let n = u16::from_be_bytes(len) as usize;
    ensure!(n <= 1024, "invitation name too long");
    let mut name = vec![0; n];
    recv.read_exact(&mut name).await?;
    ensure!(std::str::from_utf8(&name).is_ok(), "invalid device name");
    recv.read_exact(&mut len).await?;
    let port = u16::from_be_bytes(len);
    ensure!(port != 0, "invalid service port");
    let mut nonce = [0; 32];
    recv.read_exact(&mut nonce).await?;
    Ok((name, port, nonce))
}
#[allow(clippy::too_many_arguments)]
fn transcript(peer: &Peer, id: &[u8; 32], a: Fingerprint, b: Fingerprint, an: &[u8], bn: &[u8], ap: u16, bp: u16, na: &[u8; 32], nb: &[u8; 32]) -> Result<Vec<u8>> {
    let mut binding = [0; 32];
    peer.conn.export_keying_material(&mut binding, DOMAIN, id).map_err(|_| anyhow::anyhow!("TLS exporter unavailable"))?;
    let mut t = Vec::new();
    for field in [DOMAIN, id, &a.0, &b.0, an, bn, &ap.to_be_bytes(), &bp.to_be_bytes(), na, nb, &binding] { put(&mut t, field); }
    binding.zeroize();
    Ok(t)
}
fn role(role: &[u8], transcript: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    put(&mut out, role);
    out.extend_from_slice(transcript);
    out
}
fn tag(key: &hmac::Key, role_name: &[u8], transcript: &[u8]) -> [u8; 32] {
    hmac::sign(key, &role(role_name, transcript)).as_ref().try_into().expect("SHA256 tag")
}

fn sas(key: &hmac::Key, transcript: &[u8]) -> String {
    let bytes = tag(key, b"sas", transcript);
    format!("{:06}", u32::from_be_bytes(bytes[..4].try_into().expect("four bytes")) % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn invitation_id_requires_the_secret() {
        let first = invitation();
        let mut copied_public = Invitation::parse(&first.link()).unwrap();
        copied_public.secret = Zeroizing::new([42; 32]);
        assert_ne!(first.id(), copied_public.id());
        assert_ne!(first.id(), <[u8; 32]>::from(Sha256::digest(first.public_bytes())));
    }

    #[test]
    fn reject_non_unicast_invitation_addresses() {
        for address in ["127.0.0.1:1234", "[::1]:1234", "255.255.255.255:1234", "224.0.0.1:1234", "0.0.0.0:1234", "[::ffff:127.0.0.1]:1234"] {
            assert!(Invitation::new(Fingerprint([7; 32]), vec![address.parse().unwrap()], 24900, Duration::from_secs(60)).is_err(), "{address}");
        }
    }
    fn invitation() -> Invitation {
        Invitation::new(Fingerprint([7; 32]), vec!["192.168.1.2:1234".parse().unwrap()], 24900, Duration::from_secs(60)).unwrap()
    }
    #[test]
    fn links_are_bounded_versioned_and_strict() {
        let a = invitation();
        let text = a.link();
        let b = Invitation::parse(&text).unwrap();
        assert_eq!(b.link(), text);
        for cut in 0..text.len() { assert!(Invitation::parse(&text[..cut]).is_err(), "accepted truncation {cut}"); }
        for bad in ["".to_owned(), text.replace("v=1", "v=2"), format!("{}&extra=1", text.as_str()), format!("{PREFIX}%%%"), "x".repeat(MAX_LINK + 1), "é".repeat(100)] {
            assert!(Invitation::parse(&bad).is_err());
        }
        let mut bytes = URL_SAFE_NO_PAD.decode(text.strip_prefix(PREFIX).unwrap()).unwrap();
        bytes[0] = 2;
        assert!(Invitation::parse(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(&bytes))).is_err());
        bytes[0] = 1;
        bytes.push(0);
        assert!(Invitation::parse(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(&bytes))).is_err());
        // Arbitrary input must never panic, including malformed length fields.
        for n in 0..700 {
            let bytes: Vec<u8> = (0..n).map(|_| rand::random()).collect();
            let _ = Invitation::parse(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)));
        }
    }
    #[test]
    fn expiry_single_use_and_guess_budget() {
        let mut a = invitation();
        let key = hmac::Key::new(hmac::HMAC_SHA256, a.secret.as_ref());
        let proof = tag(&key, b"join", b"transcript");
        let state = InvitationState::new(&a);
        assert!(state.expired_at(Instant::now(), a.expires));
        assert!(state.expired_at(state.deadline, 0));
        assert!(!state.expired_at(Instant::now(), 0));
        assert!(state.consume(&a.id(), b"transcript", &proof).is_ok());
        assert!(!state.active());
        assert!(state.consume(&a.id(), b"transcript", &proof).is_err());
        let state = InvitationState::new(&a);
        for i in 0..MAX_GUESSES {
            assert!(state.consume(&a.id(), b"wrong transcript", &proof).is_err());
            assert_eq!(state.active(), i + 1 < MAX_GUESSES);
        }
        assert!(state.consume(&a.id(), b"transcript", &proof).is_err());
        a.expires = unix_now();
        assert!(a.is_expired());
        let state = InvitationState::new(&a);
        assert!(!state.active());
        assert!(state.consume(&a.id(), b"transcript", &proof).is_err());
        a.expires += 60;
        let mut state = InvitationState::new(&a);
        state.deadline = Instant::now();
        assert!(state.consume(&a.id(), b"transcript", &proof).is_err());
    }
    #[tokio::test]
    async fn mac_roles_and_every_transcript_field_are_bound() {
        let host = crate::Node::bind(std::sync::Arc::new(crate::Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let joiner = crate::Node::bind(std::sync::Arc::new(crate::Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (outgoing, incoming) = tokio::join!(joiner.connect(host.local_addr().unwrap()), host.accept());
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap().unwrap();
        let invitation = invitation();
        let key = hmac::Key::new(hmac::HMAC_SHA256, invitation.secret.as_ref());
        let fields = vec![invitation.id().to_vec(), host.identity().fingerprint.0.to_vec(), joiner.identity().fingerprint.0.to_vec(), b"Host".to_vec(), b"Joiner".to_vec(), 24900u16.to_be_bytes().to_vec(), 24901u16.to_be_bytes().to_vec(), vec![1; 32], vec![2; 32]];
        let build = |peer: &Peer, fields: &[Vec<u8>]| transcript(peer, &fields[0].as_slice().try_into().unwrap(), Fingerprint(fields[1].as_slice().try_into().unwrap()), Fingerprint(fields[2].as_slice().try_into().unwrap()), &fields[3], &fields[4], u16::from_be_bytes(fields[5].as_slice().try_into().unwrap()), u16::from_be_bytes(fields[6].as_slice().try_into().unwrap()), &fields[7].as_slice().try_into().unwrap(), &fields[8].as_slice().try_into().unwrap()).unwrap();
        let original = build(&incoming, &fields);
        assert_eq!(original, build(&outgoing, &fields));
        assert_eq!(sas(&key, &original), sas(&key, &build(&outgoing, &fields)));
        let proof = tag(&key, b"join", &original);
        assert_ne!(proof, tag(&key, b"accept", &original));
        assert_ne!(sas(&key, &original), sas(&hmac::Key::new(hmac::HMAC_SHA256, &[42; 32]), &original));
        for index in 0..fields.len() {
            let mut changed = fields.clone();
            changed[index][0] ^= 1;
            let changed = build(&incoming, &changed);
            assert_ne!(proof, tag(&key, b"join", &changed), "field {index}");
            if index == 2 { assert_ne!(sas(&key, &original), sas(&key, &changed)); }
        }
        host.close(); joiner.close();
    }

    #[test]
    fn public_attacker_cannot_burn_with_unknown_ids() {
        let invitation = invitation();
        let state = InvitationState::new(&invitation);
        let public_id: [u8; 32] = Sha256::digest(invitation.public_bytes()).into();
        let key = hmac::Key::new(hmac::HMAC_SHA256, invitation.secret.as_ref());
        let proof = tag(&key, b"join", b"real transcript");
        for _ in 0..1000 {
            assert!(state.consume(&public_id, b"real transcript", &proof).is_err());
            assert!(state.consume(&public_id, b"wrong proof", &[0; 32]).is_err());
        }
        assert!(state.active());
        assert!(state.consume(&invitation.id(), b"real transcript", &proof).is_ok());
    }

    #[tokio::test]
    async fn two_thousand_public_only_wire_attempts_then_real_join() {
        let host = crate::Node::bind(std::sync::Arc::new(crate::Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let joiner = crate::Node::bind(std::sync::Arc::new(crate::Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let invitation = Invitation::new(host.identity().fingerprint, vec!["192.168.1.2:1234".parse().unwrap()], 24900, Duration::from_secs(60)).unwrap();
        let state = InvitationState::new(&invitation);
        let public_id: [u8; 32] = Sha256::digest(invitation.public_bytes()).into();
        let (outgoing, incoming) = tokio::join!(joiner.connect(host.local_addr().unwrap()), host.accept());
        let (outgoing, incoming) = (outgoing.unwrap(), incoming.unwrap().unwrap());
        for include_proof in [false, true] {
            for _ in 0..1000 {
                let (refused, response) = tokio::join!(
                    InviteSession::accept(&incoming, host.identity().fingerprint, "A", 24900, &state),
                    async {
                        let (mut send, mut recv) = outgoing.open_stream(StreamKind::Pairing).await.unwrap();
                        let mut bytes = MAGIC.to_vec();
                        bytes.extend_from_slice(&public_id);
                        if include_proof { bytes.extend_from_slice(&[0; 32]); }
                        send.write_all(&bytes).await.unwrap();
                        send.finish().unwrap();
                        let mut response = [255];
                        recv.read_exact(&mut response).await.unwrap();
                        response
                    }
                );
                assert!(refused.is_err());
                assert_eq!(response, [0]);
            }
        }
        assert!(state.active());
        let (host_session, join_session) = tokio::join!(
            InviteSession::accept(&incoming, host.identity().fingerprint, "A", 24900, &state),
            InviteSession::join(&outgoing, joiner.identity().fingerprint, "B", 24901, &invitation)
        );
        let (mut host_session, mut join_session) = (host_session.unwrap(), join_session.unwrap());
        assert_eq!(host_session.code, join_session.code);
        let (host_yes, join_yes) = tokio::join!(host_session.confirm(true), join_session.confirm(true));
        assert!(host_yes.unwrap() && join_yes.unwrap());
        host.close(); joiner.close();
    }
    #[test]
    fn racing_valid_proofs_have_exactly_one_winner() {
        let a = invitation();
        let state = std::sync::Arc::new(InvitationState::new(&a));
        let key = hmac::Key::new(hmac::HMAC_SHA256, a.secret.as_ref());
        let proof = tag(&key, b"join", b"t");
        let tasks: Vec<_> = (0..16).map(|_| {
            let state = state.clone();
            std::thread::spawn(move || state.consume(&state.id, b"t", &proof).is_ok())
        }).collect();
        assert_eq!(tasks.into_iter().map(|t| usize::from(t.join().unwrap())).sum::<usize>(), 1);
    }

    #[tokio::test]
    async fn replayed_proof_fails_on_a_new_tls_connection_even_with_repeated_nonces() {
        use crate::{Identity, Node};
        use std::sync::Arc;
        let a = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let b = Node::bind(Arc::new(Identity::ephemeral().unwrap()), "127.0.0.1:0".parse().unwrap()).unwrap();
        let invite = Invitation::new(a.identity().fingerprint, vec!["192.168.1.2:1234".parse().unwrap()], 24900, Duration::from_secs(60)).unwrap();
        let key = hmac::Key::new(hmac::HMAC_SHA256, invite.secret.as_ref());
        let mut captured = None;
        for _ in 0..2 {
            let (pb, pa) = tokio::join!(b.connect(a.local_addr().unwrap()), a.accept());
            let (pb, pa) = (pb.unwrap(), pa.unwrap().unwrap());
            let t = transcript(&pa, &invite.id(), a.identity().fingerprint, b.identity().fingerprint, b"A", b"B", 24900, 24901, &[1; 32], &[2; 32]).unwrap();
            if let Some(proof) = captured {
                let state = InvitationState::new(&invite);
                assert!(state.consume(&invite.id(), &t, &proof).is_err());
                assert!(state.active());
            } else { captured = Some(tag(&key, b"join", &t)); }
            pa.conn.close(0u32.into(), b"done");
            pb.conn.close(0u32.into(), b"done");
        }
        a.close(); b.close();
    }

}
