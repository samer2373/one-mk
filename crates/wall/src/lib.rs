//! Experimental, bounded monotonic clock mapping. All times are seconds since
//! a process-local monotonic origin, never UTC. The caller owns authentication,
//! outstanding-request matching, scheduling, sleep notifications and rendering.
//! Path asymmetry and a consistently dishonest coordinator are not identifiable
//! from four timestamps. `uncertainty` is a delay bound, not a security guarantee.

use std::collections::VecDeque;

const MAX_TIME: f64 = 1.0e9;
const MAX_OFFSET: f64 = 86400.0;
const MAX_DRIFT: f64 = 500.0e-6;

/// Injectable clock; implementations must use a monotonic origin and advance
/// generation when their time domain changes (including suspend/resume).
pub trait MonotonicClock {
    fn now(&self) -> f64;
    fn generation(&self) -> u64;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidTime,
    InvalidOrder,
    Stale,
    Empty,
    Bounds,
}

fn time(value: f64) -> Result<f64, Error> {
    if value.is_finite() && (0.0..=MAX_TIME).contains(&value) {
        Ok(value)
    } else {
        Err(Error::InvalidTime)
    }
}

/// A matched response. Generation identifies the committed authority/time
/// domain; sequence numbers must be unique within that generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    pub session: u64,
    pub generation: u64,
    pub sequence: u64,
    pub t1: f64,
    pub t2: f64,
    pub t3: f64,
    pub t4: f64,
}

impl Probe {
    pub fn measurement(&self) -> Result<(f64, f64, f64), Error> {
        for value in [self.t1, self.t2, self.t3, self.t4] {
            time(value)?;
        }
        let elapsed = self.t4 - self.t1;
        let processing = self.t3 - self.t2;
        let rtt = elapsed - processing;
        if !(0.0..=2.0).contains(&elapsed) || processing < 0.0 || rtt < 0.0 {
            return Err(Error::InvalidOrder);
        }
        let offset = ((self.t2 - self.t1) + (self.t3 - self.t4)) * 0.5;
        if offset.abs() > MAX_OFFSET {
            return Err(Error::Bounds);
        }
        Ok(((self.t1 + self.t4) * 0.5, offset, rtt))
    }
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    local: f64,
    offset: f64,
}

/// Affine coordinator-minus-local map. At most 32 burst anchors retained;
/// bursts are capped at 64 replies. No work or I/O happens between calls.
#[derive(Debug)]
pub struct Estimator {
    session: u64,
    generation: u64,
    sequence: Option<u64>,
    history: VecDeque<Anchor>,
    anchor: Option<Anchor>,
    drift: f64,
    uncertainty: f64,
    window: usize,
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

impl Estimator {
    pub fn new(session: u64, generation: u64) -> Self {
        Self { session, generation, sequence: None, history: VecDeque::new(), anchor: None, drift: 0.0, uncertainty: 2.0, window: 32 }
    }

    pub fn with_window(session: u64, generation: u64, window: usize) -> Result<Self, Error> {
        if !(4..=32).contains(&window) { return Err(Error::Bounds); }
        Ok(Self { window, ..Self::new(session, generation) })
    }

    /// Explicit owner-approved coordinator change or resume, not an election.
    pub fn reset(&mut self, session: u64, generation: u64) {
        *self = Self { window: self.window, ..Self::new(session, generation) };
    }

    /// Replies may arrive out of order within a burst. Old bursts, duplicated
    /// sequence IDs, invalid observations and wrong generations cannot vote.
    /// Bad input never partially updates the map. At least three votes required.
    pub fn observe(&mut self, probes: &[Probe]) -> Result<(), Error> {
        if probes.len() > 64 {
            return Err(Error::Bounds);
        }
        let mut ordered = probes.to_vec();
        ordered.retain(|probe| probe.session == self.session && probe.generation == self.generation
            && self.sequence.is_none_or(|seq| probe.sequence > seq));
        ordered.sort_by_key(|probe| probe.sequence);
        ordered.dedup_by_key(|probe| probe.sequence);
        let mut samples: Vec<_> = ordered.iter().filter_map(|probe| probe.measurement().ok().map(|measurement| (measurement, probe.sequence))).collect();
        if samples.len() < 3 {
            return Err(Error::Empty);
        }
        let newest_sequence = samples.iter().map(|sample| sample.1).max().ok_or(Error::Empty)?;
        samples.sort_by(|left, right| left.0.2.total_cmp(&right.0.2));
        samples.truncate(samples.len().div_ceil(2).max(3));
        let mut offsets: Vec<_> = samples.iter().map(|sample| sample.0.1).collect();
        let center = median(&mut offsets);
        let mut deviations: Vec<_> = offsets.iter().map(|offset| (offset - center).abs()).collect();
        let tolerance = (median(&mut deviations) * 3.0).max(0.0005);
        samples.retain(|sample| (sample.0.1 - center).abs() <= tolerance);
        if samples.len() < 3 {
            return Err(Error::Empty);
        }
        let local = samples.iter().map(|sample| sample.0.0).sum::<f64>() / samples.len() as f64;
        let offset = samples.iter().map(|sample| sample.0.1).sum::<f64>() / samples.len() as f64;
        if self.anchor.is_some_and(|anchor| local <= anchor.local) {
            return Err(Error::Stale);
        }
        let measured = Anchor { local, offset };
        let mut slopes: Vec<_> = self.history.iter().filter(|anchor| local - anchor.local >= 10.0)
            .map(|anchor| (offset - anchor.offset) / (local - anchor.local)).collect();
        if !slopes.is_empty() {
            let desired = median(&mut slopes).clamp(-MAX_DRIFT, MAX_DRIFT);
            self.drift += (desired - self.drift).clamp(-20.0e-6, 20.0e-6);
        }
        let previous = self.anchor.map(|anchor| anchor.offset + self.drift * (local - anchor.local));
        self.anchor = Some(Anchor { local, offset: previous.map_or(offset, |old| old + (offset - old).clamp(-0.005, 0.005)) });
        self.uncertainty = samples.iter().map(|sample| sample.0.2 * 0.5 + (sample.0.1 - offset).abs()).fold(0.0, f64::max);
        self.history.push_back(measured);
        if self.history.len() > self.window {
            self.history.pop_front();
        }
        self.sequence = Some(newest_sequence);
        Ok(())
    }

    pub fn coordinator_time(&self, local: f64) -> Result<f64, Error> {
        time(local)?;
        let anchor = self.anchor.ok_or(Error::Empty)?;
        if local < anchor.local {
            return Err(Error::Stale);
        }
        Ok(local + anchor.offset + self.drift * (local - anchor.local))
    }

    pub fn uncertainty(&self, local: f64) -> Result<f64, Error> {
        self.coordinator_time(local)?;
        let anchor = self.anchor.ok_or(Error::Empty)?;
        Ok(self.uncertainty + (local - anchor.local) * MAX_DRIFT)
    }

    pub fn drift_ppm(&self) -> f64 {
        self.drift * 1.0e6
    }
}

/// Validated committed looping timeline; future epochs return `None` until
/// the scheduled start. A shared pause needs a new generation/epoch from owner.
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    epoch: f64,
    duration: f64,
    start: f64,
}

impl Timeline {
    pub fn new(epoch: f64, duration: f64, start: f64) -> Result<Self, Error> {
        time(epoch)?;
        if !duration.is_finite() || !(0.001..=600.0).contains(&duration) || !start.is_finite() || !(0.0..duration).contains(&start) {
            return Err(Error::Bounds);
        }
        Ok(Self { epoch, duration, start })
    }

    pub fn position(&self, coordinator: f64) -> Result<Option<f64>, Error> {
        time(coordinator)?;
        Ok((coordinator >= self.epoch).then(|| (coordinator - self.epoch + self.start).rem_euclid(self.duration)))
    }

    pub fn error(&self, target: f64, presented: f64) -> Result<f64, Error> {
        if !target.is_finite() || !presented.is_finite() || !(0.0..self.duration).contains(&target) || !(0.0..self.duration).contains(&presented) {
            return Err(Error::Bounds);
        }
        Ok((target - presented + self.duration * 0.5).rem_euclid(self.duration) - self.duration * 0.5)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    OnTime,
    Rate(f64),
    Seek(f64),
    HardResync(f64),
    Wait,
}

/// Renderer-owned cadence and hysteresis. Evaluate at 100 ms by default;
/// 8 ms entry / 3 ms exit deadband; max 2% rate; >80 ms boundary seek;
/// Errors above 250 ms or gaps above 2 seconds deliberately hard-resync.
/// Controller is not a decoder: adapters discard obsolete PTS, never slow the
/// timeline when decode cannot keep up. Reset after suspend/authority change.
#[derive(Debug)]
pub struct Controller {
    last: Option<f64>,
    last_resync: Option<f64>,
    correcting: bool,
    initialized: bool,
    interval: f64,
    limit: f64,
}

impl Controller {
    pub fn new(interval: f64, limit: f64) -> Result<Self, Error> {
        if !interval.is_finite() || !(0.01..=1.0).contains(&interval) || !limit.is_finite() || !(0.001..=0.02).contains(&limit) {
            return Err(Error::Bounds);
        }
        Ok(Self { last: None, last_resync: None, correcting: false, initialized: false, interval, limit })
    }

    pub fn reset(&mut self) {
        self.last = None;
        self.last_resync = None;
        self.correcting = false;
        self.initialized = false;
    }

    pub fn tick<C: MonotonicClock>(&mut self, clock: &C, estimator: &Estimator, timeline: &Timeline, presented: f64) -> Result<Action, Error> {
        let now = time(clock.now())?;
        if clock.generation() != estimator.generation {
            return Err(Error::Stale);
        }
        let Some(target) = timeline.position(estimator.coordinator_time(now)?)? else { return Ok(Action::Wait) };
        let error = timeline.error(target, presented)?;
        if self.last.is_some_and(|last| now < last) {
            return Err(Error::Stale);
        }
        if self.last.is_some_and(|last| now - last < self.interval) {
            return Ok(Action::Wait);
        }
        let interrupted = self.last.is_some_and(|last| now - last > 2.0);
        self.last = Some(now);
        let can_resync = self.last_resync.is_none_or(|last| now - last >= 5.0);
        if !self.initialized || interrupted || (error.abs() > 0.250 && can_resync) {
            self.initialized = true;
            self.correcting = false;
            self.last_resync = Some(now);
            return Ok(Action::HardResync(target));
        }
        if error.abs() > 0.080 && target < self.interval * 1.1 && can_resync {
            self.correcting = false;
            self.last_resync = Some(now);
            return Ok(Action::Seek(target));
        }
        if error.abs() < 0.003 || (!self.correcting && error.abs() <= 0.008) {
            self.correcting = false;
            return Ok(Action::OnTime);
        }
        self.correcting = true;
        Ok(Action::Rate(1.0 + (error / 4.0).clamp(-self.limit, self.limit)))
    }
}

/// Latest eligible unwrapped PTS in a bounded queue. Future frames are retained by the
/// adapter; stale frames are dropped, not displayed in FIFO order to catch up.
pub fn frame_for_target(frames: &[f64], target: f64) -> Result<Option<usize>, Error> {
    time(target)?;
    if frames.len() > 256 {
        return Err(Error::Bounds);
    }
    let mut previous = None;
    let mut selected = None;
    for (index, &pts) in frames.iter().enumerate() {
        time(pts)?;
        if previous.is_some_and(|last| pts < last) {
            return Err(Error::InvalidOrder);
        }
        previous = Some(pts);
        if pts <= target {
            selected = Some(index);
        }
    }
    Ok(selected)
}

pub mod wire;
pub mod bridge;
pub mod presentation;
pub mod presentation_wire;
pub mod presentation_auth;
