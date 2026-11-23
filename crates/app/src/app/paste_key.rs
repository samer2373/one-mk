//! Ctrl+V (Cmd+V) while a key is being recorded.
//!
//! egui-winit takes a paste shortcut for itself: the key-down of Ctrl+V or
//! Cmd+V never reaches the window as a key. It becomes `Paste(text)` when the
//! clipboard holds text, `PasteImage` when it holds a picture, and nothing at
//! all when it holds neither (it is empty, or holds files). The recorder
//! ("Press new key" and the test box) hears V going down from the first two
//! only, and a key it can't hear is the modifier alone: Ctrl+V recorded as Ctrl
//! with an empty clipboard. `eframe` lets the shortcut be turned off only for
//! the whole window at start, which would break paste in every text field.
//!
//! The key-up of V is not taken, so it does arrive. While a recorder listens, a
//! V going up that was never heard going down is a swallowed paste shortcut:
//! the down is put in front of it, as the `Paste` it would have been. (A
//! picture on the clipboard comes as `PasteImage`, which the recorder does not
//! take, so it is made a `Paste` of no text, which it does.) Text fields do not
//! listen while the recorder has the keyboard, so none sees these.
//!
//! One case stays out of reach: Ctrl let go before V, in an earlier frame than
//! V, is recorded as Ctrl alone, as the shortcut hid V from the recorder
//! until it went up.

use egui::{Event, Key, Modifiers};

/// What has been seen of V, to tell a swallowed shortcut from a plain V.
#[derive(Default)]
pub(super) struct PasteKey {
    /// V went down (as a key, or as the paste that stands for it) and not up yet.
    v_down: bool,
    /// A command key (Ctrl, or Cmd on a Mac) is held, as the last modifiers
    /// event of the frames so far said.
    command: bool,
}

fn is_v(event: &Event) -> Option<bool> {
    match event {
        Event::Key { key, physical_key, pressed, .. } if *key == Key::V || *physical_key == Some(Key::V) => Some(*pressed),
        _ => None,
    }
}

impl PasteKey {
    /// Mends this frame's `events`: while `listening` (a recorder has the
    /// keyboard), puts the V that a paste shortcut swallowed back in front of
    /// its key-up.
    pub fn mend(&mut self, events: &mut Vec<Event>, listening: bool, focused: bool) {
        if !focused {
            self.v_down = false;
        }
        let held = |m: &Modifiers| m.command || m.ctrl || m.mac_cmd;
        // Held when this frame began: a shortcut's V and its modifier can let go together.
        let held_before = self.command;
        let mut out: Vec<Event> = Vec::with_capacity(events.len() + 1);
        for event in events.drain(..) {
            match is_v(&event) {
                Some(true) => self.v_down = true,
                Some(false) => {
                    let command = match &event {
                        Event::Key { modifiers, .. } => held(modifiers),
                        _ => false,
                    } || self.command || held_before;
                    if listening && !self.v_down && command {
                        // Before the modifiers that let go in this frame too, so the recorder
                        // has V down while the chord is still held.
                        let mut at = out.len();
                        while at > 0 && matches!(&out[at - 1], Event::ModifiersChanged(_) | Event::Key { pressed: false, key: Key::ControlLeft | Key::ControlRight | Key::SuperLeft | Key::SuperRight | Key::AltLeft | Key::AltRight | Key::ShiftLeft | Key::ShiftRight, .. }) {
                            at -= 1;
                        }
                        out.insert(at, Event::Paste(String::new()));
                    }
                    self.v_down = false;
                }
                None => {}
            }
            match event {
                Event::ModifiersChanged(modifiers) => {
                    self.command = held(&modifiers);
                    out.push(event);
                }
                Event::Paste(_) => {
                    self.v_down = true;
                    out.push(event);
                }
                // The recorder takes a paste of text only.
                Event::PasteImage(_) => {
                    self.v_down = true;
                    out.push(if listening { Event::Paste(String::new()) } else { event });
                }
                other => out.push(other),
            }
        }
        *events = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Modifiers = Modifiers { ctrl: true, command: true, alt: false, shift: false, mac_cmd: false };

    fn v(pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key: Key::V, physical_key: Some(Key::V), pressed, repeat: false, modifiers }
    }

    fn ctrl_up(modifiers: Modifiers) -> Event {
        Event::Key { key: Key::ControlLeft, physical_key: Some(Key::ControlLeft), pressed: false, repeat: false, modifiers }
    }

    /// One frame for a recorder that listens, with the window focused.
    fn frame(mend: &mut PasteKey, mut events: Vec<Event>) -> Vec<Event> {
        mend.mend(&mut events, true, true);
        events
    }

    /// What the recorder would make of `events`: (evdev code, down).
    fn heard(events: &[Event]) -> Vec<(u16, bool)> {
        events.iter().filter_map(crate::tune::event_key).map(|(code, down, _)| (code, down)).collect()
    }

    #[test]
    fn a_swallowed_paste_down_is_put_back_before_its_key_up() {
        let mut mend = PasteKey::default();
        // Ctrl goes down, then V with nothing on the clipboard: egui-winit gives no event for it.
        assert!(frame(&mut mend, vec![Event::ModifiersChanged(CTRL)]).len() == 1);
        // V goes up (Ctrl still held), then Ctrl.
        let events = frame(&mut mend, vec![v(false, CTRL), ctrl_up(Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(heard(&events), vec![(47, true), (47, false), (29, false)], "V down, V up, then Ctrl up");
    }

    #[test]
    fn the_down_goes_before_a_modifier_that_let_go_in_the_same_frame() {
        let mut mend = PasteKey::default();
        frame(&mut mend, vec![Event::ModifiersChanged(CTRL)]);
        let events = frame(&mut mend, vec![ctrl_up(Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE), v(false, Modifiers::NONE)]);
        assert_eq!(heard(&events), vec![(47, true), (29, false), (47, false)]);
        // Ctrl let go in an earlier frame than V is not told from a plain V going up.
        let mut mend = PasteKey::default();
        frame(&mut mend, vec![Event::ModifiersChanged(CTRL)]);
        frame(&mut mend, vec![ctrl_up(Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]);
        let events = frame(&mut mend, vec![v(false, Modifiers::NONE)]);
        assert_eq!(heard(&events), vec![(47, false)]);
    }

    #[test]
    fn a_paste_that_arrived_is_not_doubled() {
        let mut mend = PasteKey::default();
        frame(&mut mend, vec![Event::ModifiersChanged(CTRL), Event::Paste("text".into())]);
        let events = frame(&mut mend, vec![v(false, CTRL), ctrl_up(Modifiers::NONE)]);
        assert_eq!(heard(&events), vec![(47, false), (29, false)], "its down came as the Paste");
        // And a plain V pressed and let go is just that.
        let events = frame(&mut mend, vec![v(true, Modifiers::NONE), v(false, Modifiers::NONE)]);
        assert_eq!(heard(&events), vec![(47, true), (47, false)]);
    }

    #[test]
    fn a_picture_on_the_clipboard_is_heard_as_the_paste_it_is() {
        let image = std::sync::Arc::new(egui::ColorImage::example());
        let mut mend = PasteKey::default();
        let events = frame(&mut mend, vec![Event::ModifiersChanged(CTRL), Event::PasteImage(image.clone())]);
        assert!(matches!(events[1], Event::Paste(ref text) if text.is_empty()));
        let events = frame(&mut mend, vec![v(false, CTRL)]);
        assert_eq!(heard(&events), vec![(47, false)], "not doubled");
        // Not listening: the window's own paste handling sees what egui-winit made.
        let mut mend = PasteKey::default();
        let mut events = vec![Event::PasteImage(image)];
        mend.mend(&mut events, false, true);
        assert!(matches!(events[0], Event::PasteImage(_)));
    }

    #[test]
    fn nothing_is_added_unless_a_recorder_listens() {
        let mut mend = PasteKey::default();
        let mut events = vec![Event::ModifiersChanged(CTRL), v(false, CTRL)];
        mend.mend(&mut events, false, true);
        assert_eq!(events.len(), 2, "text fields never get a paste they did not ask for");
        // A V going up with no modifier held is not a paste shortcut.
        let mut mend = PasteKey::default();
        let events = frame(&mut mend, vec![v(false, Modifiers::NONE)]);
        assert_eq!(heard(&events), vec![(47, false)]);
        // A window that lost the keyboard forgets what it had seen of V.
        let mut mend = PasteKey::default();
        frame(&mut mend, vec![Event::ModifiersChanged(CTRL), v(true, CTRL)]);
        mend.mend(&mut vec![], true, false);
        let events = frame(&mut mend, vec![v(false, CTRL)]);
        assert_eq!(heard(&events), vec![(47, true), (47, false)]);
    }
}
