//! Authenticated daemon-to-renderer presentation session (6D-M1 step A).
//!
//! I/O-free pieces shared by the laboratory daemon authority and renderer:
//! the private authority record, keyed message tags, the daemon-side plan
//! publisher and the renderer-side session that feeds `Gate::accept_encoded`.
//! Files, D-Bus, processes and surfaces belong to the callers. Nothing here
//! authorizes production playback.
//!
//! Trust model: the daemon writes a fresh random key, its authority generation,
//! its PID and its unique bus name into a mode-0600 record inside the user's
//! mode-0700 runtime directory. The renderer reads that record, checks that the
//! unique name is currently owned by that PID and UID, and from then on accepts
//! a reply only when its tag proves knowledge of the key and binds the
//! renderer's fresh nonce plus both unique names. Being on the bus, owning a
//! name or serving the object path grants nothing (no ambient trust). A record
//! left behind by a dead daemon is stale: its name has no owner, and a new
//! daemon never reuses its key or generation.

use crate::bridge::{self, Monitor};
use crate::presentation::{self, Gate, Owner, PresentationPlan, Rect, Slice};
use crate::presentation_wire;

pub const INTERFACE: &str = "org.onemk.WallpaperAuthority1";
pub const PATH: &str = "/org/onemk/WallpaperAuthority1";
pub const METHOD: &str = "Plan";
pub const CHANGED: &str = "Changed";
/// nonce, layout serial, monitors, request tag (zbus renders a multi-argument body as a structure).
pub const REQUEST_SIGNATURE: &str = "(ayua(iiiid)ay)";
/// enabled, consent revision, encoded plan, reply tag.
pub const REPLY_SIGNATURE: &str = "(btayay)";
pub const MAX_REQUEST_BODY: usize = 2048;
pub const MAX_REPLY_BODY: usize = presentation_wire::MAX_PAYLOAD + 256;
pub const RUNTIME_DIRECTORY: &str = "one-mk-wallpaper-m1";
pub const RECORD_FILE: &str = "authority";
pub const MAX_RECORD: usize = 512;
/// Refused plans in a row before the renderer stops trusting the session.
pub const MAX_CONSECUTIVE_REFUSALS: u32 = 16;
pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 32;
pub const TAG_BYTES: usize = 32;

const RECORD_MAGIC: &[u8; 4] = b"WAUT";
const CONTEXT: &[u8] = b"one-mk wallpaper presentation m1 v1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError { Bounds, Version, Key, Generation, Pid, Name }

/// What the daemon publishes in its private runtime file.
#[derive(Clone, PartialEq, Eq)]
pub struct Record {
    pub key: [u8; KEY_BYTES],
    pub generation: [u8; 16],
    pub pid: u32,
    pub unique_name: String,
}

impl std::fmt::Debug for Record {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Record").field("key", &"<redacted>").field("generation", &self.generation)
            .field("pid", &self.pid).field("unique_name", &self.unique_name).finish()
    }
}

impl Record {
    pub fn encode(&self) -> Result<Vec<u8>, RecordError> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(4 + 1 + KEY_BYTES + 16 + 4 + 1 + self.unique_name.len());
        bytes.extend_from_slice(RECORD_MAGIC);
        bytes.push(1);
        bytes.extend_from_slice(&self.key);
        bytes.extend_from_slice(&self.generation);
        bytes.extend_from_slice(&self.pid.to_le_bytes());
        bytes.push(self.unique_name.len() as u8);
        bytes.extend_from_slice(self.unique_name.as_bytes());
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordError> {
        if bytes.len() > MAX_RECORD { return Err(RecordError::Bounds); }
        let (magic, rest) = bytes.split_at_checked(4).ok_or(RecordError::Bounds)?;
        let (version, rest) = rest.split_first().ok_or(RecordError::Bounds)?;
        if magic != RECORD_MAGIC || *version != 1 { return Err(RecordError::Version); }
        let (key, rest) = rest.split_at_checked(KEY_BYTES).ok_or(RecordError::Bounds)?;
        let (generation, rest) = rest.split_at_checked(16).ok_or(RecordError::Bounds)?;
        let (pid, rest) = rest.split_at_checked(4).ok_or(RecordError::Bounds)?;
        let (length, rest) = rest.split_first().ok_or(RecordError::Bounds)?;
        if rest.len() != usize::from(*length) { return Err(RecordError::Bounds); }
        let record = Self {
            key: key.try_into().map_err(|_| RecordError::Bounds)?,
            generation: generation.try_into().map_err(|_| RecordError::Bounds)?,
            pid: u32::from_le_bytes(pid.try_into().map_err(|_| RecordError::Bounds)?),
            unique_name: std::str::from_utf8(rest).map_err(|_| RecordError::Name)?.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), RecordError> {
        if self.key == [0; KEY_BYTES] { return Err(RecordError::Key); }
        if self.generation == [0; 16] { return Err(RecordError::Generation); }
        if self.pid == 0 { return Err(RecordError::Pid); }
        if !presentation::valid_unique_name(&self.unique_name) { return Err(RecordError::Name); }
        Ok(())
    }
}

fn tag(key: &[u8; KEY_BYTES], domain: &[u8], fields: &[&[u8]]) -> [u8; TAG_BYTES] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(CONTEXT);
    hasher.update(domain);
    for field in fields {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    *hasher.finalize().as_bytes()
}

fn monitor_bytes(monitors: &[Monitor]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(monitors.len() * 24);
    for &(x, y, width, height, scale) in monitors {
        for value in [x, y, width, height] { bytes.extend_from_slice(&value.to_le_bytes()); }
        bytes.extend_from_slice(&scale.to_bits().to_le_bytes());
    }
    bytes
}

/// Proves to the daemon that the caller holds the key, for this pair of
/// connections and this nonce.
pub fn request_tag(key: &[u8; KEY_BYTES], renderer: &str, authority: &str, nonce: &[u8; NONCE_BYTES],
    serial: u32, monitors: &[Monitor]) -> [u8; TAG_BYTES] {
    tag(key, b"request", &[renderer.as_bytes(), authority.as_bytes(), nonce, &serial.to_le_bytes(),
        &monitor_bytes(monitors)])
}

/// Proves to the renderer that this answer to this request came from the key holder.
pub fn reply_tag(key: &[u8; KEY_BYTES], renderer: &str, authority: &str, nonce: &[u8; NONCE_BYTES],
    enabled: bool, revision: u64, payload: &[u8]) -> [u8; TAG_BYTES] {
    tag(key, b"reply", &[renderer.as_bytes(), authority.as_bytes(), nonce, &[u8::from(enabled)],
        &revision.to_le_bytes(), payload])
}

/// Constant-time comparison of a received tag of any length.
pub fn tag_matches(expected: &[u8; TAG_BYTES], received: &[u8]) -> bool {
    let Ok(received) = <[u8; TAG_BYTES]>::try_from(received) else { return false };
    blake3::Hash::from(*expected) == blake3::Hash::from(received)
}

#[derive(Clone, Debug, PartialEq)]
struct PlanKey { revision: u64, serial: u32, monitors: Vec<Monitor>, paused: bool }

/// Daemon side: turns live consent plus the renderer's reported layout into
/// encoded plans, advancing the sequence only when the content changes.
#[derive(Debug)]
pub struct Publisher {
    generation: [u8; 16],
    media: [u8; 32],
    canvas: Rect,
    origin: (i32, i32),
    sequence: u64,
    current: Option<(PlanKey, Vec<u8>)>,
    previous: Option<Vec<u8>>,
}

impl Publisher {
    /// `origin` is where this computer's (0, 0) sits on the shared canvas.
    pub fn new(generation: [u8; 16], media: [u8; 32], canvas: Rect, origin: (i32, i32)) -> Result<Self, presentation::Error> {
        if generation == [0; 16] { return Err(presentation::Error::Generation); }
        if media == [0; 32] { return Err(presentation::Error::Media); }
        if canvas.width == 0 || canvas.height == 0 || i32::try_from(canvas.width).is_err() ||
            i32::try_from(canvas.height).is_err() { return Err(presentation::Error::Geometry); }
        Ok(Self { generation, media, canvas, origin, sequence: 0, current: None, previous: None })
    }

    pub fn generation(&self) -> [u8; 16] { self.generation }

    /// A layout with an output outside the committed canvas (added, or moved
    /// past its edge) gets a paused plan: the whole local presentation stays
    /// static until the layout fits again; the canvas is never reflowed.
    pub fn plan(&mut self, revision: u64, serial: u32, monitors: &[Monitor], paused: bool)
        -> Result<Vec<u8>, presentation::Error> {
        if revision == 0 { return Err(presentation::Error::Consent); }
        bridge::validate_layout(serial, monitors).map_err(|_| presentation::Error::Layout)?;
        let key = PlanKey { revision, serial, monitors: monitors.to_vec(), paused };
        if let Some((current, bytes)) = &self.current && *current == key { return Ok(bytes.clone()); }
        let mut slices = Vec::with_capacity(monitors.len());
        for (index, &(x, y, width, height, _)) in monitors.iter().enumerate() {
            let x = x.checked_add(self.origin.0).ok_or(presentation::Error::Geometry)?;
            let y = y.checked_add(self.origin.1).ok_or(presentation::Error::Geometry)?;
            slices.push(Slice { screen: format!("monitor-{index}"),
                canvas: Rect { x, y, width: width as u32, height: height as u32 } });
        }
        let mut fits = true;
        for slice in &slices { fits &= self.canvas.contains(slice.canvas)?; }
        let paused = paused || !fits;
        let sequence = self.sequence.checked_add(1).ok_or(presentation::Error::Replay)?;
        let plan = PresentationPlan { generation: self.generation, consent_revision: revision, sequence,
            layout_serial: serial, media_id: self.media, canvas: self.canvas, slices, looping: true, paused };
        let bytes = presentation_wire::encode(&plan).map_err(|_| presentation::Error::Wire)?;
        self.sequence = sequence;
        self.previous = self.current.take().map(|(_, bytes)| bytes);
        self.current = Some((key, bytes.clone()));
        Ok(bytes)
    }

    /// The plan published before the current one (laboratory replay fault).
    pub fn previous(&self) -> Option<&[u8]> { self.previous.as_deref() }

    pub fn sequence(&self) -> u64 { self.sequence }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal { Envelope, Wire, Identity, Consent, Generation, Replay, Layout, Geometry, Media }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Teardown {
    /// A reply did not carry a valid tag for this request.
    Authentication,
    /// Consent was disabled, removed or replaced by a new revision.
    Revoked,
    /// The authority's connection went away (exit, crash, SIGKILL).
    OwnerLost,
    /// Too many refused plans in a row.
    Faults,
    /// The Shell bridge reported an invalid layout or pause state.
    Protocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Accepted { sequence: u64, active: bool },
    /// The same bytes as the admitted plan: nothing to do.
    Unchanged,
    /// The plan was refused; the admitted plan, if any, stays as it was.
    Refused(Refusal),
    /// The session is over: destroy the presentation and exit.
    Teardown(Teardown),
}

/// A reply envelope the transport has already bounded and type-checked.
pub struct Reply<'a> {
    pub enabled: bool,
    pub revision: u64,
    pub payload: &'a [u8],
    pub tag: &'a [u8],
}

/// Renderer side: authenticates each reply and admits plans through `Gate`.
#[derive(Debug)]
pub struct Session {
    owner: Owner,
    record: Record,
    renderer: String,
    serial: u32,
    monitors: Vec<Monitor>,
    reasons: u32,
    gate: Option<Gate>,
    revision: Option<u64>,
    admitted: Option<(Vec<u8>, PresentationPlan)>,
    refusals: u32,
    ended: Option<Teardown>,
}

impl Session {
    /// `owner` must come from the bus (unique name, UID, PID of the record's
    /// name), `renderer` is this process's own unique name.
    pub fn new(owner: Owner, record: Record, renderer: String, serial: u32, monitors: Vec<Monitor>,
        reasons: u32) -> Result<Self, presentation::Error> {
        if owner.unique_name != record.unique_name || owner.pid != record.pid ||
            !presentation::valid_unique_name(&owner.unique_name) || !presentation::valid_unique_name(&renderer) ||
            renderer == owner.unique_name { return Err(presentation::Error::Identity); }
        bridge::validate_layout(serial, &monitors).map_err(|_| presentation::Error::Layout)?;
        bridge::validate_reasons(reasons).map_err(|_| presentation::Error::Layout)?;
        Ok(Self { owner, record, renderer, serial, monitors, reasons, gate: None, revision: None,
            admitted: None, refusals: 0, ended: None })
    }

    pub fn serial(&self) -> u32 { self.serial }
    pub fn monitors(&self) -> &[Monitor] { &self.monitors }

    /// The tag for a `Plan` request with this fresh nonce.
    pub fn request_tag(&self, nonce: &[u8; NONCE_BYTES]) -> [u8; TAG_BYTES] {
        request_tag(&self.record.key, &self.renderer, &self.owner.unique_name, nonce, self.serial, &self.monitors)
    }

    /// The last admitted plan, kept through refusals; `None` after a layout
    /// change until a fresh plan arrives, and after teardown.
    pub fn admitted(&self) -> Option<&PresentationPlan> { self.admitted.as_ref().map(|(_, plan)| plan) }

    /// Whether the admitted plan should be on screen now.
    pub fn active(&self) -> bool { self.gate.as_ref().is_some_and(|gate| gate.active().is_some()) }

    pub fn ended(&self) -> Option<Teardown> { self.ended }

    fn end(&mut self, reason: Teardown) -> Outcome {
        if let Some(gate) = &mut self.gate { gate.revoke(); }
        self.admitted = None;
        self.ended.get_or_insert(reason);
        Outcome::Teardown(self.ended.unwrap_or(reason))
    }

    fn refuse(&mut self, refusal: Refusal) -> Outcome {
        self.refusals += 1;
        if self.refusals >= MAX_CONSECUTIVE_REFUSALS { return self.end(Teardown::Faults); }
        Outcome::Refused(refusal)
    }

    /// An envelope that was oversize or of the wrong type: refused unread.
    pub fn invalid_envelope(&mut self) -> Outcome {
        if let Some(reason) = self.ended { return Outcome::Teardown(reason); }
        self.refuse(Refusal::Envelope)
    }

    /// Handles the authority's answer to the request made with `nonce`.
    /// `caller` is the replying connection as identified by the bus.
    pub fn reply(&mut self, caller: &Owner, nonce: &[u8; NONCE_BYTES], reply: Reply<'_>) -> Outcome {
        if let Some(reason) = self.ended { return Outcome::Teardown(reason); }
        if caller != &self.owner { return self.end(Teardown::Authentication); }
        let expected = reply_tag(&self.record.key, &self.renderer, &self.owner.unique_name, nonce,
            reply.enabled, reply.revision, reply.payload);
        if !tag_matches(&expected, reply.tag) { return self.end(Teardown::Authentication); }
        if !reply.enabled || reply.revision == 0 || self.revision.is_some_and(|revision| revision != reply.revision) {
            return self.end(Teardown::Revoked);
        }
        if self.admitted.as_ref().is_some_and(|(bytes, _)| bytes == reply.payload) {
            self.refusals = 0;
            return Outcome::Unchanged;
        }
        let Ok(plan) = presentation_wire::decode(reply.payload) else { return self.refuse(Refusal::Wire) };
        let fresh = self.gate.is_none();
        if fresh {
            if plan.consent_revision != reply.revision { return self.refuse(Refusal::Consent); }
            if plan.generation != self.record.generation { return self.refuse(Refusal::Generation); }
            let gate = Gate::new(self.owner.clone(), reply.revision, self.record.generation, plan.canvas,
                self.serial, self.monitors.clone()).and_then(|mut gate| gate.pause_hint(self.reasons).map(|()| gate));
            match gate {
                Ok(gate) => self.gate = Some(gate),
                Err(error) => return self.refuse(refusal(error)),
            }
        }
        let gate = self.gate.as_mut().expect("gate exists");
        match gate.accept_encoded(caller, reply.payload, reply.enabled, Some(reply.revision)) {
            Ok(()) => {
                self.revision = Some(reply.revision);
                self.refusals = 0;
                let sequence = plan.sequence;
                self.admitted = Some((reply.payload.to_vec(), plan));
                Outcome::Accepted { sequence, active: self.active() }
            }
            Err(presentation::Error::Revoked) => self.end(Teardown::Revoked),
            Err(error) => {
                if fresh { self.gate = None; }
                self.refuse(refusal(error))
            }
        }
    }

    pub fn pause_hint(&mut self, reasons: u32) -> Result<(), Teardown> {
        if let Some(reason) = self.ended { return Err(reason); }
        if bridge::validate_reasons(reasons).is_err() { self.end(Teardown::Protocol); return Err(Teardown::Protocol); }
        self.reasons = reasons;
        if let Some(gate) = &mut self.gate { gate.pause_hint(reasons).map_err(|_| Teardown::Protocol)?; }
        Ok(())
    }

    /// A new Shell layout: the admitted plan no longer applies; ask for a fresh one.
    pub fn layout(&mut self, serial: u32, monitors: Vec<Monitor>) -> Result<(), Teardown> {
        if let Some(reason) = self.ended { return Err(reason); }
        if serial <= self.serial || bridge::validate_layout(serial, &monitors).is_err() {
            self.end(Teardown::Protocol);
            return Err(Teardown::Protocol);
        }
        self.serial = serial;
        self.monitors = monitors.clone();
        self.admitted = None;
        if let Some(gate) = &mut self.gate && gate.layout_changed(serial, monitors).is_err() {
            self.end(Teardown::Protocol);
            return Err(Teardown::Protocol);
        }
        Ok(())
    }

    pub fn owner_lost(&mut self) -> Teardown {
        if let Some(gate) = &mut self.gate { gate.owner_lost(); }
        match self.end(Teardown::OwnerLost) { Outcome::Teardown(reason) => reason, _ => Teardown::OwnerLost }
    }
}

fn refusal(error: presentation::Error) -> Refusal {
    match error {
        presentation::Error::Identity => Refusal::Identity,
        presentation::Error::Consent | presentation::Error::Revoked => Refusal::Consent,
        presentation::Error::Generation => Refusal::Generation,
        presentation::Error::Replay => Refusal::Replay,
        presentation::Error::Layout => Refusal::Layout,
        presentation::Error::Geometry => Refusal::Geometry,
        presentation::Error::Media => Refusal::Media,
        presentation::Error::Wire => Refusal::Wire,
    }
}
