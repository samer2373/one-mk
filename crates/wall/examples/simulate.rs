//! Deterministic one-hour frame-level simulation; no OS clocks or I/O except CSV.
use one_mk_wall_sync::{Action, Controller, Estimator, MonotonicClock, Probe, Timeline};

struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}
struct Clock(f64);
impl MonotonicClock for Clock {
    fn now(&self) -> f64 { self.0 }
    fn generation(&self) -> u64 { 1 }
}
struct Computer {
    offset: f64,
    drift: f64,
    local: f64,
    estimator: Estimator,
    controller: Controller,
    position: f64,
    rate: f64,
    frame: f64,
    sequence: u64,
    pending: Vec<(f64, Vec<Probe>)>,
}

fn delay(model: &str, rng: &mut Rng, reverse: bool) -> f64 {
    match model {
        "wired" => 0.0001 + rng.unit() * 0.0002,
        "wifi" => 0.001 + rng.unit() * 0.004,
        "bad" => 0.001 + rng.unit() * 0.008 + if reverse { 0.002 + rng.unit() * 0.008 } else { 0.0 }
            + if rng.unit() < 0.12 { 0.030 + rng.unit() * 0.120 } else { 0.0 },
        _ => if reverse { 0.0 } else { 0.120 + rng.unit() * 0.030 },
    }
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

fn run(model: &str, interval: f64, limit: f64, ppm: f64, count: usize, seed: u64, window: usize) {
    let mut rng = Rng(seed);
    let timeline = Timeline::new(1000.0, 60.0, 0.0).expect("timeline");
    let mut computers: Vec<_> = (0..count).map(|index| {
        let offset = index as f64 * 2.7;
        let mut estimator = Estimator::with_window(1, 1, window).expect("window");
        if index == 0 {
            let identity: Vec<_> = (0..8).map(|sequence| Probe { session: 1, generation: 1, sequence, t1: 1000.0, t2: 1000.0, t3: 1000.0, t4: 1000.0 }).collect();
            estimator.observe(&identity).expect("coordinator identity mapping");
        }
        Computer { offset, drift: if index == 0 { 0.0 } else if index % 2 == 0 { ppm * 1e-6 } else { -ppm * 1e-6 }, local: 1000.0 + offset,
            estimator, controller: Controller::new(0.1, limit).expect("parameters"),
            position: 0.0, rate: 1.0, frame: 0.0, sequence: 0, pending: Vec::new() }
    }).collect();
    let mut errors = Vec::with_capacity(216000);
    let mut warm_errors = Vec::with_capacity(212400);
    let mut outage_max = 0.0f64;
    let (mut nudges, mut seeks, mut hard, mut jumps) = (0, 0, 0, 0);
    let mut frame_jumps = 0;
    let mut initial_convergence = None;
    let mut recovery = None;
    let mut initial_streak = 0;
    let mut recovery_streak = 0;
    let mut max_start = 0.0f64;
    for step in 0..216000 {
        let elapsed = step as f64 / 60.0;
        let real = 1000.0 + elapsed;
        let outage = (1800.0..1805.0).contains(&elapsed);
        for (index, computer) in computers.iter_mut().enumerate() {
            let previous = computer.position;
            let previous_frame = computer.frame;
            if step > 0 { computer.local += (1.0 + computer.drift) / 60.0; }
            if index > 0 && step % 60 == 0 {
                computer.drift = (computer.drift + (rng.unit() - 0.5) * 2e-6).clamp(-ppm * 1.5e-6, ppm * 1.5e-6);
            }
            computer.position = (computer.position + computer.rate * (1.0 + computer.drift) / 60.0).rem_euclid(60.0);
            if index > 0 && step % (interval * 60.0) as usize == 0 && !outage {
                let mut probes = Vec::new();
                let mut completion = real;
                for sample in 0..8 {
                    let send = real + sample as f64 * 0.02;
                    let up = delay(model, &mut rng, false) * if model == "adversarial" { (index + 1) as f64 } else { 1.0 };
                    let mut down = delay(model, &mut rng, true);
                    if model == "bad" && (elapsed + index as f64 * 7.0) % 45.0 < 2.0 {
                        down += 0.030 + rng.unit() * 0.120;
                    }
                    if model == "bad" && rng.unit() < 0.04 { continue; }
                    let local_at = |instant: f64| computer.local + (instant - real) * (1.0 + computer.drift);
                    probes.push(Probe { session: 1, generation: 1, sequence: computer.sequence,
                        t1: local_at(send), t2: send + up, t3: send + up + 0.0001, t4: local_at(send + up + down + 0.0001) });
                    computer.sequence += 1;
                    completion = completion.max(send + up + down + 0.0001);
                }
                computer.pending.push((completion, probes));
            }
            let mut pending_index = 0;
            while pending_index < computer.pending.len() {
                if computer.pending[pending_index].0 <= real {
                    let (_, probes) = computer.pending.remove(pending_index);
                    let _ = computer.estimator.observe(&probes);
                } else { pending_index += 1; }
            }
            if let Ok(action) = computer.controller.tick(&Clock(computer.local), &computer.estimator, &timeline, computer.position) {
                match action {
                    Action::OnTime => computer.rate = 1.0,
                    Action::Rate(rate) => { if (rate - computer.rate).abs() > 1e-9 { nudges += 1; } computer.rate = rate; },
                    Action::Seek(position) => { computer.position = position; computer.rate = 1.0; seeks += 1; },
                    Action::HardResync(position) => { computer.position = position; computer.rate = 1.0; hard += 1; },
                    Action::Wait => (),
                }
            }
            // Independent refresh phases, sampled source frames. This models
            // ideal timely decode, not compositor scheduling or GPU scanout.
            computer.frame = ((computer.position - index as f64 * 0.003 * computer.rate).rem_euclid(60.0) * 60.0).floor() / 60.0;
            let frame_advance = (computer.frame - previous_frame + 30.0).rem_euclid(60.0) - 30.0;
            if step > 0 && frame_advance.abs() > 1.0 / 60.0 + 1e-9 { frame_jumps += 1; }
            let advance = (computer.position - previous + 30.0).rem_euclid(60.0) - 30.0;
            // Count correction discontinuities, excluding normal one-frame
            // advance plus bounded rate adjustment, and excluding initial join.
            if step > 60 && (advance - computer.rate * (1.0 + computer.drift) / 60.0).abs() > 1.0 / 60.0 { jumps += 1; }
        }
        let mut error = 0.0f64;
        for left in &computers {
            for right in &computers {
                error = error.max(((left.frame - right.frame + 30.0).rem_euclid(60.0) - 30.0).abs());
            }
        }
        if elapsed < 30.0 { max_start = max_start.max(error); }
        if error <= 0.050 && computers.iter().all(|computer| computer.estimator.coordinator_time(computer.local).is_ok()) { initial_streak += 1; } else { initial_streak = 0; }
        if initial_convergence.is_none() && elapsed >= 1.0 && initial_streak >= 60 { initial_convergence = Some(elapsed - 59.0 / 60.0); }
        if elapsed >= 1805.0 {
            if error <= 0.050 { recovery_streak += 1; } else { recovery_streak = 0; }
            if recovery.is_none() && recovery_streak >= 60 { recovery = Some(elapsed - 1805.0 - 59.0 / 60.0); }
        }
        errors.push(error * 1000.0);
        if elapsed >= 60.0 { warm_errors.push(error * 1000.0); }
        if (1800.0..1805.0).contains(&elapsed) { outage_max = outage_max.max(error * 1000.0); }
    }
    errors.sort_by(f64::total_cmp);
    warm_errors.sort_by(f64::total_cmp);
    let mean = errors.iter().sum::<f64>() / errors.len() as f64;
    let offset_span = computers.iter().map(|computer| computer.offset).fold(0.0, f64::max);
    println!("{model},{interval},{limit},{ppm},{count},{seed},{mean:.4},{:.4},{:.4},{:.4},{:.4},{jumps},{nudges},{seeks},{hard},{:.3},{:.3},{:.3},{offset_span:.1},{window},{:.4},{outage_max:.4},{frame_jumps}",
        percentile(&errors, 0.5), percentile(&errors, 0.95), percentile(&errors, 0.99), percentile(&errors, 1.0), initial_convergence.unwrap_or(-1.0), recovery.unwrap_or(-1.0), max_start * 1000.0, percentile(&warm_errors, 0.95));
}

fn main() {
    println!("model,probe_s,nudge_limit,ppm,computers,seed,mean_ms,p50_ms,p95_ms,p99_ms,max_ms,correction_jumps,nudge_changes,seeks,hard_resyncs,convergence_s,recovery_s,start_max_ms,offset_span_s,filter_anchors,warm_p95_ms,outage_max_ms,display_frame_jumps");
    for model in ["wired", "wifi", "bad", "adversarial"] {
        for interval in [1.0, 5.0, 10.0, 15.0] {
            for limit in [0.005, 0.01, 0.02] {
                for ppm in [50.0, 200.0] {
                    for count in [2, 3, 4] {
                        for window in [8, 32] {
                            for seed in [42, 1337] { run(model, interval, limit, ppm, count, seed, window); }
                        }
                    }
                }
            }
        }
    }
}
