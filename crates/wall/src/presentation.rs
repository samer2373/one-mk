//! Fail-closed admission for the daemon-to-renderer presentation transport.
//! Caller credentials must come from the authenticated transport, never the plan.
//! This module performs no IPC, cache verification, decoding or authorization.

use crate::bridge::{self, Monitor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub unique_name: String,
    pub uid: u32,
    pub pid: u32,
}

/// A D-Bus unique connection name such as `:1.42` (never a well-known name).
pub fn valid_unique_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(':') else { return false };
    rest.len() <= 254 && rest.split('.').count() >= 2 && rest.split('.').all(|part| !part.is_empty() &&
        part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn edges(self) -> Result<(i64, i64), Error> {
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        if self.width == 0 || self.height == 0 ||
            [i64::from(self.x), i64::from(self.y), right, bottom].iter()
                .any(|edge| !(-1_000_000..=1_000_000).contains(edge)) {
            return Err(Error::Geometry);
        }
        Ok((right, bottom))
    }

    pub(crate) fn contains(self, other: Self) -> Result<bool, Error> {
        let (right, bottom) = self.edges()?;
        let (other_right, other_bottom) = other.edges()?;
        Ok(other.x >= self.x && other.y >= self.y && other_right <= right && other_bottom <= bottom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
    pub screen: String,
    pub canvas: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationPlan {
    pub generation: [u8; 16],
    pub consent_revision: u64,
    pub sequence: u64,
    pub layout_serial: u32,
    pub media_id: [u8; 32],
    pub canvas: Rect,
    pub slices: Vec<Slice>,
    pub looping: bool,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Identity, Consent, Generation, Replay, Layout, Geometry, Media, Revoked, Wire }

#[derive(Debug)]
pub struct Gate {
    owner: Owner,
    consent_revision: u64,
    generation: [u8; 16],
    canvas: Rect,
    serial: u32,
    monitors: Vec<Monitor>,
    sequence: u64,
    reasons: Option<u32>,
    suspended: bool,
    revoked: bool,
    active: Option<PresentationPlan>,
}

impl Gate {
    pub fn accept_encoded(&mut self, caller: &Owner, bytes: &[u8], enabled: bool,
        live_revision: Option<u64>) -> Result<(), Error> {
        if caller != &self.owner { return Err(Error::Identity); }
        self.reconcile_consent(enabled, live_revision);
        if self.revoked { return Err(Error::Revoked); }
        let plan = crate::presentation_wire::decode(bytes).map_err(|_| Error::Wire)?;
        self.accept(caller, plan)
    }

    pub fn new(owner: Owner, consent_revision: u64, generation: [u8; 16], canvas: Rect,
        serial: u32, monitors: Vec<Monitor>) -> Result<Self, Error> {
        if owner.pid == 0 || !valid_unique_name(&owner.unique_name) { return Err(Error::Identity); }
        if consent_revision == 0 { return Err(Error::Consent); }
        if generation == [0; 16] { return Err(Error::Generation); }
        canvas.edges()?;
        bridge::validate_layout(serial, &monitors).map_err(|_| Error::Layout)?;
        Ok(Self { owner, consent_revision, generation, canvas, serial, monitors,
            sequence: 0, reasons: None, suspended: false, revoked: false, active: None })
    }

    pub fn accept(&mut self, caller: &Owner, plan: PresentationPlan) -> Result<(), Error> {
        if self.revoked { return Err(Error::Revoked); }
        if caller != &self.owner { return Err(Error::Identity); }
        if plan.consent_revision != self.consent_revision { return Err(Error::Consent); }
        if plan.generation != self.generation { return Err(Error::Generation); }
        if plan.sequence <= self.sequence { return Err(Error::Replay); }
        if plan.layout_serial != self.serial || self.monitors.is_empty() ||
            plan.slices.len() != self.monitors.len() { return Err(Error::Layout); }
        if plan.media_id == [0; 32] { return Err(Error::Media); }
        if plan.canvas != self.canvas { return Err(Error::Geometry); }
        let mut translation = None;
        let mut screens = std::collections::BTreeSet::new();
        for (slice, &(local_x, local_y, width, height, _)) in plan.slices.iter().zip(&self.monitors) {
            if slice.screen.is_empty() || slice.screen.len() > 256 || slice.screen.chars().any(char::is_control)
                || !screens.insert(&slice.screen) { return Err(Error::Geometry); }
            slice.canvas.edges()?;
            // A paused plan presents nothing, so a local layout that does not
            // fit the committed canvas (an added or moved output) may still be
            // admitted paused: static until the layout fits again.
            if slice.canvas.width != width as u32 || slice.canvas.height != height as u32 ||
                (!plan.paused && !self.canvas.contains(slice.canvas)?) { return Err(Error::Geometry); }
            let offset = (i64::from(slice.canvas.x) - i64::from(local_x),
                i64::from(slice.canvas.y) - i64::from(local_y));
            if translation.is_some_and(|previous| previous != offset) { return Err(Error::Geometry); }
            translation = Some(offset);
        }
        for (index, slice) in plan.slices.iter().enumerate() {
            let (right, bottom) = slice.canvas.edges()?;
            for previous in &plan.slices[..index] {
                let (previous_right, previous_bottom) = previous.canvas.edges()?;
                if i64::from(slice.canvas.x) < previous_right && i64::from(previous.canvas.x) < right &&
                    i64::from(slice.canvas.y) < previous_bottom && i64::from(previous.canvas.y) < bottom {
                    return Err(Error::Geometry);
                }
            }
        }
        self.sequence = plan.sequence;
        self.active = Some(plan);
        Ok(())
    }

    pub fn active(&self) -> Option<&PresentationPlan> {
        self.active.as_ref().filter(|plan| !plan.paused && self.reasons == Some(0) && !self.suspended && !self.revoked)
    }

    pub fn pause_hint(&mut self, reasons: u32) -> Result<(), Error> {
        if bridge::validate_reasons(reasons).is_err() {
            self.revoke();
            return Err(Error::Layout);
        }
        self.reasons = Some(reasons);
        Ok(())
    }

    pub fn layout_changed(&mut self, serial: u32, monitors: Vec<Monitor>) -> Result<(), Error> {
        self.active = None;
        if serial <= self.serial || bridge::validate_layout(serial, &monitors).is_err() {
            self.revoke();
            return Err(Error::Layout);
        }
        self.serial = serial;
        self.monitors = monitors;
        Ok(())
    }

    pub fn suspend(&mut self) {
        self.suspended = true;
        self.active = None;
    }

    pub fn resume(&mut self) {
        self.suspended = false;
        self.active = None;
    }

    pub fn revoke(&mut self) {
        self.revoked = true;
        self.active = None;
    }

    pub fn reconcile_consent(&mut self, enabled: bool, revision: Option<u64>) {
        if !enabled || revision != Some(self.consent_revision) { self.revoke(); }
    }

    pub fn owner_lost(&mut self) { self.revoke(); }
}
