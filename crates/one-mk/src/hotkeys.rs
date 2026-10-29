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
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            jump: "ctrl+alt+{n}".into(),
            // Not Ctrl+Alt+Right (GNOME's workspaces) nor Ctrl+Alt+N
            // (AltGr+N types ń on Polish keyboards).
            cycle: "ctrl+alt+pagedown".into(),
            lock_pointer: "ctrl+alt+p".into(),
            home: "ctrl+alt+h".into(),
        }
    }
}

/// `""` and `none` mean no chord.
fn chord_or_none<T: FromStr<Err = String>>(s: &str) -> Result<Option<T>, String> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") { Ok(None) } else { s.parse().map(Some) }
}

impl HotkeySettings {
    /// The shortcuts in force, or why these settings can't be used.
    pub fn bindings(&self) -> Result<Bindings, String> {
        if !self.enabled {
            return Ok(Bindings::default());
        }
        let mut list = Vec::new();
        if let Some(j) = chord_or_none::<JumpChords>(&self.jump)? {
            list.extend((1..=9).map(|n| (Action::Jump(n), j.chord(n))));
        }
        for (s, a) in [(&self.cycle, Action::Cycle), (&self.lock_pointer, Action::LockPointer), (&self.home, Action::Home)] {
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
    /// Turn every shortcut off.
    #[arg(long, conflicts_with = "on")]
    pub off: bool,
    /// Turn the shortcuts on again.
    #[arg(long)]
    pub on: bool,
}

impl HotkeyArgs {
    fn changes_anything(&self) -> bool {
        self.off || self.on || self.jump.is_some() || self.cycle.is_some() || self.lock_pointer.is_some() || self.home_key.is_some()
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
        ] {
            if let Some(v) = v {
                *field = canonical::<Chord>(v).map_err(|e| format!("{flag}: {e}"))?;
            }
        }
        if self.on || self.jump.is_some() || self.cycle.is_some() || self.lock_pointer.is_some() || self.home_key.is_some() {
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
        // The requests are the ones the command line sends.
        assert_eq!(Request::parse("switch next"), request(Action::Cycle, &o).ok());
        assert_eq!(Request::parse("lock-pointer toggle"), request(Action::LockPointer, &o).ok());
    }

    #[test]
    fn defaults_are_valid_and_distinct() {
        assert!(HotkeyArgs { jump: Some(String::new()), ..Default::default() }.apply(&mut HotkeySettings::default()).is_err());
        assert!(HotkeyArgs { cycle: Some(" ".into()), ..Default::default() }.apply(&mut HotkeySettings::default()).is_err());
        let b = HotkeySettings::default().bindings().unwrap();
        assert_eq!(b.iter().count(), 12);
        assert_eq!(b.action("ctrl+alt+5".parse().unwrap()), Some(Action::Jump(5)));
        assert_eq!(b.action("ctrl+alt+h".parse().unwrap()), Some(Action::Home));
        let off = HotkeySettings { enabled: false, ..HotkeySettings::default() };
        assert!(off.bindings().unwrap().is_empty());
        let some = HotkeySettings { jump: "none".into(), cycle: String::new(), ..HotkeySettings::default() };
        assert_eq!(some.bindings().unwrap().iter().count(), 2);
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
        let _ = std::fs::remove_dir_all(dir);
    }
}
