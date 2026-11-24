//! one-mk wire protocol and the pure logic that decides where the pointer is.
//!
//! Nothing in this crate does I/O, so all of it is unit-tested and shared by
//! every platform backend.

pub mod appmatch;
pub mod shortcuts;
pub mod pairing_layout;
pub mod autoscroll;
pub mod gesture;
mod codec;
pub mod device;
pub mod drag;
mod edge;
mod geometry;
mod keys;
pub mod keynames;
mod lock;
pub mod mediakeys;
mod motion;
pub mod overlay;
pub mod pointer;
pub mod remap;
pub mod typing;
pub mod scroll;
mod sleep;
mod transfer;
mod wrap;
pub mod wallpaper;

pub use codec::{Control, Datagram, DecodeError, FOCUS_REPEAT, FOCUS_SILENCE, MAX_SCREENS, StreamKind, caps, os};
pub use edge::{
    AWAY, Approach, EdgeGuard, Focus, Modifier, Mods, Next, Refusal, Start, app_matches, cycle_next, fullscreen_blocks,
    in_dead_corner,
};
pub use geometry::{Desktop, Edge, Pos, Rect, Side, touching};
pub use keys::{HeldKeys, swap_ctrl_meta};
pub use lock::{CAUSE_WINDOW, LockSync, OnRemote};
pub use motion::{HOME_INSET, MotionReceiver, Received, RelReceived, RelativeBase, RemoteCursor, Step};
pub use sleep::{OnRemoteSleep, SleepSync};
pub use transfer::{Batch, ClipAck, ClipFormat, ClipHeader, FileMsg};
pub use wrap::{free_edges, wrap_point};

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
/// bidi-override, zero-width and other invisible characters (which could
/// redraw a terminal or disguise a name), collapses whitespace and limits the
/// length. A name left empty is shown as "unnamed device".
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c,
                    '\u{200B}'..='\u{200F}' // zero-width, LRM/RLM
                    | '\u{202A}'..='\u{202E}' // bidi embeddings/overrides
                    | '\u{2060}'..='\u{206F}' // word joiner, invisible operators, bidi isolates, deprecated format characters
                    | '\u{FEFF}' // BOM / zero-width no-break space
                    | '\u{00AD}' // soft hyphen
                    | '\u{034F}' // combining grapheme joiner
                    | '\u{061C}' // Arabic letter mark (bidi)
                    | '\u{115F}'..='\u{1160}' // Hangul choseong/jungseong fillers
                    | '\u{17B4}'..='\u{17B5}' // Khmer inherent vowels
                    | '\u{180B}'..='\u{180F}' // Mongolian variation selectors, vowel separator
                    | '\u{2800}' // braille blank
                    | '\u{3164}' // Hangul filler
                    | '\u{FFA0}' // halfwidth Hangul filler
                    | '\u{FFF9}'..='\u{FFFB}' // interlinear annotation
                    | '\u{E0000}'..='\u{E007F}' // tag characters
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

    #[test]
    fn strips_characters_that_show_as_blank() {
        assert_eq!(sanitize_name("\u{3164}"), "unnamed device");
        assert_eq!(sanitize_name("\u{2800}\u{FFA0}\u{115F}\u{1160}"), "unnamed device");
        assert_eq!(sanitize_name("\u{E0041}\u{E007F}"), "unnamed device");
        assert_eq!(sanitize_name("Mac\u{061C}\u{2800}"), "Mac");
        assert_eq!(sanitize_name("Samer's MacBook\u{3164}"), "Samer's MacBook");
        assert_eq!(sanitize_name("a\u{206A}b"), "ab");
    }
}
