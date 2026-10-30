//! Global keyboard shortcuts: jump to a computer, the next one, lock the
//! pointer, come back here. They work while the pointer is on this computer
//! (the system's own shortcut service tells us: the GlobalShortcuts portal
//! on Linux, `RegisterHotKey` on Windows, an event tap on macOS) and while it
//! is on another one (the daemon sees every captured key, see [`Matcher`]).
//!
//! Chords are written `ctrl+alt+1`: modifiers, then one key, joined by `+`,
//! in any case. Keys are evdev codes, like everywhere else in one-mk, and
//! name the physical key (US layout).
//!
//! Stuck keys: a chord's modifiers go down before anyone can know a chord is
//! coming, so while the pointer is on another computer they have already
//! been sent there. Holding them back would delay every Ctrl and Alt, so
//! they are sent; the chord's last key is not (neither its press nor its
//! release), and whatever the shortcut does next releases the rest: a
//! computer that loses the pointer releases everything it holds, and one
//! that gets it never saw the modifiers go down, so their release is not
//! sent to it either.

#[cfg(target_os = "linux")]
#[path = "hotkeys/linux.rs"]
mod linux;
#[cfg(target_os = "macos")]
#[path = "hotkeys/macos.rs"]
mod macos;
#[cfg(windows)]
#[path = "hotkeys/windows.rs"]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::probe;

use crate::{Event, Kind};
use one_mk_proto::{HeldKeys, Modifier, Mods};
use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Move the mouse and keyboard to computer N (1 to 9), counting every
    /// paired computer and this one, sorted by name.
    Jump(u8),
    /// The next connected computer, then this one again (`one-mk switch next`).
    Cycle,
    /// Toggle `one-mk lock-pointer`.
    LockPointer,
    /// Bring the mouse and keyboard back here (`one-mk switch here`).
    Home,
    /// Send what is typed here to every connected computer, or stop
    /// (`one-mk broadcast toggle`).
    Broadcast,
}

impl Action {
    /// A stable name, for the Linux portal (it remembers the keys the user
    /// picked by this name).
    pub fn id(self) -> String {
        match self {
            Action::Jump(n) => format!("jump-{n}"),
            Action::Cycle => "cycle".into(),
            Action::LockPointer => "lock-pointer".into(),
            Action::Home => "home".into(),
            Action::Broadcast => "broadcast".into(),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "cycle" => Some(Action::Cycle),
            "lock-pointer" => Some(Action::LockPointer),
            "home" => Some(Action::Home),
            "broadcast" => Some(Action::Broadcast),
            _ => {
                let n: u8 = id.strip_prefix("jump-")?.parse().ok()?;
                (1..=9).contains(&n).then_some(Action::Jump(n))
            }
        }
    }

    /// In words, for the desktop's shortcut settings.
    pub fn describe(self) -> String {
        match self {
            Action::Jump(n) => format!("Move the mouse and keyboard to computer {n}"),
            Action::Cycle => "Move the mouse and keyboard to the next computer".into(),
            Action::LockPointer => "Lock the pointer to this computer (or unlock it)".into(),
            Action::Home => "Bring the mouse and keyboard back to this computer".into(),
            Action::Broadcast => "Type on every connected computer at once (or stop)".into(),
        }
    }
}

// ------------------------------------------------------------------ chords

/// Modifiers in a chord. Left and right keys count the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChordMods(u8);

impl ChordMods {
    pub const CTRL: ChordMods = ChordMods(1);
    pub const ALT: ChordMods = ChordMods(2);
    pub const SHIFT: ChordMods = ChordMods(4);
    /// The Windows key, Cmd on a Mac.
    pub const SUPER: ChordMods = ChordMods(8);
    const ALL: [(ChordMods, &'static str); 4] =
        [(Self::CTRL, "ctrl"), (Self::ALT, "alt"), (Self::SHIFT, "shift"), (Self::SUPER, "super")];

    pub fn contains(self, m: ChordMods) -> bool {
        self.0 & m.0 == m.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn with(self, m: ChordMods) -> Self {
        ChordMods(self.0 | m.0)
    }

    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "ctrl" | "control" => Self::CTRL,
            "alt" | "option" | "opt" => Self::ALT,
            "shift" => Self::SHIFT,
            "super" | "meta" | "win" | "cmd" | "command" | "logo" => Self::SUPER,
            _ => return None,
        })
    }

    /// The modifiers held, as the daemon tracks them. Only the left Alt
    /// counts: the right one is AltGr on many keyboards, and AltGr+7 is a
    /// `{` to be typed, not Ctrl+Alt+7 (Windows even adds a Ctrl to it).
    pub fn from_held(m: Mods) -> Self {
        let mut out = ChordMods::default();
        for (has, bit) in [(Modifier::Ctrl, Self::CTRL), (Modifier::Shift, Self::SHIFT)] {
            if m.has(has) {
                out = out.with(bit);
            }
        }
        for k in m.keys() {
            match k {
                KEY_LEFTALT => out = out.with(Self::ALT),
                KEY_LEFTMETA | KEY_RIGHTMETA => out = out.with(Self::SUPER),
                _ => {}
            }
        }
        out
    }

    /// `RegisterHotKey`'s modifier bits (`MOD_ALT` 1, `MOD_CONTROL` 2,
    /// `MOD_SHIFT` 4, `MOD_WIN` 8).
    pub fn windows(self) -> u32 {
        let mut bits = 0;
        for (m, b) in [(Self::ALT, 1), (Self::CTRL, 2), (Self::SHIFT, 4), (Self::SUPER, 8)] {
            if self.contains(m) {
                bits |= b;
            }
        }
        bits
    }

    /// From `CGEventFlags` (device-independent bits only).
    #[cfg(any(target_os = "macos", test))]
    pub fn from_mac_flags(flags: u64) -> Self {
        use crate::macos_logic::flags as f;
        let mut out = ChordMods::default();
        for (bit, m) in [(f::CONTROL, Self::CTRL), (f::ALTERNATE, Self::ALT), (f::SHIFT, Self::SHIFT), (f::COMMAND, Self::SUPER)] {
            if flags & bit != 0 {
                out = out.with(m);
            }
        }
        out
    }

    /// Names joined with `+`, `ctrl` first; `label` for people.
    fn join(self, label: bool) -> String {
        let name = |m: ChordMods, s: &'static str| -> &'static str {
            if !label {
                return s;
            }
            match (m, cfg!(target_os = "macos")) {
                (Self::CTRL, false) => "Ctrl",
                (Self::CTRL, true) => "Control",
                (Self::ALT, false) => "Alt",
                (Self::ALT, true) => "Option",
                (Self::SHIFT, _) => "Shift",
                (_, false) => "Super",
                (_, true) => "Cmd",
            }
        };
        Self::ALL.iter().filter(|(m, _)| self.contains(*m)).map(|&(m, s)| name(m, s)).collect::<Vec<_>>().join("+")
    }
}

const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;
const KEY_RIGHTMETA: u32 = 126;
const KEY_ESC: u32 = 1;

/// Letter rows of the keyboard and the evdev code of their first key.
const LETTER_ROWS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
/// `1` is evdev 2, ..., `9` is 10, `0` is 11.
const DIGITS: &str = "1234567890";
/// Other keys: name, evdev code, XKB keysym name (for the portal), Windows
/// virtual key.
const NAMED: &[(&str, u32, &str, u16)] = &[
    ("space", 57, "space", 0x20),
    ("tab", 15, "Tab", 0x09),
    ("enter", 28, "Return", 0x0D),
    ("esc", KEY_ESC, "Escape", 0x1B),
    ("backspace", 14, "BackSpace", 0x08),
    ("insert", 110, "Insert", 0x2D),
    ("delete", 111, "Delete", 0x2E),
    ("home", 102, "Home", 0x24),
    ("end", 107, "End", 0x23),
    ("pageup", 104, "Page_Up", 0x21),
    ("pagedown", 109, "Page_Down", 0x22),
    ("left", 105, "Left", 0x25),
    ("up", 103, "Up", 0x26),
    ("right", 106, "Right", 0x27),
    ("down", 108, "Down", 0x28),
    ("pause", 119, "Pause", 0x13),
    ("print", 99, "Print", 0x2C),
    ("minus", 12, "minus", 0xBD),
    ("equal", 13, "equal", 0xBB),
    ("comma", 51, "comma", 0xBC),
    ("period", 52, "period", 0xBE),
    ("slash", 53, "slash", 0xBF),
    ("semicolon", 39, "semicolon", 0xBA),
    ("apostrophe", 40, "apostrophe", 0xDE),
    ("grave", 41, "grave", 0xC0),
    ("leftbracket", 26, "bracketleft", 0xDB),
    ("rightbracket", 27, "bracketright", 0xDD),
    ("backslash", 43, "backslash", 0xDC),
];
const ALIASES: &[(&str, &str)] = &[
    ("return", "enter"),
    ("escape", "esc"),
    ("pgup", "pageup"),
    ("pgdn", "pagedown"),
    ("del", "delete"),
    ("ins", "insert"),
    ("printscreen", "print"),
    ("-", "minus"),
    ("=", "equal"),
    (",", "comma"),
    (".", "period"),
    ("/", "slash"),
    (";", "semicolon"),
    ("'", "apostrophe"),
    ("`", "grave"),
    ("[", "leftbracket"),
    ("]", "rightbracket"),
    ("\\", "backslash"),
];

/// evdev codes of F1 to F24.
fn function_key(n: u32) -> Option<u32> {
    match n {
        1..=10 => Some(58 + n),
        11 | 12 => Some(76 + n),
        13..=24 => Some(170 + n),
        _ => None,
    }
}

/// A key by name (lowercase), as its evdev code.
fn key_code(name: &str) -> Option<u32> {
    let name = ALIASES.iter().find(|(a, _)| *a == name).map_or(name, |(_, n)| n);
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if let Some(i) = DIGITS.find(c) {
            return Some(2 + i as u32);
        }
        for (row, first) in LETTER_ROWS {
            if let Some(i) = row.find(c) {
                return Some(first + i as u32);
            }
        }
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse().ok()) {
        return function_key(n);
    }
    NAMED.iter().find(|k| k.0 == name).map(|k| k.1)
}

/// What a key is called: (our name, XKB keysym, Windows virtual key).
fn key_names(code: u32) -> Option<(String, String, u16)> {
    if let Some(i) = (2..=11).contains(&code).then(|| (code - 2) as usize) {
        let c = DIGITS.as_bytes()[i];
        return Some(((c as char).to_string(), (c as char).to_string(), u16::from(c)));
    }
    for (row, first) in LETTER_ROWS {
        if let Some(c) = code.checked_sub(first).and_then(|i| row.as_bytes().get(i as usize)) {
            let c = *c as char;
            return Some((c.to_string(), c.to_string(), c.to_ascii_uppercase() as u16));
        }
    }
    if let Some(n) = (1..=24).find(|&n| function_key(n) == Some(code)) {
        return Some((format!("f{n}"), format!("F{n}"), 0x6F + n as u16));
    }
    NAMED.iter().find(|k| k.1 == code).map(|k| (k.0.to_owned(), k.2.to_owned(), k.3))
}

/// A key's name for people: `A`, `F5`, `PageDown`, `-`.
fn key_label(code: u32) -> String {
    let Some((name, _, _)) = key_names(code) else { return format!("key {code}") };
    if let Some((c, _)) = ALIASES.iter().find(|(a, n)| *n == name && a.len() == 1) {
        return (*c).to_owned();
    }
    match name.as_str() {
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        n => n[..1].to_uppercase() + &n[1..],
    }
}

/// Keys that may be a shortcut on their own (nothing types with them).
fn standalone(code: u32) -> bool {
    (1..=24).any(|n| function_key(n) == Some(code)) || code == 119 || code == 99
}

/// Modifiers and one key, like `ctrl+alt+1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: ChordMods,
    /// evdev code.
    pub key: u32,
}

/// Ctrl+Alt+Shift+Esc always brings the pointer home; it can't be rebound.
pub const EMERGENCY: Chord =
    Chord { mods: ChordMods(ChordMods::CTRL.0 | ChordMods::ALT.0 | ChordMods::SHIFT.0), key: KEY_ESC };

/// Longer is surely not a chord.
const MAX_LEN: usize = 64;

/// Splits `a+b+c` into its modifiers and the last part (lowercased).
fn split_chord(s: &str) -> Result<(ChordMods, String), String> {
    if s.len() > MAX_LEN {
        return Err("that is too long for a shortcut".into());
    }
    let lower = s.trim().to_lowercase();
    let parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("{s:?} is not a shortcut; write it like ctrl+alt+1"));
    }
    let (last, mods) = parts.split_last().expect("split gives at least one part");
    let mut out = ChordMods::default();
    for m in mods {
        let Some(bit) = ChordMods::parse(m) else {
            return Err(format!("{m:?} in {s:?} is not a modifier (ctrl, alt, shift or super)"));
        };
        if out.contains(bit) {
            return Err(format!("{m} appears twice in {s:?}"));
        }
        out = out.with(bit);
    }
    if ChordMods::parse(last).is_some() {
        return Err(format!("{s:?} ends with a modifier; add a key, like ctrl+alt+1"));
    }
    Ok((out, (*last).to_owned()))
}

impl Chord {
    fn checked(mods: ChordMods, key: u32, s: &str) -> Result<Chord, String> {
        if !standalone(key) && (mods.is_empty() || mods == ChordMods::SHIFT) {
            return Err(format!("{s:?} would take over typing; add ctrl, alt or super"));
        }
        Ok(Chord { mods, key })
    }

    /// The portal's trigger format (the XDG "shortcuts" spec): `CTRL+ALT+1`.
    pub fn xdg_trigger(&self) -> String {
        let (_, keysym, _) = key_names(self.key).unwrap_or_default();
        let mods = self.mods.join(false).to_uppercase().replace("SUPER", "LOGO");
        if mods.is_empty() { keysym } else { format!("{mods}+{keysym}") }
    }

    /// Windows virtual key for `RegisterHotKey`.
    pub fn vk(&self) -> Option<u16> {
        key_names(self.key).map(|k| k.2)
    }

    /// For people: `Ctrl+Alt+1`, `Ctrl+Alt+PageDown` (`Control+Option+1`
    /// on a Mac).
    pub fn label(&self) -> String {
        let key = key_label(self.key);
        let mods = self.mods.join(true);
        if mods.is_empty() { key } else { format!("{mods}+{key}") }
    }
}

impl FromStr for Chord {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (mods, key) = split_chord(s)?;
        let Some(code) = key_code(&key) else {
            return Err(format!("{key:?} in {s:?} is not a key one-mk knows (letters, digits, f1-f24, arrows, space, ...)"));
        };
        Chord::checked(mods, code, s)
    }
}

/// The canonical form: `ctrl+alt+1`.
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = key_names(self.key).map_or_else(|| self.key.to_string(), |k| k.0);
        let mods = self.mods.join(false);
        if mods.is_empty() { f.write_str(&key) } else { write!(f, "{mods}+{key}") }
    }
}

/// The jump shortcuts: modifiers plus `{n}`, like `ctrl+alt+{n}`, which
/// stands for the digits 1 to 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JumpChords(pub ChordMods);

impl JumpChords {
    pub fn chord(self, n: u8) -> Chord {
        Chord { mods: self.0, key: 1 + u32::from(n.clamp(1, 9)) }
    }

    /// For people: `Ctrl+Alt+1..9`.
    pub fn label(&self) -> String {
        format!("{}..9", self.chord(1).label())
    }
}

impl FromStr for JumpChords {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (mods, last) = split_chord(s)?;
        if last != "{n}" {
            return Err(format!("{s:?} needs {{n}} where the number goes, like ctrl+alt+{{n}}"));
        }
        Chord::checked(mods, 2, s).map(|c| JumpChords(c.mods))
    }
}

impl fmt::Display for JumpChords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mods = self.0.join(false);
        if mods.is_empty() { f.write_str("{n}") } else { write!(f, "{mods}+{{n}}") }
    }
}

/// Which chord does what. No chord is used twice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings(Vec<(Action, Chord)>);

impl Bindings {
    /// Refuses a chord used twice and the emergency chord.
    pub fn new(list: Vec<(Action, Chord)>) -> Result<Self, String> {
        for (i, (a, c)) in list.iter().enumerate() {
            if *c == EMERGENCY {
                return Err(format!("{} always brings the pointer back; pick another shortcut", c.label()));
            }
            if let Some((b, _)) = list[..i].iter().find(|(_, d)| d == c) {
                return Err(format!("{} would do two things: {} and {}", c.label(), describe_short(*b), describe_short(*a)));
            }
        }
        Ok(Bindings(list))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Action, Chord)> + '_ {
        self.0.iter().copied()
    }

    pub fn action(&self, chord: Chord) -> Option<Action> {
        self.0.iter().find(|(_, c)| *c == chord).map(|(a, _)| *a)
    }
}

fn describe_short(a: Action) -> String {
    match a {
        Action::Jump(n) => format!("jump to computer {n}"),
        Action::Cycle => "next computer".into(),
        Action::LockPointer => "lock the pointer".into(),
        Action::Home => "come back here".into(),
        Action::Broadcast => "broadcast to every computer".into(),
    }
}

// ----------------------------------------------------------------- matching

/// What to do with a captured key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not a shortcut: send it on as usual.
    Pass,
    /// Part of a shortcut that already fired (its repeat or release): drop it.
    Swallow,
    /// A shortcut: drop the key and do this.
    Fire(Action),
}

/// Recognizes shortcuts in the keys captured while the pointer is on
/// another computer, where the system's shortcut service doesn't see them.
///
/// A shortcut fires when its key goes down while exactly its modifiers are
/// held and no other key is: Ctrl+Alt+Shift+1 is not Ctrl+Alt+1, and a key
/// pressed while typing something else (mid-chord) never fires. Repeats
/// don't fire again. The key's press, repeats and release are swallowed.
#[derive(Clone, Default)]
pub struct Matcher {
    down: HeldKeys,
    swallowed: HeldKeys,
}

impl Matcher {
    /// `mods` are the modifiers held before this key.
    pub fn key(&mut self, bindings: &Bindings, mods: Mods, code: u32, down: bool) -> Verdict {
        if Mods::is_modifier(code) {
            return Verdict::Pass;
        }
        let ours = self.swallowed.is_held(code);
        if !down {
            self.down.update(code, false);
            self.swallowed.update(code, false);
            return if ours { Verdict::Swallow } else { Verdict::Pass };
        }
        if self.down.is_held(code) {
            return if ours { Verdict::Swallow } else { Verdict::Pass }; // a repeat
        }
        let busy = !self.down.is_empty();
        self.down.update(code, true);
        if busy {
            return Verdict::Pass;
        }
        match bindings.action(Chord { mods: ChordMods::from_held(mods), key: code }) {
            Some(a) => {
                self.swallowed.update(code, true);
                Verdict::Fire(a)
            }
            None => Verdict::Pass,
        }
    }

    /// Forgets every key (capture ended: their releases won't come here).
    pub fn reset(&mut self) {
        *self = Matcher::default();
    }
}

/// Where a shortcut was seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The system's shortcut service.
    System,
    /// The daemon's [`Matcher`], in captured input.
    Captured,
}

/// One press reported by both the system and the matcher (possible on
/// Linux, where it is unknown whether the desktop's shortcuts still fire
/// while input is captured) counts once.
#[derive(Clone, Debug, Default)]
pub struct Debounce {
    last: Option<(Action, Source, Instant)>,
}

/// Longer than the two reports of one press can be apart, shorter than a
/// deliberate second press.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

impl Debounce {
    /// False if `a` is the other source's report of a press just counted.
    pub fn admit(&mut self, a: Action, src: Source, now: Instant) -> bool {
        if let Some((last, from, at)) = self.last
            && last == a
            && from != src
            && now.saturating_duration_since(at) < DEBOUNCE
        {
            self.last = None;
            return false;
        }
        self.last = Some((a, src, now));
        true
    }
}

// --------------------------------------------------------------- listening

/// Whether the system's shortcut service works here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Being set up; says what is awaited.
    Starting(String),
    /// Working; says how, and anything that didn't work.
    Active(String),
    /// Not available on this system; says why. Shortcuts still work while
    /// the pointer is on another computer.
    Unavailable(String),
}

/// One OS's shortcut service.
trait Platform: Send {
    /// Replaces the shortcuts (none: stop listening).
    fn set(&mut self, bindings: Bindings);
    /// Stops (`true`) or resumes reacting to the shortcuts, keeping them.
    /// Where pausing would need the user again (the Linux portal shows a
    /// dialog when shortcuts are bound anew), nothing happens: the daemon
    /// ignores what arrives meanwhile.
    fn pause(&mut self, _paused: bool) {}
}

/// Listens for the shortcuts through the system's shortcut service. Its
/// events (`Event::Hotkey`, `Event::Hotkeys`) come from the receiver
/// [`listen`] returns. Listening stops when it is dropped.
pub struct Listener(Option<Box<dyn Platform>>);

impl Listener {
    pub fn set(&mut self, bindings: Bindings) {
        if let Some(p) = self.0.as_mut() {
            p.set(bindings);
        }
    }

    /// While a peer controls this computer its keys arrive through the
    /// injector, and this computer's own shortcuts must not fire on them
    /// (nor swallow them). Windows unregisters the shortcuts (its hotkeys
    /// can't tell injected keys apart) and macOS lets every key through;
    /// on Linux the chord still reaches the portal (and is swallowed by
    /// the desktop), so the daemon drops the event instead.
    pub fn pause(&mut self, paused: bool) {
        if let Some(p) = self.0.as_mut() {
            p.pause(paused);
        }
    }
}

/// Starts listening. With the null backend nothing listens (tests feed
/// `Event::Hotkey` themselves). Linux needs a running tokio runtime.
pub fn listen(kind: Kind, bindings: Bindings) -> (Listener, mpsc::UnboundedReceiver<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let platform = match kind {
        Kind::Null => {
            let _ = tx.send(Event::Hotkeys(Availability::Unavailable("not with the null input backend".into())));
            None
        }
        Kind::Native | Kind::X11 | Kind::Wlroots => native(bindings, tx),
    };
    (Listener(platform), rx)
}

#[cfg(target_os = "linux")]
fn native(bindings: Bindings, tx: mpsc::UnboundedSender<Event>) -> Option<Box<dyn Platform>> {
    Some(Box::new(linux::start(bindings, tx)))
}

#[cfg(windows)]
fn native(bindings: Bindings, tx: mpsc::UnboundedSender<Event>) -> Option<Box<dyn Platform>> {
    windows::start(bindings, tx).map(|p| Box::new(p) as Box<dyn Platform>)
}

#[cfg(target_os = "macos")]
fn native(bindings: Bindings, tx: mpsc::UnboundedSender<Event>) -> Option<Box<dyn Platform>> {
    Some(Box::new(macos::start(bindings, tx)))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn native(_bindings: Bindings, tx: mpsc::UnboundedSender<Event>) -> Option<Box<dyn Platform>> {
    let _ = tx.send(Event::Hotkeys(Availability::Unavailable("no shortcut service on this OS".into())));
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn chords_parse_in_any_case_and_print_canonically() {
        let c = chord("Ctrl+Alt+1");
        assert_eq!(c, Chord { mods: ChordMods::CTRL.with(ChordMods::ALT), key: 2 });
        for s in ["ctrl+alt+1", "ALT+CTRL+1", " control + option + 1 ", "alt+ctrl+1"] {
            assert_eq!(chord(s), c, "{s}");
        }
        assert_eq!(c.to_string(), "ctrl+alt+1");
        assert_eq!(chord("cmd+shift+Right").to_string(), "shift+super+right");
        assert_eq!(chord("win+pgdn").to_string(), "super+pagedown");
        assert_eq!(chord("ctrl+alt+Escape").to_string(), "ctrl+alt+esc");
        assert_eq!(chord("ctrl+alt+[").to_string(), "ctrl+alt+leftbracket");
        // Every key name prints back to a chord that parses to the same.
        let mut keys: Vec<String> = DIGITS.chars().map(String::from).collect();
        keys.extend(LETTER_ROWS.iter().flat_map(|(r, _)| r.chars().map(String::from)));
        keys.extend((1..=24).map(|n| format!("f{n}")));
        keys.extend(NAMED.iter().map(|k| k.0.to_owned()));
        for k in keys {
            let c = chord(&format!("ctrl+alt+{k}"));
            assert_eq!(chord(&c.to_string()), c, "{k}");
            assert!(c.vk().is_some() && !c.xdg_trigger().is_empty(), "{k}");
        }
    }

    #[test]
    fn evdev_codes_are_the_physical_keys() {
        let code = |s: &str| chord(&format!("ctrl+{s}")).key;
        assert_eq!([code("1"), code("9"), code("0")], [2, 10, 11]);
        assert_eq!([code("q"), code("a"), code("z"), code("m"), code("l")], [16, 30, 44, 50, 38]);
        assert_eq!([code("f1"), code("f10"), code("f11"), code("f12"), code("f13"), code("f24")], [59, 68, 87, 88, 183, 194]);
        assert_eq!([code("space"), code("left"), code("esc")], [57, 105, 1]);
    }

    #[test]
    fn malformed_chords_are_refused() {
        for bad in [
            "", "+", "ctrl+", "+1", "ctrl++1", "ctrl+alt", "ctrl+ctrl+1", "hyper+1", "ctrl+alt+nope", "ctrl+alt+f25",
            "ctrl+alt+f0", "ctrl alt 1", "1", "a", "shift+a", "shift+1", "ctrl+alt+{n}", "ctrl+alt+1+2", "ctrl+alt+12",
        ] {
            assert!(bad.parse::<Chord>().is_err(), "{bad:?} parsed");
        }
        assert!("ctrl+".repeat(20).parse::<Chord>().unwrap_err().contains("too long"));
        // Keys that type nothing may stand alone.
        assert_eq!(chord("F13").mods, ChordMods::default());
        assert!("pause".parse::<Chord>().is_ok() && "shift+f5".parse::<Chord>().is_ok());
    }

    #[test]
    fn jump_template() {
        let j: JumpChords = "Ctrl+Alt+{n}".parse().unwrap();
        assert_eq!(j.chord(3), chord("ctrl+alt+3"));
        assert_eq!(j.chord(9), chord("ctrl+alt+9"));
        assert_eq!(j.to_string(), "ctrl+alt+{n}");
        assert_eq!(j.to_string().parse::<JumpChords>().unwrap(), j);
        assert_eq!(j.label(), if cfg!(target_os = "macos") { "Control+Option+1..9" } else { "Ctrl+Alt+1..9" });
        for bad in ["ctrl+alt+1", "{n}", "shift+{n}", "ctrl+{N}x", "ctrl+alt+{n}+x", "ctrl+{m}", ""] {
            assert!(bad.parse::<JumpChords>().is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn platform_forms() {
        let c = chord("ctrl+alt+shift+super+1");
        assert_eq!(c.xdg_trigger(), "CTRL+ALT+SHIFT+LOGO+1");
        assert_eq!(chord("ctrl+alt+pageup").xdg_trigger(), "CTRL+ALT+Page_Up");
        assert_eq!(chord("f13").xdg_trigger(), "F13");
        assert_eq!(c.mods.windows(), 1 | 2 | 4 | 8);
        if !cfg!(target_os = "macos") {
            assert_eq!(c.label(), "Ctrl+Alt+Shift+Super+1");
            let labels = ["ctrl+alt+pgdn", "ctrl+alt+esc", "ctrl+alt+-", "alt+f5", "ctrl+alt+space", "ctrl+alt+z"]
                .map(|s| chord(s).label());
            assert_eq!(labels, ["Ctrl+Alt+PageDown", "Ctrl+Alt+Esc", "Ctrl+Alt+-", "Alt+F5", "Ctrl+Alt+Space", "Ctrl+Alt+Z"]);
        }
        assert_eq!(chord("ctrl+alt+1").vk(), Some(0x31));
        assert_eq!(chord("ctrl+alt+n").vk(), Some(0x4E));
        assert_eq!(chord("ctrl+f12").vk(), Some(0x7B));
        assert_eq!(chord("ctrl+f24").vk(), Some(0x87));
        assert_eq!(chord("ctrl+right").vk(), Some(0x27));
        use crate::macos_logic::flags as f;
        assert_eq!(ChordMods::from_mac_flags(f::CONTROL | f::ALTERNATE | f::ALPHA_SHIFT | f::DEV_LCTL), chord("ctrl+alt+1").mods);
        assert_eq!(ChordMods::from_mac_flags(f::COMMAND), ChordMods::SUPER);
        // The macOS tap reads keys by Mac key code: every chord key has one.
        for k in ["1", "0", "a", "z", "f1", "f12", "space", "left", "esc", "slash"] {
            let code = chord(&format!("ctrl+{k}")).key;
            assert!(crate::macos_logic::evdev_to_mac(code).is_some(), "{k}");
        }
    }

    #[test]
    fn actions_have_stable_ids() {
        for a in [Action::Jump(1), Action::Jump(9), Action::Cycle, Action::LockPointer, Action::Home, Action::Broadcast] {
            assert_eq!(Action::from_id(&a.id()), Some(a));
        }
        for bad in ["jump-0", "jump-10", "jump-x", "jump", "", "Cycle"] {
            assert_eq!(Action::from_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn bindings_refuse_duplicates_and_the_emergency_chord() {
        let ok = Bindings::new(vec![(Action::Cycle, chord("ctrl+alt+n")), (Action::Home, chord("ctrl+alt+h"))]).unwrap();
        assert_eq!(ok.action(chord("ctrl+alt+h")), Some(Action::Home));
        assert_eq!(ok.action(chord("ctrl+alt+shift+h")), None);
        let dup = Bindings::new(vec![(Action::Jump(1), chord("ctrl+alt+1")), (Action::Home, chord("alt+ctrl+1"))]);
        assert!(dup.unwrap_err().contains("two things"));
        assert!(Bindings::new(vec![(Action::Home, chord("ctrl+alt+shift+esc"))]).is_err());
    }

    fn mods(codes: &[u32]) -> Mods {
        Mods::from_keys(codes.iter().copied())
    }

    const LCTRL: u32 = 29;
    const RCTRL: u32 = 97;
    const LALT: u32 = 56;
    const LSHIFT: u32 = 42;

    fn bindings() -> Bindings {
        let jump: JumpChords = "ctrl+alt+{n}".parse().unwrap();
        let mut list: Vec<_> = (1..=9).map(|n| (Action::Jump(n), jump.chord(n))).collect();
        list.push((Action::Cycle, chord("ctrl+alt+n")));
        Bindings::new(list).unwrap()
    }

    #[test]
    fn matcher_fires_on_exact_chords_and_swallows_the_key() {
        let b = bindings();
        let mut m = Matcher::default();
        let ca = mods(&[LCTRL, LALT]);
        assert_eq!(m.key(&b, ca, LCTRL, true), Verdict::Pass, "modifiers always pass");
        assert_eq!(m.key(&b, ca, 3, true), Verdict::Fire(Action::Jump(2)));
        assert_eq!(m.key(&b, ca, 3, true), Verdict::Swallow, "a repeat doesn't fire again");
        assert_eq!(m.key(&b, ca, 3, false), Verdict::Swallow, "nor is its release sent");
        assert_eq!(m.key(&b, ca, 3, true), Verdict::Fire(Action::Jump(2)), "a second press fires again");
        assert_eq!(m.key(&b, ca, 3, false), Verdict::Swallow);
        // Right Ctrl counts as Ctrl.
        assert_eq!(m.key(&b, mods(&[RCTRL, LALT]), 49, true), Verdict::Fire(Action::Cycle));
        assert_eq!(m.key(&b, mods(&[]), 49, false), Verdict::Swallow, "released after the modifiers");
        // More or fewer modifiers: not the chord.
        assert_eq!(m.key(&b, mods(&[LCTRL, LALT, LSHIFT]), 2, true), Verdict::Pass);
        assert_eq!(m.key(&b, mods(&[LCTRL, LALT, LSHIFT]), 2, false), Verdict::Pass);
        assert_eq!(m.key(&b, mods(&[LCTRL]), 2, true), Verdict::Pass);
        assert_eq!(m.key(&b, mods(&[LCTRL]), 2, false), Verdict::Pass);
    }

    #[test]
    fn matcher_never_fires_mid_chord() {
        let b = bindings();
        let mut m = Matcher::default();
        let ca = mods(&[LCTRL, LALT]);
        // `x` held, then the chord's key: something else is being typed.
        assert_eq!(m.key(&b, mods(&[]), 45, true), Verdict::Pass);
        assert_eq!(m.key(&b, ca, 2, true), Verdict::Pass);
        assert_eq!(m.key(&b, ca, 45, false), Verdict::Pass);
        assert_eq!(m.key(&b, ca, 2, true), Verdict::Pass, "still held: a repeat");
        assert_eq!(m.key(&b, ca, 2, false), Verdict::Pass, "the release of a key sent on is sent on");
        assert_eq!(m.key(&b, ca, 2, true), Verdict::Fire(Action::Jump(1)), "alone now");
        // A key pressed during the chord's key passes.
        assert_eq!(m.key(&b, ca, 4, true), Verdict::Pass);
        assert_eq!(m.key(&b, ca, 4, false), Verdict::Pass);
        assert_eq!(m.key(&b, ca, 2, false), Verdict::Swallow);
        // A capture that ended with a key held doesn't block the next one.
        assert_eq!(m.key(&b, mods(&[]), 45, true), Verdict::Pass);
        m.reset();
        assert_eq!(m.key(&b, ca, 2, true), Verdict::Fire(Action::Jump(1)));
    }

    #[test]
    fn modifiers_held_map_to_chord_mods() {
        assert_eq!(ChordMods::from_held(mods(&[LCTRL, LALT, 126])), chord("ctrl+alt+super+1").mods);
        assert_eq!(ChordMods::from_held(mods(&[54])), ChordMods::SHIFT);
        // AltGr (right Alt; on Windows with a left Ctrl) is not Ctrl+Alt.
        assert_eq!(ChordMods::from_held(mods(&[LCTRL, 100])), ChordMods::CTRL);
        let b = bindings();
        assert_eq!(Matcher::default().key(&b, mods(&[LCTRL, 100]), 8, true), Verdict::Pass, "AltGr+7 types");
        assert_eq!(ChordMods::from_held(mods(&[])), ChordMods::default());
    }

    #[test]
    fn one_press_seen_twice_counts_once() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut d = Debounce::default();
        assert!(d.admit(Action::Cycle, Source::Captured, t));
        assert!(!d.admit(Action::Cycle, Source::System, ms(20)), "the same press, reported twice");
        assert!(d.admit(Action::Cycle, Source::System, ms(40)), "a second press");
        assert!(d.admit(Action::Cycle, Source::System, ms(80)), "fast presses from one source all count");
        assert!(d.admit(Action::Home, Source::Captured, ms(90)), "another shortcut");
        assert!(d.admit(Action::Home, Source::System, ms(90) + DEBOUNCE), "too late to be the same press");
    }
}
