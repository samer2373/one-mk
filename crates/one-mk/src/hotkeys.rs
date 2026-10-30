//! Keyboard shortcuts: the `[hotkeys]` table in `settings.toml`, `one-mk
//! hotkeys`, how computers are numbered, and what each shortcut asks the
//! daemon (the same requests as `one-mk switch` and `one-mk lock-pointer`).
//! The chords and the listening live in `one_mk_input::hotkeys`.

use crate::control::{self, Request};
use crate::settings::Settings;
use anyhow::Result;
use one_mk_ctl::client::{SwitchTo, Toggle};
use one_mk_ctl::status::Status;
use one_mk_input::hotkeys::{Action, Bindings, Chord, EMERGENCY, JumpChords};
use one_mk_net::{Identity, TrustStore};
use one_mk_proto::sanitize_name;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;

/// `[hotkeys]` in `settings.toml`. A chord left empty (or `none`) is off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    /// Off: no shortcuts at all (Ctrl+Alt+Shift+Esc still works).
    pub enabled: bool,
    /// Jump to computer N; `{n}` stands for 1 to 9.
    pub jump: String,
    /// The next computer, round and round (`one-mk switch next`).
    pub cycle: String,
    /// Lock the pointer to this computer, or unlock it.
    pub lock_pointer: String,
    /// Back to this computer (`one-mk switch here`).
    pub home: String,
    /// Start or stop broadcast (`one-mk broadcast toggle`).
    pub broadcast: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            jump: default_jump(cfg!(windows)).into(),
            // Not Ctrl+Alt+Right (GNOME's workspaces) nor Ctrl+Alt+N
            // (AltGr+N types ń on Polish keyboards).
            cycle: "ctrl+alt+pagedown".into(),
            lock_pointer: "ctrl+alt+p".into(),
            home: "ctrl+alt+h".into(),
            broadcast: default_broadcast(cfg!(windows)).into(),
        }
    }
}

/// The jump shortcuts' default: Ctrl+Alt+1..9, but Ctrl+Alt+Shift+1..9 on
/// Windows, whose `RegisterHotKey` can't tell AltGr (which Windows reports
/// as Ctrl+Alt) from Ctrl+Alt: AltGr+digit types `{ [ ] }` on German
/// keyboards and most of the digit row's symbols on French AZERTY ones.
/// (The matcher used while the pointer is away counts only the left Alt,
/// so elsewhere AltGr is no problem.)
fn default_jump(windows: bool) -> &'static str {
    if windows { "ctrl+alt+shift+{n}" } else { "ctrl+alt+{n}" }
}

/// The broadcast shortcut's default: Ctrl+Alt+B, but Ctrl+Alt+Shift+B on
/// Windows, where AltGr counts as Ctrl+Alt and AltGr+B types `{` on
/// Hungarian keyboards (see [`default_jump`]).
fn default_broadcast(windows: bool) -> &'static str {
    if windows { "ctrl+alt+shift+b" } else { "ctrl+alt+b" }
}

/// `""` and `none` mean no chord.
fn chord_or_none<T: FromStr<Err = String>>(s: &str) -> Result<Option<T>, String> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") { Ok(None) } else { s.parse().map(Some) }
}

impl HotkeySettings {
    /// The shortcuts in force with `computers` computers (counting this
    /// one): no jump to a computer that doesn't exist, so Ctrl+Alt+3..9
    /// stay free for programs with two computers. Every chord is checked
    /// all the same.
    pub fn bindings_for(&self, computers: usize) -> Result<Bindings, String> {
        let max = computers.min(9);
        let all = self.bindings()?;
        Bindings::new(all.iter().filter(|(a, _)| !matches!(a, Action::Jump(n) if usize::from(*n) > max)).collect())
    }

    /// Every shortcut these settings ask for (all nine jumps), or why they
    /// can't be used.
    pub fn bindings(&self) -> Result<Bindings, String> {
        if !self.enabled {
            return Ok(Bindings::default());
        }
        let mut list = Vec::new();
        if let Some(j) = chord_or_none::<JumpChords>(&self.jump)? {
            list.extend((1..=9).map(|n| (Action::Jump(n), j.chord(n))));
        }
        for (s, a) in [
            (&self.cycle, Action::Cycle),
            (&self.lock_pointer, Action::LockPointer),
            (&self.home, Action::Home),
            (&self.broadcast, Action::Broadcast),
        ] {
            if let Some(c) = chord_or_none::<Chord>(s)? {
                list.push((a, c));
            }
        }
        Bindings::new(list)
    }
}

// --------------------------------------------------------------- numbering

/// A computer, as the jump shortcuts count them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numbered {
    pub name: String,
    /// The full fingerprint (unique, unlike names).
    pub id: String,
    /// This computer.
    pub this: bool,
}

/// This computer and every paired one, sorted by name (then id): with every
/// computer paired to every other, Ctrl+Alt+2 is the same computer
/// whichever keyboard presses it.
pub fn numbered(this: (&str, &str), paired: impl IntoIterator<Item = (String, String)>) -> Vec<Numbered> {
    let mut all: Vec<Numbered> = paired.into_iter().map(|(name, id)| Numbered { name, id, this: false }).collect();
    all.push(Numbered { name: this.0.to_owned(), id: this.1.to_owned(), this: true });
    all.sort_by(|a, b| (a.name.to_lowercase(), &a.id).cmp(&(b.name.to_lowercase(), &b.id)));
    all
}

/// The numbering from what is on disk: the identity and `devices.toml`.
pub fn numbered_from(identity: &Identity, trust: &TrustStore) -> Vec<Numbered> {
    let paired = trust.devices().iter().map(|d| (d.name.clone(), d.fingerprint.clone()));
    numbered((&identity.name, &identity.fingerprint.to_string()), paired)
}

/// What a shortcut asks the daemon: exactly what `one-mk switch` and
/// `one-mk lock-pointer` ask.
pub fn request(action: Action, order: &[Numbered]) -> Result<Request, String> {
    Ok(match action {
        Action::Jump(n) => match usize::from(n).checked_sub(1).and_then(|i| order.get(i)) {
            Some(c) if c.this => Request::Switch(SwitchTo::Here),
            Some(c) => Request::Switch(SwitchTo::Device(c.id.clone())),
            None => return Err(format!("there is no computer {n} (counting this one, there are {})", order.len())),
        },
        Action::Cycle => Request::Switch(SwitchTo::Next),
        Action::LockPointer => Request::LockPointer(Toggle::Toggle),
        Action::Home => Request::Switch(SwitchTo::Here),
        Action::Broadcast => Request::Broadcast(Toggle::Toggle),
    })
}

// --------------------------------------------------------- one-mk hotkeys

/// `one-mk hotkeys` options.
#[derive(clap::Args, Debug, Default)]
pub struct HotkeyArgs {
    /// Jump to computer N: modifiers and {n}, like 'ctrl+alt+{n}' (none: off).
    #[arg(long, value_name = "CHORD")]
    pub jump: Option<String>,
    /// The next computer, like 'ctrl+alt+pagedown' (none: off).
    #[arg(long, value_name = "CHORD")]
    pub cycle: Option<String>,
    /// Lock the pointer to this computer or unlock it, like 'ctrl+alt+p' (none: off).
    #[arg(long, value_name = "CHORD")]
    pub lock_pointer: Option<String>,
    /// Back to this computer, like 'ctrl+alt+h' (none: off).
    #[arg(long, value_name = "CHORD")]
    pub home_key: Option<String>,
    /// Start or stop broadcast to every connected computer, like 'ctrl+alt+b' (none: off).
    #[arg(long, value_name = "CHORD")]
    pub broadcast: Option<String>,
    /// Turn every shortcut off.
    #[arg(long, conflicts_with = "on")]
    pub off: bool,
    /// Turn the shortcuts on again.
    #[arg(long)]
    pub on: bool,
}

impl HotkeyArgs {
    fn changes_anything(&self) -> bool {
        self.off || self.on || self.sets_a_chord()
    }

    fn sets_a_chord(&self) -> bool {
        self.jump.is_some() || self.cycle.is_some() || self.lock_pointer.is_some() || self.home_key.is_some() || self.broadcast.is_some()
    }

    /// Applies these options to `s`, writing chords in their canonical form.
    /// Setting a chord turns the shortcuts on.
    pub fn apply(&self, s: &mut HotkeySettings) -> Result<(), String> {
        fn canonical<T: FromStr<Err = String> + ToString>(v: &str) -> Result<String, String> {
            if v.trim().is_empty() {
                return Err("a shortcut cannot be empty; use `none` to turn it off".into());
            }
            Ok(chord_or_none::<T>(v)?.map(|c| c.to_string()).unwrap_or_default())
        }
        if let Some(v) = &self.jump {
            s.jump = canonical::<JumpChords>(v).map_err(|e| format!("--jump: {e}"))?;
        }
        for (v, field, flag) in [
            (&self.cycle, &mut s.cycle, "--cycle"),
            (&self.lock_pointer, &mut s.lock_pointer, "--lock-pointer"),
            (&self.home_key, &mut s.home, "--home-key"),
            (&self.broadcast, &mut s.broadcast, "--broadcast"),
        ] {
            if let Some(v) = v {
                *field = canonical::<Chord>(v).map_err(|e| format!("{flag}: {e}"))?;
            }
        }
        if self.on || self.sets_a_chord() {
            s.enabled = true;
        }
        if self.off {
            s.enabled = false;
        }
        s.bindings().map(|_| ())
    }
}

/// The shortcuts and the numbering, in words.
pub fn describe(s: &HotkeySettings, order: &[Numbered]) -> String {
    if !s.enabled {
        return "Keyboard shortcuts are off (`one-mk hotkeys --on` turns them on).\n".into();
    }
    let bindings = match s.bindings() {
        Ok(b) => b,
        Err(e) => return format!("The keyboard shortcuts in settings.toml can't be used: {e}.\n"),
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Ok(Some(j)) = chord_or_none::<JumpChords>(&s.jump) {
        rows.push((j.label(), "move the mouse and keyboard to computer 1 to 9:".into()));
        for (i, c) in order.iter().take(9).enumerate() {
            let this = if c.this { " (this computer)" } else { "" };
            rows.push((String::new(), format!("  {}  {}{this}", i + 1, sanitize_name(&c.name))));
        }
    }
    for (a, c) in bindings.iter().filter(|(a, _)| !matches!(a, Action::Jump(_))) {
        let what = match a {
            Action::Cycle => "the next computer",
            Action::LockPointer => "lock the pointer to this computer (or unlock it)",
            Action::Broadcast => "type on every connected computer at once (or stop)",
            _ => "bring the mouse and keyboard back here",
        };
        rows.push((c.label(), what.into()));
    }
    rows.push((EMERGENCY.label(), "always brings the pointer back (can't be changed)".into()));
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut out = "Keyboard shortcuts:\n".to_owned();
    for (k, what) in rows {
        out += &format!("  {k:<width$}  {what}\n");
    }
    out
}

/// `one-mk hotkeys [...]`: shows the shortcuts, or changes them.
pub async fn hotkeys(dir: &Path, args: HotkeyArgs) -> Result<()> {
    let settings = if args.changes_anything() {
        Settings::update(dir, |s| {
            args.apply(&mut s.hotkeys).map_err(anyhow::Error::msg)?;
            Ok(s.hotkeys.clone())
        })?
    } else {
        Settings::load(dir)?.hotkeys
    };
    let order = numbered_from(&Identity::load_or_create(dir)?, &TrustStore::load(dir)?);
    print!("{}", describe(&settings, &order));
    if args.changes_anything() {
        crate::settings::apply(dir).await?;
    }
    // How they work here, as the running daemon knows.
    if settings.enabled
        && let Some(reply) = control::request(dir, Request::Status).await?
        && let Some(status) = Status::decode(&reply)
        && let Some(line) = status.hotkeys_line()
    {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<Numbered> {
        numbered(("mid", "bbbb"), [("Zed".into(), "cccc".into()), ("alpha".into(), "dddd".into()), ("alpha".into(), "aaaa".into())])
    }

    #[test]
    fn computers_are_numbered_by_name_then_id_with_this_one() {
        let o = order();
        let names: Vec<(&str, &str, bool)> = o.iter().map(|c| (c.name.as_str(), c.id.as_str(), c.this)).collect();
        assert_eq!(names, [("alpha", "aaaa", false), ("alpha", "dddd", false), ("mid", "bbbb", true), ("Zed", "cccc", false)]);
        // The same set seen from another computer numbers the same way.
        let there = numbered(("Zed", "cccc"), [("mid".into(), "bbbb".into()), ("alpha".into(), "aaaa".into()), ("alpha".into(), "dddd".into())]);
        let ids = |o: &[Numbered]| o.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&there), ids(&o));
    }

    #[test]
    fn shortcuts_ask_what_the_commands_ask() {
        let o = order();
        assert_eq!(request(Action::Jump(1), &o), Ok(Request::Switch(SwitchTo::Device("aaaa".into()))));
        assert_eq!(request(Action::Jump(3), &o), Ok(Request::Switch(SwitchTo::Here)), "this computer");
        assert_eq!(request(Action::Jump(4), &o), Ok(Request::Switch(SwitchTo::Device("cccc".into()))));
        assert!(request(Action::Jump(5), &o).unwrap_err().contains("no computer 5"));
        assert!(request(Action::Jump(0), &o).is_err());
        assert_eq!(request(Action::Cycle, &o), Ok(Request::Switch(SwitchTo::Next)));
        assert_eq!(request(Action::LockPointer, &o), Ok(Request::LockPointer(Toggle::Toggle)));
        assert_eq!(request(Action::Home, &o), Ok(Request::Switch(SwitchTo::Here)));
        assert_eq!(request(Action::Broadcast, &o), Ok(Request::Broadcast(Toggle::Toggle)));
        assert_eq!(Request::parse("broadcast toggle"), request(Action::Broadcast, &o).ok());
        // The requests are the ones the command line sends.
        assert_eq!(Request::parse("switch next"), request(Action::Cycle, &o).ok());
        assert_eq!(Request::parse("lock-pointer toggle"), request(Action::LockPointer, &o).ok());
    }

    #[test]
    fn defaults_are_valid_and_distinct() {
        assert!(HotkeyArgs { jump: Some(String::new()), ..Default::default() }.apply(&mut HotkeySettings::default()).is_err());
        assert!(HotkeyArgs { cycle: Some(" ".into()), ..Default::default() }.apply(&mut HotkeySettings::default()).is_err());
        let b = HotkeySettings::default().bindings().unwrap();
        assert_eq!(b.iter().count(), 13);
        let jump: JumpChords = HotkeySettings::default().jump.parse().unwrap();
        assert_eq!(b.action(jump.chord(5)), Some(Action::Jump(5)));
        assert_eq!(b.action("ctrl+alt+h".parse().unwrap()), Some(Action::Home));
        let broadcast: Chord = HotkeySettings::default().broadcast.parse().unwrap();
        assert_eq!(b.action(broadcast), Some(Action::Broadcast));
        let off = HotkeySettings { enabled: false, ..HotkeySettings::default() };
        assert!(off.bindings().unwrap().is_empty());
        let some = HotkeySettings { jump: "none".into(), cycle: String::new(), ..HotkeySettings::default() };
        assert_eq!(some.bindings().unwrap().iter().count(), 3);
    }

    /// Windows can't tell AltGr from Ctrl+Alt, so its jump default adds
    /// Shift; elsewhere it is Ctrl+Alt+digit. Both parse, and differ.
    #[test]
    fn the_jump_default_avoids_altgr_on_windows() {
        use one_mk_input::hotkeys::ChordMods;
        let (win, other): (JumpChords, JumpChords) = (default_jump(true).parse().unwrap(), default_jump(false).parse().unwrap());
        assert!(win.0.contains(ChordMods::SHIFT) && win.0.contains(ChordMods::CTRL) && win.0.contains(ChordMods::ALT));
        assert_eq!(other.chord(7), "ctrl+alt+7".parse().unwrap());
        assert_eq!(win.chord(7), "ctrl+alt+shift+7".parse().unwrap());
        assert_eq!(win.to_string(), "ctrl+alt+shift+{n}", "printed as written");
        assert_eq!(HotkeySettings::default().jump, default_jump(cfg!(windows)));
    }

    /// Broadcast is Ctrl+Alt+B, plus Shift on Windows (AltGr+B types `{`
    /// on Hungarian keyboards there). Both are valid next to every other
    /// default on either system.
    #[test]
    fn the_broadcast_default_avoids_altgr_on_windows() {
        let (win, other): (Chord, Chord) = (default_broadcast(true).parse().unwrap(), default_broadcast(false).parse().unwrap());
        assert_eq!(other, "ctrl+alt+b".parse().unwrap());
        assert_eq!(win, "ctrl+alt+shift+b".parse().unwrap());
        assert_eq!(HotkeySettings::default().broadcast, default_broadcast(cfg!(windows)));
        for windows in [true, false] {
            let s = HotkeySettings { jump: default_jump(windows).into(), broadcast: default_broadcast(windows).into(), ..HotkeySettings::default() };
            assert_eq!(s.bindings_for(9).unwrap().iter().count(), 13, "windows: {windows}");
        }
        // Changed, turned off, and refused when it clashes.
        let mut s = HotkeySettings::default();
        HotkeyArgs { broadcast: Some("Super+Alt+B".into()), ..HotkeyArgs::default() }.apply(&mut s).unwrap();
        assert_eq!(s.broadcast, "alt+super+b");
        HotkeyArgs { broadcast: Some("none".into()), ..HotkeyArgs::default() }.apply(&mut s).unwrap();
        assert!(s.bindings().unwrap().iter().all(|(a, _)| a != Action::Broadcast));
        let mut t = s.clone();
        assert!(HotkeyArgs { broadcast: Some("ctrl+alt+h".into()), ..HotkeyArgs::default() }.apply(&mut t).unwrap_err().contains("two things"));
        assert!(HotkeyArgs { broadcast: Some("b".into()), ..HotkeyArgs::default() }.apply(&mut t).is_err(), "would take over typing");
        assert!(describe(&HotkeySettings::default(), &order()).contains("type on every connected computer at once"));
    }

    /// Only jumps to computers that exist are bound: with two computers
    /// Ctrl+Alt+3..9 stay free. A clash with a jump that isn't bound is
    /// still an error (it would be one once more computers are paired).
    #[test]
    fn jumps_only_to_computers_that_exist() {
        let s = HotkeySettings::default();
        let jumps = |b: &Bindings| b.iter().filter(|(a, _)| matches!(a, Action::Jump(_))).count();
        let jump: JumpChords = s.jump.parse().unwrap();
        let two = s.bindings_for(2).unwrap();
        assert_eq!(jumps(&two), 2);
        assert_eq!(two.action(jump.chord(2)), Some(Action::Jump(2)));
        assert_eq!(two.action(jump.chord(3)), None, "no third computer");
        assert_eq!(jumps(&s.bindings_for(1).unwrap()), 1);
        assert_eq!(jumps(&s.bindings_for(30).unwrap()), 9);
        assert_eq!(two.iter().count(), 6, "and cycle, lock, home, broadcast");
        let clash = HotkeySettings { home: jump.chord(7).to_string(), ..s };
        assert!(clash.bindings_for(2).is_err());
    }

    #[test]
    fn the_command_line_checks_and_normalizes() {
        let mut s = HotkeySettings { enabled: false, ..HotkeySettings::default() };
        let args = HotkeyArgs { jump: Some("Super+Shift+{N}".into()), cycle: Some("NONE".into()), ..HotkeyArgs::default() };
        args.apply(&mut s).unwrap();
        assert_eq!((s.enabled, s.jump.as_str(), s.cycle.as_str()), (true, "shift+super+{n}", ""));
        for bad in [
            HotkeyArgs { jump: Some("ctrl+alt+1".into()), ..HotkeyArgs::default() },
            HotkeyArgs { cycle: Some("ctrl+alt+".into()), ..HotkeyArgs::default() },
            HotkeyArgs { home_key: Some("q".into()), ..HotkeyArgs::default() },
            // Already a jump shortcut (shift+super+{n} now).
            HotkeyArgs { lock_pointer: Some("shift+super+3".into()), ..HotkeyArgs::default() },
            HotkeyArgs { home_key: Some("ctrl+alt+shift+esc".into()), ..HotkeyArgs::default() },
        ] {
            let before = s.clone();
            let mut t = s.clone();
            assert!(bad.apply(&mut t).is_err(), "{bad:?}");
            assert_eq!(s, before);
        }
        HotkeyArgs { off: true, ..HotkeyArgs::default() }.apply(&mut s).unwrap();
        assert!(!s.enabled && s.bindings().unwrap().is_empty());
        assert!(!HotkeyArgs::default().changes_anything());
    }

    #[test]
    fn described_with_the_numbering() {
        let text = describe(&HotkeySettings::default(), &order());
        assert!(text.contains("1..9") && text.contains("3  mid (this computer)") && text.contains("4  Zed"), "{text}");
        assert!(text.contains("Esc"), "{text}");
        let off = describe(&HotkeySettings { enabled: false, ..HotkeySettings::default() }, &order());
        assert!(off.contains("off"));
        let bad = describe(&HotkeySettings { home: "hyper+x".into(), ..HotkeySettings::default() }, &order());
        assert!(bad.contains("can't be used"), "{bad}");
    }

    #[test]
    fn settings_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("one-mk-hotkeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(&dir).unwrap().hotkeys, HotkeySettings::default(), "no file: the defaults");
        Settings::update(&dir, |s| {
            HotkeyArgs { home_key: Some("ctrl+alt+end".into()), ..HotkeyArgs::default() }.apply(&mut s.hotkeys).map_err(anyhow::Error::msg)
        })
        .unwrap();
        let text = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(text.contains("[hotkeys]") && text.contains("home = \"ctrl+alt+end\""), "{text}");
        assert_eq!(Settings::load(&dir).unwrap().hotkeys.home, "ctrl+alt+end");
        // A file from before shortcuts existed gets the defaults.
        std::fs::write(dir.join("settings.toml"), "lock_sync = true\n").unwrap();
        assert_eq!(Settings::load(&dir).unwrap().hotkeys, HotkeySettings::default());
        // One from before broadcast keeps its shortcuts and gets broadcast's default.
        std::fs::write(dir.join("settings.toml"), "[hotkeys]\nenabled = true\nhome = \"ctrl+alt+end\"\ncycle = \"\"\n").unwrap();
        let old = Settings::load(&dir).unwrap().hotkeys;
        assert_eq!((old.home.as_str(), old.cycle.as_str()), ("ctrl+alt+end", ""));
        assert_eq!(old.broadcast, HotkeySettings::default().broadcast);
        let _ = std::fs::remove_dir_all(dir);
    }
}
