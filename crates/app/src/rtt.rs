//! How each connection is doing, as the window shows it: a short history of
//! round-trip times per computer, worked out without drawing anything.
//!
//! The daemon reads the numbers from QUIC when asked; the window asks while
//! a graph is on screen and keeps what it was told here, in memory only.
//! Samples sit on a time axis (the right edge is now), so a stretch when
//! nobody looked shows as a gap, not as a flat line.

use one_mk_ctl::status::LinkStats;
use std::collections::{HashMap, VecDeque};

/// Samples kept per computer.
pub const CAPACITY: usize = 120;
/// Seconds between samples while a graph is on screen.
pub const EVERY: f64 = 1.0;
/// The graph's width in time: [`CAPACITY`] samples.
pub const SPAN: f64 = CAPACITY as f64 * EVERY;
/// A reading closer than this to the one before is the same look twice.
const MIN_GAP: f64 = EVERY * 0.8;
/// Samples further apart than this are not joined by a line.
const BREAK: f64 = EVERY * 2.5;
/// Fewer samples than this say nothing yet.
pub const MIN_SAMPLES: usize = 3;
/// The 95th percentile above this is "Some stalls" (a mouse move late).
pub const STALL_US: u32 = LinkStats::STALL_US;
/// ... and above this "Slow".
pub const SLOW_US: u32 = LinkStats::SLOW_US;
/// Packets lost within the history that make it "Slow" on their own.
pub const SLOW_LOSS: u32 = 8;
/// The graph is at least this tall in time, so a steady 4 ms stays flat.
const FLOOR_US: u32 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    at: f64,
    rtt_us: u32,
    /// Packets lost since the sample before.
    lost: u32,
}

/// One computer's recent round trips.
#[derive(Clone, Debug, Default)]
pub struct Ring {
    samples: VecDeque<Sample>,
    /// The daemon's running count of lost packets at the last sample.
    lost_seen: Option<u64>,
}

impl Ring {
    /// Adds what the daemon said at `now` (seconds on any steady clock).
    pub fn push(&mut self, now: f64, link: LinkStats) {
        if !now.is_finite() || self.samples.back().is_some_and(|s| now - s.at < MIN_GAP) {
            return;
        }
        let lost = match self.lost_seen {
            Some(before) if link.lost >= before => u32::try_from(link.lost - before).unwrap_or(u32::MAX),
            // The count went down: another connection. Nothing before it counts.
            Some(_) => {
                self.samples.clear();
                0
            }
            // The first look has nothing to compare with.
            None => 0,
        };
        self.lost_seen = Some(link.lost);
        while self.samples.front().is_some_and(|s| now - s.at > SPAN) || self.samples.len() >= CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample { at: now, rtt_us: link.rtt_us, lost });
    }

    fn recent(&self, now: f64) -> impl Iterator<Item = &Sample> {
        self.samples.iter().filter(move |s| now - s.at <= SPAN && s.at <= now)
    }

    /// The numbers over the history, or `None` with nothing in it.
    pub fn summary(&self, now: f64) -> Option<Summary> {
        let latest = self.recent(now).last()?;
        let mut times: Vec<u32> = self.recent(now).map(|s| s.rtt_us).collect();
        times.sort_unstable();
        let n = times.len();
        let sum: u64 = times.iter().map(|&t| u64::from(t)).sum();
        Some(Summary {
            count: n,
            now_us: latest.rtt_us,
            min_us: times[0],
            avg_us: (sum / n as u64) as u32,
            max_us: times[n - 1],
            // Nearest rank: the smallest time that 95 % of the samples do not exceed.
            p95_us: times[(n * 95).div_ceil(100).saturating_sub(1).min(n - 1)],
            lost: self.recent(now).map(|s| s.lost).fold(0u32, u32::saturating_add),
        })
    }

    /// The line to draw in a `w` × `h` box (origin top left, newest at the
    /// right edge), in as many pieces as there were gaps, and where
    /// packets were lost.
    pub fn trace(&self, now: f64, w: f32, h: f32) -> Trace {
        let mut trace = Trace::default();
        let top = self.recent(now).map(|s| s.rtt_us).max().map_or(FLOOR_US, |m| m.max(FLOOR_US));
        let mut last_at = f64::NEG_INFINITY;
        for s in self.recent(now) {
            let x = (w as f64 * (1.0 - (now - s.at) / SPAN)).clamp(0.0, w as f64) as f32;
            let y = h * (1.0 - s.rtt_us as f32 / top as f32).clamp(0.0, 1.0);
            if s.at - last_at > BREAK {
                trace.lines.push(Vec::new());
            }
            if let Some(line) = trace.lines.last_mut() {
                line.push([x, y]);
            }
            if s.lost > 0 {
                trace.losses.push(x);
            }
            last_at = s.at;
        }
        trace.top_us = top;
        trace
    }
}

/// What [`Ring::trace`] works out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    pub lines: Vec<Vec<[f32; 2]>>,
    /// Where along the width packets were lost.
    pub losses: Vec<f32>,
    /// What the full height means.
    pub top_us: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub count: usize,
    pub now_us: u32,
    pub min_us: u32,
    pub avg_us: u32,
    pub max_us: u32,
    pub p95_us: u32,
    /// Packets lost within the history.
    pub lost: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Smooth,
    Stalls,
    Slow,
}

impl Quality {
    pub fn word(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Stalls => "Some stalls",
            Self::Slow => "Slow",
        }
    }
}

impl Summary {
    /// `None` until there is enough to judge by.
    pub fn quality(&self) -> Option<Quality> {
        if self.count < MIN_SAMPLES {
            return None;
        }
        Some(if self.p95_us > SLOW_US || self.lost >= SLOW_LOSS {
            Quality::Slow
        } else if self.p95_us > STALL_US || self.lost > 0 {
            Quality::Stalls
        } else {
            Quality::Smooth
        })
    }

    /// "Latency to desk: 4 ms, smooth. Over the last 2 minutes: lowest 3 ms, average 4 ms, highest 9 ms."
    pub fn describe(&self, who: &str) -> String {
        let Some(q) = self.quality() else {
            return format!("Latency to {who}: {}, still measuring.", ms(self.now_us));
        };
        let mut out = format!("Latency to {who}: {}, {}.", ms(self.now_us), q.word().to_lowercase());
        out += &format!(" Over the last {}: lowest {}, average {}, highest {}.", span_words(), ms(self.min_us), ms(self.avg_us), ms(self.max_us));
        match self.lost {
            0 => {}
            1 => out += " 1 packet was lost.",
            n => out += &format!(" {n} packets were lost."),
        }
        out
    }
}

/// "2 minutes".
pub fn span_words() -> String {
    format!("{} minutes", SPAN as u32 / 60)
}

/// 4200 -> "4 ms", 400 -> "<1 ms".
pub fn ms(us: u32) -> String {
    match us {
        0..=999 => "<1 ms".to_owned(),
        us => format!("{} ms", us.saturating_add(500) / 1000),
    }
}

/// The number alone, for a spot with "ms" elsewhere: "4", "<1".
pub fn ms_number(us: u32) -> String {
    if us < 1000 { "<1".to_owned() } else { (us.saturating_add(500) / 1000).to_string() }
}

/// The history of every connected computer, by id.
#[derive(Clone, Debug, Default)]
pub struct Rtts {
    rings: HashMap<String, Ring>,
}

impl Rtts {
    /// Takes what the daemon said at `now`. A computer that has no link
    /// (offline) loses its history: a graph of a connection that is gone
    /// would only mislead.
    pub fn record(&mut self, now: f64, links: &[(String, LinkStats)]) {
        self.rings.retain(|id, _| links.iter().any(|(l, _)| l == id));
        for (id, link) in links {
            self.rings.entry(id.clone()).or_default().push(now, *link);
        }
    }

    pub fn of(&self, id: &str) -> Option<&Ring> {
        self.rings.get(id)
    }

    /// Replaces a computer's history (the screenshot hook).
    #[cfg(debug_assertions)]
    pub fn set(&mut self, id: &str, ring: Ring) {
        self.rings.insert(id.to_owned(), ring);
    }
}

/// A history from `ms,ms,+loss` text, one second apart, ending at `now`
/// (the screenshot hook): `4,5,6+2` is three samples, the last 6 ms with
/// 2 packets lost since the one before.
#[cfg(debug_assertions)]
pub fn scripted(now: f64, text: &str) -> Ring {
    let items: Vec<&str> = text.split(',').filter(|s| !s.is_empty()).collect();
    let mut ring = Ring::default();
    let mut lost = 0u64;
    for (k, item) in items.iter().enumerate() {
        let (t, l) = item.split_once('+').unwrap_or((item, "0"));
        lost += l.parse::<u64>().unwrap_or(0);
        let at = now - (items.len() - 1 - k) as f64 * EVERY;
        ring.push(at, LinkStats { rtt_us: (t.parse::<f32>().unwrap_or(0.0) * 1000.0) as u32, lost, sent: 0 });
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(ms: f32, lost: u64) -> LinkStats {
        LinkStats { rtt_us: (ms * 1000.0) as u32, lost, sent: 0 }
    }

    fn ring(times: &[f32]) -> (Ring, f64) {
        let mut r = Ring::default();
        for (k, t) in times.iter().enumerate() {
            r.push(k as f64 * EVERY, link(*t, 0));
        }
        (r, times.len().saturating_sub(1) as f64 * EVERY)
    }

    #[test]
    fn the_numbers_over_the_history() {
        let (r, now) = ring(&[4.0, 5.0, 3.0, 9.0, 4.0]);
        let s = r.summary(now).unwrap();
        assert_eq!((s.count, s.now_us, s.min_us, s.avg_us, s.max_us), (5, 4000, 3000, 5000, 9000));
        // Nearest rank: 95 % of five samples is the fifth.
        assert_eq!(s.p95_us, 9000);
        assert_eq!(s.quality(), Some(Quality::Smooth));
        assert!(Ring::default().summary(0.0).is_none());
    }

    #[test]
    fn p95_ignores_one_stray_in_a_hundred() {
        let mut times = vec![4.0; 99];
        times.push(300.0);
        let (r, now) = ring(&times);
        let s = r.summary(now).unwrap();
        // 95 % of 100 samples is the 95th: the one slow sample doesn't count.
        assert_eq!((s.p95_us, s.max_us), (4000, 300_000));
        assert_eq!(s.quality(), Some(Quality::Smooth));
        // Six in a hundred do.
        times[10..15].fill(300.0);
        let (r, now) = ring(&times);
        assert_eq!(r.summary(now).unwrap().quality(), Some(Quality::Slow));
    }

    #[test]
    fn thresholds_pick_the_word() {
        let q = |times: &[f32]| {
            let (r, now) = ring(times);
            r.summary(now).unwrap().quality()
        };
        assert_eq!(q(&[4.0, 4.0]), None, "two samples say nothing yet");
        assert_eq!(q(&[4.0, 4.0, 4.0]), Some(Quality::Smooth));
        assert_eq!(q(&[40.0; 5]), Some(Quality::Smooth), "40 ms is still fine");
        assert_eq!(q(&[40.1; 5]), Some(Quality::Stalls));
        assert_eq!(q(&[100.0; 5]), Some(Quality::Stalls));
        assert_eq!(q(&[100.1; 5]), Some(Quality::Slow));
        assert_eq!(Quality::Smooth.word(), "Smooth");
        assert_eq!(Quality::Stalls.word(), "Some stalls");
        assert_eq!(Quality::Slow.word(), "Slow");
    }

    #[test]
    fn any_loss_shows_and_a_lot_is_slow() {
        let mut r = Ring::default();
        for k in 0..10 {
            r.push(k as f64, link(4.0, if k >= 5 { 1 } else { 0 }));
        }
        // The first look counts nothing; one lost packet is "some stalls".
        let s = r.summary(9.0).unwrap();
        assert_eq!((s.lost, s.quality()), (1, Some(Quality::Stalls)));
        let mut r = Ring::default();
        for k in 0..10 {
            r.push(k as f64, link(4.0, k as u64 * 2));
        }
        let s = r.summary(9.0).unwrap();
        assert_eq!((s.lost, s.quality()), (18, Some(Quality::Slow)));
        // Loss the window started out knowing about (a long-lived connection) is not counted.
        let mut r = Ring::default();
        for k in 0..10 {
            r.push(k as f64, link(4.0, 5000));
        }
        assert_eq!(r.summary(9.0).unwrap().lost, 0);
    }

    #[test]
    fn a_new_connection_starts_the_history_again() {
        let mut r = Ring::default();
        for k in 0..5 {
            r.push(k as f64, link(80.0, 40));
        }
        // The count fell: not the same connection, and its past is not this one's.
        for k in 5..8 {
            r.push(k as f64, link(4.0, 0));
        }
        let s = r.summary(7.0).unwrap();
        assert_eq!((s.count, s.max_us, s.lost), (3, 4000, 0));
    }

    #[test]
    fn old_samples_fall_off_and_the_ring_stays_small() {
        let mut r = Ring::default();
        for k in 0..1000 {
            r.push(k as f64 * EVERY, link(4.0, 0));
        }
        assert!(r.samples.len() <= CAPACITY, "{}", r.samples.len());
        // Looking later, with nothing new: what is older than the span is gone.
        assert_eq!(r.summary(999.0 + SPAN + 1.0).map(|s| s.count), None);
        assert!(r.summary(999.0 + SPAN - 10.0).is_some());
        // Looks closer than a second apart are one look.
        let mut r = Ring::default();
        for k in 0..20 {
            r.push(k as f64 * 0.25, link(4.0, 0));
        }
        assert_eq!(r.samples.len(), 5);
    }

    #[test]
    fn the_line_runs_to_the_right_edge_and_breaks_at_gaps() {
        let mut r = Ring::default();
        for k in 0..4 {
            r.push(k as f64, link(10.0, 0));
        }
        // The window was elsewhere for twenty seconds.
        for k in 24..28 {
            r.push(k as f64, link(40.0, if k >= 26 { 3 } else { 0 }));
        }
        let t = r.trace(27.0, 120.0, 50.0);
        assert_eq!(t.lines.len(), 2, "a gap is a break: {t:?}");
        let last = *t.lines[1].last().unwrap();
        assert_eq!(last[0], 120.0, "newest sample at the right edge");
        // 40 ms is the top of a graph whose tallest sample it is.
        assert_eq!(t.top_us, 40_000);
        assert_eq!(last[1], 0.0);
        // 10 ms is a quarter of the way up.
        assert_eq!(t.lines[0][0][1], 37.5);
        assert_eq!(t.losses.len(), 1);
        // A steady 4 ms stays low on the graph instead of filling it.
        let (r, now) = ring(&[4.0, 4.0, 4.0]);
        let t = r.trace(now, 100.0, 40.0);
        assert_eq!(t.top_us, FLOOR_US);
        assert!(t.lines[0].iter().all(|p| p[1] == 32.0), "{t:?}");
    }

    /// Whatever the daemon says, the trace stays in its box and nothing panics.
    #[test]
    fn hostile_input_never_draws_outside_or_panics() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let mut r = Ring::default();
            let mut now = 0.0f64;
            for _ in 0..(next() % 400) {
                now += [0.0, 0.3, 1.0, 1.0, 1.0, 3.0, 40.0, -5.0, f64::MAX][(next() % 9) as usize];
                let rtt_us = [0, 1, 999, 4_000, 60_000_000, u32::MAX][(next() % 6) as usize];
                let lost = [0, 1, 7, u64::MAX, next()][(next() % 5) as usize];
                r.push(now, LinkStats { rtt_us, lost, sent: next() });
                assert!(r.samples.len() <= CAPACITY);
            }
            for at in [now, now + 5.0, now - 1000.0, 0.0, f64::MAX, f64::NAN] {
                let (w, h) = (96.0, 40.0);
                let t = r.trace(at, w, h);
                for p in t.lines.iter().flatten() {
                    assert!((0.0..=w).contains(&p[0]) && (0.0..=h).contains(&p[1]), "{p:?}");
                }
                assert!(t.losses.iter().all(|x| (0.0..=w).contains(x)));
                if let Some(s) = r.summary(at) {
                    assert!(s.min_us <= s.avg_us && s.avg_us <= s.max_us && s.p95_us <= s.max_us && s.p95_us >= s.min_us);
                    let _ = (s.quality(), s.describe("x"));
                }
            }
        }
    }

    #[test]
    fn words_for_screen_readers() {
        let (r, now) = ring(&[4.0, 4.0, 5.0, 9.0, 3.0]);
        let s = r.summary(now).unwrap();
        assert_eq!(
            s.describe("desk"),
            "Latency to desk: 3 ms, smooth. Over the last 2 minutes: lowest 3 ms, average 5 ms, highest 9 ms."
        );
        let (r, now) = ring(&[4.0, 4.0]);
        assert_eq!(r.summary(now).unwrap().describe("desk"), "Latency to desk: 4 ms, still measuring.");
        let mut r = Ring::default();
        for k in 0..6 {
            r.push(k as f64, link(60.0, if k >= 3 { 2 } else { 0 }));
        }
        let text = r.summary(5.0).unwrap().describe("MacBook Air");
        assert!(text.starts_with("Latency to MacBook Air: 60 ms, some stalls.") && text.ends_with("2 packets were lost."), "{text}");
        assert_eq!((ms(0), ms(999), ms(1000), ms(4499), ms(4500)), ("<1 ms".into(), "<1 ms".into(), "1 ms".into(), "4 ms".into(), "5 ms".into()));
        assert_eq!((ms_number(400), ms_number(12_600)), ("<1".to_owned(), "13".to_owned()));
    }

    #[test]
    fn an_offline_computer_keeps_no_history() {
        let mut all = Rtts::default();
        let two = [("a".to_owned(), link(4.0, 0)), ("b".to_owned(), link(5.0, 0))];
        for k in 0..4 {
            all.record(k as f64, &two);
        }
        assert!(all.of("a").is_some() && all.of("b").is_some());
        // b drops off: its graph goes with it, and a back-again b starts empty.
        all.record(4.0, &two[..1]);
        assert!(all.of("b").is_none() && all.of("a").is_some());
        all.record(5.0, &two);
        assert_eq!(all.of("b").unwrap().summary(5.0).unwrap().count, 1);
        all.record(6.0, &[]);
        assert!(all.of("a").is_none());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_screenshot_script_makes_histories() {
        let r = scripted(100.0, "4,5,6+2");
        let s = r.summary(100.0).unwrap();
        assert_eq!((s.count, s.now_us, s.lost), (3, 6000, 2));
        assert!(scripted(0.0, "").summary(0.0).is_none());
        assert!(scripted(0.0, "x,,+").summary(0.0).is_some());
    }
}
