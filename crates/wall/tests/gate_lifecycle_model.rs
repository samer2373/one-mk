//! Random lifecycles of the presentation gate against a plain model of what must hold:
//! whatever order plans, pauses, layout changes, suspends, consent changes and owner loss come
//! in, nothing is presented that was not admitted, fresh, consented, unpaused and awake; a
//! refused plan changes nothing; and once the gate is revoked it never opens again.

use one_mk_wall_sync::presentation::{Error, Gate, Owner, PresentationPlan, Rect, Slice};
use one_mk_wall_sync::presentation_wire;

const CONSENT: u64 = 7;
const GENERATION: [u8; 16] = [1; 16];

fn owner() -> Owner { Owner { unique_name: ":1.42".into(), uid: 1000, pid: 1234 } }
fn canvas() -> Rect { Rect { x: -100, y: -50, width: 500, height: 200 } }
fn monitors() -> Vec<(i32, i32, i32, i32, f64)> { vec![(0, 0, 100, 100, 1.0), (120, 0, 100, 100, 1.25)] }

fn plan(sequence: u64, serial: u32, paused: bool) -> PresentationPlan {
    PresentationPlan { generation: GENERATION, consent_revision: CONSENT, sequence, layout_serial: serial, media_id: [2; 32], canvas: canvas(), slices: vec![
        Slice { screen: "left".into(), canvas: Rect { x: -100, y: -50, width: 100, height: 100 } },
        Slice { screen: "right".into(), canvas: Rect { x: 20, y: -50, width: 100, height: 100 } },
    ], looping: true, paused }
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
    fn below(&mut self, bound: u64) -> u64 { self.next() % bound }
}

#[derive(Default)]
struct Model { revoked: bool, suspended: bool, reasons: Option<u32>, active: Option<PresentationPlan>, sequence: u64, serial: u32 }

impl Model {
    fn expected(&self) -> Option<&PresentationPlan> {
        self.active.as_ref().filter(|plan| !plan.paused && self.reasons == Some(0) && !self.suspended && !self.revoked)
    }
    fn revoke(&mut self) { self.revoked = true; self.active = None; }
}

fn run(seed: u64) {
    let mut random = Random(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut gate = Gate::new(owner(), CONSENT, GENERATION, canvas(), 3, monitors()).unwrap();
    let mut model = Model { serial: 3, ..Default::default() };
    for step in 0..200 {
        let what;
        match random.below(14) {
            0 | 1 | 2 => {
                what = "a fresh plan";
                let next = plan(model.sequence + 1 + random.below(3), model.serial, random.below(4) == 0);
                let result = gate.accept(&owner(), next.clone());
                if model.revoked { assert_eq!(result, Err(Error::Revoked)); } else {
                    assert_eq!(result, Ok(()));
                    model.sequence = next.sequence;
                    model.active = Some(next);
                }
            }
            3 => {
                what = "the same or an older plan again";
                let stale = plan(random.below(model.sequence + 1), model.serial, false);
                let result = gate.accept(&owner(), stale);
                assert_eq!(result, Err(if model.revoked { Error::Revoked } else { Error::Replay }));
            }
            4 => {
                what = "a plan that is wrong in one way";
                let mut bad = plan(model.sequence + 1, model.serial, false);
                let expected = match random.below(7) {
                    0 => { bad.consent_revision += 1; Error::Consent }
                    1 => { bad.generation = [9; 16]; Error::Generation }
                    2 => { bad.layout_serial += 1; Error::Layout }
                    3 => { bad.media_id = [0; 32]; Error::Media }
                    4 => { bad.canvas.width += 1; Error::Geometry }
                    5 => { bad.slices.pop(); Error::Layout }
                    _ => { bad.slices[1].screen = "left".into(); Error::Geometry }
                };
                let result = gate.accept(&owner(), bad);
                assert_eq!(result, Err(if model.revoked { Error::Revoked } else { expected }));
            }
            5 => {
                what = "a plan from a stranger";
                let stranger = Owner { pid: 1235, ..owner() };
                let result = gate.accept(&stranger, plan(model.sequence + 1, model.serial, false));
                assert_eq!(result, Err(if model.revoked { Error::Revoked } else { Error::Identity }));
            }
            6 => {
                what = "pause reasons";
                let reasons = random.below(16) as u32 * u32::from(random.below(3) == 0);
                assert_eq!(gate.pause_hint(reasons), Ok(()));
                model.reasons = Some(reasons);
            }
            7 => {
                what = "pause reasons that do not exist";
                assert_eq!(gate.pause_hint(16 + random.below(1000) as u32), Err(Error::Layout));
                model.revoke();
            }
            8 => {
                what = "a new layout";
                let serial = model.serial + 1 + random.below(3) as u32;
                assert_eq!(gate.layout_changed(serial, monitors()), Ok(()));
                model.serial = serial;
                model.active = None;
            }
            9 => {
                what = "a layout that is not newer";
                let serial = random.below(u64::from(model.serial) + 1) as u32;
                assert_eq!(gate.layout_changed(serial, monitors()), Err(Error::Layout));
                model.revoke();
            }
            10 => {
                what = if random.below(2) == 0 { "suspend" } else { "resume" };
                if what == "suspend" { gate.suspend(); model.suspended = true; } else { gate.resume(); model.suspended = false; }
                model.active = None;
            }
            11 => {
                what = "consent changing";
                let (enabled, revision) = match random.below(4) { 0 => (false, Some(CONSENT)), 1 => (true, Some(CONSENT + 1)), 2 => (true, None), _ => (true, Some(CONSENT)) };
                gate.reconcile_consent(enabled, revision);
                if !enabled || revision != Some(CONSENT) { model.revoke(); }
            }
            12 => {
                what = "the owner leaving";
                if random.below(6) == 0 { gate.owner_lost(); model.revoke(); }
            }
            _ => {
                what = "an encoded plan";
                let sequence = model.sequence + 1;
                let next = plan(sequence, model.serial, false);
                let garbage = random.below(3) == 0;
                let bytes = if garbage { (0..random.below(200)).map(|_| random.next() as u8).collect() } else { presentation_wire::encode(&next).unwrap() };
                let result = gate.accept_encoded(&owner(), &bytes, true, Some(CONSENT));
                if model.revoked { assert_eq!(result, Err(Error::Revoked)); }
                else if garbage { assert_eq!(result, Err(Error::Wire)); }
                else { assert_eq!(result, Ok(())); model.sequence = sequence; model.active = Some(next); }
            }
        }
        assert_eq!(gate.active(), model.expected(), "seed {seed}, step {step}, after {what}");
    }
    if !model.revoked {
        // A refused plan consumed nothing: the next sequence is still the next.
        assert_eq!(gate.accept(&owner(), plan(model.sequence + 1, model.serial, false)), Ok(()), "seed {seed}");
    } else {
        for _ in 0..3 {
            assert_eq!(gate.accept(&owner(), plan(model.sequence + 1, model.serial, false)), Err(Error::Revoked), "seed {seed}");
            assert!(gate.active().is_none());
            gate.resume();
            assert!(gate.pause_hint(0).is_ok());
            assert!(gate.active().is_none(), "a revoked gate must not reopen by resume or pause hints, seed {seed}");
        }
    }
}

#[test]
fn random_lifecycles_agree_with_the_model() {
    for seed in 1..=400 { run(seed); }
}

#[test]
fn encoded_plans_that_are_valid_but_for_another_owner_or_without_consent_never_reach_the_gate() {
    let mut gate = Gate::new(owner(), CONSENT, GENERATION, canvas(), 3, monitors()).unwrap();
    gate.pause_hint(0).unwrap();
    let bytes = presentation_wire::encode(&plan(1, 3, false)).unwrap();
    let stranger = Owner { uid: 0, ..owner() };
    assert_eq!(gate.accept_encoded(&stranger, &bytes, true, Some(CONSENT)), Err(Error::Identity));
    assert!(gate.active().is_none());
    // No live consent (switched off, or a different revision) revokes before the bytes are even decoded.
    assert_eq!(gate.accept_encoded(&owner(), &bytes, false, Some(CONSENT)), Err(Error::Revoked));
    assert_eq!(gate.accept_encoded(&owner(), &bytes, true, Some(CONSENT)), Err(Error::Revoked), "consent revocation is terminal");
}
