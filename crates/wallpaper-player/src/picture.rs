use crate::{Media, Plan};
use anyhow::{Context, Result, ensure};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const MAX_OUTPUT: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode { #[default] Native, Overlay }

impl Mode {
    pub fn for_native_safety(restorable: bool) -> Self { if restorable { Self::Native } else { Self::Overlay } }
}

/// Said when a Linux desktop keeps its wallpaper somewhere oneMK can't set.
pub const UNSUPPORTED_DESKTOP: &str = "Picture wallpapers on Linux need GNOME (or Ubuntu's or Budgie's desktop, which use GNOME's wallpaper settings), or an experimental layer-shell desktop such as Sway. This desktop keeps its wallpaper elsewhere, so oneMK leaves it unchanged. Set the picture in this desktop's own settings instead.";

/// Whether this Linux desktop draws its wallpaper from GNOME's settings
/// (`org.gnome.desktop.background`): GNOME itself, and the desktops built
/// on it that name it in `XDG_CURRENT_DESKTOP` (Ubuntu, Budgie) or use it
/// unnamed (Unity, Pantheon). KDE, Xfce, Cinnamon, MATE, LXQt and the rest
/// don't, whatever schemas happen to be installed.
pub fn gnome_desktop() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| gnome_settings_desktop(&desktop))
}

pub fn gnome_settings_desktop(desktop: &str) -> bool {
    desktop.split(':').any(|part| matches!(part.trim().to_ascii_lowercase().as_str(), "gnome" | "gnome-classic" | "gnome-flashback" | "unity" | "pantheon" | "budgie"))
}

pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    // GNOME first: it needs no question to the compositor (on GNOME that
    // question is a D-Bus round trip, and the window asks several times a second).
    return gnome_desktop() || (std::env::var_os("WAYLAND_DISPLAY").is_some() && crate::wayland::available().is_ok());
    #[cfg(not(target_os = "linux"))]
    true
}

pub fn is_picture(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "heic"))
}

struct Job(Child);
impl Drop for Job {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

fn capture(stream: impl Read + Send + 'static) -> Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stream.take(MAX_OUTPUT + 1).read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    receiver
}

fn run(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut job = Job(command.spawn().context("The picture wallpaper helper could not start")?);
    let stdout = capture(job.0.stdout.take().context("Missing picture helper output")?);
    let stderr = capture(job.0.stderr.take().context("Missing picture helper errors")?);
    let started = Instant::now();
    let mut output = None;
    let mut errors = None;
    loop {
        for (receiver, result) in [(&stdout, &mut output), (&stderr, &mut errors)] {
            if result.is_none() && let Ok(bytes) = receiver.try_recv() {
                let bytes = bytes?;
                ensure!(bytes.len() as u64 <= MAX_OUTPUT, "The picture helper returned too much output");
                *result = Some(bytes);
            }
        }
        if let Some(status) = job.0.try_wait()? && let (Some(output), Some(errors)) = (&output, &errors) {
            ensure!(status.success(), "{}", helper_error(&String::from_utf8_lossy(errors)));
            return Ok(output.clone());
        }
        ensure!(started.elapsed() < timeout, "The picture helper took too long");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// What a failed helper said, as one sentence for a person: its own
/// message, without Rust's `Error:` prefix or the `Caused by:` chain of
/// internals (decoder names, cache file names) under it.
fn helper_error(errors: &str) -> String {
    let message = errors.trim().strip_prefix("Error:").unwrap_or(errors).split("\n\nCaused by:").next().unwrap_or_default().trim();
    if message.is_empty() { "The picture helper failed without saying why. Try again, or choose another picture.".into() } else { message.into() }
}

fn plan_arguments(command: &mut Command, plan: &Plan, computer: &str) -> Result<()> {
    ensure!(plan.valid() && plan.picture, "Invalid picture wallpaper plan");
    ensure!(!computer.is_empty() && computer.len() <= 128, "Invalid picture computer");
    let json = serde_json::to_string(plan)?;
    ensure!(json.len() <= 12000, "The picture layout has too many monitors");
    command.arg(json).arg(computer);
    Ok(())
}

pub fn inspect(helper: &Path, path: &Path, poster: &Path) -> Result<Media> {
    let output = run(Command::new(helper).arg("--picture-inspect").arg(path).arg(poster), Duration::from_secs(40))?;
    let media: Media = serde_json::from_slice(&output).context("Invalid picture helper dimensions")?;
    ensure!(media.width > 0 && media.height > 0 && media.width <= 16384 && media.height <= 16384
        && u64::from(media.width) * u64::from(media.height) <= 64 * 1024 * 1024
        && media.duration_ms == 1 && media.frame_rate == 0, "Invalid picture helper dimensions");
    Ok(media)
}

/// Writes the first frame of `video` as a PNG about `width` pixels wide and
/// returns its exact dimensions (Linux helper; GStreamer decodes it).
pub fn still(helper: &Path, video: &Path, output: &Path, width: u32) -> Result<Media> {
    ensure!(run(Command::new(helper).arg("--still").arg(video).arg(output).arg(width.to_string()), Duration::from_secs(40))?.is_empty(), "Unexpected still helper output");
    let mut header = [0_u8; 24];
    std::fs::File::open(output)?.read_exact(&mut header).context("The first-frame still is not a picture")?;
    ensure!(&header[..8] == b"\x89PNG\r\n\x1a\n" && &header[12..16] == b"IHDR", "The first-frame still is not a PNG");
    let (width, height) = (u32::from_be_bytes(header[16..20].try_into()?), u32::from_be_bytes(header[20..24].try_into()?));
    ensure!(width > 0 && height > 0 && width <= 16384 && height <= 16384, "Invalid first-frame still size");
    Ok(Media { width, height, duration_ms: 1, frame_rate: 0 })
}

pub fn prepare(helper: &Path, source: &Path, plan: &Plan, computer: &str, path: &Path) -> Result<()> {
    let mut command = Command::new(helper);
    command.arg("--picture-prepare").arg(source).arg(path);
    plan_arguments(&mut command, plan, computer)?;
    ensure!(run(&mut command, Duration::from_secs(180))?.is_empty(), "Unexpected picture helper output");
    Ok(())
}

pub fn apply(helper: &Path, plan: &Plan, computer: &str, path: &Path, root: &Path) -> Result<Mode> {
    let mut command = Command::new(helper);
    command.arg("--picture-apply").arg(path).arg(root);
    plan_arguments(&mut command, plan, computer)?;
    let output = run(&mut command, Duration::from_secs(180))?;
    // Older installed helpers only set native pictures and return no output.
    if output.is_empty() { return Ok(Mode::Native); }
    serde_json::from_slice(&output).context("Invalid picture helper mode")
}

pub fn restore(helper: &Path, root: &Path) -> Result<()> {
    if !root.join("previous.json").exists() { return Ok(()); }
    ensure!(run(Command::new(helper).arg("--picture-restore").arg(root), Duration::from_secs(40))?.is_empty(), "Unexpected picture helper output");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn first_frame_still_reports_the_exact_png_size_and_rejects_anything_else() {
        let dir = std::env::temp_dir().join(format!("one-mk-still-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Use an existing executable: executing a freshly written script can
        // race a concurrent fork that briefly inherits its writable descriptor
        // on Linux (ETXTBSY). This test checks the helper's output file.
        let helper = Path::new("true");
        let output = dir.join("frame.png");
        std::fs::write(&output, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x07\0\0\0\x03").unwrap();
        let media = still(helper, Path::new("video"), &output, 1).unwrap();
        assert_eq!((media.width, media.height, media.duration_ms), (7, 3, 1));
        std::fs::write(&output, b"nope").unwrap();
        let error = still(helper, Path::new("video"), &output, 2).unwrap_err();
        assert!(error.to_string().contains("first-frame still is not a picture"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn helper_errors_read_as_one_sentence_without_internals() {
        let leaked = "Error: This picture cannot be opened. Try a PNG, JPEG or WebP picture.\n\nCaused by:\n    The file extension `.\"video\"` was not recognized as an image format\n";
        assert_eq!(helper_error(leaked), "This picture cannot be opened. Try a PNG, JPEG or WebP picture.");
        assert_eq!(helper_error("plain words\n"), "plain words");
        assert!(helper_error("  \n").contains("without saying why"));
        let error = run(Command::new("sh").args(["-c", &format!("printf '{}' >&2; exit 1", leaked.replace('\n', "\\n").replace('`', "\\`"))]), Duration::from_secs(2)).unwrap_err();
        assert_eq!(error.to_string(), "This picture cannot be opened. Try a PNG, JPEG or WebP picture.");
    }

    #[test]
    fn only_desktops_that_read_gnome_wallpaper_settings_get_native_pictures() {
        for desktop in ["GNOME", "ubuntu:GNOME", "GNOME-Classic:GNOME", "Budgie:GNOME", "Unity", "Pantheon", "GNOME-Flashback:GNOME"] {
            assert!(gnome_settings_desktop(desktop), "{desktop}");
        }
        for desktop in ["KDE", "XFCE", "X-Cinnamon", "MATE", "LXQt", "sway", "Hyprland", "COSMIC", "", "notgnome"] {
            assert!(!gnome_settings_desktop(desktop), "{desktop}");
        }
    }

    #[test]
    fn helper_timeout_and_output_limits_leave_the_caller_alive() {
        let error = run(Command::new("sh").args(["-c", "exec sleep 30"]), Duration::from_millis(30)).unwrap_err();
        assert!(error.to_string().contains("too long"));
        for stream in ["", " >&2"] {
            let error = run(Command::new("sh").args(["-c", &format!("exec head -c 20000 /dev/zero{stream}")]), Duration::from_secs(2)).unwrap_err();
            assert!(error.to_string().contains("too much output"));
        }
        assert_eq!(run(Command::new("sh").args(["-c", "printf ok"]), Duration::from_secs(2)).unwrap(), b"ok");
    }
}
