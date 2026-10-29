//! Shared deterministic stream generator and down/up invariant oracle.
//! Available only in tests or with the explicit `test-support` feature.

use super::Emit;
use std::collections::BTreeSet;

pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, upper: usize) -> usize {
        (self.next_u64() % upper as u64) as usize
    }
}

#[derive(Default)]
pub struct Wire {
    pub down: BTreeSet<u32>,
}

impl Wire {
    pub fn apply(&mut self, event: Emit, physical_repeat: bool) {
        if let Emit::Key { code, down } = event {
            if down {
                let new = self.down.insert(code);
                assert!(new || physical_repeat, "{code} pressed twice, not by a repeat");
            } else {
                assert!(self.down.remove(&code), "{code} released but not down");
            }
        }
    }
}
