#[derive(Clone, Copy)]
pub struct Shortcut {
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
}

pub const fn defaults(windows: bool) -> [Shortcut; 6] {
    [
        Shortcut { key: "jump", label: "Jump to computer N", default: if windows { "ctrl+alt+shift+{n}" } else { "ctrl+alt+{n}" } },
        Shortcut { key: "cycle", label: "The next computer", default: "ctrl+alt+pagedown" },
        Shortcut { key: "lock_pointer", label: "Lock the pointer here", default: "ctrl+alt+p" },
        Shortcut { key: "home", label: "Back to this computer", default: "ctrl+alt+h" },
        Shortcut { key: "broadcast", label: "Broadcast to all computers", default: if windows { "ctrl+alt+shift+b" } else { "ctrl+alt+b" } },
        Shortcut { key: "fetch_clipboard", label: "Fetch the next computer's clipboard", default: if windows { "ctrl+alt+shift+v" } else { "ctrl+alt+v" } },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_has_a_unique_key_and_default() {
        for windows in [false, true] {
            let shortcuts = defaults(windows);
            for (index, shortcut) in shortcuts.iter().enumerate() {
                assert!(!shortcut.label.is_empty());
                assert!(!shortcut.default.is_empty());
                assert!(shortcuts[..index].iter().all(|other| other.key != shortcut.key && other.default != shortcut.default));
            }
            assert_eq!(shortcuts[4].key, "broadcast");
            assert_eq!(shortcuts[5].key, "fetch_clipboard");
            assert_eq!(shortcuts[4].default.contains("shift"), windows);
            assert_eq!(shortcuts[5].default.contains("shift"), windows);
        }
    }
}
