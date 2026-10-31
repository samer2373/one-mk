// SPDX-License-Identifier: MIT OR Apache-2.0
//! 6D-M1 step B laboratory playback (feature `m1-lab`, never in release
//! builds): turns the admitted plan into pixels.
//!
//! The renderer resolves the plan's media hash in the private import library,
//! re-verifies the source against that hash, and starts one sandboxed decode
//! worker (`one-mk-wall-import --play`, the importer's bubblewrap/seccomp/
//! cgroup model). Frames arrive as a bounded BGRx stream; one presenter thread
//! owns the inherited Wayland connection, one xdg toplevel spanning the local
//! monitors (the extension clones it per monitor) and two `wl_shm` buffers.
//! Each monitor gets its own slice of the one shared canvas (`crop.rs`).
//! Pausing destroys the worker and the surface: no paused child or timer.

use crate::crop::{Layout, Yuv};
use one_mk_wall_import::play::{Pixels, StreamHeader};
use one_mk_wall_import::sandbox::{Playback, PlaybackOptions};
use one_mk_wall_sync::bridge::Monitor;
use one_mk_wall_sync::presentation::PresentationPlan;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_region, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

pub type Result<T> = std::result::Result<T, String>;

/// Environment variable naming the laboratory playback configuration file.
pub const CONFIG_ENV: &str = "ONE_MK_WALL_M1_PLAYBACK";
const MAX_CONFIG: u64 = 4096;
const MAX_RECORD: u64 = 64 * 1024;
/// Decoded frames waiting for presentation (design: at most three ready).
const READY_FRAMES: usize = 2;

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

fn log(message: std::fmt::Arguments<'_>) {
    eprintln!("wallpaper-m1-renderer[{}]: {message}", std::process::id());
}

/// Trusted local configuration (never from a peer or the plan): where the
/// private import library, audited runtime, delegated cgroup and decode
/// worker are. A production package would fix these at install time.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub library: PathBuf,
    pub runtime: PathBuf,
    pub cgroup_root: PathBuf,
    pub worker: PathBuf,
}

fn private_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != uid() || metadata.mode() & 0o022 != 0 || metadata.len() > limit {
        return Err(format!("{} is not a bounded file owned by this user", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit { return Err("file grew past its bound".into()); }
    Ok(bytes)
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let path = PathBuf::from(std::env::var_os(CONFIG_ENV).ok_or("no laboratory playback configuration")?);
        if !path.is_absolute() { return Err("playback configuration path must be absolute".into()); }
        let config: Self = serde_json::from_slice(&private_file(&path, MAX_CONFIG)?).map_err(|error| error.to_string())?;
        for path in [&config.library, &config.runtime, &config.cgroup_root, &config.worker] {
            if !path.is_absolute() { return Err("playback configuration paths must be absolute".into()); }
        }
        let metadata = std::fs::symlink_metadata(&config.library).map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.uid() != uid() || metadata.mode() & 0o777 != 0o700 {
            return Err("import library must be a private directory owned by this user".into());
        }
        Ok(config)
    }
}

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }

/// The imported item named by `media`, re-verified against its hash. The
/// record and source were written by the importer; the bytes stay untrusted
/// and are only ever decoded inside the sandbox.
pub struct Media {
    pub record: one_mk_wall_import::ImportRecord,
    pub source: File,
}

pub fn resolve(config: &Config, media: &[u8; 32]) -> Result<Media> {
    let name = hex(media);
    let directory = config.library.join(&name);
    let metadata = std::fs::symlink_metadata(&directory).map_err(|_| format!("media {} is not in the import library", &name[..16]))?;
    if !metadata.is_dir() || metadata.uid() != uid() || metadata.mode() & 0o022 != 0 {
        return Err("imported item is not a private directory".into());
    }
    let record: one_mk_wall_import::ImportRecord = serde_json::from_slice(&private_file(&directory.join("record.json"), MAX_RECORD)?)
        .map_err(|error| format!("import record: {error}"))?;
    if record.schema != 1 || record.source_blake3 != name { return Err("import record does not name this media".into()); }
    let source = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("source")).map_err(|error| format!("imported source: {error}"))?;
    let metadata = source.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != uid() || metadata.mode() & 0o022 != 0
        || metadata.len() != record.source_bytes || metadata.len() > one_mk_wall_import::MAX_SOURCE {
        return Err("imported source does not match its record".into());
    }
    // Hash the open descriptor that the worker will read.
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    let mut total = 0_u64;
    let mut reader = &source;
    loop {
        let count = reader.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 { break; }
        total += count as u64;
        if total > record.source_bytes { return Err("imported source grew".into()); }
        hasher.update(&buffer[..count]);
    }
    if total != record.source_bytes || hasher.finalize().as_bytes() != media {
        return Err("imported source no longer matches the authorized media hash".into());
    }
    // SAFETY: lseek on an owned descriptor.
    if unsafe { libc::lseek(source.as_raw_fd(), 0, libc::SEEK_SET) } != 0 { return Err("cannot rewind source".into()); }
    Ok(Media { record, source })
}

/// What the session asks the presenter to show.
pub struct Request {
    pub plan: PresentationPlan,
    pub monitors: Vec<Monitor>,
}

enum Command { Play(u64, Box<Request>), Stop, Shutdown }

/// What the presenter tells the session. Each report names the playback
/// (`generation`, as returned by `Presenter::play`) it belongs to, so a
/// report from a replaced or stopped playback can be ignored.
#[derive(Debug)]
pub enum Report {
    /// The first frame of this playback was committed.
    FirstFrame { generation: u64, latency: Duration },
    /// A still or a long frame is on screen: no new frame is due soon, so the
    /// Shell must not expect damage (bridge state Holding).
    Holding { generation: u64 },
    /// Frames are changing again after a hold (bridge state Playing).
    Moving { generation: u64 },
    /// Playback cannot continue (decoder, sandbox or surface failure).
    Failed(String),
}

struct Wake(OwnedFd);

impl Wake {
    fn new() -> Result<Self> {
        // SAFETY: eventfd with valid flags; the result is checked.
        let descriptor = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if descriptor < 0 { return Err(std::io::Error::last_os_error().to_string()); }
        // SAFETY: a fresh descriptor owned from here on.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(descriptor) }))
    }
    fn signal(&self) {
        let one = 1_u64;
        // SAFETY: writes eight bytes from a valid u64.
        unsafe { libc::write(self.0.as_raw_fd(), (&one as *const u64).cast(), 8) };
    }
    fn drain(&self) {
        let mut value = 0_u64;
        // SAFETY: reads eight bytes into a valid u64.
        unsafe { libc::read(self.0.as_raw_fd(), (&mut value as *mut u64).cast(), 8) };
    }
}

pub struct Presenter {
    commands: mpsc::Sender<Command>,
    wake: std::sync::Arc<Wake>,
    pub reports: tokio::sync::mpsc::UnboundedReceiver<Report>,
    thread: Option<std::thread::JoinHandle<()>>,
    generation: u64,
}

impl Presenter {
    /// Takes ownership of the Shell-inherited Wayland socket.
    pub fn start(socket: UnixStream, config: Config) -> Result<Self> {
        let connection = Connection::from_socket(socket).map_err(|error| format!("Wayland: {error}"))?;
        let (commands, receiver) = mpsc::channel();
        let (reporter, reports) = tokio::sync::mpsc::unbounded_channel();
        let wake = std::sync::Arc::new(Wake::new()?);
        let thread_wake = wake.clone();
        let thread = std::thread::Builder::new().name("presenter".into()).spawn(move || {
            if let Err(error) = run(connection, config, receiver, &thread_wake, &reporter) {
                let _ = reporter.send(Report::Failed(error));
            }
        }).map_err(|error| error.to_string())?;
        Ok(Self { commands, wake, reports, thread: Some(thread), generation: 0 })
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
        self.wake.signal();
    }

    /// Starts (or replaces) playback; returns the generation its reports carry.
    pub fn play(&mut self, request: Request) -> u64 {
        self.generation += 1;
        self.send(Command::Play(self.generation, Box::new(request)));
        self.generation
    }

    pub fn stop(&self) { self.send(Command::Stop) }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

struct Buffer {
    proxy: wl_buffer::WlBuffer,
    pixels: *mut u32,
    length: usize,
    busy: bool,
    _file: File,
}

impl Buffer {
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: a live MAP_SHARED mapping of exactly `length` pixels, only
        // written by this thread while the compositor does not hold it.
        unsafe { std::slice::from_raw_parts_mut(self.pixels, self.length) }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.proxy.destroy();
        // SAFETY: unmaps the mapping created in `Window::new`.
        unsafe { libc::munmap(self.pixels.cast(), self.length * 4) };
    }
}

struct Window {
    surface: wl_surface::WlSurface,
    role: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    buffers: Vec<Buffer>,
    width: usize,
    height: usize,
    serial: Option<u32>,
    configured: bool,
    closed: bool,
    shown: Option<usize>,
}

impl Drop for Window {
    fn drop(&mut self) {
        self.buffers.clear();
        self.toplevel.destroy();
        self.role.destroy();
        self.surface.destroy();
    }
}

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm: Option<xdg_wm_base::XdgWmBase>,
    window: Option<Window>,
}

impl State {
    fn open_window(&mut self, width: usize, height: usize, handle: &QueueHandle<Self>) -> Result<()> {
        self.window = None;
        let compositor = self.compositor.as_ref().ok_or("no wl_compositor")?;
        let shm = self.shm.as_ref().ok_or("no wl_shm")?;
        let wm = self.wm.as_ref().ok_or("no xdg_wm_base")?;
        let bytes = width.checked_mul(height).and_then(|pixels| pixels.checked_mul(4)).filter(|&bytes| bytes <= i32::MAX as usize)
            .ok_or("surface too large")?;
        let mut buffers = Vec::with_capacity(2);
        for index in 0..2_usize {
            // SAFETY: memfd_create with a constant name; the result is checked.
            let descriptor = unsafe { libc::memfd_create(c"one-mk-wall-frame".as_ptr(), libc::MFD_CLOEXEC) };
            if descriptor < 0 { return Err(std::io::Error::last_os_error().to_string()); }
            // SAFETY: a fresh descriptor owned from here on.
            let file = unsafe { File::from_raw_fd(descriptor) };
            file.set_len(bytes as u64).map_err(|error| error.to_string())?;
            // SAFETY: maps the whole, just-sized file; checked below.
            let pixels = unsafe { libc::mmap(std::ptr::null_mut(), bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, descriptor, 0) };
            if pixels == libc::MAP_FAILED { return Err(std::io::Error::last_os_error().to_string()); }
            let pool = shm.create_pool(file.as_fd(), bytes as i32, handle, ());
            let proxy = pool.create_buffer(0, width as i32, height as i32, width as i32 * 4, wl_shm::Format::Xrgb8888, handle, index);
            pool.destroy();
            buffers.push(Buffer { proxy, pixels: pixels.cast(), length: width * height, busy: false, _file: file });
        }
        let surface = compositor.create_surface(handle, ());
        // An empty input region: the wallpaper never takes pointer input.
        let region = compositor.create_region(handle, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let role = wm.get_xdg_surface(&surface, handle, ());
        let toplevel = role.get_toplevel(handle, ());
        toplevel.set_app_id("one-mk-wall".into());
        toplevel.set_title("one-mk wallpaper".into());
        // Initial empty commit; the first buffer is attached only after the
        // configure is acknowledged (the M1a Mutter finding).
        surface.commit();
        self.window = Some(Window { surface, role, toplevel, buffers, width, height, serial: None, configured: false, closed: false, shown: None });
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, handle: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" if version >= 4 => state.compositor = Some(registry.bind(name, 4, handle, ())),
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "xdg_wm_base" => state.wm = Some(registry.bind(name, 1, handle, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(_: &mut Self, wm: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event { wm.pong(serial); }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(state: &mut Self, role: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event
            && let Some(window) = state.window.as_mut().filter(|window| window.role == *role) {
            window.serial = Some(serial);
            window.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(state: &mut Self, toplevel: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_toplevel::Event::Close = event
            && let Some(window) = state.window.as_mut().filter(|window| window.toplevel == *toplevel) {
            window.closed = true;
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, usize> for State {
    fn event(state: &mut Self, proxy: &wl_buffer::WlBuffer, event: wl_buffer::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event
            && let Some(buffer) = state.window.as_mut().and_then(|window| window.buffers.get_mut(*index)).filter(|buffer| buffer.proxy == *proxy) {
            buffer.busy = false;
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_region::WlRegion);

type Frame = std::result::Result<Vec<u32>, String>;

/// One playback: its worker, frame reader and schedule.
struct Running {
    worker: Playback,
    header: StreamHeader,
    layout: Layout,
    yuv: Option<Yuv>,
    frames: mpsc::Receiver<Frame>,
    recycle: mpsc::SyncSender<Vec<u32>>,
    reader: Option<std::thread::JoinHandle<()>>,
    pending: Option<Vec<u32>>,
    requested: Instant,
    generation: u64,
    /// Whether the Shell was told that no damage is due (still or long frame).
    holding: bool,
    /// Index of the next frame to present and when it is due.
    next: usize,
    due: Option<Instant>,
    last_commit: Option<Instant>,
    loops: u64,
    presented: u64,
    late: u64,
    starved: u64,
    finished: bool,
    /// Whether the frame now due was already counted as starved.
    waiting: bool,
}

impl Running {
    /// Kills the worker (and its cgroup) and joins the frame reader, which
    /// ends at the pipe's end-of-file or when its channel is gone.
    fn end_worker(&mut self) -> Result<String> {
        let errors = self.worker.stop();
        self.frames = mpsc::sync_channel(0).1;
        self.pending = None;
        if let Some(reader) = self.reader.take() { let _ = reader.join(); }
        errors
    }

    fn stop(&mut self, reason: &str) {
        let errors = self.end_worker();
        match errors {
            Ok(text) if text.trim().is_empty() => log(format_args!("event=playback-stop reason={reason} presented={} loops={} late={} starved={}",
                self.presented, self.loops, self.late, self.starved)),
            Ok(text) => log(format_args!("event=playback-stop reason={reason} presented={} worker_stderr={:?}", self.presented,
                text.chars().take(300).collect::<String>())),
            Err(error) => log(format_args!("event=playback-stop reason={reason} cleanup_error={error}")),
        }
    }
}

fn start(config: &Config, generation: u64, request: &Request, state: &mut State, handle: &QueueHandle<State>, wake: &std::sync::Arc<Wake>) -> Result<Running> {
    let requested = Instant::now();
    let media = resolve(config, &request.plan.media_id)?;
    let format = media.record.media.format;
    let mut worker = one_mk_wall_import::sandbox::spawn_playback(PlaybackOptions {
        runtime: &config.runtime, cgroup_root: &config.cgroup_root, worker: &config.worker, input: media.source })?;
    let mut pipe = worker.take_frames().ok_or("no frame stream")?;
    let header = StreamHeader::read(&mut pipe).map_err(|error| format!("worker: {error}"))?;
    let record = &media.record.media;
    if header.format != format || header.width != record.width || header.height != record.height
        || header.durations.len() != record.frames as usize {
        return Err("decoded stream does not match the import record".into());
    }
    let layout = Layout::new(&request.plan, &request.monitors, (header.width, header.height))?;
    state.open_window(layout.width, layout.height, handle)?;
    let (sender, frames) = mpsc::sync_channel::<Frame>(READY_FRAMES);
    let (recycle, recycled) = mpsc::sync_channel::<Vec<u32>>(READY_FRAMES + 2);
    let frame_bytes = header.frame_bytes();
    let words = frame_bytes.div_ceil(4);
    let wake = wake.clone();
    let reader = std::thread::Builder::new().name("frames".into()).spawn(move || loop {
        let mut frame = recycled.try_recv().unwrap_or_else(|_| vec![0_u32; words]);
        // SAFETY: a u32 slice viewed as bytes; every byte pattern is valid.
        let bytes = unsafe { std::slice::from_raw_parts_mut(frame.as_mut_ptr().cast::<u8>(), frame_bytes) };
        let result = pipe.read_exact(bytes).map(|()| frame).map_err(|error| error.to_string());
        let end = result.is_err();
        if sender.send(result).is_err() { return; }
        wake.signal();
        if end { return; }
    }).map_err(|error| error.to_string())?;
    log(format_args!("event=playback-start media={} format={format:?} pixels={:?} source={}x{} frames={} loop_ms={} surface={}x{} worker_pid={}",
        &hex(&request.plan.media_id)[..16], header.pixels, header.width, header.height, header.durations.len(), header.loop_micros() / 1000,
        layout.width, layout.height, worker.pid()));
    let yuv = match header.pixels { Pixels::I420 { bt709, full_range } => Some(Yuv::new(bt709, full_range)), Pixels::Bgrx => None };
    Ok(Running { worker, header, layout, yuv, frames, recycle, reader: Some(reader), pending: None, requested, generation,
        holding: false, next: 0, due: None, last_commit: None, loops: 0, presented: 0, late: 0, starved: 0, finished: false,
        waiting: false })
}

/// The Shell's damage watchdog stops a renderer that shows nothing new for
/// 3 s while it reports Playing. A still, or a frame that stays on screen at
/// least this long, is announced instead (bridge state Holding: no damage
/// due, no watchdog), so the same pixels are never committed again just to
/// look alive and a still costs no periodic presentation work.
pub const HOLD_AFTER: Duration = Duration::from_millis(2000);

/// Whether a frame shown for `duration` (a still: `None`) must be announced
/// as held rather than kept alive by repeated commits.
pub fn holds(duration: Option<Duration>) -> bool { duration.is_none_or(|duration| duration >= HOLD_AFTER) }

/// Acknowledges a pending configure by committing the frame on screen again;
/// nothing is committed otherwise. A buffer still held by the compositor is
/// retried when its release wakes the loop.
fn acknowledge(window: &mut Window) {
    let Some(index) = window.shown else { return };
    if window.serial.is_none() || window.buffers[index].busy { return; }
    if let Some(serial) = window.serial.take() { window.role.ack_configure(serial); }
    let buffer = &mut window.buffers[index];
    window.surface.attach(Some(&buffer.proxy), 0, 0);
    window.surface.damage_buffer(0, 0, window.width as i32, window.height as i32);
    window.surface.commit();
    buffer.busy = true;
}

/// Presents what is due; returns how long to sleep at most.
fn step(running: &mut Running, state: &mut State, reporter: &tokio::sync::mpsc::UnboundedSender<Report>) -> Result<Option<Duration>> {
    let window = state.window.as_mut().ok_or("presentation surface missing")?;
    if window.closed { return Err("the compositor closed the wallpaper surface".into()); }
    if !window.configured { return Ok(None); }
    if running.finished {
        // A still keeps its one frame: no timer, only later configures are
        // acknowledged with it.
        acknowledge(window);
        return Ok(None);
    }
    let now = Instant::now();
    if let Some(due) = running.due && now < due {
        acknowledge(window);
        return Ok(Some(due - now));
    }
    if running.pending.is_none() {
        match running.frames.try_recv() {
            Ok(Ok(frame)) => running.pending = Some(frame),
            Ok(Err(error)) => return Err(format!("decode worker stopped: {error}")),
            Err(mpsc::TryRecvError::Empty) => {
                if running.due.is_some() && !running.waiting { running.starved += 1; running.waiting = true; }
                return Ok(None);
            }
            Err(mpsc::TryRecvError::Disconnected) => return Err("frame reader ended".into()),
        }
    }
    let Some(index) = window.buffers.iter().position(|buffer| !buffer.busy) else { return Ok(None) };
    let frame = running.pending.take().ok_or("no frame")?;
    running.waiting = false;
    match &running.yuv {
        // SAFETY: the u32 frame viewed as the bytes the reader filled.
        Some(yuv) => running.layout.compose_i420(unsafe { std::slice::from_raw_parts(frame.as_ptr().cast::<u8>(), frame.len() * 4) },
            yuv, window.buffers[index].pixels()),
        None => running.layout.compose(&frame, window.buffers[index].pixels()),
    }
    let _ = running.recycle.try_send(frame);
    if let Some(serial) = window.serial.take() { window.role.ack_configure(serial); }
    let buffer = &mut window.buffers[index];
    window.surface.attach(Some(&buffer.proxy), 0, 0);
    window.surface.damage_buffer(0, 0, window.width as i32, window.height as i32);
    window.surface.commit();
    buffer.busy = true;
    window.shown = Some(index);
    let committed = Instant::now();
    let presented = running.next;
    let duration = Duration::from_micros(u64::from(running.header.durations[presented]));
    if running.presented == 0 {
        let latency = committed - running.requested;
        log(format_args!("event=first-frame latency_ms={}", latency.as_millis()));
        let _ = reporter.send(Report::FirstFrame { generation: running.generation, latency });
    }
    let hold = holds((!running.header.still()).then_some(duration));
    if hold != running.holding {
        running.holding = hold;
        let generation = running.generation;
        let _ = reporter.send(if hold { Report::Holding { generation } } else { Report::Moving { generation } });
    }
    running.presented += 1;
    if presented == 0 && running.presented > 1 {
        // Loop seam: the gap between the previous loop's last frame and this one.
        let previous = *running.header.durations.last().expect("frames");
        let gap = running.last_commit.map_or(0, |last| (committed - last).as_micros());
        running.loops += 1;
        log(format_args!("event=loop loops={} seam_us={gap} nominal_us={previous} presented={} late={} starved={}",
            running.loops, running.presented, running.late, running.starved));
    }
    running.last_commit = Some(committed);
    if running.header.still() {
        running.finished = true;
        // The worker has sent its only frame; reap it now: no decoder stays.
        let errors = running.end_worker();
        log(format_args!("event=still-presented worker=stopped cleanup={} holding=true", if errors.is_ok() { "ok" } else { "failed" }));
        return Ok(None);
    }
    // Keep the timeline anchored; re-anchor only after falling a frame behind.
    let due = running.due.unwrap_or(committed);
    running.due = Some(if committed > due + duration { running.late += 1; committed + duration } else { due + duration });
    running.next = (presented + 1) % running.header.durations.len();
    Ok(running.due.map(|due| due.saturating_duration_since(Instant::now())))
}

fn run(connection: Connection, config: Config, commands: mpsc::Receiver<Command>, wake: &std::sync::Arc<Wake>,
    reporter: &tokio::sync::mpsc::UnboundedSender<Report>) -> Result<()> {
    let mut queue: EventQueue<State> = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).map_err(|error| format!("Wayland registry: {error}"))?;
    let mut running: Option<Running> = None;
    let result = (|| loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Play(generation, request) => {
                    if let Some(mut previous) = running.take() { previous.stop("replaced"); }
                    state.window = None;
                    running = Some(start(&config, generation, &request, &mut state, &handle, wake)?);
                }
                Command::Stop => {
                    if let Some(mut previous) = running.take() { previous.stop("plan"); }
                    state.window = None;
                }
                Command::Shutdown => return Ok(()),
            }
        }
        let timeout = match running.as_mut() {
            Some(current) => step(current, &mut state, reporter)?,
            None => None,
        };
        connection.flush().map_err(|error| format!("Wayland flush: {error}"))?;
        let Some(guard) = queue.prepare_read() else {
            queue.dispatch_pending(&mut state).map_err(|error| error.to_string())?;
            continue;
        };
        let mut descriptors = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        let timespec = timeout.map(|timeout| libc::timespec { tv_sec: timeout.as_secs() as libc::time_t, tv_nsec: timeout.subsec_nanos() as libc::c_long });
        // SAFETY: two valid pollfds and an optional valid timespec.
        let ready = unsafe { libc::ppoll(descriptors.as_mut_ptr(), 2, timespec.as_ref().map_or(std::ptr::null(), |value| value as *const _), std::ptr::null()) };
        if ready < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if descriptors[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 { return Err("Wayland connection closed".into()); }
        if descriptors[0].revents & libc::POLLIN != 0 {
            match guard.read() {
                Ok(_) => {}
                // Nothing to read after all (the readiness was consumed or
                // spurious): not an error, the next poll waits again.
                Err(wayland_client::backend::WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(format!("Wayland read: {error}")),
            }
        } else {
            drop(guard);
        }
        queue.dispatch_pending(&mut state).map_err(|error| error.to_string())?;
        if descriptors[1].revents & libc::POLLIN != 0 { wake.drain(); }
    })();
    if let Some(mut current) = running.take() { current.stop(if result.is_ok() { "shutdown" } else { "failure" }); }
    state.window = None;
    let _ = connection.flush();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stills_and_long_frames_are_held_short_frames_are_not() {
        assert!(holds(None), "a still is held");
        assert!(!holds(Some(Duration::from_millis(1999))));
        assert!(holds(Some(HOLD_AFTER)));
        assert!(holds(Some(Duration::from_secs(10))));
        // Held frames must be announced before the Shell's 3 s damage watchdog.
        assert!(HOLD_AFTER < Duration::from_secs(3));
    }
}
