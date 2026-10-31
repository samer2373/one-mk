mod admission;
mod decode;
pub mod sandbox;
mod transaction;
#[cfg(target_os = "linux")]
mod cgroup;
#[cfg(target_os = "linux")]
mod runtime_elf;

pub use admission::{Format, Media, inspect};
pub use transaction::{ImportOptions, ImportRecord, import};
pub fn decode_webm(codec: &str, width: u32, height: u32) -> Result<(), String> { decode::exec_webm(codec, width, height) }

pub const MAX_SOURCE: u64 = 512 * 1024 * 1024;
pub const MAX_FRAMES: u32 = 36_000;
pub const MAX_DURATION_MS: u64 = 600_000;
pub const MAX_PIXELS: u64 = 4096 * 2160;
pub const MAX_DECODED_PIXELS: u64 = 1_000_000_000;
pub const MAX_METADATA: usize = 256 * 1024;
pub const MAX_RESPONSE: u64 = 1024 * 1024;

pub(crate) fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > 4096 || height > 2160 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("dimensions exceed the 4096 x 2160 admission tier".into());
    }
    Ok(())
}

pub fn worker() -> Result<(), String> {
    sandbox::check_worker()?;
    checkpoint("start");
    let bytes = std::fs::read("/input").map_err(|error| error.to_string())?;
    checkpoint("probe");
    let media = inspect(&bytes).map_err(|error| format!("validation: {error}"))?;
    checkpoint("decode");
    let decoded = decode::decode(&bytes, media).map_err(|error| format!("validation: {error}"))?;
    checkpoint("poster");
    checkpoint("respond");
    serde_json::to_writer(std::io::stdout().lock(), &decoded).map_err(|error| error.to_string())
}

fn checkpoint(_stage: &str) {
    #[cfg(all(feature = "test-faults", target_os = "linux"))]
    if std::env::var("ONE_MK_WALL_TEST_STOP").as_deref() == Ok(_stage) { unsafe { libc::raise(libc::SIGSTOP); } }
}
