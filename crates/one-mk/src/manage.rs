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
use anyhow::{Result, bail};
use one_mk_net::{Device, Fingerprint, Swap, TrustStore};
use one_mk_proto::Side;
pub use one_mk_ctl::status::{LayoutStatus, PeerStatus, Status, on_your, placement, takeback_hint};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unknown_device(device: &str) -> anyhow::Error {
    anyhow::anyhow!("no paired computer called {device:?}; see `one-mk devices` for names and ids")
}

pub async fn status(dir: &Path) -> Result<()> {
    if let Some(notice) = crate::update::notice(dir) { println!("Updates: {notice}."); }
    if !crate::update_key::PRODUCTION_KEY { println!("Updates: publisher signing key not configured; install releases manually."); }
    #[cfg(any(windows, target_os = "macos"))]
    println!("Updates: executable replacement is untested on this operating system.");
    let reply = control::request(dir, Request::Status).await?;
    match reply.as_deref().and_then(Status::decode) {
        Some(s) => {
            print!("{}", s.render());
            if let Some(line) = crate::settings::Settings::load(dir).ok().and_then(|s| s.input.summary()) {
                println!("{line}");
            }
        }
        None if reply.is_some() => bail!("unexpected answer from one-mk: {reply:?}"),
        None => {
            println!("one-mk is not running.");
            println!("Start it with `one-mk run`, or make it start when you log in with `one-mk install`.");
        }
    }
    let settings = crate::settings::Settings::load(dir)?;
    if settings.hooks.blocked.is_some() {
        println!("{}", settings.hooks.describe());
    }
    println!("Clipboard: {}.", settings.clipboard.summary());
    Ok(())
}

/// `one-mk takeback`: makes the computer controlling this one let go.
pub async fn takeback(dir: &Path) -> Result<()> {
    let reply = control::request(dir, Request::TakeBack).await?;
    match reply.as_deref() {
        None => println!("one-mk is not running."),
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
    let mut trust = TrustStore::load(dir)?;
    let Some(found) = trust.find(device) else { return Err(unknown_device(device)) };
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
    let dev = dev.clone();
    trust.save()?;
    Ok(dev)
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

/// [`place`] with the clock given.
pub fn place_at(dev: &mut Device, side: Side, offset: Option<(i32, i32)>, now: u64) {
    dev.side = side.to_string();
    dev.offset = offset;
    dev.layout_version = now.max(dev.layout_version.saturating_add(1));
}

pub async fn layout(dir: &Path, device: &str, side: Option<Side>, offset: Option<&str>, swap: Option<Swap>) -> Result<()> {
    if side.is_none() && offset.is_none() && swap.is_none() {
        bail!("say where {device:?} is (left, right, top or bottom), and/or use --offset DX,DY|none or --swap-ctrl-cmd on|off|auto");
    }
    let offset = offset.map(parse_offset).transpose()?;
    let dev = set_layout(dir, device, side, offset, swap)?;
    let name = one_mk_proto::sanitize_name(&dev.name);
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
    if control::request(dir, Request::Reload).await?.is_some() {
        println!("Applied right away.{}", if moved { " Its computer picks up the change too." } else { "" });
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
    }
}
