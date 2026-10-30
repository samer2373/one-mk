//! EXPERIMENTAL macOS adapter logic. Native execution is UNTESTED.

use crate::macos_logic::{self as logic, flags};
use one_mk_proto::HeldKeys;
use std::sync::atomic::{AtomicU32, Ordering};

pub const TAG: u64 = 0x6d6b_6c6f_0000_0000;

#[derive(Default)]
pub struct Receipt {
    expected: AtomicU32,
    seen: AtomicU32,
}

impl Receipt {
    pub fn arm(&self, sequence: u32) { self.expected.store(sequence, Ordering::Release); }
    pub fn observe(&self, sequence: u32) -> bool {
        if sequence == 0 || self.expected.compare_exchange(sequence, 0, Ordering::AcqRel, Ordering::Acquire).is_err() { return false; }
        self.seen.store(sequence, Ordering::Release);
        true
    }
    pub fn seen(&self, sequence: u32) -> bool { self.seen.load(Ordering::Acquire) == sequence }
}

pub fn receipt(tag: i64) -> Option<u32> {
    let tag = tag as u64;
    (tag & 0xffff_ffff_0000_0000 == TAG && tag as u32 != 0).then_some(tag as u32)
}

pub fn eligible(code: u32) -> bool {
    code != 58 && code != 464 && !(59..=68).contains(&code) && !(87..=88).contains(&code) && !(183..=194).contains(&code)
        && !one_mk_proto::mediakeys::MEDIA_KEYS.iter().any(|key| key.evdev == code)
        && logic::evdev_to_mac(code).is_some()
}

#[derive(Default)]
pub struct Flags {
    physical: HeldKeys,
    suppressed: HeldKeys,
}

impl Flags {
    pub fn rewrite(&self, original: u64, injected: u64, active: bool) -> u64 {
        if !active { return original; }
        self.apply(original, injected)
    }

    pub fn key(&mut self, code: u32, down: bool, swallowed: bool) {
        if !one_mk_proto::Mods::is_modifier(code) { return; }
        self.physical.update(code, down);
        if swallowed { self.suppressed.update(code, down); }
        else if !down { self.suppressed.update(code, false); }
    }

    pub fn apply(&self, original: u64, injected: u64) -> u64 {
        let mut passed = logic::Modifiers::default();
        for code in self.physical.iter().filter(|&code| !self.suppressed.is_held(code)) {
            if logic::evdev_to_mac(code).and_then(|key| logic::modifier_down(key, original)) == Some(true) {
                passed.update(code, true);
            }
        }
        let mask = flags::SHIFT | flags::CONTROL | flags::ALTERNATE | flags::COMMAND | flags::DEV_ALL;
        (original & !mask) | (original & passed.flags() & !flags::NON_COALESCED) | injected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_modifier_release_cannot_add_phantom_flags() {
        let mut state = Flags::default();
        state.key(125, true, false);
        state.key(42, true, false);
        assert_eq!(state.apply(flags::NON_COALESCED, 0), flags::NON_COALESCED);
    }

    #[test]
    fn disabled_and_paused_flags_are_bit_exact_main_passthrough() {
        let mut state = Flags::default();
        state.key(125, true, false);
        state.key(42, true, true);
        let mut random = 0x9132_abc7_u64;
        for _ in 0..100_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            assert_eq!(state.rewrite(random, u64::MAX, false), random);
        }
    }

    #[test]
    fn stale_opposite_side_cannot_keep_an_aggregate_modifier() {
        let mut state = Flags::default();
        state.key(42, true, true);
        state.key(54, true, false);
        assert_eq!(state.apply(flags::SHIFT | flags::DEV_LSHIFT, 0), 0);
    }

    #[test]
    fn only_the_outstanding_receipt_can_acknowledge_a_post() {
        let receipt = Receipt::default();
        assert!(!receipt.observe(1));
        receipt.arm(1);
        assert!(!receipt.observe(2));
        assert!(!receipt.seen(1));
        assert!(receipt.observe(1));
        assert!(!receipt.observe(1));
        receipt.arm(2);
        assert!(!receipt.observe(1));
        assert!(!receipt.seen(2));
        assert!(receipt.observe(2));
        assert!(receipt.seen(2));
        assert!(!receipt.observe(0));
    }

    #[test]
    fn receipt_tags_are_exact_and_never_wrap_to_an_untagged_event() {
        assert_eq!(receipt(TAG as i64), None);
        assert_eq!(receipt((TAG | 42) as i64), Some(42));
        assert_eq!(receipt(crate::macos_localmap_logic::TAG.wrapping_add(1 << 32) as i64), None);
        assert_eq!(receipt(0x6f6e_655f_6d6b), None);
    }

    #[test]
    fn caps_fn_media_and_function_keys_are_not_local_physical_keys() {
        for code in [58, 464, 59, 60, 88, 183, 194, 115, 164] { assert!(!eligible(code)); }
        for code in [30, 42, 54, 29, 125] { assert!(eligible(code)); }
    }

    #[test]
    fn suppressed_side_does_not_clear_passed_opposite_modifier() {
        let mut state = Flags::default();
        state.key(42, true, true);
        state.key(54, true, false);
        let original = flags::SHIFT | flags::DEV_LSHIFT | flags::DEV_RSHIFT;
        assert_eq!(state.apply(original, 0), flags::SHIFT | flags::DEV_RSHIFT);
        state.key(54, false, false);
        assert_eq!(state.apply(flags::SHIFT | flags::DEV_LSHIFT, 0), 0);
        state.key(42, false, true);
        assert_eq!(state.apply(0, 0), 0);
    }

    #[test]
    fn synthetic_flags_never_become_physical_ownership() {
        let mut state = Flags::default();
        state.key(42, true, true);
        let synthetic = flags::CONTROL | flags::DEV_LCTL;
        let toggle = flags::ALPHA_SHIFT | flags::SECONDARY_FN;
        assert_eq!(state.apply(synthetic | toggle, synthetic), synthetic | toggle);
        assert_eq!(state.apply(synthetic | toggle, 0), toggle);
        state.key(97, true, false);
        assert_eq!(state.apply(synthetic | toggle, 0), toggle);
        state.key(97, false, false);
        state.key(42, false, true);
        assert_eq!(state.apply(synthetic | toggle, 0), toggle);
    }

    #[test]
    fn random_flags_changed_streams_never_leave_a_suppressed_flag() {
        let mut random = 0x7182_7439_u64;
        let codes = [42, 54, 29, 97, 56, 100, 125, 126];
        let mut state = Flags::default();
        let mut physical = logic::Modifiers::default();
        for _ in 0..100_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let code = codes[(random as usize >> 4) % codes.len()];
            let down = random & 1 != 0;
            physical.update(code, down);
            let kc = logic::evdev_to_mac(code).unwrap();
            assert_eq!(logic::modifier_down(kc, physical.flags()), Some(down));
            state.key(code, down, true);
            assert_eq!(state.apply(physical.flags(), 0), flags::NON_COALESCED);
        }
        for code in codes { state.key(code, false, true); }
        assert_eq!(state.apply(0, 0), 0);
    }
}
