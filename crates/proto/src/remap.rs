//! The key mapper: what a key (or mouse button) becomes on the computer
//! input is sent to. Remaps, combos (`ctrl+h` -> Backspace), sequences,
//! tap/hold keys, layers and actions. Pure logic: the daemon feeds it
//! physical key transitions and sends what it emits. See
//! `docs/INPUT-TUNING.md` for the design.
//!
//! Per event it does a table lookup and a few counter updates, and never
//! allocates; the tables are built once per visit ([`Keymap::compile`]).
//!
//! The one guarantee everything else rests on: every key-down it emits
//! gets exactly one key-up, whatever happens in between (layers ending,
//! the keymap changing, the visit ending). It holds by construction: what
//! is down on the wire is *derived* (a code is down iff something holds it
//! and no combo hides it), and only changes of that derived state are
//! emitted.

use crate::keynames::{self, CODE_LIMIT, Chord, MOD_CODES, NameError, Trigger, mod_bit};
use crate::{HeldKeys, os};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Named layers a keymap may have.
pub const MAX_LAYERS: usize = 8;
/// Steps in a sequence.
pub const MAX_SEQUENCE: usize = 16;
/// Rules in one keymap.
pub const MAX_RULES: usize = 4096;
/// How long a tap/hold key waits before it counts as held.
pub const DEFAULT_TAP: Duration = Duration::from_millis(200);
/// Bounds for that wait.
pub const MIN_TAP_MS: u32 = 50;
pub const MAX_TAP_MS: u32 = 2000;

const KEYS: usize = CODE_LIMIT as usize;

// ------------------------------------------------------------- outputs

/// Something a key can do instead of typing: the same things `one-mk
/// switch`, `lock-pointer` and `lock-all` do, scrolling, or a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `switch next`, `switch here` or `switch COMPUTER` (as `one-mk switch` takes).
    Switch(String),
    /// `lock-pointer [on|off|toggle]`: `None` toggles.
    LockPointer(Option<bool>),
    LockAll,
    /// `run: COMMAND`, split into words like a `[hooks]` command (no shell).
    Run(String),
    /// `scroll up|down|left|right [N]`: N wheel notches, in 1/120 notch.
    Scroll { dx: i32, dy: i32 },
}

/// Browser back and forward, as the target's OS spells them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Back,
    Forward,
}

/// What a key becomes, as written in a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Held as long as the key is (`esc`, `ctrl+shift+esc`).
    Chord(Chord),
    /// Typed once when the key goes down (`["ctrl+a", "ctrl+c"]`).
    Sequence(Vec<Chord>),
    /// The key does nothing.
    None,
    /// `layer:NAME`: the layer is on while the key is held.
    Layer(String),
    /// `back` or `forward` for the target OS.
    Nav(Nav),
    /// Runs once when the key goes down.
    Action(Action),
}

fn err<T>(msg: impl Into<String>) -> Result<T, NameError> {
    Err(NameError(msg.into()))
}

fn valid_layer(name: &str) -> Result<String, NameError> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok { Ok(name.to_ascii_lowercase()) } else { err(format!("layer name {name:?}: use 1 to 32 letters, digits, - or _")) }
}

impl FromStr for Output {
    type Err = NameError;

    fn from_str(s: &str) -> Result<Self, NameError> {
        let t = s.trim();
        let lower = t.to_ascii_lowercase();
        let (head, rest) = match lower.split_once(char::is_whitespace) {
            Some((h, r)) => (h, r.trim()),
            None => (lower.as_str(), ""),
        };
        let original_rest = || t.split_once(char::is_whitespace).map_or("", |(_, r)| r.trim());
        if t.is_empty() {
            return err("nothing given (a key, a chord like ctrl+c, `none`, an action like `switch next`)");
        }
        if let Some(cmd) = lower.strip_prefix("run:") {
            let cmd = t[t.len() - cmd.len()..].trim();
            if cmd.is_empty() {
                return err("`run:` needs a command, like `run: notify-send hello`");
            }
            return Ok(Output::Action(Action::Run(cmd.to_owned())));
        }
        if let Some(name) = lower.strip_prefix("layer:") {
            return Ok(Output::Layer(valid_layer(name.trim())?));
        }
        Ok(match (head, rest) {
            ("none" | "disabled" | "nothing" | "off", "") => Output::None,
            ("back", "") => Output::Nav(Nav::Back),
            ("forward", "") => Output::Nav(Nav::Forward),
            ("switch", "") => return err("`switch` needs `next`, `here` or a computer's name"),
            ("switch", _) => {
                let to = original_rest();
                if to.contains(['\n', '\r']) {
                    return err("a computer's name can't contain a line break");
                }
                let to = if matches!(rest, "next" | "here") { rest } else { to };
                Output::Action(Action::Switch(to.to_owned()))
            }
            ("lock-pointer" | "lockpointer", r) => Output::Action(Action::LockPointer(match r {
                "" | "toggle" => None,
                "on" => Some(true),
                "off" => Some(false),
                r => return err(format!("lock-pointer {r:?}: say on, off or toggle")),
            })),
            ("lock-all" | "lockall", "") => Output::Action(Action::LockAll),
            ("scroll", r) => {
                let mut w = r.split_whitespace();
                let dir = w.next().unwrap_or("");
                let n = match w.next() {
                    None => 1,
                    Some(n) => match n.parse::<i32>() {
                        Ok(n) if (1..=100).contains(&n) => n,
                        _ => return err(format!("scroll {r}: the number of notches goes from 1 to 100")),
                    },
                };
                if w.next().is_some() {
                    return err(format!("scroll {r}: say a direction and at most a number, like `scroll down 3`"));
                }
                let u = 120 * n;
                Output::Action(match dir {
                    "up" => Action::Scroll { dx: 0, dy: -u },
                    "down" => Action::Scroll { dx: 0, dy: u },
                    "left" => Action::Scroll { dx: -u, dy: 0 },
                    "right" => Action::Scroll { dx: u, dy: 0 },
                    _ => return err(format!("scroll {r:?}: say up, down, left or right")),
                })
            }
            _ => Output::Chord(t.parse()?),
        })
    }
}

impl Output {
    /// A sequence from its steps (an array in `settings.toml`, several
    /// words on the command line). One step is still a sequence: typed
    /// once on press, not held.
    pub fn sequence<S: AsRef<str>>(steps: &[S]) -> Result<Output, NameError> {
        if steps.is_empty() {
            return err("an empty sequence (use `none` to make a key do nothing)");
        }
        if steps.len() > MAX_SEQUENCE {
            return err(format!("a sequence has at most {MAX_SEQUENCE} steps, not {}", steps.len()));
        }
        let chords = steps.iter().map(|s| s.as_ref().parse::<Chord>()).collect::<Result<Vec<_>, _>>()?;
        Ok(Output::Sequence(chords))
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Switch(to) => write!(f, "switch {to}"),
            Action::LockPointer(None) => f.write_str("lock-pointer toggle"),
            Action::LockPointer(Some(on)) => write!(f, "lock-pointer {}", if *on { "on" } else { "off" }),
            Action::LockAll => f.write_str("lock-all"),
            Action::Run(cmd) => write!(f, "run: {cmd}"),
            Action::Scroll { dx, dy } => {
                let (dir, n) = match (dx.signum(), dy.signum()) {
                    (0, -1) => ("up", -dy),
                    (0, _) => ("down", *dy),
                    (-1, _) => ("left", -dx),
                    _ => ("right", *dx),
                };
                if n / 120 == 1 { write!(f, "scroll {dir}") } else { write!(f, "scroll {dir} {}", n / 120) }
            }
        }
    }
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Output::Chord(c) => write!(f, "{c}"),
            Output::Sequence(s) => {
                f.write_str("[")?;
                for (i, c) in s.iter().enumerate() {
                    write!(f, "{}{c}", if i > 0 { ", " } else { "" })?;
                }
                f.write_str("]")
            }
            Output::None => f.write_str("none"),
            Output::Layer(l) => write!(f, "layer:{l}"),
            Output::Nav(Nav::Back) => f.write_str("back"),
            Output::Nav(Nav::Forward) => f.write_str("forward"),
            Output::Action(a) => write!(f, "{a}"),
        }
    }
}

// ----------------------------------------------------------------- rules

/// What a rule does with its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    To(Output),
    /// Tapped, `tap`; held past `timeout` (or while another key is
    /// pressed), `hold`.
    TapHold { tap: Output, hold: Output, timeout: Option<Duration> },
}

/// One rule, as written (already parsed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSpec {
    pub from: Trigger,
    /// The layer the rule belongs to; `None` for the base rules.
    pub layer: Option<String>,
    pub what: What,
}

/// The computer input is sent to, for what depends on it (`back`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// One of [`os`].
    pub os: u8,
    /// Ctrl and Cmd are swapped after the mapper (see `swap_ctrl_meta`).
    pub swap: bool,
}

impl Target {
    /// Back/forward as typed on this keyboard, so that after the swap the
    /// target gets Alt+Left/Right, or Cmd+[ / Cmd+] on a Mac.
    fn nav(self, nav: Nav) -> Chord {
        if self.os == os::MACOS {
            let cmd = if self.swap { keynames::LCTRL } else { keynames::LMETA };
            Chord { mods: cmd, key: if nav == Nav::Back { 26 } else { 27 } }
        } else {
            Chord { mods: keynames::LALT, key: if nav == Nav::Back { 105 } else { 106 } }
        }
    }
}

/// An output, compiled: codes and indexes, `Copy`, no references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    None,
    Chord(Chord),
    Seq { start: u16, len: u16 },
    Action(u16),
    Layer(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Plain(Out),
    Dual { tap: Out, hold: Out, timeout: Duration },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rule {
    trigger: Trigger,
    layer: u8,
    kind: Kind,
}

/// Rules compiled for one target: per layer, a table over the whole code
/// space pointing at each key's rules (most specific first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keymap {
    rules: Vec<Rule>,
    /// `(start, len)` into `rules`, `KEYS` entries per layer (layer 0 is
    /// the base). Empty when there are no rules: every key is itself.
    slots: Vec<(u16, u16)>,
    layer_names: Vec<String>,
    chords: Vec<Chord>,
    actions: Vec<Arc<Action>>,
}

/// A compiled keymap and what is odd about the rules (they still work).
#[derive(Clone, Debug, Default)]
pub struct Compiled {
    pub map: Keymap,
    pub warnings: Vec<String>,
}

impl Keymap {
    /// Builds the tables. Errors are rules that can't work at all; odd but
    /// usable things (a duplicate rule, a layer never switched on) are
    /// warnings. Of two rules for the same key, layer and modifiers, the
    /// first counts.
    pub fn compile(specs: &[RuleSpec], target: Target, tap: Duration) -> Result<Compiled, NameError> {
        if specs.len() > MAX_RULES {
            return err(format!("at most {MAX_RULES} key rules, not {}", specs.len()));
        }
        let mut map = Keymap::default();
        let mut warnings = Vec::new();
        let mut activated = Vec::new();
        let layer = |map: &mut Keymap, name: &str| -> Result<u8, NameError> {
            if let Some(i) = map.layer_names.iter().position(|n| n == name) {
                return Ok(i as u8 + 1);
            }
            if map.layer_names.len() == MAX_LAYERS {
                return err(format!("at most {MAX_LAYERS} layers (layer {name:?} is one too many)"));
            }
            map.layer_names.push(name.to_owned());
            Ok(map.layer_names.len() as u8)
        };
        let mut seen = std::collections::HashSet::new();
        for spec in specs {
            let where_ = || match &spec.layer {
                Some(l) => format!("{} (layer {l})", spec.from),
                None => spec.from.to_string(),
            };
            if u32::from(spec.from.key) >= CODE_LIMIT {
                return err(format!("{}: no such key", where_()));
            }
            let l = match &spec.layer {
                Some(name) => layer(&mut map, &valid_layer(name)?)?,
                None => 0,
            };
            let out = |map: &mut Keymap, o: &Output, activated: &mut Vec<u8>| -> Result<Out, NameError> {
                Ok(match o {
                    Output::None => Out::None,
                    Output::Chord(c) => Out::Chord(*c),
                    Output::Nav(n) => Out::Chord(target.nav(*n)),
                    Output::Sequence(s) => {
                        if s.is_empty() || s.len() > MAX_SEQUENCE {
                            return err(format!("{}: a sequence has 1 to {MAX_SEQUENCE} steps", where_()));
                        }
                        let start = map.chords.len() as u16;
                        map.chords.extend_from_slice(s);
                        Out::Seq { start, len: s.len() as u16 }
                    }
                    Output::Action(a) => {
                        map.actions.push(Arc::new(a.clone()));
                        Out::Action(map.actions.len() as u16 - 1)
                    }
                    Output::Layer(name) => {
                        let i = layer(map, &valid_layer(name)?)?;
                        activated.push(i);
                        Out::Layer(i)
                    }
                })
            };
            let kind = match &spec.what {
                What::To(o) => Kind::Plain(out(&mut map, o, &mut activated)?),
                What::TapHold { tap: t, hold, timeout } => {
                    if spec.from.has_mods() {
                        return err(format!("{}: a tap/hold key can't need modifiers", where_()));
                    }
                    if matches!(t, Output::Layer(_)) {
                        return err(format!("{}: `tap` can't be a layer (use `hold = \"layer:...\"`)", where_()));
                    }
                    let timeout = timeout.unwrap_or(tap);
                    let ms = timeout.as_millis();
                    if !(u128::from(MIN_TAP_MS)..=u128::from(MAX_TAP_MS)).contains(&ms) {
                        return err(format!("{}: the tap time goes from {MIN_TAP_MS} to {MAX_TAP_MS} ms, not {ms}", where_()));
                    }
                    Kind::Dual { tap: out(&mut map, t, &mut activated)?, hold: out(&mut map, hold, &mut activated)?, timeout }
                }
            };
            if !seen.insert((l, spec.from)) {
                warnings.push(format!("{}: more than one rule; the first one counts", where_()));
                continue;
            }
            map.rules.push(Rule { trigger: spec.from, layer: l, kind });
        }
        for (i, name) in map.layer_names.iter().enumerate() {
            let i = i as u8 + 1;
            if !activated.contains(&i) {
                warnings.push(format!("layer {name:?} is never switched on (no key has `layer:{name}`)"));
            } else if !map.rules.iter().any(|r| r.layer == i) {
                warnings.push(format!("layer {name:?} has no rules"));
            }
        }
        if map.rules.is_empty() {
            return Ok(Compiled { map: Keymap::default(), warnings });
        }
        // Per layer and key, the most specific trigger first.
        map.rules.sort_by_key(|r| (r.layer, r.trigger.key, std::cmp::Reverse(r.trigger.specificity())));
        map.slots = vec![(0, 0); (map.layer_names.len() + 1) * KEYS];
        for (i, r) in map.rules.iter().enumerate() {
            let slot = &mut map.slots[usize::from(r.layer) * KEYS + usize::from(r.trigger.key)];
            if slot.1 == 0 {
                slot.0 = i as u16;
            }
            slot.1 += 1;
        }
        Ok(Compiled { map, warnings })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rule for `code` in `layer` with the modifiers `held`.
    fn find(&self, layer: u8, code: u32, held: u8) -> Option<usize> {
        let (start, len) = *self.slots.get(usize::from(layer) * KEYS + code as usize)?;
        (usize::from(start)..usize::from(start) + usize::from(len)).find(|&i| self.rules[i].trigger.matches(held))
    }
}

// --------------------------------------------------------------- mapper

/// What the mapper says to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Emit {
    /// Send this key (or button) transition. A down for a key already
    /// down is an auto-repeat.
    Key { code: u32, down: bool },
    /// Run this action. Shared with the keymap it came from, not an index
    /// into it: a keymap replaced before it runs can't change it.
    Action(Arc<Action>),
}

/// What a physical key's press produced: its release undoes exactly this.
/// Codes, not references into the keymap, so a new keymap can't change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rec {
    Up,
    /// Produced nothing that lasts (or was cut off): its release does nothing.
    Swallow,
    /// Holds `key` and the modifiers `mods`, hides the modifiers `hidden`;
    /// resolved in `layer`.
    Held { key: u16, mods: u8, hidden: u8, layer: u8 },
    /// Keeps `layer` on; resolved in layer `from`.
    Layer { layer: u8, from: u8 },
    /// A tap/hold key not decided yet.
    Pending,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    code: u16,
    rule: u16,
    layer: u8,
    deadline: Instant,
}

/// The key mapper for one visit.
pub struct Mapper {
    map: Keymap,
    recs: Box<[Rec]>,
    /// How many things hold each code down.
    holders: Box<[u16]>,
    /// How many things hide each modifier (by bit index).
    hide: [u16; 8],
    /// Modifiers hidden until released (modifier-scroll).
    lifted: u8,
    /// Modifiers shown whatever hides them, for the chord being typed.
    force: u8,
    /// What is down on the wire.
    wire: HeldKeys,
    /// Physical keys down.
    phys: HeldKeys,
    /// How many keys hold each layer on (index 0 unused).
    layer_on: [u8; MAX_LAYERS + 1],
    /// The layers on, in the order they came on.
    order: [u8; MAX_LAYERS],
    n_order: usize,
    pending: Option<Pending>,
}

impl Mapper {
    pub fn new(map: Keymap) -> Self {
        Mapper {
            map,
            recs: vec![Rec::Up; KEYS].into_boxed_slice(),
            holders: vec![0; KEYS].into_boxed_slice(),
            hide: [0; 8],
            lifted: 0,
            force: 0,
            wire: HeldKeys::new(),
            phys: HeldKeys::new(),
            layer_on: [0; MAX_LAYERS + 1],
            order: [0; MAX_LAYERS],
            n_order: 0,
            pending: None,
        }
    }

    pub fn keymap(&self) -> &Keymap {
        &self.map
    }

    /// A physical key (or button) went down or up; `down` again while down
    /// is an auto-repeat.
    pub fn key(&mut self, code: u32, down: bool, now: Instant, out: &mut impl FnMut(Emit)) {
        if code >= CODE_LIMIT {
            return;
        }
        // A tap/hold key past its time was held, even if this event comes
        // before the timer does (a busy loop): its release is not a tap.
        self.timer(now, out);
        if down {
            if self.phys.is_held(code) {
                return self.repeat(code, out);
            }
            self.phys.update(code, true);
            // Another key while a tap/hold key is undecided: it's held.
            if self.pending.is_some() {
                self.resolve_hold(out);
            }
            self.press(code, now, out);
        } else {
            if !self.phys.is_held(code) {
                return;
            }
            self.phys.update(code, false);
            if self.pending.is_some_and(|p| u32::from(p.code) == code) {
                return self.resolve_tap(out);
            }
            self.release(code, out);
        }
    }

    /// When [`Mapper::timer`] must be called: only while a tap/hold key is
    /// undecided.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|p| p.deadline)
    }

    /// Time passed: an undecided tap/hold key held past its time is held.
    pub fn timer(&mut self, now: Instant, out: &mut impl FnMut(Emit)) {
        if self.pending.is_some_and(|p| now >= p.deadline) {
            self.resolve_hold(out);
        }
    }

    /// A click or scroll: an undecided tap/hold key is being held for it.
    pub fn interrupt(&mut self, out: &mut impl FnMut(Emit)) {
        if self.pending.is_some() {
            self.resolve_hold(out);
        }
    }

    /// The modifiers held as far as the target is concerned (sided bits),
    /// hidden or not.
    pub fn mods(&self) -> u8 {
        let mut bits = 0;
        for (i, &c) in MOD_CODES.iter().enumerate() {
            if self.holders[usize::from(c)] > 0 {
                bits |= 1 << i;
            }
        }
        bits
    }

    /// Takes the modifiers `bits` off the target until they are released
    /// here (modifier-scroll: Shift turned scrolling sideways, so the
    /// target must not see Shift).
    pub fn lift(&mut self, bits: u8, out: &mut impl FnMut(Emit)) {
        let new = bits & self.mods() & !self.lifted;
        self.lifted |= new;
        self.hide_mods(new, out);
    }

    /// Whether anything is down on the wire.
    pub fn is_idle(&self) -> bool {
        self.wire.is_empty() && self.pending.is_none()
    }

    /// Lets go of everything emitted (key-ups for all of it). Keys still
    /// physically down do nothing more until they come up.
    pub fn release_all(&mut self, out: &mut impl FnMut(Emit)) {
        for code in self.wire.iter() {
            out(Emit::Key { code, down: false });
        }
        self.wire = HeldKeys::new();
        self.holders.fill(0);
        self.hide = [0; 8];
        self.lifted = 0;
        self.layer_on = [0; MAX_LAYERS + 1];
        self.n_order = 0;
        self.pending = None;
        self.recs.fill(Rec::Up);
        for code in self.phys.iter() {
            self.recs[code as usize] = Rec::Swallow;
        }
    }

    /// Uses `map` from now on. The same keymap changes nothing. Otherwise
    /// an undecided tap/hold key is held, every layer ends (releasing what
    /// was typed in it), and what other keys hold stays held until they
    /// come up.
    pub fn set_keymap(&mut self, map: Keymap, out: &mut impl FnMut(Emit)) {
        if map == self.map {
            return;
        }
        if self.pending.is_some() {
            self.resolve_hold(out);
        }
        let phys = self.phys.clone();
        for code in phys.iter() {
            if let Rec::Layer { .. } = self.recs[code as usize] {
                self.release(code, out);
                self.recs[code as usize] = Rec::Swallow;
            }
        }
        self.map = map;
    }

    // ---------------------------------------------------------- inside

    fn press(&mut self, code: u32, now: Instant, out: &mut impl FnMut(Emit)) {
        let held = self.mods();
        let Some((i, layer)) = self.lookup(code, held) else {
            return self.apply(code, Out::Chord(Chord::key(code as u16)), 0, 0, out);
        };
        let rule = self.map.rules[i];
        match rule.kind {
            Kind::Plain(o) => self.apply(code, o, rule.trigger.used(held), layer, out),
            Kind::Dual { timeout, .. } => {
                self.recs[code as usize] = Rec::Pending;
                self.pending = Some(Pending { code: code as u16, rule: i as u16, layer, deadline: now + timeout });
            }
        }
    }

    /// The most recently switched-on layer with a rule for `code`, then
    /// the base rules.
    fn lookup(&self, code: u32, held: u8) -> Option<(usize, u8)> {
        if self.map.slots.is_empty() {
            return None;
        }
        for &l in self.order[..self.n_order].iter().rev() {
            if let Some(i) = self.map.find(l, code, held) {
                return Some((i, l));
            }
        }
        self.map.find(0, code, held).map(|i| (i, 0))
    }

    /// Key `code` went down and does `o`; `used` are the modifiers its
    /// trigger needed (hidden from the target while a chord is held).
    fn apply(&mut self, code: u32, o: Out, used: u8, layer: u8, out: &mut impl FnMut(Emit)) {
        let rec = match o {
            Out::None => Rec::Swallow,
            Out::Chord(c) => {
                let hidden = used & !c.mods;
                self.hide_mods(hidden, out);
                self.hold_chord(c, out);
                Rec::Held { key: c.key, mods: c.mods, hidden, layer }
            }
            Out::Seq { start, len } => {
                self.sequence(start, len, used, out);
                Rec::Swallow
            }
            Out::Action(a) => {
                out(Emit::Action(self.map.actions[usize::from(a)].clone()));
                Rec::Swallow
            }
            Out::Layer(l) => {
                self.layer_up(l);
                Rec::Layer { layer: l, from: layer }
            }
        };
        self.recs[code as usize] = rec;
    }

    /// Something that happens once (a tap): a chord pressed and released.
    fn once(&mut self, o: Out, out: &mut impl FnMut(Emit)) {
        match o {
            Out::None | Out::Layer(_) => {}
            Out::Chord(c) => self.pulse(c, true, out),
            Out::Seq { start, len } => self.sequence(start, len, 0, out),
            Out::Action(a) => out(Emit::Action(self.map.actions[usize::from(a)].clone())),
        }
    }

    fn sequence(&mut self, start: u16, len: u16, used: u8, out: &mut impl FnMut(Emit)) {
        for i in usize::from(start)..usize::from(start) + usize::from(len) {
            let c = self.map.chords[i];
            let hidden = used & !c.mods;
            self.hide_mods(hidden, out);
            let end = usize::from(start) + usize::from(len);
            let last = !self.map.chords[i + 1..end].iter().any(|next| next.key == c.key);
            self.pulse(c, last, out);
            self.unhide_mods(hidden, out);
        }
    }

    fn release(&mut self, code: u32, out: &mut impl FnMut(Emit)) {
        let rec = std::mem::replace(&mut self.recs[code as usize], Rec::Up);
        match rec {
            Rec::Up | Rec::Swallow | Rec::Pending => {}
            Rec::Held { key, mods, hidden, .. } => {
                self.release_chord(Chord { mods, key }, out);
                self.unhide_mods(hidden, out);
            }
            Rec::Layer { layer, .. } => self.layer_down(layer, out),
        }
    }

    fn repeat(&mut self, code: u32, out: &mut impl FnMut(Emit)) {
        if let Rec::Held { key, .. } = self.recs[code as usize]
            && self.wire.is_held(u32::from(key))
        {
            out(Emit::Key { code: u32::from(key), down: true });
        }
    }

    fn resolve_tap(&mut self, out: &mut impl FnMut(Emit)) {
        let Some(p) = self.pending.take() else { return };
        self.recs[usize::from(p.code)] = Rec::Up;
        if let Kind::Dual { tap, .. } = self.map.rules[usize::from(p.rule)].kind {
            self.once(tap, out);
        }
    }

    fn resolve_hold(&mut self, out: &mut impl FnMut(Emit)) {
        let Some(p) = self.pending.take() else { return };
        if let Kind::Dual { hold, .. } = self.map.rules[usize::from(p.rule)].kind {
            self.apply(u32::from(p.code), hold, 0, p.layer, out);
        }
    }

    fn layer_up(&mut self, l: u8) {
        let n = &mut self.layer_on[usize::from(l)];
        *n += 1;
        if *n == 1 && self.n_order < MAX_LAYERS {
            self.order[self.n_order] = l;
            self.n_order += 1;
        }
    }

    /// A key holding layer `l` came up; if it was the last one, everything
    /// resolved in the layer is released (and swallowed until it comes up).
    fn layer_down(&mut self, l: u8, out: &mut impl FnMut(Emit)) {
        let n = &mut self.layer_on[usize::from(l)];
        *n = n.saturating_sub(1);
        if *n > 0 {
            return;
        }
        if let Some(at) = self.order[..self.n_order].iter().position(|&x| x == l) {
            self.order.copy_within(at + 1..self.n_order, at);
            self.n_order -= 1;
        }
        if self.pending.is_some_and(|p| p.layer == l) {
            let p = self.pending.take().expect("checked");
            self.recs[usize::from(p.code)] = Rec::Swallow;
        }
        let phys = self.phys.clone();
        for code in phys.iter() {
            let in_layer = match self.recs[code as usize] {
                Rec::Held { layer, .. } => layer == l,
                Rec::Layer { from, .. } => from == l,
                _ => false,
            };
            if in_layer {
                self.release(code, out);
                self.recs[code as usize] = Rec::Swallow;
            }
        }
    }

    // ------------------------------------------------- the derived wire

    /// Each tap has one fresh down. The last tap of an already-held key
    /// leaves that down for its holder to release, if it should be down
    /// at all (a combo may hide it): the state `sync` would produce.
    fn pulse(&mut self, c: Chord, last: bool, out: &mut impl FnMut(Emit)) {
        self.force = c.mods;
        for (i, &m) in MOD_CODES.iter().enumerate() {
            if c.mods & (1 << i) != 0 {
                self.hold(u32::from(m), out);
            }
        }
        let key = u32::from(c.key);
        if self.wire.is_held(key) {
            self.wire.update(key, false);
            out(Emit::Key { code: key, down: false });
        }
        out(Emit::Key { code: key, down: true });
        self.wire.update(key, true);
        if !last || !self.wanted(key) {
            out(Emit::Key { code: key, down: false });
            self.wire.update(key, false);
        }
        self.force = 0;
        for (i, &m) in MOD_CODES.iter().enumerate() {
            if c.mods & (1 << i) != 0 {
                self.unhold(u32::from(m), out);
            }
        }
    }

    fn hold_chord(&mut self, c: Chord, out: &mut impl FnMut(Emit)) {
        for (i, &m) in MOD_CODES.iter().enumerate() {
            if c.mods & (1 << i) != 0 {
                self.hold(u32::from(m), out);
            }
        }
        self.hold(u32::from(c.key), out);
    }

    fn release_chord(&mut self, c: Chord, out: &mut impl FnMut(Emit)) {
        self.unhold(u32::from(c.key), out);
        for (i, &m) in MOD_CODES.iter().enumerate() {
            if c.mods & (1 << i) != 0 {
                self.unhold(u32::from(m), out);
            }
        }
    }

    fn hold(&mut self, code: u32, out: &mut impl FnMut(Emit)) {
        self.holders[code as usize] += 1;
        self.sync(code, out);
    }

    fn unhold(&mut self, code: u32, out: &mut impl FnMut(Emit)) {
        let n = &mut self.holders[code as usize];
        *n = n.saturating_sub(1);
        let gone = *n == 0;
        self.sync(code, out);
        // A modifier lifted for scrolling comes back once it is let go.
        let bit = mod_bit(code);
        if gone && self.lifted & bit != 0 {
            self.lifted &= !bit;
            self.unhide_mods(bit, out);
        }
    }

    fn hide_mods(&mut self, bits: u8, out: &mut impl FnMut(Emit)) {
        for (i, &m) in MOD_CODES.iter().enumerate() {
            if bits & (1 << i) != 0 {
                self.hide[i] += 1;
                self.sync(u32::from(m), out);
            }
        }
    }

    fn unhide_mods(&mut self, bits: u8, out: &mut impl FnMut(Emit)) {
        for (i, &m) in MOD_CODES.iter().enumerate() {
            if bits & (1 << i) != 0 {
                self.hide[i] = self.hide[i].saturating_sub(1);
                self.sync(u32::from(m), out);
            }
        }
    }

    /// Whether `code` should be down on the wire: something holds it and
    /// no combo hides it (unless the chord being typed needs it).
    fn wanted(&self, code: u32) -> bool {
        let bit = mod_bit(code);
        let hidden = bit != 0 && self.hide[bit.trailing_zeros() as usize] > 0 && self.force & bit == 0;
        self.holders[code as usize] > 0 && !hidden
    }

    /// Emits a transition if what should be down on the wire for `code`
    /// differs from what is.
    fn sync(&mut self, code: u32, out: &mut impl FnMut(Emit)) {
        let want = self.wanted(code);
        if want != self.wire.is_held(code) {
            self.wire.update(code, want);
            out(Emit::Key { code, down: want });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: u32 = 58;
    const ESC: u32 = 1;
    const LCTRL: u32 = 29;
    const RCTRL: u32 = 97;
    const LALT: u32 = 56;
    const LMETA: u32 = 125;
    const RALT: u32 = 100;
    const H: u32 = 35;
    const J: u32 = 36;
    const Q: u32 = 16;
    const A: u32 = 30;
    const C: u32 = 46;
    const LEFT: u32 = 105;
    const DOWN: u32 = 108;
    const BACKSPACE: u32 = 14;
    const LSHIFT: u32 = 42;

    fn rule(from: &str, to: &str) -> RuleSpec {
        RuleSpec { from: from.parse().unwrap(), layer: None, what: What::To(to.parse().unwrap()) }
    }

    fn in_layer(layer: &str, from: &str, to: &str) -> RuleSpec {
        RuleSpec { layer: Some(layer.into()), ..rule(from, to) }
    }

    fn dual(from: &str, tap: &str, hold: &str) -> RuleSpec {
        RuleSpec {
            from: from.parse().unwrap(),
            layer: None,
            what: What::TapHold { tap: tap.parse().unwrap(), hold: hold.parse().unwrap(), timeout: None },
        }
    }

    const PC: Target = Target { os: os::LINUX, swap: false };

    fn map(specs: &[RuleSpec]) -> Keymap {
        Keymap::compile(specs, PC, DEFAULT_TAP).unwrap().map
    }

    /// A mapper and a clock, recording what it emits.
    struct T {
        m: Mapper,
        t0: Instant,
        now: Instant,
        out: Vec<Emit>,
    }

    impl T {
        fn new(specs: &[RuleSpec]) -> T {
            let t0 = Instant::now();
            T { m: Mapper::new(map(specs)), t0, now: t0, out: Vec::new() }
        }
        fn key(&mut self, code: u32, down: bool) -> Vec<Emit> {
            let mut out = Vec::new();
            self.m.key(code, down, self.now, &mut |e| out.push(e));
            self.out.extend(out.iter().cloned());
            out
        }
        fn tap(&mut self, code: u32) -> Vec<Emit> {
            let mut v = self.key(code, true);
            v.extend(self.key(code, false));
            v
        }
        fn wait(&mut self, ms: u64) -> Vec<Emit> {
            self.now += Duration::from_millis(ms);
            let mut out = Vec::new();
            self.m.timer(self.now, &mut |e| out.push(e));
            self.out.extend(out.iter().cloned());
            out
        }
    }

    fn d(code: u32) -> Emit {
        Emit::Key { code, down: true }
    }
    fn u(code: u32) -> Emit {
        Emit::Key { code, down: false }
    }

    #[test]
    fn no_rules_passes_everything_through() {
        let mut t = T::new(&[]);
        assert_eq!(t.key(A, true), [d(A)]);
        assert_eq!(t.key(A, true), [d(A)], "auto-repeat");
        assert_eq!(t.key(A, false), [u(A)]);
        assert_eq!(t.key(A, false), [], "a release of a key not down");
        assert!(t.m.is_idle());
    }

    #[test]
    fn simple_remaps_and_swaps() {
        let mut t = T::new(&[rule("capslock", "esc"), rule("lalt", "lmeta"), rule("lmeta", "lalt")]);
        assert_eq!(t.tap(CAPS), [d(ESC), u(ESC)]);
        assert_eq!(t.key(LALT, true), [d(LMETA)]);
        assert_eq!(t.key(LMETA, true), [d(LALT)]);
        assert_eq!(t.key(LALT, false), [u(LMETA)]);
        assert_eq!(t.key(LMETA, false), [u(LALT)]);
    }

    #[test]
    fn a_disabled_key_does_nothing() {
        let mut t = T::new(&[rule("capslock", "none")]);
        assert_eq!(t.tap(CAPS), []);
        assert_eq!(t.key(CAPS, true), []);
        assert_eq!(t.key(CAPS, true), [], "nor repeats");
        assert_eq!(t.key(CAPS, false), []);
    }

    #[test]
    fn combos_hide_their_modifiers() {
        let mut t = T::new(&[rule("ctrl+h", "backspace"), rule("meta+q", "none")]);
        assert_eq!(t.key(RCTRL, true), [d(RCTRL)]);
        assert_eq!(t.key(H, true), [u(RCTRL), d(BACKSPACE)]);
        assert_eq!(t.key(H, true), [d(BACKSPACE)], "repeats the mapped key");
        assert_eq!(t.key(H, false), [u(BACKSPACE), d(RCTRL)]);
        assert_eq!(t.key(RCTRL, false), [u(RCTRL)]);
        // Without Ctrl, h is h.
        assert_eq!(t.tap(H), [d(H), u(H)]);
        // Cmd+Q does nothing (Cmd itself still goes through).
        assert_eq!(t.key(LMETA, true), [d(LMETA)]);
        assert_eq!(t.tap(Q), []);
        assert_eq!(t.key(LMETA, false), [u(LMETA)]);
    }

    #[test]
    fn modifiers_released_during_a_combo_are_not_stuck() {
        let mut t = T::new(&[rule("ctrl+h", "backspace")]);
        t.key(LCTRL, true);
        t.key(H, true);
        // Ctrl comes up first: it was hidden, so nothing is sent for it now...
        assert_eq!(t.key(LCTRL, false), []);
        // ...and Backspace's release doesn't bring it back.
        assert_eq!(t.key(H, false), [u(BACKSPACE)]);
        assert!(t.m.is_idle());
    }

    #[test]
    fn the_most_specific_combo_wins_and_extra_modifiers_pass() {
        let mut t = T::new(&[rule("h", "left"), rule("ctrl+h", "backspace"), rule("ctrl+shift+h", "a")]);
        assert_eq!(t.tap(H), [d(LEFT), u(LEFT)]);
        t.key(LCTRL, true);
        t.key(LSHIFT, true);
        assert_eq!(t.tap(H), [u(LCTRL), u(LSHIFT), d(A), u(A), d(LCTRL), d(LSHIFT)]);
        t.key(LSHIFT, false);
        t.key(LCTRL, false);
        // Alt+h has no rule of its own: h's rule, Alt passing through.
        t.key(LALT, true);
        assert_eq!(t.tap(H), [d(LEFT), u(LEFT)]);
    }

    #[test]
    fn chords_and_sequences() {
        let mut t = T::new(&[
            rule("f1", "ctrl+shift+esc"),
            RuleSpec { from: "f2".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["ctrl+a", "ctrl+c"]).unwrap()) },
        ]);
        assert_eq!(t.key(59, true), [d(LCTRL), d(LSHIFT), d(ESC)]);
        assert_eq!(t.key(59, true), [d(ESC)]);
        assert_eq!(t.key(59, false), [u(ESC), u(LCTRL), u(LSHIFT)]);
        assert_eq!(t.key(60, true), [d(LCTRL), d(A), u(A), u(LCTRL), d(LCTRL), d(C), u(C), u(LCTRL)]);
        assert_eq!(t.key(60, true), [], "a sequence doesn't repeat");
        assert_eq!(t.key(60, false), []);
    }

    #[test]
    fn two_sources_of_one_code_send_one_down_and_one_up() {
        let mut t = T::new(&[rule("capslock", "lctrl")]);
        assert_eq!(t.key(LCTRL, true), [d(LCTRL)]);
        assert_eq!(t.key(CAPS, true), []);
        assert_eq!(t.key(LCTRL, false), []);
        assert_eq!(t.key(CAPS, false), [u(LCTRL)]);
    }

    #[test]
    fn tap_hold_taps_when_released_in_time() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl")]);
        assert_eq!(t.key(CAPS, true), []);
        assert_eq!(t.m.deadline(), Some(t.t0 + DEFAULT_TAP));
        assert_eq!(t.wait(100), []);
        assert_eq!(t.key(CAPS, true), [], "repeats while undecided are ignored");
        assert_eq!(t.key(CAPS, false), [d(ESC), u(ESC)]);
        assert_eq!(t.m.deadline(), None, "the timer is cancelled");
        assert_eq!(t.wait(500), []);
    }

    #[test]
    fn tap_hold_holds_after_the_timeout() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl")]);
        t.key(CAPS, true);
        assert_eq!(t.wait(199), []);
        assert_eq!(t.wait(1), [d(LCTRL)]);
        assert_eq!(t.m.deadline(), None, "fires once");
        assert_eq!(t.key(CAPS, true), [d(LCTRL)], "a held hold repeats");
        assert_eq!(t.tap(C), [d(C), u(C)]);
        assert_eq!(t.key(CAPS, false), [u(LCTRL)]);
    }

    #[test]
    fn another_key_makes_it_a_hold_at_once() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl"), rule("ctrl+h", "backspace")]);
        t.key(CAPS, true);
        // Caps as Ctrl + C is Ctrl+C, without waiting.
        assert_eq!(t.key(C, true), [d(LCTRL), d(C)]);
        assert_eq!(t.key(C, false), [u(C)]);
        // and combos see the mapped Ctrl.
        assert_eq!(t.tap(H), [u(LCTRL), d(BACKSPACE), u(BACKSPACE), d(LCTRL)]);
        assert_eq!(t.key(CAPS, false), [u(LCTRL)]);
        assert!(t.m.is_idle());
    }

    #[test]
    fn a_click_makes_it_a_hold() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl")]);
        t.key(CAPS, true);
        let mut out = Vec::new();
        t.m.interrupt(&mut |e| out.push(e));
        assert_eq!(out, [d(LCTRL)]);
        assert_eq!(t.key(CAPS, false), [u(LCTRL)]);
    }

    #[test]
    fn a_key_already_down_released_while_undecided_is_just_released() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl")]);
        t.key(A, true);
        t.key(CAPS, true);
        assert_eq!(t.key(A, false), [u(A)]);
        assert_eq!(t.key(CAPS, false), [d(ESC), u(ESC)]);
    }

    #[test]
    fn layers() {
        let mut t = T::new(&[
            rule("ralt", "layer:nav"),
            in_layer("nav", "h", "left"),
            in_layer("nav", "j", "down"),
        ]);
        assert_eq!(t.key(RALT, true), []);
        assert_eq!(t.tap(H), [d(LEFT), u(LEFT)]);
        assert_eq!(t.tap(A), [d(A), u(A)], "keys the layer doesn't map fall through");
        assert_eq!(t.key(J, true), [d(DOWN)]);
        // The layer ends while j is still held: Down is released at once...
        assert_eq!(t.key(RALT, false), [u(DOWN)]);
        // ...and j does nothing more until it comes up.
        assert_eq!(t.key(J, true), []);
        assert_eq!(t.key(J, false), []);
        assert_eq!(t.tap(J), [d(J), u(J)]);
    }

    #[test]
    fn a_key_held_before_the_layer_stays_itself() {
        let mut t = T::new(&[rule("ralt", "layer:nav"), in_layer("nav", "h", "left")]);
        assert_eq!(t.key(H, true), [d(H)]);
        t.key(RALT, true);
        assert_eq!(t.key(H, false), [u(H)]);
        assert_eq!(t.key(RALT, false), []);
    }

    #[test]
    fn tap_hold_layer_key() {
        let mut t = T::new(&[
            RuleSpec {
                from: "space".parse().unwrap(),
                layer: None,
                what: What::TapHold { tap: "space".parse().unwrap(), hold: "layer:nav".parse().unwrap(), timeout: Some(Duration::from_millis(150)) },
            },
            in_layer("nav", "h", "left"),
        ]);
        assert_eq!(t.tap(57), [d(57), u(57)]);
        t.key(57, true);
        assert_eq!(t.key(H, true), [d(LEFT)], "the layer is on as soon as another key is pressed");
        assert_eq!(t.key(57, false), [u(LEFT)]);
        assert_eq!(t.key(H, false), []);
    }

    #[test]
    fn nested_layers_end_with_their_parent() {
        let mut t = T::new(&[
            rule("ralt", "layer:a"),
            in_layer("a", "rctrl", "layer:b"),
            in_layer("b", "h", "left"),
        ]);
        t.key(RALT, true);
        t.key(RCTRL, true);
        assert_eq!(t.key(H, true), [d(LEFT)]);
        assert_eq!(t.key(RALT, false), [u(LEFT)]);
        assert_eq!(t.key(RCTRL, false), []);
        assert_eq!(t.key(H, false), []);
        assert!(t.m.is_idle());
    }

    #[test]
    fn actions_fire_once_on_press() {
        let mut t = T::new(&[rule("f13", "switch next"), rule("f14", "lock-all")]);
        assert_eq!(t.key(183, true), [Emit::Action(Arc::new(Action::Switch("next".into())))]);
        assert_eq!(t.key(183, true), [], "no repeats");
        assert_eq!(t.key(183, false), []);
        assert_eq!(t.tap(184), [Emit::Action(Arc::new(Action::LockAll))]);
    }

    #[test]
    fn back_and_forward_follow_the_target() {
        let specs = [rule("button4", "back"), rule("button5", "forward")];
        let chord = |target| {
            let m = Keymap::compile(&specs, target, DEFAULT_TAP).unwrap().map;
            let mut t = T { m: Mapper::new(m), t0: Instant::now(), now: Instant::now(), out: Vec::new() };
            t.key(0x113, true)
        };
        assert_eq!(chord(PC), [d(LALT), d(LEFT)]);
        assert_eq!(chord(Target { os: os::MACOS, swap: false }), [d(LMETA), d(26)]);
        // With the swap on, Ctrl here becomes Cmd there.
        assert_eq!(chord(Target { os: os::MACOS, swap: true }), [d(LCTRL), d(26)]);
    }

    #[test]
    fn buttons_map_and_stay_paired() {
        let mut t = T::new(&[rule("button3", "button1"), rule("button8", "ctrl+c")]);
        assert_eq!(t.key(0x112, true), [d(0x110)]);
        assert_eq!(t.key(0x112, false), [u(0x110)]);
        assert_eq!(t.tap(0x117), [d(LCTRL), d(C), u(C), u(LCTRL)]);
    }

    #[test]
    fn release_all_releases_what_was_emitted() {
        let mut t = T::new(&[rule("capslock", "lctrl"), rule("ralt", "layer:nav"), in_layer("nav", "h", "left")]);
        t.key(CAPS, true);
        t.key(RALT, true);
        t.key(H, true);
        let mut out = Vec::new();
        t.m.release_all(&mut |e| out.push(e));
        out.sort_by_key(|e| match e {
            Emit::Key { code, .. } => *code,
            Emit::Action(_) => 0,
        });
        assert_eq!(out, [u(LCTRL), u(LEFT)], "not capslock, ralt and h");
        assert!(t.m.is_idle());
        // Physical releases afterwards are swallowed.
        assert_eq!(t.key(H, false), []);
        assert_eq!(t.key(CAPS, false), []);
        assert_eq!(t.key(RALT, false), []);
        assert_eq!(t.tap(H), [d(H), u(H)]);
    }

    #[test]
    fn a_new_keymap_keeps_held_keys_until_they_come_up() {
        let mut t = T::new(&[rule("capslock", "lctrl"), rule("ralt", "layer:nav"), in_layer("nav", "h", "left")]);
        t.key(CAPS, true);
        t.key(RALT, true);
        t.key(H, true);
        let mut out = Vec::new();
        t.m.set_keymap(map(&[rule("capslock", "esc")]), &mut |e| out.push(e));
        assert_eq!(out, [u(LEFT)], "the layer ended");
        assert_eq!(t.key(H, false), []);
        assert_eq!(t.key(CAPS, false), [u(LCTRL)], "released as what it was pressed as");
        assert_eq!(t.tap(CAPS), [d(ESC), u(ESC)], "new presses use the new rules");
        assert_eq!(t.key(RALT, false), []);
        assert!(t.m.is_idle());
        // The same keymap again changes nothing, even mid-press.
        t.key(CAPS, true);
        let same = t.m.keymap().clone();
        t.m.set_keymap(same, &mut |_| panic!("nothing to emit"));
        assert_eq!(t.key(CAPS, false), [u(ESC)]);
    }

    #[test]
    fn a_reload_while_undecided_holds() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl")]);
        t.key(CAPS, true);
        let mut out = Vec::new();
        t.m.set_keymap(Keymap::default(), &mut |e| out.push(e));
        assert_eq!(out, [d(LCTRL)]);
        assert_eq!(t.m.deadline(), None);
        assert_eq!(t.key(CAPS, false), [u(LCTRL)]);
    }

    /// A reload while a tap/hold key's hold is an action: the action run is
    /// the one the old keymap had, whatever the new one has at that index.
    #[test]
    fn a_reload_runs_the_old_keymaps_action() {
        for hold in ["switch here", "lock-all", "run: echo old"] {
            let mut t = T::new(&[dual("capslock", "esc", hold)]);
            t.key(CAPS, true);
            let mut out = Vec::new();
            let new = map(&[rule("f1", "run: echo new"), rule("f2", "lock-all"), rule("f3", "switch next")]);
            t.m.set_keymap(new, &mut |e| out.push(e));
            // Run after the swap, as the daemon does.
            let ran: Vec<Action> = out.iter().filter_map(|e| match e {
                Emit::Action(a) => Some(Action::clone(a)),
                Emit::Key { .. } => None,
            }).collect();
            let Output::Action(want) = hold.parse().unwrap() else { unreachable!() };
            assert_eq!(ran, [want], "{hold}");
            assert_eq!(t.key(CAPS, false), []);
        }
    }

    /// A tap/hold key released after its time, before the timer ran (a
    /// busy loop): it was held, not tapped.
    #[test]
    fn a_late_release_is_a_hold() {
        let mut t = T::new(&[dual("capslock", "esc", "lctrl")]);
        t.key(CAPS, true);
        t.now += Duration::from_millis(500);
        assert_eq!(t.key(CAPS, false), [d(LCTRL), u(LCTRL)]);
        assert!(t.m.is_idle());
        // Released exactly at the deadline: held too (as the timer has it).
        t.key(CAPS, true);
        t.now += DEFAULT_TAP;
        assert_eq!(t.key(CAPS, false), [d(LCTRL), u(LCTRL)]);
        // Any other event late resolves it first as well.
        t.key(CAPS, true);
        t.now += Duration::from_millis(300);
        assert_eq!(t.key(CAPS, true), [d(LCTRL), d(LCTRL)], "held, then the repeat");
        assert_eq!(t.key(CAPS, false), [u(LCTRL)]);
    }

    /// Typing a key that is already held (physically or by a mapping):
    /// it is let go of, typed, and pressed again for its holder.
    #[test]
    fn taps_of_a_held_key_are_typed() {
        let seq = |from: &str, steps: &[&str]| RuleSpec { from: from.parse().unwrap(), layer: None, what: What::To(Output::sequence(steps).unwrap()) };
        let mut t = T::new(&[seq("f1", &["a", "a"]), dual("capslock", "a", "lctrl"), seq("f2", &["lctrl", "ctrl+c"]), rule("ctrl+h", "backspace")]);
        assert_eq!(t.key(A, true), [d(A)]);
        let sequence = t.tap(59);
        assert_eq!(sequence.iter().filter(|event| **event == d(A)).count(), 2);
        assert_eq!(sequence, [u(A), d(A), u(A), d(A)]);
        let tap = t.tap(CAPS);
        assert_eq!(tap.iter().filter(|event| **event == d(A)).count(), 1);
        assert_eq!(tap, [u(A), d(A)]);
        assert_eq!(t.key(A, false), [u(A)]);
        assert!(t.m.is_idle());
        // A held modifier typed as a step comes back afterwards, and a
        // chord needing it still has it.
        assert_eq!(t.key(LCTRL, true), [d(LCTRL)]);
        assert_eq!(t.tap(60), [u(LCTRL), d(LCTRL), d(C), u(C)]);
        assert_eq!(t.key(LCTRL, false), [u(LCTRL)]);
        // A chord needs its modifier even while a combo hides it; the
        // Ctrl step is a tap of its own, and Ctrl stays hidden after it.
        t.key(LCTRL, true);
        assert_eq!(t.key(H, true), [u(LCTRL), d(BACKSPACE)]);
        assert_eq!(t.tap(60), [d(LCTRL), u(LCTRL), d(LCTRL), d(C), u(C), u(LCTRL)]);
        assert_eq!(t.key(H, false), [u(BACKSPACE), d(LCTRL)]);
        assert_eq!(t.key(LCTRL, false), [u(LCTRL)]);
        assert!(t.m.is_idle());
    }

    /// The last tap of a key held but hidden by a combo leaves it up, as
    /// the combo wants: Shift+Space (Backspace) held, then Shift typed
    /// twice must not leave Shift down under the Backspace repeat.
    #[test]
    fn taps_of_a_hidden_key_leave_it_hidden() {
        const SPACE: u32 = 57;
        const F12: u32 = 88;
        let seq = |from: &str, steps: &[&str]| RuleSpec { from: from.parse().unwrap(), layer: None, what: What::To(Output::sequence(steps).unwrap()) };
        let mut t = T::new(&[rule("shift+space", "backspace"), seq("f12", &["lshift", "lshift"])]);
        assert_eq!(t.key(LSHIFT, true), [d(LSHIFT)]);
        assert_eq!(t.key(SPACE, true), [u(LSHIFT), d(BACKSPACE)]);
        let taps = t.tap(F12);
        assert_eq!(taps.iter().filter(|event| **event == d(LSHIFT)).count(), 2, "one fresh press per tap");
        assert_eq!(taps, [d(LSHIFT), u(LSHIFT), d(LSHIFT), u(LSHIFT)]);
        assert_eq!(t.key(SPACE, true), [d(BACKSPACE)], "a repeat of Backspace, not Shift+Backspace");
        assert!(!t.m.wire.is_held(LSHIFT));
        assert_eq!(t.key(SPACE, false), [u(BACKSPACE), d(LSHIFT)]);
        assert_eq!(t.key(LSHIFT, false), [u(LSHIFT)]);
        assert!(t.m.is_idle());
    }

    #[test]
    fn lifted_modifiers_come_back_only_after_release() {
        let mut t = T::new(&[]);
        t.key(LSHIFT, true);
        let mut out = Vec::new();
        t.m.lift(keynames::SHIFT | keynames::LSHIFT, &mut |e| out.push(e));
        assert_eq!(out, [u(LSHIFT)]);
        assert_eq!(t.m.mods(), keynames::LSHIFT, "still held for modifier-scroll");
        assert_eq!(t.key(LSHIFT, true), [], "no repeat while lifted");
        assert_eq!(t.key(LSHIFT, false), []);
        assert_eq!(t.key(LSHIFT, true), [d(LSHIFT)], "pressed again, it goes through");
        assert_eq!(t.key(LSHIFT, false), [u(LSHIFT)]);
    }

    #[test]
    fn bad_rules_are_errors() {
        let compile = |specs: &[RuleSpec]| Keymap::compile(specs, PC, DEFAULT_TAP).map(|_| ()).unwrap_err().0;
        assert!(compile(&[RuleSpec { from: "ctrl+a".parse().unwrap(), ..dual("a", "b", "c") }]).contains("can't need modifiers"));
        assert!(compile(&[dual("a", "layer:x", "c")]).contains("`tap` can't be a layer"));
        let many: Vec<_> = (0..9).map(|i| in_layer(&format!("l{i}"), "a", "b")).collect();
        assert!(compile(&many).contains("at most 8 layers"));
        let slow = RuleSpec {
            what: What::TapHold { tap: Output::None, hold: Output::None, timeout: Some(Duration::from_secs(5)) },
            ..dual("a", "b", "c")
        };
        assert!(compile(&[slow]).contains("from 50 to 2000 ms"));
    }

    #[test]
    fn odd_rules_are_warnings() {
        let c = Keymap::compile(&[rule("a", "b"), rule("a", "c"), in_layer("nav", "h", "left"), rule("f1", "layer:empty")], PC, DEFAULT_TAP)
            .unwrap();
        let w = c.warnings.join("\n");
        assert!(w.contains("a: more than one rule"), "{w}");
        assert!(w.contains("layer \"nav\" is never switched on"), "{w}");
        assert!(w.contains("layer \"empty\" has no rules"), "{w}");
        let mut t = T { m: Mapper::new(c.map), t0: Instant::now(), now: Instant::now(), out: Vec::new() };
        assert_eq!(t.tap(A), [d(48), u(48)], "the first rule counts");
    }

    #[test]
    fn outputs_parse_and_print() {
        for (s, printed) in [
            ("esc", "esc"),
            ("Ctrl+Shift+Esc", "ctrl+shift+esc"),
            ("none", "none"),
            ("Disabled", "none"),
            ("layer:Nav", "layer:nav"),
            ("back", "back"),
            ("forward", "forward"),
            ("switch next", "switch next"),
            ("switch  Here", "switch here"),
            ("switch My Laptop", "switch My Laptop"),
            ("lock-pointer", "lock-pointer toggle"),
            ("lock-pointer on", "lock-pointer on"),
            ("lock-all", "lock-all"),
            ("run: notify-send 'hi there'", "run: notify-send 'hi there'"),
            ("scroll down", "scroll down"),
            ("scroll left 3", "scroll left 3"),
            ("volume up", "volume up"),
        ] {
            let o: Output = s.parse().unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(o.to_string(), printed, "{s}");
            assert_eq!(printed.parse::<Output>().unwrap(), o, "{printed}");
        }
        assert_eq!("scroll up 2".parse::<Output>().unwrap(), Output::Action(Action::Scroll { dx: 0, dy: -240 }));
        for (bad, says) in [
            ("", "nothing given"),
            ("run:", "needs a command"),
            ("switch", "needs `next`"),
            ("lock-pointer maybe", "say on, off or toggle"),
            ("scroll sideways", "say up, down"),
            ("scroll up 0", "from 1 to 100"),
            ("scroll up 1 2", "at most a number"),
            ("layer:", "layer name"),
            ("layer:a b", "layer name"),
            ("ctrl+capslok", "not a key name"),
        ] {
            let e = bad.parse::<Output>().unwrap_err().0;
            assert!(e.contains(says), "{bad:?}: {e}");
        }
        assert!(Output::sequence::<&str>(&[]).is_err());
        assert!(Output::sequence(&["a"; 17]).unwrap_err().0.contains("at most 16"));
        assert_eq!(Output::sequence(&["a", "ctrl+v"]).unwrap().to_string(), "[a, ctrl+v]");
    }

    // ------------------------------------------------- the invariant

    /// xorshift: deterministic, no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Checks the emitted stream as the target would see it: every up is
    /// for a key down, a down for a key already down is only a repeat.
    #[derive(Default)]
    struct Wire {
        down: std::collections::BTreeSet<u32>,
    }
    impl Wire {
        fn apply(&mut self, e: Emit, physical_repeat: bool) {
            if let Emit::Key { code, down } = e {
                if down {
                    let new = self.down.insert(code);
                    assert!(new || physical_repeat, "{code} pressed twice, not by a repeat");
                } else {
                    assert!(self.down.remove(&code), "{code} released but not down");
                }
            }
        }
    }

    fn configs() -> [Vec<RuleSpec>; 3] {
        [
            vec![
                dual("capslock", "esc", "lctrl"),
                rule("lalt", "lmeta"),
                rule("lmeta", "lalt"),
                rule("ctrl+h", "backspace"),
                rule("meta+q", "none"),
                rule("ralt", "layer:nav"),
                in_layer("nav", "h", "left"),
                in_layer("nav", "j", "down"),
                in_layer("nav", "rctrl", "layer:deep"),
                in_layer("deep", "a", "ctrl+shift+esc"),
                rule("f1", "ctrl+alt+c"),
                RuleSpec { from: "f2".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["ctrl+a", "c"]).unwrap()) },
                rule("f13", "switch next"),
                rule("button3", "button1"),
                RuleSpec {
                    from: "space".parse().unwrap(),
                    layer: None,
                    what: What::TapHold { tap: "space".parse().unwrap(), hold: "layer:nav".parse().unwrap(), timeout: None },
                },
            ],
            vec![
                rule("capslock", "lctrl"),
                rule("h", "lshift"),
                dual("j", "j", "ralt"),
                rule("ctrl+a", "rctrl+q"),
                // Typing keys that may be held already: A, Ctrl, Shift, J.
                RuleSpec { from: "f1".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["a", "a", "lctrl", "ctrl+a", "lshift+j"]).unwrap()) },
                RuleSpec { from: "ctrl+f2".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["ctrl+c", "a"]).unwrap()) },
                dual("q", "a", "lshift"),
                dual("c", "lctrl", "c"),
                rule("f13", "lock-all"),
                // Typing a modifier a combo hides while it is held.
                rule("shift+space", "backspace"),
                RuleSpec { from: "f12".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["lshift", "lshift"]).unwrap()) },
            ],
            vec![],
        ]
    }

    /// Each config with rules for an app on top (per-app rules): the same
    /// keys mean something else while that app has the focus, which comes
    /// and goes mid-stream (`set_keymap`, as the daemon does when the
    /// computer in use says another app has the focus).
    fn in_app(base: &[RuleSpec]) -> Vec<RuleSpec> {
        let mut over = vec![
            rule("capslock", "lctrl"),
            dual("ralt", "esc", "layer:nav"),
            in_layer("nav", "h", "lshift"),
            dual("h", "h", "lalt"),
            rule("lshift", "layer:deep"),
            in_layer("deep", "j", "ctrl+c"),
            RuleSpec { from: "f1".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["lctrl", "a"]).unwrap()) },
        ];
        let replaced: Vec<_> = over.iter().map(|r| (r.from, r.layer.clone())).collect();
        over.extend(base.iter().filter(|r| !replaced.contains(&(r.from, r.layer.clone()))).cloned());
        over
    }

    /// Random presses, releases, repeats, timeouts (in time and late),
    /// clicks, modifier-lifts, keymap reloads, app changes and visit ends,
    /// about a million in all: every emitted down gets exactly one up, and
    /// once every physical key is up nothing is left down.
    #[test]
    fn no_stuck_keys_under_random_streams() {
        let keys = [CAPS, LALT, LMETA, LCTRL, RCTRL, LSHIFT, H, J, Q, A, C, RALT, 57, 59, 60, 88, 183, 0x112, 0x110];
        let maps: Vec<Keymap> = configs().iter().map(|c| map(c)).collect();
        let app_maps: Vec<Keymap> = configs().iter().map(|c| map(&in_app(c))).collect();
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for stream in 0..10_000 {
            let family = stream % maps.len();
            let mut m = Mapper::new(maps[family].clone());
            let mut wire = Wire::default();
            let mut phys = std::collections::BTreeSet::new();
            let mut now = Instant::now();
            for _ in 0..rng.below(200) + 1 {
                let mut emitted = Vec::new();
                let mut repeat = false;
                match rng.below(22) {
                    0..=11 => {
                        let k = keys[rng.below(keys.len())];
                        let down = rng.below(2) == 0;
                        repeat = down && phys.contains(&k);
                        if down {
                            phys.insert(k);
                        } else {
                            phys.remove(&k);
                        }
                        m.key(k, down, now, &mut |e| emitted.push(e));
                    }
                    // Time passes, and the timer may run late.
                    12 => now += Duration::from_millis(rng.below(400) as u64),
                    13 | 14 => {
                        now += Duration::from_millis(rng.below(400) as u64);
                        m.timer(now, &mut |e| emitted.push(e));
                    }
                    15 => m.interrupt(&mut |e| emitted.push(e)),
                    16 => m.lift(rng.next() as u8, &mut |e| emitted.push(e)),
                    17 | 18 => m.set_keymap(maps[rng.below(maps.len())].clone(), &mut |e| emitted.push(e)),
                    // The app with the focus changes (or is the same again).
                    19 | 20 => {
                        let to = if rng.below(3) != 0 { &app_maps[family] } else { &maps[family] };
                        m.set_keymap(to.clone(), &mut |e| emitted.push(e));
                    }
                    _ => m.release_all(&mut |e| emitted.push(e)),
                }
                for e in emitted {
                    wire.apply(e, repeat);
                }
                assert_eq!(wire.down.iter().copied().collect::<Vec<_>>(), m.wire.iter().collect::<Vec<_>>());
                // Down on the wire exactly when held and not hidden.
                for code in 0..CODE_LIMIT {
                    assert_eq!(m.wire.is_held(code), m.wanted(code), "stream {stream}: code {code}");
                }
            }
            // Everything comes up physically (and a pending key's time runs out).
            for k in std::mem::take(&mut phys) {
                let mut emitted = Vec::new();
                m.key(k, false, now, &mut |e| emitted.push(e));
                emitted.into_iter().for_each(|e| wire.apply(e, false));
            }
            assert!(wire.down.is_empty(), "stream {stream}: left down {:?}", wire.down);
            assert!(m.is_idle());
            assert_eq!(m.mods(), 0);
            // A visit ending mid-press releases what was emitted.
            m.key(CAPS, true, now, &mut |e| wire.apply(e, false));
            m.key(RALT, true, now, &mut |e| wire.apply(e, false));
            m.key(H, true, now, &mut |e| wire.apply(e, false));
            m.timer(now + Duration::from_secs(1), &mut |e| wire.apply(e, false));
            m.release_all(&mut |e| wire.apply(e, false));
            assert!(wire.down.is_empty(), "stream {stream}: release_all left {:?}", wire.down);
        }
    }
}
