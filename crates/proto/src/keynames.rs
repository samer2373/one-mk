//! Names for keys and mouse buttons, for people: one table of evdev codes
//! with their aliases, and the parser and printer for keys, chords
//! (`ctrl+shift+esc`) and triggers (`ctrl+h`), shared by the command line,
//! `settings.toml` and the window.
//!
//! Names are case-insensitive; spaces, `-` and `_` inside a name don't
//! matter (`volume up`, `Volume-Up` and `volumeup` are one key). Any code
//! can also be written `code:58` or `evdev:KEY_CAPSLOCK`.

use std::fmt;
use std::str::FromStr;

/// Codes are evdev codes below this (keys and mouse buttons share it).
pub const CODE_LIMIT: u32 = 0x300;

/// The first mouse button code (`BTN_LEFT`) and how many there are names for.
pub const BTN_LEFT: u32 = 0x110;
pub const BUTTONS: u32 = 8;

/// One key: its code, its evdev name (also accepted without `KEY_`) and
/// its names; the first name is the one printed.
struct Key {
    code: u16,
    evdev: &'static str,
    names: &'static [&'static str],
}

const fn k(code: u16, evdev: &'static str, names: &'static [&'static str]) -> Key {
    Key { code, evdev, names }
}

#[rustfmt::skip]
const KEYS: &[Key] = &[
    k(1, "KEY_ESC", &["esc", "escape"]),
    k(2, "KEY_1", &["1"]), k(3, "KEY_2", &["2"]), k(4, "KEY_3", &["3"]),
    k(5, "KEY_4", &["4"]), k(6, "KEY_5", &["5"]), k(7, "KEY_6", &["6"]),
    k(8, "KEY_7", &["7"]), k(9, "KEY_8", &["8"]), k(10, "KEY_9", &["9"]),
    k(11, "KEY_0", &["0"]),
    k(12, "KEY_MINUS", &["minus", "-", "dash"]),
    k(13, "KEY_EQUAL", &["equal", "=", "equals"]),
    k(14, "KEY_BACKSPACE", &["backspace", "bksp"]),
    k(15, "KEY_TAB", &["tab"]),
    k(16, "KEY_Q", &["q"]), k(17, "KEY_W", &["w"]), k(18, "KEY_E", &["e"]), k(19, "KEY_R", &["r"]),
    k(20, "KEY_T", &["t"]), k(21, "KEY_Y", &["y"]), k(22, "KEY_U", &["u"]), k(23, "KEY_I", &["i"]),
    k(24, "KEY_O", &["o"]), k(25, "KEY_P", &["p"]),
    k(26, "KEY_LEFTBRACE", &["leftbrace", "[", "lbracket"]),
    k(27, "KEY_RIGHTBRACE", &["rightbrace", "]", "rbracket"]),
    k(28, "KEY_ENTER", &["enter", "return", "ret"]),
    k(29, "KEY_LEFTCTRL", &["lctrl", "ctrl", "control", "leftcontrol", "lcontrol"]),
    k(30, "KEY_A", &["a"]), k(31, "KEY_S", &["s"]), k(32, "KEY_D", &["d"]), k(33, "KEY_F", &["f"]),
    k(34, "KEY_G", &["g"]), k(35, "KEY_H", &["h"]), k(36, "KEY_J", &["j"]), k(37, "KEY_K", &["k"]),
    k(38, "KEY_L", &["l"]),
    k(39, "KEY_SEMICOLON", &["semicolon", ";"]),
    k(40, "KEY_APOSTROPHE", &["apostrophe", "'", "quote"]),
    k(41, "KEY_GRAVE", &["grave", "`", "backtick"]),
    k(42, "KEY_LEFTSHIFT", &["lshift", "shift"]),
    k(43, "KEY_BACKSLASH", &["backslash", "\\"]),
    k(44, "KEY_Z", &["z"]), k(45, "KEY_X", &["x"]), k(46, "KEY_C", &["c"]), k(47, "KEY_V", &["v"]),
    k(48, "KEY_B", &["b"]), k(49, "KEY_N", &["n"]), k(50, "KEY_M", &["m"]),
    k(51, "KEY_COMMA", &["comma", ","]),
    k(52, "KEY_DOT", &["dot", ".", "period"]),
    k(53, "KEY_SLASH", &["slash", "/"]),
    k(54, "KEY_RIGHTSHIFT", &["rshift"]),
    k(55, "KEY_KPASTERISK", &["kpasterisk", "kpmultiply", "kp*"]),
    k(56, "KEY_LEFTALT", &["lalt", "alt", "option", "opt", "loption"]),
    k(57, "KEY_SPACE", &["space", "spacebar"]),
    k(58, "KEY_CAPSLOCK", &["capslock", "caps"]),
    k(59, "KEY_F1", &["f1"]), k(60, "KEY_F2", &["f2"]), k(61, "KEY_F3", &["f3"]), k(62, "KEY_F4", &["f4"]),
    k(63, "KEY_F5", &["f5"]), k(64, "KEY_F6", &["f6"]), k(65, "KEY_F7", &["f7"]), k(66, "KEY_F8", &["f8"]),
    k(67, "KEY_F9", &["f9"]), k(68, "KEY_F10", &["f10"]),
    k(69, "KEY_NUMLOCK", &["numlock"]),
    k(70, "KEY_SCROLLLOCK", &["scrolllock"]),
    k(71, "KEY_KP7", &["kp7"]), k(72, "KEY_KP8", &["kp8"]), k(73, "KEY_KP9", &["kp9"]),
    k(74, "KEY_KPMINUS", &["kpminus"]),
    k(75, "KEY_KP4", &["kp4"]), k(76, "KEY_KP5", &["kp5"]), k(77, "KEY_KP6", &["kp6"]),
    k(78, "KEY_KPPLUS", &["kpplus", "plus"]),
    k(79, "KEY_KP1", &["kp1"]), k(80, "KEY_KP2", &["kp2"]), k(81, "KEY_KP3", &["kp3"]),
    k(82, "KEY_KP0", &["kp0"]),
    k(83, "KEY_KPDOT", &["kpdot", "kpdecimal", "kp."]),
    k(85, "KEY_ZENKAKUHANKAKU", &["zenkakuhankaku"]),
    k(86, "KEY_102ND", &["102nd", "iso"]),
    k(87, "KEY_F11", &["f11"]), k(88, "KEY_F12", &["f12"]),
    k(89, "KEY_RO", &["ro"]),
    k(90, "KEY_KATAKANA", &["katakana"]),
    k(91, "KEY_HIRAGANA", &["hiragana"]),
    k(92, "KEY_HENKAN", &["henkan"]),
    k(93, "KEY_KATAKANAHIRAGANA", &["katakanahiragana"]),
    k(94, "KEY_MUHENKAN", &["muhenkan"]),
    k(95, "KEY_KPJPCOMMA", &["kpjpcomma"]),
    k(96, "KEY_KPENTER", &["kpenter"]),
    k(97, "KEY_RIGHTCTRL", &["rctrl", "rcontrol", "rightcontrol"]),
    k(98, "KEY_KPSLASH", &["kpslash", "kpdivide", "kp/"]),
    k(99, "KEY_SYSRQ", &["sysrq", "printscreen", "print", "prtsc"]),
    k(100, "KEY_RIGHTALT", &["ralt", "altgr", "roption"]),
    k(101, "KEY_LINEFEED", &["linefeed"]),
    k(102, "KEY_HOME", &["home"]),
    k(103, "KEY_UP", &["up", "uparrow"]),
    k(104, "KEY_PAGEUP", &["pageup", "pgup"]),
    k(105, "KEY_LEFT", &["left", "leftarrow"]),
    k(106, "KEY_RIGHT", &["right", "rightarrow"]),
    k(107, "KEY_END", &["end"]),
    k(108, "KEY_DOWN", &["down", "downarrow"]),
    k(109, "KEY_PAGEDOWN", &["pagedown", "pgdn"]),
    k(110, "KEY_INSERT", &["insert", "ins"]),
    k(111, "KEY_DELETE", &["delete", "del"]),
    k(113, "KEY_MUTE", &["mute"]),
    k(114, "KEY_VOLUMEDOWN", &["volume down", "voldown"]),
    k(115, "KEY_VOLUMEUP", &["volume up", "volup"]),
    k(116, "KEY_POWER", &["power"]),
    k(117, "KEY_KPEQUAL", &["kpequal", "kp="]),
    k(119, "KEY_PAUSE", &["pause", "break"]),
    k(121, "KEY_KPCOMMA", &["kpcomma"]),
    k(122, "KEY_HANGEUL", &["hangeul", "hangul"]),
    k(123, "KEY_HANJA", &["hanja"]),
    k(124, "KEY_YEN", &["yen"]),
    k(125, "KEY_LEFTMETA", &["lmeta", "meta", "super", "win", "cmd", "command", "lsuper", "lwin", "lcmd"]),
    k(126, "KEY_RIGHTMETA", &["rmeta", "rsuper", "rwin", "rcmd"]),
    k(127, "KEY_COMPOSE", &["compose", "menu", "apps", "application"]),
    k(128, "KEY_STOP", &["browser stop"]),
    k(129, "KEY_AGAIN", &["again", "redo"]),
    k(130, "KEY_PROPS", &["props"]),
    k(131, "KEY_UNDO", &["undo"]),
    k(132, "KEY_FRONT", &["front"]),
    k(133, "KEY_COPY", &["copy"]),
    k(134, "KEY_OPEN", &["open"]),
    k(135, "KEY_PASTE", &["paste"]),
    k(136, "KEY_FIND", &["find"]),
    k(137, "KEY_CUT", &["cut"]),
    k(138, "KEY_HELP", &["help"]),
    k(140, "KEY_CALC", &["launch calculator", "calculator", "calc"]),
    k(142, "KEY_SLEEP", &["sleep"]),
    k(143, "KEY_WAKEUP", &["wakeup"]),
    k(155, "KEY_MAIL", &["launch mail", "mail"]),
    k(156, "KEY_BOOKMARKS", &["browser favorites", "bookmarks"]),
    k(157, "KEY_COMPUTER", &["launch computer", "computer"]),
    k(158, "KEY_BACK", &["browser back"]),
    k(159, "KEY_FORWARD", &["browser forward"]),
    k(161, "KEY_EJECTCD", &["eject"]),
    k(163, "KEY_NEXTSONG", &["next track", "nextsong", "next"]),
    k(164, "KEY_PLAYPAUSE", &["play/pause", "playpause"]),
    k(165, "KEY_PREVIOUSSONG", &["previous track", "previoussong", "prevtrack", "previous", "prev"]),
    k(166, "KEY_STOPCD", &["stop", "stopcd"]),
    k(168, "KEY_REWIND", &["rewind"]),
    k(172, "KEY_HOMEPAGE", &["browser home", "homepage"]),
    k(173, "KEY_REFRESH", &["browser refresh", "refresh"]),
    k(183, "KEY_F13", &["f13"]), k(184, "KEY_F14", &["f14"]), k(185, "KEY_F15", &["f15"]),
    k(186, "KEY_F16", &["f16"]), k(187, "KEY_F17", &["f17"]), k(188, "KEY_F18", &["f18"]),
    k(189, "KEY_F19", &["f19"]), k(190, "KEY_F20", &["f20"]), k(191, "KEY_F21", &["f21"]),
    k(192, "KEY_F22", &["f22"]), k(193, "KEY_F23", &["f23"]), k(194, "KEY_F24", &["f24"]),
    k(200, "KEY_PLAYCD", &["playcd"]),
    k(201, "KEY_PAUSECD", &["pausecd"]),
    k(207, "KEY_PLAY", &["play"]),
    k(208, "KEY_FASTFORWARD", &["fast forward", "fastforward", "ff"]),
    k(217, "KEY_SEARCH", &["browser search", "search"]),
    k(224, "KEY_BRIGHTNESSDOWN", &["brightness down"]),
    k(225, "KEY_BRIGHTNESSUP", &["brightness up"]),
    k(226, "KEY_MEDIA", &["launch media", "media"]),
    k(228, "KEY_KBDILLUMTOGGLE", &["keyboard light toggle"]),
    k(229, "KEY_KBDILLUMDOWN", &["keyboard light down"]),
    k(230, "KEY_KBDILLUMUP", &["keyboard light up"]),
    k(248, "KEY_MICMUTE", &["microphone mute", "micmute"]),
    k(0x1d0, "KEY_FN", &["fn"]),
    k(0x110, "BTN_LEFT", &["button1", "leftbutton", "lbutton", "mouse1"]),
    k(0x111, "BTN_RIGHT", &["button2", "rightbutton", "rbutton", "mouse2"]),
    k(0x112, "BTN_MIDDLE", &["button3", "middlebutton", "mbutton", "mouse3"]),
    k(0x113, "BTN_SIDE", &["button4", "sidebutton", "mouse4", "xbutton1"]),
    k(0x114, "BTN_EXTRA", &["button5", "extrabutton", "mouse5", "xbutton2"]),
    k(0x115, "BTN_FORWARD", &["button6", "mouse6"]),
    k(0x116, "BTN_BACK", &["button7", "mouse7"]),
    k(0x117, "BTN_TASK", &["button8", "mouse8"]),
];

/// What went wrong reading a name; the message says what and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameError(pub String);

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NameError {}

fn err<T>(msg: impl Into<String>) -> Result<T, NameError> {
    Err(NameError(msg.into()))
}

/// Lower case, without spaces, `-` and `_`: the form names are compared in.
/// A name that is only punctuation is kept as it is (`-` is a key).
fn fold(s: &str) -> String {
    let folded: String =
        s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).flat_map(char::to_lowercase).collect();
    if folded.is_empty() { s.trim().to_owned() } else { folded }
}

/// The code of the key or button called `name`.
pub fn parse_key(name: &str) -> Result<u32, NameError> {
    let s = name.trim();
    if s.is_empty() {
        return err("a key name is missing");
    }
    let lower = s.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix("code:") {
        let code = parse_number(n.trim()).ok_or_else(|| NameError(format!("{s:?}: `code:` needs a number, like code:58")))?;
        if code >= CODE_LIMIT {
            return err(format!("{s:?}: codes go up to {} (0x{:x})", CODE_LIMIT - 1, CODE_LIMIT - 1));
        }
        return Ok(code);
    }
    if let Some(n) = lower.strip_prefix("evdev:") {
        let n = n.trim();
        return KEYS
            .iter()
            .find(|k| k.evdev.eq_ignore_ascii_case(n))
            .map(|k| u32::from(k.code))
            .ok_or_else(|| NameError(format!("{s:?}: no such evdev name here (use code:N for others)")));
    }
    // Single punctuation characters are names as they are.
    if s.chars().count() == 1
        && let Some(k) = KEYS.iter().find(|k| k.names.contains(&s))
    {
        return Ok(u32::from(k.code));
    }
    // Names first, then evdev names without their prefix (`leftctrl`).
    let f = fold(s);
    let by_name = KEYS.iter().find(|k| k.names.iter().any(|n| fold(n) == f));
    let by_evdev = || KEYS.iter().find(|k| k.evdev.strip_prefix("KEY_").is_some_and(|e| fold(e) == f));
    if let Some(k) = by_name.or_else(by_evdev) {
        return Ok(u32::from(k.code));
    }
    err(format!("{s:?} is not a key name (examples: capslock, esc, lctrl, f13, kp5, volume up, button4, code:58)"))
}

fn parse_number(s: &str) -> Option<u32> {
    match s.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// The name `code` is printed as: its first name, else `code:N`.
pub fn key_name(code: u32) -> String {
    match KEYS.iter().find(|k| u32::from(k.code) == code) {
        Some(k) => k.names[0].to_owned(),
        None => format!("code:{code}"),
    }
}

/// Every key with a name: `(code, printed name, all names)`, for a picker.
pub fn all_names() -> impl Iterator<Item = (u32, &'static str, &'static [&'static str])> {
    KEYS.iter().map(|k| (u32::from(k.code), k.names[0], k.names))
}

/// Mouse button `n` (1 left, 2 right, 3 middle, 4 back/side, 5
/// forward/extra, 6..8) as a code.
pub fn button_code(n: u32) -> Option<u32> {
    (1..=BUTTONS).contains(&n).then(|| BTN_LEFT + n - 1)
}

/// The number [`button_code`] gives `code`.
pub fn button_number(code: u32) -> Option<u32> {
    (BTN_LEFT..BTN_LEFT + BUTTONS).contains(&code).then(|| code - BTN_LEFT + 1)
}

/// Whether `code` is a mouse button (what goes out as `Control::Button`).
pub fn is_button(code: u32) -> bool {
    (0x110..0x118).contains(&code)
}

// ------------------------------------------------------------- modifiers

/// The eight modifier keys, one bit each (`Mods` bits).
pub const LCTRL: u8 = 1;
pub const LSHIFT: u8 = 2;
pub const LALT: u8 = 4;
pub const LMETA: u8 = 8;
pub const RCTRL: u8 = 16;
pub const RSHIFT: u8 = 32;
pub const RALT: u8 = 64;
pub const RMETA: u8 = 128;

/// The modifier codes by bit index.
pub const MOD_CODES: [u16; 8] = [29, 42, 56, 125, 97, 54, 100, 126];

/// The modifier bit of `code`, 0 if it isn't a modifier.
pub const fn mod_bit(code: u32) -> u8 {
    match code {
        29 => LCTRL,
        42 => LSHIFT,
        56 => LALT,
        125 => LMETA,
        97 => RCTRL,
        54 => RSHIFT,
        100 => RALT,
        126 => RMETA,
        _ => 0,
    }
}

/// Modifier groups (either side): bit `g` stands for the pair `g` and `g + 4`.
pub const CTRL: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const META: u8 = 8;

/// Both sides of the groups in `groups`.
pub const fn sides(groups: u8) -> u8 {
    (groups & 0xf) | ((groups & 0xf) << 4)
}

const GROUP_NAMES: [&str; 4] = ["ctrl", "shift", "alt", "meta"];
const SIDED_NAMES: [&str; 8] = ["lctrl", "lshift", "lalt", "lmeta", "rctrl", "rshift", "ralt", "rmeta"];

// ---------------------------------------------------------------- chords

/// Keys to press: modifiers (sided bits) and a key, like `ctrl+shift+esc`.
/// In a chord `ctrl` is the left Ctrl. A modifier alone (`lctrl`) is a key
/// with no modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: u8,
    pub key: u16,
}

impl Chord {
    pub const fn key(code: u16) -> Self {
        Chord { mods: 0, key: code }
    }
}

/// Splits `ctrl+shift+a` into words; `ctrl++` would be ambiguous, so a
/// lone `+` key is written `kpplus` or `equal`.
fn words(s: &str) -> Result<Vec<&str>, NameError> {
    let s = s.trim();
    if s.is_empty() {
        return err("nothing given (a key like `esc`, or a chord like `ctrl+c`)");
    }
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if let Some(i) = parts.iter().position(|p| p.is_empty()) {
        let what = if i + 1 == parts.len() { "after the last `+`" } else { "between two `+`" };
        return err(format!("{s:?}: a key is missing {what} (write a plus key as `kpplus` or `equal`)"));
    }
    Ok(parts)
}

/// A word's error, saying which chord it is in if there are several words.
fn within(s: &str, parts: &[&str], e: NameError) -> NameError {
    if parts.len() > 1 { NameError(format!("in {:?}: {e}", s.trim())) } else { e }
}

impl FromStr for Chord {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, NameError> {
        let parts = words(s)?;
        let (last, mods) = parts.split_last().expect("words() is never empty");
        let mut bits = 0u8;
        for m in mods {
            let code = parse_key(m).map_err(|e| within(s, &parts, e))?;
            let bit = mod_bit(code);
            if bit == 0 {
                return err(format!("in {:?}: {m:?} is not a modifier (only the last key of a chord may be another key)", s.trim()));
            }
            bits |= bit;
        }
        let key = parse_key(last).map_err(|e| within(s, &parts, e))?;
        if mods.len() > 4 {
            return err(format!("{:?}: at most 4 modifiers", s.trim()));
        }
        Ok(Chord { mods: bits, key: key as u16 })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, name) in SIDED_NAMES.iter().enumerate() {
            if self.mods & (1 << i) != 0 {
                // The left ones print as plain `ctrl`, which means left in a chord.
                let plain = if i < 4 { GROUP_NAMES[i] } else { name };
                write!(f, "{plain}+")?;
            }
        }
        f.write_str(&key_name(u32::from(self.key)))
    }
}

// -------------------------------------------------------------- triggers

/// What a rule reacts to: a key, with modifiers that must be held. `ctrl`
/// here means either Ctrl (`groups`), `lctrl`/`rctrl` one side (`sided`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Trigger {
    pub groups: u8,
    pub sided: u8,
    pub key: u16,
}

impl Trigger {
    pub const fn key(code: u16) -> Self {
        Trigger { groups: 0, sided: 0, key: code }
    }

    pub fn has_mods(&self) -> bool {
        self.groups != 0 || self.sided != 0
    }

    /// Whether the modifiers held (sided bits) satisfy this trigger.
    pub fn matches(&self, held: u8) -> bool {
        (0..4).all(|g| self.groups & (1 << g) == 0 || held & sides(1 << g) != 0) && held & self.sided == self.sided
    }

    /// How many modifiers it asks for: the more, the more specific.
    pub fn specificity(&self) -> u32 {
        self.groups.count_ones() + self.sided.count_ones()
    }

    /// The held modifiers (sided bits) this trigger used.
    pub fn used(&self, held: u8) -> u8 {
        held & (sides(self.groups) | self.sided)
    }
}

impl FromStr for Trigger {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, NameError> {
        let parts = words(s)?;
        let (last, mods) = parts.split_last().expect("words() is never empty");
        let mut t = Trigger::default();
        for m in mods {
            let f = fold(m);
            if let Some(g) = ["ctrl", "shift", "alt", "meta"].iter().position(|n| *n == f) {
                t.groups |= 1 << g;
                continue;
            }
            let code = parse_key(m).map_err(|e| within(s, &parts, e))?;
            let bit = mod_bit(code);
            if bit == 0 {
                return err(format!("in {:?}: {m:?} is not a modifier (only the last key may be another key)", s.trim()));
            }
            // Group aliases (`control`, `super`, `cmd`, `option`) mean either side too.
            let group_alias = ["control", "option", "opt", "super", "win", "cmd", "command"].contains(&f.as_str());
            if group_alias && bit & 0xf != 0 {
                t.groups |= bit;
            } else {
                t.sided |= bit;
            }
        }
        if mods.len() > 4 {
            return err(format!("{:?}: at most 4 modifiers", s.trim()));
        }
        t.key = parse_key(last).map_err(|e| within(s, &parts, e))? as u16;
        Ok(t)
    }
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (g, name) in GROUP_NAMES.iter().enumerate() {
            if self.groups & (1 << g) != 0 {
                write!(f, "{name}+")?;
            }
        }
        for (i, name) in SIDED_NAMES.iter().enumerate() {
            if self.sided & (1 << i) != 0 {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(u32::from(self.key)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_aliases() {
        for (name, code) in [
            ("capslock", 58),
            ("CapsLock", 58),
            ("esc", 1),
            ("Escape", 1),
            ("ctrl", 29),
            ("lctrl", 29),
            ("rctrl", 97),
            ("alt", 56),
            ("altgr", 100),
            ("ralt", 100),
            ("meta", 125),
            ("super", 125),
            ("win", 125),
            ("cmd", 125),
            ("f1", 59),
            ("F12", 88),
            ("f13", 183),
            ("f24", 194),
            ("kp0", 82),
            ("kp5", 76),
            ("a", 30),
            ("Z", 44),
            ("1", 2),
            ("0", 11),
            ("-", 12),
            ("minus", 12),
            ("/", 53),
            ("[", 26),
            ("volume up", 115),
            ("Volume-Up", 115),
            ("volume_up", 115),
            ("play/pause", 164),
            ("playpause", 164),
            ("button4", 0x113),
            ("code:58", 58),
            ("code:0x3a", 58),
            ("evdev:KEY_CAPSLOCK", 58),
            ("evdev:btn_left", 0x110),
            ("leftctrl", 29),
            ("pagedown", 109),
        ] {
            assert_eq!(parse_key(name), Ok(code), "{name}");
        }
    }

    #[test]
    fn every_name_is_unique_and_round_trips() {
        let mut seen = std::collections::HashMap::new();
        for (code, printed, names) in all_names() {
            assert_eq!(parse_key(printed), Ok(code), "{printed}");
            assert_eq!(key_name(code), printed);
            for n in names {
                let f = fold(n);
                if let Some(other) = seen.insert(f.clone(), code) {
                    assert_eq!(other, code, "{n:?} names two keys");
                }
                assert_eq!(parse_key(n), Ok(code), "{n}");
            }
        }
        // The media table's names are all keys here.
        for m in crate::mediakeys::MEDIA_KEYS {
            assert_eq!(parse_key(m.name), Ok(m.evdev), "{}", m.name);
        }
        assert_eq!(key_name(300), "code:300");
        assert_eq!(parse_key(&key_name(300)), Ok(300));
    }

    #[test]
    fn bad_names_say_what_is_wrong() {
        for (bad, says) in [
            ("", "missing"),
            ("capslok", "\"capslok\" is not a key name"),
            ("code:", "needs a number"),
            ("code:x", "needs a number"),
            ("code:768", "codes go up to"),
            ("evdev:KEY_NOPE", "no such evdev name"),
        ] {
            let e = parse_key(bad).unwrap_err().0;
            assert!(e.contains(says), "{bad:?}: {e}");
        }
        for (bad, says) in [
            ("ctrl+", "after the last `+`"),
            ("ctrl++a", "between two `+`"),
            ("a+b", "\"a\" is not a modifier"),
            ("ctrl+nope", "\"nope\" is not a key name"),
            ("  ", "nothing given"),
            ("ctrl+shift+alt+meta+rctrl+a", "at most 4"),
        ] {
            let e = bad.parse::<Chord>().unwrap_err().0;
            assert!(e.contains(says), "{bad:?}: {e}");
            assert!(bad.parse::<Trigger>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn chords_parse_and_print() {
        let c: Chord = "Ctrl+Shift+Esc".parse().unwrap();
        assert_eq!(c, Chord { mods: LCTRL | LSHIFT, key: 1 });
        assert_eq!(c.to_string(), "ctrl+shift+esc");
        let c: Chord = "rctrl+altgr+e".parse().unwrap();
        assert_eq!(c, Chord { mods: RCTRL | RALT, key: 18 });
        assert_eq!(c.to_string().parse::<Chord>().unwrap(), c);
        assert_eq!("lctrl".parse::<Chord>().unwrap(), Chord::key(29));
        assert_eq!("cmd+[".parse::<Chord>().unwrap(), Chord { mods: LMETA, key: 26 });
    }

    #[test]
    fn triggers_match_either_side_or_one() {
        let t: Trigger = "ctrl+h".parse().unwrap();
        assert_eq!(t, Trigger { groups: CTRL, sided: 0, key: 35 });
        assert!(t.matches(LCTRL) && t.matches(RCTRL) && t.matches(LCTRL | LSHIFT));
        assert!(!t.matches(0) && !t.matches(LSHIFT));
        assert_eq!(t.used(RCTRL | LSHIFT), RCTRL);
        let t: Trigger = "lctrl+shift+h".parse().unwrap();
        assert_eq!(t, Trigger { groups: SHIFT, sided: LCTRL, key: 35 });
        assert!(t.matches(LCTRL | RSHIFT) && !t.matches(RCTRL | RSHIFT));
        assert_eq!(t.specificity(), 2);
        assert_eq!(t.to_string(), "shift+lctrl+h");
        assert_eq!(t.to_string().parse::<Trigger>().unwrap(), t);
        assert_eq!("cmd+q".parse::<Trigger>().unwrap(), Trigger { groups: META, sided: 0, key: 16 });
        assert_eq!("capslock".parse::<Trigger>().unwrap(), Trigger::key(58));
    }

    #[test]
    fn buttons_by_number() {
        assert_eq!(button_code(1), Some(0x110));
        assert_eq!(button_code(4), Some(0x113));
        assert_eq!(button_code(9), None);
        assert_eq!(button_code(0), None);
        assert_eq!(button_number(0x114), Some(5));
        assert_eq!(button_number(30), None);
        assert!(is_button(0x110) && !is_button(30));
    }

    #[test]
    fn mod_bits_cover_the_modifier_codes() {
        for (i, &c) in MOD_CODES.iter().enumerate() {
            assert_eq!(mod_bit(u32::from(c)), 1 << i);
        }
        assert_eq!(mod_bit(30), 0);
        assert_eq!(sides(CTRL | META), LCTRL | RCTRL | LMETA | RMETA);
    }
}
