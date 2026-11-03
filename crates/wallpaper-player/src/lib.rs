use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub mod picture;
#[cfg(target_os = "linux")]
pub mod wayland;

pub const MAX_MEDIA_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit { #[default] Fill, Fit, Stretch }

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }

impl Rect {
    pub fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height].into_iter().all(f64::is_finite)
            && self.width > 0.0 && self.height > 0.0
            && self.x.abs() <= 1_000_000.0 && self.y.abs() <= 1_000_000.0
            && self.width <= 1_000_000.0 && self.height <= 1_000_000.0
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self { x, y, width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Monitor {
    pub computer: String,
    pub name: String,
    pub rect: Rect,
    #[serde(default)]
    pub local: Option<Rect>,
    pub scale: f64,
    pub online: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Canvas { pub monitors: Vec<Monitor> }

impl Canvas {
    pub fn bounds(&self) -> Option<Rect> { self.monitors.iter().map(|monitor| monitor.rect).reduce(Rect::union) }
    pub fn valid(&self) -> bool {
        !self.monitors.is_empty() && self.monitors.len() <= 64 && self.monitors.iter().all(|monitor|
            monitor.rect.valid() && monitor.local.is_none_or(Rect::valid) && monitor.scale.is_finite() && (0.5..=8.0).contains(&monitor.scale)
            && monitor.rect.width * monitor.scale <= 32768.0 && monitor.rect.height * monitor.scale <= 32768.0
            && !monitor.computer.is_empty() && monitor.computer.len() <= 128 && monitor.name.len() <= 256)
    }
    pub fn image_rect(&self, media: &Media, fit: Fit) -> Option<Rect> {
        let canvas = self.bounds()?;
        let source_width = f64::from(media.width.max(1));
        let source_height = f64::from(media.height.max(1));
        if fit == Fit::Stretch { return Some(canvas); }
        let horizontal = canvas.width / source_width;
        let vertical = canvas.height / source_height;
        let scale = if fit == Fit::Fill { horizontal.max(vertical) } else { horizontal.min(vertical) };
        let width = source_width * scale;
        let height = source_height * scale;
        Some(Rect { x: canvas.x + (canvas.width - width) / 2.0,
            y: canvas.y + (canvas.height - height) / 2.0, width, height })
    }
    pub fn slice(&self, monitor: &Monitor, media: &Media, fit: Fit) -> Option<Rect> {
        let image = self.image_rect(media, fit)?;
        Some(Rect { x: (monitor.rect.x - image.x) / image.width,
            y: (monitor.rect.y - image.y) / image.height,
            width: monitor.rect.width / image.width, height: monitor.rect.height / image.height })
    }
    pub fn update(&mut self, monitors: Vec<Monitor>) {
        for saved in &mut self.monitors { saved.online = false; }
        for monitor in monitors {
            if let Some(saved) = self.monitors.iter_mut().find(|saved| saved.computer == monitor.computer && saved.name.rsplit(" · ").next() == monitor.name.rsplit(" · ").next()) {
                *saved = monitor;
            } else { self.monitors.push(monitor); }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    pub width: u32, pub height: u32, pub duration_ms: u64,
    #[serde(default = "default_frame_rate")]
    pub frame_rate: u32,
}
fn default_frame_rate() -> u32 { 30 }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ImportTarget { pub width: u32, pub height: u32, pub bitrate: u32 }
impl ImportTarget {
    pub fn for_canvas(canvas: &Canvas, media: &Media, bytes: u64) -> Option<Self> {
        if !canvas.valid() || media.width < 2 || media.height < 2 || media.duration_ms == 0 { return None; }
        let mut computers = std::collections::BTreeMap::<&str, (Rect, f64)>::new();
        for monitor in &canvas.monitors {
            computers.entry(&monitor.computer).and_modify(|(bounds, scale)| { *bounds = bounds.union(monitor.rect); *scale = scale.max(monitor.scale); }).or_insert((monitor.rect, monitor.scale));
        }
        let width = computers.values().map(|(rect, scale)| rect.width * scale).fold(2.0, f64::max);
        let height = computers.values().map(|(rect, scale)| rect.height * scale).fold(2.0, f64::max);
        let scale = (width / f64::from(media.width)).min(height / f64::from(media.height)).min(1.0);
        let width = ((f64::from(media.width) * scale) as u32 / 2 * 2).max(2);
        let height = ((f64::from(media.height) * scale) as u32 / 2 * 2).max(2);
        let bitrate = (f64::from(width) * f64::from(height) * f64::from(media.frame_rate.clamp(1, 60)) * 0.045).clamp(128_000.0, 100_000_000.0) as u32;
        let source_bitrate = bytes as f64 * 8000.0 / media.duration_ms.max(1) as f64;
        (scale < 1.0 || source_bitrate > f64::from(bitrate) * 1.5 || media.frame_rate > 60).then_some(Self { width, height, bitrate })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub hash: String,
    pub title: String,
    pub media: Media,
    pub canvas: Canvas,
    pub start_ms: i64,
    pub fit: Fit,
    pub computers: Vec<String>,
    pub pause_on_battery: bool,
    pub paused: bool,
    #[serde(default)]
    pub picture: bool,
    #[serde(default)]
    pub asset_hash: Option<String>,
}

impl Plan {
    pub fn playing_message(&self) -> String { self.playing_summary(&std::collections::BTreeMap::new()) }

    /// What is playing, leaving out computers that said they can't play it
    /// (a Wayland desktop, a decoder missing), and saying how many those are.
    pub fn playing_summary(&self, playback: &std::collections::BTreeMap<String, Playback>) -> String {
        let failed = |computer: &String| matches!(playback.get(computer), Some(Playback::Error(_)));
        let monitors: Vec<_> = self.canvas.monitors.iter().filter(|monitor| monitor.online && self.computers.contains(&monitor.computer) && !failed(&monitor.computer)).collect();
        let computers = monitors.iter().map(|monitor| &monitor.computer).collect::<BTreeSet<_>>().len();
        let not = self.computers.iter().filter(|computer| failed(computer)).count();
        let mut text = format!("Playing on {} {} on {computers} {}", monitors.len(), if monitors.len() == 1 { "monitor" } else { "monitors" }, if computers == 1 { "computer" } else { "computers" });
        if not > 0 { text += &format!(" · {not} {} can't play it", if not == 1 { "computer" } else { "computers" }); }
        text
    }

    pub fn valid(&self) -> bool {
        valid_hash(&self.hash) && self.asset_hash.as_ref().is_none_or(|hash| valid_hash(hash)) && self.title.len() <= 256 && self.canvas.valid()
            && self.media.width > 0 && self.media.width <= 16384 && self.media.height > 0 && self.media.height <= 16384
            && (1..=7 * 24 * 3600 * 1000).contains(&self.media.duration_ms)
            && self.computers.len() <= 16 && self.start_ms > 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub plan: Option<Plan>,
    pub message: String,
    pub available: bool,
    pub poster: Option<PathBuf>,
    #[serde(default)]
    pub computers: std::collections::BTreeMap<String, Playback>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum Playback { #[default] Preparing, Ready, Picture, Paused(String), Error(String) }

impl Playback {
    pub fn message(&self) -> &str { match self { Self::Preparing => "preparing…", Self::Ready => "playing", Self::Picture => "Shown by oneMK; your own wallpaper returns when you turn it off", Self::Paused(message) | Self::Error(message) => message } }
    pub fn is_ready(&self) -> bool { matches!(self, Self::Ready | Self::Picture) }
}

pub fn event(playback: Playback) {
    use std::io::Write;
    let mut output = std::io::stdout().lock();
    if let Ok(json) = serde_json::to_string(&playback) { let _ = writeln!(output, "{json}"); let _ = output.flush(); }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    Set { path: PathBuf },
    Off,
    Pause { paused: bool },
    Fit { fit: Fit },
    Battery { enabled: bool },
    Computer { id: String, enabled: bool },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Wire {
    Plan { plan: Plan },
    PlanReady { hash: String },
    Off,
    Clock { sent_ms: i64 },
    ClockReply { sent_ms: i64, received_ms: i64, replied_ms: i64 },
    Playback { hash: String, playback: Playback },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerConfig {
    pub plan: Plan,
    pub computer: String,
    pub path: PathBuf,
    pub clock_offset_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Correction { None, Rate(f64), Seek(u64) }

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64
}

pub fn clock_offset(sent: i64, received: i64, replied: i64, arrived: i64) -> Option<(i64, i64)> {
    let elapsed = arrived.checked_sub(sent)?;
    let processing = replied.checked_sub(received)?;
    let rtt = elapsed.checked_sub(processing)?;
    if !(0..=2000).contains(&rtt) || processing < 0 { return None; }
    Some((received.checked_sub(sent)?.checked_add(replied.checked_sub(arrived)?)? / 2, rtt))
}

pub fn position(now: i64, offset: i64, start: i64, duration: u64) -> u64 {
    if duration == 0 { return 0; }
    (i128::from(now) + i128::from(offset) - i128::from(start)).rem_euclid(i128::from(duration)) as u64
}

pub fn correction(actual: u64, expected: u64, duration: u64) -> Correction {
    if duration == 0 { return Correction::None; }
    let length = i128::from(duration);
    let drift = (i128::from(expected) - i128::from(actual) + length / 2).rem_euclid(length) - length / 2;
    if drift.abs() > 40 { Correction::Rate(if drift > 0 { 1.02 } else { 0.98 }) }
    else { Correction::None }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hold { User, Locked, Asleep, Battery, LowPower, Occluded }

#[derive(Default, Debug)]
pub struct Holds(BTreeSet<Hold>);
impl Holds {
    pub fn set(&mut self, reason: Hold, held: bool) { if held { self.0.insert(reason); } else { self.0.remove(&reason); } }
    pub fn playing(&self) -> bool { self.0.is_empty() }
    pub fn message(&self) -> &'static str {
        match self.0.first() {
            None => "Playing",
            Some(Hold::User) => "Paused",
            Some(Hold::Locked) => "Paused while the screen is locked",
            Some(Hold::Asleep) => "Paused while the screen is asleep",
            Some(Hold::Battery) => "Paused on battery",
            Some(Hold::LowPower) => "Paused in Low Power Mode",
            Some(Hold::Occluded) => "Paused while the desktop is covered",
        }
    }
}

pub fn report(config: &PlayerConfig, holds: &Holds) {
    if !holds.playing() { event(Playback::Paused(holds.message().into())); }
    if let Some(directory) = config.path.parent() {
        let temporary = directory.join("playback.tmp");
        if std::fs::write(&temporary, holds.message()).is_ok() { let _ = std::fs::rename(temporary, directory.join("playback.txt")); }
    }
}

pub fn valid_hash(hash: &str) -> bool { hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) }

pub fn cache_path(root: &Path, hash: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(valid_hash(hash), "Invalid wallpaper fingerprint");
    Ok(root.join(format!("{hash}.video")))
}

pub fn prune_cache(root: &Path, current: &str, previous: Option<&str>) -> std::io::Result<()> {
    prune_cache_except(root, &std::iter::once(current.to_owned()).chain(previous.map(str::to_owned)).collect())
}

/// Where the player opens cached videos from on macOS and Windows: hard
/// links named with each video's real type (their players go by the
/// extension), beside the cache so its size limits don't count them twice.
pub fn links_dir(root: &Path) -> PathBuf { root.with_file_name("wallpaper-links") }

pub fn prune_cache_except(root: &Path, keep: &BTreeSet<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "video" || extension == "png")
            && path.file_stem().and_then(|name| name.to_str()).is_some_and(|hash| valid_hash(hash) && !keep.contains(hash))
        { std::fs::remove_file(path)?; }
    }
    // A link keeps its video's bytes on disk: it goes with the video.
    if let Ok(entries) = std::fs::read_dir(links_dir(root)) {
        for entry in entries.flatten() {
            let path = entry.path();
            // `<hash>.<type>`, or `.<hash>-….part` while a player makes it.
            let name = entry.file_name().to_string_lossy().into_owned();
            let hash = name.strip_prefix('.').map_or_else(|| path.file_stem().and_then(|name| name.to_str()).unwrap_or(""), |rest| rest.get(..64).unwrap_or(""));
            if !keep.contains(hash) { let _ = std::fs::remove_file(path); }
        }
    }
    Ok(())
}

pub fn availability() -> Result<(), &'static str> {
    // A Wayland session's compositor answers for itself, and the window
    // asks several times a second: asked again only every few seconds.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        static RECENT: std::sync::Mutex<Option<(std::time::Instant, Result<(), &'static str>)>> = std::sync::Mutex::new(None);
        let mut recent = RECENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, result)) = *recent && at.elapsed() < std::time::Duration::from_secs(5) { return result; }
        let result = detect_availability();
        *recent = Some((std::time::Instant::now(), result));
        return result;
    }
    static AVAILABLE: std::sync::OnceLock<Result<(), &'static str>> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(detect_availability)
}

fn detect_availability() -> Result<(), &'static str> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            wayland::available()?;
        }
        else if std::env::var_os("DISPLAY").is_none() { return Err("Video wallpapers need an X11 desktop."); }
        if unsafe { libloading::Library::new("libgstreamer-1.0.so.0") }.is_err() {
            return Err("Install GStreamer to play video wallpapers.");
        }
    }
    Ok(())
}

pub fn has_video_frame(result: i32) -> bool { result == 0 }

pub fn file_identity(file: &std::fs::File) -> anyhow::Result<Vec<u64>> {
    let metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(vec![metadata.len(), metadata.dev(), metadata.ino(), metadata.mtime() as u64, metadata.mtime_nsec() as u64, metadata.ctime() as u64, metadata.ctime_nsec() as u64])
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle}};
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information)?; }
        Ok(vec![metadata.len(), u64::from(information.dwVolumeSerialNumber), u64::from(information.nFileIndexHigh) << 32 | u64::from(information.nFileIndexLow),
            u64::from(information.ftLastWriteTime.dwHighDateTime) << 32 | u64::from(information.ftLastWriteTime.dwLowDateTime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_s_ok_contains_a_video_frame() {
        assert!(has_video_frame(0));
        assert!(!has_video_frame(1));
        assert!(!has_video_frame(i32::MIN));
    }
    fn monitor(computer: &str, x: f64, scale: f64) -> Monitor {
        Monitor { computer: computer.into(), name: "0".into(), rect: Rect { x, y: 0.0, width: 1920.0, height: 1080.0 }, local: None, scale, online: true }
    }
    #[test]
    fn spanning_mixed_density_and_offline_reservations() {
        let mut canvas = Canvas { monitors: vec![monitor("mac", -1920.0, 2.0), monitor("pc", 0.0, 1.0)] };
        let media = Media { width: 3840, height: 1080, duration_ms: 1000, frame_rate: 30 };
        let left = canvas.slice(&canvas.monitors[0], &media, Fit::Fill).unwrap();
        let right = canvas.slice(&canvas.monitors[1], &media, Fit::Fill).unwrap();
        assert_eq!(left.x + left.width, right.x);
        assert_eq!(left.width, 0.5);
        let bounds = canvas.bounds();
        canvas.update(vec![monitor("mac", -1920.0, 2.0)]);
        assert_eq!(canvas.bounds(), bounds);
        assert!(!canvas.monitors[1].online);
        let plan = Plan { hash: "a".repeat(64), title: "Video".into(), media, canvas, start_ms: 1, fit: Fit::Fill,
            computers: vec!["mac".into(), "pc".into()], pause_on_battery: true, paused: false, picture: false, asset_hash: None };
        assert_eq!(plan.playing_message(), "Playing on 1 monitor on 1 computer");
        // A computer that said it can't play it isn't counted as playing.
        let mut plan = plan;
        for monitor in &mut plan.canvas.monitors { monitor.online = true; }
        assert_eq!(plan.playing_message(), "Playing on 2 monitors on 2 computers");
        let said = std::collections::BTreeMap::from([("pc".to_owned(), Playback::Error("Video wallpapers are not available on Wayland yet".into())), ("mac".to_owned(), Playback::Ready)]);
        assert_eq!(plan.playing_summary(&said), "Playing on 1 monitor on 1 computer · 1 computer can't play it");
    }
    #[test]
    fn fit_modes_keep_one_canvas_transform() {
        let canvas = Canvas { monitors: vec![monitor("mac", 0.0, 2.0), monitor("mac", 1920.0, 1.0)] };
        let media = Media { width: 1920, height: 1080, duration_ms: 1000, frame_rate: 30 };
        let fill = canvas.image_rect(&media, Fit::Fill).unwrap();
        let fit = canvas.image_rect(&media, Fit::Fit).unwrap();
        assert_eq!((fill.width, fill.height, fill.y), (3840.0, 2160.0, -540.0));
        assert_eq!((fit.width, fit.height, fit.x), (1920.0, 1080.0, 960.0));
        assert_eq!(canvas.image_rect(&media, Fit::Stretch), canvas.bounds());
    }
    #[test]
    fn midpoint_offset_wrap_and_non_seeking_sync() {
        assert_eq!(clock_offset(1000, 1110, 1112, 1022), Some((100, 20)));
        assert_eq!(clock_offset(1000, 1110, 1120, 1005), None);
        assert_eq!(clock_offset(i64::MIN, i64::MAX, i64::MAX, i64::MIN), None);
        assert_eq!(position(999, 0, 1000, 1000), 999);
        assert_eq!(correction(9980, 20, 10000), Correction::None);
        assert_eq!(correction(1000, 1041, 10000), Correction::Rate(1.02));
        assert_eq!(correction(1000, 1499, 10000), Correction::Rate(1.02));
        assert_eq!(correction(1000, 1500, 10000), Correction::Rate(1.02));
        assert_eq!(correction(1000, 1501, 10000), Correction::Rate(1.02));
        assert_eq!(correction(1000, 950, 10000), Correction::Rate(0.98));
        assert_eq!(position(25005, 100, 10000, 5000), 105);
    }
    #[test]
    fn independent_holds_cannot_resume_each_other() {
        let mut holds = Holds::default();
        holds.set(Hold::User, true);
        holds.set(Hold::Locked, true);
        holds.set(Hold::Locked, false);
        assert!(!holds.playing());
        holds.set(Hold::User, false);
        assert!(holds.playing());
    }
    #[test]
    fn hash_paths_never_escape_cache() {
        assert!(cache_path(Path::new("/tmp/cache"), "../video").is_err());
        assert!(cache_path(Path::new("/tmp/cache"), &"a".repeat(64)).unwrap().starts_with("/tmp/cache"));
    }
    #[test]
    fn import_only_reduces_oversized_or_expensive_video() {
        let canvas = Canvas { monitors: vec![monitor("mac", 0.0, 1.0)] };
        let mut media = Media { width: 1920, height: 1080, duration_ms: 1000, frame_rate: 30 };
        assert!(ImportTarget::for_canvas(&canvas, &media, 100_000).is_none());
        let target = ImportTarget::for_canvas(&canvas, &media, 1_000_000).unwrap();
        assert_eq!((target.width, target.height, target.bitrate), (1920, 1080, 2_799_360));
        media.width = 3840; media.height = 2160;
        let target = ImportTarget::for_canvas(&canvas, &media, 100_000).unwrap();
        assert_eq!((target.width, target.height), (1920, 1080));
        media.width = 640; media.height = 360; media.frame_rate = 120;
        let target = ImportTarget::for_canvas(&canvas, &media, 100_000).unwrap();
        assert_eq!((target.width, target.height), (640, 360));
    }
    #[test]
    fn mixed_density_import_need_is_translation_invariant() {
        let mut canvas = Canvas { monitors: vec![monitor("mac", 0.0, 2.0), monitor("mac", 1920.0, 1.0)] };
        let media = Media { width: 7680, height: 2160, duration_ms: 1000, frame_rate: 30 };
        assert!(ImportTarget::for_canvas(&canvas, &media, 100_000).is_none());
        for monitor in &mut canvas.monitors { monitor.rect.x -= 9000.0; monitor.rect.y += 4000.0; }
        assert!(ImportTarget::for_canvas(&canvas, &media, 100_000).is_none());
    }
    #[test]
    fn old_peer_status_and_plans_keep_working() {
        let status: Status = serde_json::from_str(r#"{"plan":null,"message":"Playing","poster":null,"available":true}"#).unwrap();
        assert!(status.computers.is_empty());
        let media: Media = serde_json::from_str(r#"{"width":1920,"height":1080,"duration_ms":1000}"#).unwrap();
        assert_eq!(media.frame_rate, 30);
        assert_eq!(Playback::Ready.message(), "playing");
        assert_eq!(Playback::Preparing.message(), "preparing…");
    }
}
