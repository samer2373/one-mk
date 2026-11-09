//! The commands that talk to a running daemon or change its settings:
//! `status`, `takeback` and `layout`.
//!
//! Layout data lives in `devices.toml`, one record per paired computer:
//! `side` ([`Side`]), `offset` (where its (0,0) sits in our pixels, if it
//! was placed exactly: the pointer then crosses only where the screens
//! touch) and `layout_version` (when either was last chosen).
//! [`set_layout`] is the one way to change them here: it bumps the version,
//! so the change wins when the two computers compare layouts (a tie is
//! broken by [`resolve_layout`]), and a running
//! daemon applies it on `Request::Reload` and sends the peer the mirror
//! image. Profiles (`profiles.rs`) go through it too.

use crate::control::{self, Request};
use anyhow::{Context, Result, bail};
use one_mk_net::{Device, Fingerprint, PlacedBy, Swap, TrustStore};
use one_mk_proto::Side;
pub use one_mk_ctl::status::{LayoutStatus, LinkStats, PeerStatus, Status, on_your, placement, takeback_hint};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unknown_device(device: &str) -> anyhow::Error {
    anyhow::anyhow!("no paired computer called {device:?}; see `one-mk devices` for names and ids")
}

/// Why `device` named no one computer in `trust`: none has that name or
/// id, or several do (two computers may share a name).
pub fn not_found(trust: &TrustStore, device: &str) -> anyhow::Error {
    match trust.explain_miss(device) {
        Some(why) => anyhow::anyhow!(why),
        None => unknown_device(device),
    }
}

/// `one-mk rename`: gives a paired computer a name of this computer's own
/// (or, with `reset`, takes it away). The computer keeps calling itself
/// what it did; a running one-mk shows the new name after `Reload`.
pub fn rename(dir: &Path, device: &str, new_name: &str, reset: bool) -> Result<String> {
    let new_name = one_mk_proto::sanitize_name(new_name);
    TrustStore::update(dir, |trust| {
        let Some(found) = trust.find(device) else { return Err(not_found(trust, device)) };
        let (fp, own, before) = (found.fingerprint.clone(), one_mk_proto::sanitize_name(&found.name), trust.display(found));
        if !reset {
            if new_name == "unnamed device" && !device.is_empty() {
                bail!("say the new name after the computer, for example: one-mk rename {device} \"Office PC\"");
            }
            if let Some(other) = trust.devices().iter().find(|d| d.fingerprint != fp && d.label().trim().eq_ignore_ascii_case(&new_name)) {
                bail!("another paired computer is already shown as {:?} (id {}); pick a different name", one_mk_proto::sanitize_name(other.label()), other.short_id());
            }
        }
        let fingerprint: Fingerprint = fp.parse().context("corrupt fingerprint in devices.toml")?;
        let d = trust.get_mut(&fingerprint).context("corrupt fingerprint in devices.toml")?;
        d.alias = (!reset && new_name != own).then_some(new_name);
        Ok(match &d.alias {
            Some(alias) => format!("{before} is now called {alias:?} here (it still calls itself {own:?})."),
            None => format!("{before} is shown by the name it gave itself, {own:?}."),
        })
    })
}

pub async fn status(dir: &Path) -> Result<()> {
    if let Some(notice) = crate::update::notice(dir) { println!("Updates: {notice}."); }
    #[cfg(any(windows, target_os = "macos"))]
    println!("Updates: executable replacement is untested on this operating system.");
    let reply = control::request(dir, Request::Status).await?;
    match reply.as_deref().and_then(Status::decode) {
        Some(s) => {
            print!("{}", s.render());
            if let Ok(Some((count, bytes, age, _))) = partials_of(dir, false).await
                && let Some(text) = partials_text(count, bytes, age)
            {
                println!("{text}");
            }
            if let Some(line) = crate::settings::Settings::load_lenient(dir).ok().and_then(|s| s.input.summary()) {
                println!("{line}");
            }
        }
        None if reply.is_some() => bail!("unexpected answer from one-mk: {reply:?}"),
        None => {
            println!("one-mk is not running.");
            println!("Start it with `one-mk run`, or make it start when you log in with `one-mk install`.");
            let trust = TrustStore::load(dir)?;
            for device in trust.devices().iter().filter(|device| device.placed_by == PlacedBy::Pending) {
                println!("{}", crate::pairing_layout::warning_for(&trust, device));
            }
        }
    }
    // The rest of the report must not be lost to a settings file that can't be
    // read at all: say so, and show what the defaults would be.
    let settings = crate::settings::Settings::view(dir).unwrap_or_else(|error| {
        println!("settings.toml can't be read, so the defaults are shown for the rest: {}.", format!("{error:#}").lines().next().unwrap_or(""));
        crate::settings::Settings::default()
    });
    if settings.hooks.blocked.is_some() {
        println!("{}", settings.hooks.describe());
    }
    println!("Clipboard: {}.", settings.clipboard.summary());
    if settings.wallpaper.enabled || !settings.wallpaper.grants.is_empty() {
        println!("{}", crate::wallpaper::summary(&settings.wallpaper));
    }
    Ok(())
}

/// The unfinished received files a running daemon keeps for resuming:
/// (how many, bytes, seconds since the oldest was written, how many a clear just removed).
pub async fn partials_of(dir: &Path, clear: bool) -> Result<Option<(usize, u64, u64, usize)>> {
    let Some(reply) = control::request(dir, Request::Partials { clear }).await? else { return Ok(None) };
    let mut lines = reply.lines();
    if lines.next() != Some("ok") {
        bail!("{}", reply.trim().trim_start_matches("error "));
    }
    let mut number = || lines.next().and_then(|line| line.parse::<u64>().ok()).context("unexpected answer from one-mk");
    let (count, bytes, age, removed) = (number()?, number()?, number()?, number()?);
    Ok(Some((count as usize, bytes, age, removed as usize)))
}

/// What to tell people about leftover partial files, or `None` when there are none.
pub fn partials_text(count: usize, bytes: u64, age: u64) -> Option<String> {
    (count > 0).then(|| format!(
        "{count} unfinished received file{} ({}) kept so a send can resume; the oldest was last written {} and files older than {} days are deleted by themselves. Delete them now with `one-mk partials --clear`.",
        if count == 1 { "" } else { "s" },
        crate::clip::mb(usize::try_from(bytes).unwrap_or(usize::MAX)),
        if age >= 86_400 { format!("{} days ago", age / 86_400) } else if age >= 3600 { format!("{} hours ago", age / 3600) } else if age >= 60 { format!("{} minutes ago", age / 60) } else { "less than a minute ago".to_owned() },
        crate::transfer::PARTIAL_MAX_AGE.as_secs() / 86_400,
    ))
}

/// `one-mk partials [--clear]`: the unfinished received files kept so an
/// interrupted send can resume, and a way to delete them.
///
/// With no one-mk running they are only files on this computer, so this
/// works then too (in `downloads`, or the usual received-files folder),
/// unless a one-mk holds the settings folder without answering yet.
pub async fn partials(dir: &Path, clear: bool, downloads: Option<std::path::PathBuf>) -> Result<()> {
    let answer = match partials_of(dir, clear).await? {
        Some(answer) => answer,
        None => {
            // No daemon answered: only go on if none holds the folder (one that
            // is still starting, or stuck, must not be raced for the files).
            if control::InstanceLock::try_acquire(dir)?.is_none() {
                bail!("one-mk is starting or busy with this settings folder and did not answer; try again in a moment");
            }
            let root = match downloads {
                Some(root) => root,
                None => crate::default_downloads()?,
            };
            let removed = if clear { crate::transfer::clear_idle_partials_in(&root) } else { 0 };
            let kept = crate::transfer::partials_in(&root);
            println!("one-mk is not running; looking in {}.", root.display());
            (kept.count, kept.bytes, kept.oldest.as_secs(), removed)
        }
    };
    let (count, bytes, age, removed) = answer;
    if clear {
        println!("Deleted {removed} unfinished received file{}.", if removed == 1 { "" } else { "s" });
        if count > 0 { println!("{count} left alone because {} still being received (try again in 30 seconds if that sender stopped).", if count == 1 { "it is" } else { "they are" }); }
        return Ok(());
    }
    match partials_text(count, bytes, age) {
        Some(text) => println!("{text}"),
        None => println!("No unfinished received files are kept."),
    }
    Ok(())
}

/// `one-mk takeback`: makes the computer controlling this one let go.
pub async fn takeback(dir: &Path) -> Result<()> {
    let reply = control::request(dir, Request::TakeBack).await?;
    match reply.as_deref() {
        None => bail!("{}", control::NOT_RUNNING),
        Some("none\n") => println!("No other computer is controlling this one."),
        Some(r) => match r.strip_prefix("ok\n") {
            Some(name) if !name.trim().is_empty() => println!("Took control back from {}.", name.trim()),
            Some(_) => println!("Took control back."),
            None => bail!("unexpected answer from one-mk: {r:?}"),
        },
    }
    Ok(())
}

/// `DX,DY` or `none`, for `one-mk layout --offset`.
pub fn parse_offset(s: &str) -> Result<Option<(i32, i32)>> {
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let parsed = s.split_once(',').and_then(|(x, y)| Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?)));
    let Some((x, y)) = parsed else { bail!("expected --offset DX,DY (e.g. -1920,0) or none, not {s:?}") };
    let max = one_mk_net::MAX_OFFSET;
    if x.max(y) > max || x.min(y) < -max {
        bail!("--offset must be within -{max} and {max} pixels on each axis");
    }
    Ok(Some((x, y)))
}

/// Checks `--offset` when the command line is read (see [`parse_offset`]).
pub fn offset_arg(s: &str) -> Result<String, String> {
    parse_offset(s).map(|_| s.to_owned()).map_err(|e| e.to_string())
}

/// Changes a paired device's side, offset and/or Ctrl/Cmd setting in
/// `devices.toml`. A new side or offset gets a version newer than any
/// seen, so it wins when the two computers compare layouts; a new side
/// also drops an offset that was meant for the old side, unless a new
/// one is given.
pub fn set_layout(
    dir: &Path,
    device: &str,
    side: Option<Side>,
    offset: Option<Option<(i32, i32)>>,
    swap: Option<Swap>,
) -> Result<Device> {
    set_layout_at(dir, device, side, offset, swap, now_ms())
}

/// [`set_layout`] with the clock (milliseconds since 1970) given, so a test
/// can make two computers choose under the same version.
pub fn set_layout_at(
    dir: &Path,
    device: &str,
    side: Option<Side>,
    offset: Option<Option<(i32, i32)>>,
    swap: Option<Swap>,
    now: u64,
) -> Result<Device> {
    TrustStore::update(dir, |trust| {
        let Some(found) = trust.find(device) else { return Err(not_found(trust, device)) };
        let Some(fp) = found.fingerprint() else { bail!("corrupt fingerprint for {device:?} in devices.toml") };
        let dev = trust.get_mut(&fp).expect("found above");
        if side.is_some() || offset.is_some() {
            let new_side = side.unwrap_or_else(|| dev.side());
            // An offset chosen for one side means something else on another.
            let keep = if new_side == dev.side() { dev.offset } else { None };
            place_at(dev, new_side, offset.unwrap_or(keep), now);
        }
        if let Some(side) = side {
            if dev.side() != side {
                dev.offset = None;
            }
            dev.side = side.to_string();
        }
        if let Some(offset) = offset {
            dev.offset = offset;
        }
        if let Some(swap) = swap {
            dev.swap_ctrl_cmd = swap;
        }
        Ok(dev.clone())
    })
}

/// What to do with a layout the peer sent, given its version against ours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutVerdict {
    /// Ours stands (the peer's is older, or says what we already hold).
    Keep,
    /// The peer's replaces ours.
    Adopt,
    /// A tie, and ours wins: bump our version and send it again, so the
    /// peer, whatever it runs, ends up with ours.
    Win,
}

/// Compares the peer's layout (`theirs`, a version) with ours (`ours`), as
/// both computers do on receipt. `same` says whether the two describe the
/// same arrangement (the peer's, already mirrored, equals ours).
///
/// A higher version wins. Equal versions (two changes made in the same
/// millisecond on both computers) are broken the same way on both ends: the
/// numerically larger certificate fingerprint, compared as bytes, wins.
/// Equal versions with the same arrangement settle nothing, so nothing is
/// sent: that is what stops a tie from bouncing between the computers.
///
/// An older peer doesn't know the rule and ignores equal versions, so a
/// winner re-sends under a higher version, which every build adopts.
pub fn resolve_layout(ours: u64, theirs: u64, same: bool, our_fp: &Fingerprint, peer_fp: &Fingerprint) -> LayoutVerdict {
    match theirs.cmp(&ours) {
        std::cmp::Ordering::Less => LayoutVerdict::Keep,
        std::cmp::Ordering::Greater => LayoutVerdict::Adopt,
        std::cmp::Ordering::Equal if same => LayoutVerdict::Keep,
        std::cmp::Ordering::Equal if peer_fp.0 > our_fp.0 => LayoutVerdict::Adopt,
        std::cmp::Ordering::Equal => LayoutVerdict::Win,
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Puts `dev` on `side`, at `offset`, under a new layout version:
/// `now`, but always above the one it has. A layout received from the peer
/// is stored under its version, so a change made after receiving it is
/// above that too.
pub fn place(dev: &mut Device, side: Side, offset: Option<(i32, i32)>) {
    place_at(dev, side, offset, now_ms());
}

/// [`place`] with the clock given. The place is the user's choice from
/// now on (see [`PlacedBy`]).
pub fn place_at(dev: &mut Device, side: Side, offset: Option<(i32, i32)>, now: u64) {
    dev.side = side.to_string();
    dev.offset = offset;
    dev.layout_version = now.max(dev.layout_version.saturating_add(1));
    dev.placed_by = PlacedBy::User;
}

/// Layout learning (`[edge] learn = auto`) puts `fp` on `side`, anywhere
/// along it, as [`set_layout`] would, but marked as learned, so learning
/// may move it again until it is used. Refuses (returns `None`) if its
/// place is no longer a guess: `devices.toml` may have changed since the
/// daemon read it. Returns the device as it was before.
pub fn learn_layout(dir: &Path, fp: &Fingerprint, side: Side) -> Result<Option<Device>> {
    TrustStore::update_within(dir, one_mk_net::DAEMON_LOCK_WAIT, |trust| {
        let Some(dev) = trust.get_mut(fp).filter(|d| d.learnable() && d.side() != side) else { return Ok(None) };
        let before = dev.clone();
        place_at(dev, side, None, now_ms());
        dev.placed_by = PlacedBy::Learned;
        Ok(Some(before))
    })
}

pub async fn layout(dir: &Path, device: &str, side: Option<Side>, offset: Option<&str>, swap: Option<Swap>) -> Result<()> {
    if side.is_none() && offset.is_none() && swap.is_none() {
        bail!("say where {device:?} is (left, right, top or bottom), and/or use --offset DX,DY|none or --swap-ctrl-cmd on|off|auto");
    }
    let offset = offset.map(parse_offset).transpose()?;
    let dev = set_layout(dir, device, side, offset, swap)?;
    // As `status` shows it (with its id if another computer is called the same).
    let name = TrustStore::load(dir).map_or_else(|_| one_mk_proto::sanitize_name(dev.label()), |trust| trust.display(&dev));
    let moved = side.is_some() || offset.is_some();
    if let Some(side) = side {
        println!("{name} is now {}.", on_your(side));
    }
    match offset {
        Some(Some((x, y))) => println!("{name}'s (0,0) sits at {x},{y} of yours; the pointer crosses where the screens touch."),
        Some(None) => println!("{name} has no offset; the pointer crosses anywhere along its side."),
        None => {}
    }
    match swap {
        Some(Swap::On) => println!("Ctrl and Cmd are swapped for keys sent to {name}."),
        Some(Swap::Off) => println!("Ctrl and Cmd are not swapped for keys sent to {name}."),
        Some(Swap::Auto) => println!("Ctrl and Cmd are swapped for keys sent to {name} only between a Mac and another computer."),
        None => {}
    }
    if control::request(dir, Request::Reload).await.map_err(control::NotAnswering::saved)?.is_some() {
        println!("Applied right away.{}", if moved { " Its computer picks up the change too." } else { "" });
        // Only the running one-mk knows the other computer's screens.
        if moved && let Some(status) = control::request(dir, Request::Status).await.ok().flatten().as_deref().and_then(Status::decode) {
            for warning in status.layout.warnings.iter().filter(|w| w.contains(&format!("layout of {name} "))) {
                println!("Warning: {warning}.");
            }
        }
    } else {
        println!("one-mk is not running; this takes effect the next time it starts.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use LayoutVerdict::{Adopt, Keep, Win};

    fn fp(first: u8) -> Fingerprint {
        let mut b = [0u8; 32];
        b[0] = first;
        Fingerprint(b)
    }

    #[test]
    fn newer_version_wins_whatever_the_fingerprints() {
        for (small, big) in [(fp(1), fp(2)), (fp(2), fp(1))] {
            for same in [false, true] {
                assert_eq!(resolve_layout(5, 6, same, &small, &big), Adopt);
                assert_eq!(resolve_layout(5, 6, same, &big, &small), Adopt);
                assert_eq!(resolve_layout(6, 5, same, &small, &big), Keep);
                assert_eq!(resolve_layout(6, 5, same, &big, &small), Keep);
            }
        }
    }

    #[test]
    fn a_tie_goes_to_the_larger_fingerprint_on_both_ends() {
        let (small, big) = (fp(1), fp(2));
        // On the small one's computer the peer (big) wins; on the big one's, we do.
        assert_eq!(resolve_layout(7, 7, false, &small, &big), Adopt);
        assert_eq!(resolve_layout(7, 7, false, &big, &small), Win);
        // Compared as bytes, from the first byte: later bytes only break equal prefixes.
        let (mut lo, mut hi) = (fp(9), fp(9));
        (lo.0[31], hi.0[31]) = (0xff, 0);
        assert_eq!(resolve_layout(0, 0, false, &hi, &lo), Adopt);
        assert_eq!(resolve_layout(0, 0, false, &lo, &hi), Win);
        let (mut lo, mut hi) = (fp(0x7f), fp(0x80));
        (lo.0[1], hi.0[1]) = (0xff, 0);
        assert_eq!(resolve_layout(3, 3, false, &hi, &lo), Win);
        assert_eq!(resolve_layout(3, 3, false, &lo, &hi), Adopt);
    }

    /// Two computers with one name: the error says so and asks for the id.
    #[test]
    fn a_shared_name_asks_for_the_id() {
        let dir = std::env::temp_dir().join(format!("one-mk-shared-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TrustStore::update(&dir, |t| {
            for b in [0xaa_u8, 0xbb] {
                t.upsert(Device { name: "Victus".into(), fingerprint: Fingerprint([b; 32]).to_string(), ..Device::default() });
            }
            Ok(())
        }).unwrap();
        let err = set_layout(&dir, "victus", Some(Side::Left), None, None).unwrap_err().to_string();
        assert!(err.starts_with("2 paired computers are called \"victus\"; use its id"), "{err}");
        let id = Fingerprint([0xaa; 32]).short();
        assert_eq!(set_layout(&dir, &id, Some(Side::Left), None, None).unwrap().side(), Side::Left);
        assert!(set_layout(&dir, "nobody", Some(Side::Left), None, None).unwrap_err().to_string().starts_with("no paired computer called"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `one-mk rename`: a name of this computer's own for a paired one; it
    /// refuses a name another computer already shows, and `--reset` goes back.
    #[test]
    fn renaming_a_paired_computer_is_local_and_reversible() {
        let dir = std::env::temp_dir().join(format!("one-mk-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TrustStore::update(&dir, |t| {
            for b in [0xaa_u8, 0xbb] {
                t.upsert(Device { name: "Victus".into(), fingerprint: Fingerprint([b; 32]).to_string(), ..Device::default() });
            }
            Ok(())
        }).unwrap();
        let (a, b) = (Fingerprint([0xaa; 32]).short(), Fingerprint([0xbb; 32]).short());
        // By a name that fits both: it says to use the id.
        assert!(rename(&dir, "Victus", "Office", false).unwrap_err().to_string().contains("2 paired computers are called"));
        let said = rename(&dir, &a, "  Office   PC ", false).unwrap();
        assert!(said.contains("\"Office PC\"") && said.contains("still calls itself \"Victus\""), "{said}");
        let trust = TrustStore::load(&dir).unwrap();
        assert_eq!(trust.display(&trust.devices()[0]), "Office PC");
        assert_eq!(trust.display(&trust.devices()[1]), "Victus", "the other is no longer one of two");
        assert_eq!(trust.devices()[0].name, "Victus");
        // Not a name the other already shows (either way round).
        assert!(rename(&dir, &b, "office pc", false).unwrap_err().to_string().contains("already shown as"));
        assert!(rename(&dir, "Office PC", "Victus", false).unwrap_err().to_string().contains("already shown as"));
        // The new name finds it, the old one too (it is unique now).
        assert_eq!(set_layout(&dir, "office pc", Some(Side::Left), None, None).unwrap().short_id(), a);
        assert_eq!(set_layout(&dir, "Victus", Some(Side::Right), None, None).unwrap().short_id(), b);
        // Back to what it calls itself.
        assert!(rename(&dir, "Office PC", "", true).unwrap().contains("name it gave itself"));
        assert!(TrustStore::load(&dir).unwrap().devices()[0].alias.is_none());
        assert!(!std::fs::read_to_string(dir.join("devices.toml")).unwrap().contains("alias"));
        assert!(rename(&dir, "nobody", "x", false).unwrap_err().to_string().starts_with("no paired computer called"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_offset() {
        assert_eq!(super::parse_offset("-2592, 98").unwrap(), Some((-2592, 98)));
        assert_eq!(super::parse_offset("None").unwrap(), None);
        assert!(super::parse_offset("10").is_err());
        assert!(super::parse_offset("16777217,0").is_err(), "beyond what the wire carries");
    }

    #[test]
    fn a_tie_with_the_same_layout_settles_nothing() {
        let (small, big) = (fp(1), fp(2));
        assert_eq!(resolve_layout(7, 7, true, &small, &big), Keep);
        assert_eq!(resolve_layout(7, 7, true, &big, &small), Keep);
        assert_eq!(resolve_layout(0, 0, true, &big, &small), Keep);
    }

    #[test]
    fn a_change_after_receiving_is_above_the_peers_version() {
        let mut dev = Device { layout_version: 1_000, ..Device::default() };
        place_at(&mut dev, Side::Left, None, 10); // the clock is behind
        assert_eq!(dev.layout_version, 1_001);
        place_at(&mut dev, Side::Left, Some((-1920, 0)), 5_000);
        assert_eq!((dev.layout_version, dev.offset), (5_000, Some((-1920, 0))));
        place_at(&mut dev, Side::Top, None, 5_000); // same millisecond
        assert_eq!(dev.layout_version, 5_001);
        assert_eq!(dev.placed_by, PlacedBy::User);
    }

    /// Learning places only a guessed computer, marks it learned (so it
    /// may be learned again), and never touches one the user placed.
    #[test]
    fn learning_moves_only_a_guess() {
        let dir = std::env::temp_dir().join(format!("one-mk-manage-learn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = fp(7);
        let mut t = TrustStore::load(&dir).unwrap();
        t.upsert(Device { name: "laptop".into(), fingerprint: id.to_string(), side: "right".into(), ..Device::default() });
        t.save().unwrap();
        let before = learn_layout(&dir, &id, Side::Left).unwrap().expect("a guess is learned");
        assert_eq!(before.side(), Side::Right);
        let d = TrustStore::load(&dir).unwrap().get(&id).unwrap().clone();
        assert_eq!((d.side(), d.offset, d.placed_by), (Side::Left, None, PlacedBy::Learned));
        assert!(d.layout_version > 0, "a newer version, so the other computer adopts the mirror");
        assert!(learn_layout(&dir, &id, Side::Left).unwrap().is_none(), "already there");
        assert!(learn_layout(&dir, &id, Side::Top).unwrap().is_some(), "learned again");
        set_layout(&dir, "laptop", Some(Side::Right), None, None).unwrap();
        assert!(learn_layout(&dir, &id, Side::Left).unwrap().is_none(), "the user's choice stands");
        assert_eq!(TrustStore::load(&dir).unwrap().get(&id).unwrap().side(), Side::Right);
        assert!(learn_layout(&dir, &fp(8), Side::Left).unwrap().is_none(), "not paired");
        let _ = std::fs::remove_dir_all(dir);
    }
}
