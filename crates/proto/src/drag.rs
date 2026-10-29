#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_pointer_arms_once_and_leave_hides() {
        let mut state = Drag::default();
        assert_eq!(state.pointer(false, Some(3)), Change::None);
        assert_eq!(state.pointer(true, Some(3)), Change::Show(3));
        assert_eq!(state.pointer(true, Some(3)), Change::None);
        assert_eq!(state.pointer(true, None), Change::Hide);
        assert_eq!(state.drop_on(3), None);
    }

    #[test]
    fn drop_is_single_use_and_cancel_latches_until_release() {
        let mut state = Drag::default();
        state.pointer(true, Some(2));
        assert_eq!(state.drop_on(9), None);
        assert_eq!(state.drop_on(2), Some(2));
        assert_eq!(state.drop_on(2), None);
        assert_eq!(state.pointer(true, Some(2)), Change::None);
        state.pointer(false, None);
        assert_eq!(state.pointer(true, Some(2)), Change::Show(2));
        assert_eq!(state.cancel(), Change::Hide);
        assert_eq!(state.pointer(true, Some(2)), Change::None);
        state.pointer(false, None);
        assert_eq!(state.pointer(true, Some(2)), Change::Show(2));
        assert_eq!(state.pointer(false, None), Change::Hide);
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Change {
    #[default]
    None,
    Show(usize),
    Hide,
}

/// Single-use edge drops. Identical samples debounce; cancellation latches until up.
#[derive(Default, Debug)]
pub struct Drag {
    zone: Option<usize>,
    canceled: bool,
}

impl Drag {
    pub fn pointer(&mut self, held: bool, zone: Option<usize>) -> Change {
        if !held {
            self.canceled = false;
        }
        let next = zone.filter(|_| held && !self.canceled);
        if next == self.zone {
            return Change::None;
        }
        self.zone = next;
        next.map_or(Change::Hide, Change::Show)
    }

    pub fn cancel(&mut self) -> Change {
        self.canceled = true;
        self.zone.take().map_or(Change::None, |_| Change::Hide)
    }

    pub fn drop_on(&mut self, zone: usize) -> Option<usize> {
        if self.zone != Some(zone) || self.canceled {
            return None;
        }
        self.cancel();
        Some(zone)
    }
}
