use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
pub struct Media { pub width: u32, pub height: u32, pub duration_ms: u64 }

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
}

impl Plan {
    pub fn playing_message(&self) -> String {
        let monitors: Vec<_> = self.canvas.monitors.iter().filter(|monitor| monitor.online && self.computers.contains(&monitor.computer)).collect();
        let computers = monitors.iter().map(|monitor| &monitor.computer).collect::<BTreeSet<_>>().len();
        format!("Playing on {} {} on {computers} {}", monitors.len(), if monitors.len() == 1 { "monitor" } else { "monitors" }, if computers == 1 { "computer" } else { "computers" })
    }

    pub fn valid(&self) -> bool {
        valid_hash(&self.hash) && self.title.len() <= 256 && self.canvas.valid()
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
    Off,
    Clock { sent_ms: i64 },
    ClockReply { sent_ms: i64, received_ms: i64, replied_ms: i64 },
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
    if drift.abs() > 500 { Correction::Seek(expected) }
    else if drift.abs() > 40 { Correction::Rate(if drift > 0 { 1.02 } else { 0.98 }) }
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
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "video" || extension == "png")
            && path.file_stem().and_then(|name| name.to_str()).is_some_and(|hash| valid_hash(hash) && hash != current && Some(hash) != previous)
        { std::fs::remove_file(path)?; }
    }
    Ok(())
}

pub fn availability() -> Result<(), &'static str> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Err("Video wallpapers are not available on Wayland yet. GNOME needs a shell extension; KDE and other Wayland desktops are not supported yet.");
        }
        if std::env::var_os("DISPLAY").is_none() { return Err("Video wallpapers need an X11 desktop."); }
        if unsafe { libloading::Library::new("libgstreamer-1.0.so.0") }.is_err() {
            return Err("Install GStreamer to play video wallpapers.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(computer: &str, x: f64, scale: f64) -> Monitor {
        Monitor { computer: computer.into(), name: "0".into(), rect: Rect { x, y: 0.0, width: 1920.0, height: 1080.0 }, local: None, scale, online: true }
    }
    #[test]
    fn spanning_mixed_density_and_offline_reservations() {
        let mut canvas = Canvas { monitors: vec![monitor("mac", -1920.0, 2.0), monitor("pc", 0.0, 1.0)] };
        let media = Media { width: 3840, height: 1080, duration_ms: 1000 };
        let left = canvas.slice(&canvas.monitors[0], &media, Fit::Fill).unwrap();
        let right = canvas.slice(&canvas.monitors[1], &media, Fit::Fill).unwrap();
        assert_eq!(left.x + left.width, right.x);
        assert_eq!(left.width, 0.5);
        let bounds = canvas.bounds();
        canvas.update(vec![monitor("mac", -1920.0, 2.0)]);
        assert_eq!(canvas.bounds(), bounds);
        assert!(!canvas.monitors[1].online);
        let plan = Plan { hash: "a".repeat(64), title: "Video".into(), media, canvas, start_ms: 1, fit: Fit::Fill,
            computers: vec!["mac".into(), "pc".into()], pause_on_battery: true, paused: false };
        assert_eq!(plan.playing_message(), "Playing on 1 monitor on 1 computer");
    }
    #[test]
    fn fit_modes_keep_one_canvas_transform() {
        let canvas = Canvas { monitors: vec![monitor("mac", 0.0, 2.0), monitor("mac", 1920.0, 1.0)] };
        let media = Media { width: 1920, height: 1080, duration_ms: 1000 };
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
        assert_eq!(correction(1000, 1501, 10000), Correction::Seek(1501));
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
}
