use super::{LogicError, Message, decode};
use ring::hmac;
use std::time::Duration;

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

fn data(domain: &[u8], context: AuthContext, challenge: Challenge) -> [u8; 160] {
    let mut result = [0; 160];
    result[0] = domain.len() as u8;
    let mut offset = 1;
    let helper = [match context.helper { HelperKind::None => 0, HelperKind::UserDesktop => 1, HelperKind::SecureDesktop => 2 }];
    for field in [domain, &IPC_TRANSCRIPT_VERSION.to_le_bytes(), &context.user_identity,
        &context.paired_device, &context.session.to_le_bytes(), &helper,
        &challenge.serial.to_le_bytes(), &challenge.nonce] {
        result[offset..offset + field.len()].copy_from_slice(field);
        offset += field.len();
    }
    result
}

fn tag(key: &hmac::Key, domain: &[u8], context: AuthContext, challenge: Challenge) -> [u8; 32] {
    let signature = hmac::sign(key, &data(domain, context, challenge));
    let mut result = [0; 32];
    result.copy_from_slice(signature.as_ref());
    result
}

/// Client-side credential; not a network certificate or private key.
pub struct ClientCredential(hmac::Key);

impl ClientCredential {
    pub fn new(per_boot_token: [u8; 32]) -> Self { Self(hmac::Key::new(hmac::HMAC_SHA256, &per_boot_token)) }

    pub fn proof(&self, context: AuthContext, challenge: Challenge) -> Message {
        Message::Proof(tag(&self.0, PROOF_DOMAIN, context, challenge))
    }

    pub fn verify_accepted(&self, context: AuthContext, challenge: Challenge, message: &Message) -> Result<(), LogicError> {
        let Message::Accepted(signature) = message else { return Err(LogicError::State); };
        hmac::verify(&self.0, &data(ACCEPT_DOMAIN, context, challenge), signature).map_err(|_| LogicError::Unauthorized)
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
}

impl Authenticator {
    /// Caller supplies cryptographically random boot token and monotonic time.
    pub fn new(per_boot_token: [u8; 32], context: AuthContext, now: Duration) -> Result<Self, LogicError> {
        if context.helper == HelperKind::None { return Err(LogicError::Invalid); }
        Ok(Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, &per_boot_token), context,
            expires: now.checked_add(BOOT_TOKEN_TTL).ok_or(LogicError::Clock)?,
            last_time: now, serial: 0, attempts: 0, state: State::Idle,
        })
    }

    fn check_time(&mut self, now: Duration) -> Result<(), LogicError> {
        let error = if now < self.last_time { Some(LogicError::Clock) }
            else if now >= self.expires { Some(LogicError::Expired) }
            else if self.attempts >= AUTH_ATTEMPTS { Some(LogicError::Exhausted) }
            else if self.state == State::Closed { Some(LogicError::State) }
            else { None };
        self.last_time = now;
        if let Some(error) = error { self.state = State::Closed; return Err(error); }
        Ok(())
    }

    /// Invalidates any earlier connection. Fresh nonce from the host CSPRNG.
    pub fn challenge(&mut self, now: Duration, nonce: [u8; 32]) -> Result<Challenge, LogicError> {
        self.check_time(now)?;
        if matches!(self.state, State::Pending { .. }) {
            self.state = State::Closed;
            return Err(LogicError::State);
        }
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
        let state = std::mem::replace(&mut self.state, State::Closed);
        let State::Pending { challenge, deadline } = state else { return Err(LogicError::State); };
        self.attempts += 1;
        if now >= deadline { return Err(LogicError::Expired); }
        let signature = match decode(bytes) {
            Ok(Message::Proof(signature)) => signature,
            _ => { self.state = State::Idle; return Err(LogicError::Unauthorized); }
        };
        if hmac::verify(&self.key, &data(PROOF_DOMAIN, self.context, challenge), &signature).is_err() {
            self.state = State::Idle;
            return Err(LogicError::Unauthorized);
        }
        self.attempts -= 1;
        self.state = State::Ready(challenge.serial);
        Ok(Message::Accepted(tag(&self.key, ACCEPT_DOMAIN, self.context, challenge)))
    }

    /// Transport must associate the serial with this exact authenticated pipe.
    /// Call only for commands attributed by the daemon to the pinned paired device.
    pub fn input(&mut self, now: Duration, serial: u64, paired_device: [u8; 32], bytes: &[u8]) -> Result<Message, LogicError> {
        self.check_time(now)?;
        if self.state != State::Ready(serial) || paired_device != self.context.paired_device {
            self.state = State::Closed;
            return Err(LogicError::Unauthorized);
        }
        match decode(bytes) {
            Ok(message @ (Message::Input(_) | Message::ReleaseAll)) => Ok(message),
            _ => { self.state = State::Closed; Err(LogicError::Invalid) }
        }
    }

    pub fn close(&mut self) { self.state = State::Closed; }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Inject, windows_service_logic::encode};

    fn context() -> AuthContext {
        AuthContext { user_identity: [1; 32], paired_device: [2; 32], session: 3, helper: HelperKind::UserDesktop }
    }

    fn server() -> Authenticator { Authenticator::new([7; 32], context(), Duration::ZERO).unwrap() }
    fn proof(challenge: Challenge) -> Vec<u8> {
        encode(&ClientCredential::new([7; 32]).proof(context(), challenge)).unwrap().as_bytes().to_vec()
    }
    fn input() -> Vec<u8> { encode(&Message::Input(Inject::Key { code: 30, down: true })).unwrap().as_bytes().to_vec() }

    #[test]
    fn mutual_authentication_and_input_release() {
        let mut server = server();
        let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert_eq!(decode(encode(&challenge.message()).unwrap().as_bytes()), Ok(challenge.message()));
        let accepted = server.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        let client = ClientCredential::new([7; 32]);
        assert_eq!(client.verify_accepted(context(), challenge, &accepted), Ok(()));
        assert!(client.verify_accepted(context(), challenge, &client.proof(context(), challenge)).is_err());
        assert!(ClientCredential::new([6; 32]).verify_accepted(context(), challenge, &accepted).is_err());
        assert_eq!(server.input(Duration::ZERO, challenge.serial, [2; 32], &input()), decode(&input()));
        let release = encode(&Message::ReleaseAll).unwrap();
        assert_eq!(server.input(Duration::ZERO, challenge.serial, [2; 32], release.as_bytes()), Ok(Message::ReleaseAll));
        server.close();
        assert!(server.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
    }

    #[test]
    fn input_before_authentication_and_wrong_peer_fail_closed() {
        let mut idle = server();
        assert!(idle.input(Duration::ZERO, 0, [2; 32], &input()).is_err());
        assert!(idle.challenge(Duration::ZERO, [8; 32]).is_err());
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
            let forged = encode(&ClientCredential::new([7; 32]).proof(changed, challenge)).unwrap();
            assert_eq!(server.authenticate(Duration::ZERO, forged.as_bytes()), Err(LogicError::Unauthorized));
        }
        let mut server = server();
        let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        let reflected = encode(&Message::Proof(tag(&server.key, ACCEPT_DOMAIN, context(), challenge))).unwrap();
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
            let bytes = encode(&credential.proof(context(), challenge)).unwrap();
            assert!(server.authenticate(Duration::ZERO, bytes.as_bytes()).is_err());
        }
        let mut server = server();
        let first = server.challenge(Duration::ZERO, [8; 32]).unwrap();
        server.authenticate(Duration::ZERO, &proof(first)).unwrap();
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
        assert!(expired.challenge(CHALLENGE_TTL, [9; 32]).is_err());
        let mut boot = server();
        assert_eq!(boot.challenge(BOOT_TOKEN_TTL, [8; 32]), Err(LogicError::Expired));
        let mut rollback = server();
        rollback.challenge(Duration::from_secs(1), [8; 32]).unwrap();
        assert_eq!(rollback.authenticate(Duration::ZERO, &[]), Err(LogicError::Clock));
        let mut pending = server();
        pending.challenge(Duration::ZERO, [8; 32]).unwrap();
        assert!(pending.challenge(Duration::ZERO, [9; 32]).is_err());
        assert!(pending.authenticate(Duration::ZERO, &[]).is_err());
        assert!(server().authenticate(Duration::ZERO, &[]).is_err());
        assert!(Authenticator::new([7; 32], AuthContext { helper: HelperKind::None, ..context() }, Duration::ZERO).is_err());
        assert!(Authenticator::new([7; 32], context(), Duration::MAX).is_err());
    }

    #[test]
    fn authenticated_non_input_and_malformed_input_close_the_channel() {
        for bytes in [vec![], encode(&Message::LocalInput).unwrap().as_bytes().to_vec(),
            encode(&Message::Proof([0; 32])).unwrap().as_bytes().to_vec()] {
            let mut server = server();
            let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            server.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
            assert_eq!(server.input(Duration::ZERO, challenge.serial, [2; 32], &bytes), Err(LogicError::Invalid));
            assert!(server.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
        }
        let mut replay = server();
        let challenge = replay.challenge(Duration::ZERO, [8; 32]).unwrap();
        replay.authenticate(Duration::ZERO, &proof(challenge)).unwrap();
        assert!(replay.authenticate(Duration::ZERO, &proof(challenge)).is_err());
        assert!(replay.input(Duration::ZERO, challenge.serial, [2; 32], &input()).is_err());
    }

    #[test]
    fn every_mac_byte_and_every_proof_truncation_is_rejected() {
        for index in 0..32 {
            let mut server = server();
            let challenge = server.challenge(Duration::ZERO, [8; 32]).unwrap();
            let mut bytes = proof(challenge);
            bytes[8 + index] ^= 1;
            assert_eq!(server.authenticate(Duration::ZERO, &bytes), Err(LogicError::Unauthorized));
        }
        for end in 0..40 {
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
        for _ in 1..AUTH_ATTEMPTS {
            server.challenge(Duration::ZERO, [8; 32]).unwrap();
            assert!(server.authenticate(Duration::ZERO, &[]).is_err());
        }
        assert_eq!(server.challenge(Duration::ZERO, [8; 32]), Err(LogicError::Exhausted));
    }
}
