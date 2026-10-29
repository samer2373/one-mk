use super::{LogicError, Message, decode, encode};
use super::hmac_sha256 as hmac;
use std::time::Duration;
use zeroize::Zeroize;

pub const AUTH_ATTEMPTS: u8 = 3;
pub const CHALLENGE_TTL: Duration = Duration::from_secs(5);
pub const BOOT_TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperKind { None, UserDesktop, SecureDesktop }

/// Policy facts supplied by trusted provisioning, never by a pipe client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthContext {
    pub user_identity: [u8; 32],
    pub paired_device: [u8; 32],
    pub session: u32,
    pub helper: HelperKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub serial: u64,
    pub nonce: [u8; 32],
}

impl Challenge {
    pub fn message(self) -> Message { Message::Challenge { serial: self.serial, nonce: self.nonce } }
}

const IPC_TRANSCRIPT_VERSION: u16 = super::IPC_VERSION;
const PROOF_DOMAIN: &[u8] = b"one-mk/local-helper/client-proof\0";
const ACCEPT_DOMAIN: &[u8] = b"one-mk/local-helper/server-accepted\0";
const SESSION_DOMAIN: &[u8] = b"one-mk/local-helper/session-key\0";

fn data(domain: &[u8], context: AuthContext, challenge: Challenge, nonce: [u8; 32]) -> [u8; 192] {
    let mut result = [0; 192];
    result[0] = domain.len() as u8;
    let mut offset = 1;
    let helper = [match context.helper { HelperKind::None => 0, HelperKind::UserDesktop => 1, HelperKind::SecureDesktop => 2 }];
    for field in [domain, &IPC_TRANSCRIPT_VERSION.to_le_bytes(), &context.user_identity,
        &context.paired_device, &context.session.to_le_bytes(), &helper,
        &challenge.serial.to_le_bytes(), &challenge.nonce, &nonce] {
        result[offset..offset + field.len()].copy_from_slice(field);
        offset += field.len();
    }
    result
}

fn tag(key: &hmac::Key, domain: &[u8], context: AuthContext, challenge: Challenge, nonce: [u8; 32]) -> [u8; 32] {
    hmac::sign(key, &data(domain, context, challenge, nonce))
}

fn session_key(key: &hmac::Key, context: AuthContext, challenge: Challenge, nonce: [u8; 32]) -> hmac::Key {
    let mut secret = tag(key, SESSION_DOMAIN, context, challenge, nonce);
    let key = hmac::Key::new(&secret);
    secret.zeroize();
    key
}

pub struct ClientSession { key: hmac::Key, counter: u64 }

impl ClientSession {
    pub fn seal(&mut self, message: &Message) -> Result<Vec<u8>, LogicError> {
        if !matches!(message, Message::Input(_) | Message::ReleaseAll) { return Err(LogicError::Invalid); }
        let frame = encode(message)?;
        let counter = self.counter.checked_add(1).ok_or(LogicError::Exhausted)?;
        let mut bytes = Vec::with_capacity(8 + frame.as_bytes().len() + 32);
        bytes.extend_from_slice(&counter.to_le_bytes());
        bytes.extend_from_slice(frame.as_bytes());
        bytes.extend_from_slice(&hmac::sign(&self.key, &bytes));
        self.counter = counter;
        Ok(bytes)
    }
}

/// Client-side credential; not a network certificate or private key.
pub struct ClientCredential(hmac::Key);

impl ClientCredential {
    pub fn new(mut per_boot_token: [u8; 32]) -> Self {
        let key = hmac::Key::new(&per_boot_token);
        per_boot_token.zeroize();
        Self(key)
    }

    pub fn proof(&self, context: AuthContext, challenge: Challenge, nonce: [u8; 32]) -> Message {
        Message::Proof { nonce, signature: tag(&self.0, PROOF_DOMAIN, context, challenge, nonce) }
    }

    pub fn verify_accepted(&self, context: AuthContext, challenge: Challenge, nonce: [u8; 32], message: &Message) -> Result<(), LogicError> {
        let Message::Accepted(signature) = message else { return Err(LogicError::State); };
        if hmac::verify(&self.0, &data(ACCEPT_DOMAIN, context, challenge, nonce), signature) { Ok(()) } else { Err(LogicError::Unauthorized) }
    }

    pub fn session(&self, context: AuthContext, challenge: Challenge, nonce: [u8; 32], accepted: &Message) -> Result<ClientSession, LogicError> {
        self.verify_accepted(context, challenge, nonce, accepted)?;
        Ok(ClientSession { key: session_key(&self.0, context, challenge, nonce), counter: 0 })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Pending { challenge: Challenge, deadline: Duration },
    Ready(u64),
    Closed,
}

/// One boot-scoped authority for one provisioned user/device/session/helper.
/// Retain it across reconnects: replacing it resets the attempt/replay budget.
pub struct Authenticator {
    key: hmac::Key,
    context: AuthContext,
    expires: Duration,
    last_time: Duration,
    serial: u64,
    attempts: u8,
    state: State,
    session_key: Option<hmac::Key>,
    counter: u64,
}

impl Authenticator {
    /// Caller supplies cryptographically random boot token and monotonic time.
    pub fn new(mut per_boot_token: [u8; 32], context: AuthContext, now: Duration) -> Result<Self, LogicError> {
        let key = hmac::Key::new(&per_boot_token);
        per_boot_token.zeroize();
        if context.helper == HelperKind::None { return Err(LogicError::Invalid); }
        Ok(Self {
            key, context,
            expires: now.checked_add(BOOT_TOKEN_TTL).ok_or(LogicError::Clock)?,
            last_time: now, serial: 0, attempts: 0, state: State::Idle, session_key: None, counter: 0,
        })
    }

    fn check_time(&mut self, now: Duration) -> Result<(), LogicError> {
        let error = if now < self.last_time { Some(LogicError::Clock) }
            else if now >= self.expires { Some(LogicError::Expired) }
            else if self.attempts >= AUTH_ATTEMPTS { Some(LogicError::Exhausted) }
            else if self.state == State::Closed { Some(LogicError::State) }
            else { None };
        self.last_time = now;
        if let Some(error) = error {
            self.session_key = None;
            self.state = if error == LogicError::Expired { State::Idle } else { State::Closed };
            return Err(error);
        }
        Ok(())
    }

    /// Fresh nonce from the host CSPRNG. An authenticated connection is never evicted.
    pub fn challenge(&mut self, now: Duration, nonce: [u8; 32]) -> Result<Challenge, LogicError> {
        self.check_time(now)?;
        if matches!(self.state, State::Ready(_)) { return Err(LogicError::State); }
        if matches!(self.state, State::Pending { .. }) { self.abandon(); self.check_time(now)?; }
        let Some(serial) = self.serial.checked_add(1) else {
            self.state = State::Closed;
            return Err(LogicError::Exhausted);
        };
        self.serial = serial;
        let challenge = Challenge { serial, nonce };
        let deadline = now.saturating_add(CHALLENGE_TTL).min(self.expires);
        self.state = State::Pending { challenge, deadline };
        Ok(challenge)
    }

    /// Consumes the challenge even on malformed proof. HMAC verifies in constant time.
    pub fn authenticate(&mut self, now: Duration, bytes: &[u8]) -> Result<Message, LogicError> {
        self.check_time(now)?;
        let state = self.state;
        let State::Pending { challenge, deadline } = state else { return Err(LogicError::State); };
        self.abandon();
        if now >= deadline { return Err(LogicError::Expired); }
        let (nonce, signature) = match decode(bytes) {
            Ok(Message::Proof { nonce, signature }) => (nonce, signature),
            _ => return Err(LogicError::Unauthorized),
        };
        if !hmac::verify(&self.key, &data(PROOF_DOMAIN, self.context, challenge, nonce), &signature) { return Err(LogicError::Unauthorized); }
        self.attempts -= 1;
        self.state = State::Ready(challenge.serial);
        self.session_key = Some(session_key(&self.key, self.context, challenge, nonce));
        self.counter = 0;
        Ok(Message::Accepted(tag(&self.key, ACCEPT_DOMAIN, self.context, challenge, nonce)))
    }

    /// Transport must associate the serial with this exact authenticated pipe.
    /// Call only for commands attributed by the daemon to the pinned paired device.
    pub fn input(&mut self, now: Duration, serial: u64, paired_device: [u8; 32], bytes: &[u8]) -> Result<Message, LogicError> {
        self.check_time(now)?;
        if self.state != State::Ready(serial) || paired_device != self.context.paired_device {
            self.abandon();
            return Err(LogicError::Unauthorized);
        }
        let result = self.verify_input(bytes);
        if result.is_err() { self.abandon(); }
        result
    }

    fn verify_input(&mut self, bytes: &[u8]) -> Result<Message, LogicError> {
        if bytes.len() < 48 || bytes.len() > super::MAX_FRAME + 40 { return Err(LogicError::Invalid); }
        let end = bytes.len() - 32;
        let counter = u64::from_le_bytes(bytes[..8].try_into().map_err(|_| LogicError::Invalid)?);
        if self.counter.checked_add(1) != Some(counter)
            || !hmac::verify(self.session_key.as_ref().ok_or(LogicError::State)?, &bytes[..end], &bytes[end..]) {
            return Err(LogicError::Unauthorized);
        }
        match decode(&bytes[8..end]) {
            Ok(message @ (Message::Input(_) | Message::ReleaseAll)) => { self.counter = counter; Ok(message) }
            _ => Err(LogicError::Invalid),
        }
    }

    pub fn abandon(&mut self) {
        if matches!(self.state, State::Pending { .. }) { self.attempts += 1; }
        self.session_key = None;
        if self.state != State::Closed {
            self.state = if self.attempts >= AUTH_ATTEMPTS { State::Closed } else { State::Idle };
        }
    }

    pub fn close(&mut self) { self.abandon(); }

    pub fn reprovision(&mut self, context: AuthContext) -> Result<(), LogicError> {
        if context.helper == HelperKind::None { return Err(LogicError::Invalid); }
        if self.state == State::Closed { return Err(LogicError::State); }
        self.abandon();
        if self.state == State::Closed { return Err(LogicError::Exhausted); }
        self.context = context;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ready_refuses_new_challenges_and_unsigned_input() {
        let mut authority = server();
        let challenge = authority.challenge(Duration::ZERO, [8; 32]).unwrap();
        authority.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert!(authority.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
    }

    #[test]
    fn ready_challenge_does_not_evict_session() {
        let mut authority = server();
        let challenge = authority.challenge(Duration::ZERO, [8; 32]).unwrap();
        authority.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert_eq!(authority.challenge(Duration::ZERO, [9; 32]), Err(LogicError::State));
        let signed = client_session(challenge).seal(&Message::ReleaseAll).unwrap();
        assert_eq!(authority.input(Duration::ZERO, challenge.serial, [2; 32], &signed), Ok(Message::ReleaseAll));
    }

    #[test]
    fn expired_handshake_can_reconnect() {
        let mut authority = server();
        let challenge = authority.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert_eq!(authority.authenticate(CHALLENGE_TTL, &proof(challenge)), Err(LogicError::Expired));
        assert!(authority.challenge(CHALLENGE_TTL, [9; 32]).is_ok());
    }

    use super::*;
    use crate::Inject;

    fn context() -> AuthContext {
        AuthContext { user_identity: [1; 32], paired_device: [2; 32], session: 3, helper: HelperKind::UserDesktop }
    }

    fn server() -> Authenticator { Authenticator::new([7; 32], context(), Duration::ZERO).unwrap() }
    fn proof(challenge: Challenge) -> Vec<u8> {
        encode(&ClientCredential::new([7; 32]).proof(context(), challenge, [6; 32])).unwrap().as_bytes().to_vec()
    }
    fn input() -> Vec<u8> { encode(&Message::Input(Inject::Key { code: 30, down: true })).unwrap().as_bytes().to_vec() }

    fn client_session(challenge: Challenge) -> ClientSession {
        let credential = ClientCredential::new([7; 32]);
        let accepted = Message::Accepted(tag(&credential.0, ACCEPT_DOMAIN, context(), challenge, [6; 32]));
        credential.session(context(), challenge, [6; 32], &accepted).unwrap()
    }

    #[test]
    fn mutual_authentication_and_input_release() {
        let mut server = server();
        let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert_eq!(decode(encode(&challenge.message()).unwrap().as_bytes()), Ok(challenge.message()));
        let accepted = server.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        let client = ClientCredential::new([7; 32]);
        assert_eq!(client.verify_accepted(context(), challenge, [6; 32], &accepted), Ok(()));
        assert!(client.verify_accepted(context(), challenge, [5; 32], &accepted).is_err());
        assert!(client.verify_accepted(context(), challenge, [6; 32], &client.proof(context(), challenge, [6; 32])).is_err());
        assert!(ClientCredential::new([6; 32]).verify_accepted(context(), challenge, [6; 32], &accepted).is_err());
        let mut session = client.session(context(), challenge, [6; 32], &accepted).unwrap();
        let signed = session.seal(&decode(&input()).unwrap()).unwrap();
        assert_eq!(server.input(Duration::ZERO, challenge.serial, [2; 32], &signed), decode(&input()));
        let release = session.seal(&Message::ReleaseAll).unwrap();
        assert_eq!(server.input(Duration::ZERO, challenge.serial, [2; 32], &release), Ok(Message::ReleaseAll));
        server.close();
        assert!(server.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
    }

    #[test]
    fn input_before_authentication_and_wrong_peer_fail_closed() {
        let mut idle = server();
        assert!(idle.input(Duration::ZERO, 0, [2; 32], &input()).is_err());
        assert!(idle.challenge(Duration::ZERO, [8; 32]).is_ok());
        let mut ready = server();
        let challenge = ready.challenge(Duration::ZERO, [8; 32]).unwrap();
        ready.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert!(ready.input(Duration::ZERO, challenge.serial, [9; 32], &input()).is_err());
        assert!(ready.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
    }

    #[test]
    fn bounded_attempts_survive_reconnects() {
        let mut server = server();
        for attempt in 0..AUTH_ATTEMPTS {
            server.challenge(Duration::ZERO, [attempt; 32]).unwrap();
            assert_eq!(server.authenticate(Duration::ZERO, &[255; 40]), Err(LogicError::Unauthorized));
        }
        assert_eq!(server.challenge(Duration::ZERO, [8; 32]), Err(LogicError::Exhausted));
    }

    #[test]
    fn every_transcript_field_is_bound_and_roles_are_separated() {
        let original = context();
        let mut variants = [original; 5];
        variants[0].user_identity[0] ^= 1;
        variants[1].paired_device[0] ^= 1;
        variants[2].session += 1;
        variants[3].helper = HelperKind::SecureDesktop;
        variants[4].helper = HelperKind::None;
        for changed in variants {
            let mut server = server();
            let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            let forged = encode(&ClientCredential::new([7; 32]).proof(changed, challenge, [6; 32])).unwrap();
            assert_eq!(server.authenticate(Duration::ZERO, forged.as_bytes()), Err(LogicError::Unauthorized));
        }
        let mut server = server();
        let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        let reflected = encode(&Message::Proof { nonce: [6; 32], signature: tag(&server.key, ACCEPT_DOMAIN, context(), challenge, [6; 32]) }).unwrap();
        assert!(server.authenticate(Duration::ZERO, reflected.as_bytes()).is_err());
    }

    #[test]
    fn nonce_serial_boot_token_and_connection_replay_are_rejected() {
        for change in 0..3 {
            let mut server = server();
            let mut challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            if change == 0 { challenge.nonce[0] ^= 1; }
            if change == 1 { challenge.serial += 1; }
            let credential = ClientCredential::new(if change == 2 { [9; 32] } else { [7; 32] });
            let bytes = encode(&credential.proof(context(), challenge, [6; 32])).unwrap();
            assert!(server.authenticate(Duration::ZERO, bytes.as_bytes()).is_err());
        }
        let mut server = server();
        let first = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        server.authenticate(Duration::ZERO, &proof(first)).unwrap();
        server.abandon();
        let second = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert_ne!(first.serial, second.serial);
        assert!(server.authenticate(Duration::ZERO, &proof(first)).is_err());
        let third = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        server.authenticate(Duration::ZERO, &proof(third)).unwrap();
        assert!(server.input(Duration::ZERO, first.serial, [2; 32], &input()).is_err());
    }

    #[test]
    fn expiry_clock_rollback_and_terminal_states() {
        let mut expired = server();
        let challenge = expired.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert_eq!(expired.authenticate(CHALLENGE_TTL, &proof(challenge)), Err(LogicError::Expired));
        assert!(expired.challenge(CHALLENGE_TTL, [9; 32]).is_ok());
        let mut boot = server();
        assert_eq!(boot.challenge(BOOT_TOKEN_TTL, [8; 32]), Err(LogicError::Expired));
        let mut rollback = server();
        rollback.challenge(Duration::from_secs(1), [8; 32]).unwrap();
        assert_eq!(rollback.authenticate(Duration::ZERO, &[]), Err(LogicError::Clock));
        let mut pending = server();
        pending.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert!(pending.challenge(Duration::ZERO, [9; 32]).is_ok());
        assert!(pending.authenticate(Duration::ZERO, &[]).is_err());
        assert!(server().authenticate(Duration::ZERO, &[]).is_err());
        assert!(Authenticator::new([7; 32], AuthContext { helper: HelperKind::None, ..context() }, Duration::ZERO).is_err());
        assert!(Authenticator::new([7; 32], context(), Duration::MAX).is_err());
    }

    #[test]
    fn authenticated_non_input_and_malformed_input_close_the_channel() {
        for bytes in [vec![], encode(&Message::LocalInput).unwrap().as_bytes().to_vec(),
            encode(&Message::Proof { nonce: [0; 32], signature: [0; 32] }).unwrap().as_bytes().to_vec()] {
            let mut server = server();
            let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            server.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
            let session = client_session(challenge);
            let mut signed = 1u64.to_le_bytes().to_vec();
            signed.extend_from_slice(&bytes);
            signed.extend_from_slice(&hmac::sign(&session.key, &signed));
            assert_eq!(server.input(Duration::ZERO, challenge.serial, [2; 32], &signed), Err(LogicError::Invalid));
            assert!(server.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
        }
        let mut replay = server();
        let challenge = replay.challenge(Duration::ZERO, [8; 32]).unwrap();
        replay.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert!(replay.authenticate(Duration::ZERO, &proof(challenge)).is_err());
        let signed = client_session(challenge).seal(&Message::ReleaseAll).unwrap();
        assert_eq!(replay.input(Duration::ZERO, challenge.serial, [2; 32], &signed), Ok(Message::ReleaseAll));
    }

    #[test]
    fn every_mac_byte_and_every_proof_truncation_is_rejected() {
        for index in 0..64 {
            let mut server = server();
            let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            let mut bytes = proof(challenge);
            bytes[8 + index] ^= 1;
            assert_eq!(server.authenticate(Duration::ZERO, &bytes), Err(LogicError::Unauthorized));
        }
        for end in 0..72 {
            let mut server = server();
            let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            assert_eq!(server.authenticate(Duration::ZERO, &proof(challenge)[..end]), Err(LogicError::Unauthorized));
        }
    }

    #[test]
    fn ready_connection_expires_and_failed_budget_is_not_reset_by_success() {
        let mut expired = server();
        let challenge = expired.challenge(Duration::ZERO, [8; 32]).unwrap();
        expired.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert_eq!(expired.input(BOOT_TOKEN_TTL, challenge.serial, [2; 32], &input()), Err(LogicError::Expired));
        let mut server = server();
        server.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert!(server.authenticate(Duration::ZERO, &[]).is_err());
        let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        server.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert_eq!(server.attempts, 1);
        server.abandon();
        for _ in 1..AUTH_ATTEMPTS {
            server.challenge(Duration::ZERO, [8; 32]).unwrap();
            assert!(server.authenticate(Duration::ZERO, &[]).is_err());
        }
        assert_eq!(server.challenge(Duration::ZERO, [8; 32]), Err(LogicError::Exhausted));
    }

    #[test]
    fn signed_input_rejects_tampering_replay_reordering_and_other_sessions() {
        for attack in 0..5 {
            let mut authority = server();
            let challenge = authority.challenge(Duration::ZERO, [8; 32]).unwrap();
            authority.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
            let mut session = client_session(challenge);
            let first = session.seal(&decode(&input()).unwrap()).unwrap();
            let second = session.seal(&Message::ReleaseAll).unwrap();
            let mut bytes = first.clone();
            match attack {
                0 => bytes[16] ^= 1,
                1 => { authority.input(Duration::ZERO, challenge.serial, [2; 32], &first).unwrap(); }
                2 => bytes = second,
                3 => bytes = client_session(Challenge { serial: challenge.serial + 1, ..challenge }).seal(&Message::ReleaseAll).unwrap(),
                _ => { let last = bytes.len() - 1; bytes[last] ^= 1; }
            }
            assert!(authority.input(Duration::ZERO, challenge.serial, [2; 32], &bytes).is_err());
            assert!(authority.input(Duration::ZERO, challenge.serial, [2; 32], &first).is_err());
        }
    }

    #[test]
    fn abandon_rechallenge_and_reprovision_preserve_boot_limits() {
        let mut authority = server();
        authority.abandon();
        assert_eq!(authority.attempts, 0);
        let first = authority.challenge(Duration::ZERO, [8; 32]).unwrap();
        let second = authority.challenge(Duration::ZERO, [9; 32]).unwrap();
        assert!(second.serial > first.serial);
        assert_eq!(authority.attempts, 1);
        let context = AuthContext { paired_device: [4; 32], ..context() };
        let expiry = authority.expires;
        authority.reprovision(context).unwrap();
        assert_eq!(authority.attempts, 2);
        assert_eq!(authority.expires, expiry);
        assert_eq!(authority.last_time, Duration::ZERO);
        let third = authority.challenge(Duration::ZERO, [10; 32]).unwrap();
        let credential = ClientCredential::new([7; 32]);
        let proof = encode(&credential.proof(context, third, [11; 32])).unwrap();
        authority.authenticate(Duration::ZERO, proof.as_bytes()).unwrap();
        assert_eq!(authority.attempts, 2);
        authority.abandon();
        authority.challenge(Duration::ZERO, [12; 32]).unwrap();
        authority.abandon();
        assert_eq!(authority.challenge(Duration::ZERO, [13; 32]), Err(LogicError::Exhausted));
        assert!(authority.reprovision(context).is_err());
    }
}
