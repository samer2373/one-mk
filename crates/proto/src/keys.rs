/// Keys and buttons currently held down, by Linux evdev code (the canonical
/// code space on the wire; other platforms map to it).
///
/// Whenever control moves, a peer disconnects or traffic stops, every held
/// key is released, so modifiers can never get stuck.
#[derive(Clone, Default)]
pub struct HeldKeys {
    bits: [u64; Self::WORDS],
}

impl HeldKeys {
    /// evdev KEY_MAX is 0x2ff; buttons (BTN_*) live in the same space.
    const WORDS: usize = 0x300 / 64;

    pub fn new() -> Self {
        Self::default()
    }

    /// Records a transition. Returns false for codes out of range or for a
    /// release of a key that is not held (which should not be forwarded).
    pub fn update(&mut self, code: u32, down: bool) -> bool {
        let Some((w, m)) = Self::slot(code) else {
            return false;
        };
        let was = self.bits[w] & m != 0;
        if down {
            self.bits[w] |= m;
            true
        } else {
            self.bits[w] &= !m;
            was
        }
    }

    pub fn is_held(&self, code: u32) -> bool {
        Self::slot(code).is_some_and(|(w, m)| self.bits[w] & m != 0)
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }

    /// Clears the set, returning every code that was held.
    pub fn take_all(&mut self) -> Vec<u32> {
        let mut out = Vec::new();
        for (w, word) in self.bits.iter_mut().enumerate() {
            let mut b = std::mem::take(word);
            while b != 0 {
                let i = b.trailing_zeros();
                out.push(w as u32 * 64 + i);
                b &= b - 1;
            }
        }
        out
    }

    fn slot(code: u32) -> Option<(usize, u64)> {
        let w = (code / 64) as usize;
        (w < Self::WORDS).then(|| (w, 1u64 << (code % 64)))
    }
}

/// Trades Ctrl and Cmd (Meta) places, left with left and right with right;
/// every other code is unchanged. Applying it twice gives the original.
pub fn swap_ctrl_meta(code: u32) -> u32 {
    match code {
        29 => 125,  // KEY_LEFTCTRL <-> KEY_LEFTMETA
        125 => 29,
        97 => 126,  // KEY_RIGHTCTRL <-> KEY_RIGHTMETA
        126 => 97,
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_ctrl_and_meta_only() {
        assert_eq!([29, 97, 125, 126].map(swap_ctrl_meta), [125, 126, 29, 97]);
        assert_eq!([30, 56, 42, 0x110].map(swap_ctrl_meta), [30, 56, 42, 0x110]);
        for c in 0..0x300 {
            assert_eq!(swap_ctrl_meta(swap_ctrl_meta(c)), c);
        }
    }

    #[test]
    fn tracks_and_releases() {
        let mut k = HeldKeys::new();
        assert!(k.update(29, true)); // KEY_LEFTCTRL
        assert!(k.update(0x110, true)); // BTN_LEFT
        assert!(!k.update(30, false)); // release of a key never pressed
        assert!(k.is_held(29));
        assert_eq!(k.take_all(), vec![29, 0x110]);
        assert!(k.is_empty());
        assert!(!k.update(10_000, true));
    }
}
