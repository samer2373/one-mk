use crate::identity::Fingerprint;
use anyhow::{Context, Result};
use one_mk_proto::Side;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use crate::files::COMMAND_LOCK_WAIT;
use std::time::Duration;

/// Whether Ctrl and Cmd (Meta) trade places for keys sent to a device, so
/// copy and paste shortcuts work between a Mac and a PC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Swap {
    /// Swap when exactly one of the two computers is a Mac.
    #[default]
    Auto,
    On,
    Off,
}

impl fmt::Display for Swap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Swap::Auto => "auto",
            Swap::On => "on",
            Swap::Off => "off",
        })
    }
}

impl FromStr for Swap {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Swap::Auto),
            "on" => Ok(Swap::On),
            "off" => Ok(Swap::Off),
            _ => Err(format!("expected on, off or auto, not {s:?}")),
        }
    }
}

/// A paired device.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Device {
    /// The name the computer gave itself.
    pub name: String,
    /// A name chosen on THIS computer (`one-mk rename`), shown instead of
    /// `name` here and nowhere else; it tells apart two computers that call
    /// themselves the same. Older files have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub fingerprint: String,
    /// Which side of this machine the device sits on.
    #[serde(default = "default_side")]
    pub side: String,
    /// Last addresses it was reached at, for networks that block discovery.
    #[serde(default)]
    pub addrs: Vec<String>,
    /// When `side` (or `offset`) was last chosen (milliseconds since 1970,
    /// but always above the previous value, and above any version received
    /// from the device). Both computers keep the newer layout; a tie is
    /// broken by the larger fingerprint (`one_mk::manage::resolve_layout`).
    #[serde(default)]
    pub layout_version: u64,
    /// Ctrl/Cmd swapping for keys sent to this device.
    #[serde(default)]
    pub swap_ctrl_cmd: Swap,
    /// The device's OS when last connected (see `one_mk_proto::os`).
    #[serde(default)]
    pub os: u8,
    /// Where the device's desktop origin sits in this machine's logical
    /// pixels; the pointer crosses only where the screens then touch.
    /// None: anywhere along `side`, scaled to fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<(i32, i32)>,
    /// Who chose `side` and `offset` (see [`PlacedBy`]); only a guess may
    /// be moved by layout learning.
    #[serde(default, skip_serializing_if = "PlacedBy::is_default")]
    pub placed_by: PlacedBy,
}

/// Who chose where a device sits. Pushing the pointer toward a computer
/// (`[edge] learn`) may move it only while its place is a guess.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlacedBy {
    /// Not said: pairing's guess while `layout_version` is 0, the user's
    /// choice otherwise (files from before this field have only that).
    #[default]
    Default,
    /// `one-mk layout`, the window, a layout profile, or the other
    /// computer (whose user may have chosen it).
    User,
    /// Layout learning, from the pointer pushed toward it.
    Learned,
    /// The guessed or learned place has been used: the pointer crossed to
    /// the device through that edge.
    Confirmed,
    Pairing,
    Pending,
}

impl PlacedBy {
    fn is_default(&self) -> bool {
        *self == PlacedBy::Default
    }
}

impl<'de> Deserialize<'de> for PlacedBy {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "user" => PlacedBy::User,
            "learned" => PlacedBy::Learned,
            "confirmed" => PlacedBy::Confirmed,
            "pairing" => PlacedBy::Pairing,
            "pending" => PlacedBy::Pending,
            _ => PlacedBy::Default,
        })
    }
}

fn default_side() -> String {
    Side::Right.to_string()
}

impl Device {
    /// The name to show for it on this computer: the one chosen here, else
    /// the one it gave itself.
    pub fn label(&self) -> &str {
        self.alias.as_deref().filter(|a| !a.trim().is_empty()).unwrap_or(&self.name)
    }
    pub fn side(&self) -> Side {
        self.side.parse().unwrap_or(Side::Right)
    }
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint.parse().ok()
    }
    pub fn short_id(&self) -> String {
        self.fingerprint().map(|f| f.short()).unwrap_or_default()
    }
    /// Its place is only a guess that layout learning may change: pairing's
    /// default, or learned and not used since, and not at an exact offset.
    pub fn learnable(&self) -> bool {
        self.offset.is_none()
            && match self.placed_by {
                PlacedBy::Default => self.layout_version == 0,
                PlacedBy::Learned | PlacedBy::Pairing => true,
                PlacedBy::User | PlacedBy::Confirmed | PlacedBy::Pending => false,
            }
    }
}

/// Largest offset (either axis) kept from `devices.toml`, as on the wire
/// (`Control::Layout`): far beyond any desktop, and small enough that the
/// geometry never overflows. A hand edit past it is cut down to it.
pub const MAX_OFFSET: i32 = 1 << 24;

fn bounded_offset((x, y): (i32, i32)) -> (i32, i32) {
    (x.clamp(-MAX_OFFSET, MAX_OFFSET), y.clamp(-MAX_OFFSET, MAX_OFFSET))
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, rename = "device")]
    devices: Vec<Device>,
}

/// Held (an exclusive advisory lock on this file next to `devices.toml`)
/// while the paired computers are changed; see [`TrustStore::update`].
const LOCK_FILE: &str = "devices.toml.lock";

/// Devices this machine has paired with, in `devices.toml`.
///
/// The daemon and every `one-mk` command read and write this file. A change
/// goes through [`TrustStore::update`]: it waits for an exclusive lock,
/// reads what is on disk *now*, applies the change and writes the result
/// through a private temporary file that is flushed and renamed over the
/// old one. So a change made by someone else in between is never written
/// over with a stale copy, and a reader never sees half a file.
#[derive(Clone)]
pub struct TrustStore {
    path: PathBuf,
    file: File,
}

impl TrustStore {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("devices.toml");
        let mut file: File = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("reading {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(e) => return Err(e).with_context(|| format!("reading paired computers {}", path.display())),
        };
        for device in &mut file.devices {
            device.offset = device.offset.map(bounded_offset);
        }
        Ok(Self { path, file })
    }

    /// Writes this copy over `devices.toml`, under the lock. Whatever was
    /// saved since this copy was loaded is lost: to change the paired
    /// computers use [`TrustStore::update`].
    pub fn save(&self) -> Result<()> {
        let dir = self.dir();
        crate::identity::secure_dir(&dir)?;
        let _lock = lock(&dir, None)?;
        self.write()
    }

    /// Changes the paired computers: waits for an exclusive lock on a file
    /// next to `devices.toml` (at most [`COMMAND_LOCK_WAIT`]), loads what is
    /// there now, applies `change` and saves if anything changed. If
    /// `change` fails nothing is written. Returns `change`'s result.
    pub fn update<R>(dir: &Path, change: impl FnOnce(&mut TrustStore) -> Result<R>) -> Result<R> {
        Self::update_locked(dir, None, change)
    }

    /// [`TrustStore::update`], but giving up (with an error) if the lock
    /// is not free within `wait`: for the daemon, whose event loop must
    /// never hang on a stopped command.
    pub fn update_within<R>(dir: &Path, wait: Duration, change: impl FnOnce(&mut TrustStore) -> Result<R>) -> Result<R> {
        Self::update_locked(dir, Some(wait), change)
    }

    fn update_locked<R>(dir: &Path, wait: Option<Duration>, change: impl FnOnce(&mut TrustStore) -> Result<R>) -> Result<R> {
        crate::identity::secure_dir(dir)?;
        let _lock = lock(dir, wait)?;
        // Unlocked when `_lock` is dropped (also if this fails or panics).
        let mut store = Self::load(dir)?;
        let before = toml::to_string_pretty(&store.file)?;
        let out = change(&mut store)?;
        if toml::to_string_pretty(&store.file)? != before {
            store.write()?;
        }
        Ok(out)
    }

    fn dir(&self) -> PathBuf {
        self.path.parent().map_or_else(PathBuf::new, Path::to_path_buf)
    }

    /// Writes through a temporary file of this writer's own, flushed to
    /// disk before it is renamed over `devices.toml` (and the folder
    /// flushed after), so a crash leaves the old file or the new one.
    fn write(&self) -> Result<()> {
        static N: AtomicU32 = AtomicU32::new(0);
        let name = self.path.file_name().and_then(|n| n.to_str()).unwrap_or("devices.toml");
        let tmp = self.path.with_file_name(format!("{name}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let write = || -> Result<()> {
            let text = toml::to_string_pretty(&self.file)?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&tmp).with_context(|| format!("writing paired computers {}", tmp.display()))?;
            file.write_all(text.as_bytes()).with_context(|| format!("writing paired computers {}", tmp.display()))?;
            file.sync_all().with_context(|| format!("flushing paired computers {}", tmp.display()))?;
            drop(file);
            std::fs::rename(&tmp, &self.path).with_context(|| format!("saving paired computers {}", self.path.display()))?;
            crate::files::sync_dir(&self.dir())
        };
        write().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    pub fn devices(&self) -> &[Device] {
        &self.file.devices
    }

    pub fn get(&self, fp: &Fingerprint) -> Option<&Device> {
        let hex = fp.to_string();
        self.file.devices.iter().find(|d| d.fingerprint == hex)
    }

    pub fn get_mut(&mut self, fp: &Fingerprint) -> Option<&mut Device> {
        let hex = fp.to_string();
        self.file.devices.iter_mut().find(|d| d.fingerprint == hex)
    }

    pub fn is_trusted(&self, fp: &Fingerprint) -> bool {
        self.get(fp).is_some()
    }

    /// Adds or replaces a device, keeping its settings if already known.
    pub fn upsert(&mut self, mut device: Device) {
        if let Some(existing) = self.file.devices.iter_mut().find(|d| d.fingerprint == device.fingerprint) {
            device.side = std::mem::take(&mut existing.side);
            device.layout_version = existing.layout_version;
            device.swap_ctrl_cmd = existing.swap_ctrl_cmd;
            device.offset = existing.offset;
            device.placed_by = existing.placed_by;
            device.alias = existing.alias.take();
            *existing = device;
        } else {
            self.file.devices.push(device);
        }
    }

    /// Finds a device by name (case-insensitive) or fingerprint prefix.
    /// A query that fits two devices finds neither: a computer's name is its
    /// own to choose, and must not stand in for another's ID.
    pub fn find(&self, query: &str) -> Option<&Device> {
        let mut hits = self.matching(query);
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    }

    /// Every device `query` names (see [`TrustStore::find`]): more than one
    /// when two computers share a name.
    pub fn matching(&self, query: &str) -> impl Iterator<Item = &Device> + use<'_> {
        let (name, q) = (query.to_owned(), query.replace('-', "").to_ascii_lowercase());
        self.file.devices.iter().filter(move |d| {
            // By the name shown here (the one chosen here, else its own), or its id.
            d.label().eq_ignore_ascii_case(&name) || (q.len() >= 4 && d.fingerprint.starts_with(&q))
        })
    }

    /// Whether another paired computer is shown under the same name as `d`.
    pub fn label_is_shared(&self, d: &Device) -> bool {
        self.file.devices.iter().any(|o| o.fingerprint != d.fingerprint && o.label().trim().eq_ignore_ascii_case(d.label().trim()))
    }

    /// `d`'s name as people should see it: its name, with its short id
    /// after it when another paired computer is shown under the same name
    /// (`MacBook-Pro (ab12-cd34)`), so two of them can be told apart.
    pub fn display(&self, d: &Device) -> String {
        let label = one_mk_proto::sanitize_name(d.label());
        if self.label_is_shared(d) { format!("{label} ({})", d.short_id()) } else { label }
    }

    /// [`TrustStore::display`] for the computer with fingerprint `fp`
    /// (empty if it is not paired).
    pub fn display_fp(&self, fp: &Fingerprint) -> String {
        self.get(fp).map(|d| self.display(d)).unwrap_or_default()
    }

    /// What to type on the command line to name `d`: its name when that
    /// finds it and nothing else and is safe to type, else its short id.
    pub fn arg(&self, d: &Device) -> String {
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
        let label = d.label();
        let unique = self.find(label).is_some_and(|f| f.fingerprint == d.fingerprint);
        if unique && !label.is_empty() && label.chars().all(plain) {
            label.to_owned()
        } else if unique && !label.trim().is_empty() && label.trim() == label && label.chars().all(|c| plain(c) || c == ' ') {
            format!("\"{label}\"")
        } else {
            d.short_id()
        }
    }

    /// Why `query` names no single computer (or `None` when it names one):
    /// no one, or several (which it lists by id).
    pub fn explain_miss(&self, query: &str) -> Option<String> {
        let hits: Vec<&Device> = self.matching(query).collect();
        match hits.len() {
            1 => None,
            0 => Some(format!("no paired computer called {query:?}; see `one-mk devices` for names and ids")),
            n => Some(format!(
                "{n} paired computers are called {query:?}; use its id instead: {} (see `one-mk devices`)",
                hits.iter().map(|d| d.short_id()).collect::<Vec<_>>().join(" or ")
            )),
        }
    }

    /// By fingerprint (or its leading part, as the window shows it) only:
    /// a paired computer can't capture the match by naming itself after
    /// another one's id.
    pub fn find_fingerprint(&self, prefix: &str) -> Option<&Device> {
        let q = prefix.replace('-', "").to_ascii_lowercase();
        if q.len() < 4 || !q.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut hits = self.file.devices.iter().filter(|d| d.fingerprint.starts_with(&q));
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    }

    pub fn remove(&mut self, query: &str) -> Option<Device> {
        let fp = self.find(query)?.fingerprint.clone();
        let i = self.file.devices.iter().position(|d| d.fingerprint == fp)?;
        Some(self.file.devices.remove(i))
    }
}

/// Takes the exclusive lock next to `devices.toml` in `dir`: waits for it
/// for at most `wait`, or [`COMMAND_LOCK_WAIT`]. Released when the file is
/// dropped.
fn lock(dir: &Path, wait: Option<Duration>) -> Result<std::fs::File> {
    crate::files::lock_file(&dir.join(LOCK_FILE), wait.unwrap_or(COMMAND_LOCK_WAIT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_lookup_ignores_names() {
        let dir = std::env::temp_dir().join(format!("omk-trust-fp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = TrustStore::load(&dir).unwrap();
        let mk = |fp: &str, name: &str| Device { fingerprint: fp.into(), name: name.into(), ..Default::default() };
        store.file.devices = vec![mk("abcd1234ffff", "Studio"), mk("5555666677", "abcd-1234")];
        // The window sends a short id: the computer named like it can't take the match.
        assert_eq!(store.find_fingerprint("abcd-1234").map(|d| d.name.as_str()), Some("Studio"));
        assert!(store.find_fingerprint("Studio").is_none());
        assert!(store.find_fingerprint("ab").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two computers that call themselves the same are shown with their
    /// ids, named on the command line by id, and a name of your own (kept
    /// beside the one it gave itself) tells them apart.
    #[test]
    fn computers_with_the_same_name_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-twins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = TrustStore::load(&dir).unwrap();
        let (a, b, c) = ("aa11".repeat(16), "bb22".repeat(16), "cc33".repeat(16));
        let mk = |fp: &str, name: &str| Device { fingerprint: fp.into(), name: name.into(), ..Default::default() };
        store.file.devices = vec![mk(&a, "MacBook-Pro"), mk(&b, "macbook-pro"), mk(&c, "Studio")];
        let (da, dc) = (store.devices()[0].clone(), store.devices()[2].clone());
        assert_eq!(store.display(&da), format!("MacBook-Pro ({})", da.short_id()), "{:?}", store.display(&da));
        assert_eq!(store.display(&dc), "Studio", "a name that is unique stays plain");
        assert_eq!(store.arg(&da), da.short_id(), "a name that fits two is no way to name one");
        assert_eq!(store.arg(&dc), "Studio");
        let miss = store.explain_miss("macbook-pro").unwrap();
        assert!(miss.contains("2 paired computers are called") && miss.contains(&da.short_id()) && miss.contains(&store.devices()[1].short_id()), "{miss}");
        assert!(store.explain_miss("Studio").is_none());
        assert!(store.explain_miss("Nobody").unwrap().starts_with("no paired computer called"));
        assert_eq!(store.find(&da.short_id()).map(|d| d.fingerprint.clone()), Some(a.clone()), "an id always names one");

        // A name chosen here: shown instead, matched too, the real one kept, and it survives a save.
        store.get_mut(&a.parse().unwrap()).unwrap().alias = Some("Office PC".into());
        let da = store.devices()[0].clone();
        assert_eq!((da.label(), da.name.as_str()), ("Office PC", "MacBook-Pro"));
        assert_eq!(store.display(&da), "Office PC", "no longer reads like another computer");
        assert_eq!(store.display(&store.devices()[1]), "macbook-pro");
        assert_eq!(store.find("office pc").map(|d| d.fingerprint.clone()), Some(a.clone()));
        assert_eq!(store.arg(&da), "\"Office PC\"");
        store.save().unwrap();
        let saved = std::fs::read_to_string(dir.join("devices.toml")).unwrap();
        assert!(saved.contains("alias = \"Office PC\"") && saved.matches("alias").count() == 1, "{saved}");
        let mut again = TrustStore::load(&dir).unwrap();
        assert_eq!(again.devices()[0].alias.as_deref(), Some("Office PC"));
        // The computer's own name arriving again (pairing, reconnecting) leaves it alone.
        again.upsert(Device { name: "MacBook-Pro (renamed)".into(), fingerprint: a.clone(), ..Default::default() });
        assert_eq!(again.devices()[0].label(), "Office PC");
        assert_eq!(again.devices()[0].name, "MacBook-Pro (renamed)");
        // A file from before aliases has none.
        std::fs::write(dir.join("devices.toml"), format!("[[device]]\nname = 'old'\nfingerprint = '{a}'\n")).unwrap();
        let old = TrustStore::load(&dir).unwrap();
        assert_eq!((old.devices()[0].alias.clone(), old.devices()[0].label()), (None, "old"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn automatic_and_pending_placements_round_trip_without_becoming_user_choices() {
        for origin in [PlacedBy::Pairing, PlacedBy::Pending] {
            let device = Device { placed_by: origin, layout_version: 16, ..Device::default() };
            let saved = toml::to_string(&device).unwrap();
            let restored: Device = toml::from_str(&saved).unwrap();
            assert_eq!(restored.placed_by, origin);
            assert_eq!(restored.learnable(), origin == PlacedBy::Pairing);
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_devices_are_owner_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("one-mk-private-devices-{}-{}", std::process::id(), rand::random::<u64>()));
        TrustStore::load(&dir).unwrap().save().unwrap();
        assert_eq!(std::fs::metadata(dir.join("devices.toml")).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A hand-edited offset too large for the geometry is cut down on load;
    /// the `align` of builds before the placement model is ignored (it was
    /// never released), so that device has no offset.
    #[test]
    fn hand_edited_offsets_are_normalized_on_load() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-offset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fp = "ab".repeat(32);
        let text = format!("[[device]]\nname = 'peer'\nfingerprint = '{fp}'\nside = 'left'\noffset = [{}, 5]\n[[device]]\nname = 'old'\nfingerprint = '{}'\nalign = '300'\n", i32::MIN, "cd".repeat(32));
        std::fs::write(dir.join("devices.toml"), text).unwrap();
        let trust = TrustStore::load(&dir).unwrap();
        assert_eq!(trust.devices()[0].offset, Some((-MAX_OFFSET, 5)));
        assert_eq!(trust.devices()[1].offset, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Who placed a device: a file from before `placed_by` reads as the
    /// user's choice once a layout was chosen, pairing's guess before;
    /// the field is written only when it says more than that.
    #[test]
    fn placement_origin_and_what_learning_may_move() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-placed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = ("ab".repeat(32), "cd".repeat(32), "ef".repeat(32));
        let text = format!(
            "[[device]]\nname = 'guess'\nfingerprint = '{a}'\nside = 'right'\n\
             [[device]]\nname = 'chosen'\nfingerprint = '{b}'\nside = 'left'\nlayout_version = 1700000000000\n\
             [[device]]\nname = 'learned'\nfingerprint = '{c}'\nside = 'top'\nlayout_version = 5\nplaced_by = 'learned'\n"
        );
        std::fs::write(dir.join("devices.toml"), text).unwrap();
        let mut t = TrustStore::load(&dir).unwrap();
        let learnable: Vec<bool> = t.devices().iter().map(Device::learnable).collect();
        assert_eq!(learnable, [true, false, true]);
        let guess = t.get_mut(&a.parse().unwrap()).unwrap();
        guess.offset = Some((1920, 0));
        assert!(!guess.learnable(), "an exact place is never a guess");
        guess.offset = None;
        guess.placed_by = PlacedBy::Confirmed;
        assert!(!guess.learnable());
        t.save().unwrap();
        let saved = std::fs::read_to_string(dir.join("devices.toml")).unwrap();
        assert_eq!(saved.matches("placed_by").count(), 2, "{saved}");
        assert!(saved.contains("placed_by = \"confirmed\"") && saved.contains("placed_by = \"learned\""));
        assert!(TrustStore::load(&dir).is_ok());
        // A value this build doesn't know (a newer build's, a typo) is no
        // reason to lose the paired computers: it reads as not said.
        std::fs::write(dir.join("devices.toml"), format!("[[device]]\nname = 'x'\nfingerprint = '{a}'\nplaced_by = 'robot'\n")).unwrap();
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].placed_by, PlacedBy::Default);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Many writers at once (threads here; each takes its own lock, as
    /// separate processes do): every change lands, none is written over
    /// with a stale copy, and no temporary file is left behind.
    #[test]
    fn concurrent_updates_all_land() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-race-{}-{}", std::process::id(), rand::random::<u64>()));
        let fps: Vec<String> = (0..8u8).map(|i| format!("{i:02x}").repeat(32)).collect();
        // Each update waits for the others and flushes to disk, which is slow on
        // a Windows runner: fewer rounds there, the same race.
        const ROUNDS: u64 = if cfg!(windows) { 5 } else { 25 };
        TrustStore::update(&dir, |t| {
            for (i, fp) in fps.iter().enumerate() {
                t.upsert(Device { name: format!("pc{i}"), fingerprint: fp.clone(), ..Default::default() });
            }
            Ok(())
        })
        .unwrap();
        // The folder is flushed after the rename too.
        #[cfg(all(unix, debug_assertions))]
        assert!(crate::files::was_synced(&dir));
        std::thread::scope(|scope| {
            for fp in &fps {
                let dir = &dir;
                scope.spawn(move || {
                    for round in 1..=ROUNDS {
                        TrustStore::update(dir, |t| {
                            t.get_mut(&fp.parse().unwrap()).unwrap().layout_version = round;
                            Ok(())
                        })
                        .unwrap();
                    }
                });
            }
        });
        let t = TrustStore::load(&dir).unwrap();
        assert!(t.devices().iter().all(|d| d.layout_version == ROUNDS), "a change was lost");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        // Nothing changed: nothing is written.
        let before = std::fs::metadata(dir.join("devices.toml")).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        TrustStore::update(&dir, |_| Ok(())).unwrap();
        assert_eq!(std::fs::metadata(dir.join("devices.toml")).unwrap().modified().unwrap(), before);
        // A held lock: the daemon's bounded wait gives up and changes nothing.
        let held = lock(&dir, None).unwrap();
        let err = TrustStore::update_within(&dir, Duration::from_millis(50), |t| {
            t.remove("pc0");
            Ok(())
        })
        .unwrap_err();
        assert!(format!("{err:#}").contains("another one-mk command is holding the lock"), "{err:#}");
        drop(held);
        assert_eq!(TrustStore::load(&dir).unwrap().devices().len(), 8);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A name that is also another computer's ID never picks either.
    #[test]
    fn an_ambiguous_query_finds_nothing() {
        let dev = |name: &str, fp: &str| Device { name: name.into(), fingerprint: fp.into(), ..Default::default() };
        let dir = std::env::temp_dir().join(format!("one-mk-trust-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = TrustStore::load(&dir).unwrap();
        t.upsert(dev("abcd1234", &"ee".repeat(32)));
        t.upsert(dev("victim", &format!("abcd1234{}", "00".repeat(28))));
        assert!(t.find("abcd1234").is_none());
        assert_eq!(t.find("victim").unwrap().name, "victim");
        assert_eq!(t.find("eeeeeeee").unwrap().name, "abcd1234");
    }

    /// A `devices.toml` from before layout versions and Ctrl/Cmd settings
    /// still loads, with defaults.
    #[test]
    fn old_files_still_load() {
        let dir = std::env::temp_dir().join(format!("one-mk-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fp = "ab".repeat(32);
        std::fs::write(dir.join("devices.toml"), format!("[[device]]\nname = \"old\"\nfingerprint = \"{fp}\"\nside = \"left\"\n")).unwrap();
        let mut t = TrustStore::load(&dir).unwrap();
        let d = &t.devices()[0];
        assert_eq!((d.side(), d.layout_version, d.swap_ctrl_cmd, d.os), (Side::Left, 0, Swap::Auto, 0));
        assert_eq!(d.offset, None);
        t.get_mut(&fp.parse().unwrap()).unwrap().swap_ctrl_cmd = Swap::Off;
        t.save().unwrap();
        assert!(!std::fs::read_to_string(dir.join("devices.toml")).unwrap().contains("offset"), "the default isn't written");
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].swap_ctrl_cmd, Swap::Off);
        assert_eq!(t.devices()[0].offset, None);
        t.get_mut(&fp.parse().unwrap()).unwrap().offset = Some((-3000, -900));
        t.save().unwrap();
        assert_eq!(TrustStore::load(&dir).unwrap().devices()[0].offset, Some((-3000, -900)));
        // Pairing again keeps the layout.
        let mut t = TrustStore::load(&dir).unwrap();
        t.get_mut(&fp.parse().unwrap()).unwrap().placed_by = PlacedBy::Learned;
        t.upsert(Device { name: "old".into(), fingerprint: fp.clone(), ..Device::default() });
        assert_eq!(t.devices()[0].offset, Some((-3000, -900)));
        assert_eq!(t.devices()[0].placed_by, PlacedBy::Learned);
        assert_eq!("ON".parse::<Swap>(), Ok(Swap::On));
        assert!("maybe".parse::<Swap>().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
