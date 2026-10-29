//! one-mk wire protocol and the pure logic that decides where the pointer is.
//!
//! Nothing in this crate does I/O, so all of it is unit-tested and shared by
//! every platform backend.

mod codec;
pub mod drag;
mod edge;
mod geometry;
mod keys;
pub mod keynames;
mod layout;
mod lock;
pub mod mediakeys;
mod motion;
pub mod pointer;
pub mod remap;
pub mod scroll;
mod sleep;
mod transfer;

pub use codec::{Control, Datagram, DecodeError, FOCUS_REPEAT, FOCUS_SILENCE, MAX_SCREENS, StreamKind, caps, os};
pub use edge::{
    AWAY, Approach, EdgeGuard, Focus, Modifier, Mods, Next, Refusal, Start, app_matches, cycle_next, fullscreen_blocks,
    in_dead_corner,
};
pub use geometry::{Desktop, Pos, Rect, Side};
pub use keys::{HeldKeys, swap_ctrl_meta};
pub use layout::{Align, Seam, overlaps};
pub use lock::{CAUSE_WINDOW, LockSync, OnRemote};
pub use motion::{HOME_INSET, MotionReceiver, Received, RelReceived, RemoteCursor, Step};
pub use sleep::{OnRemoteSleep, SleepSync};
pub use transfer::{Batch, ClipAck, ClipFormat, ClipHeader, FileMsg};

/// Bumped on any incompatible wire change; peers refuse mismatched versions.
pub const PROTOCOL_VERSION: u16 = 2;
/// QUIC ALPN identifier.
pub const ALPN: &[u8] = b"one-mk/1";
/// Default UDP port.
pub const DEFAULT_PORT: u16 = 24900;

/// The app id one-mk goes by on Linux desktops. `one-mk install` writes a
/// desktop entry by this name, without which the portals won't accept it
/// (the GlobalShortcuts portal needs an app id).
pub const LINUX_APP_ID: &str = "io.github.onemk.OneMk";

/// Makes a peer-supplied device name safe to display: removes control,
/// bidi-override and zero-width characters (which could redraw a terminal or
/// disguise a name), collapses whitespace and limits the length.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c,
                    '\u{200B}'..='\u{200F}' // zero-width, LRM/RLM
                    | '\u{202A}'..='\u{202E}' // bidi embeddings/overrides
                    | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
                    | '\u{2066}'..='\u{2069}' // bidi isolates
                    | '\u{FEFF}' // BOM / zero-width no-break space
                )
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let limited: String = collapsed.chars().take(64).collect();
    if limited.is_empty() { "unnamed device".to_owned() } else { limited }
}

#[cfg(test)]
mod tests {
    use super::sanitize_name;

    #[test]
    fn strips_terminal_escapes_and_invisible_characters() {
        assert_eq!(sanitize_name("B-laptop\x1b[3A\r\x1b[2K  123 456"), "B-laptop[3A[2K 123 456");
        assert_eq!(sanitize_name("evil\u{202E}txt.exe"), "eviltxt.exe");
        assert_eq!(sanitize_name("\u{200B}\n"), "unnamed device");
        assert_eq!(sanitize_name(&"x".repeat(300)).len(), 64);
    }
}
